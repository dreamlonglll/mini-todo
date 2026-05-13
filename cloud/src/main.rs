//! minitodo-cloud：mini-todo 的云端 HTTP API。
//!
//! 启动顺序：
//! 1. 加载 `config.toml`（缺字段直接 panic 出来）
//! 2. 打开 SQLite + 建表
//! 3. 启动时同步执行一次 `pull_once`，把 WebDAV 上现有数据灌进本地
//! 4. spawn 后台 `start_pull_loop`（60s 轮询） + `start_push_loop`（1s 检查
//!    dirty 并条件 PUT 回 WebDAV） + `spawn_bootstrap`（一次性图片镜像）
//! 5. 启动 axum，监听 `config.bind`

use std::env;
use std::path::PathBuf;
use std::sync::Arc;

use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

mod api;
mod config;
mod db;
mod sync;
mod time;
mod util;

use crate::api::AppState;
use crate::config::Config;
use crate::db::Db;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();

    let cfg_path = resolve_config_path();
    info!(target: "minitodo_cloud", "loading config from {}", cfg_path.display());
    let cfg = Arc::new(Config::load(&cfg_path)?);

    // 准备 data_dir / images_dir
    std::fs::create_dir_all(&cfg.data_dir)
        .map_err(|e| anyhow::anyhow!("创建 data_dir {} 失败: {}", cfg.data_dir.display(), e))?;
    let db_path: PathBuf = cfg.data_dir.join("data.db");
    let db = Db::open(&db_path)?;

    // 启动时同步拉一次。失败不阻断启动（远端可能暂时不可用），但记日志。
    match sync::pull::pull_once(&cfg, &db) {
        Ok(()) => info!(target: "minitodo_cloud", "initial pull ok"),
        Err(e) => warn!(target: "minitodo_cloud", "initial pull failed: {:#}", e),
    }

    // 后台 worker
    sync::pull::start_pull_loop(cfg.clone(), db.clone());
    sync::push::start_push_loop(cfg.clone(), db.clone());
    sync::images::spawn_bootstrap(cfg.clone());

    // axum
    let state = AppState {
        config: cfg.clone(),
        db: db.clone(),
    };
    let router = api::build_router(state);

    let listener = tokio::net::TcpListener::bind(&cfg.bind)
        .await
        .map_err(|e| anyhow::anyhow!("无法绑定 {}: {}", cfg.bind, e))?;
    info!(target: "minitodo_cloud", "listening on http://{}", cfg.bind);
    axum::serve(listener, router)
        .await
        .map_err(|e| anyhow::anyhow!("axum serve 失败: {}", e))?;
    Ok(())
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,minitodo_cloud=debug"));
    tracing_subscriber::fmt().with_env_filter(filter).init();
}

fn resolve_config_path() -> PathBuf {
    let mut args = env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--config" | "-c" => {
                if let Some(v) = args.next() {
                    return PathBuf::from(v);
                }
            }
            other if other.starts_with("--config=") => {
                return PathBuf::from(&other["--config=".len()..]);
            }
            _ => {}
        }
    }
    if let Ok(p) = env::var("MINITODO_CONFIG") {
        return PathBuf::from(p);
    }
    PathBuf::from("config.toml")
}
