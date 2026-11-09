//! 同步在 SQLite 侧的存储与合并（K2 / K3）。
//!
//! - `sync_meta`：本地变更计数 `local_seq`（触发器递增）与最近一次成功同步时的 `synced_seq`
//! - `tombstones`：删除墓碑，30 天保留
//! - 远端记录合并：记录级 LWW（平局保留本地）+ 墓碑压制 + 并集，**没有**"缺席即删除"
//! - 强制拉取 / 强制推送的本地侧操作
//! - 本地快照（一次查询待办、一次查询子任务，无 N+1）
//!
//! 网络与 JSON 文档拼装在 `commands::sync_cmd`；本模块只认已解析的记录和 SQLite。

use super::models::{
    todo_list_from_row, ListTombstone, SubTask, Todo, TodoList, Tombstone, DEFAULT_LIST_ID,
    TODO_LIST_COLUMNS,
};
use super::time::{normalize_datetime, superseding, DefaultTime};
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

// ============================================================================
// 清单墓碑（v31 `list_tombstones`，同步文档顶层 `listTombstones`）
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

/// 列出 `deleted_at >= cutoff` 的清单墓碑（按 id 排序）
pub fn load_list_tombstones(conn: &Connection, cutoff: &str) -> Result<Vec<ListTombstone>> {
    let mut stmt = conn.prepare_cached(
        "SELECT entity_id, deleted_at FROM list_tombstones
         WHERE deleted_at >= ?1 AND entity_id != ?2 ORDER BY entity_id",
    )?;
    let rows = stmt.query_map(params![cutoff, DEFAULT_LIST_ID], |r| {
        Ok(ListTombstone {
            entity_id: r.get(0)?,
            deleted_at: r.get(1)?,
        })
    })?;
    rows.collect()
}

/// 清理 `deleted_at < cutoff` 的清单墓碑（与待办墓碑同一个保留期截止点），返回清理条数
pub fn purge_list_tombstones_before(conn: &Connection, cutoff: &str) -> Result<usize> {
    conn.execute(
        "DELETE FROM list_tombstones WHERE deleted_at < ?1",
        [cutoff],
    )
}

/// 清单墓碑索引：清单 id → deleted_at
pub type ListTombstoneIndex = HashMap<i64, String>;

fn load_list_tombstone_index(conn: &Connection) -> Result<ListTombstoneIndex> {
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
fn apply_list_tombstones_to_local(conn: &Connection) -> Result<u32> {
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

/// 取记录的 `updatedAt`（规范化后）；无法识别 / 缺失时为空串，即"比任何合法时间都旧"，
/// 与 cloud 合并的 `canon_ts` 同一规则（K3 三处一致）。
///
/// 以前无法识别的值按原串参与比较：`"zzz"` 这类乱码在字符串比较里大于任何合法时间，于是
/// LWW 永远选它（本地合法的修改再也传不出去），强制推送还会把它抄进本地记录的 updated_at。
pub fn value_updated_at(v: &Value) -> String {
    v.get("updatedAt")
        .and_then(Value::as_str)
        .and_then(|raw| normalize_datetime(raw, DefaultTime::StartOfDay))
        .unwrap_or_default()
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

/// 宽容解析一条远端清单墓碑（`listTombstones` 的元素）；id 非整数 / 时间无法解析返回 `None`。
/// 默认清单（id 1）的墓碑也返回 `None`：默认清单不可删除，收到一律忽略（不存、不传播）
pub fn parse_list_tombstone(v: &Value) -> Option<ListTombstone> {
    let id = match v.get("entityId")? {
        Value::Number(n) => n.as_i64()?,
        Value::String(s) => s.trim().parse().ok()?,
        _ => return None,
    };
    if id == DEFAULT_LIST_ID {
        return None;
    }
    let deleted_at = normalize_datetime(v.get("deletedAt")?.as_str()?, DefaultTime::StartOfDay)?;
    Some(ListTombstone {
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

/// 严格规范化清单（合并远端用）：`updatedAt` 无法识别即报错（该记录按无法识别跳过），`createdAt` 缺失时取 `updatedAt`
pub fn normalize_list(l: &mut TodoList) -> std::result::Result<(), String> {
    l.updated_at = normalize_required(&l.updated_at, "updatedAt")?;
    l.created_at = if l.created_at.trim().is_empty() {
        l.updated_at.clone()
    } else {
        normalize_required(&l.created_at, "createdAt")?
    };
    Ok(())
}

/// 宽容规范化清单（导出 / 上传 / 手动导入用），规则同 [`normalize_todo_lenient`]
pub fn normalize_list_lenient(l: &mut TodoList) {
    lenient(&mut l.created_at, DefaultTime::StartOfDay);
    lenient(&mut l.updated_at, DefaultTime::StartOfDay);
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
    /// 规范化后的 updatedAt（无法识别 / 缺失时为空串），无法解析的记录也用它判断墓碑压制
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

/// 一条远端清单（`todoLists` 的元素）：`list` 为 `None` 表示无法识别
#[derive(Debug, Clone)]
pub struct RemoteList {
    pub id: Option<i64>,
    pub list: Option<TodoList>,
    /// 规范化后的 updatedAt（无法识别 / 缺失时为空串），无法解析的记录也用它判断墓碑压制
    pub updated_at: String,
    pub error: Option<String>,
}

/// 逐条解析远端 `todoLists` 数组，单条失败不影响其它条（契约 §4-5）
pub fn parse_remote_lists(values: &[Value]) -> Vec<RemoteList> {
    values.iter().map(parse_remote_list).collect()
}

fn parse_remote_list(v: &Value) -> RemoteList {
    let id = value_id(v);
    let updated_at = value_updated_at(v);
    if !v.is_object() {
        return RemoteList {
            id,
            list: None,
            updated_at,
            error: Some("不是 JSON 对象".to_string()),
        };
    }
    let (list, error) = match serde_json::from_value::<TodoList>(v.clone()) {
        Ok(mut l) => match normalize_list(&mut l) {
            Ok(()) => (Some(l), None),
            Err(e) => (None, Some(e)),
        },
        Err(e) => (None, Some(e.to_string())),
    };
    RemoteList {
        id,
        list,
        updated_at,
        error,
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
                            repeat_weekdays, repeat_month_day, list_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14,
                 ?15, ?16, ?17, ?18, ?19, ?20)",
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
            repeat_weekdays = ?17, repeat_month_day = ?18, list_id = ?19
         WHERE id = ?20",
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

/// 本地待办的 (updated_at, list_id)；`list_id` 为 `None` 表示归属未知
fn local_todo_version(conn: &Connection, id: i64) -> Result<Option<(String, Option<i64>)>> {
    conn.prepare_cached("SELECT updated_at, list_id FROM todos WHERE id = ?1")?
        .query_row([id], |r| Ok((r.get(0)?, r.get(1)?)))
        .optional()
}

/// 契约 §12-3：本地归属未知时采纳远端已知的 `listId`——只改 `list_id`，**不改** `updated_at`
/// （远端本来就有这个值，这不是本地的新版本）。返回是否改了
fn adopt_list_id(conn: &Connection, id: i64, list_id: i64) -> Result<bool> {
    Ok(conn
        .prepare_cached("UPDATE todos SET list_id = ?1 WHERE id = ?2 AND list_id IS NULL")?
        .execute(params![list_id, id])?
        > 0)
}

/// K3 记录级合并：把远端待办 / 子任务 / 墓碑合并进本地（调用方提供事务）。
///
/// 1. 远端墓碑（`deleted_at >= cutoff`）并入本地墓碑表，同键取较大时间
/// 2. 墓碑应用到本地：`updated_at <= deleted_at` 的记录删除（待办连带子任务）
/// 3. 逐条 LWW：远端较新 → 覆盖（保留远端 updatedAt）；平局 / 本地较新 → 不动；
///    本地没有 → 以远端 id 插入；被墓碑压制 → 丢弃
/// 4. 仅本地存在的记录保留（不做缺席清理）
/// 5. 无法识别 / 无法落库的远端记录跳过并计数，不影响其它记录
/// 6. 待办的清单归属（契约 §12，`listId` 缺失 = 未知）：远端胜出但没有 `listId` → 沿用本地的归属；
///    远端有 `listId`、本地未知 → 不论 LWW 谁胜（含平局、本地较新）都采纳远端的值且不改 `updated_at`。
///    采纳计入 `todos_updated`（本地数据变了，界面要刷新）；它让 `local_seq` 加一，但本次同步收尾时 `synced_seq`
///    取合并之后的值，不会因此多上传一次
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
            log::warn!(
                "[sync] 跳过缺少 id 的远端待办: {}",
                rt.error.as_deref().unwrap_or("")
            );
            continue;
        };

        match &rt.todo {
            Some(todo) => {
                if !is_suppressed(&tombs, EntityKind::Todo, todo_id, &todo.updated_at) {
                    let label = format!("todo#{todo_id}");
                    match local_todo_version(tx, todo_id)? {
                        None => {
                            if write_record(tx, &label, |c| insert_todo_row(c, todo))? {
                                stats.todos_inserted += 1;
                            } else {
                                stats.records_skipped += 1;
                            }
                        }
                        Some((local_at, local_list)) if todo.updated_at > local_at => {
                            // 远端胜出但不认识清单（没有 listId）：沿用本地的归属，未知不覆盖已知
                            let filled;
                            let incoming = if todo.list_id.is_none() && local_list.is_some() {
                                filled = Todo {
                                    list_id: local_list,
                                    ..todo.clone()
                                };
                                &filled
                            } else {
                                todo
                            };
                            if write_record(tx, &label, |c| update_todo_row(c, incoming))? {
                                stats.todos_updated += 1;
                            } else {
                                stats.records_skipped += 1;
                            }
                        }
                        Some((_, None)) => {
                            // 平局 / 本地较新、本地归属未知：采纳远端已知的 listId（不改 updated_at）
                            if let Some(list_id) = todo.list_id {
                                if adopt_list_id(tx, todo_id, list_id)? {
                                    stats.todos_updated += 1;
                                }
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
/// 1. 远端清单墓碑（保留期内；默认清单的在解析时已丢弃）并入本地，同键取较大时间
/// 2. 墓碑应用到本地：`updated_at <= deleted_at` 的清单删除（默认清单永远保留）；其下待办不改写，读侧按默认清单显示
/// 3. `remote = None`（远端文档没有 `todoLists` 键：旧版本写入方 / 还没有新版本写过）：视为远端没有清单信息，只做 1、2
/// 4. 逐条 LWW：远端较新 → 覆盖；平局 / 本地较新 → 不动；本地没有 → 以远端 id 插入；被墓碑压制 → 丢弃
/// 5. 仅本地存在的清单保留（不做缺席清理）；同步造成的重名不处理
/// 6. 无法识别 / 无法落库的远端清单跳过并计数，不影响其它记录
pub fn merge_remote_lists(
    tx: &mut Transaction<'_>,
    remote: Option<&[RemoteList]>,
    remote_tombstones: &[ListTombstone],
    cutoff: &str,
) -> Result<MergeStats> {
    let mut stats = MergeStats::default();
    for tb in remote_tombstones {
        if tb.deleted_at.as_str() >= cutoff {
            record_list_tombstone(tx, tb.entity_id, &tb.deleted_at)?;
        }
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
                // 远端不认识清单（没有 listId）：保留本地已知的归属——未知不覆盖已知（契约 §12）
                let filled;
                let incoming = match &local {
                    Some(l) if todo.list_id.is_none() && l.list_id.is_some() => {
                        filled = Todo {
                            list_id: l.list_id,
                            ..todo.clone()
                        };
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
/// - 本地清单墓碑替换为远端的（保留期内）
/// - 删除本地独有的清单（不写墓碑；其下待办靠读侧兜底显示在默认清单）；默认清单永远保留
/// - 远端清单无条件覆盖本地（被远端自身墓碑压制的除外）；无法识别的跳过计数，同 id 的本地记录保留
pub fn force_pull_replace_lists(
    tx: &mut Transaction<'_>,
    remote: Option<&[RemoteList]>,
    remote_tombstones: &[ListTombstone],
    cutoff: &str,
) -> Result<MergeStats> {
    let mut stats = MergeStats::default();
    let Some(remote) = remote else {
        return Ok(stats);
    };

    tx.execute("DELETE FROM list_tombstones", [])?;
    for tb in remote_tombstones {
        if tb.deleted_at.as_str() >= cutoff {
            record_list_tombstone(tx, tb.entity_id, &tb.deleted_at)?;
        }
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
/// - 本地归属未知、远端同一条已知 → 采纳远端的 `listId`（只改归属；"本地覆盖远端"不该丢掉本机本来就不知道的信息）
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
                    // 本机归属未知、远端已知：先采纳（契约 §12-3 同理），覆盖远端时不把它的已知值丢掉
                    if let Some(list_id) = rv.get("listId").and_then(Value::as_i64) {
                        adopt_list_id(tx, id, list_id)?;
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
    /// 保留期内的清单墓碑
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
        list_tombstones: load_list_tombstones(conn, cutoff)?,
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

    /// 回归：远端记录的 updatedAt 是乱码时，强制推送以前会把它当成"更新的版本"，
    /// 用 plus_one_second（乱码原样返回）把乱码抄进本地记录的 updated_at——此后这条记录在
    /// 每一次 LWW 里都赢、其它写入方又都解析不了它。乱码时间现在视为最旧：不推进、不进墓碑。
    #[test]
    fn force_push_prepare_ignores_unparseable_remote_timestamps() {
        let db = db();
        merge(
            &db,
            vec![todo_json(1, "本地", "2026-05-01 10:00:00")],
            vec![],
        );
        let mut garbage_shared = todo_json(1, "远端乱码时间", "2026-05-01 10:00:00");
        garbage_shared["updatedAt"] = json!("zzz-not-a-time");
        let mut garbage_only = todo_json(2, "远端独有乱码时间", "2026-05-01 10:00:00");
        garbage_only["updatedAt"] = json!("zzz-not-a-time");
        let now = "2026-05-03 00:00:00";

        let bumped = db
            .with_transaction(|tx| {
                force_push_prepare(tx, &[garbage_shared, garbage_only], &[], now)
            })
            .unwrap();
        assert_eq!(bumped, 0, "乱码时间不算更新的版本");
        let local: String = db
            .with_connection(|c| {
                c.query_row("SELECT updated_at FROM todos WHERE id = 1", [], |r| {
                    r.get(0)
                })
            })
            .unwrap();
        assert_eq!(local, "2026-05-01 10:00:00");
        let tombs = db.with_connection(|c| list_tombstones(c, CUTOFF)).unwrap();
        assert_eq!(tombs, vec![tomb("todo", 2, now)], "墓碑时间只取合法时间");
    }

    #[test]
    fn deletion_time_only_trusts_parseable_record_times() {
        let now = "2026-05-03 00:00:00";
        assert_eq!(deletion_time(now, "2026-05-01 10:00:00"), now);
        assert_eq!(
            deletion_time(now, "2099-01-01T08:00"),
            "2099-01-01 08:00:00",
            "比现在还新的合法时间：取它（规范化后）"
        );
        for bad in ["zzz", "", "not a time"] {
            assert_eq!(deletion_time(now, bad), now, "{bad:?}");
        }

        // 本地记录带着乱码时间被删除：墓碑落规范时间
        let db = db();
        db.with_connection(|c| {
            c.execute(
                "INSERT INTO todos (id, title, updated_at) VALUES (5, 'x', 'zzz')",
                [],
            )
            .map(|_| ())
        })
        .unwrap();
        db.with_transaction(|tx| delete_todo_with_tombstones(tx, 5, now))
            .unwrap();
        let tombs = db.with_connection(|c| list_tombstones(c, CUTOFF)).unwrap();
        assert_eq!(tombs, vec![tomb("todo", 5, now)]);
    }

    #[test]
    fn value_updated_at_treats_garbage_as_oldest() {
        assert_eq!(
            value_updated_at(&json!({"updatedAt": "2026-05-01T10:00"})),
            "2026-05-01 10:00:00"
        );
        for v in [
            json!({"updatedAt": "zzz"}),
            json!({"updatedAt": 20260501}),
            json!({}),
            json!("not an object"),
        ] {
            assert_eq!(value_updated_at(&v), "", "{v}");
        }
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

    // ---- 清单（v31，契约 §4 / §5）----

    const WORK: i64 = 1_731_000_000_000_123;
    const LIFE: i64 = 1_731_000_000_000_456;

    fn list_json(id: i64, name: &str, updated_at: &str) -> Value {
        json!({"id": id, "name": name, "sortOrder": 1,
               "createdAt": "2026-01-01 00:00:00", "updatedAt": updated_at})
    }

    fn list_tomb(id: i64, at: &str) -> ListTombstone {
        ListTombstone {
            entity_id: id,
            deleted_at: at.to_string(),
        }
    }

    fn merge_lists(
        db: &Database,
        lists: Option<Vec<Value>>,
        tombs: Vec<ListTombstone>,
    ) -> MergeStats {
        let parsed = lists.as_deref().map(parse_remote_lists);
        db.with_transaction(|tx| merge_remote_lists(tx, parsed.as_deref(), &tombs, CUTOFF))
            .expect("合并清单失败")
    }

    fn list_name(db: &Database, id: i64) -> Option<String> {
        db.with_connection(|c| {
            c.query_row("SELECT name FROM todo_lists WHERE id = ?1", [id], |r| {
                r.get(0)
            })
            .optional()
        })
        .unwrap()
    }

    fn list_tombs(db: &Database) -> Vec<ListTombstone> {
        db.with_connection(|c| load_list_tombstones(c, CUTOFF))
            .unwrap()
    }

    #[test]
    fn remote_lists_merge_by_lww_and_ties_keep_local() {
        let db = db();
        let s = merge_lists(
            &db,
            Some(vec![list_json(WORK, "工作", "2026-05-01 10:00:00")]),
            vec![],
        );
        assert_eq!(s.lists_inserted, 1);
        assert_eq!(list_name(&db, WORK).as_deref(), Some("工作"));

        // 远端较新 → 覆盖
        let s = merge_lists(
            &db,
            Some(vec![list_json(WORK, "工作 2", "2026-05-02 10:00:00")]),
            vec![],
        );
        assert_eq!(s.lists_updated, 1);
        assert_eq!(list_name(&db, WORK).as_deref(), Some("工作 2"));
        // 平局 / 远端较旧 → 保留本地
        for (name, at) in [
            ("平局", "2026-05-02 10:00:00"),
            ("旧", "2026-04-01 10:00:00"),
        ] {
            let s = merge_lists(&db, Some(vec![list_json(WORK, name, at)]), vec![]);
            assert_eq!(s, MergeStats::default());
            assert_eq!(list_name(&db, WORK).as_deref(), Some("工作 2"));
        }

        // 默认清单：迁移种子是固定的 2000-01-01，任何一次改名都胜出；本地改过名（更新）则本地胜出
        let s = merge_lists(
            &db,
            Some(vec![list_json(1, "收件箱", "2026-05-03 10:00:00")]),
            vec![],
        );
        assert_eq!(s.lists_updated, 1);
        assert_eq!(list_name(&db, 1).as_deref(), Some("收件箱"));
        let s = merge_lists(
            &db,
            Some(vec![list_json(1, "待办清单", "2000-01-01 00:00:00")]),
            vec![],
        );
        assert_eq!(s.lists_updated, 0, "另一台设备没改过名的 id 1 不会抢");
        assert_eq!(list_name(&db, 1).as_deref(), Some("收件箱"));
    }

    #[test]
    fn local_only_lists_survive_and_a_remote_without_list_info_deletes_nothing() {
        let db = db();
        merge_lists(
            &db,
            Some(vec![list_json(WORK, "本地独有", "2026-05-01 10:00:00")]),
            vec![],
        );
        let s = merge_lists(
            &db,
            Some(vec![list_json(LIFE, "远端", "2026-05-01 10:00:00")]),
            vec![],
        );
        assert_eq!((s.lists_inserted, s.lists_deleted), (1, 0), "并集");
        // 远端文档没有 todoLists 键（旧版本写入方）：本地清单一条都不删
        let s = merge_lists(&db, None, vec![]);
        assert_eq!(s, MergeStats::default());
        assert!(list_name(&db, WORK).is_some() && list_name(&db, LIFE).is_some());
        // 有 todoLists 但是空数组：同样不删（没有"缺席即删除"）
        let s = merge_lists(&db, Some(vec![]), vec![]);
        assert_eq!(s.lists_deleted, 0);
        assert_eq!(count(&db, "todo_lists"), 3);
    }

    #[test]
    fn list_tombstones_delete_older_lists_but_not_ones_edited_after_deletion() {
        let db = db();
        merge_lists(
            &db,
            Some(vec![
                list_json(WORK, "将被删", "2026-05-01 10:00:00"),
                list_json(LIFE, "删除后又改", "2026-05-05 10:00:00"),
            ]),
            vec![],
        );
        db.with_connection(|c| {
            c.execute(
                "INSERT INTO todos (id, title, list_id) VALUES (1, 't', ?1)",
                [WORK],
            )
        })
        .unwrap();

        let s = merge_lists(
            &db,
            None,
            vec![
                list_tomb(WORK, "2026-05-02 10:00:00"),
                list_tomb(LIFE, "2026-05-02 10:00:00"),
            ],
        );
        assert_eq!(s.lists_deleted, 1);
        assert!(list_name(&db, WORK).is_none());
        assert!(
            list_name(&db, LIFE).is_some(),
            "updatedAt > deletedAt：保留"
        );
        let list_id: i64 = db
            .with_connection(|c| {
                c.query_row("SELECT list_id FROM todos WHERE id = 1", [], |r| r.get(0))
            })
            .unwrap();
        assert_eq!(
            list_id, WORK,
            "收到清单墓碑不改写待办（读侧按默认清单显示）"
        );
        assert_eq!(
            list_tombs(&db),
            vec![
                list_tomb(WORK, "2026-05-02 10:00:00"),
                list_tomb(LIFE, "2026-05-02 10:00:00")
            ],
            "墓碑并入本地，供后续上传"
        );

        // 本地墓碑压制远端的旧版本（另一台设备还没同步到删除）；墓碑并集取较大值
        let s = merge_lists(
            &db,
            Some(vec![list_json(WORK, "旧版本", "2026-05-01 10:00:00")]),
            vec![list_tomb(WORK, "2026-05-03 10:00:00")],
        );
        assert_eq!(s, MergeStats::default(), "被墓碑压制不算跳过");
        assert!(list_name(&db, WORK).is_none());
        assert_eq!(list_tombs(&db)[0], list_tomb(WORK, "2026-05-03 10:00:00"));

        // 远端带着较新的版本 + 较旧的墓碑：插入
        let db2 = self::db();
        let s = merge_lists(
            &db2,
            Some(vec![list_json(WORK, "复活", "2026-05-04 10:00:00")]),
            vec![list_tomb(WORK, "2026-05-03 10:00:00")],
        );
        assert_eq!(s.lists_inserted, 1);
    }

    #[test]
    fn default_list_tombstones_are_ignored_everywhere() {
        assert_eq!(
            parse_list_tombstone(&json!({"entityId": 1, "deletedAt": "2026-05-01 10:00:00"})),
            None,
            "默认清单不可删除：它的墓碑不存、不传播"
        );
        let db = db();
        db.with_transaction(|tx| record_list_tombstone(tx, 1, "2099-01-01 00:00:00"))
            .unwrap();
        assert!(list_tombs(&db).is_empty());
        // 就算墓碑表里有（外部工具写的），默认清单也不会被删、不会被压制
        db.with_connection(|c| {
            c.execute(
                "INSERT INTO list_tombstones (entity_id, deleted_at) VALUES (1, '2099-01-01 00:00:00')",
                [],
            )
        })
        .unwrap();
        let s = merge_lists(
            &db,
            Some(vec![list_json(1, "收件箱", "2026-05-01 10:00:00")]),
            vec![list_tomb(1, "2099-01-01 00:00:00")],
        );
        assert_eq!((s.lists_deleted, s.lists_updated), (0, 1));
        assert_eq!(list_name(&db, 1).as_deref(), Some("收件箱"));
        assert!(list_tombs(&db).is_empty(), "读出 / 上传时也不带它");
    }

    #[test]
    fn bad_remote_lists_are_skipped_and_counted() {
        let db = db();
        db.with_transaction(|tx| record_list_tombstone(tx, 77, "2026-05-02 10:00:00"))
            .unwrap();
        let mut no_name = list_json(WORK, "x", "2026-05-01 10:00:00");
        no_name.as_object_mut().unwrap().remove("name");
        let mut bad_type = list_json(LIFE, "x", "2026-05-01 10:00:00");
        bad_type["sortOrder"] = json!("first");
        let mut bad_time = list_json(55, "x", "2026-05-01 10:00:00");
        bad_time["updatedAt"] = json!("昨天");
        let mut suppressed_bad = list_json(77, "x", "2026-05-01 10:00:00");
        suppressed_bad["name"] = json!(123);
        let s = merge_lists(
            &db,
            Some(vec![
                no_name,
                bad_type,
                bad_time,
                json!("not an object"),
                json!({"name": "没有 id", "updatedAt": "2026-05-01 10:00:00"}),
                suppressed_bad,
                list_json(66, "正常", "2026-05-01 10:00:00"),
            ]),
            vec![],
        );
        assert_eq!(s.lists_inserted, 1);
        assert_eq!(s.records_skipped, 5, "已被墓碑压制的坏记录不算跳过");
        assert_eq!(count(&db, "todo_lists"), 2);
    }

    /// (list_id, title, updated_at)
    fn todo_row(db: &Database, id: i64) -> (Option<i64>, String, String) {
        db.with_connection(|c| {
            c.query_row(
                "SELECT list_id, title, updated_at FROM todos WHERE id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
        })
        .unwrap()
    }

    fn todo_in(id: i64, title: &str, updated_at: &str, list_id: i64) -> Value {
        let mut todo = todo_json(id, title, updated_at);
        todo["listId"] = json!(list_id);
        todo
    }

    /// 契约 §12：`listId` 缺失 = 未知，永远不覆盖已知的归属。远端胜出但没有 listId → 内容取远端、归属沿用本地；
    /// 远端新增且没有 listId → 本地也是未知（不是默认清单）
    #[test]
    fn missing_list_id_means_unknown_and_never_overrides_a_known_list() {
        let db = db();
        merge(
            &db,
            vec![todo_in(1, "新设备上的", "2026-05-01 10:00:00", WORK)],
            vec![],
        );
        assert_eq!(todo_row(&db, 1).0, Some(WORK), "listId 随记录落库");

        // 旧设备上传的同一版本（平局）缺 listId：保留本地
        let s = merge(
            &db,
            vec![todo_json(1, "旧设备", "2026-05-01 10:00:00")],
            vec![],
        );
        assert_eq!(s, MergeStats::default());
        assert_eq!(todo_row(&db, 1).1, "新设备上的");
        // 旧设备改过（远端较新）缺 listId：内容取远端，归属沿用本地清单
        let s = merge(
            &db,
            vec![todo_json(1, "旧设备改过", "2026-05-02 10:00:00")],
            vec![],
        );
        assert_eq!(s.todos_updated, 1);
        assert_eq!(
            todo_row(&db, 1),
            (
                Some(WORK),
                "旧设备改过".to_string(),
                "2026-05-02 10:00:00".to_string()
            )
        );
        // 远端新增、没有 listId：未知（显示在默认清单，但不是"已知的 1"）
        merge(
            &db,
            vec![todo_json(2, "旧设备新建", "2026-05-01 10:00:00")],
            vec![],
        );
        assert_eq!(todo_row(&db, 2).0, None);
        let local = db
            .with_connection(|c| local_records(c, CUTOFF))
            .unwrap()
            .todos;
        let written: Vec<Value> = local
            .iter()
            .map(|t| serde_json::to_value(t).unwrap())
            .collect();
        assert_eq!(written[0]["listId"], WORK);
        assert!(
            written[1].get("listId").is_none(),
            "上传时未知不写 listId 键"
        );
    }

    /// 契约 §12-3：远端有 listId、本地未知 → 不论 LWW 谁胜（平局、本地较新、远端较新）都采纳远端的值；
    /// 平局 / 本地较新时只改归属、不改 updated_at 与内容。本地已知时不"采纳"别的值（只有 LWW 胜出才改）
    #[test]
    fn unknown_local_todos_adopt_a_known_remote_list_whoever_wins() {
        let db = db();
        merge(
            &db,
            vec![
                todo_json(1, "本地", "2026-05-01 10:00:00"),
                todo_json(2, "本地较新", "2026-05-05 10:00:00"),
                todo_json(3, "本地较旧", "2026-05-01 10:00:00"),
                todo_json(4, "两边都未知", "2026-05-01 10:00:00"),
            ],
            vec![],
        );
        for id in 1..=4 {
            assert_eq!(todo_row(&db, id).0, None, "前提：本地归属未知");
        }

        let s = merge(
            &db,
            vec![
                todo_in(1, "远端平局", "2026-05-01 10:00:00", WORK),
                todo_in(2, "远端较旧", "2026-05-01 10:00:00", LIFE),
                todo_in(3, "远端较新", "2026-05-02 10:00:00", DEFAULT_LIST_ID),
                todo_json(4, "两边都未知", "2026-05-01 10:00:00"),
            ],
            vec![],
        );
        assert_eq!(s.todos_updated, 3, "采纳归属也是本地数据变化，界面要刷新");
        assert_eq!(
            todo_row(&db, 1),
            (
                Some(WORK),
                "本地".to_string(),
                "2026-05-01 10:00:00".to_string()
            ),
            "平局：内容保留本地，采纳远端的归属，不改 updated_at"
        );
        assert_eq!(
            todo_row(&db, 2),
            (
                Some(LIFE),
                "本地较新".to_string(),
                "2026-05-05 10:00:00".to_string()
            ),
            "本地较新：同上"
        );
        assert_eq!(
            todo_row(&db, 3),
            (
                Some(DEFAULT_LIST_ID),
                "远端较新".to_string(),
                "2026-05-02 10:00:00".to_string()
            )
        );
        assert_eq!(todo_row(&db, 4).0, None, "远端也未知：仍是未知");

        // 本地已经已知：平局时不被远端的另一个值改掉
        let s = merge(
            &db,
            vec![todo_in(1, "远端平局", "2026-05-01 10:00:00", LIFE)],
            vec![],
        );
        assert_eq!(s, MergeStats::default());
        assert_eq!(todo_row(&db, 1).0, Some(WORK));
    }

    /// 强制推送前：本机未知、远端已知的归属先采纳（只改归属），本机已知的照旧以本机为准
    #[test]
    fn force_push_prepare_adopts_known_remote_lists_for_unknown_local_todos() {
        let db = db();
        merge(
            &db,
            vec![
                todo_json(1, "本机未知", "2026-05-09 10:00:00"),
                todo_in(2, "本机已知", "2026-05-09 10:00:00", WORK),
            ],
            vec![],
        );
        let remote = vec![
            todo_in(1, "远端", "2026-05-01 10:00:00", LIFE),
            todo_in(2, "远端", "2026-05-01 10:00:00", LIFE),
        ];
        db.with_transaction(|tx| force_push_prepare(tx, &remote, &[], "2026-05-10 00:00:00"))
            .unwrap();
        assert_eq!(
            todo_row(&db, 1),
            (
                Some(LIFE),
                "本机未知".to_string(),
                "2026-05-09 10:00:00".to_string()
            )
        );
        assert_eq!(todo_row(&db, 2).0, Some(WORK), "本机已知：以本机为准");
    }

    /// 强制拉取同样不让"未知"覆盖本地已知的归属；远端已知时照常覆盖
    #[test]
    fn force_pull_keeps_a_known_list_when_the_remote_does_not_know_it() {
        let db = db();
        merge(
            &db,
            vec![
                todo_in(1, "本地", "2026-05-09 10:00:00", WORK),
                todo_in(2, "本地", "2026-05-09 10:00:00", WORK),
            ],
            vec![],
        );
        let remote = parse_remote_todos(&[
            todo_json(1, "远端不认识清单", "2026-05-01 10:00:00"),
            todo_in(2, "远端已知", "2026-05-01 10:00:00", LIFE),
        ]);
        db.with_transaction(|tx| force_pull_replace(tx, &remote, &[], CUTOFF))
            .unwrap();
        assert_eq!(
            todo_row(&db, 1),
            (
                Some(WORK),
                "远端不认识清单".to_string(),
                "2026-05-01 10:00:00".to_string()
            )
        );
        assert_eq!(todo_row(&db, 2).0, Some(LIFE));
    }

    #[test]
    fn force_pull_lists_makes_local_lists_equal_remote() {
        let db = db();
        merge_lists(
            &db,
            Some(vec![
                list_json(WORK, "本地较新", "2026-05-09 10:00:00"),
                list_json(LIFE, "本地独有", "2026-05-01 10:00:00"),
                list_json(1, "本地改名", "2026-05-01 10:00:00"),
            ]),
            vec![],
        );
        db.with_transaction(|tx| record_list_tombstone(tx, 99, "2026-05-01 10:00:00"))
            .unwrap();

        let force_pull = |lists: Option<Vec<Value>>, tombs: Vec<ListTombstone>| {
            let parsed = lists.as_deref().map(parse_remote_lists);
            db.with_transaction(|tx| {
                force_pull_replace_lists(tx, parsed.as_deref(), &tombs, CUTOFF)
            })
            .unwrap()
        };

        // 远端没有清单信息：什么都不动
        assert_eq!(force_pull(None, vec![]), MergeStats::default());
        assert_eq!(count(&db, "todo_lists"), 3);
        assert_eq!(list_tombs(&db).len(), 1);

        // 远端（不含默认清单——写入方有问题）：本地独有的删掉（不写墓碑），默认清单保留，墓碑表替换为远端
        let s = force_pull(
            Some(vec![
                list_json(WORK, "远端旧版", "2026-05-01 10:00:00"),
                list_json(55, "远端自相矛盾", "2026-05-01 10:00:00"),
            ]),
            vec![
                list_tomb(55, "2026-05-02 10:00:00"),
                list_tomb(7, "2026-05-01 10:00:00"),
            ],
        );
        assert_eq!(
            (s.lists_updated, s.lists_deleted, s.lists_inserted),
            (1, 1, 0)
        );
        assert_eq!(list_name(&db, WORK).as_deref(), Some("远端旧版"));
        assert!(list_name(&db, LIFE).is_none());
        assert!(list_name(&db, 55).is_none(), "被远端自己的墓碑压制");
        assert_eq!(
            list_name(&db, 1).as_deref(),
            Some("本地改名"),
            "默认清单永远保留"
        );
        assert_eq!(
            list_tombs(&db),
            vec![
                list_tomb(7, "2026-05-01 10:00:00"),
                list_tomb(55, "2026-05-02 10:00:00")
            ]
        );
    }

    #[test]
    fn force_push_prepare_lists_tombstones_remote_only_and_bumps_conflicts() {
        let db = db();
        merge_lists(
            &db,
            Some(vec![
                list_json(WORK, "本地旧", "2026-05-01 10:00:00"),
                list_json(LIFE, "本地", "2026-05-01 10:00:00"),
            ]),
            vec![],
        );
        let now = "2026-05-03 00:00:00";
        let remote = vec![
            list_json(WORK, "远端新", "2026-05-05 10:00:00"),
            list_json(55, "远端独有", "2099-01-01 08:00:00"),
            list_json(1, "远端改名的默认清单", "2026-05-04 00:00:00"),
        ];
        let bumped = db
            .with_transaction(|tx| {
                force_push_prepare_lists(
                    tx,
                    Some(&remote),
                    &[list_tomb(LIFE, "2026-05-02 00:00:00")],
                    now,
                )
            })
            .unwrap();
        assert_eq!(bumped, 3);
        let ua = |id: i64| -> String {
            db.with_connection(|c| {
                c.query_row(
                    "SELECT updated_at FROM todo_lists WHERE id = ?1",
                    [id],
                    |r| r.get(0),
                )
            })
            .unwrap()
        };
        assert_eq!(ua(WORK), "2026-05-05 10:00:01", "必须严格晚于远端版本");
        assert_eq!(ua(LIFE), now, "必须压过远端墓碑");
        assert_eq!(ua(1), "2026-05-04 00:00:01");
        assert_eq!(
            list_tombs(&db),
            vec![list_tomb(55, "2099-01-01 08:00:00")],
            "远端独有的清单写墓碑，时间压得住它的 updatedAt"
        );
    }

    #[test]
    fn parse_list_tombstone_is_lenient_but_validates() {
        assert_eq!(
            parse_list_tombstone(&json!({"entityId": WORK, "deletedAt": "2026-05-01T10:00"})),
            Some(list_tomb(WORK, "2026-05-01 10:00:00"))
        );
        assert_eq!(
            parse_list_tombstone(&json!({"entityId": "42", "deletedAt": "2026-05-01"})),
            Some(list_tomb(42, "2026-05-01 00:00:00"))
        );
        for bad in [
            json!({"entityId": "x", "deletedAt": "2026-05-01"}),
            json!({"entityId": 5, "deletedAt": "yesterday"}),
            json!({"entityId": 5}),
            json!({"deletedAt": "2026-05-01"}),
            json!("not an object"),
        ] {
            assert_eq!(parse_list_tombstone(&bad), None, "{bad}");
        }
    }
}
