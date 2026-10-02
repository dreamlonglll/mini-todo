//! 后台同步：WebDAV 客户端 + 合并规则 + pull / push / 图片镜像 worker。
//!
//! 模块分工：
//! - `webdav`：阻塞 reqwest 客户端（条件 GET / PUT、PROPFIND、MKCOL）
//! - `doc`：sync-data 文档模型（远端文档解析、墓碑、要上传文档的构造）
//! - `merge`：跨端契约 K3——把远端文档合并进本地 SQLite
//! - `pull` / `push`：单次同步操作（都在同步锁内、阻塞线程里执行）
//! - `images`：K5 文件名规则、图片上传队列、远端图片镜像
//! - `worker`：后台循环（退避 + 抖动、写入去抖、优雅停机前的补推）

pub mod doc;
pub mod images;
pub mod merge;
pub mod pull;
pub mod push;
pub mod webdav;
pub mod worker;

#[cfg(test)]
pub(crate) mod mock_dav;
#[cfg(test)]
mod scenario_tests;

use std::io::{Read as _, Write as _};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use tokio::sync::{Mutex, Notify};
use tracing::warn;

use crate::config::Config;
use crate::db::Db;
use crate::sync::webdav::WebDavClient;

/// 远端同步目录。
pub const REMOTE_DIR: &str = "/mini-todo";
/// 远端图片目录。
pub const REMOTE_IMAGES_DIR: &str = "/mini-todo/images";
/// 远端 sync-data 文件（gzip 压缩的 JSON）。
pub const SYNC_DATA_FILE: &str = "/mini-todo/sync-data.json.gz";

/// 全进程共享的同步上下文：配置、数据库、同步互斥锁、复用的 WebDAV 客户端。
///
/// **同步互斥锁**：pull tick / push tick / 图片以外的一切 WebDAV 同步操作 /
/// `POST /sync*` 共享同一把，保证任一时刻只有一个"拉取-合并-推送"在跑。
/// 只保护整段同步操作，CRUD 写请求**不**拿这把锁——普通写请求不该被慢速网络
/// 同步阻塞；写入与同步之间的一致性由 `meta.dirty` + `dirty_generation` 保证
/// （见 `db::repo::mark_dirty`）。
pub struct SyncCtx {
    pub cfg: Arc<Config>,
    pub db: Db,
    lock: Arc<Mutex<()>>,
    /// reqwest **blocking** 客户端：内部自带运行时线程，构造与发请求都必须在
    /// 阻塞上下文里做（debug 构建在 async 上下文里调用会 panic，审查 F1）。
    /// 因此懒初始化，第一次由 `spawn_blocking` 闭包里的同步代码触发。
    dav: OnceLock<WebDavClient>,
    /// 图片镜像 worker 的唤醒信号（pull 拿到新文档后触发）。`Notify` 会保留一个
    /// 许可，worker 还没开始等待时发出的通知也不会丢。
    mirror_wakeup: Notify,
    /// 服务端被证实错误处理 `If-Match`（远端没变却回 412）后，本进程不再用
    /// `If-Match`，退回 `If-Unmodified-Since`。
    if_match_unreliable: AtomicBool,
}

impl SyncCtx {
    pub fn new(cfg: Arc<Config>, db: Db) -> Arc<Self> {
        Arc::new(SyncCtx {
            cfg,
            db,
            lock: Arc::new(Mutex::new(())),
            dav: OnceLock::new(),
            mirror_wakeup: Notify::new(),
            if_match_unreliable: AtomicBool::new(false),
        })
    }

    /// 取（必要时创建）全局复用的 WebDAV 客户端。**只能在阻塞上下文里调用。**
    pub fn dav(&self) -> anyhow::Result<&WebDavClient> {
        if let Some(c) = self.dav.get() {
            return Ok(c);
        }
        let client = WebDavClient::new(
            &self.cfg.webdav_url,
            &self.cfg.webdav_username,
            &self.cfg.webdav_password,
        )?;
        // 并发初始化时只有一个能 set 成功，另一个直接丢弃（阻塞上下文里 drop 安全）
        let _ = self.dav.set(client);
        Ok(self.dav.get().expect("dav client initialised above"))
    }

    /// 在同步锁内、阻塞线程里执行一段同步操作。
    ///
    /// 锁用 owned guard 并 move 进 `spawn_blocking` 闭包：调用方的 future 被取消
    /// （例如 HTTP 客户端断开连接，handler future 被 drop）时，已经开始的阻塞
    /// 操作仍持有锁直到真正结束，互斥不会提前失效（审查 F3）。
    pub async fn run_locked<T, F>(self: &Arc<Self>, f: F) -> anyhow::Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&SyncCtx) -> anyhow::Result<T> + Send + 'static,
    {
        let guard = self.lock.clone().lock_owned().await;
        let ctx = self.clone();
        tokio::task::spawn_blocking(move || {
            let _guard = guard;
            f(&ctx)
        })
        .await
        .map_err(|e| anyhow::anyhow!("同步任务异常退出: {}", e))?
    }

    /// 请求后台补下缺失图片（非阻塞，可在任意上下文调用）。
    pub fn request_image_mirror(&self) {
        self.mirror_wakeup.notify_one();
    }

    pub(crate) async fn mirror_requested(&self) {
        self.mirror_wakeup.notified().await;
    }

    pub(crate) fn if_match_unreliable(&self) -> bool {
        self.if_match_unreliable.load(Ordering::Relaxed)
    }

    pub(crate) fn mark_if_match_unreliable(&self) {
        if !self.if_match_unreliable.swap(true, Ordering::Relaxed) {
            warn!(
                target: "minitodo_cloud::push",
                "WebDAV 服务端对未变化的文件拒绝了 If-Match（412），本进程改用 If-Unmodified-Since"
            );
        }
    }
}

pub(crate) fn gzip(data: &[u8]) -> anyhow::Result<Vec<u8>> {
    let mut enc = GzEncoder::new(Vec::new(), Compression::default());
    enc.write_all(data)
        .map_err(|e| anyhow::anyhow!("gzip 压缩失败: {}", e))?;
    enc.finish()
        .map_err(|e| anyhow::anyhow!("gzip 压缩失败: {}", e))
}

pub(crate) fn gunzip(data: &[u8]) -> anyhow::Result<String> {
    let mut dec = GzDecoder::new(data);
    let mut out = String::new();
    dec.read_to_string(&mut out)
        .map_err(|e| anyhow::anyhow!("gunzip 失败: {}", e))?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gzip_roundtrip() {
        let body = b"hello, world!";
        let compressed = gzip(body).unwrap();
        let decompressed = gunzip(&compressed).unwrap();
        assert_eq!(decompressed.as_bytes(), body);
    }

    #[test]
    fn gunzip_rejects_garbage() {
        assert!(gunzip(b"definitely not gzip").is_err());
    }
}
