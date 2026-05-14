//! `/todos` CRUD。
//!
//! 写路径统一：变更 → `repo::set_meta(conn, "dirty", "true")` 唤醒 push worker。
//! merge 语义：PATCH 把请求 body 的字段覆盖到 `data_json` 上，未提及字段保留
//! （包括 PC 端 v24/v25 加的未知字段也透传）。

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use rusqlite::Connection;
use serde::Deserialize;
use serde_json::{json, Value};

use super::error::ApiError;
use super::ids::new_id_string;
use super::AppState;
use crate::db::repo::{self, ListTodosFilter};
use crate::time::now_local_string;

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
// GET /todos
// =============================================================================

pub async fn list_todos(
    State(state): State<AppState>,
    Query(q): Query<ListTodosQuery>,
) -> Result<Json<Value>, ApiError> {
    let filter = parse_list_filter(&q)?;
    let with_subtasks = parse_bool_flag(&q.with_subtasks);

    let todos = state.db.with_conn(|conn| -> rusqlite::Result<Value> {
        let rows = repo::list_todos_filtered(conn, &filter)?;
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let mut v: Value = serde_json::from_str(&row.data_json)
                .unwrap_or_else(|_| json!({"id": row.id, "raw": row.data_json}));
            let id_str = id_field_as_string(&v).unwrap_or_else(|| row.id.clone());

            if with_subtasks {
                let subs = repo::list_subtasks_for_todo(conn, &id_str)?;
                let subs_vals: Vec<Value> = subs
                    .into_iter()
                    .map(|s| {
                        serde_json::from_str::<Value>(&s.data_json)
                            .unwrap_or_else(|_| json!({"id": s.id}))
                    })
                    .collect();
                v["subtasks"] = Value::Array(subs_vals);
            } else {
                let n = repo::count_subtasks_for_todo(conn, &id_str)?;
                v["subtaskCount"] = json!(n);
                // 不嵌套时移除已有 subtasks 数组以避免 token 浪费（保留 count）
                if v.get("subtasks").is_some() {
                    if let Value::Object(ref mut map) = v {
                        map.remove("subtasks");
                    }
                }
            }
            out.push(v);
        }
        Ok(Value::Array(out))
    })?;

    Ok(Json(todos))
}

// =============================================================================
// GET /todos/:id
// =============================================================================

pub async fn get_todo(
    State(state): State<AppState>,
    Path(id): Path<String>,
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
            let Some(row) = repo::get_todo(conn, &id)? else {
                return Ok(None);
            };
            let mut v: Value =
                serde_json::from_str(&row.data_json).unwrap_or_else(|_| json!({"id": row.id}));
            if with_subtasks {
                let subs = repo::list_subtasks_for_todo(conn, &id)?;
                let subs_vals: Vec<Value> = subs
                    .into_iter()
                    .map(|s| {
                        serde_json::from_str::<Value>(&s.data_json)
                            .unwrap_or_else(|_| json!({"id": s.id}))
                    })
                    .collect();
                v["subtasks"] = Value::Array(subs_vals);
            } else if let Value::Object(ref mut map) = v {
                map.remove("subtasks");
                let n = repo::count_subtasks_for_todo(conn, &id)?;
                v["subtaskCount"] = json!(n);
            }
            Ok(Some(v))
        })?;

    match res {
        Some(v) => Ok(Json(v)),
        None => Err(ApiError::not_found(format!("todo {} not found", id))),
    }
}

// =============================================================================
// POST /todos
// =============================================================================

pub async fn create_todo(
    State(state): State<AppState>,
    Json(body): Json<Value>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    if !body.is_object() {
        return Err(ApiError::bad_request("body must be a JSON object"));
    }
    let title = body
        .get("title")
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| ApiError::bad_request("title is required"))?
        .to_string();

    let now = now_local_string(state.config.timezone_offset);
    let id_str = new_id_string();

    let mut obj = body.as_object().cloned().unwrap_or_default();
    // id 强制由服务端生成（i64 数字形式，与 PC SQLite AUTOINCREMENT 兼容）
    obj.insert("id".into(), json!(id_str.parse::<i64>().unwrap_or(0)));
    obj.insert("title".into(), json!(title));
    obj.entry("createdAt").or_insert(json!(now.clone()));
    obj.insert("updatedAt".into(), json!(now.clone()));
    obj.entry("completed").or_insert(json!(false));
    obj.entry("color").or_insert(json!("#10B981"));
    obj.entry("quadrant").or_insert(json!(4));
    obj.entry("sortOrder").or_insert(json!(0));
    obj.entry("notifyBefore").or_insert(json!(0));
    obj.entry("notified").or_insert(json!(false));

    let v = Value::Object(obj);
    let body_str = v.to_string();

    state.db.with_conn(|conn| -> rusqlite::Result<()> {
        repo::upsert_todo(conn, &id_str, &body_str, &now)?;
        repo::set_meta(conn, "dirty", "true")?;
        Ok(())
    })?;

    Ok((StatusCode::CREATED, Json(v)))
}

// =============================================================================
// PATCH /todos/:id
// =============================================================================

pub async fn patch_todo(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    if !body.is_object() {
        return Err(ApiError::bad_request("body must be a JSON object"));
    }

    let now = now_local_string(state.config.timezone_offset);

    let updated: Option<Value> = state
        .db
        .with_conn(|conn| -> rusqlite::Result<Option<Value>> {
            let Some(row) = repo::get_todo(conn, &id)? else {
                return Ok(None);
            };
            let mut current: Value =
                serde_json::from_str(&row.data_json).unwrap_or_else(|_| json!({"id": row.id}));
            merge_json_shallow(&mut current, &body);
            // 防止 PATCH body 改 id
            if let Some(obj) = current.as_object_mut() {
                obj.insert("id".into(), json!(id.parse::<i64>().unwrap_or(0)));
                obj.insert("updatedAt".into(), json!(now.clone()));
            }
            let body_str = current.to_string();
            repo::upsert_todo(conn, &id, &body_str, &now)?;
            repo::set_meta(conn, "dirty", "true")?;
            Ok(Some(current))
        })?;

    match updated {
        Some(v) => Ok(Json(v)),
        None => Err(ApiError::not_found(format!("todo {} not found", id))),
    }
}

// =============================================================================
// DELETE /todos/:id
// =============================================================================

pub async fn delete_todo(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let now = now_local_string(state.config.timezone_offset);
    let removed = state.db.with_conn(|conn| -> rusqlite::Result<bool> {
        let tx = conn.transaction()?;
        // 先收集子任务 id：`delete_todo_cascade` 会把 subtasks 一起删掉，
        // 若放在 cascade 之后再 query 就拿不到任何 id，导致 subtask tombstones 漏写。
        let sub_ids: Vec<String> = tx
            .prepare("SELECT id FROM subtasks WHERE todo_id = ?1")?
            .query_map([&id], |r| r.get::<_, String>(0))?
            .filter_map(|r| r.ok())
            .collect();

        let existed = repo::delete_todo_cascade(&tx, &id)?;
        if existed {
            repo::add_tombstone(&tx, TOMBSTONE_TODO, &id, &now)?;
            for sid in sub_ids {
                repo::add_tombstone(&tx, "subtask", &sid, &now)?;
            }
            repo::set_meta(&tx, "dirty", "true")?;
        }
        tx.commit()?;
        Ok(existed)
    })?;
    if removed {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::not_found(format!("todo {} not found", id)))
    }
}

// =============================================================================
// 工具
// =============================================================================

fn parse_list_filter(q: &ListTodosQuery) -> Result<ListTodosFilter, ApiError> {
    let completed = match q.completed.as_deref() {
        None => None,
        Some(s) => Some(
            parse_bool(s)
                .ok_or_else(|| ApiError::bad_request(format!("invalid completed flag: {}", s)))?,
        ),
    };
    let quadrant = match q.quadrant.as_deref() {
        None => None,
        Some(s) => Some(
            parse_quadrant(s)
                .ok_or_else(|| ApiError::bad_request(format!("invalid quadrant: {}", s)))?,
        ),
    };
    let sort = q.sort.as_deref().map(parse_sort);
    let limit = q.limit.and_then(|l| if l > 0 { Some(l) } else { None });
    let offset = q.offset.and_then(|o| if o >= 0 { Some(o) } else { None });

    Ok(ListTodosFilter {
        completed,
        priority: q.priority.clone(),
        quadrant,
        due_date_before: q.due_date_before.clone(),
        due_date_after: q.due_date_after.clone(),
        start_date: q.start_date.clone(),
        q: q.q.clone(),
        sort,
        limit,
        offset,
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

/// 接受数字（1-4）或字符串别名。
fn parse_quadrant(s: &str) -> Option<i64> {
    if let Ok(n) = s.parse::<i64>() {
        if (1..=4).contains(&n) {
            return Some(n);
        }
    }
    match s.to_ascii_lowercase().as_str() {
        "urgent_important" | "important_urgent" => Some(1),
        "important_not_urgent" => Some(2),
        "urgent_not_important" => Some(3),
        "not_urgent_not_important" => Some(4),
        _ => None,
    }
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

/// 浅合并：把 `patch` 的 top-level 字段覆盖到 `target`；patch 中的 `null` 也写入
/// （表示显式置空）。这与 PC 端 PATCH 语义对齐。
fn merge_json_shallow(target: &mut Value, patch: &Value) {
    let (Value::Object(t), Value::Object(p)) = (target, patch) else {
        return;
    };
    for (k, v) in p {
        if k == "id" {
            continue; // id 不能改
        }
        t.insert(k.clone(), v.clone());
    }
}

// 复用 util 模块实现（push / pull 也用同一份）。
use crate::util::id_string as id_field_as_string;

/// 子任务嵌套创建工具：供 subtasks 模块共用。
pub(crate) fn ensure_todo_exists(conn: &Connection, id: &str) -> Result<(), ApiError> {
    match repo::get_todo(conn, id) {
        Ok(Some(_)) => Ok(()),
        Ok(None) => Err(ApiError::not_found(format!("todo {} not found", id))),
        Err(e) => Err(e.into()),
    }
}
