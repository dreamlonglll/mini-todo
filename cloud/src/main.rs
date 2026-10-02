//! minitodo-cloud：mini-todo 的云端 HTTP API。
//!
//! 启动顺序：
//! 1. 加载 `config.toml`（缺字段直接报错退出）
//! 2. 打开 SQLite + 建表 / 幂等升级
//! 3. 首次 pull：在同步锁内、`spawn_blocking` 里执行（WebDAV 客户端是 reqwest
//!    blocking，在 async 上下文里构造 / 调用会让 debug 构建 panic，审查 F1）。
//!    失败不阻断启动（远端可能暂时不可用），只记日志
//! 4. 启动后台 worker：pull 轮询、push（去抖 + 退避）、图片镜像（启动时先跑一次）
//! 5. 启动 axum，监听 `config.bind`
//!
//! 停机（SIGTERM / SIGINT）：停止接收新连接并限时等待在途请求 → 通知 worker 退出 →
//! 有待推送内容时限时补推一次 → runtime 限时关闭（不无限等待卡住的阻塞任务）。

use std::env;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::watch;
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
use crate::sync::SyncCtx;

/// 收到停机信号后等待在途 HTTP 请求结束的上限。
const DRAIN_TIMEOUT: Duration = Duration::from_secs(10);
/// 停机前补推一次的总预算（含等待正在进行的同步释放锁）。
const FINAL_PUSH_BUDGET: Duration = Duration::from_secs(20);
/// 等待后台 worker 退出的上限。
const WORKER_STOP_TIMEOUT: Duration = Duration::from_secs(5);
/// runtime 关闭时等待残留阻塞任务的上限。
const RUNTIME_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);

fn main() -> anyhow::Result<()> {
    init_tracing();
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| anyhow::anyhow!("创建 tokio runtime 失败: {}", e))?;
    let result = rt.block_on(run());
    rt.shutdown_timeout(RUNTIME_SHUTDOWN_TIMEOUT);
    result
}

async fn run() -> anyhow::Result<()> {
    let cfg_path = resolve_config_path();
    info!(target: "minitodo_cloud", "loading config from {}", cfg_path.display());
    let cfg = Arc::new(Config::load(&cfg_path)?);

    // 准备 data_dir / images_dir
    std::fs::create_dir_all(&cfg.data_dir)
        .map_err(|e| anyhow::anyhow!("创建 data_dir {} 失败: {}", cfg.data_dir.display(), e))?;
    let db_path: PathBuf = cfg.data_dir.join("data.db");
    let db = Db::open(&db_path)?;

    let sync_ctx = SyncCtx::new(cfg.clone(), db.clone());

    // 首次 pull（阻塞线程 + 同步锁）。失败不阻断启动。
    match sync_ctx.run_locked(sync::pull::pull_once).await {
        Ok(_) => info!(target: "minitodo_cloud", "initial pull ok"),
        Err(e) => warn!(target: "minitodo_cloud", "initial pull failed: {:#}", e),
    }

    // 后台 worker
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let workers = sync::worker::spawn_workers(sync_ctx.clone(), shutdown_rx.clone());
    // 启动时补一次图片镜像（覆盖进程停机期间远端新增的图片）
    sync_ctx.request_image_mirror();

    // axum
    let state = AppState {
        config: cfg.clone(),
        db: db.clone(),
        sync: sync_ctx.clone(),
    };
    let router = api::build_router(state);
    let listener = tokio::net::TcpListener::bind(&cfg.bind)
        .await
        .map_err(|e| anyhow::anyhow!("无法绑定 {}: {}", cfg.bind, e))?;
    info!(target: "minitodo_cloud", "listening on http://{}", cfg.bind);

    let mut server_shutdown = shutdown_rx.clone();
    let mut server = tokio::spawn(async move {
        axum::serve(listener, router)
            .with_graceful_shutdown(async move {
                let _ = server_shutdown.wait_for(|stop| *stop).await;
            })
            .await
    });

    let server_result = tokio::select! {
        _ = shutdown_signal() => {
            info!(target: "minitodo_cloud", "shutdown signal received, draining");
            let _ = shutdown_tx.send(true);
            match tokio::time::timeout(DRAIN_TIMEOUT, &mut server).await {
                Ok(joined) => flatten_server_result(joined),
                Err(_) => {
                    warn!(target: "minitodo_cloud", "in-flight requests did not finish within {:?}", DRAIN_TIMEOUT);
                    server.abort();
                    Ok(())
                }
            }
        }
        joined = &mut server => {
            let _ = shutdown_tx.send(true);
            flatten_server_result(joined)
        }
    };

    // worker 在下一个等待点退出；正在跑的同步操作结束后才会释放锁
    for handle in workers {
        if tokio::time::timeout(WORKER_STOP_TIMEOUT, handle)
            .await
            .is_err()
        {
            warn!(target: "minitodo_cloud", "a sync worker did not stop within {:?}", WORKER_STOP_TIMEOUT);
        }
    }

    // 有待推送内容时限时补推一次
    sync::worker::final_push(&sync_ctx, FINAL_PUSH_BUDGET).await;
    info!(target: "minitodo_cloud", "bye");
    server_result
}

fn flatten_server_result(
    joined: Result<std::io::Result<()>, tokio::task::JoinError>,
) -> anyhow::Result<()> {
    match joined {
        Ok(Ok(())) => Ok(()),
        Ok(Err(e)) => Err(anyhow::anyhow!("axum serve 失败: {}", e)),
        Err(e) => Err(anyhow::anyhow!("axum serve 任务异常退出: {}", e)),
    }
}

/// 等待 SIGINT（Ctrl-C）或 SIGTERM（systemd stop）。信号处理器装不上时只记日志、
/// 永不触发（不能因此立刻停机）。
async fn shutdown_signal() {
    let ctrl_c = async {
        if let Err(e) = tokio::signal::ctrl_c().await {
            warn!(target: "minitodo_cloud", "无法监听 SIGINT: {}", e);
            std::future::pending::<()>().await;
        }
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut sig) => {
                sig.recv().await;
            }
            Err(e) => {
                warn!(target: "minitodo_cloud", "无法监听 SIGTERM: {}", e);
                std::future::pending::<()>().await;
            }
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {}
        _ = terminate => {}
    }
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
