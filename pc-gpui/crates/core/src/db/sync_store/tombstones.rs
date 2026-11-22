//! 墓碑管理：待办 / 子任务（30 天保留）与清单（永不过期，契约 §13-10）。

use super::EntityKind;
use crate::db::models::{ListTombstone, Tombstone, DEFAULT_LIST_ID};
use crate::db::time::{normalize_datetime, DefaultTime};
use rusqlite::{params, Connection, OptionalExtension, Result};
use std::collections::HashMap;

/// 写入墓碑；同键已存在时保留较大的 `deleted_at`
pub fn record_tombstone(
    conn: &Connection,
    kind: EntityKind,
    id: i64,
    deleted_at: &str,
) -> Result<()> {
    conn.execute(
        "INSERT INTO tombstones (entity_type, entity_id, deleted_at) VALUES (?1, ?2, ?3)
         ON CONFLICT(entity_type, entity_id)
         DO UPDATE SET deleted_at = MAX(deleted_at, excluded.deleted_at)",
        params![kind.as_str(), id, deleted_at],
    )?;
    Ok(())
}

pub fn remove_tombstone(conn: &Connection, kind: EntityKind, id: i64) -> Result<()> {
    conn.execute(
        "DELETE FROM tombstones WHERE entity_type = ?1 AND entity_id = ?2",
        params![kind.as_str(), id],
    )?;
    Ok(())
}

/// 列出 `deleted_at >= cutoff` 的墓碑（按类型、id 排序）
pub fn list_tombstones(conn: &Connection, cutoff: &str) -> Result<Vec<Tombstone>> {
    let mut stmt = conn.prepare_cached(
        "SELECT entity_type, entity_id, deleted_at FROM tombstones
         WHERE deleted_at >= ?1 ORDER BY entity_type, entity_id",
    )?;
    let rows = stmt.query_map([cutoff], |r| {
        Ok(Tombstone {
            entity_type: r.get(0)?,
            entity_id: r.get(1)?,
            deleted_at: r.get(2)?,
        })
    })?;
    rows.collect()
}

/// 清理 `deleted_at < cutoff` 的墓碑，返回清理条数
pub fn purge_tombstones_before(conn: &Connection, cutoff: &str) -> Result<usize> {
    conn.execute("DELETE FROM tombstones WHERE deleted_at < ?1", [cutoff])
}

// ============================================================================
// 清单墓碑（v31 `list_tombstones`，同步文档顶层 `listTombstones`）
//
// **永不过期**（契约 §13-10，取代 §4-4 的 30 天）：清单很少，留着它们保证长期离线的设备回来时已删的清单不复活。
// 待办 / 子任务的墓碑仍是 30 天。
// ============================================================================

/// 写入清单墓碑；同键已存在时保留较大的 `deleted_at`。默认清单（id 1）不可删除，它的墓碑一律不存
pub fn record_list_tombstone(conn: &Connection, list_id: i64, deleted_at: &str) -> Result<()> {
    if list_id == DEFAULT_LIST_ID {
        return Ok(());
    }
    conn.execute(
        "INSERT INTO list_tombstones (entity_id, deleted_at) VALUES (?1, ?2)
         ON CONFLICT(entity_id) DO UPDATE SET deleted_at = MAX(deleted_at, excluded.deleted_at)",
        params![list_id, deleted_at],
    )?;
    Ok(())
}

pub fn remove_list_tombstone(conn: &Connection, list_id: i64) -> Result<()> {
    conn.execute(
        "DELETE FROM list_tombstones WHERE entity_id = ?1",
        [list_id],
    )?;
    Ok(())
}

/// 全部清单墓碑（按 id 排序；永不过期，默认清单的不算）
pub fn load_list_tombstones(conn: &Connection) -> Result<Vec<ListTombstone>> {
    let mut stmt = conn.prepare_cached(
        "SELECT entity_id, deleted_at FROM list_tombstones
         WHERE entity_id != ?1 ORDER BY entity_id",
    )?;
    let rows = stmt.query_map([DEFAULT_LIST_ID], |r| {
        Ok(ListTombstone {
            entity_id: r.get(0)?,
            deleted_at: r.get(1)?,
        })
    })?;
    rows.collect()
}

/// 清单墓碑索引：清单 id → deleted_at
pub type ListTombstoneIndex = HashMap<i64, String>;

pub(super) fn load_list_tombstone_index(conn: &Connection) -> Result<ListTombstoneIndex> {
    let mut stmt = conn.prepare_cached("SELECT entity_id, deleted_at FROM list_tombstones")?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?;
    let mut index = ListTombstoneIndex::new();
    for row in rows {
        let (id, deleted_at) = row?;
        if id != DEFAULT_LIST_ID {
            index.insert(id, deleted_at);
        }
    }
    Ok(index)
}

/// 契约 §4-2：存在 `deleted_at >= updated_at` 的清单墓碑即被压制；默认清单永不被压制
pub fn is_list_suppressed(index: &ListTombstoneIndex, id: i64, updated_at: &str) -> bool {
    id != DEFAULT_LIST_ID
        && index
            .get(&id)
            .is_some_and(|deleted_at| deleted_at.as_str() >= updated_at)
}

/// 把清单墓碑应用到本地：删除 `updated_at <= deleted_at` 的清单（默认清单除外）。其下待办不改写——
/// 读侧按默认清单显示它们（契约 §3）。返回删除的清单数
pub(super) fn apply_list_tombstones_to_local(conn: &Connection) -> Result<u32> {
    Ok(conn.execute(
        "DELETE FROM todo_lists WHERE id != ?1 AND id IN (
             SELECT l.id FROM todo_lists l
             JOIN list_tombstones t ON t.entity_id = l.id
             WHERE t.deleted_at >= l.updated_at)",
        [DEFAULT_LIST_ID],
    )? as u32)
}

/// 删除墓碑时间：至少是现在，且不早于记录自己的 updated_at（防止另一端时钟偏快时，
/// 记录的 updated_at 比"现在"还新，导致删除压不住、下次同步又被复活）。
///
/// 只认能识别的记录时间：以前直接按字符串比较，乱码时间（如 `"zzz"`）"大于"任何合法时间，
/// 会被原样写进墓碑表——之后既清理不掉、又会压制同 id 的一切版本，远端还解析不了这条墓碑。
pub(crate) fn deletion_time(now: &str, record_updated_at: &str) -> String {
    match normalize_datetime(record_updated_at, DefaultTime::StartOfDay) {
        Some(at) if at.as_str() > now => at,
        _ => now.to_string(),
    }
}

/// 删除待办及其全部子任务并写墓碑（R1-3）。待办不存在返回 `Ok(false)`。
/// 调用方负责事务。
pub fn delete_todo_with_tombstones(conn: &Connection, todo_id: i64, now: &str) -> Result<bool> {
    let todo_updated_at: Option<String> = conn
        .query_row(
            "SELECT updated_at FROM todos WHERE id = ?1",
            [todo_id],
            |r| r.get(0),
        )
        .optional()?;
    let Some(todo_updated_at) = todo_updated_at else {
        return Ok(false);
    };

    let subtasks: Vec<(i64, String)> = {
        let mut stmt =
            conn.prepare_cached("SELECT id, updated_at FROM subtasks WHERE parent_id = ?1")?;
        let rows = stmt.query_map([todo_id], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect::<Result<_>>()?
    };
    for (sub_id, sub_updated_at) in &subtasks {
        record_tombstone(
            conn,
            EntityKind::Subtask,
            *sub_id,
            &deletion_time(now, sub_updated_at),
        )?;
    }
    record_tombstone(
        conn,
        EntityKind::Todo,
        todo_id,
        &deletion_time(now, &todo_updated_at),
    )?;

    conn.execute("DELETE FROM subtasks WHERE parent_id = ?1", [todo_id])?;
    conn.execute("DELETE FROM todos WHERE id = ?1", [todo_id])?;
    Ok(true)
}

/// 删除子任务并写墓碑。子任务不存在返回 `Ok(false)`。调用方负责事务。
pub fn delete_subtask_with_tombstone(
    conn: &Connection,
    subtask_id: i64,
    now: &str,
) -> Result<bool> {
    let updated_at: Option<String> = conn
        .query_row(
            "SELECT updated_at FROM subtasks WHERE id = ?1",
            [subtask_id],
            |r| r.get(0),
        )
        .optional()?;
    let Some(updated_at) = updated_at else {
        return Ok(false);
    };
    record_tombstone(
        conn,
        EntityKind::Subtask,
        subtask_id,
        &deletion_time(now, &updated_at),
    )?;
    conn.execute("DELETE FROM subtasks WHERE id = ?1", [subtask_id])?;
    Ok(true)
}

/// 墓碑索引：(类型, id) → deleted_at
pub type TombstoneIndex = HashMap<(EntityKind, i64), String>;

pub(super) fn load_tombstone_index(conn: &Connection) -> Result<TombstoneIndex> {
    let mut stmt =
        conn.prepare_cached("SELECT entity_type, entity_id, deleted_at FROM tombstones")?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, i64>(1)?,
            r.get::<_, String>(2)?,
        ))
    })?;
    let mut index = TombstoneIndex::new();
    for row in rows {
        let (kind, id, deleted_at) = row?;
        if let Some(kind) = EntityKind::parse(&kind) {
            index.insert((kind, id), deleted_at);
        }
    }
    Ok(index)
}

/// K3-2：存在 `deleted_at >= updated_at` 的墓碑即被压制
pub fn is_suppressed(index: &TombstoneIndex, kind: EntityKind, id: i64, updated_at: &str) -> bool {
    index
        .get(&(kind, id))
        .is_some_and(|deleted_at| deleted_at.as_str() >= updated_at)
}

/// 把墓碑应用到本地记录：删除 `updated_at <= deleted_at` 的待办（连带其子任务）与子任务。
/// 返回 (删除的待办数, 删除的子任务数)。
pub(super) fn apply_tombstones_to_local(conn: &Connection) -> Result<(u32, u32)> {
    let doomed_todos: Vec<i64> = {
        let mut stmt = conn.prepare_cached(
            "SELECT t.id FROM todos t
             JOIN tombstones tb ON tb.entity_type = 'todo' AND tb.entity_id = t.id
             WHERE tb.deleted_at >= t.updated_at",
        )?;
        let rows = stmt.query_map([], |r| r.get(0))?;
        rows.collect::<Result<_>>()?
    };
    let mut todos_deleted = 0u32;
    let mut subtasks_deleted = 0u32;
    for id in doomed_todos {
        subtasks_deleted += conn.execute("DELETE FROM subtasks WHERE parent_id = ?1", [id])? as u32;
        todos_deleted += conn.execute("DELETE FROM todos WHERE id = ?1", [id])? as u32;
    }
    subtasks_deleted += conn.execute(
        "DELETE FROM subtasks WHERE id IN (
             SELECT s.id FROM subtasks s
             JOIN tombstones tb ON tb.entity_type = 'subtask' AND tb.entity_id = s.id
             WHERE tb.deleted_at >= s.updated_at)",
        [],
    )? as u32;
    Ok((todos_deleted, subtasks_deleted))
}
