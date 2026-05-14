//! HTTP API 层。
//!
//! 路由结构：
//! - `/health`
//! - `/todos`、`/todos/:id`、`/todos/:id/subtasks`
//! - `/subtasks/:id`
//! - `/images`、`/images/:name`
//!
//! 中间件洋葱：内层 auth（先校验 token）+ 外层 inject_sync_headers（所有响应
//! 包括 401 都附 X-Sync-Status / X-Last-Sync-At）。

pub mod auth;
pub mod error;
pub mod headers;
pub mod health;
pub mod ids;
pub mod images;
pub mod subtasks;
pub mod todos;

#[cfg(test)]
mod integration_tests;

use std::sync::Arc;

use axum::extract::DefaultBodyLimit;
use axum::middleware;
use axum::routing::{delete, get, patch, post};
use axum::Router;

use crate::config::Config;
use crate::db::Db;

/// API 路由层共享的 state。
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub db: Db,
}

pub fn build_router(state: AppState) -> Router {
    // axum/tower 的洋葱模型：`.layer(A).layer(B)` 表示 B 是外层、A 是内层，
    // 请求顺序 B → A → handler，响应顺序 handler → A → B。
    //
    // 所有响应（包括 401）都带 X-Sync-Status；auth 401 后短路返回时也要被
    // header 注入。因此 `inject_sync_headers` 必须是外层，注册顺序后于
    // `require_bearer`。
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
        .route("/images", post(images::upload_image))
        .route("/images/:name", get(images::get_image))
        // multipart 最大 32 MiB
        .layer(DefaultBodyLimit::max(32 * 1024 * 1024))
        // 内层：先校验 token
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth::require_bearer,
        ))
        // 外层：无论 handler / 内层 middleware 怎么应答都注入 sync header
        .layer(middleware::from_fn_with_state(
            state.clone(),
            headers::inject_sync_headers,
        ))
        .with_state(state)
}
