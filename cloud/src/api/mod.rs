//! HTTP API 层。
//!
//! PR1 范围：
//! - 一条 `/health` 路由
//! - Bearer auth middleware（每个请求强制校验，错/缺 token 返回 401）
//! - 响应 header 注入：`X-Sync-Status` / `X-Last-Sync-At` / `Warning: 110 …`
//!
//! 资源 CRUD（/todos /subtasks /images）属于 PR2 范围，此处只把 Router 搭好。

pub mod auth;
pub mod headers;
pub mod health;

use std::sync::Arc;

use axum::middleware;
use axum::routing::get;
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
    // 我们要：所有响应（包括 401）都带 X-Sync-Status；auth 401 后短路返回时
    // 也要被 header 注入。因此 `inject_sync_headers` 必须是外层，注册顺序要
    // 后于 `require_bearer`。
    Router::new()
        .route("/health", get(health::get_health))
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
