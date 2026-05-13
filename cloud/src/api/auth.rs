//! Bearer Token middleware：`Authorization: Bearer {api_key}` 缺/错 → 401。
//!
//! 单 API key（来自 `config.toml`），与 prd "Out of Scope: 多 API Key / token
//! 轮换" 一致。

use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{header, StatusCode};
use axum::middleware::Next;
use axum::response::Response;

use super::AppState;

pub async fn require_bearer(
    State(state): State<AppState>,
    req: Request,
    next: Next,
) -> Result<Response, Response> {
    let token = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .map(|s| s.trim().to_string());

    let supplied = match token {
        Some(t) if !t.is_empty() => t,
        _ => return Err(unauthorized("missing bearer token")),
    };

    if supplied != state.config.api_key {
        return Err(unauthorized("invalid api key"));
    }

    Ok(next.run(req).await)
}

fn unauthorized(detail: &str) -> Response {
    let body = format!(r#"{{"error":"unauthorized","detail":"{}"}}"#, detail);
    Response::builder()
        .status(StatusCode::UNAUTHORIZED)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::WWW_AUTHENTICATE, r#"Bearer realm="minitodo""#)
        .body(Body::from(body))
        .unwrap()
}
