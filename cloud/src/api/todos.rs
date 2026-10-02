//! `/todos` CRUD。
//!
//! 写入（K6）：body 先经 `model::validate_todo_input` 校验——只接受 PC `Todo` 模型的
//! 字段与类型，别名归一化，时间统一成 `YYYY-MM-DD HH:MM:SS`；未知字段 / 类型错误 → 400。
//! 每个写操作的全部语句在一个事务里；变更 → `repo::mark_dirty` 唤醒 push worker。
//!
//! PATCH 语义：校验后的字段覆盖到 `data_json` 上，未提及字段保留（包括 PC 新版本加的
//! 未知字段）；补丁不改变任何内容时（例如把读到的对象原样写回）不刷新 `updatedAt`、
//! 不标脏——否则一次无意义的回写会在 LWW 里压过 PC 还没同步上来的编辑。
//!
//! 读取：响应附加派生字段 `priority`（由 color 映射）与 cloud 短码 `seq`，都不入库。

use std::collections::HashMap;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use chrono::NaiveTime;
use chrono_tz::Tz;
use rusqlite::Connection;
use serde::Deserialize;
use serde_json::{json, Map, Value};

use super::error::{ApiError, ApiJson};
use super::ids::{insert_with_fresh_id, new_id};
use super::AppState;
use crate::db::repo::{self, ListTodosFilter, SubtaskRow};
use crate::model::{self, Priority, WriteMode};
use crate::time::{now_local_string, parse_datetime};

const TOMBSTONE_TODO: &str = "todo";

// =============================================================================
// Query 参数
// =============================================================================

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListTodosQuery {
    pub completed: Option<String>,
    pub priority: Option<String>,
    pub quadrant: Option<String>,
    pub due_date_before: Option<String>,
    pub due_date_after: Option<String>,
    pub start_date: Option<String>,
    pub q: Option<String>,
    pub sort: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
    pub with_subtasks: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetTodoQuery {
    pub with_subtasks: Option<String>,
}

// =============================================================================
// 读取
// =============================================================================

/// `data_json` → JSON 对象；损坏的行退化为只有 id（不让一条坏数据拖垮整个列表）。
fn row_value(data_json: &str, id: &str) -> Value {
    match serde_json::from_str::<Value>(data_json) {
        Ok(v @ Value::Object(_)) => v,
        _ => json!({ "id": id.parse::<i64>().map(Value::from).unwrap_or_else(|_| json!(id)) }),
    }
}

fn subtask_values(rows: Vec<SubtaskRow>) -> Value {
    Value::Array(
        rows.into_iter()
            .map(|s| row_value(&s.data_json, &s.id))
            .collect(),
    )
}

/// 组装一条 todo 响应：嵌套子任务或只给数量，再加派生字段。
fn todo_response(
    mut v: Value,
    seq: Option<i64>,
    subtasks: Option<Vec<SubtaskRow>>,
    subtask_count: i64,
) -> Value {
    if let Value::Object(obj) = &mut v {
        match subtasks {
            Some(rows) => {
                obj.insert("subtasks".into(), subtask_values(rows));
            }
            None => {
                // 不嵌套时去掉可能残留的 subtasks 数组（省 token），只给数量
                obj.remove("subtasks");
                obj.insert("subtaskCount".into(), json!(subtask_count));
            }
        }
    }
    model::todo_view(v, seq)
}

// =============================================================================
// GET /todos
// =============================================================================

pub async fn list_todos(
    State(state): State<AppState>,
    Query(q): Query<ListTodosQuery>,
) -> Result<Json<Value>, ApiError> {
    let filter = parse_list_filter(&q, state.config.timezone)?;
    let with_subtasks = parse_bool_flag(&q.with_subtasks);

    let todos = state.db.with_conn(|conn| -> rusqlite::Result<Value> {
        let rows = repo::list_todos_filtered(conn, &filter)?;
        let ids: Vec<String> = rows.iter().map(|r| r.id.clone()).collect();
        // 每类附加信息一次查询（不随条数增长），避免 N+1
        let seqs = repo::seq_map_for_todos(conn, &ids)?;
        let (mut subs, counts) = if with_subtasks {
            (repo::subtasks_for_todos(conn, &ids)?, HashMap::new())
        } else {
            (HashMap::new(), repo::subtask_counts_for_todos(conn, &ids)?)
        };
        let out = rows
            .into_iter()
            .map(|row| {
                let nested = with_subtasks.then(|| subs.remove(&row.id).unwrap_or_default());
                let count = counts.get(&row.id).copied().unwrap_or(0);
                todo_response(
                    row_value(&row.data_json, &row.id),
                    seqs.get(&row.id).copied(),
                    nested,
                    count,
                )
            })
            .collect();
        Ok(Value::Array(out))
    })?;

    Ok(Json(todos))
}

// =============================================================================
// GET /todos/:id
// =============================================================================

pub async fn get_todo(
    State(state): State<AppState>,
    Path(raw_id): Path<String>,
    Query(q): Query<GetTodoQuery>,
) -> Result<Json<Value>, ApiError> {
    // 默认 detail 是嵌套；显式 ?withSubtasks=false 才扁平
    let with_subtasks = q
        .with_subtasks
        .as_deref()
        .map(|s| !matches!(s.to_ascii_lowercase().as_str(), "false" | "0" | "no"))
        .unwrap_or(true);

    let res = state
        .db
        .with_conn(|conn| -> rusqlite::Result<Option<Value>> {
            let Some(id) = resolve_todo_ref(conn, &raw_id)? else {
                return Ok(None);
            };
            let Some(row) = repo::get_todo(conn, &id)? else {
                return Ok(None);
            };
            let seq = repo::get_seq(conn, &id)?;
            let (nested, count) = if with_subtasks {
                (Some(repo::list_subtasks_for_todo(conn, &id)?), 0)
            } else {
                (None, repo::count_subtasks_for_todo(conn, &id)?)
            };
            Ok(Some(todo_response(
                row_value(&row.data_json, &row.id),
                seq,
                nested,
                count,
            )))
        })?;

    match res {
        Some(v) => Ok(Json(v)),
        None => Err(ApiError::not_found(format!("todo {} not found", raw_id))),
    }
}

// =============================================================================
// POST /todos
// =============================================================================

pub async fn create_todo(
    State(state): State<AppState>,
    ApiJson(body): ApiJson<Value>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let tz = state.config.timezone;
    let patch =
        model::validate_todo_input(&body, WriteMode::Create, tz).map_err(ApiError::validation)?;
    let now = now_local_string(tz);

    // 插入 + 分配 seq + 标脏在同一事务里。id 冲突换号重试，绝不覆盖已有记录。
    // seq 是 cloud-only 字段，存独立的 `todo_seq` 表，不进 data_json
    // （PC 往返会把它丢掉，见 schema.rs）。
    let (record, seq) =
        state
            .db
            .with_conn(|conn| -> rusqlite::Result<(Map<String, Value>, i64)> {
                let tx = conn.transaction()?;
                let mut record = Map::new();
                let id = insert_with_fresh_id(new_id, |id| {
                    record = model::new_todo_record(id, &patch, &now);
                    repo::insert_todo(
                        &tx,
                        &id.to_string(),
                        &Value::Object(record.clone()).to_string(),
                        &now,
                    )
                })?;
                let seq = repo::assign_seq(&tx, &id.to_string())?;
                repo::mark_dirty(&tx)?;
                tx.commit()?;
                Ok((record, seq))
            })?;

    Ok((
        StatusCode::CREATED,
        Json(model::todo_view(Value::Object(record), Some(seq))),
    ))
}

// =============================================================================
// PATCH /todos/:id
// =============================================================================

pub async fn patch_todo(
    State(state): State<AppState>,
    Path(raw_id): Path<String>,
    ApiJson(body): ApiJson<Value>,
) -> Result<Json<Value>, ApiError> {
    let tz = state.config.timezone;
    let patch =
        model::validate_todo_input(&body, WriteMode::Update, tz).map_err(ApiError::validation)?;
    let now = now_local_string(tz);

    let updated = state
        .db
        .with_conn(|conn| -> rusqlite::Result<Option<Value>> {
            let tx = conn.transaction()?;
            let Some(id) = resolve_todo_ref(&tx, &raw_id)? else {
                return Ok(None);
            };
            let Some(row) = repo::get_todo(&tx, &id)? else {
                return Ok(None);
            };
            let mut base = match serde_json::from_str::<Value>(&row.data_json) {
                Ok(Value::Object(m)) => m,
                _ => Map::new(),
            };
            // 旧版 API 写进来的别名 / 错误类型先按 K6 修好，再应用补丁
            let normalized = model::normalize_stored_todo(&mut base, tz, &id, &row.updated_at);
            // 读出来改别名（priority / dueDate / notes）再整包写回时，别名的修改要生效
            let mut patch = patch.clone();
            model::apply_shadowed_aliases(&mut patch, &body, &base, tz);
            let mut next = base.clone();
            model::apply_todo_patch(&mut next, &patch);

            let changed = normalized.semantic || next != base;
            if changed {
                next.insert("updatedAt".into(), json!(now));
                repo::upsert_todo(&tx, &id, &Value::Object(next.clone()).to_string(), &now)?;
                repo::mark_dirty(&tx)?;
            } else if normalized.format {
                // 只有表示形式变了：原地改写，不刷新 updatedAt、不标脏
                repo::upsert_todo(
                    &tx,
                    &id,
                    &Value::Object(next.clone()).to_string(),
                    &row.updated_at,
                )?;
            }
            let seq = repo::get_seq(&tx, &id)?;
            tx.commit()?;
            Ok(Some(model::todo_view(Value::Object(next), seq)))
        })?;

    match updated {
        Some(v) => Ok(Json(v)),
        None => Err(ApiError::not_found(format!("todo {} not found", raw_id))),
    }
}

// =============================================================================
// DELETE /todos/:id
// =============================================================================

pub async fn delete_todo(
    State(state): State<AppState>,
    Path(raw_id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let now = now_local_string(state.config.timezone);
    let removed = state.db.with_conn(|conn| -> rusqlite::Result<bool> {
        let tx = conn.transaction()?;
        let id = match resolve_todo_ref(&tx, &raw_id)? {
            Some(id) => id,
            None => {
                tx.commit()?;
                return Ok(false);
            }
        };
        // 先收集子任务 id：`delete_todo_cascade` 会把 subtasks 一起删掉，
        // 若放在 cascade 之后再 query 就拿不到任何 id，导致 subtask tombstones 漏写。
        let sub_ids: Vec<String> = tx
            .prepare("SELECT id FROM subtasks WHERE todo_id = ?1")?
            .query_map([&id], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<_>>()?;

        let existed = repo::delete_todo_cascade(&tx, &id)?;
        if existed {
            repo::add_tombstone(&tx, TOMBSTONE_TODO, &id, &now)?;
            for sid in sub_ids {
                repo::add_tombstone(&tx, "subtask", &sid, &now)?;
            }
            repo::delete_seq(&tx, &id)?;
            repo::mark_dirty(&tx)?;
        }
        tx.commit()?;
        Ok(existed)
    })?;
    if removed {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::not_found(format!("todo {} not found", raw_id)))
    }
}

// =============================================================================
// 工具
// =============================================================================

fn parse_list_filter(q: &ListTodosQuery, tz: Tz) -> Result<ListTodosFilter, ApiError> {
    let completed = match q.completed.as_deref() {
        None => None,
        Some(s) => Some(
            parse_bool(s)
                .ok_or_else(|| ApiError::bad_request(format!("invalid completed flag: {}", s)))?,
        ),
    };
    // 优先级是 color 的派生值：按颜色过滤
    let priority_color = match q.priority.as_deref() {
        None => None,
        Some(s) => Some(
            Priority::parse(s)
                .ok_or_else(|| {
                    ApiError::bad_request(format!(
                        "invalid priority: {} (expected high, medium or low)",
                        s
                    ))
                })?
                .color()
                .to_string(),
        ),
    };
    let quadrant = match q.quadrant.as_deref() {
        None => None,
        Some(s) => Some(
            model::parse_quadrant_str(s)
                .ok_or_else(|| ApiError::bad_request(format!("invalid quadrant: {}", s)))?,
        ),
    };
    // 时间参数按 K1 规范化后再做字符串比较；仅日期：Before → 当天 23:59:59（含当天），
    // After → 当天 00:00:00
    let end_of_day = NaiveTime::from_hms_opt(23, 59, 59).expect("valid time");
    let due_date_before = q
        .due_date_before
        .as_deref()
        .map(|s| normalize_param("dueDateBefore", s, tz, end_of_day))
        .transpose()?;
    let due_date_after = q
        .due_date_after
        .as_deref()
        .map(|s| normalize_param("dueDateAfter", s, tz, NaiveTime::MIN))
        .transpose()?;
    let start_date = q
        .start_date
        .as_deref()
        .map(|s| {
            parse_datetime(s, tz, NaiveTime::MIN)
                .map(|d| d.format("%Y-%m-%d").to_string())
                .ok_or_else(|| {
                    ApiError::bad_request(format!("invalid startDate: {} (expected YYYY-MM-DD)", s))
                })
        })
        .transpose()?;
    let sort = q.sort.as_deref().map(parse_sort);
    let limit = q.limit.filter(|&l| l > 0);
    let offset = q.offset.filter(|&o| o >= 0);

    Ok(ListTodosFilter {
        completed,
        priority_color,
        quadrant,
        due_date_before,
        due_date_after,
        start_date,
        q: q.q.clone(),
        sort,
        limit,
        offset,
    })
}

fn normalize_param(
    name: &str,
    raw: &str,
    tz: Tz,
    default_time: NaiveTime,
) -> Result<String, ApiError> {
    crate::time::normalize_datetime(raw, tz, default_time).ok_or_else(|| {
        ApiError::bad_request(format!(
            "invalid {}: {} (expected 'YYYY-MM-DD' or 'YYYY-MM-DD HH:MM:SS')",
            name, raw
        ))
    })
}

fn parse_bool(s: &str) -> Option<bool> {
    match s.to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" => Some(true),
        "false" | "0" | "no" => Some(false),
        _ => None,
    }
}

fn parse_bool_flag(s: &Option<String>) -> bool {
    s.as_deref().and_then(parse_bool).unwrap_or(false)
}

fn parse_sort(s: &str) -> (String, bool) {
    if let Some(rest) = s.strip_prefix('-') {
        (rest.to_string(), false)
    } else if let Some(rest) = s.strip_prefix('+') {
        (rest.to_string(), true)
    } else {
        (s.to_string(), true)
    }
}

/// 解析 path 里的 `:id`。**只有两种语义**：
/// - `C{n}` / `c{n}`：把 n 当 cloud 短码 seq，反查内部 todo_id
/// - 纯字符串：当 i64 id 直查；查到才返回（否则 None → 上层 404）
///
/// 不做"裸数字先 try id 再 try seq"的兜底——PC 端来的 todo id 数值小（1..），
/// cloud seq 也从 1 起，二者会撞，必须靠 `C` 前缀消歧。
pub(crate) fn resolve_todo_ref(conn: &Connection, raw: &str) -> rusqlite::Result<Option<String>> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    if let Some(rest) = trimmed.strip_prefix(['C', 'c']) {
        return match rest.parse::<i64>() {
            Ok(seq) => repo::get_todo_id_by_seq(conn, seq),
            Err(_) => Ok(None),
        };
    }
    if repo::get_todo(conn, trimmed)?.is_some() {
        return Ok(Some(trimmed.to_string()));
    }
    Ok(None)
}

/// 子任务嵌套创建工具：供 subtasks 模块共用。返回**内部 todo_id**（解析过 ref）。
pub(crate) fn ensure_todo_exists(conn: &Connection, raw: &str) -> Result<String, ApiError> {
    match resolve_todo_ref(conn, raw) {
        Ok(Some(id)) => Ok(id),
        Ok(None) => Err(ApiError::not_found(format!("todo {} not found", raw))),
        Err(e) => Err(e.into()),
    }
}
