//! HTTP API 错误：统一 `{"error", "detail", ...}` JSON 响应。
//!
//! 内部错误（SQLite、文件系统、同步层）的完整内容只写服务端日志，客户端只看到错误码与
//! 通用说明——不把原始 SQLite 错误（会暴露表结构 / SQL）和服务端文件路径返回出去
//! （审查 F9）。
//!
//! 请求体不是合法 JSON 时 axum 默认回纯文本；`ApiJson` 提取器把它换成同样的 JSON 错误
//! 格式，客户端只需要处理一种错误体。

use std::fmt::Display;

use axum::body::Body;
use axum::extract::rejection::JsonRejection;
use axum::extract::FromRequest;
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use serde_json::{json, Map, Value};
use tracing::error;

use crate::model::ValidationError;

/// 内部错误给客户端的说明。
pub const INTERNAL_DETAIL: &str = "internal server error (details in server log)";

/// 标准化错误响应。所有 handler 用 `Result<T, ApiError>` 返回。
#[derive(Debug)]
pub struct ApiError {
    pub status: StatusCode,
    pub code: &'static str,
    pub detail: String,
    /// 附加到错误体顶层的结构化字段（校验错误的字段列表等）。
    pub extra: Map<String, Value>,
}

impl ApiError {
    pub fn new(status: StatusCode, code: &'static str, detail: impl Into<String>) -> Self {
        Self {
            status,
            code,
            detail: detail.into(),
            extra: Map::new(),
        }
    }

    pub fn bad_request(detail: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "bad_request", detail)
    }

    pub fn not_found(detail: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, "not_found", detail)
    }

    /// 内部错误：完整错误写进服务端日志，客户端只看到通用说明。
    pub fn internal(context: &str, err: impl Display) -> Self {
        error!(target: "minitodo_cloud::api", "{}: {}", context, err);
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            INTERNAL_DETAIL,
        )
    }

    /// K6 字段校验失败：400，`detail` 是可读说明，另附 `unknownFields` /
    /// `invalidFields` / `allowedFields` 方便程序处理。
    pub fn validation(e: ValidationError) -> Self {
        let mut err = Self::bad_request(e.message());
        if !e.unknown.is_empty() {
            err.extra.insert("unknownFields".into(), json!(e.unknown));
        }
        if !e.invalid.is_empty() {
            let invalid: Map<String, Value> = e
                .invalid
                .iter()
                .map(|(f, m)| (f.clone(), json!(m)))
                .collect();
            err.extra
                .insert("invalidFields".into(), Value::Object(invalid));
        }
        err.extra.insert("allowedFields".into(), json!(e.allowed));
        err
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let mut body = Map::new();
        body.insert("error".into(), json!(self.code));
        body.insert("detail".into(), json!(self.detail));
        for (k, v) in self.extra {
            body.entry(k).or_insert(v);
        }
        let bytes = serde_json::to_vec(&Value::Object(body))
            .unwrap_or_else(|_| br#"{"error":"internal"}"#.to_vec());
        Response::builder()
            .status(self.status)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(bytes))
            .unwrap_or_else(|_| {
                Response::builder()
                    .status(StatusCode::INTERNAL_SERVER_ERROR)
                    .body(Body::empty())
                    .unwrap()
            })
    }
}

impl From<rusqlite::Error> for ApiError {
    fn from(e: rusqlite::Error) -> Self {
        ApiError::internal("storage error", e)
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(e: anyhow::Error) -> Self {
        ApiError::internal("internal error", format!("{:#}", e))
    }
}

impl From<JsonRejection> for ApiError {
    fn from(r: JsonRejection) -> Self {
        let status = r.status();
        let code = match status {
            StatusCode::UNSUPPORTED_MEDIA_TYPE => "unsupported_media_type",
            StatusCode::PAYLOAD_TOO_LARGE => "payload_too_large",
            _ => "bad_request",
        };
        ApiError::new(status, code, r.body_text())
    }
}

/// `axum::Json` 的包装：解析失败时返回本模块统一的 JSON 错误体。
#[derive(FromRequest)]
#[from_request(via(axum::Json), rejection(ApiError))]
pub struct ApiJson<T>(pub T);
