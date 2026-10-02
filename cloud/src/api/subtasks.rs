//! `/subtasks` CRUD（独立 PATCH/DELETE）+ 嵌于 `/todos/:id/subtasks` 的 POST。
//!
//! 写入（K6）：只接受 `title` / `content` / `completed` / `sortOrder`；`parentId` 只能等于
//! 当前父待办（改归属 → 400：PC 合并时以外层 todo 为准，改了也没用，旧版还会让 PC 的外键
//! 失败、整个合并回滚）；未知字段 / 类型错误 → 400。每个写操作一个事务。

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde_json::{json, Map, Value};

use super::error::{ApiError, ApiJson};
use super::ids::{insert_with_fresh_id, new_id};
use super::todos::ensure_todo_exists;
use super::AppState;
use crate::db::repo;
use crate::model::{self, WriteMode};
use crate::time::{bump_updated_at, deletion_time, now_local_string};

const TOMBSTONE_SUBTASK: &str = "subtask";

/// todo 主键（i64 字符串）→ 数值。缓存里的主键都来自 i64 id，解析失败只可能是坏数据。
fn todo_id_num(id: &str) -> Result<i64, ApiError> {
    id.parse::<i64>()
        .map_err(|_| ApiError::internal("non-numeric todo id in cache", id))
}

// =============================================================================
// POST /todos/:id/subtasks
// =============================================================================

pub async fn create_subtask(
    State(state): State<AppState>,
    Path(raw_todo_ref): Path<String>,
    ApiJson(body): ApiJson<Value>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let now = now_local_string(state.config.timezone);

    // 同一事务内解析父 todo ref（支持 C 短码）+ 校验 + 写 subtask + 标脏。
    let record = state
        .db
        .with_conn(|conn| -> Result<Map<String, Value>, ApiError> {
            let tx = conn.transaction()?;
            let parent_id = ensure_todo_exists(&tx, &raw_todo_ref)?;
            let parent_num = todo_id_num(&parent_id)?;
            let patch = model::validate_subtask_input(&body, WriteMode::Create, parent_num)
                .map_err(ApiError::validation)?;

            let mut record = Map::new();
            insert_with_fresh_id(new_id, |id| {
                record = model::new_subtask_record(id, parent_num, &patch, &now);
                repo::insert_subtask(
                    &tx,
                    &id.to_string(),
                    &parent_id,
                    &Value::Object(record.clone()).to_string(),
                    &now,
                )
            })?;
            repo::mark_dirty(&tx)?;
            tx.commit()?;
            Ok(record)
        })?;

    Ok((StatusCode::CREATED, Json(Value::Object(record))))
}

// =============================================================================
// PATCH /subtasks/:id
// =============================================================================

pub async fn patch_subtask(
    State(state): State<AppState>,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<Value>,
) -> Result<Json<Value>, ApiError> {
    let tz = state.config.timezone;
    let now = now_local_string(tz);

    let updated = state
        .db
        .with_conn(|conn| -> Result<Option<Value>, ApiError> {
            let tx = conn.transaction()?;
            let Some(row) = repo::get_subtask(&tx, &id)? else {
                return Ok(None);
            };
            let parent_num = todo_id_num(&row.todo_id)?;
            let patch = model::validate_subtask_input(&body, WriteMode::Update, parent_num)
                .map_err(ApiError::validation)?;

            let mut base = match serde_json::from_str::<Value>(&row.data_json) {
                Ok(Value::Object(m)) => m,
                _ => Map::new(),
            };
            let normalized = model::normalize_stored_subtask(
                &mut base,
                tz,
                &row.id,
                &row.todo_id,
                &row.updated_at,
            );
            let mut next = base.clone();
            crate::util::merge_json_shallow(&mut next, &patch);

            if normalized.semantic || next != base {
                // 新版本严格晚于旧版本（见 time::bump_updated_at）
                let updated_at = bump_updated_at(
                    &now,
                    &[&row.updated_at, model::json_updated_at(&base).unwrap_or("")],
                    tz,
                );
                next.insert("updatedAt".into(), json!(updated_at));
                repo::upsert_subtask(
                    &tx,
                    &row.id,
                    &row.todo_id,
                    &Value::Object(next.clone()).to_string(),
                    &updated_at,
                )?;
                repo::mark_dirty(&tx)?;
            } else if normalized.format {
                repo::upsert_subtask(
                    &tx,
                    &row.id,
                    &row.todo_id,
                    &Value::Object(next.clone()).to_string(),
                    &row.updated_at,
                )?;
            }
            tx.commit()?;
            Ok(Some(Value::Object(next)))
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
    let tz = state.config.timezone;
    let now = now_local_string(tz);
    let removed = state.db.with_conn(|conn| -> rusqlite::Result<bool> {
        let tx = conn.transaction()?;
        let Some(row) = repo::get_subtask(&tx, &id)? else {
            return Ok(false);
        };
        let existed = repo::delete_subtask(&tx, &row.id)?;
        if existed {
            // 墓碑时间 = max(now, 记录的 updatedAt)（与 PC 一致，见 time::deletion_time）
            let json_ts = model::data_json_updated_at(&row.data_json);
            let deleted_at = deletion_time(
                &now,
                &[&row.updated_at, json_ts.as_deref().unwrap_or("")],
                tz,
            );
            repo::add_tombstone(&tx, TOMBSTONE_SUBTASK, &row.id, &deleted_at)?;
            repo::mark_dirty(&tx)?;
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
