//! HTTP API 层。
//!
//! 路由结构：
//! - `/health`
//! - `/todos`、`/todos/:id`、`/todos/:id/subtasks`
//! - `/subtasks/:id`
//! - `/images`、`/images/:name`
//!
//! 中间件洋葱：外层 auth（先校验 token，401 直接短路返回）+ 内层
//! inject_sync_headers（只有鉴权通过的响应才附 X-Sync-Status / X-Last-Sync-At；
//! 未鉴权请求不查库、不暴露同步状态）。

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

use axum::extract::DefaultBodyLimit;
use axum::middleware;
use axum::routing::{delete, get, patch, post};
use axum::Router;

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
        // 外层：先校验 token
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth::require_bearer,
        ))
        .with_state(state)
}
