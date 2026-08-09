//! Pull worker：定期 GET `/mini-todo/sync-data.json.gz`，per-record LWW
//! 合并进本地 SQLite。
//!
//! - `pull_once`：单次拉取 + 合并 + seq 回填
//! - `start_pull_loop`：tokio 后台 spawn 的轮询循环（间隔 `pull_interval_secs`）
//! - 合并后做孤儿清理：远端不再出现的 todo/subtask 在本地删除（连带 `todo_seq`）；
//!   `meta.dirty == "true"` 时跳过清理，保护 API 本地新建还没 push 的记录
//! - 推送在 `push.rs` 的 push worker 负责

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
#[allow(dead_code)] // version / device_id / images 等元信息字段暂未读取，保留以完整映射 sync-data 结构
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
    pull_once_inner(cfg, db)?;
    // 不管远端是否变化，本地都可能有从 PC 端同步来的、还没分配 cloud 短码
    // `seq` 的 todo。每次 pull tick 末尾扫一遍 `todo_seq` LEFT JOIN 缺失行，
    // 给它们补 seq。开销 O(N) 且只命中没 seq 的，N 一般 < 1000，可忽略。
    let backfilled = backfill_missing_seq(db).map_err(|e| anyhow::anyhow!("回填 seq: {}", e))?;
    if backfilled > 0 {
        info!(target: "minitodo_cloud::pull", "backfilled {} todo seq(s)", backfilled);
    }
    Ok(())
}

fn pull_once_inner(cfg: &Config, db: &Db) -> anyhow::Result<()> {
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
        // settings 整 JSON 存一行。当前只写不读（push 合并时用的是远端 GET 回来的
        // settings），保留作调试快照 / 未来 /settings 端点的数据源。
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

/// 扫 todos 表，给在 `todo_seq` 中无对应行的 todo 分配 seq。
/// 不修改 data_json / updated_at（避免触发不必要的 dirty 同步），seq 仅本地表持有。
pub(crate) fn backfill_missing_seq(db: &Db) -> rusqlite::Result<usize> {
    db.with_conn(|conn| -> rusqlite::Result<usize> {
        let ids = repo::todo_ids_without_seq(conn)?;
        for id in &ids {
            repo::assign_seq(conn, id)?;
        }
        Ok(ids.len())
    })
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

/// per-record LWW merge + 孤儿清理。
///
/// 1. 远端 record.updated_at ≥ 本地 → upsert；反之保留本地
/// 2. merge 完毕后，删除"本地有但远端没有"的 todos/subtasks（孤儿清理）
///    — 当 `meta.dirty == "true"` 时跳过清理，保护 cloud API 本地新建还没 push 的记录
fn merge_into_sqlite(db: &Db, data: &SyncData) -> anyhow::Result<(usize, usize)> {
    db.with_conn(|conn| -> rusqlite::Result<(usize, usize)> {
        let tx = conn.transaction()?;

        // dirty flag 必须在事务内读，防止事务开始后 API handler 新建 todo
        // 设 dirty=true 但清理逻辑仍按旧的 dirty=false 执行。
        let is_dirty = repo::get_meta(&tx, "dirty");
        let skip_cleanup = is_dirty.as_deref() == Some("true");

        let mut todo_n = 0usize;
        let mut sub_n = 0usize;

        let mut remote_todo_ids: std::collections::HashSet<String> =
            std::collections::HashSet::new();
        let mut remote_subtask_ids: std::collections::HashSet<String> =
            std::collections::HashSet::new();

        for todo in &data.todos {
            let id = match extract_id(todo) {
                Some(v) => v,
                None => continue,
            };
            remote_todo_ids.insert(id.clone());

            let updated_at = todo
                .get("updatedAt")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            if let Some(subtasks) = todo.get("subtasks").and_then(|v| v.as_array()) {
                for sub in subtasks {
                    let sid = match extract_id(sub) {
                        Some(v) => v,
                        None => continue,
                    };
                    remote_subtask_ids.insert(sid.clone());
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

            let body = todo.to_string();
            if repo::upsert_todo_if_newer(&tx, &id, &body, &updated_at)? {
                todo_n += 1;
            }
        }

        if !skip_cleanup {
            repo::delete_todos_not_in(&tx, &remote_todo_ids)?;
            repo::delete_subtasks_not_in(&tx, &remote_subtask_ids)?;
        }

        tx.commit()?;
        Ok((todo_n, sub_n))
    })
    .map_err(|e| anyhow::anyhow!("merge_into_sqlite 失败: {}", e))
}

/// PC 端 todo / subtask 的 `id` 是 i64；这里统一转字符串便于 PK 处理。
/// 复用 `crate::util::id_string`（同一份逻辑也在 push / api 用）。
use crate::util::id_string as extract_id;

fn gunzip(data: &[u8]) -> anyhow::Result<String> {
    let mut dec = GzDecoder::new(data);
    let mut out = String::new();
    dec.read_to_string(&mut out)
        .map_err(|e| anyhow::anyhow!("gunzip 失败: {}", e))?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn fresh_db() -> (Db, TempDir) {
        let tmp = TempDir::new().expect("tempdir");
        let db = Db::open(&tmp.path().join("data.db")).expect("open db");
        (db, tmp)
    }

    fn todo_value(id: i64, title: &str, updated_at: &str) -> serde_json::Value {
        serde_json::json!({
            "id": id,
            "title": title,
            "updatedAt": updated_at,
            "subtasks": []
        })
    }

    fn subtask_value(id: i64, parent_id: i64, title: &str, updated_at: &str) -> serde_json::Value {
        serde_json::json!({
            "id": id,
            "parentId": parent_id,
            "title": title,
            "updatedAt": updated_at
        })
    }

    fn sync_data(todos: Vec<serde_json::Value>) -> SyncData {
        SyncData {
            version: "4.0".to_string(),
            device_id: "test".to_string(),
            updated_at: String::new(),
            todos,
            settings: serde_json::Value::Null,
            images: Vec::new(),
        }
    }

    fn todo_title(db: &Db, id: &str) -> Option<String> {
        db.with_conn(|conn| {
            repo::get_todo(conn, id).unwrap().map(|row| {
                serde_json::from_str::<serde_json::Value>(&row.data_json).unwrap()["title"]
                    .as_str()
                    .unwrap()
                    .to_string()
            })
        })
    }

    #[test]
    fn merge_cleanup_removes_remote_missing_todos_and_seq() {
        let (db, _tmp) = fresh_db();
        merge_into_sqlite(
            &db,
            &sync_data(vec![
                todo_value(1, "留", "2026-01-01 10:00:00"),
                todo_value(2, "删", "2026-01-01 10:00:00"),
            ]),
        )
        .unwrap();
        db.with_conn(|conn| repo::assign_seq(conn, "2").unwrap());

        merge_into_sqlite(
            &db,
            &sync_data(vec![todo_value(1, "留", "2026-01-01 10:00:00")]),
        )
        .unwrap();

        assert!(todo_title(&db, "1").is_some());
        assert!(todo_title(&db, "2").is_none());
        // 清理必须连带 todo_seq，否则 seq 会被下一个新 todo 复用出错
        db.with_conn(|conn| {
            assert_eq!(repo::get_seq(conn, "2").unwrap(), None);
        });
    }

    #[test]
    fn merge_cleanup_removes_orphan_subtasks() {
        let (db, _tmp) = fresh_db();
        let mut t = todo_value(1, "父", "2026-01-01 10:00:00");
        t["subtasks"] = serde_json::json!([
            subtask_value(11, 1, "留", "2026-01-01 10:00:00"),
            subtask_value(12, 1, "删", "2026-01-01 10:00:00"),
        ]);
        merge_into_sqlite(&db, &sync_data(vec![t])).unwrap();

        let mut t2 = todo_value(1, "父", "2026-01-01 10:00:00");
        t2["subtasks"] = serde_json::json!([subtask_value(11, 1, "留", "2026-01-01 10:00:00")]);
        merge_into_sqlite(&db, &sync_data(vec![t2])).unwrap();

        db.with_conn(|conn| {
            assert!(repo::get_subtask(conn, "11").unwrap().is_some());
            assert!(repo::get_subtask(conn, "12").unwrap().is_none());
        });
    }

    /// dirty=true 表示 cloud API 有本地新建还没 push 的记录，此时跳过清理，
    /// 避免"远端还没见过的新记录"被当孤儿删掉。
    #[test]
    fn merge_cleanup_skipped_when_dirty() {
        let (db, _tmp) = fresh_db();
        merge_into_sqlite(
            &db,
            &sync_data(vec![
                todo_value(1, "A", "2026-01-01 10:00:00"),
                todo_value(2, "本地新建", "2026-01-01 10:00:00"),
            ]),
        )
        .unwrap();
        db.with_conn(|conn| repo::set_meta(conn, "dirty", "true").unwrap());

        merge_into_sqlite(
            &db,
            &sync_data(vec![todo_value(1, "A", "2026-01-01 10:00:00")]),
        )
        .unwrap();

        assert!(todo_title(&db, "2").is_some(), "dirty 时不得清理本地记录");
    }

    #[test]
    fn merge_lww_keeps_newer_local() {
        let (db, _tmp) = fresh_db();
        merge_into_sqlite(
            &db,
            &sync_data(vec![todo_value(1, "本地较新", "2026-01-05 10:00:00")]),
        )
        .unwrap();

        merge_into_sqlite(
            &db,
            &sync_data(vec![todo_value(1, "远端较旧", "2026-01-02 10:00:00")]),
        )
        .unwrap();

        assert_eq!(todo_title(&db, "1").as_deref(), Some("本地较新"));
    }

    #[test]
    fn merge_lww_applies_newer_remote() {
        let (db, _tmp) = fresh_db();
        merge_into_sqlite(
            &db,
            &sync_data(vec![todo_value(1, "旧", "2026-01-01 10:00:00")]),
        )
        .unwrap();

        merge_into_sqlite(
            &db,
            &sync_data(vec![todo_value(1, "新", "2026-01-03 10:00:00")]),
        )
        .unwrap();

        assert_eq!(todo_title(&db, "1").as_deref(), Some("新"));
    }
}
