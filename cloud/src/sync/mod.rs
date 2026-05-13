//! 后台 sync worker：WebDAV 客户端 + pull 循环 + push 循环 + 图片 bootstrap。
//!
//! PR2 范围：
//! - `pull_once` / `start_pull_loop`：60s 拉取
//! - `start_push_loop`：1s 检查 dirty 并 PUT 回 WebDAV（含 dirty_images）
//! - `spawn_bootstrap`：启动时一次性图片镜像

pub mod images;
pub mod pull;
pub mod push;
pub mod webdav;
