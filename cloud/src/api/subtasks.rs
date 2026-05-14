//! `/subtasks` CRUD（独立 PATCH/DELETE）+ 嵌于 `/todos/:id/subtasks` 的 POST。

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde_json::{json, Value};

use super::error::ApiError;
use super::ids::new_id_string;
use super::todos::ensure_todo_exists;
use super::AppState;
use crate::db::repo;
use crate::time::now_local_string;

const TOMBSTONE_SUBTASK: &str = "subtask";

// =============================================================================
// POST /todos/:id/subtasks
// =============================================================================

pub async fn create_subtask(
    State(state): State<AppState>,
    Path(raw_todo_ref): Path<String>,
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

    // 同一事务内解析父 todo ref（支持 C 短码）+ 写 subtask。
    let v = state.db.with_conn(|conn| -> Result<Value, ApiError> {
        let parent_id = ensure_todo_exists(conn, &raw_todo_ref)?;

        let mut obj = body.as_object().cloned().unwrap_or_default();
        obj.insert("id".into(), json!(id_str.parse::<i64>().unwrap_or(0)));
        obj.insert(
            "parentId".into(),
            json!(parent_id.parse::<i64>().unwrap_or(0)),
        );
        obj.insert("title".into(), json!(title));
        obj.entry("createdAt").or_insert(json!(now.clone()));
        obj.insert("updatedAt".into(), json!(now.clone()));
        obj.entry("completed").or_insert(json!(false));
        obj.entry("sortOrder").or_insert(json!(0));
        obj.entry("content").or_insert(json!(null));

        let v = Value::Object(obj);
        let body_str = v.to_string();

        repo::upsert_subtask(conn, &id_str, &parent_id, &body_str, &now)?;
        repo::set_meta(conn, "dirty", "true")?;
        Ok(v)
    })?;

    Ok((StatusCode::CREATED, Json(v)))
}

// =============================================================================
// PATCH /subtasks/:id
// =============================================================================

pub async fn patch_subtask(
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
            let Some(row) = repo::get_subtask(conn, &id)? else {
                return Ok(None);
            };
            let mut current: Value =
                serde_json::from_str(&row.data_json).unwrap_or_else(|_| json!({"id": row.id}));
            merge_json_shallow(&mut current, &body);
            if let Some(obj) = current.as_object_mut() {
                obj.insert("id".into(), json!(id.parse::<i64>().unwrap_or(0)));
                obj.insert("updatedAt".into(), json!(now.clone()));
            }
            let body_str = current.to_string();
            repo::upsert_subtask(conn, &id, &row.todo_id, &body_str, &now)?;
            repo::set_meta(conn, "dirty", "true")?;
            Ok(Some(current))
        })?;

    match updated {
        Some(v) => Ok(Json(v)),
        None => Err(ApiError::not_found(format!("subtask {} not found", id))),
    }
}

// =============================================================================
// DELETE /subtasks/:id
// =============================================================================

pub async fn delete_subtask(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let now = now_local_string(state.config.timezone_offset);
    let removed = state.db.with_conn(|conn| -> rusqlite::Result<bool> {
        let tx = conn.transaction()?;
        let existed = repo::delete_subtask(&tx, &id)?;
        if existed {
            repo::add_tombstone(&tx, TOMBSTONE_SUBTASK, &id, &now)?;
            repo::set_meta(&tx, "dirty", "true")?;
        }
        tx.commit()?;
        Ok(existed)
    })?;
    if removed {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::not_found(format!("subtask {} not found", id)))
    }
}

fn merge_json_shallow(target: &mut Value, patch: &Value) {
    let (Value::Object(t), Value::Object(p)) = (target, patch) else {
        return;
    };
    for (k, v) in p {
        if k == "id" {
            continue;
        }
        t.insert(k.clone(), v.clone());
    }
}
