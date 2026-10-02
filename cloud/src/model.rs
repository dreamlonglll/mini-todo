//! 记录字段契约（跨端契约 K6），以 PC 端 `Todo` / `SubTask` 强类型模型为准
//! （`pc/src-tauri/src/db/models.rs`）。
//!
//! PC 合并远端时用强类型反序列化：未知字段直接丢弃，类型不符的记录整条跳过（两端从此
//! 分叉）。云端存的是原始 JSON（KV-style），所以每条进入缓存的路径都要守住契约：
//!
//! - API 写入：`validate_todo_input` / `validate_subtask_input`——只接受契约字段，
//!   别名先归一化（`dueDate`→`endTime`、`priority`→`color`、`notes`→`description`），
//!   时间统一成 K1 规范格式；未知字段 / 类型错误 → 400 并列出字段
//! - 存量数据：`normalize_stored_todo` / `normalize_stored_subtask`——启动后（第一次
//!   成功 pull 之后）一次性把旧版 API 写进来的别名字段、错误类型修好
//! - 合并远端：`canonicalize_record_times`——只统一时间格式，不改语义、不动未知字段
//!   （未知字段可能是更新版本 PC 加的新字段，必须原样保留）
//!
//! 读取响应由 `todo_view` 附加派生字段 `priority`（由 color 映射，自定义颜色为 null）
//! 与 `seq`；派生字段永不入库。

use chrono::NaiveTime;
use chrono_tz::Tz;
use serde_json::{json, Map, Value};

use crate::time::{normalize_datetime, CANONICAL_FORMAT};

/// 高 / 中 / 低优先级在 PC 端对应的颜色（`pc/src/types/todo.ts` 的预设色）。
pub const COLOR_HIGH: &str = "#EF4444";
pub const COLOR_MEDIUM: &str = "#F59E0B";
pub const COLOR_LOW: &str = "#10B981";
/// 新建待办的默认颜色（与 PC 端 `DEFAULT_COLOR` 一致）。
pub const DEFAULT_TODO_COLOR: &str = COLOR_LOW;
/// PC 反序列化时缺 `color` 的默认值（`models.rs::default_color`）。
pub const PC_FALLBACK_COLOR: &str = COLOR_MEDIUM;
/// 修复缺失 / 非字符串标题时使用的标题（PC 要求 title 必填）。
const UNTITLED: &str = "（无标题）";

// =============================================================================
// 优先级（派生自 color）
// =============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Priority {
    High,
    Medium,
    Low,
}

impl Priority {
    /// `high` / `medium` / `low`，大小写不敏感。
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "high" => Some(Priority::High),
            "medium" => Some(Priority::Medium),
            "low" => Some(Priority::Low),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Priority::High => "high",
            Priority::Medium => "medium",
            Priority::Low => "low",
        }
    }

    pub fn color(self) -> &'static str {
        match self {
            Priority::High => COLOR_HIGH,
            Priority::Medium => COLOR_MEDIUM,
            Priority::Low => COLOR_LOW,
        }
    }

    /// 三种优先级色（大小写不敏感）→ 优先级；其它颜色返回 `None`。
    pub fn from_color(color: &str) -> Option<Self> {
        [Priority::High, Priority::Medium, Priority::Low]
            .into_iter()
            .find(|p| p.color().eq_ignore_ascii_case(color.trim()))
    }
}

/// 记录在 PC 端呈现的颜色：缺失 / null 时 PC 用 `#F59E0B`。非字符串返回 `None`。
fn effective_color(todo: &Map<String, Value>) -> Option<&str> {
    match todo.get("color") {
        None | Some(Value::Null) => Some(PC_FALLBACK_COLOR),
        Some(Value::String(s)) => Some(s.as_str()),
        Some(_) => None,
    }
}

/// 派生优先级：由 color 映射，自定义颜色为 `None`。
pub fn derived_priority(todo: &Map<String, Value>) -> Option<Priority> {
    effective_color(todo).and_then(Priority::from_color)
}

/// API 响应里的 todo：附加派生字段 `priority` 与（有的话）`seq`。不改动其它字段。
pub fn todo_view(mut v: Value, seq: Option<i64>) -> Value {
    if let Value::Object(obj) = &mut v {
        let p = derived_priority(obj).map(Priority::as_str);
        obj.insert("priority".into(), json!(p));
        if let Some(seq) = seq {
            obj.insert("seq".into(), json!(seq));
        }
    }
    v
}

// =============================================================================
// 象限
// =============================================================================

/// 象限的字符串别名（大小写不敏感）。`important_urgent` 是历史上接受过的写法。
const QUADRANT_ALIASES: &[(&str, i64)] = &[
    ("urgent_important", 1),
    ("important_urgent", 1),
    ("important_not_urgent", 2),
    ("urgent_not_important", 3),
    ("not_urgent_not_important", 4),
];

/// 接受 `"1"`–`"4"` 或别名。
pub fn parse_quadrant_str(s: &str) -> Option<i64> {
    let t = s.trim();
    if let Ok(n) = t.parse::<i64>() {
        return (1..=4).contains(&n).then_some(n);
    }
    let lower = t.to_ascii_lowercase();
    QUADRANT_ALIASES
        .iter()
        .find(|(a, _)| *a == lower)
        .map(|(_, n)| *n)
}

// =============================================================================
// 时间字段
// =============================================================================

/// 带业务语义的时间字段：决定仅日期输入的默认时刻（K1，与 PC 编辑器一致）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeField {
    StartTime,
    EndTime,
    NotifyAt,
}

impl TimeField {
    pub fn default_time(self) -> NaiveTime {
        match self {
            TimeField::StartTime => NaiveTime::MIN,
            TimeField::EndTime => NaiveTime::from_hms_opt(23, 59, 0).expect("valid time"),
            TimeField::NotifyAt => NaiveTime::from_hms_opt(9, 0, 0).expect("valid time"),
        }
    }
}

/// todo 上的业务时间字段。
const TODO_TIME_FIELDS: &[(&str, TimeField)] = &[
    ("notifyAt", TimeField::NotifyAt),
    ("startTime", TimeField::StartTime),
    ("endTime", TimeField::EndTime),
];

/// 记录的元信息时间字段（默认时刻 00:00:00）。
const META_TIME_FIELDS: &[&str] = &["createdAt", "updatedAt"];

const DATETIME_HINT: &str = "must be a date/time string 'YYYY-MM-DD HH:MM:SS' (also accepted: \
     'YYYY-MM-DD', 'YYYY-MM-DD HH:MM', 'YYYY-MM-DDTHH:MM[:SS]', optional Z/±HH:MM), \
     or null / \"\" to clear";

// =============================================================================
// 输入校验
// =============================================================================

/// 写入方式：创建时 `title` 必填。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteMode {
    Create,
    Update,
}

/// 校验失败的详情：未知字段与类型错误。`message()` 生成给客户端的说明。
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ValidationError {
    pub unknown: Vec<String>,
    /// `(字段, 说明)`
    pub invalid: Vec<(String, String)>,
    /// 允许写入的字段（含别名），出错时一并告诉客户端。
    pub allowed: Vec<&'static str>,
}

impl ValidationError {
    fn is_empty(&self) -> bool {
        self.unknown.is_empty() && self.invalid.is_empty()
    }

    fn invalid(&mut self, field: &str, msg: impl Into<String>) {
        self.invalid.push((field.to_string(), msg.into()));
    }

    pub fn message(&self) -> String {
        let mut parts = Vec::new();
        if !self.unknown.is_empty() {
            parts.push(format!(
                "unknown field(s): {}; allowed: {}",
                self.unknown.join(", "),
                self.allowed.join(", ")
            ));
        }
        for (field, msg) in &self.invalid {
            parts.push(format!("{}: {}", field, msg));
        }
        parts.join("; ")
    }
}

#[derive(Debug, Clone, Copy)]
enum Kind {
    Title,
    NullableString,
    Color,
    Quadrant,
    DateTime(TimeField),
    NonNegativeInt,
    Bool,
    Int,
    RepeatType,
    PositiveInt,
    Weekdays,
    MonthDay,
}

/// todo 可写字段（K6）。
const TODO_FIELDS: &[(&str, Kind)] = &[
    ("title", Kind::Title),
    ("description", Kind::NullableString),
    ("color", Kind::Color),
    ("quadrant", Kind::Quadrant),
    ("notifyAt", Kind::DateTime(TimeField::NotifyAt)),
    ("startTime", Kind::DateTime(TimeField::StartTime)),
    ("endTime", Kind::DateTime(TimeField::EndTime)),
    ("notifyBefore", Kind::NonNegativeInt),
    ("notified", Kind::Bool),
    ("completed", Kind::Bool),
    ("sortOrder", Kind::Int),
    ("repeatEnabled", Kind::Bool),
    ("repeatType", Kind::RepeatType),
    ("repeatInterval", Kind::PositiveInt),
    ("repeatWeekdays", Kind::Weekdays),
    ("repeatMonthDay", Kind::MonthDay),
];

/// todo 输入别名：`(别名, 规范字段)`。同时给了规范字段时以规范字段为准。
const TODO_ALIASES: &[(&str, &str)] = &[
    ("dueDate", "endTime"),
    ("priority", "color"),
    ("notes", "description"),
];

/// 服务端维护的字段与只读派生字段：写入时忽略（方便客户端"读出来改完整包写回"）。
const TODO_IGNORED: &[&str] = &[
    "id",
    "createdAt",
    "updatedAt",
    "seq",
    "subtasks",
    "subtaskCount",
];

/// subtask 可写字段（K6）。`parentId` 单独处理：只能等于当前父待办。
const SUBTASK_FIELDS: &[(&str, Kind)] = &[
    ("title", Kind::Title),
    ("content", Kind::NullableString),
    ("completed", Kind::Bool),
    ("sortOrder", Kind::Int),
];

const SUBTASK_IGNORED: &[&str] = &["id", "createdAt", "updatedAt"];

fn as_i32(v: &Value) -> Option<i64> {
    v.as_i64().filter(|n| i32::try_from(*n).is_ok())
}

/// 把 `"1, 3,5"` / `[1, 3, 5]` 规范成 `"1,3,5"`（升序去重）；空列表 → `None`。
fn normalize_weekdays(v: &Value) -> Result<Option<String>, ()> {
    let mut days: Vec<i64> = match v {
        Value::String(s) if s.trim().is_empty() => return Ok(None),
        Value::String(s) => s
            .split(',')
            .filter(|p| !p.trim().is_empty())
            .map(|p| p.trim().parse::<i64>().map_err(|_| ()))
            .collect::<Result<_, _>>()?,
        Value::Array(items) => items
            .iter()
            .map(|i| i.as_i64().ok_or(()))
            .collect::<Result<_, _>>()?,
        _ => return Err(()),
    };
    if days.iter().any(|d| !(1..=7).contains(d)) {
        return Err(());
    }
    days.sort_unstable();
    days.dedup();
    if days.is_empty() {
        return Ok(None);
    }
    Ok(Some(
        days.iter()
            .map(i64::to_string)
            .collect::<Vec<_>>()
            .join(","),
    ))
}

/// 按字段类型校验并规范化一个值。`Err` 是给客户端看的说明。
fn coerce(kind: Kind, v: &Value, tz: Tz) -> Result<Value, String> {
    match kind {
        Kind::Title => match v.as_str() {
            Some(s) if !s.trim().is_empty() => Ok(json!(s)),
            _ => Err("must be a non-empty string".into()),
        },
        Kind::NullableString => match v {
            Value::Null | Value::String(_) => Ok(v.clone()),
            _ => Err("must be a string or null".into()),
        },
        Kind::Color => match v.as_str().map(str::trim) {
            Some(s) if is_hex_color(s) => Ok(json!(s.to_ascii_uppercase())),
            _ => Err("must be a hex color '#RRGGBB', e.g. \"#EF4444\"".into()),
        },
        Kind::Quadrant => {
            let n = match v {
                Value::Number(_) => v.as_i64().filter(|n| (1..=4).contains(n)),
                Value::String(s) => parse_quadrant_str(s),
                _ => None,
            };
            n.map(|n| json!(n)).ok_or_else(|| {
                "must be 1-4 or one of urgent_important, important_not_urgent, \
                 urgent_not_important, not_urgent_not_important"
                    .into()
            })
        }
        Kind::DateTime(field) => match v {
            Value::Null => Ok(Value::Null),
            Value::String(s) if s.trim().is_empty() => Ok(Value::Null),
            Value::String(s) => normalize_datetime(s, tz, field.default_time())
                .map(Value::String)
                .ok_or_else(|| DATETIME_HINT.into()),
            _ => Err(DATETIME_HINT.into()),
        },
        Kind::NonNegativeInt => as_i32(v)
            .filter(|n| *n >= 0)
            .map(|n| json!(n))
            .ok_or_else(|| "must be an integer >= 0".into()),
        Kind::Bool => v
            .as_bool()
            .map(|b| json!(b))
            .ok_or_else(|| "must be true or false".into()),
        Kind::Int => as_i32(v)
            .map(|n| json!(n))
            .ok_or_else(|| "must be an integer (32-bit)".into()),
        Kind::RepeatType => match v {
            Value::Null => Ok(Value::Null),
            Value::String(s) if s.trim().is_empty() => Ok(Value::Null),
            Value::String(s) => {
                let lower = s.trim().to_ascii_lowercase();
                if matches!(lower.as_str(), "daily" | "weekly" | "monthly") {
                    Ok(json!(lower))
                } else {
                    Err("must be daily, weekly, monthly or null".into())
                }
            }
            _ => Err("must be daily, weekly, monthly or null".into()),
        },
        Kind::PositiveInt => as_i32(v)
            .filter(|n| *n >= 1)
            .map(|n| json!(n))
            .ok_or_else(|| "must be an integer >= 1".into()),
        Kind::Weekdays => {
            if v.is_null() {
                return Ok(Value::Null);
            }
            normalize_weekdays(v).map(|d| json!(d)).map_err(|_| {
                "must be weekdays 1-7 (1 = Monday) as \"1,3,5\" or [1, 3, 5], or null".into()
            })
        }
        Kind::MonthDay => match v {
            Value::Null => Ok(Value::Null),
            _ => v
                .as_i64()
                .filter(|d| (1..=31).contains(d))
                .map(|d| json!(d))
                .ok_or_else(|| "must be an integer 1-31 or null".into()),
        },
    }
}

fn is_hex_color(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 7 && b[0] == b'#' && b[1..].iter().all(u8::is_ascii_hexdigit)
}

fn allowed_list(
    fields: &[(&'static str, Kind)],
    aliases: &[(&'static str, &'static str)],
) -> Vec<&'static str> {
    fields
        .iter()
        .map(|(f, _)| *f)
        .chain(aliases.iter().map(|(a, _)| *a))
        .collect()
}

/// 校验 `POST /todos` / `PATCH /todos/:id` 的 body，返回只含规范字段、值已规范化的补丁。
///
/// - 未知字段、类型错误 → `Err`（列出全部问题与允许的字段）
/// - 别名：`dueDate`→`endTime`（仅日期 → 23:59:00）、`priority`(high/medium/low)→`color`、
///   `notes`→`description`；同时给了规范字段时以规范字段为准；`priority: null` 忽略
///   （读出的派生值对自定义颜色是 null，原样写回不报错）
/// - `id` / `createdAt` / `updatedAt` / `seq` / `subtasks` / `subtaskCount` 忽略
/// - 创建时 `title` 必填
pub fn validate_todo_input(
    body: &Value,
    mode: WriteMode,
    tz: Tz,
) -> Result<Map<String, Value>, ValidationError> {
    let mut err = ValidationError {
        allowed: allowed_list(TODO_FIELDS, TODO_ALIASES),
        ..Default::default()
    };
    let Value::Object(input) = body else {
        err.invalid("body", "must be a JSON object");
        return Err(err);
    };

    let mut out = Map::new();
    let mut alias_values: Vec<(&str, Value)> = Vec::new();
    for (key, value) in input {
        if TODO_IGNORED.contains(&key.as_str()) {
            continue;
        }
        if let Some((_, kind)) = TODO_FIELDS.iter().find(|(f, _)| f == key) {
            match coerce(*kind, value, tz) {
                Ok(v) => {
                    out.insert(key.clone(), v);
                }
                Err(msg) => err.invalid(key, msg),
            }
            continue;
        }
        match key.as_str() {
            "dueDate" => match coerce(Kind::DateTime(TimeField::EndTime), value, tz) {
                Ok(v) => alias_values.push(("endTime", v)),
                Err(msg) => err.invalid(key, msg),
            },
            "priority" => match value {
                Value::Null => {}
                Value::String(s) => match Priority::parse(s) {
                    Some(p) => alias_values.push(("color", json!(p.color()))),
                    None => err.invalid(key, "must be high, medium, low (or null)"),
                },
                _ => err.invalid(key, "must be high, medium, low (or null)"),
            },
            "notes" => match coerce(Kind::NullableString, value, tz) {
                Ok(v) => alias_values.push(("description", v)),
                Err(msg) => err.invalid(key, msg),
            },
            _ => err.unknown.push(key.clone()),
        }
    }
    for (canonical, v) in alias_values {
        out.entry(canonical.to_string()).or_insert(v);
    }
    if mode == WriteMode::Create && !out.contains_key("title") && !input.contains_key("title") {
        err.invalid("title", "is required");
    }
    if err.is_empty() {
        Ok(out)
    } else {
        Err(err)
    }
}

/// PATCH 的别名补充规则：body 里同时有别名与规范字段时，规范字段**没有改动**（等于当前
/// 存储值）而别名给出了不同的值 → 以别名为准。
///
/// 典型场景是"读出来改完整包写回"：客户端把读到的对象里的 `priority` 改成 `high` 再整包
/// PATCH，对象里还带着原来的 `color`。按"规范字段优先"会静默忽略这次修改；规范字段确实
/// 被改过时仍以规范字段为准（K6）。`body` 必须已经通过 `validate_todo_input`。
pub fn apply_shadowed_aliases(
    patch: &mut Map<String, Value>,
    body: &Value,
    current: &Map<String, Value>,
    tz: Tz,
) {
    let Value::Object(input) = body else {
        return;
    };
    for (alias, canonical) in TODO_ALIASES {
        let (Some(alias_raw), true) = (input.get(*alias), input.contains_key(*canonical)) else {
            continue;
        };
        let unchanged =
            patch.get(*canonical) == Some(current.get(*canonical).unwrap_or(&Value::Null));
        if !unchanged {
            continue;
        }
        let alias_value = match *alias {
            "priority" => alias_raw
                .as_str()
                .and_then(Priority::parse)
                .map(|p| json!(p.color())),
            "dueDate" => coerce(Kind::DateTime(TimeField::EndTime), alias_raw, tz).ok(),
            "notes" => coerce(Kind::NullableString, alias_raw, tz).ok(),
            _ => None,
        };
        if let Some(v) = alias_value {
            patch.insert(canonical.to_string(), v);
        }
    }
}

/// 校验 `POST /todos/:id/subtasks` / `PATCH /subtasks/:id` 的 body。
///
/// `parentId` 不是可写字段：给了且不等于 `parent_id`（当前 / 路径里的父待办）→ 400；
/// 等于时忽略（客户端原样写回读到的对象）。
pub fn validate_subtask_input(
    body: &Value,
    mode: WriteMode,
    parent_id: i64,
) -> Result<Map<String, Value>, ValidationError> {
    let mut err = ValidationError {
        allowed: allowed_list(SUBTASK_FIELDS, &[]),
        ..Default::default()
    };
    let Value::Object(input) = body else {
        err.invalid("body", "must be a JSON object");
        return Err(err);
    };
    // 子任务没有时间类字段，时区不影响校验
    let tz = chrono_tz::UTC;
    let mut out = Map::new();
    for (key, value) in input {
        if SUBTASK_IGNORED.contains(&key.as_str()) {
            continue;
        }
        if key == "parentId" {
            let same = match value {
                Value::Number(_) => value.as_i64() == Some(parent_id),
                Value::String(s) => s.trim().parse::<i64>().ok() == Some(parent_id),
                _ => false,
            };
            if !same {
                err.invalid(
                    key,
                    format!(
                        "cannot be changed (subtask belongs to todo {}); delete it and create a new one under the other todo",
                        parent_id
                    ),
                );
            }
            continue;
        }
        match SUBTASK_FIELDS.iter().find(|(f, _)| f == key) {
            Some((_, kind)) => match coerce(*kind, value, tz) {
                Ok(v) => {
                    out.insert(key.clone(), v);
                }
                Err(msg) => err.invalid(key, msg),
            },
            None => err.unknown.push(key.clone()),
        }
    }
    if mode == WriteMode::Create && !input.contains_key("title") {
        err.invalid("title", "is required");
    }
    if err.is_empty() {
        Ok(out)
    } else {
        Err(err)
    }
}

/// 新建 todo 的完整记录（PC 导出的形态，所有字段齐全），再叠加已校验的补丁。
pub fn new_todo_record(id: i64, patch: &Map<String, Value>, now: &str) -> Map<String, Value> {
    let mut obj = json!({
        "id": id,
        "title": "",
        "description": null,
        "color": DEFAULT_TODO_COLOR,
        "quadrant": 4,
        "notifyAt": null,
        "notifyBefore": 0,
        "notified": false,
        "completed": false,
        "sortOrder": 0,
        "startTime": null,
        "endTime": null,
        "createdAt": now,
        "updatedAt": now,
        "repeatEnabled": false,
        "repeatType": null,
        "repeatInterval": 1,
        "repeatWeekdays": null,
        "repeatMonthDay": null,
    })
    .as_object()
    .cloned()
    .unwrap_or_default();
    crate::util::merge_json_shallow(&mut obj, patch);
    obj
}

/// 新建 subtask 的完整记录（PC 导出的形态），再叠加已校验的补丁。
pub fn new_subtask_record(
    id: i64,
    parent_id: i64,
    patch: &Map<String, Value>,
    now: &str,
) -> Map<String, Value> {
    let mut obj = json!({
        "id": id,
        "parentId": parent_id,
        "title": "",
        "content": null,
        "completed": false,
        "sortOrder": 0,
        "createdAt": now,
        "updatedAt": now,
    })
    .as_object()
    .cloned()
    .unwrap_or_default();
    crate::util::merge_json_shallow(&mut obj, patch);
    obj
}

/// 把已校验的补丁应用到现有 todo 上，并按 PC 的规则联动：
/// - `notifyAt` 改了而客户端没显式给 `notified` → `notified = false`（新时间要重新提醒，
///   与 PC `update_todo` 一致）
/// - 开启重复（`repeatEnabled: true`）而没显式给 `notifyBefore` → `notifyBefore = 0`
///   （重复提醒按 notifyAt 准点触发，与 PC 一致）
pub fn apply_todo_patch(obj: &mut Map<String, Value>, patch: &Map<String, Value>) {
    let notify_changed = patch
        .get("notifyAt")
        .is_some_and(|new| obj.get("notifyAt").unwrap_or(&Value::Null) != new);
    crate::util::merge_json_shallow(obj, patch);
    if notify_changed && !patch.contains_key("notified") {
        obj.insert("notified".into(), json!(false));
    }
    if patch.get("repeatEnabled") == Some(&json!(true)) && !patch.contains_key("notifyBefore") {
        obj.insert("notifyBefore".into(), json!(0));
    }
}

// =============================================================================
// 存量数据归一化
// =============================================================================

/// 一条记录归一化的结果。
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct NormalizeOutcome {
    /// 改变了 PC 看到的内容（别名转换、修复会让 PC 反序列化失败的类型等）：
    /// 必须刷新 `updatedAt` 并标脏，让修复传播出去。
    pub semantic: bool,
    /// 只改了表示形式（时间格式、去掉 PC 不认识的冗余键）：原地改写即可，不刷新
    /// `updatedAt`——否则会用一次纯格式修改在 LWW 里压过 PC 还没同步上来的真实编辑。
    pub format: bool,
}

/// 仅格式的时间规范化：可解析的非规范时间串 → 规范格式；空串、无法解析的值、非字符串
/// 一律不动。返回是否改动。合并远端记录时用（不改语义，不动其它字段）。
pub fn canonicalize_record_times(obj: &mut Map<String, Value>, is_todo: bool, tz: Tz) -> bool {
    let mut changed = false;
    let fields: Vec<(&str, NaiveTime)> = if is_todo {
        TODO_TIME_FIELDS
            .iter()
            .map(|(f, k)| (*f, k.default_time()))
            .chain(META_TIME_FIELDS.iter().map(|f| (*f, NaiveTime::MIN)))
            .collect()
    } else {
        META_TIME_FIELDS
            .iter()
            .map(|f| (*f, NaiveTime::MIN))
            .collect()
    };
    for (field, default_time) in fields {
        let Some(Value::String(s)) = obj.get(field) else {
            continue;
        };
        if s.trim().is_empty() || is_canonical(s) {
            continue;
        }
        if let Some(canonical) = normalize_datetime(s, tz, default_time) {
            obj.insert(field.to_string(), json!(canonical));
            changed = true;
        }
    }
    changed
}

fn is_canonical(s: &str) -> bool {
    chrono::NaiveDateTime::parse_from_str(s, CANONICAL_FORMAT).is_ok() && s.len() == 19
}

fn bool_like(v: &Value) -> bool {
    match v {
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0),
        Value::String(s) => matches!(s.trim().to_ascii_lowercase().as_str(), "true" | "1" | "yes"),
        _ => false,
    }
}

fn int_like(v: &Value) -> Option<i64> {
    match v {
        Value::Number(n) => n
            .as_i64()
            .or_else(|| n.as_f64().filter(|f| f.fract() == 0.0).map(|f| f as i64)),
        Value::String(s) => s.trim().parse::<i64>().ok(),
        _ => None,
    }
    .filter(|n| i32::try_from(*n).is_ok())
}

struct Fixer<'a> {
    obj: &'a mut Map<String, Value>,
    out: NormalizeOutcome,
    tz: Tz,
}

impl Fixer<'_> {
    fn set(&mut self, field: &str, v: Value, semantic: bool) {
        if self.obj.get(field) == Some(&v) {
            return;
        }
        self.obj.insert(field.to_string(), v);
        if semantic {
            self.out.semantic = true;
        } else {
            self.out.format = true;
        }
    }

    fn remove(&mut self, field: &str) -> Option<Value> {
        let v = self.obj.remove(field);
        if v.is_some() {
            // PC 从不认识这些键（往返会被丢掉），删掉只是表示形式的变化
            self.out.format = true;
        }
        v
    }

    fn id(&mut self, row_id: &str) {
        let Ok(id) = row_id.parse::<i64>() else {
            return;
        };
        if self.obj.get("id").and_then(Value::as_i64) != Some(id) {
            self.set("id", json!(id), true);
        }
    }

    fn title(&mut self) {
        match self.obj.get("title") {
            Some(Value::String(_)) => {}
            Some(Value::Number(n)) => {
                let s = n.to_string();
                self.set("title", json!(s), true);
            }
            Some(Value::Bool(b)) => {
                let s = b.to_string();
                self.set("title", json!(s), true);
            }
            _ => self.set("title", json!(UNTITLED), true),
        }
    }

    /// 可空字符串字段：非字符串的值转成字符串（数组 / 对象序列化成 JSON）。
    fn nullable_string(&mut self, field: &str) {
        match self.obj.get(field) {
            None | Some(Value::Null) | Some(Value::String(_)) => {}
            Some(other) => {
                let s = match other {
                    Value::Number(n) => n.to_string(),
                    Value::Bool(b) => b.to_string(),
                    v => v.to_string(),
                };
                self.set(field, json!(s), true);
            }
        }
    }

    fn color(&mut self) {
        match self.obj.get("color") {
            None => {}
            Some(Value::String(s)) if is_hex_color(s.trim()) => {
                let up = s.trim().to_ascii_uppercase();
                self.set("color", json!(up), false);
            }
            _ => self.set("color", json!(PC_FALLBACK_COLOR), true),
        }
    }

    fn quadrant(&mut self) {
        let fixed = match self.obj.get("quadrant") {
            None => return,
            Some(v @ Value::Number(_)) => match v.as_i64() {
                Some(n) if (1..=4).contains(&n) => return,
                _ => int_like(v).filter(|n| (1..=4).contains(n)).unwrap_or(4),
            },
            Some(Value::String(s)) => parse_quadrant_str(s).unwrap_or(4),
            Some(_) => 4,
        };
        self.set("quadrant", json!(fixed), true);
    }

    fn bool_field(&mut self, field: &str) {
        match self.obj.get(field) {
            None | Some(Value::Bool(_)) => {}
            Some(v) => {
                let b = bool_like(v);
                self.set(field, json!(b), true);
            }
        }
    }

    fn int_field(&mut self, field: &str, default: i64, min: Option<i64>) {
        let Some(v) = self.obj.get(field) else {
            return;
        };
        if v.as_i64()
            .is_some_and(|n| i32::try_from(n).is_ok() && min.is_none_or(|m| n >= m))
        {
            return;
        }
        let fixed = int_like(v)
            .filter(|n| min.is_none_or(|m| *n >= m))
            .unwrap_or(default);
        self.set(field, json!(fixed), true);
    }

    fn nullable_int_field(&mut self, field: &str, range: std::ops::RangeInclusive<i64>) {
        match self.obj.get(field) {
            None | Some(Value::Null) => {}
            Some(v) => {
                if v.as_i64().is_some_and(|n| range.contains(&n)) {
                    return;
                }
                let fixed = int_like(v).filter(|n| range.contains(n));
                self.set(field, json!(fixed), true);
            }
        }
    }

    fn time_field(&mut self, field: &str, default_time: NaiveTime) {
        match self.obj.get(field) {
            None | Some(Value::Null) => {}
            Some(Value::String(s)) if s.trim().is_empty() => self.set(field, Value::Null, true),
            Some(Value::String(s)) => {
                if is_canonical(s) {
                    return;
                }
                match normalize_datetime(s, self.tz, default_time) {
                    Some(c) => self.set(field, json!(c), false),
                    None => {
                        tracing::warn!(
                            target: "minitodo_cloud::model",
                            "drop unparseable {} {:?}", field, s
                        );
                        self.set(field, Value::Null, true);
                    }
                }
            }
            Some(_) => self.set(field, Value::Null, true),
        }
    }

    /// createdAt / updatedAt：PC 要求是字符串。坏值改成 `fallback`（规范格式）。
    fn meta_time(&mut self, field: &str, fallback: &str) {
        match self.obj.get(field) {
            None => {}
            Some(Value::String(s)) if is_canonical(s) => {}
            Some(Value::String(s)) => match normalize_datetime(s, self.tz, NaiveTime::MIN) {
                Some(c) => self.set(field, json!(c), false),
                None => self.set(field, json!(fallback), true),
            },
            Some(_) => self.set(field, json!(fallback), true),
        }
    }

    fn repeat_type(&mut self) {
        match self.obj.get("repeatType") {
            None | Some(Value::Null) => {}
            Some(Value::String(s)) => {
                let t = s.trim().to_ascii_lowercase();
                if t.is_empty() {
                    self.set("repeatType", Value::Null, true);
                } else if matches!(t.as_str(), "daily" | "weekly" | "monthly") && t != *s {
                    self.set("repeatType", json!(t), true);
                }
            }
            Some(_) => self.set("repeatType", Value::Null, true),
        }
    }

    fn weekdays(&mut self) {
        match self.obj.get("repeatWeekdays") {
            None | Some(Value::Null) | Some(Value::String(_)) => {}
            Some(v) => {
                let fixed = normalize_weekdays(v).ok().flatten();
                self.set("repeatWeekdays", json!(fixed), true);
            }
        }
    }
}

/// 归一化缓存里的一条 todo（存量数据修复，见模块注释）。`row_id` 是表主键，
/// `row_updated_at` 是 `updated_at` 列（规范格式），坏的 createdAt / updatedAt 用它兜底。
///
/// 旧版 API 写进来的别名字段：
/// - `priority`：能识别的优先级且当前颜色缺失 / 无效 / 是三种优先级色之一 → `color` =
///   优先级色（缓存里还留着 `priority` 说明 PC 没有编辑过这条——PC 往返会丢掉它——所以
///   它代表 AI 最近一次明确的意图；旧版 API 新建时 color 默认 `#10B981` 并非用户选择）；
///   当前是自定义颜色时保留颜色
/// - `dueDate` → `endTime`（同理以 `dueDate` 为准：旧版 API 的过滤 / 排序都以它为主）
/// - `notes` → `description`（description 为空时直接替换，否则追加在后面，不丢内容）
///
/// 字段类型修复只针对会让 PC 反序列化整条失败的情况；未知字段原样保留（可能是更新版本
/// PC 的新字段）。
pub fn normalize_stored_todo(
    obj: &mut Map<String, Value>,
    tz: Tz,
    row_id: &str,
    row_updated_at: &str,
) -> NormalizeOutcome {
    let mut f = Fixer {
        obj,
        out: NormalizeOutcome::default(),
        tz,
    };

    // ---- 旧版 API 的别名字段 ----
    if let Some(p) = f.remove("priority") {
        if let Some(priority) = p.as_str().and_then(Priority::parse) {
            let keep_custom = match f.obj.get("color") {
                Some(Value::String(c)) => {
                    is_hex_color(c.trim()) && Priority::from_color(c).is_none()
                }
                _ => false,
            };
            if !keep_custom {
                f.set("color", json!(priority.color()), true);
            }
        }
    }
    if let Some(due) = f.remove("dueDate") {
        match due {
            Value::String(s) if !s.trim().is_empty() => {
                match normalize_datetime(&s, tz, TimeField::EndTime.default_time()) {
                    Some(c) => f.set("endTime", json!(c), true),
                    None => tracing::warn!(
                        target: "minitodo_cloud::model",
                        "todo {}: drop unparseable legacy dueDate {:?}", row_id, s
                    ),
                }
            }
            _ => {}
        }
    }
    if let Some(notes) = f.remove("notes") {
        let notes = match notes {
            Value::String(s) => s,
            Value::Null => String::new(),
            other => other.to_string(),
        };
        if !notes.trim().is_empty() {
            let merged = match f.obj.get("description") {
                Some(Value::String(d)) if !d.trim().is_empty() => {
                    if d.contains(notes.trim()) {
                        d.clone()
                    } else {
                        format!("{}\n\n{}", d, notes)
                    }
                }
                _ => notes,
            };
            f.set("description", json!(merged), true);
        }
    }
    // 只读 / 派生字段不入库
    f.remove("seq");
    f.remove("subtaskCount");
    f.remove("subtasks");

    // ---- PC 强类型反序列化会失败的类型 ----
    f.id(row_id);
    f.title();
    f.nullable_string("description");
    f.color();
    f.quadrant();
    f.int_field("notifyBefore", 0, None);
    f.bool_field("notified");
    f.bool_field("completed");
    f.int_field("sortOrder", 0, None);
    f.bool_field("repeatEnabled");
    f.repeat_type();
    f.int_field("repeatInterval", 1, Some(1));
    f.weekdays();
    f.nullable_int_field("repeatMonthDay", 1..=31);

    // ---- 时间：可解析的统一成规范格式（仅格式），坏值置空（语义修复） ----
    for (field, kind) in TODO_TIME_FIELDS {
        f.time_field(field, kind.default_time());
    }
    let fallback = row_updated_at.to_string();
    f.meta_time("createdAt", &fallback);
    f.meta_time("updatedAt", &fallback);

    f.out
}

/// 归一化缓存里的一条 subtask。`parentId` 对齐到所属 todo（行的 `todo_id` 列；
/// 嵌套关系就是父子关系，PC 合并时也以外层 todo 为准，所以这只是表示形式的修正）。
pub fn normalize_stored_subtask(
    obj: &mut Map<String, Value>,
    tz: Tz,
    row_id: &str,
    row_todo_id: &str,
    row_updated_at: &str,
) -> NormalizeOutcome {
    let mut f = Fixer {
        obj,
        out: NormalizeOutcome::default(),
        tz,
    };
    f.id(row_id);
    if let Ok(parent) = row_todo_id.parse::<i64>() {
        if f.obj.get("parentId").and_then(Value::as_i64) != Some(parent) {
            f.set("parentId", json!(parent), false);
        }
    }
    f.title();
    f.nullable_string("content");
    f.bool_field("completed");
    f.int_field("sortOrder", 0, None);
    let fallback = row_updated_at.to_string();
    f.meta_time("createdAt", &fallback);
    f.meta_time("updatedAt", &fallback);
    f.out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tz() -> Tz {
        "Asia/Shanghai".parse().unwrap()
    }

    fn obj(v: Value) -> Map<String, Value> {
        v.as_object().cloned().unwrap()
    }

    fn todo_ok(v: Value) -> Map<String, Value> {
        validate_todo_input(&v, WriteMode::Update, tz()).expect("valid")
    }

    fn todo_err(v: Value) -> ValidationError {
        validate_todo_input(&v, WriteMode::Update, tz()).expect_err("invalid")
    }

    // ------------------------------------------------------------------ 优先级

    #[test]
    fn priority_color_mapping() {
        assert_eq!(Priority::parse(" HIGH "), Some(Priority::High));
        assert_eq!(Priority::parse("urgent"), None);
        assert_eq!(Priority::from_color("#ef4444"), Some(Priority::High));
        assert_eq!(Priority::from_color("#F59E0B"), Some(Priority::Medium));
        assert_eq!(Priority::from_color("#10b981"), Some(Priority::Low));
        assert_eq!(Priority::from_color("#3B82F6"), None);
        assert_eq!(
            derived_priority(&obj(json!({"color": "#EF4444"}))),
            Some(Priority::High)
        );
        // 缺 color：PC 按 #F59E0B 呈现
        assert_eq!(derived_priority(&obj(json!({}))), Some(Priority::Medium));
        assert_eq!(derived_priority(&obj(json!({"color": 3}))), None);
    }

    #[test]
    fn todo_view_adds_derived_fields_only() {
        let v = todo_view(
            json!({"id": 1, "color": "#3B82F6", "priority": "high"}),
            Some(7),
        );
        assert_eq!(v["priority"], Value::Null, "自定义颜色的派生优先级为 null");
        assert_eq!(v["seq"], 7);
        let v = todo_view(json!({"id": 1, "color": "#10B981"}), None);
        assert_eq!(v["priority"], "low");
        assert!(v.get("seq").is_none());
    }

    #[test]
    fn quadrant_parsing() {
        assert_eq!(parse_quadrant_str("2"), Some(2));
        assert_eq!(parse_quadrant_str("5"), None);
        assert_eq!(parse_quadrant_str("Urgent_Important"), Some(1));
        assert_eq!(parse_quadrant_str("important_urgent"), Some(1));
        assert_eq!(parse_quadrant_str("not_urgent_not_important"), Some(4));
        assert_eq!(parse_quadrant_str("soon"), None);
    }

    // ------------------------------------------------------------------ 校验矩阵

    #[test]
    fn valid_todo_fields_are_normalized() {
        let p = todo_ok(json!({
            "title": "写周报",
            "description": null,
            "color": "#ef4444",
            "quadrant": "important_not_urgent",
            "notifyAt": "2026-05-20",
            "startTime": "2026-05-20T08:30",
            "endTime": "2026-05-20T10:00:00Z",
            "notifyBefore": 15,
            "notified": false,
            "completed": true,
            "sortOrder": -3,
            "repeatEnabled": true,
            "repeatType": "Weekly",
            "repeatInterval": 2,
            "repeatWeekdays": "5, 1,3,3",
            "repeatMonthDay": null
        }));
        assert_eq!(p["color"], "#EF4444");
        assert_eq!(p["quadrant"], 2);
        assert_eq!(
            p["notifyAt"], "2026-05-20 09:00:00",
            "仅日期的 notifyAt → 09:00"
        );
        assert_eq!(p["startTime"], "2026-05-20 08:30:00");
        assert_eq!(p["endTime"], "2026-05-20 18:00:00", "Z → 上海墙钟");
        assert_eq!(p["repeatType"], "weekly");
        assert_eq!(p["repeatWeekdays"], "1,3,5");
        assert_eq!(p["sortOrder"], -3);
        assert_eq!(p.len(), 16);
    }

    #[test]
    fn date_only_defaults_per_field() {
        let p = todo_ok(json!({"startTime": "2026-05-20", "endTime": "2026-05-20"}));
        assert_eq!(p["startTime"], "2026-05-20 00:00:00");
        assert_eq!(p["endTime"], "2026-05-20 23:59:00");
        let p = todo_ok(json!({"dueDate": "2026-05-21"}));
        assert_eq!(p["endTime"], "2026-05-21 23:59:00");
        assert!(p.get("dueDate").is_none());
    }

    #[test]
    fn empty_string_clears_nullable_fields() {
        let p = todo_ok(
            json!({"notifyAt": "", "endTime": "  ", "repeatType": "", "repeatWeekdays": ""}),
        );
        for f in ["notifyAt", "endTime", "repeatType", "repeatWeekdays"] {
            assert_eq!(p[f], Value::Null, "{}", f);
        }
        let p = todo_ok(json!({"repeatWeekdays": [7, 1]}));
        assert_eq!(p["repeatWeekdays"], "1,7");
        let p = todo_ok(json!({"repeatWeekdays": []}));
        assert_eq!(p["repeatWeekdays"], Value::Null);
    }

    #[test]
    fn aliases_map_to_canonical_fields_and_canonical_wins() {
        let p = todo_ok(json!({"priority": "High", "notes": "n", "dueDate": "2026-05-20 18:00"}));
        assert_eq!(p["color"], COLOR_HIGH);
        assert_eq!(p["description"], "n");
        assert_eq!(p["endTime"], "2026-05-20 18:00:00");
        assert!(p.get("priority").is_none() && p.get("notes").is_none());

        let p = todo_ok(json!({
            "priority": "high", "color": "#3B82F6",
            "notes": "alias", "description": "canonical",
            "dueDate": "2026-05-20", "endTime": "2026-05-22 10:00:00"
        }));
        assert_eq!(p["color"], "#3B82F6");
        assert_eq!(p["description"], "canonical");
        assert_eq!(p["endTime"], "2026-05-22 10:00:00");

        // 读出的派生 priority 为 null 时原样写回不报错，也不改颜色
        let p = todo_ok(json!({"priority": null, "title": "x"}));
        assert!(p.get("color").is_none());
    }

    #[test]
    fn alias_applies_when_canonical_field_is_unchanged() {
        let current =
            obj(json!({"color": "#10B981", "endTime": "2026-05-20 23:59:00", "description": "d"}));
        // 读出来改了 priority / dueDate / notes，color / endTime / description 原样带回
        let body = json!({
            "color": "#10B981", "priority": "high",
            "endTime": "2026-05-20 23:59:00", "dueDate": "2026-06-01",
            "description": "d", "notes": "new notes"
        });
        let mut p = todo_ok(body.clone());
        assert_eq!(p["color"], "#10B981", "校验阶段规范字段优先");
        apply_shadowed_aliases(&mut p, &body, &current, tz());
        assert_eq!(p["color"], COLOR_HIGH);
        assert_eq!(p["endTime"], "2026-06-01 23:59:00");
        assert_eq!(p["description"], "new notes");

        // 规范字段确实改了 → 仍以规范字段为准
        let body = json!({"color": "#3B82F6", "priority": "high"});
        let mut p = todo_ok(body.clone());
        apply_shadowed_aliases(&mut p, &body, &current, tz());
        assert_eq!(p["color"], "#3B82F6");
        // 派生值为 null 的 priority 原样写回：不动颜色
        let body = json!({"color": "#10B981", "priority": null});
        let mut p = todo_ok(body.clone());
        apply_shadowed_aliases(&mut p, &body, &current, tz());
        assert_eq!(p["color"], "#10B981");
    }

    #[test]
    fn server_fields_are_ignored() {
        let p = todo_ok(json!({
            "id": 5, "createdAt": "x", "updatedAt": "y", "seq": 3,
            "subtasks": [{"title": "s"}], "subtaskCount": 1, "completed": true
        }));
        assert_eq!(p, obj(json!({"completed": true})));
    }

    #[test]
    fn unknown_fields_are_rejected_with_allowed_list() {
        let e = todo_err(json!({"title": "x", "tags": ["a"], "due": "2026-05-20"}));
        assert_eq!(e.unknown, vec!["due", "tags"]);
        let msg = e.message();
        assert!(msg.contains("unknown field(s): due, tags"), "{}", msg);
        assert!(msg.contains("allowed:") && msg.contains("endTime") && msg.contains("dueDate"));
    }

    #[test]
    fn type_errors_name_the_field() {
        for (body, field) in [
            (json!({"title": ""}), "title"),
            (json!({"title": null}), "title"),
            (json!({"title": 3}), "title"),
            (json!({"description": 1}), "description"),
            (json!({"color": null}), "color"),
            (json!({"color": "red"}), "color"),
            (json!({"color": "#12345"}), "color"),
            (json!({"quadrant": 0}), "quadrant"),
            (json!({"quadrant": "urgent"}), "quadrant"),
            (json!({"quadrant": 1.5}), "quadrant"),
            (json!({"notifyAt": "tomorrow"}), "notifyAt"),
            (json!({"startTime": 20260520}), "startTime"),
            (json!({"endTime": "2026-02-30"}), "endTime"),
            (json!({"notifyBefore": -1}), "notifyBefore"),
            (json!({"notifyBefore": "5"}), "notifyBefore"),
            (json!({"notified": "true"}), "notified"),
            (json!({"completed": null}), "completed"),
            (json!({"completed": 1}), "completed"),
            (json!({"sortOrder": 3_000_000_000_i64}), "sortOrder"),
            (json!({"sortOrder": 1.5}), "sortOrder"),
            (json!({"repeatEnabled": "yes"}), "repeatEnabled"),
            (json!({"repeatType": "yearly"}), "repeatType"),
            (json!({"repeatInterval": 0}), "repeatInterval"),
            (json!({"repeatInterval": null}), "repeatInterval"),
            (json!({"repeatWeekdays": "1,8"}), "repeatWeekdays"),
            (json!({"repeatWeekdays": "mon"}), "repeatWeekdays"),
            (json!({"repeatMonthDay": 32}), "repeatMonthDay"),
            (json!({"priority": "urgent"}), "priority"),
            (json!({"dueDate": "someday"}), "dueDate"),
            (json!({"notes": 5}), "notes"),
        ] {
            let e = todo_err(body.clone());
            assert_eq!(e.invalid.len(), 1, "{} -> {:?}", body, e);
            assert_eq!(e.invalid[0].0, field, "{}", body);
            assert!(e.message().starts_with(field), "{}", e.message());
        }
    }

    #[test]
    fn all_problems_are_reported_together() {
        let e = todo_err(json!({"color": "x", "quadrant": 9, "foo": 1}));
        assert_eq!(e.unknown, vec!["foo"]);
        assert_eq!(e.invalid.len(), 2);
    }

    #[test]
    fn create_requires_title_and_body_must_be_object() {
        let e =
            validate_todo_input(&json!({"completed": true}), WriteMode::Create, tz()).unwrap_err();
        assert!(
            e.message().contains("title: is required"),
            "{}",
            e.message()
        );
        let e = validate_todo_input(&json!([1]), WriteMode::Create, tz()).unwrap_err();
        assert!(e.message().contains("body"));
        assert!(validate_todo_input(&json!({}), WriteMode::Update, tz()).is_ok());
    }

    #[test]
    fn subtask_validation_and_parent_guard() {
        let p = validate_subtask_input(
            &json!({"title": "s", "content": null, "completed": true, "sortOrder": 2,
                    "id": 1, "createdAt": "x", "updatedAt": "y", "parentId": 5}),
            WriteMode::Create,
            5,
        )
        .unwrap();
        assert_eq!(p.len(), 4);
        // parentId 字符串形式且相同 → 忽略
        assert!(validate_subtask_input(&json!({"parentId": "5"}), WriteMode::Update, 5).is_ok());
        let e = validate_subtask_input(&json!({"parentId": 6}), WriteMode::Update, 5).unwrap_err();
        assert_eq!(e.invalid[0].0, "parentId");
        assert!(e.message().contains("cannot be changed"));
        let e = validate_subtask_input(&json!({"done": true}), WriteMode::Update, 5).unwrap_err();
        assert_eq!(e.unknown, vec!["done"]);
        assert!(e
            .message()
            .contains("allowed: title, content, completed, sortOrder"));
        let e = validate_subtask_input(&json!({"content": "c"}), WriteMode::Create, 5).unwrap_err();
        assert_eq!(e.invalid[0].0, "title");
        let e =
            validate_subtask_input(&json!({"completed": "yes"}), WriteMode::Update, 5).unwrap_err();
        assert_eq!(e.invalid[0].0, "completed");
    }

    #[test]
    fn new_records_have_full_pc_shape() {
        let p = todo_ok(json!({"title": "t", "priority": "high"}));
        let r = new_todo_record(42, &p, "2026-05-13 10:00:00");
        assert_eq!(r["id"], 42);
        assert_eq!(r["title"], "t");
        assert_eq!(r["color"], COLOR_HIGH);
        assert_eq!(r["quadrant"], 4);
        assert_eq!(r["repeatInterval"], 1);
        assert_eq!(r["createdAt"], "2026-05-13 10:00:00");
        assert_eq!(r.len(), 19, "与 PC Todo 字段一一对应（不含 subtasks）");
        assert!(r.get("priority").is_none());

        let s = new_subtask_record(7, 42, &obj(json!({"title": "s"})), "2026-05-13 10:00:00");
        assert_eq!(s["parentId"], 42);
        assert_eq!(s.len(), 8);
    }

    #[test]
    fn patch_resets_notified_only_when_notify_time_changes() {
        let mut t =
            obj(json!({"notifyAt": "2026-05-20 09:00:00", "notified": true, "notifyBefore": 15}));
        apply_todo_patch(&mut t, &obj(json!({"notifyAt": "2026-05-20 09:00:00"})));
        assert_eq!(t["notified"], true, "同一时间原样写回不重置");
        apply_todo_patch(&mut t, &obj(json!({"notifyAt": "2026-05-21 09:00:00"})));
        assert_eq!(t["notified"], false);
        t.insert("notified".into(), json!(true));
        apply_todo_patch(
            &mut t,
            &obj(json!({"notifyAt": "2026-05-22 09:00:00", "notified": true})),
        );
        assert_eq!(t["notified"], true, "显式给出的 notified 优先");
        apply_todo_patch(&mut t, &obj(json!({"repeatEnabled": true})));
        assert_eq!(t["notifyBefore"], 0, "开启重复 → 准点提醒");
    }

    // ------------------------------------------------------------------ 存量归一化

    fn norm_todo(v: Value) -> (Map<String, Value>, NormalizeOutcome) {
        let mut o = obj(v);
        let out = normalize_stored_todo(&mut o, tz(), "1", "2026-05-13 10:00:00");
        (o, out)
    }

    fn pc_shaped() -> Value {
        json!({
            "id": 1, "title": "t", "description": null, "color": "#10B981", "quadrant": 4,
            "notifyAt": null, "notifyBefore": 0, "notified": false, "completed": false,
            "sortOrder": 0, "startTime": null, "endTime": "2026-05-20 23:59:00",
            "createdAt": "2026-05-13 10:00:00", "updatedAt": "2026-05-13 10:00:00",
            "repeatEnabled": false, "repeatType": null, "repeatInterval": 1,
            "repeatWeekdays": null, "repeatMonthDay": null, "futurePcField": {"x": 1}
        })
    }

    #[test]
    fn pc_shaped_record_is_untouched() {
        let (o, out) = norm_todo(pc_shaped());
        assert_eq!(out, NormalizeOutcome::default());
        assert_eq!(Value::Object(o), pc_shaped(), "未知字段原样保留");
        let (_, out) = norm_todo(json!({"id": 1, "title": "minimal"}));
        assert_eq!(
            out,
            NormalizeOutcome::default(),
            "缺字段由 PC 的 serde 默认值补齐，不需要修"
        );
    }

    #[test]
    fn legacy_priority_overrides_old_default_color_but_not_custom_color() {
        // 旧版 API 新建：priority=high，color 是默认的 #10B981
        let (o, out) =
            norm_todo(json!({"id": 1, "title": "t", "priority": "high", "color": "#10B981"}));
        assert!(out.semantic);
        assert_eq!(o["color"], COLOR_HIGH);
        assert!(o.get("priority").is_none());
        // 自定义颜色保留
        let (o, out) =
            norm_todo(json!({"id": 1, "title": "t", "priority": "high", "color": "#3B82F6"}));
        assert!(!out.semantic && out.format);
        assert_eq!(o["color"], "#3B82F6");
        // 颜色本来就一致：只是去掉冗余键
        let (o, out) =
            norm_todo(json!({"id": 1, "title": "t", "priority": "low", "color": "#10B981"}));
        assert_eq!(
            out,
            NormalizeOutcome {
                semantic: false,
                format: true
            }
        );
        assert_eq!(o["color"], COLOR_LOW);
        // 不认识的优先级：丢掉
        let (o, _) =
            norm_todo(json!({"id": 1, "title": "t", "priority": "urgent", "color": "#10B981"}));
        assert_eq!(o["color"], COLOR_LOW);
        assert!(o.get("priority").is_none());
    }

    #[test]
    fn legacy_due_date_and_notes_are_folded_in() {
        let (o, out) =
            norm_todo(json!({"id": 1, "title": "t", "dueDate": "2026-05-20", "notes": "n"}));
        assert!(out.semantic);
        assert_eq!(o["endTime"], "2026-05-20 23:59:00");
        assert_eq!(o["description"], "n");
        assert!(o.get("dueDate").is_none() && o.get("notes").is_none());

        let (o, _) = norm_todo(json!({
            "id": 1, "title": "t", "dueDate": "2026-05-21 18:00", "endTime": "2026-05-20 23:59:00",
            "description": "desc", "notes": "more"
        }));
        assert_eq!(
            o["endTime"], "2026-05-21 18:00:00",
            "旧版 API 以 dueDate 为准"
        );
        assert_eq!(o["description"], "desc\n\nmore", "不丢 notes 的内容");

        let (o, out) = norm_todo(json!({"id": 1, "title": "t", "dueDate": "someday"}));
        assert!(o.get("dueDate").is_none() && o.get("endTime").is_none());
        assert!(!out.semantic && out.format);
    }

    #[test]
    fn types_that_break_pc_deserialization_are_repaired() {
        let (o, out) = norm_todo(json!({
            "id": "1", "title": null, "description": 5, "color": null,
            "quadrant": "urgent_important", "notifyBefore": null, "notified": 1,
            "completed": "true", "sortOrder": 2.0, "repeatEnabled": null,
            "repeatType": "Daily", "repeatInterval": 0, "repeatWeekdays": [3, 1],
            "repeatMonthDay": "15", "notifyAt": "", "startTime": "garbage",
            "createdAt": null, "updatedAt": "2026-05-13T10:00:00"
        }));
        assert!(out.semantic);
        assert_eq!(o["id"], 1);
        assert_eq!(o["title"], UNTITLED);
        assert_eq!(o["description"], "5");
        assert_eq!(o["color"], PC_FALLBACK_COLOR);
        assert_eq!(o["quadrant"], 1);
        assert_eq!(o["notifyBefore"], 0);
        assert_eq!(o["notified"], true);
        assert_eq!(o["completed"], true);
        assert_eq!(o["sortOrder"], 2);
        assert_eq!(o["repeatEnabled"], false);
        assert_eq!(o["repeatType"], "daily");
        assert_eq!(o["repeatInterval"], 1);
        assert_eq!(o["repeatWeekdays"], "1,3");
        assert_eq!(o["repeatMonthDay"], 15);
        assert_eq!(o["notifyAt"], Value::Null);
        assert_eq!(o["startTime"], Value::Null);
        assert_eq!(o["createdAt"], "2026-05-13 10:00:00");
        assert_eq!(o["updatedAt"], "2026-05-13 10:00:00");
    }

    #[test]
    fn time_format_only_changes_are_not_semantic() {
        let (o, out) = norm_todo(json!({
            "id": 1, "title": "t", "color": "#ef4444",
            "notifyAt": "2026-05-20T09:00", "endTime": "2026-05-20",
            "createdAt": "2026-05-13T08:00:00", "seq": 3, "subtaskCount": 2
        }));
        assert_eq!(
            out,
            NormalizeOutcome {
                semantic: false,
                format: true
            }
        );
        assert_eq!(o["notifyAt"], "2026-05-20 09:00:00");
        assert_eq!(o["endTime"], "2026-05-20 23:59:00");
        assert_eq!(o["createdAt"], "2026-05-13 08:00:00");
        assert_eq!(o["color"], "#EF4444");
        assert!(o.get("seq").is_none() && o.get("subtaskCount").is_none());
    }

    #[test]
    fn subtask_normalization() {
        let mut s = obj(json!({
            "id": 9, "parentId": 999, "title": "s", "content": null, "completed": 0,
            "sortOrder": 1, "createdAt": "2026-05-13T08:00:00", "updatedAt": "2026-05-13 10:00:00",
            "extra": true
        }));
        let out = normalize_stored_subtask(&mut s, tz(), "9", "5", "2026-05-13 10:00:00");
        assert!(out.semantic, "completed: 0 → false 是语义修复");
        assert_eq!(s["parentId"], 5);
        assert_eq!(s["completed"], false);
        assert_eq!(s["createdAt"], "2026-05-13 08:00:00");
        assert_eq!(s["extra"], true);
        let mut ok = obj(json!({"id": 9, "parentId": 5, "title": "s", "completed": false}));
        assert_eq!(
            normalize_stored_subtask(&mut ok, tz(), "9", "5", "x"),
            NormalizeOutcome::default()
        );
    }

    #[test]
    fn canonicalize_times_is_format_only() {
        let mut t = obj(json!({
            "notifyAt": "2026-05-20T09:00", "startTime": "", "endTime": "garbage",
            "createdAt": "2026-05-13 08:00:00", "updatedAt": "2026-05-13T10:00:00+08:00",
            "title": "2026-05-20T09:00"
        }));
        assert!(canonicalize_record_times(&mut t, true, tz()));
        assert_eq!(t["notifyAt"], "2026-05-20 09:00:00");
        assert_eq!(t["startTime"], "", "空串不动");
        assert_eq!(t["endTime"], "garbage", "坏值不动（合并时不改语义）");
        assert_eq!(t["updatedAt"], "2026-05-13 10:00:00");
        assert_eq!(t["title"], "2026-05-20T09:00");
        assert!(!canonicalize_record_times(&mut t, true, tz()), "幂等");
        let mut s = obj(json!({"notifyAt": "2026-05-20T09:00"}));
        assert!(
            !canonicalize_record_times(&mut s, false, tz()),
            "子任务没有业务时间字段"
        );
    }
}
