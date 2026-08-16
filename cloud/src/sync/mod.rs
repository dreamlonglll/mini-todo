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

use std::sync::Arc;

/// 同步操作互斥锁：pull tick / push tick / `POST /sync`（含 `/sync/pull`、
/// `/sync/push`）共享同一把，保证任一时刻只有一个同步操作在跑。
///
/// 为什么需要：pull 的孤儿清理会删"远端没有的本地记录"，push 的推送窗口
/// （GET → merge → PUT，慢速网络下可达数秒）内若并发跑 pull，本地新建还没
/// 推上去的记录会被当孤儿删掉。
///
/// 只保护"整段同步操作"，CRUD 写请求**不**拿这把锁——普通写请求不该被慢速
/// 网络同步阻塞；写入与同步之间的一致性由 `meta.dirty` + `dirty_generation`
/// 保证（见 `db::repo::mark_dirty`）。
pub type SyncLock = Arc<tokio::sync::Mutex<()>>;

/// 创建一把新的同步互斥锁。整个进程只应在 `main` 里创建一次并到处 clone。
pub fn new_sync_lock() -> SyncLock {
    Arc::new(tokio::sync::Mutex::new(()))
}
