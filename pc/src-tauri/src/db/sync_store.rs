//! 同步在 SQLite 侧的存储与合并（K2 / K3）。
//!
//! - `sync_meta`：本地变更计数 `local_seq`（触发器递增）与最近一次成功同步时的 `synced_seq`
//! - `tombstones`：删除墓碑，30 天保留
//! - 远端记录合并：记录级 LWW（平局保留本地）+ 墓碑压制 + 并集，**没有**"缺席即删除"
//! - 强制拉取 / 强制推送的本地侧操作
//! - 本地快照（一次查询待办、一次查询子任务，无 N+1）
//!
//! 网络与 JSON 文档拼装在 `commands::sync_cmd`；本模块只认已解析的记录和 SQLite。

use super::models::{SubTask, Todo, Tombstone};
use super::time::{normalize_datetime, plus_one_second, DefaultTime};
use super::{subtask_from_row, todo_from_row, SUBTASK_COLUMNS, TODO_COLUMNS};
use rusqlite::{params, Connection, ErrorCode, OptionalExtension, Result, Transaction};
use serde_json::Value;
use std::collections::{HashMap, HashSet};

/// 墓碑保留天数（K2）
pub const TOMBSTONE_RETENTION_DAYS: i64 = 30;

/// 墓碑 / 记录的实体类型
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum EntityKind {
    Todo,
    Subtask,
}

impl EntityKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Todo => "todo",
            Self::Subtask => "subtask",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "todo" => Some(Self::Todo),
            "subtask" => Some(Self::Subtask),
            _ => None,
        }
    }
}

// ============================================================================
// sync_meta
// ============================================================================

fn get_meta(conn: &Connection, key: &str) -> Result<i64> {
    conn.query_row("SELECT value FROM sync_meta WHERE key = ?1", [key], |r| {
        r.get(0)
    })
    .optional()
    .map(|v| v.unwrap_or(0))
}

/// 本地变更计数：todos / subtasks 任意增删改都会递增（v28 触发器）
pub fn local_seq(conn: &Connection) -> Result<i64> {
    get_meta(conn, "local_seq")
}

/// 最近一次成功同步时的 `local_seq`
pub fn synced_seq(conn: &Connection) -> Result<i64> {
    get_meta(conn, "synced_seq")
}

pub fn set_synced_seq(conn: &Connection, value: i64) -> Result<()> {
    conn.execute(
        "INSERT INTO sync_meta (key, value) VALUES ('synced_seq', ?1)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        [value],
    )?;
    Ok(())
}

// ============================================================================
// 墓碑
// ============================================================================

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

/// 删除墓碑时间：至少是现在，且不早于记录自己的 updated_at（防止另一端时钟偏快时，
/// 记录的 updated_at 比"现在"还新，导致删除压不住、下次同步又被复活）
fn deletion_time(now: &str, record_updated_at: &str) -> String {
    if record_updated_at > now {
        record_updated_at.to_string()
    } else {
        now.to_string()
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

fn load_tombstone_index(conn: &Connection) -> Result<TombstoneIndex> {
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
fn apply_tombstones_to_local(conn: &Connection) -> Result<(u32, u32)> {
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

// ============================================================================
// JSON 记录工具（远端文档是 serde_json::Value，字段名 camelCase）
// ============================================================================

/// 取记录的 `id`：整数，或可解析为整数的字符串
pub fn value_id(v: &Value) -> Option<i64> {
    match v.get("id")? {
        Value::Number(n) => n.as_i64().or_else(|| {
            n.as_f64()
                .filter(|f| f.fract() == 0.0 && f.abs() < 9.0e15)
                .map(|f| f as i64)
        }),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

/// 取记录的 `updatedAt`（规范化后；无法识别时原样；缺失时为空串）
pub fn value_updated_at(v: &Value) -> String {
    match v.get("updatedAt").and_then(Value::as_str) {
        Some(raw) => {
            normalize_datetime(raw, DefaultTime::StartOfDay).unwrap_or_else(|| raw.to_string())
        }
        None => String::new(),
    }
}

/// 待办 JSON 里嵌套的子任务数组
pub fn value_subtasks(v: &Value) -> &[Value] {
    v.get("subtasks")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

/// 宽容解析一条远端墓碑；无法识别（类型未知 / id 非整数 / 时间无法解析）返回 `None`
pub fn parse_tombstone(v: &Value) -> Option<Tombstone> {
    let kind = EntityKind::parse(v.get("entityType")?.as_str()?)?;
    let id = match v.get("entityId")? {
        Value::Number(n) => n.as_i64()?,
        Value::String(s) => s.trim().parse().ok()?,
        _ => return None,
    };
    let deleted_at = normalize_datetime(v.get("deletedAt")?.as_str()?, DefaultTime::StartOfDay)?;
    Some(Tombstone {
        entity_type: kind.as_str().to_string(),
        entity_id: id,
        deleted_at,
    })
}

// ============================================================================
// 时间字段规范化
// ============================================================================

fn normalize_required(value: &str, field: &str) -> std::result::Result<String, String> {
    normalize_datetime(value, DefaultTime::StartOfDay)
        .ok_or_else(|| format!("{field} 无法识别: {value:?}"))
}

fn normalize_optional(
    value: Option<String>,
    default: DefaultTime,
    field: &str,
) -> std::result::Result<Option<String>, String> {
    match value {
        None => Ok(None),
        Some(v) if v.trim().is_empty() => Ok(None),
        Some(v) => normalize_datetime(&v, default)
            .map(Some)
            .ok_or_else(|| format!("{field} 无法识别: {v:?}")),
    }
}

/// 严格规范化（合并远端用）：任一时间字段无法识别即报错，该记录按"无法识别"跳过。
/// `createdAt` 缺失时取 `updatedAt`。
pub fn normalize_todo(t: &mut Todo) -> std::result::Result<(), String> {
    t.updated_at = normalize_required(&t.updated_at, "updatedAt")?;
    t.created_at = if t.created_at.trim().is_empty() {
        t.updated_at.clone()
    } else {
        normalize_required(&t.created_at, "createdAt")?
    };
    t.notify_at = normalize_optional(t.notify_at.take(), DefaultTime::Notify, "notifyAt")?;
    t.start_time = normalize_optional(t.start_time.take(), DefaultTime::StartOfDay, "startTime")?;
    t.end_time = normalize_optional(t.end_time.take(), DefaultTime::EndOfDay, "endTime")?;
    Ok(())
}

/// 严格规范化子任务（合并远端用）
pub fn normalize_subtask(s: &mut SubTask) -> std::result::Result<(), String> {
    s.updated_at = normalize_required(&s.updated_at, "updatedAt")?;
    s.created_at = if s.created_at.trim().is_empty() {
        s.updated_at.clone()
    } else {
        normalize_required(&s.created_at, "createdAt")?
    };
    Ok(())
}

fn lenient(value: &mut String, default: DefaultTime) {
    if let Some(n) = normalize_datetime(value, default) {
        *value = n;
    }
}

fn lenient_opt(value: &mut Option<String>, default: DefaultTime) {
    if let Some(v) = value.as_mut() {
        lenient(v, default);
    }
}

/// 宽容规范化（导出 / 上传 / 手动导入用）：能识别的转成规范格式，识别不了的原样保留，
/// 绝不因为格式问题丢本地数据。会连同嵌套子任务一起处理。
pub fn normalize_todo_lenient(t: &mut Todo) {
    lenient(&mut t.created_at, DefaultTime::StartOfDay);
    lenient(&mut t.updated_at, DefaultTime::StartOfDay);
    lenient_opt(&mut t.notify_at, DefaultTime::Notify);
    lenient_opt(&mut t.start_time, DefaultTime::StartOfDay);
    lenient_opt(&mut t.end_time, DefaultTime::EndOfDay);
    for s in &mut t.subtasks {
        lenient(&mut s.created_at, DefaultTime::StartOfDay);
        lenient(&mut s.updated_at, DefaultTime::StartOfDay);
    }
}

// ============================================================================
// 远端记录解析
// ============================================================================

/// 一条远端待办：`todo` 为 `None` 表示无法识别（类型错误 / 缺字段 / 时间格式错误）
#[derive(Debug, Clone)]
pub struct RemoteTodo {
    pub id: Option<i64>,
    pub todo: Option<Todo>,
    /// 规范化后的 updatedAt（无法识别时为原始串），无法解析的记录也用它判断墓碑压制
    pub updated_at: String,
    pub subtasks: Vec<RemoteSubtask>,
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct RemoteSubtask {
    pub id: Option<i64>,
    pub subtask: Option<SubTask>,
    pub error: Option<String>,
}

/// 逐条解析远端 `todos` 数组。单条失败不影响其它条（K3-6）：
/// 待办本体与子任务分开解析，子任务的 `parentId` 一律以外层待办的 id 为准。
pub fn parse_remote_todos(values: &[Value]) -> Vec<RemoteTodo> {
    values.iter().map(parse_remote_todo).collect()
}

fn parse_remote_todo(v: &Value) -> RemoteTodo {
    let id = value_id(v);
    let updated_at = value_updated_at(v);

    let Some(obj) = v.as_object() else {
        return RemoteTodo {
            id,
            todo: None,
            updated_at,
            subtasks: Vec::new(),
            error: Some("不是 JSON 对象".to_string()),
        };
    };

    let mut shell = obj.clone();
    shell.remove("subtasks");
    let (todo, error) = match serde_json::from_value::<Todo>(Value::Object(shell)) {
        Ok(mut t) => match normalize_todo(&mut t) {
            Ok(()) => (Some(t), None),
            Err(e) => (None, Some(e)),
        },
        Err(e) => (None, Some(e.to_string())),
    };

    let subtasks = value_subtasks(v)
        .iter()
        .map(|sv| parse_remote_subtask(sv, id))
        .collect();

    RemoteTodo {
        id,
        todo,
        updated_at,
        subtasks,
        error,
    }
}

fn parse_remote_subtask(v: &Value, parent_id: Option<i64>) -> RemoteSubtask {
    let id = value_id(v);
    let Some(obj) = v.as_object() else {
        return RemoteSubtask {
            id,
            subtask: None,
            error: Some("不是 JSON 对象".to_string()),
        };
    };
    let Some(parent_id) = parent_id else {
        return RemoteSubtask {
            id,
            subtask: None,
            error: Some("所属待办缺少 id".to_string()),
        };
    };
    let mut obj = obj.clone();
    obj.insert("parentId".to_string(), Value::from(parent_id));
    match serde_json::from_value::<SubTask>(Value::Object(obj)) {
        Ok(mut s) => match normalize_subtask(&mut s) {
            Ok(()) => RemoteSubtask {
                id,
                subtask: Some(s),
                error: None,
            },
            Err(e) => RemoteSubtask {
                id,
                subtask: None,
                error: Some(e),
            },
        },
        Err(e) => RemoteSubtask {
            id,
            subtask: None,
            error: Some(e.to_string()),
        },
    }
}

// ============================================================================
// 行写入（保留远端 / 备份里的 id）
// ============================================================================

pub fn insert_todo_row(conn: &Connection, t: &Todo) -> Result<()> {
    conn.prepare_cached(
        "INSERT INTO todos (id, title, description, color, quadrant,
                            notify_at, notify_before, notified, completed,
                            sort_order, start_time, end_time, created_at, updated_at,
                            repeat_enabled, repeat_type, repeat_interval,
                            repeat_weekdays, repeat_month_day)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14,
                 ?15, ?16, ?17, ?18, ?19)",
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
            repeat_weekdays = ?17, repeat_month_day = ?18
         WHERE id = ?19",
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
    /// 无法识别 / 无法落库而跳过的远端记录数（绝不当作删除）
    pub records_skipped: u32,
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
            eprintln!("[sync] 跳过无法落库的远端记录 {}: {}", what, e);
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

/// K3 记录级合并：把远端待办 / 子任务 / 墓碑合并进本地（调用方提供事务）。
///
/// 1. 远端墓碑（`deleted_at >= cutoff`）并入本地墓碑表，同键取较大时间
/// 2. 墓碑应用到本地：`updated_at <= deleted_at` 的记录删除（待办连带子任务）
/// 3. 逐条 LWW：远端较新 → 覆盖（保留远端 updatedAt）；平局 / 本地较新 → 不动；
///    本地没有 → 以远端 id 插入；被墓碑压制 → 丢弃
/// 4. 仅本地存在的记录保留（不做缺席清理）
/// 5. 无法识别 / 无法落库的远端记录跳过并计数，不影响其它记录
pub fn merge_remote(
    tx: &mut Transaction<'_>,
    remote: &[RemoteTodo],
    remote_tombstones: &[Tombstone],
    cutoff: &str,
) -> Result<MergeStats> {
    let mut stats = MergeStats::default();

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
            eprintln!(
                "[sync] 跳过缺少 id 的远端待办: {}",
                rt.error.as_deref().unwrap_or("")
            );
            continue;
        };

        match &rt.todo {
            Some(todo) => {
                if !is_suppressed(&tombs, EntityKind::Todo, todo_id, &todo.updated_at) {
                    match local_updated_at(tx, EntityKind::Todo, todo_id)? {
                        None => {
                            let label = format!("todo#{todo_id}");
                            if write_record(tx, &label, |c| insert_todo_row(c, todo))? {
                                stats.todos_inserted += 1;
                            } else {
                                stats.records_skipped += 1;
                            }
                        }
                        Some(local) if todo.updated_at > local => {
                            let label = format!("todo#{todo_id}");
                            if write_record(tx, &label, |c| update_todo_row(c, todo))? {
                                stats.todos_updated += 1;
                            } else {
                                stats.records_skipped += 1;
                            }
                        }
                        Some(_) => {}
                    }
                }
            }
            // 看不懂、但已被墓碑压制的远端版本：本来就该删，不算跳过
            None if is_suppressed(&tombs, EntityKind::Todo, todo_id, &rt.updated_at) => {}
            None => {
                stats.records_skipped += 1;
                eprintln!(
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
                    eprintln!(
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
/// - 远端记录无条件覆盖本地（被远端自身墓碑压制的除外）
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
            Some(todo) => match load_todo_row(tx, todo_id)? {
                None => {
                    let label = format!("todo#{todo_id}");
                    if write_record(tx, &label, |c| insert_todo_row(c, todo))? {
                        stats.todos_inserted += 1;
                    } else {
                        stats.records_skipped += 1;
                    }
                }
                Some(local) if &local != todo => {
                    let label = format!("todo#{todo_id}");
                    if write_record(tx, &label, |c| update_todo_row(c, todo))? {
                        stats.todos_updated += 1;
                    } else {
                        stats.records_skipped += 1;
                    }
                }
                Some(_) => {}
            },
            None => {
                stats.records_skipped += 1;
                eprintln!(
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

/// 强制推送（K7 `webdav_force_push`）前的本地准备：让即将上传的本地版本在所有设备上胜出。
///
/// - 远端独有的待办 / 子任务（含无法识别的）→ 生成墓碑（deleted_at = now）
/// - 远端版本比本地新、或远端墓碑会压制本地记录 → 把本地记录的 updated_at 推到
///   "至少 now、且严格晚于远端那个时间戳"，避免其它设备按 LWW 留下自己的旧版本
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
                None => record_tombstone(tx, EntityKind::Todo, id, now)?,
                Some(local) => {
                    let remote_ts = value_updated_at(rv);
                    if remote_ts > *local {
                        note((EntityKind::Todo, id), remote_ts);
                    }
                }
            }
        }
        for sv in value_subtasks(rv) {
            let Some(sid) = value_id(sv) else { continue };
            match local_subtasks.get(&sid) {
                None => record_tombstone(tx, EntityKind::Subtask, sid, now)?,
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
        let target = {
            let after = plus_one_second(&beat);
            if after.as_str() > now {
                after
            } else {
                now.to_string()
            }
        };
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
    /// 读取快照时的 `local_seq`（与快照在同一把锁内读取）
    pub seq: i64,
}

pub fn local_records(conn: &Connection, cutoff: &str) -> Result<LocalRecords> {
    let mut todos = load_todos_with_subtasks(conn, "sort_order ASC, id ASC")?;
    for t in &mut todos {
        normalize_todo_lenient(t);
    }
    Ok(LocalRecords {
        todos,
        tombstones: list_tombstones(conn, cutoff)?,
        seq: local_seq(conn)?,
    })
}

/// 当前墓碑保留期截止时间（规范格式）
pub fn retention_cutoff() -> String {
    super::time::days_ago_local(TOMBSTONE_RETENTION_DAYS)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use serde_json::json;

    const CUTOFF: &str = "2000-01-01 00:00:00";

    fn db() -> Database {
        Database::new_in_memory().expect("内存库")
    }

    fn todo_json(id: i64, title: &str, updated_at: &str) -> Value {
        json!({
            "id": id, "title": title, "description": null, "color": "#EF4444", "quadrant": 1,
            "notifyAt": null, "notifyBefore": 0, "notified": false, "completed": false,
            "sortOrder": 0, "startTime": null, "endTime": null,
            "createdAt": "2026-01-01 00:00:00", "updatedAt": updated_at, "subtasks": []
        })
    }

    fn sub_json(id: i64, parent: i64, title: &str, updated_at: &str) -> Value {
        json!({
            "id": id, "parentId": parent, "title": title, "content": null, "completed": false,
            "sortOrder": 0, "createdAt": "2026-01-01 00:00:00", "updatedAt": updated_at
        })
    }

    fn with_subs(mut todo: Value, subs: Vec<Value>) -> Value {
        todo["subtasks"] = Value::Array(subs);
        todo
    }

    fn merge(db: &Database, todos: Vec<Value>, tombs: Vec<Tombstone>) -> MergeStats {
        let parsed = parse_remote_todos(&todos);
        db.with_transaction(|tx| merge_remote(tx, &parsed, &tombs, CUTOFF))
            .expect("合并失败")
    }

    fn tomb(kind: &str, id: i64, at: &str) -> Tombstone {
        Tombstone {
            entity_type: kind.to_string(),
            entity_id: id,
            deleted_at: at.to_string(),
        }
    }

    fn title(db: &Database, table: &str, id: i64) -> Option<String> {
        db.with_connection(|c| {
            c.query_row(
                &format!("SELECT title FROM {table} WHERE id = ?1"),
                [id],
                |r| r.get(0),
            )
            .optional()
        })
        .unwrap()
    }

    fn count(db: &Database, table: &str) -> i64 {
        db.with_connection(|c| {
            c.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
        })
        .unwrap()
    }

    #[test]
    fn inserts_remote_records_with_their_ids() {
        let db = db();
        let stats = merge(
            &db,
            vec![with_subs(
                todo_json(1715000000000123, "远端", "2026-05-01 10:00:00"),
                vec![sub_json(77, 999, "子", "2026-05-01 10:00:00")],
            )],
            vec![],
        );
        assert_eq!(stats.todos_inserted, 1);
        assert_eq!(stats.subtasks_inserted, 1);
        assert_eq!(
            title(&db, "todos", 1715000000000123).as_deref(),
            Some("远端")
        );
        // parentId 以外层待办为准
        let parent: i64 = db
            .with_connection(|c| {
                c.query_row("SELECT parent_id FROM subtasks WHERE id = 77", [], |r| {
                    r.get(0)
                })
            })
            .unwrap();
        assert_eq!(parent, 1715000000000123);
    }

    #[test]
    fn lww_remote_newer_wins_local_newer_and_ties_keep_local() {
        let db = db();
        merge(&db, vec![todo_json(1, "v1", "2026-05-01 10:00:00")], vec![]);

        // 远端较新 → 覆盖
        let s = merge(&db, vec![todo_json(1, "v2", "2026-05-02 10:00:00")], vec![]);
        assert_eq!(s.todos_updated, 1);
        assert_eq!(title(&db, "todos", 1).as_deref(), Some("v2"));

        // 平局 → 保留本地
        let s = merge(
            &db,
            vec![todo_json(1, "tie", "2026-05-02 10:00:00")],
            vec![],
        );
        assert_eq!(s.todos_updated, 0);
        assert_eq!(title(&db, "todos", 1).as_deref(), Some("v2"));

        // 远端较旧 → 保留本地
        let s = merge(
            &db,
            vec![todo_json(1, "old", "2026-04-01 10:00:00")],
            vec![],
        );
        assert_eq!(s, MergeStats::default());
        assert_eq!(title(&db, "todos", 1).as_deref(), Some("v2"));
    }

    #[test]
    fn lww_compares_normalized_timestamps() {
        let db = db();
        merge(
            &db,
            vec![todo_json(1, "local", "2026-05-01 10:00:00")],
            vec![],
        );
        // 带 T 的同一时刻 = 平局，保留本地
        let s = merge(
            &db,
            vec![todo_json(1, "T 格式", "2026-05-01T10:00:00")],
            vec![],
        );
        assert_eq!(s.todos_updated, 0);
        // 带 T、晚一分钟 → 远端较新
        let s = merge(
            &db,
            vec![todo_json(1, "T 格式新", "2026-05-01T10:01")],
            vec![],
        );
        assert_eq!(s.todos_updated, 1);
        let stored: String = db
            .with_connection(|c| {
                c.query_row("SELECT updated_at FROM todos WHERE id = 1", [], |r| {
                    r.get(0)
                })
            })
            .unwrap();
        assert_eq!(stored, "2026-05-01 10:01:00", "落库前规范化");
    }

    #[test]
    fn absence_on_remote_never_deletes_local_records() {
        let db = db();
        merge(
            &db,
            vec![with_subs(
                todo_json(1, "本地独有", "2026-05-01 10:00:00"),
                vec![sub_json(11, 1, "子", "2026-05-01 10:00:00")],
            )],
            vec![],
        );
        let s = merge(
            &db,
            vec![todo_json(2, "远端", "2026-05-01 10:00:00")],
            vec![],
        );
        assert_eq!(s.todos_deleted + s.subtasks_deleted, 0);
        assert!(title(&db, "todos", 1).is_some());
        assert!(title(&db, "subtasks", 11).is_some());
        assert!(title(&db, "todos", 2).is_some());
    }

    #[test]
    fn remote_tombstone_deletes_older_local_record_and_its_subtasks() {
        let db = db();
        merge(
            &db,
            vec![with_subs(
                todo_json(1, "将被删", "2026-05-01 10:00:00"),
                vec![sub_json(11, 1, "随父删", "2026-05-03 10:00:00")],
            )],
            vec![],
        );
        let s = merge(&db, vec![], vec![tomb("todo", 1, "2026-05-02 10:00:00")]);
        assert_eq!(s.todos_deleted, 1);
        assert_eq!(s.subtasks_deleted, 1);
        assert_eq!(count(&db, "todos"), 0);
        assert_eq!(count(&db, "subtasks"), 0);
        // 墓碑被并入本地，供后续上传
        let tombs = db.with_connection(|c| list_tombstones(c, CUTOFF)).unwrap();
        assert_eq!(tombs, vec![tomb("todo", 1, "2026-05-02 10:00:00")]);
    }

    #[test]
    fn record_edited_after_deletion_survives_tombstone() {
        let db = db();
        merge(
            &db,
            vec![todo_json(1, "删除后又编辑", "2026-05-03 10:00:00")],
            vec![],
        );
        let s = merge(&db, vec![], vec![tomb("todo", 1, "2026-05-02 10:00:00")]);
        assert_eq!(s.todos_deleted, 0);
        assert!(title(&db, "todos", 1).is_some());

        // 远端带着较新的版本 + 较旧的墓碑：插入
        let db2 = self::db();
        let s = merge(
            &db2,
            vec![todo_json(5, "复活", "2026-05-03 10:00:00")],
            vec![tomb("todo", 5, "2026-05-02 10:00:00")],
        );
        assert_eq!(s.todos_inserted, 1);
    }

    #[test]
    fn local_tombstone_suppresses_stale_remote_record() {
        let db = db();
        db.with_transaction(|tx| record_tombstone(tx, EntityKind::Todo, 9, "2026-05-02 10:00:00"))
            .unwrap();
        db.with_transaction(|tx| {
            record_tombstone(tx, EntityKind::Subtask, 21, "2026-05-02 10:00:00")
        })
        .unwrap();
        let s = merge(
            &db,
            vec![
                todo_json(9, "已删除的旧版本", "2026-05-01 10:00:00"),
                with_subs(
                    todo_json(2, "保留", "2026-05-01 10:00:00"),
                    vec![
                        sub_json(21, 2, "已删除子任务", "2026-05-01 10:00:00"),
                        sub_json(22, 2, "保留子任务", "2026-05-01 10:00:00"),
                    ],
                ),
            ],
            vec![],
        );
        assert_eq!(s.todos_inserted, 1);
        assert_eq!(s.subtasks_inserted, 1);
        assert_eq!(s.records_skipped, 0, "被墓碑压制不算跳过");
        assert!(title(&db, "todos", 9).is_none());
        assert!(title(&db, "subtasks", 21).is_none());
        assert!(title(&db, "subtasks", 22).is_some());
    }

    #[test]
    fn bad_records_are_skipped_without_affecting_others() {
        let db = db();
        merge(
            &db,
            vec![todo_json(3, "本地已有", "2026-05-01 10:00:00")],
            vec![],
        );

        let mut bad_type = todo_json(3, "类型错误", "2026-05-09 10:00:00");
        bad_type["quadrant"] = json!("urgent_important");
        let mut bad_time = todo_json(4, "时间错误", "2026-05-09 10:00:00");
        bad_time["notifyAt"] = json!("明天早上");
        let good = with_subs(
            todo_json(5, "正常", "2026-05-09 10:00:00"),
            vec![
                sub_json(51, 5, "正常子任务", "2026-05-09 10:00:00"),
                json!({"id": 52, "title": 123, "updatedAt": "2026-05-09 10:00:00"}),
            ],
        );
        let s = merge(
            &db,
            vec![bad_type, bad_time, json!("not an object"), good],
            vec![],
        );
        assert_eq!(s.todos_inserted, 1);
        assert_eq!(s.subtasks_inserted, 1);
        assert_eq!(s.records_skipped, 4);
        assert_eq!(
            title(&db, "todos", 3).as_deref(),
            Some("本地已有"),
            "绝不当作删除"
        );
        assert!(title(&db, "todos", 4).is_none());
        assert!(title(&db, "todos", 5).is_some());
    }

    #[test]
    fn subtask_of_unparseable_todo_merges_into_existing_parent() {
        let db = db();
        merge(
            &db,
            vec![todo_json(3, "本地父", "2026-05-01 10:00:00")],
            vec![],
        );
        let mut bad_parent = with_subs(
            todo_json(3, "父看不懂", "2026-05-09 10:00:00"),
            vec![sub_json(31, 3, "子能看懂", "2026-05-09 10:00:00")],
        );
        bad_parent["completed"] = json!("yes");
        let s = merge(&db, vec![bad_parent], vec![]);
        assert_eq!(s.records_skipped, 1);
        assert_eq!(s.subtasks_inserted, 1);
        assert_eq!(title(&db, "todos", 3).as_deref(), Some("本地父"));
    }

    #[test]
    fn delete_helpers_write_tombstones() {
        let db = db();
        merge(
            &db,
            vec![with_subs(
                todo_json(1, "父", "2026-05-01 10:00:00"),
                vec![
                    sub_json(11, 1, "a", "2026-05-01 10:00:00"),
                    sub_json(12, 1, "b", "2099-01-01 00:00:00"),
                ],
            )],
            vec![],
        );
        db.with_transaction(|tx| delete_subtask_with_tombstone(tx, 11, "2026-06-01 00:00:00"))
            .unwrap();
        db.with_transaction(|tx| delete_todo_with_tombstones(tx, 1, "2026-06-02 00:00:00"))
            .unwrap();
        assert!(!db
            .with_transaction(|tx| delete_todo_with_tombstones(tx, 1, "2026-06-02 00:00:00"))
            .unwrap());
        assert_eq!(count(&db, "todos"), 0);
        assert_eq!(count(&db, "subtasks"), 0);
        let tombs = db.with_connection(|c| list_tombstones(c, CUTOFF)).unwrap();
        assert_eq!(
            tombs,
            vec![
                tomb("subtask", 11, "2026-06-01 00:00:00"),
                // 记录时间比"现在"还新（另一端时钟偏快）：墓碑取记录时间，保证压得住
                tomb("subtask", 12, "2099-01-01 00:00:00"),
                tomb("todo", 1, "2026-06-02 00:00:00"),
            ]
        );
    }

    #[test]
    fn force_pull_makes_local_equal_remote() {
        let db = db();
        merge(
            &db,
            vec![
                with_subs(
                    todo_json(1, "本地较新", "2026-05-09 10:00:00"),
                    vec![sub_json(11, 1, "本地独有子任务", "2026-05-01 10:00:00")],
                ),
                todo_json(2, "本地独有", "2026-05-01 10:00:00"),
            ],
            vec![],
        );
        db.with_transaction(|tx| record_tombstone(tx, EntityKind::Todo, 99, "2026-05-01 10:00:00"))
            .unwrap();

        let remote = parse_remote_todos(&[todo_json(1, "远端旧版", "2026-05-01 10:00:00")]);
        let s = db
            .with_transaction(|tx| {
                force_pull_replace(
                    tx,
                    &remote,
                    &[tomb("todo", 7, "2026-05-01 10:00:00")],
                    CUTOFF,
                )
            })
            .unwrap();
        assert_eq!(s.todos_updated, 1);
        assert_eq!(s.todos_deleted, 1);
        assert_eq!(s.subtasks_deleted, 1);
        assert_eq!(title(&db, "todos", 1).as_deref(), Some("远端旧版"));
        assert!(title(&db, "todos", 2).is_none());
        let tombs = db.with_connection(|c| list_tombstones(c, CUTOFF)).unwrap();
        assert_eq!(
            tombs,
            vec![tomb("todo", 7, "2026-05-01 10:00:00")],
            "墓碑表替换为远端"
        );
    }

    #[test]
    fn force_push_prepare_tombstones_remote_only_and_bumps_conflicts() {
        let db = db();
        merge(
            &db,
            vec![
                todo_json(1, "本地旧", "2026-05-01 10:00:00"),
                todo_json(2, "本地", "2026-05-01 10:00:00"),
            ],
            vec![],
        );
        let remote = vec![
            todo_json(1, "远端新", "2026-05-05 10:00:00"),
            with_subs(
                todo_json(3, "远端独有", "2026-05-01 10:00:00"),
                vec![sub_json(31, 3, "远端独有子", "2026-05-01 10:00:00")],
            ),
        ];
        let now = "2026-05-03 00:00:00";
        let bumped = db
            .with_transaction(|tx| {
                force_push_prepare(tx, &remote, &[tomb("todo", 2, "2026-05-02 00:00:00")], now)
            })
            .unwrap();
        assert_eq!(bumped, 2);
        let ua = |id: i64| -> String {
            db.with_connection(|c| {
                c.query_row("SELECT updated_at FROM todos WHERE id = ?1", [id], |r| {
                    r.get(0)
                })
            })
            .unwrap()
        };
        assert_eq!(ua(1), "2026-05-05 10:00:01", "必须严格晚于远端版本");
        assert_eq!(ua(2), now, "必须压过远端墓碑");
        let tombs = db.with_connection(|c| list_tombstones(c, CUTOFF)).unwrap();
        assert_eq!(tombs, vec![tomb("subtask", 31, now), tomb("todo", 3, now)]);
    }

    #[test]
    fn load_todos_groups_subtasks_without_n_plus_one() {
        let db = db();
        merge(
            &db,
            vec![
                with_subs(
                    todo_json(1, "a", "2026-05-01 10:00:00"),
                    vec![sub_json(11, 1, "a1", "2026-05-01 10:00:00")],
                ),
                with_subs(
                    todo_json(2, "b", "2026-05-01 10:00:00"),
                    vec![
                        sub_json(21, 2, "b1", "2026-05-01 10:00:00"),
                        sub_json(22, 2, "b2", "2026-05-01 10:00:00"),
                    ],
                ),
            ],
            vec![],
        );
        let todos = db
            .with_connection(|c| load_todos_with_subtasks(c, "id ASC"))
            .unwrap();
        assert_eq!(todos.len(), 2);
        assert_eq!(todos[0].subtasks.len(), 1);
        assert_eq!(todos[1].subtasks.len(), 2);
    }

    #[test]
    fn parse_tombstone_is_lenient_but_validates() {
        assert_eq!(
            parse_tombstone(
                &json!({"entityType": "todo", "entityId": 5, "deletedAt": "2026-05-01T10:00"})
            ),
            Some(tomb("todo", 5, "2026-05-01 10:00:00"))
        );
        assert_eq!(
            parse_tombstone(
                &json!({"entityType": "subtask", "entityId": "6", "deletedAt": "2026-05-01 10:00:00"})
            ),
            Some(tomb("subtask", 6, "2026-05-01 10:00:00"))
        );
        assert_eq!(
            parse_tombstone(
                &json!({"entityType": "note", "entityId": 5, "deletedAt": "2026-05-01"})
            ),
            None
        );
        assert_eq!(
            parse_tombstone(
                &json!({"entityType": "todo", "entityId": "x", "deletedAt": "2026-05-01"})
            ),
            None
        );
        assert_eq!(
            parse_tombstone(
                &json!({"entityType": "todo", "entityId": 5, "deletedAt": "yesterday"})
            ),
            None
        );
    }
}
