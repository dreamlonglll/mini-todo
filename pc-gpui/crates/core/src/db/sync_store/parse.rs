//! 远端 JSON 记录解析、时间规范化与待办清单归属（[`ListAssignment`]）。

use super::EntityKind;
use crate::db::models::{ListTombstone, SubTask, Todo, TodoList, Tombstone, DEFAULT_LIST_ID};
use crate::db::time::{normalize_datetime, DefaultTime};
use rusqlite::{params, Connection, Result};
use serde_json::Value;
use std::collections::HashMap;

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
// 待办的清单归属（契约 §13：`(listId, listUpdatedAt)` 字段级 LWW）
// ============================================================================

/// 待办的清单归属：`listId` 与它自己的版本 `listUpdatedAt`。合并时与整条记录的 `updatedAt` 无关——
/// 结果记录存在时，归属取所有可见版本（本地合并前的版本、远端版本，墓碑不影响）里最新的那个。
///
/// 新旧比较（契约 §13-3）：未知（`listId` 缺失 / null）最旧；已知但没有版本（`listUpdatedAt` 缺失或无法识别）次旧；
/// 已知且有版本的按时间比较。两边一样新、`listId` 不同时取**较大的 `listId`**（确定性规则，两端结论相同、保证收敛）
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ListAssignment {
    /// `None` = 未知
    pub list_id: Option<i64>,
    /// 规范时间；`None` = 没有版本。归属未知时恒为 `None`
    pub list_updated_at: Option<String>,
}

impl ListAssignment {
    /// 规范化构造：版本能识别的转成规范格式、识别不了的当作没有版本；归属未知时丢掉版本
    pub fn new(list_id: Option<i64>, list_updated_at: Option<&str>) -> Self {
        let list_updated_at = list_id
            .and(list_updated_at)
            .and_then(|raw| normalize_datetime(raw, DefaultTime::StartOfDay));
        Self {
            list_id,
            list_updated_at,
        }
    }

    pub fn of(todo: &Todo) -> Self {
        Self::new(todo.list_id, todo.list_updated_at.as_deref())
    }

    /// 同步文档里一条待办的归属。畸形记录不参与归属合并（契约 §13-11）：不是对象、`listId` 类型不对、
    /// 记录的 `updatedAt` 无法识别时返回 `None`；`listUpdatedAt` 不是字符串只当作没有版本
    pub fn from_value(v: &Value) -> Option<Self> {
        if !v.is_object() || value_updated_at(v).is_empty() {
            return None;
        }
        let list_id = match v.get("listId") {
            None | Some(Value::Null) => None,
            Some(id) => Some(id.as_i64()?),
        };
        Some(Self::new(
            list_id,
            v.get("listUpdatedAt").and_then(Value::as_str),
        ))
    }

    /// 比较键：(级别, 版本, listId)，按字典序比较即契约 §13-3
    fn rank(&self) -> (u8, &str, i64) {
        match (self.list_id, self.list_updated_at.as_deref()) {
            (None, _) => (0, "", 0),
            (Some(id), None) => (1, "", id),
            (Some(id), Some(at)) => (2, at, id),
        }
    }

    /// 是否严格比 `other` 新（完全相同不算）
    pub fn is_newer_than(&self, other: &Self) -> bool {
        self.rank() > other.rank()
    }

    /// 两者中较新的一个（一样新时保留 `self`）
    pub fn newest(self, other: Self) -> Self {
        if other.is_newer_than(&self) {
            other
        } else {
            self
        }
    }
}

/// 全部本地待办的归属（id → 归属）。合并时在应用墓碑**之前**读出：被墓碑删掉、又被远端更新的编辑救回来的待办，
/// 它合并前的归属也算可见版本（契约 §13-4，F2）
pub(super) fn load_assignments(conn: &Connection) -> Result<HashMap<i64, ListAssignment>> {
    let mut stmt = conn.prepare_cached("SELECT id, list_id, list_updated_at FROM todos")?;
    let rows = stmt.query_map([], |r| {
        let list_id: Option<i64> = r.get(1)?;
        let at: Option<String> = r.get(2)?;
        Ok((
            r.get::<_, i64>(0)?,
            ListAssignment::new(list_id, at.as_deref()),
        ))
    })?;
    rows.collect()
}

/// 只改一条待办的归属（两列一起），**不改** `updated_at`（契约 §13-5）。没有这条或归属没变时不写，返回是否改了。
/// 会让 `local_seq` 加一（触发器），但同步收尾时 `synced_seq` 取合并之后的值，不会因此多上传一次
pub(super) fn set_assignment(
    conn: &Connection,
    id: i64,
    assignment: &ListAssignment,
) -> Result<bool> {
    Ok(conn
        .prepare_cached(
            "UPDATE todos SET list_id = ?1, list_updated_at = ?2
             WHERE id = ?3 AND (list_id IS NOT ?1 OR list_updated_at IS NOT ?2)",
        )?
        .execute(params![assignment.list_id, assignment.list_updated_at, id])?
        > 0)
}

pub(super) fn with_assignment(todo: &Todo, assignment: &ListAssignment) -> Todo {
    Todo {
        list_id: assignment.list_id,
        list_updated_at: assignment.list_updated_at.clone(),
        ..todo.clone()
    }
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
/// `createdAt` 缺失时取 `updatedAt`。`listUpdatedAt` 例外：无法识别只当作"没有版本"（契约 §13-3），不让整条记录作废
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
    t.list_updated_at = ListAssignment::of(t).list_updated_at;
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
    lenient_opt(&mut t.list_updated_at, DefaultTime::StartOfDay);
    // 归属未知时两个键都不写出（契约 §13-7）
    if t.list_id.is_none() {
        t.list_updated_at = None;
    }
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
