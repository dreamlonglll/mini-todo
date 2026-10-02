//! `GET /health`：服务与同步健康状态。
//!
//! `sync` 取 pull / push 中较差者（见 `headers::classify`）。不是 healthy 时
//! `status = "degraded"` 并返回 **503**，响应体带出排查所需的字段：最近一次成功
//! 拉取 / 推送时间、最近的错误、本地积压（dirty 起始时间、图片队列长度）。

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde::Serialize;

use super::headers::{compute_sync_status, Level};
use super::AppState;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthResp {
    /// `healthy` | `degraded`
    pub status: &'static str,
    /// pull / push 中较差者：`healthy` | `stale` | `offline`
    pub sync: &'static str,
    pub pull: &'static str,
    pub push: &'static str,
    pub last_pull_at: Option<String>,
    pub last_pull_error: Option<String>,
    pub last_push_ok_at: Option<String>,
    pub last_push_error: Option<String>,
    pub dirty: bool,
    /// 本地写入开始积压的时刻（本地墙钟）。
    pub dirty_since: Option<String>,
    pub image_queue_length: usize,
}

pub async fn get_health(State(state): State<AppState>) -> (StatusCode, Json<HealthResp>) {
    let s = compute_sync_status(&state);
    let degraded = s.overall != Level::Healthy;
    let code = if degraded {
        StatusCode::SERVICE_UNAVAILABLE
    } else {
        StatusCode::OK
    };
    (
        code,
        Json(HealthResp {
            status: if degraded { "degraded" } else { "healthy" },
            sync: s.overall.as_str(),
            pull: s.pull.as_str(),
            push: s.push.as_str(),
            last_pull_at: s.snapshot.last_pull_at,
            last_pull_error: s.snapshot.last_pull_error,
            last_push_ok_at: s.snapshot.last_push_ok_at,
            last_push_error: s.snapshot.last_push_error,
            dirty: s.snapshot.dirty,
            dirty_since: s.dirty_since_local,
            image_queue_length: s.snapshot.image_queue_length,
        }),
    )
}
