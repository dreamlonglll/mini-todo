//! `/sync` 手动触发 WebDAV 同步。
//!
//! 与后台 pull / push worker 共用同步锁，语义是"排队等待"而非"拒绝并发"。
//! 锁以 owned guard 的形式交给阻塞线程（`SyncCtx::run_locked`）：客户端中途断开、
//! handler future 被丢弃时，已经开始的同步仍持锁跑完，互斥不会提前失效。

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde::Serialize;
use serde_json::json;

use super::error::ApiError;
use super::AppState;
use crate::sync::{pull, push};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncResp {
    pub pull: &'static str,
    pub push: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pull_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub push_error: Option<String>,
}

pub async fn post_sync(State(state): State<AppState>) -> (StatusCode, Json<SyncResp>) {
    let (pull_res, push_res) = match state
        .sync
        .run_locked(|ctx| {
            let p = pull::pull_once(ctx).map(|_| ());
            let s = push::push_once(ctx).map(|_| ());
            Ok((p, s))
        })
        .await
    {
        Ok(pair) => pair,
        Err(e) => {
            let msg = format!("{:#}", e);
            (Err(anyhow::anyhow!(msg.clone())), Err(anyhow::anyhow!(msg)))
        }
    };

    let pull_ok = pull_res.is_ok();
    let push_ok = push_res.is_ok();
    let status = if pull_ok && push_ok {
        StatusCode::OK
    } else {
        StatusCode::MULTI_STATUS
    };

    (
        status,
        Json(SyncResp {
            pull: if pull_ok { "ok" } else { "error" },
            push: if push_ok { "ok" } else { "error" },
            pull_error: pull_res.err().map(|e| format!("{:#}", e)),
            push_error: push_res.err().map(|e| format!("{:#}", e)),
        }),
    )
}

pub async fn post_sync_pull(
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let r = state
        .sync
        .run_locked(pull::pull_once)
        .await
        .map_err(|e| ApiError::internal(format!("pull failed: {:#}", e)))?;
    Ok(Json(json!({
        "status": "ok",
        "remoteExists": r.remote_exists,
        "changed": r.changed,
        "todosUpserted": r.stats.todos_upserted,
        "todosDeleted": r.stats.todos_deleted,
        "subtasksUpserted": r.stats.subtasks_upserted,
        "subtasksDeleted": r.stats.subtasks_deleted,
        "tombstonesApplied": r.stats.tombstones_applied,
        "recordsSkipped": r.stats.records_skipped,
    })))
}

pub async fn post_sync_push(
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let r = state
        .sync
        .run_locked(push::push_once)
        .await
        .map_err(|e| ApiError::internal(format!("push failed: {:#}", e)))?;
    Ok(Json(json!({
        "status": "ok",
        "pushed": r.pushed,
        "attempts": r.attempts,
        "dirtyCleared": r.dirty_cleared,
        "imagesUploaded": r.images.uploaded,
        "imagesDropped": r.images.dropped,
    })))
}
