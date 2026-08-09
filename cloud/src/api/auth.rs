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
        .and_then(|s| {
            // RFC 7235: auth-scheme 大小写不敏感
            let (scheme, rest) = s.split_once(' ')?;
            scheme.eq_ignore_ascii_case("bearer").then_some(rest)
        })
        .map(|s| s.trim().to_string());

    let supplied = match token {
        Some(t) if !t.is_empty() => t,
        _ => return Err(unauthorized("missing bearer token")),
    };

    if !constant_time_eq(supplied.as_bytes(), state.config.api_key.as_bytes()) {
        return Err(unauthorized("invalid api key"));
    }

    Ok(next.run(req).await)
}

/// 常数时间字节比较，防止通过响应时延逐字节猜 api_key。
/// 长度不同时提前返回只泄露长度信息，与 `subtle::ConstantTimeEq` 的约束一致。
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
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
