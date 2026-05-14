//! `/sync` 手动触发 WebDAV 同步。

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
    let cfg = state.config.clone();
    let db = state.db.clone();

    let (pull_res, push_res) = tokio::task::spawn_blocking(move || {
        let p = pull::pull_once(&cfg, &db);
        let s = push::push_tick(&cfg, &db);
        (p, s)
    })
    .await
    .unwrap_or_else(|e| {
        (
            Err(anyhow::anyhow!("panic: {}", e)),
            Err(anyhow::anyhow!("panic: {}", e)),
        )
    });

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
    let cfg = state.config.clone();
    let db = state.db.clone();

    tokio::task::spawn_blocking(move || pull::pull_once(&cfg, &db))
        .await
        .map_err(|e| ApiError::internal(format!("task panic: {}", e)))?
        .map_err(|e| ApiError::internal(format!("pull failed: {:#}", e)))?;

    Ok(Json(json!({"status": "ok"})))
}

pub async fn post_sync_push(
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let cfg = state.config.clone();
    let db = state.db.clone();

    tokio::task::spawn_blocking(move || push::push_tick(&cfg, &db))
        .await
        .map_err(|e| ApiError::internal(format!("task panic: {}", e)))?
        .map_err(|e| ApiError::internal(format!("push failed: {:#}", e)))?;

    Ok(Json(json!({"status": "ok"})))
}
