//! 后台 sync worker：WebDAV 客户端 + pull 循环 + 图片 bootstrap。
//!
//! PR1 范围：
//! - 启动时同步执行一次 `pull_once`，确保 SQLite 已有数据再开 HTTP server
//! - 后台 spawn 60s 轮询的 `start_pull_loop`
//! - 后台 spawn 一次性 `bootstrap_images`
//!
//! push（dirty flag + 条件 PUT）属于 PR2 范围，本 PR 不实现。

pub mod images;
pub mod pull;
pub mod webdav;
