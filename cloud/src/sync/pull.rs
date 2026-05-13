//! Pull worker：定期 GET `/mini-todo/sync-data.json.gz`，per-record LWW
//! 合并进本地 SQLite。
//!
//! PR1 范围：
//! - `pull_once`：单次拉取 + 合并
//! - `start_pull_loop`：tokio 后台 spawn 的 60s 循环
//!
//! 不做：
//! - 删除合并（远端不再出现的 record 在本地不删 — 留 TODO，PR2 处理）
//! - dirty flag / 推送（PR2 push worker 范围）

use std::io::Read as _;
use std::sync::Arc;
use std::time::Duration;

use flate2::read::GzDecoder;
use serde::Deserialize;
use tracing::{error, info, warn};

use crate::config::Config;
use crate::db::{repo, Db};
use crate::sync::webdav::WebDavClient;
use crate::time::now_local_string;

/// 远端 `/mini-todo` 同步目录路径。
const REMOTE_DIR: &str = "/mini-todo";
const SYNC_DATA_FILE: &str = "/mini-todo/sync-data.json.gz";

/// 与 `pc::commands::sync_cmd::SyncData` 对齐的反序列化结构。
///
/// - 字段 camelCase（与 PC 端 `#[serde(rename_all = "camelCase")]` 一致）
/// - todos 中含嵌套 `subtasks`（PC 端导出时也把 subtask 嵌进去）
/// - settings / 未知字段全部以 `serde_json::Value` 透传，保持 schema 漂移宽容
/// - `serde` 默认忽略未知字段，因此 v3.0 旧数据也能解析
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)] // version / device_id / images 等元信息字段 PR2 会用到
pub struct SyncData {
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub device_id: String,
    #[serde(default)]
    pub updated_at: String,
    #[serde(default)]
    pub todos: Vec<serde_json::Value>,
    #[serde(default)]
    pub settings: serde_json::Value,
    #[serde(default)]
    pub images: Vec<String>,
}

/// 主入口：拉一次 + 合并；返回是否成功拿到远端数据。
///
/// 304 → 视为成功但跳过解码；调用方读 `meta.last_pull_at` 已被更新即可。
/// 404 → 远端还没有 sync-data，返回成功但 `data` 为空；进程继续工作。
pub fn pull_once(cfg: &Config, db: &Db) -> anyhow::Result<()> {
    let client = WebDavClient::new(&cfg.webdav_url, &cfg.webdav_username, &cfg.webdav_password)?;
    let _ = client.ensure_dir(REMOTE_DIR);

    let last_etag = db.with_conn(|conn| repo::get_meta(conn, "last_etag"));
    let res = client.get(SYNC_DATA_FILE, last_etag.as_deref())?;

    let now = now_local_string(cfg.timezone_offset);

    match res.status_code {
        304 => {
            // 远端无变化，只刷新 last_pull_at
            db.with_conn(|conn| -> rusqlite::Result<()> {
                repo::set_meta(conn, "last_pull_at", &now)?;
                Ok(())
            })
            .map_err(|e| anyhow::anyhow!("写 meta 失败: {}", e))?;
            info!(target: "minitodo_cloud::pull", "remote unchanged (304)");
            return Ok(());
        }
        404 => {
            // 远端还没创建过 sync-data；不算 error
            db.with_conn(|conn| -> rusqlite::Result<()> {
                repo::set_meta(conn, "last_pull_at", &now)?;
                Ok(())
            })
            .map_err(|e| anyhow::anyhow!("写 meta 失败: {}", e))?;
            warn!(target: "minitodo_cloud::pull", "remote sync-data.json.gz 尚不存在（404）");
            return Ok(());
        }
        200 => {}
        other => anyhow::bail!("pull 收到意外状态 {}", other),
    }

    let bytes = res.bytes.unwrap_or_default();
    let json = gunzip(&bytes)?;
    let data: SyncData =
        serde_json::from_str(&json).map_err(|e| anyhow::anyhow!("解析 sync-data 失败: {}", e))?;

    let (todo_n, sub_n) = merge_into_sqlite(db, &data)?;
    let settings_str = data.settings.to_string();

    db.with_conn(|conn| -> rusqlite::Result<()> {
        repo::set_meta(conn, "last_pull_at", &now)?;
        if let Some(etag) = res.etag.as_deref() {
            repo::set_meta(conn, "last_etag", etag)?;
        }
        if let Some(lm) = res.last_modified.as_deref() {
            repo::set_meta(conn, "last_modified", lm)?;
        }
        // settings 整 JSON 存一行，PR2 提供 /settings GET 时直接返回
        repo::set_setting(conn, "all", &settings_str)?;
        Ok(())
    })
    .map_err(|e| anyhow::anyhow!("写 meta/settings 失败: {}", e))?;

    info!(
        target: "minitodo_cloud::pull",
        "pull ok: {} todos merged, {} subtasks merged, last_modified={:?}",
        todo_n, sub_n, res.last_modified
    );
    Ok(())
}

/// 后台 spawn 的轮询循环。
pub fn start_pull_loop(cfg: Arc<Config>, db: Db) {
    let interval = Duration::from_secs(cfg.pull_interval_secs);
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(interval).await;
            let cfg_ref = cfg.clone();
            let db_ref = db.clone();
            let res = tokio::task::spawn_blocking(move || pull_once(&cfg_ref, &db_ref)).await;
            match res {
                Ok(Ok(())) => {}
                Ok(Err(e)) => error!(target: "minitodo_cloud::pull", "pull tick failed: {:#}", e),
                Err(join_err) => {
                    error!(target: "minitodo_cloud::pull", "pull task panicked: {}", join_err)
                }
            }
        }
    });
}

/// per-record LWW merge：远端 record.updated_at ≥ 本地 → upsert；
/// 反之保留本地。删除（远端缺失 = 远端被删）目前**不处理**，PR2 push 完成
/// 后再做软删除/tombstone。
fn merge_into_sqlite(db: &Db, data: &SyncData) -> anyhow::Result<(usize, usize)> {
    db.with_conn(|conn| -> rusqlite::Result<(usize, usize)> {
        let tx = conn.transaction()?;
        let mut todo_n = 0usize;
        let mut sub_n = 0usize;
        for todo in &data.todos {
            let id = match extract_id(todo) {
                Some(v) => v,
                None => continue,
            };
            let updated_at = todo
                .get("updatedAt")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            // 把嵌套 subtasks 拆出来，单独 upsert
            if let Some(subtasks) = todo.get("subtasks").and_then(|v| v.as_array()) {
                for sub in subtasks {
                    let sid = match extract_id(sub) {
                        Some(v) => v,
                        None => continue,
                    };
                    let sub_updated = sub
                        .get("updatedAt")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let body = sub.to_string();
                    if repo::upsert_subtask_if_newer(&tx, &sid, &id, &body, &sub_updated)? {
                        sub_n += 1;
                    }
                }
            }

            // todo 本身落到 todos 表（保留 subtasks 字段在 data_json 里 — 列表端
            // 可直接返回；写时也仍按嵌套形态原样存。）
            let body = todo.to_string();
            if repo::upsert_todo_if_newer(&tx, &id, &body, &updated_at)? {
                todo_n += 1;
            }
        }
        tx.commit()?;
        Ok((todo_n, sub_n))
    })
    .map_err(|e| anyhow::anyhow!("merge_into_sqlite 失败: {}", e))
}

/// PC 端 todo / subtask 的 `id` 是 i64；这里统一转字符串便于 PK 处理。
fn extract_id(v: &serde_json::Value) -> Option<String> {
    let raw = v.get("id")?;
    if let Some(n) = raw.as_i64() {
        return Some(n.to_string());
    }
    if let Some(s) = raw.as_str() {
        if !s.is_empty() {
            return Some(s.to_string());
        }
    }
    None
}

fn gunzip(data: &[u8]) -> anyhow::Result<String> {
    let mut dec = GzDecoder::new(data);
    let mut out = String::new();
    dec.read_to_string(&mut out)
        .map_err(|e| anyhow::anyhow!("gunzip 失败: {}", e))?;
    Ok(out)
}
