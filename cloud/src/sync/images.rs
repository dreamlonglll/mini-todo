//! 图片 bootstrap：启动时把 WebDAV `/mini-todo/images/` 里有、本地没有的
//! 图片下到 `config.images_dir`。
//!
//! PR1 范围：bootstrap 只跑一次；启动时 spawn 一个 blocking task 异步完成。
//! PR2 才加 dirty image 队列 + 双向 PUT。

use std::path::PathBuf;
use std::sync::Arc;

use tracing::{error, info, warn};

use crate::config::Config;
use crate::sync::webdav::WebDavClient;

const REMOTE_IMAGES_DIR: &str = "/mini-todo/images";

pub fn bootstrap_images(cfg: &Config) -> anyhow::Result<usize> {
    std::fs::create_dir_all(&cfg.images_dir).map_err(|e| {
        anyhow::anyhow!(
            "创建本地 images 目录 {} 失败: {}",
            cfg.images_dir.display(),
            e
        )
    })?;

    let client = WebDavClient::new(&cfg.webdav_url, &cfg.webdav_username, &cfg.webdav_password)?;
    let _ = client.ensure_dir(REMOTE_IMAGES_DIR);

    let remote_names = match client.list_files(REMOTE_IMAGES_DIR) {
        Ok(v) => v,
        Err(e) => {
            warn!(target: "minitodo_cloud::images", "list remote images failed: {:#}", e);
            return Ok(0);
        }
    };

    let mut downloaded = 0usize;
    for name in remote_names {
        let local_path: PathBuf = cfg.images_dir.join(&name);
        if local_path.exists() {
            continue;
        }
        let remote_path = format!("{}/{}", REMOTE_IMAGES_DIR, name);
        match client.download_to(&remote_path, &local_path) {
            Ok(n) => {
                info!(target: "minitodo_cloud::images", "downloaded {} ({} bytes)", name, n);
                downloaded += 1;
            }
            Err(e) => {
                warn!(target: "minitodo_cloud::images", "download {} 失败: {:#}", name, e);
            }
        }
    }
    Ok(downloaded)
}

/// 启动时 spawn 的一次性 bootstrap 任务。
pub fn spawn_bootstrap(cfg: Arc<Config>) {
    tokio::spawn(async move {
        let res = tokio::task::spawn_blocking(move || bootstrap_images(&cfg)).await;
        match res {
            Ok(Ok(n)) => {
                info!(target: "minitodo_cloud::images", "image bootstrap done, {} new files", n)
            }
            Ok(Err(e)) => {
                error!(target: "minitodo_cloud::images", "image bootstrap failed: {:#}", e)
            }
            Err(join_err) => {
                error!(target: "minitodo_cloud::images", "image bootstrap task panicked: {}", join_err)
            }
        }
    });
}
