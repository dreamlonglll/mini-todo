//! 响应 header 注入。
//!
//! - `X-Sync-Status: healthy | stale | offline`
//!   * healthy: 最近 pull 成功 ≤ pull_interval * 2
//!   * stale:   最近 pull 成功 > pull_interval * 2 但 ≤ 5 分钟
//!   * offline: 超过 5 分钟没成功 pull
//! - `X-Last-Sync-At: <ISO 字符串>`（直接用 meta.last_pull_at 原值，与 PC 端
//!   SQLite 字符串保持一致；客户端按"墙钟时间"理解）
//! - offline 时额外加 `Warning: 110 "sync offline"`（RFC 7234）

use axum::extract::{Request, State};
use axum::http::header::HeaderValue;
use axum::http::HeaderName;
use axum::middleware::Next;
use axum::response::Response;
use chrono::{NaiveDateTime, Utc};

use super::AppState;
use crate::db::repo;

const X_SYNC_STATUS: HeaderName = HeaderName::from_static("x-sync-status");
const X_LAST_SYNC_AT: HeaderName = HeaderName::from_static("x-last-sync-at");

/// 同步状态判定结果。
pub struct SyncStatus {
    pub status: &'static str,
    pub last_pull_at: Option<String>,
}

pub async fn inject_sync_headers(
    State(state): State<AppState>,
    req: Request,
    next: Next,
) -> Response {
    let mut resp = next.run(req).await;
    let status = compute_sync_status(&state);

    if let Ok(v) = HeaderValue::from_str(status.status) {
        resp.headers_mut().insert(X_SYNC_STATUS, v);
    }
    if let Some(ref last) = status.last_pull_at {
        if let Ok(v) = HeaderValue::from_str(last) {
            resp.headers_mut().insert(X_LAST_SYNC_AT, v);
        }
    }
    if status.status == "offline" {
        let v = HeaderValue::from_static(r#"110 - "sync offline""#);
        resp.headers_mut().insert(axum::http::header::WARNING, v);
    }
    resp
}

pub fn compute_sync_status(state: &AppState) -> SyncStatus {
    let last_pull_at = state
        .db
        .with_conn(|conn| repo::get_meta(conn, "last_pull_at"));

    let Some(ref s) = last_pull_at else {
        return SyncStatus {
            status: "offline",
            last_pull_at: None,
        };
    };

    // PC SQLite 字符串形如 "2026-05-13 12:34:56"，按 config 时区解析回 UTC
    let parsed = NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").ok();
    let Some(naive) = parsed else {
        return SyncStatus {
            status: "offline",
            last_pull_at: last_pull_at.clone(),
        };
    };

    let pull_dt = naive
        .and_local_timezone(state.config.timezone_offset)
        .single()
        .map(|d| d.with_timezone(&Utc));
    let Some(pull_dt) = pull_dt else {
        return SyncStatus {
            status: "offline",
            last_pull_at: last_pull_at.clone(),
        };
    };

    let age_sec = (Utc::now() - pull_dt).num_seconds().max(0) as u64;
    let interval = state.config.pull_interval_secs;
    let status = if age_sec <= interval * 2 {
        "healthy"
    } else if age_sec <= 300 {
        "stale"
    } else {
        "offline"
    };

    SyncStatus {
        status,
        last_pull_at,
    }
}
