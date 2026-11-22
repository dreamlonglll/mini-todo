//! 行写入、记录级 LWW 合并、强制拉取/推送准备与本地快照读取。

use super::parse::{
    load_assignments, normalize_list_lenient, normalize_todo_lenient, set_assignment, value_id,
    value_subtasks, value_updated_at, with_assignment, ListAssignment, RemoteList, RemoteTodo,
};
use super::tombstones::{
    apply_list_tombstones_to_local, apply_tombstones_to_local, deletion_time, is_list_suppressed,
    is_suppressed, list_tombstones, load_list_tombstone_index, load_list_tombstones,
    load_tombstone_index, record_list_tombstone, record_tombstone, remove_list_tombstone,
    remove_tombstone,
};
use super::{local_seq, EntityKind, TOMBSTONE_RETENTION_DAYS};
use crate::db::models::{
    todo_list_from_row, ListTombstone, SubTask, Todo, TodoList, Tombstone, DEFAULT_LIST_ID,
    TODO_LIST_COLUMNS,
};
use crate::db::time::{days_ago_local, superseding};
use crate::db::{subtask_from_row, todo_from_row, SUBTASK_COLUMNS, TODO_COLUMNS};
use rusqlite::{params, Connection, ErrorCode, OptionalExtension, Result, Transaction};
use serde_json::Value;
use std::collections::{HashMap, HashSet};

// ============================================================================
// 行写入（保留远端 / 备份里的 id）
// ============================================================================

pub fn insert_todo_row(conn: &Connection, t: &Todo) -> Result<()> {
    conn.prepare_cached(
        "INSERT INTO todos (id, title, description, color, quadrant,
                            notify_at, notify_before, notified, completed,
                            sort_order, start_time, end_time, created_at, updated_at,
                            repeat_enabled, repeat_type, repeat_interval,
                            repeat_weekdays, repeat_month_day, list_id, list_updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14,
                 ?15, ?16, ?17, ?18, ?19, ?20, ?21)",
    )?
    .execute(params![
        t.id,
        t.title,
        t.description,
        t.color,
        t.quadrant,
        t.notify_at,
        t.notify_before,
        t.notified as i32,
        t.completed as i32,
        t.sort_order,
        t.start_time,
        t.end_time,
        t.created_at,
        t.updated_at,
        t.repeat_enabled as i32,
        t.repeat_type,
        t.repeat_interval,
        t.repeat_weekdays,
        t.repeat_month_day,
        t.list_id,
        t.list_id.and(t.list_updated_at.as_deref()),
    ])?;
    Ok(())
}

pub fn update_todo_row(conn: &Connection, t: &Todo) -> Result<()> {
    conn.prepare_cached(
        "UPDATE todos SET
            title = ?1, description = ?2, color = ?3, quadrant = ?4,
            notify_at = ?5, notify_before = ?6, notified = ?7,
            completed = ?8, sort_order = ?9, start_time = ?10, end_time = ?11,
            created_at = ?12, updated_at = ?13,
            repeat_enabled = ?14, repeat_type = ?15, repeat_interval = ?16,
            repeat_weekdays = ?17, repeat_month_day = ?18, list_id = ?19,
            list_updated_at = ?20
         WHERE id = ?21",
    )?
    .execute(params![
        t.title,
        t.description,
        t.color,
        t.quadrant,
        t.notify_at,
        t.notify_before,
        t.notified as i32,
        t.completed as i32,
        t.sort_order,
        t.start_time,
        t.end_time,
        t.created_at,
        t.updated_at,
        t.repeat_enabled as i32,
        t.repeat_type,
        t.repeat_interval,
        t.repeat_weekdays,
        t.repeat_month_day,
        t.list_id,
        t.list_id.and(t.list_updated_at.as_deref()),
        t.id,
    ])?;
    Ok(())
}

pub fn insert_subtask_row(conn: &Connection, s: &SubTask) -> Result<()> {
    conn.prepare_cached(
        "INSERT INTO subtasks (id, parent_id, title, content, completed,
                               sort_order, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
    )?
    .execute(params![
        s.id,
        s.parent_id,
        s.title,
        s.content,
        s.completed as i32,
        s.sort_order,
        s.created_at,
        s.updated_at,
    ])?;
    Ok(())
}

pub fn update_subtask_row(conn: &Connection, s: &SubTask) -> Result<()> {
    conn.prepare_cached(
        "UPDATE subtasks SET
            parent_id = ?1, title = ?2, content = ?3, completed = ?4,
            sort_order = ?5, created_at = ?6, updated_at = ?7
         WHERE id = ?8",
    )?
    .execute(params![
        s.parent_id,
        s.title,
        s.content,
        s.completed as i32,
        s.sort_order,
        s.created_at,
        s.updated_at,
        s.id,
    ])?;
    Ok(())
}

pub fn insert_list_row(conn: &Connection, l: &TodoList) -> Result<()> {
    conn.prepare_cached(
        "INSERT INTO todo_lists (id, name, sort_order, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
    )?
    .execute(params![
        l.id,
        l.name,
        l.sort_order,
        l.created_at,
        l.updated_at
    ])?;
    Ok(())
}

pub fn update_list_row(conn: &Connection, l: &TodoList) -> Result<()> {
    conn.prepare_cached(
        "UPDATE todo_lists SET name = ?1, sort_order = ?2, created_at = ?3, updated_at = ?4
         WHERE id = ?5",
    )?
    .execute(params![
        l.name,
        l.sort_order,
        l.created_at,
        l.updated_at,
        l.id
    ])?;
    Ok(())
}

/// 全部清单，按 `(sort_order, id)` 升序（契约 §8）
pub fn load_lists(conn: &Connection) -> Result<Vec<TodoList>> {
    let mut stmt = conn.prepare_cached(&format!(
        "SELECT {TODO_LIST_COLUMNS} FROM todo_lists ORDER BY sort_order ASC, id ASC"
    ))?;
    let rows = stmt.query_map([], todo_list_from_row)?;
    rows.collect()
}

fn load_list_row(conn: &Connection, id: i64) -> Result<Option<TodoList>> {
    conn.prepare_cached(&format!(
        "SELECT {TODO_LIST_COLUMNS} FROM todo_lists WHERE id = ?1"
    ))?
    .query_row([id], todo_list_from_row)
    .optional()
}

// ============================================================================
// 合并
// ============================================================================

/// 一次合并 / 强制拉取对本地的影响统计
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct MergeStats {
    pub todos_inserted: u32,
    pub todos_updated: u32,
    pub todos_deleted: u32,
    pub subtasks_inserted: u32,
    pub subtasks_updated: u32,
    pub subtasks_deleted: u32,
    /// 待办清单（v31）
    pub lists_inserted: u32,
    pub lists_updated: u32,
    pub lists_deleted: u32,
    /// 无法识别 / 无法落库而跳过的远端记录数（含清单；绝不当作删除）
    pub records_skipped: u32,
}

impl MergeStats {
    /// 把同一次合并里另一部分（清单）的统计加进来
    pub fn absorb(&mut self, other: MergeStats) {
        self.todos_inserted += other.todos_inserted;
        self.todos_updated += other.todos_updated;
        self.todos_deleted += other.todos_deleted;
        self.subtasks_inserted += other.subtasks_inserted;
        self.subtasks_updated += other.subtasks_updated;
        self.subtasks_deleted += other.subtasks_deleted;
        self.lists_inserted += other.lists_inserted;
        self.lists_updated += other.lists_updated;
        self.lists_deleted += other.lists_deleted;
        self.records_skipped += other.records_skipped;
    }
}

/// 在 savepoint 里写单条记录：约束冲突（外键、唯一键等）只回滚这一条并返回 `Ok(false)`，
/// 其它错误（IO、磁盘满……）向上传播让整个合并回滚。
fn write_record<F>(tx: &mut Transaction<'_>, what: &str, f: F) -> Result<bool>
where
    F: FnOnce(&Connection) -> Result<()>,
{
    let sp = tx.savepoint()?;
    match f(&sp) {
        Ok(()) => {
            sp.commit()?;
            Ok(true)
        }
        Err(e) if e.sqlite_error_code() == Some(ErrorCode::ConstraintViolation) => {
            log::warn!("[sync] 跳过无法落库的远端记录 {}: {}", what, e);
            Ok(false)
        }
        Err(e) => Err(e),
    }
}

fn local_updated_at(conn: &Connection, kind: EntityKind, id: i64) -> Result<Option<String>> {
    let sql = match kind {
        EntityKind::Todo => "SELECT updated_at FROM todos WHERE id = ?1",
        EntityKind::Subtask => "SELECT updated_at FROM subtasks WHERE id = ?1",
    };
    conn.prepare_cached(sql)?
        .query_row([id], |r| r.get(0))
        .optional()
}

fn todo_exists(conn: &Connection, id: i64) -> Result<bool> {
    Ok(conn
        .prepare_cached("SELECT 1 FROM todos WHERE id = ?1")?
        .query_row([id], |_| Ok(()))
        .optional()?
        .is_some())
}

/// 本地待办当前的 (updated_at, 归属)
fn local_todo_version(conn: &Connection, id: i64) -> Result<Option<(String, ListAssignment)>> {
    conn.prepare_cached("SELECT updated_at, list_id, list_updated_at FROM todos WHERE id = ?1")?
        .query_row([id], |r| {
            let list_id: Option<i64> = r.get(1)?;
            let at: Option<String> = r.get(2)?;
            Ok((r.get(0)?, ListAssignment::new(list_id, at.as_deref())))
        })
        .optional()
}

/// K3 记录级合并：把远端待办 / 子任务 / 墓碑合并进本地（调用方提供事务）。
///
/// 1. 远端墓碑（`deleted_at >= cutoff`）并入本地墓碑表，同键取较大时间
/// 2. 墓碑应用到本地：`updated_at <= deleted_at` 的记录删除（待办连带子任务）
/// 3. 逐条 LWW：远端较新 → 覆盖（保留远端 updatedAt）；平局 / 本地较新 → 不动；
///    本地没有 → 以远端 id 插入；被墓碑压制 → 丢弃
/// 4. 仅本地存在的记录保留（不做缺席清理）
/// 5. 无法识别 / 无法落库的远端记录跳过并计数，不影响其它记录
/// 6. 待办的清单归属（契约 §13，字段级 LWW，取代 §12 的规则 2 / 3）：记录级结果照上面判定；结果记录存在时，它的归属取
///    所有可见版本里最新的一个（[`ListAssignment`]）——本地合并前的版本（在第 2 步之前读出：哪怕它被墓碑删掉、又被
///    远端更新的编辑救回来，F2）与远端版本（哪怕它被墓碑压制，F4）；墓碑不影响归属。畸形的远端记录不参与（F6）。
///    只改归属时不改 `updated_at`，计入 `todos_updated`（本地数据变了，界面要刷新）；它让 `local_seq` 加一，但本次
///    同步收尾时 `synced_seq` 取合并之后的值，不会因此多上传一次
pub fn merge_remote(
    tx: &mut Transaction<'_>,
    remote: &[RemoteTodo],
    remote_tombstones: &[Tombstone],
    cutoff: &str,
) -> Result<MergeStats> {
    let mut stats = MergeStats::default();
    // 归属的可见版本之一：本地合并前的归属，必须在应用墓碑之前读出
    let before = load_assignments(tx)?;

    for tb in remote_tombstones {
        let Some(kind) = EntityKind::parse(&tb.entity_type) else {
            continue;
        };
        if tb.deleted_at.as_str() >= cutoff {
            record_tombstone(tx, kind, tb.entity_id, &tb.deleted_at)?;
        }
    }

    let (todos_deleted, subtasks_deleted) = apply_tombstones_to_local(tx)?;
    stats.todos_deleted += todos_deleted;
    stats.subtasks_deleted += subtasks_deleted;

    let tombs = load_tombstone_index(tx)?;

    for rt in remote {
        let Some(todo_id) = rt.id else {
            stats.records_skipped += 1 + rt.subtasks.len() as u32;
            log::warn!(
                "[sync] 跳过缺少 id 的远端待办: {}",
                rt.error.as_deref().unwrap_or("")
            );
            continue;
        };

        match &rt.todo {
            Some(todo) => {
                let label = format!("todo#{todo_id}");
                let current = local_todo_version(tx, todo_id)?;
                // 所有可见版本里最新的归属：远端版本、本地合并前的版本、本地当前版本（墓碑不影响；当前版本只在远端数组里
                // 同一 id 出现多次时与合并前不同，带上它保证不会退回更旧的归属）
                let mut best = ListAssignment::of(todo);
                if let Some(local) = before.get(&todo_id) {
                    best = local.clone().newest(best);
                }
                if let Some((_, assignment)) = &current {
                    best = assignment.clone().newest(best);
                }
                if !is_suppressed(&tombs, EntityKind::Todo, todo_id, &todo.updated_at) {
                    match current {
                        // 本地没有（或刚被墓碑删掉、被远端更新的编辑救回来）：以远端 id 插入，归属取最新的
                        None => {
                            let incoming = with_assignment(todo, &best);
                            if write_record(tx, &label, |c| insert_todo_row(c, &incoming))? {
                                stats.todos_inserted += 1;
                            } else {
                                stats.records_skipped += 1;
                            }
                        }
                        Some((local_at, _)) if todo.updated_at > local_at => {
                            let incoming = with_assignment(todo, &best);
                            if write_record(tx, &label, |c| update_todo_row(c, &incoming))? {
                                stats.todos_updated += 1;
                            } else {
                                stats.records_skipped += 1;
                            }
                        }
                        // 平局 / 本地较新：记录不动，只按需采纳更新的归属（不改 updated_at）
                        Some((_, assignment)) => {
                            if best != assignment && set_assignment(tx, todo_id, &best)? {
                                stats.todos_updated += 1;
                            }
                        }
                    }
                } else if let Some((_, assignment)) = current {
                    // 远端版本被墓碑压制、本地记录还在（删除后又编辑过）：记录不动，归属照样取最新的（F4）
                    if best != assignment && set_assignment(tx, todo_id, &best)? {
                        stats.todos_updated += 1;
                    }
                }
            }
            // 看不懂、但已被墓碑压制的远端版本：本来就该删，不算跳过
            None if is_suppressed(&tombs, EntityKind::Todo, todo_id, &rt.updated_at) => {}
            None => {
                stats.records_skipped += 1;
                log::warn!(
                    "[sync] 跳过无法识别的远端待办 #{}: {}",
                    todo_id,
                    rt.error.as_deref().unwrap_or("")
                );
            }
        }

        let parent_exists = todo_exists(tx, todo_id)?;
        let parent_tombstoned = tombs.contains_key(&(EntityKind::Todo, todo_id));
        for rs in &rt.subtasks {
            let Some(sub) = &rs.subtask else {
                // 父待办已删除时随父丢弃；否则跳过计数
                if parent_exists || !parent_tombstoned {
                    stats.records_skipped += 1;
                    log::warn!(
                        "[sync] 跳过无法识别的远端子任务 {:?}（待办 #{}）: {}",
                        rs.id,
                        todo_id,
                        rs.error.as_deref().unwrap_or("")
                    );
                }
                continue;
            };
            if is_suppressed(&tombs, EntityKind::Subtask, sub.id, &sub.updated_at) {
                continue;
            }
            if !parent_exists {
                // 父待办已被删除：随父丢弃；父待办因无法识别而不存在：跳过计数
                if !parent_tombstoned {
                    stats.records_skipped += 1;
                }
                continue;
            }
            match local_updated_at(tx, EntityKind::Subtask, sub.id)? {
                None => {
                    let label = format!("subtask#{}", sub.id);
                    if write_record(tx, &label, |c| insert_subtask_row(c, sub))? {
                        stats.subtasks_inserted += 1;
                    } else {
                        stats.records_skipped += 1;
                    }
                }
                Some(local) if sub.updated_at > local => {
                    let label = format!("subtask#{}", sub.id);
                    if write_record(tx, &label, |c| update_subtask_row(c, sub))? {
                        stats.subtasks_updated += 1;
                    } else {
                        stats.records_skipped += 1;
                    }
                }
                Some(_) => {}
            }
        }
    }

    Ok(stats)
}

fn local_list_updated_at(conn: &Connection, id: i64) -> Result<Option<String>> {
    conn.prepare_cached("SELECT updated_at FROM todo_lists WHERE id = ?1")?
        .query_row([id], |r| r.get(0))
        .optional()
}

/// 清单的记录级合并（契约 §4，与待办同构；调用方提供事务，与 [`merge_remote`] 同一个事务）。
///
/// 1. 远端清单墓碑（永不过期，契约 §13-10；默认清单的在解析时已丢弃）并入本地，同键取较大时间
/// 2. 墓碑应用到本地：`updated_at <= deleted_at` 的清单删除（默认清单永远保留）；其下待办不改写，读侧按默认清单显示
/// 3. `remote = None`（远端文档没有 `todoLists` 键：旧版本写入方 / 还没有新版本写过）：视为远端没有清单信息，只做 1、2
/// 4. 逐条 LWW：远端较新 → 覆盖；平局 / 本地较新 → 不动；本地没有 → 以远端 id 插入；被墓碑压制 → 丢弃
/// 5. 仅本地存在的清单保留（不做缺席清理）；同步造成的重名不处理
/// 6. 无法识别 / 无法落库的远端清单跳过并计数，不影响其它记录
pub fn merge_remote_lists(
    tx: &mut Transaction<'_>,
    remote: Option<&[RemoteList]>,
    remote_tombstones: &[ListTombstone],
) -> Result<MergeStats> {
    let mut stats = MergeStats::default();
    for tb in remote_tombstones {
        record_list_tombstone(tx, tb.entity_id, &tb.deleted_at)?;
    }
    stats.lists_deleted += apply_list_tombstones_to_local(tx)?;

    let Some(remote) = remote else {
        return Ok(stats);
    };
    let tombs = load_list_tombstone_index(tx)?;
    for rl in remote {
        let Some(id) = rl.id else {
            stats.records_skipped += 1;
            log::warn!(
                "[sync] 跳过缺少 id 的远端清单: {}",
                rl.error.as_deref().unwrap_or("")
            );
            continue;
        };
        match &rl.list {
            Some(list) => {
                if is_list_suppressed(&tombs, id, &list.updated_at) {
                    continue;
                }
                let label = format!("list#{id}");
                match local_list_updated_at(tx, id)? {
                    None => {
                        if write_record(tx, &label, |c| insert_list_row(c, list))? {
                            stats.lists_inserted += 1;
                        } else {
                            stats.records_skipped += 1;
                        }
                    }
                    Some(local) if list.updated_at > local => {
                        if write_record(tx, &label, |c| update_list_row(c, list))? {
                            stats.lists_updated += 1;
                        } else {
                            stats.records_skipped += 1;
                        }
                    }
                    Some(_) => {}
                }
            }
            // 看不懂、但已被墓碑压制的远端版本：本来就该删，不算跳过
            None if is_list_suppressed(&tombs, id, &rl.updated_at) => {}
            None => {
                stats.records_skipped += 1;
                log::warn!(
                    "[sync] 跳过无法识别的远端清单 #{}: {}",
                    id,
                    rl.error.as_deref().unwrap_or("")
                );
            }
        }
    }
    Ok(stats)
}

fn load_todo_row(conn: &Connection, id: i64) -> Result<Option<Todo>> {
    let sql = format!("SELECT {} FROM todos WHERE id = ?1", TODO_COLUMNS);
    conn.query_row(&sql, [id], todo_from_row).optional()
}

fn load_subtask_row(conn: &Connection, id: i64) -> Result<Option<SubTask>> {
    let sql = format!("SELECT {} FROM subtasks WHERE id = ?1", SUBTASK_COLUMNS);
    conn.query_row(&sql, [id], subtask_from_row).optional()
}

fn all_ids(conn: &Connection, table: &str) -> Result<Vec<i64>> {
    let mut stmt = conn.prepare(&format!("SELECT id FROM {table}"))?;
    let rows = stmt.query_map([], |r| r.get(0))?;
    rows.collect()
}

/// 强制拉取（K7 `webdav_force_pull`）：让本地等于远端。
///
/// - 本地墓碑表替换为远端墓碑（保留期内）
/// - 删除本地独有的待办 / 子任务（不生成墓碑）
/// - 远端记录无条件覆盖本地（被远端自身墓碑压制的除外）；远端待办没有 `listId`（归属未知）时保留本地已知的归属
///   （`listId` 与 `listUpdatedAt` 一起），远端已知时连同它的 `listUpdatedAt` 照搬
/// - 无法识别的远端记录跳过计数；同 id 的本地记录保留（无法用看不懂的版本替换）
pub fn force_pull_replace(
    tx: &mut Transaction<'_>,
    remote: &[RemoteTodo],
    remote_tombstones: &[Tombstone],
    cutoff: &str,
) -> Result<MergeStats> {
    let mut stats = MergeStats::default();

    tx.execute("DELETE FROM tombstones", [])?;
    for tb in remote_tombstones {
        let Some(kind) = EntityKind::parse(&tb.entity_type) else {
            continue;
        };
        if tb.deleted_at.as_str() >= cutoff {
            record_tombstone(tx, kind, tb.entity_id, &tb.deleted_at)?;
        }
    }
    let tombs = load_tombstone_index(tx)?;

    let remote_todo_ids: HashSet<i64> = remote.iter().filter_map(|r| r.id).collect();
    let remote_subtask_ids: HashSet<i64> = remote
        .iter()
        .flat_map(|r| r.subtasks.iter().filter_map(|s| s.id))
        .collect();

    for id in all_ids(tx, "subtasks")? {
        if !remote_subtask_ids.contains(&id) {
            stats.subtasks_deleted +=
                tx.execute("DELETE FROM subtasks WHERE id = ?1", [id])? as u32;
        }
    }
    for id in all_ids(tx, "todos")? {
        if !remote_todo_ids.contains(&id) {
            stats.subtasks_deleted +=
                tx.execute("DELETE FROM subtasks WHERE parent_id = ?1", [id])? as u32;
            stats.todos_deleted += tx.execute("DELETE FROM todos WHERE id = ?1", [id])? as u32;
        }
    }

    for rt in remote {
        let Some(todo_id) = rt.id else {
            stats.records_skipped += 1 + rt.subtasks.len() as u32;
            continue;
        };
        match &rt.todo {
            Some(todo) if is_suppressed(&tombs, EntityKind::Todo, todo_id, &todo.updated_at) => {
                // 远端自相矛盾（记录与压制它的墓碑并存）：按墓碑处理
                stats.subtasks_deleted +=
                    tx.execute("DELETE FROM subtasks WHERE parent_id = ?1", [todo_id])? as u32;
                stats.todos_deleted +=
                    tx.execute("DELETE FROM todos WHERE id = ?1", [todo_id])? as u32;
                continue;
            }
            Some(todo) => {
                let local = load_todo_row(tx, todo_id)?;
                // 远端不认识清单（没有 listId）：保留本地已知的归属（两列一起）——未知不覆盖已知；
                // 远端已知时以远端为准（强制拉取 = 云端覆盖本地，哪怕本地的归属版本更新）
                let filled;
                let incoming = match &local {
                    Some(l) if todo.list_id.is_none() && l.list_id.is_some() => {
                        filled = with_assignment(todo, &ListAssignment::of(l));
                        &filled
                    }
                    _ => todo,
                };
                let label = format!("todo#{todo_id}");
                match local {
                    None => {
                        if write_record(tx, &label, |c| insert_todo_row(c, incoming))? {
                            stats.todos_inserted += 1;
                        } else {
                            stats.records_skipped += 1;
                        }
                    }
                    Some(local) if &local != incoming => {
                        if write_record(tx, &label, |c| update_todo_row(c, incoming))? {
                            stats.todos_updated += 1;
                        } else {
                            stats.records_skipped += 1;
                        }
                    }
                    Some(_) => {}
                }
            }
            None => {
                stats.records_skipped += 1;
                log::warn!(
                    "[sync] 强制拉取：跳过无法识别的远端待办 #{}: {}",
                    todo_id,
                    rt.error.as_deref().unwrap_or("")
                );
            }
        }

        let parent_exists = todo_exists(tx, todo_id)?;
        for rs in &rt.subtasks {
            let Some(sub) = &rs.subtask else {
                stats.records_skipped += 1;
                continue;
            };
            if is_suppressed(&tombs, EntityKind::Subtask, sub.id, &sub.updated_at) {
                stats.subtasks_deleted +=
                    tx.execute("DELETE FROM subtasks WHERE id = ?1", [sub.id])? as u32;
                continue;
            }
            if !parent_exists {
                stats.records_skipped += 1;
                continue;
            }
            match load_subtask_row(tx, sub.id)? {
                None => {
                    let label = format!("subtask#{}", sub.id);
                    if write_record(tx, &label, |c| insert_subtask_row(c, sub))? {
                        stats.subtasks_inserted += 1;
                    } else {
                        stats.records_skipped += 1;
                    }
                }
                Some(local) if &local != sub => {
                    let label = format!("subtask#{}", sub.id);
                    if write_record(tx, &label, |c| update_subtask_row(c, sub))? {
                        stats.subtasks_updated += 1;
                    } else {
                        stats.records_skipped += 1;
                    }
                }
                Some(_) => {}
            }
        }
    }

    Ok(stats)
}

/// 强制拉取的清单部分（契约 §5）：让本地清单 = 远端。调用方提供事务（与 [`force_pull_replace`] 同一个）。
///
/// - `remote = None`（远端文档没有 `todoLists` 键）：远端没有清单信息，本地清单与清单墓碑都不动
/// - 本地清单墓碑替换为远端的（清单墓碑永不过期，远端写出的是全部）
/// - 删除本地独有的清单（不写墓碑；其下待办靠读侧兜底显示在默认清单）；默认清单永远保留
/// - 远端清单无条件覆盖本地（被远端自身墓碑压制的除外）；无法识别的跳过计数，同 id 的本地记录保留
pub fn force_pull_replace_lists(
    tx: &mut Transaction<'_>,
    remote: Option<&[RemoteList]>,
    remote_tombstones: &[ListTombstone],
) -> Result<MergeStats> {
    let mut stats = MergeStats::default();
    let Some(remote) = remote else {
        return Ok(stats);
    };

    tx.execute("DELETE FROM list_tombstones", [])?;
    for tb in remote_tombstones {
        record_list_tombstone(tx, tb.entity_id, &tb.deleted_at)?;
    }
    let tombs = load_list_tombstone_index(tx)?;

    let remote_ids: HashSet<i64> = remote.iter().filter_map(|r| r.id).collect();
    for id in all_ids(tx, "todo_lists")? {
        if id != DEFAULT_LIST_ID && !remote_ids.contains(&id) {
            stats.lists_deleted += tx.execute("DELETE FROM todo_lists WHERE id = ?1", [id])? as u32;
        }
    }

    for rl in remote {
        let Some(id) = rl.id else {
            stats.records_skipped += 1;
            continue;
        };
        match &rl.list {
            Some(list) if is_list_suppressed(&tombs, id, &list.updated_at) => {
                // 远端自相矛盾（清单与压制它的墓碑并存）：按墓碑处理
                stats.lists_deleted +=
                    tx.execute("DELETE FROM todo_lists WHERE id = ?1", [id])? as u32;
            }
            Some(list) => {
                let label = format!("list#{id}");
                match load_list_row(tx, id)? {
                    None => {
                        if write_record(tx, &label, |c| insert_list_row(c, list))? {
                            stats.lists_inserted += 1;
                        } else {
                            stats.records_skipped += 1;
                        }
                    }
                    Some(local) if &local != list => {
                        if write_record(tx, &label, |c| update_list_row(c, list))? {
                            stats.lists_updated += 1;
                        } else {
                            stats.records_skipped += 1;
                        }
                    }
                    Some(_) => {}
                }
            }
            None => {
                stats.records_skipped += 1;
                log::warn!(
                    "[sync] 强制拉取：跳过无法识别的远端清单 #{}: {}",
                    id,
                    rl.error.as_deref().unwrap_or("")
                );
            }
        }
    }
    Ok(stats)
}

/// 强制推送（K7 `webdav_force_push`）前的本地准备：让即将上传的本地版本在所有设备上胜出。
///
/// - 远端独有的待办 / 子任务（含无法识别的）→ 生成墓碑（deleted_at = now；远端记录的
///   updatedAt 比 now 还新——另一端时钟偏快或时区配置不同——时取它，保证墓碑压得住）
/// - 远端版本比本地新、或远端墓碑会压制本地记录 → 把本地记录的 updated_at 推到
///   "至少 now、且严格晚于远端那个时间戳"，避免其它设备按 LWW 留下自己的旧版本
/// - 待办的清单归属（契约 §13）：本机未知、远端已知 → 采纳远端的归属（本机没有意见，"本地覆盖远端"不该丢掉它）；
///   本机已知、远端的更新 → 把本机归属的版本推到"至少 now、且严格晚于远端那个版本"，让本机的归属胜出。
///   畸形的远端记录不参与
///
/// 返回被推进 updated_at 的记录数。
pub fn force_push_prepare(
    tx: &mut Transaction<'_>,
    remote_todos: &[Value],
    remote_tombstones: &[Tombstone],
    now: &str,
) -> Result<u32> {
    let local_todos = id_updated_map(tx, "todos")?;
    let local_subtasks = id_updated_map(tx, "subtasks")?;
    let local_assignments = load_assignments(tx)?;

    // (类型, id) → 需要超过的远端时间戳
    let mut must_beat: HashMap<(EntityKind, i64), String> = HashMap::new();
    let mut note = |key: (EntityKind, i64), ts: String| {
        let entry = must_beat.entry(key).or_default();
        if ts > *entry {
            *entry = ts;
        }
    };

    for rv in remote_todos {
        if let Some(id) = value_id(rv) {
            match local_todos.get(&id) {
                None => record_tombstone(
                    tx,
                    EntityKind::Todo,
                    id,
                    &deletion_time(now, &value_updated_at(rv)),
                )?,
                Some(local) => {
                    let remote_ts = value_updated_at(rv);
                    if remote_ts > *local {
                        note((EntityKind::Todo, id), remote_ts);
                    }
                    if let (Some(remote), Some(local)) =
                        (ListAssignment::from_value(rv), local_assignments.get(&id))
                    {
                        if local.list_id.is_none() {
                            set_assignment(tx, id, &remote)?;
                        } else if remote.is_newer_than(local) {
                            let beat = remote.list_updated_at.as_deref().unwrap_or("");
                            let ours = ListAssignment {
                                list_id: local.list_id,
                                list_updated_at: Some(superseding(now, beat)),
                            };
                            set_assignment(tx, id, &ours)?;
                        }
                    }
                }
            }
        }
        for sv in value_subtasks(rv) {
            let Some(sid) = value_id(sv) else { continue };
            match local_subtasks.get(&sid) {
                None => record_tombstone(
                    tx,
                    EntityKind::Subtask,
                    sid,
                    &deletion_time(now, &value_updated_at(sv)),
                )?,
                Some(local) => {
                    let remote_ts = value_updated_at(sv);
                    if remote_ts > *local {
                        note((EntityKind::Subtask, sid), remote_ts);
                    }
                }
            }
        }
    }
    for tb in remote_tombstones {
        let Some(kind) = EntityKind::parse(&tb.entity_type) else {
            continue;
        };
        let local = match kind {
            EntityKind::Todo => local_todos.get(&tb.entity_id),
            EntityKind::Subtask => local_subtasks.get(&tb.entity_id),
        };
        if local.is_some_and(|l| tb.deleted_at >= *l) {
            note((kind, tb.entity_id), tb.deleted_at.clone());
        }
    }

    let mut bumped = 0u32;
    for ((kind, id), beat) in must_beat {
        let target = superseding(now, &beat);
        let table = match kind {
            EntityKind::Todo => "todos",
            EntityKind::Subtask => "subtasks",
        };
        bumped += tx.execute(
            &format!("UPDATE {table} SET updated_at = ?1 WHERE id = ?2"),
            params![target, id],
        )? as u32;
        // 本地若有同键墓碑（理论上不会），一并清掉，免得自己压制自己
        remove_tombstone(tx, kind, id)?;
    }
    Ok(bumped)
}

/// 强制推送前的清单准备（契约 §5，同 [`force_push_prepare`]）：
///
/// - 远端独有的清单（含无法识别的；默认清单除外）→ 写清单墓碑（`deletedAt = max(now, 远端 updatedAt)`）
/// - 远端版本比本地新、或远端墓碑会压制本地清单 → 把本地清单的 `updated_at` 推到"至少 now、且严格晚于远端那个时间戳"
///
/// `remote_lists = None`（远端没有 `todoLists`）时只看远端清单墓碑。返回被推进 `updated_at` 的清单数
pub fn force_push_prepare_lists(
    tx: &mut Transaction<'_>,
    remote_lists: Option<&[Value]>,
    remote_tombstones: &[ListTombstone],
    now: &str,
) -> Result<u32> {
    let local = id_updated_map(tx, "todo_lists")?;
    let mut must_beat: HashMap<i64, String> = HashMap::new();
    let mut note = |id: i64, ts: String| {
        let entry = must_beat.entry(id).or_default();
        if ts > *entry {
            *entry = ts;
        }
    };
    for rv in remote_lists.unwrap_or(&[]) {
        let Some(id) = value_id(rv) else { continue };
        let remote_ts = value_updated_at(rv);
        match local.get(&id) {
            None => record_list_tombstone(tx, id, &deletion_time(now, &remote_ts))?,
            Some(local_ts) => {
                if remote_ts > *local_ts {
                    note(id, remote_ts);
                }
            }
        }
    }
    for tb in remote_tombstones {
        if local
            .get(&tb.entity_id)
            .is_some_and(|l| tb.deleted_at >= *l)
        {
            note(tb.entity_id, tb.deleted_at.clone());
        }
    }

    let mut bumped = 0u32;
    for (id, beat) in must_beat {
        bumped += tx.execute(
            "UPDATE todo_lists SET updated_at = ?1 WHERE id = ?2",
            params![superseding(now, &beat), id],
        )? as u32;
        remove_list_tombstone(tx, id)?;
    }
    Ok(bumped)
}

fn id_updated_map(conn: &Connection, table: &str) -> Result<HashMap<i64, String>> {
    let mut stmt = conn.prepare(&format!("SELECT id, updated_at FROM {table}"))?;
    let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
    rows.collect()
}

// ============================================================================
// 本地快照
// ============================================================================

/// 读取全部待办（含嵌套子任务）：一次查待办、一次查子任务，按 parent 分组（无 N+1）。
/// `todo_order_by` 为待办的 ORDER BY 子句（代码内常量，不接受外部输入）。
pub fn load_todos_with_subtasks(conn: &Connection, todo_order_by: &str) -> Result<Vec<Todo>> {
    let todo_sql = format!(
        "SELECT {} FROM todos ORDER BY {}",
        TODO_COLUMNS, todo_order_by
    );
    let mut todos: Vec<Todo> = {
        let mut stmt = conn.prepare_cached(&todo_sql)?;
        let rows = stmt.query_map([], todo_from_row)?;
        rows.collect::<Result<_>>()?
    };

    let subtask_sql = format!(
        "SELECT {} FROM subtasks ORDER BY parent_id ASC, sort_order ASC, id ASC",
        SUBTASK_COLUMNS
    );
    let mut by_parent: HashMap<i64, Vec<SubTask>> = HashMap::new();
    {
        let mut stmt = conn.prepare_cached(&subtask_sql)?;
        let rows = stmt.query_map([], subtask_from_row)?;
        for row in rows {
            let sub = row?;
            by_parent.entry(sub.parent_id).or_default().push(sub);
        }
    }
    for todo in &mut todos {
        todo.subtasks = by_parent.remove(&todo.id).unwrap_or_default();
    }
    Ok(todos)
}

/// 同步快照的记录部分
#[derive(Debug, Clone)]
pub struct LocalRecords {
    /// 全部待办（含子任务），时间字段已宽容规范化
    pub todos: Vec<Todo>,
    /// 保留期内的墓碑
    pub tombstones: Vec<Tombstone>,
    /// 全部清单（按 `(sortOrder, id)`），时间字段已宽容规范化
    pub lists: Vec<TodoList>,
    /// 全部清单墓碑（永不过期）
    pub list_tombstones: Vec<ListTombstone>,
    /// 读取快照时的 `local_seq`（与快照在同一把锁内读取）
    pub seq: i64,
}

pub fn local_records(conn: &Connection, cutoff: &str) -> Result<LocalRecords> {
    let mut todos = load_todos_with_subtasks(conn, "sort_order ASC, id ASC")?;
    for t in &mut todos {
        normalize_todo_lenient(t);
    }
    let mut lists = load_lists(conn)?;
    for l in &mut lists {
        normalize_list_lenient(l);
    }
    Ok(LocalRecords {
        todos,
        tombstones: list_tombstones(conn, cutoff)?,
        lists,
        list_tombstones: load_list_tombstones(conn)?,
        seq: local_seq(conn)?,
    })
}

/// 当前墓碑保留期截止时间（规范格式）
pub fn retention_cutoff() -> String {
    days_ago_local(TOMBSTONE_RETENTION_DAYS)
}
