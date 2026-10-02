//! `/sync` 手动触发 WebDAV 同步。
//!
//! 与后台 pull / push worker 共用同步锁，语义是"排队等待"而非"拒绝并发"。
//! 锁以 owned guard 的形式交给阻塞线程（`SyncCtx::run_locked`）：客户端中途断开、
//! handler future 被丢弃时，已经开始的同步仍持锁跑完，互斥不会提前失效。
//!
//! 错误：完整错误链写服务端日志；响应里是 `util::public_error_message` 脱敏后的消息
//! （WebDAV 状态码 / 连接错误照常给出，本地 SQLite / 文件系统错误只说"本地存储错误"）。
//!
//! 手动拉取一律**无条件 GET**（不带 `If-None-Match`）：用户 / AI 显式要求"拿最新数据"时不能
//! 被 nginx 秒级 ETag 的盲区（同一秒内写入且长度相同 → 304）挡住；后台轮询才用条件 GET。

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde::Serialize;
use serde_json::json;
use tracing::warn;

use super::error::ApiError;
use super::AppState;
use crate::sync::{pull, push};
use crate::util::public_error_message;

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

fn public(op: &str, e: &anyhow::Error) -> String {
    warn!(target: "minitodo_cloud::api", "manual {} failed: {:#}", op, e);
    public_error_message(e)
}

fn sync_failed(op: &str, e: anyhow::Error) -> ApiError {
    ApiError::new(
        StatusCode::INTERNAL_SERVER_ERROR,
        "sync_failed",
        format!("{} failed: {}", op, public(op, &e)),
    )
}

pub async fn post_sync(State(state): State<AppState>) -> (StatusCode, Json<SyncResp>) {
    let (pull_res, push_res) = match state
        .sync
        .run_locked(|ctx| {
            let p = pull::pull_once_with(ctx, true).map(|_| ());
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
            pull_error: pull_res.err().map(|e| public("pull", &e)),
            push_error: push_res.err().map(|e| public("push", &e)),
        }),
    )
}

pub async fn post_sync_pull(
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let r = state
        .sync
        .run_locked(|ctx| pull::pull_once_with(ctx, true))
        .await
        .map_err(|e| sync_failed("pull", e))?;
    Ok(Json(json!({
        "status": "ok",
        "remoteExists": r.remote_exists,
        "changed": r.changed,
        "repushScheduled": r.repush_scheduled,
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
        .map_err(|e| sync_failed("push", e))?;
    Ok(Json(json!({
        "status": "ok",
        "pushed": r.pushed,
        "attempts": r.attempts,
        "dirtyCleared": r.dirty_cleared,
        "imagesUploaded": r.images.uploaded,
        "imagesDropped": r.images.dropped,
    })))
}
