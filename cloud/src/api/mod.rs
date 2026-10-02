//! HTTP API 层。
//!
//! 路由结构：
//! - `/health`
//! - `/todos`、`/todos/:id`、`/todos/:id/subtasks`
//! - `/subtasks/:id`
//! - `/images`、`/images/:name`
//!
//! 中间件洋葱（外 → 内）：
//! 1. 访问日志（tower-http `TraceLayer`）：方法、路径（不含 query）、状态码、耗时；
//!    **从不记录请求头**（`Authorization` 里是 api_key），401 也会记录
//! 2. auth：先校验 token，401 直接短路返回
//! 3. inject_sync_headers：只有鉴权通过的响应才附 X-Sync-Status / X-Last-Sync-At；
//!    未鉴权请求不查库、不暴露同步状态

pub mod auth;
pub mod error;
pub mod headers;
pub mod health;
pub mod ids;
pub mod images;
pub mod subtasks;
pub mod sync;
pub mod todos;

#[cfg(test)]
mod integration_tests;

use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::extract::DefaultBodyLimit;
use axum::http::{Request, Response};
use axum::middleware;
use axum::routing::{delete, get, patch, post};
use axum::Router;
use tower_http::trace::TraceLayer;
use tracing::Span;

use crate::config::Config;
use crate::db::Db;
use crate::sync::SyncCtx;

/// API 路由层共享的 state。
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub db: Db,
    /// 与后台 pull / push worker 共享的同步上下文（同步互斥锁 + 复用的 WebDAV
    /// 客户端）。只有 `/sync` 系列端点会获取同步锁；CRUD 写路径不拿锁，避免被
    /// 慢速网络同步阻塞。
    pub sync: Arc<SyncCtx>,
}

pub fn build_router(state: AppState) -> Router {
    // axum/tower 的洋葱模型：`.layer(A).layer(B)` 表示 B 是外层、A 是内层，
    // 请求顺序 B → A → handler，响应顺序 handler → A → B。
    //
    // 鉴权必须是最外层：401 在这里短路返回，既不查库也不在响应里暴露同步状态
    // （早期版本把 header 注入放在外层，未鉴权请求也会触发 DB 查询）。
    Router::new()
        .route("/health", get(health::get_health))
        .route("/todos", get(todos::list_todos).post(todos::create_todo))
        .route(
            "/todos/:id",
            get(todos::get_todo)
                .patch(todos::patch_todo)
                .delete(todos::delete_todo),
        )
        .route("/todos/:id/subtasks", post(subtasks::create_subtask))
        .route("/subtasks/:id", patch(subtasks::patch_subtask))
        .route("/subtasks/:id", delete(subtasks::delete_subtask))
        .route(
            "/images",
            // multipart 最大 32 MiB；只放宽图片上传这一条路由，
            // 其余路由（含 POST /todos 的 JSON body）维持 axum 默认 2 MB 上限
            post(images::upload_image).layer(DefaultBodyLimit::max(32 * 1024 * 1024)),
        )
        .route("/images/:name", get(images::get_image))
        .route("/sync", post(sync::post_sync))
        .route("/sync/pull", post(sync::post_sync_pull))
        .route("/sync/push", post(sync::post_sync_push))
        // 内层：鉴权通过的响应注入 sync header
        .layer(middleware::from_fn_with_state(
            state.clone(),
            headers::inject_sync_headers,
        ))
        // 中层：先校验 token
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth::require_bearer,
        ))
        // 最外层：访问日志（5xx 也只经 on_response 记一条，不重复报 failure）
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(access_span)
                .on_request(())
                .on_response(access_on_response)
                .on_body_chunk(())
                .on_eos(())
                .on_failure(()),
        )
        .with_state(state)
}

/// 访问日志 span：只带方法与路径（**不含 query**：搜索词等可能涉及隐私；也从不记录任何
/// 请求头，`Authorization` 不会出现在日志里）。
fn access_span(req: &Request<Body>) -> Span {
    tracing::info_span!(
        target: "minitodo_cloud::access",
        "request",
        method = %req.method(),
        path = %req.uri().path(),
    )
}

/// 访问日志：响应时记录状态码与耗时（在 `access_span` 里输出）。
fn access_on_response(resp: &Response<Body>, latency: Duration, _span: &Span) {
    tracing::info!(
        target: "minitodo_cloud::access",
        status = resp.status().as_u16(),
        latency_ms = latency.as_millis() as u64,
        "response"
    );
}
