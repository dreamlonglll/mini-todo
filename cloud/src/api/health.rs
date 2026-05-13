//! `GET /health`：基础健康检查 + 同步状态。

use axum::extract::State;
use axum::Json;
use serde::Serialize;

use super::headers::compute_sync_status;
use super::AppState;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthResp {
    pub status: &'static str,
    pub sync: &'static str,
    pub last_pull_at: Option<String>,
}

pub async fn get_health(State(state): State<AppState>) -> Json<HealthResp> {
    let sync = compute_sync_status(&state);
    Json(HealthResp {
        status: "healthy",
        sync: sync.status,
        last_pull_at: sync.last_pull_at,
    })
}
