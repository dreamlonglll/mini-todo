//! Push worker：1s tick 扫 `meta.dirty`；若 dirty，把云端 SQLite 当前快照
//! merge 进远端 `sync-data.json.gz` 并条件 PUT 回去。
//!
//! 同时挂一个图片 push：扫 `meta.dirty_images`（JSON 数组），逐个 PUT 到
//! WebDAV `/mini-todo/images/`。

use std::collections::HashSet;
use std::io::Write as _;
use std::sync::Arc;
use std::time::Duration;

use flate2::write::GzEncoder;
use flate2::Compression;
use serde::Serialize;
use serde_json::{json, Value};
use tracing::{error, info, warn};

use crate::config::Config;
use crate::db::{repo, Db};
use crate::sync::webdav::WebDavClient;
use crate::time::now_local_string;

const REMOTE_DIR: &str = "/mini-todo";
const REMOTE_IMAGES_DIR: &str = "/mini-todo/images";
const SYNC_DATA_FILE: &str = "/mini-todo/sync-data.json.gz";

/// 后台 spawn 的 push 循环（1s tick）。
pub fn start_push_loop(cfg: Arc<Config>, db: Db) {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(1)).await;
            let cfg_ref = cfg.clone();
            let db_ref = db.clone();
            let res = tokio::task::spawn_blocking(move || push_tick(&cfg_ref, &db_ref)).await;
            match res {
                Ok(Ok(_)) => {}
                Ok(Err(e)) => error!(target: "minitodo_cloud::push", "push tick failed: {:#}", e),
                Err(join_err) => {
                    error!(target: "minitodo_cloud::push", "push task panicked: {}", join_err)
                }
            }
        }
    });
}

/// 单次 tick：检查 dirty / dirty_images 并处理。
pub fn push_tick(cfg: &Config, db: &Db) -> anyhow::Result<()> {
    // === dirty sync-data ===
    let dirty = db.with_conn(|conn| repo::get_meta(conn, "dirty"));
    if dirty.as_deref() == Some("true") {
        // CAS：先标 false，处理失败再标回 true。这样并发写入新增的 dirty=true
        // 不会被本轮覆盖。
        db.with_conn(|conn| -> rusqlite::Result<()> {
            repo::set_meta(conn, "dirty", "false")?;
            Ok(())
        })?;
        if let Err(e) = do_push_sync_data(cfg, db) {
            // 失败：复位 dirty=true，下轮重试
            let _ = db.with_conn(|conn| -> rusqlite::Result<()> {
                repo::set_meta(conn, "dirty", "true")?;
                Ok(())
            });
            return Err(e);
        }
    }

    // === dirty images ===
    push_dirty_images(cfg, db)?;
    Ok(())
}

fn do_push_sync_data(cfg: &Config, db: &Db) -> anyhow::Result<()> {
    let client = WebDavClient::new(&cfg.webdav_url, &cfg.webdav_username, &cfg.webdav_password)?;
    let _ = client.ensure_dir(REMOTE_DIR);
    let _ = client.ensure_dir(REMOTE_IMAGES_DIR);

    // 1) 读远端最新快照（注意：不用 If-None-Match——这里要拿到 last_modified
    //    并基于它构建合并 + 后续条件 PUT）
    let res = client.get(SYNC_DATA_FILE, None)?;
    let (remote_data, remote_last_modified) = match res.status_code {
        200 => {
            let bytes = res.bytes.unwrap_or_default();
            let json = gunzip(&bytes)?;
            let v: Value = serde_json::from_str(&json)
                .map_err(|e| anyhow::anyhow!("解析远端 sync-data 失败: {}", e))?;
            (v, res.last_modified)
        }
        404 => (json!({}), None), // 远端还没有，第一次 PUT
        other => anyhow::bail!("push 阶段 GET 收到状态 {}", other),
    };

    // 2) merge 本地快照进 remote_data
    let local_snapshot = build_local_snapshot(cfg, db)?;
    let merged = merge_sync_data(&remote_data, &local_snapshot, db, cfg)?;

    // 3) gzip + 条件 PUT
    let payload = serde_json::to_vec(&merged)?;
    let compressed = gzip(&payload)?;

    let put = client.put(
        SYNC_DATA_FILE,
        &compressed,
        "application/gzip",
        remote_last_modified.as_deref(),
    )?;
    match put.status_code {
        200 | 201 | 204 => {
            // 成功：更新 meta.last_pull_at + last_modified（重读拿到新值最准；
            // 这里简化为「PUT 后立刻 GET 一次 HEAD-ish」即重新 GET 拿 Last-Modified）。
            // 但额外 GET 会浪费一次往返；用现有 If-Unmodified-Since（如果有）+ 当前墙钟兜底
            let after_get = client.get(SYNC_DATA_FILE, None).ok();
            let new_lm = after_get.as_ref().and_then(|g| g.last_modified.clone());

            let now_local = now_local_string(cfg.timezone_offset);
            db.with_conn(|conn| -> rusqlite::Result<()> {
                repo::set_meta(conn, "last_pull_at", &now_local)?;
                if let Some(lm) = new_lm.as_deref() {
                    repo::set_meta(conn, "last_modified", lm)?;
                }
                if let Some(ref etag) = after_get.and_then(|g| g.etag) {
                    repo::set_meta(conn, "last_etag", etag)?;
                }
                // 清理超过 7 天的 tombstone（用 PC 风格本地时间字符串比较；
                // chrono 算 7 天前的本地时间）
                let cutoff = chrono::Utc::now()
                    .with_timezone(&cfg.timezone_offset)
                    .checked_sub_signed(chrono::Duration::days(7))
                    .map(|d| d.format("%Y-%m-%d %H:%M:%S").to_string());
                if let Some(c) = cutoff {
                    let _ = repo::purge_tombstones_before(conn, &c);
                }
                Ok(())
            })?;
            info!(target: "minitodo_cloud::push", "push ok");
            Ok(())
        }
        412 => {
            // 远端被别人改过：dirty 回设 true，下轮重试
            warn!(target: "minitodo_cloud::push", "412 precondition failed; will retry");
            db.with_conn(|conn| -> rusqlite::Result<()> {
                repo::set_meta(conn, "dirty", "true")?;
                Ok(())
            })?;
            Ok(())
        }
        other => {
            anyhow::bail!("push PUT 收到状态 {}", other);
        }
    }
}

/// 本地 SQLite 全部 todos + subtasks 序列化成一个简化的 "snapshot" 形式：
/// 每条 todo 的 `data_json` 反序列化为 Value，并把 subtasks 嵌入。
#[derive(Debug, Clone, Serialize)]
struct LocalSnapshot {
    todos: Vec<Value>,
    images: Vec<String>,
}

type TodoTuple = (String, Value, String);
type SubtaskTuple = (Value, String);
type SnapshotRaw = (
    Vec<TodoTuple>,
    std::collections::HashMap<String, Vec<SubtaskTuple>>,
);

fn build_local_snapshot(cfg: &Config, db: &Db) -> anyhow::Result<LocalSnapshot> {
    let (todos, subtasks_by_todo) = db.with_conn(|conn| -> rusqlite::Result<SnapshotRaw> {
        let todo_rows = repo::all_todos(conn)?;
        let mut todos_acc: Vec<TodoTuple> = Vec::with_capacity(todo_rows.len());
        for r in todo_rows {
            let v: Value =
                serde_json::from_str(&r.data_json).unwrap_or_else(|_| json!({"id": r.id}));
            todos_acc.push((r.id.clone(), v, r.updated_at));
        }
        let sub_rows = repo::all_subtasks(conn)?;
        let mut sub_map: std::collections::HashMap<String, Vec<SubtaskTuple>> =
            std::collections::HashMap::new();
        for r in sub_rows {
            let v: Value =
                serde_json::from_str(&r.data_json).unwrap_or_else(|_| json!({"id": r.id}));
            sub_map
                .entry(r.todo_id.clone())
                .or_default()
                .push((v, r.updated_at));
        }
        Ok((todos_acc, sub_map))
    })?;

    let mut out_todos = Vec::with_capacity(todos.len());
    for (id, mut v, _ts) in todos {
        let mut subs_vals: Vec<Value> = subtasks_by_todo
            .get(&id)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .map(|(sv, _)| sv)
            .collect();
        // 保持顺序：按 sortOrder asc
        subs_vals.sort_by_key(|s| s.get("sortOrder").and_then(|v| v.as_i64()).unwrap_or(0));
        v["subtasks"] = Value::Array(subs_vals);
        out_todos.push(v);
    }
    // todos 按 sortOrder asc
    out_todos.sort_by_key(|t| t.get("sortOrder").and_then(|v| v.as_i64()).unwrap_or(0));

    // 本地 images 目录文件列表
    let mut images = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&cfg.images_dir) {
        for entry in entries.flatten() {
            if let Some(name) = entry.file_name().to_str() {
                images.push(name.to_string());
            }
        }
    }
    images.sort();

    Ok(LocalSnapshot {
        todos: out_todos,
        images,
    })
}

/// per-record LWW merge：本地 + 远端 → 合并 SyncData。
///
/// - todos & nested subtasks：updatedAt 大的胜
/// - 本地有 tombstone → 把对应 record 从合并结果中剔除
/// - 远端 settings 总是优先（云端不写 settings）
fn merge_sync_data(
    remote: &Value,
    local: &LocalSnapshot,
    db: &Db,
    cfg: &Config,
) -> anyhow::Result<Value> {
    // 收集本地 tombstones
    let (todo_tombs, subtask_tombs) = db.with_conn(
        |conn| -> rusqlite::Result<(HashSet<String>, HashSet<String>)> {
            let mut t = HashSet::new();
            let mut s = HashSet::new();
            for (typ, id, _ts) in repo::list_tombstones(conn)? {
                match typ.as_str() {
                    "todo" => {
                        t.insert(id);
                    }
                    "subtask" => {
                        s.insert(id);
                    }
                    _ => {}
                }
            }
            Ok((t, s))
        },
    )?;

    // 远端 todos & subtasks
    let remote_todos = remote
        .get("todos")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    // 按 id 索引 local todos
    let mut local_by_id: std::collections::HashMap<String, Value> =
        std::collections::HashMap::new();
    for t in &local.todos {
        if let Some(id) = id_string(t) {
            local_by_id.insert(id, t.clone());
        }
    }

    // 输出 todos：以 union(id) 为基准
    let mut all_ids: Vec<String> = local_by_id.keys().cloned().collect();
    for r in &remote_todos {
        if let Some(id) = id_string(r) {
            if !all_ids.contains(&id) {
                all_ids.push(id);
            }
        }
    }

    let mut out_todos: Vec<Value> = Vec::new();
    for id in &all_ids {
        if todo_tombs.contains(id) {
            // 本地已删除，丢弃
            continue;
        }
        let remote_t = remote_todos
            .iter()
            .find(|t| id_string(t).as_deref() == Some(id));
        let local_t = local_by_id.get(id);
        let merged_todo = match (remote_t, local_t) {
            (Some(r), Some(l)) => {
                if updated_at(l) >= updated_at(r) {
                    // 本地更新或同时：以本地为主体，但子任务还要 union-merge
                    let mut base = l.clone();
                    let merged_subs =
                        merge_subtasks_into(remote_subs(r), local_subs(l), &subtask_tombs);
                    base["subtasks"] = Value::Array(merged_subs);
                    base
                } else {
                    let mut base = r.clone();
                    let merged_subs =
                        merge_subtasks_into(remote_subs(r), local_subs(l), &subtask_tombs);
                    base["subtasks"] = Value::Array(merged_subs);
                    base
                }
            }
            (Some(r), None) => {
                // 本地没有 + 远端有：可能本地没 pull 过；保留远端
                let mut base = r.clone();
                let merged_subs = merge_subtasks_into(remote_subs(r), Vec::new(), &subtask_tombs);
                base["subtasks"] = Value::Array(merged_subs);
                base
            }
            (None, Some(l)) => {
                let mut base = l.clone();
                let merged_subs = merge_subtasks_into(Vec::new(), local_subs(l), &subtask_tombs);
                base["subtasks"] = Value::Array(merged_subs);
                base
            }
            (None, None) => continue, // 不可能命中（id 来源于其中之一）
        };
        out_todos.push(merged_todo);
    }

    // settings：远端优先。若远端为空（首次部署，云端 push 比 PC 第一次 PUT 还早
    // 的边角场景），写入一个最小合法的 PC AppSettings——`is_fixed` / `window_position`
    // / `window_size` 在 PC 端不带 `serde(default)`，缺失会导致 PC import 失败。
    let settings = match remote.get("settings") {
        Some(v) if !v.is_null() && v.is_object() => v.clone(),
        _ => default_app_settings_value(),
    };

    // images：远端 ∪ 本地
    let mut images: HashSet<String> = local.images.iter().cloned().collect();
    if let Some(arr) = remote.get("images").and_then(|v| v.as_array()) {
        for v in arr {
            if let Some(s) = v.as_str() {
                images.insert(s.to_string());
            }
        }
    }
    let mut images_vec: Vec<String> = images.into_iter().collect();
    images_vec.sort();

    // 元信息
    let now_iso = chrono::Utc::now()
        .with_timezone(&cfg.timezone_offset)
        .format("%Y-%m-%dT%H:%M:%S%:z")
        .to_string();
    let version = remote
        .get("version")
        .and_then(|v| v.as_str())
        .unwrap_or("4.0")
        .to_string();
    let device_id = remote
        .get("deviceId")
        .and_then(|v| v.as_str())
        .unwrap_or("cloud")
        .to_string();

    Ok(json!({
        "version": version,
        "deviceId": device_id,
        "updatedAt": now_iso,
        "todos": out_todos,
        "settings": settings,
        "images": images_vec,
    }))
}

fn remote_subs(t: &Value) -> Vec<Value> {
    t.get("subtasks")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default()
}

fn local_subs(t: &Value) -> Vec<Value> {
    t.get("subtasks")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default()
}

fn merge_subtasks_into(
    remote_subs: Vec<Value>,
    local_subs: Vec<Value>,
    subtask_tombs: &HashSet<String>,
) -> Vec<Value> {
    let mut by_id: std::collections::HashMap<String, Value> = std::collections::HashMap::new();
    for s in remote_subs {
        if let Some(id) = id_string(&s) {
            if subtask_tombs.contains(&id) {
                continue;
            }
            by_id.insert(id, s);
        }
    }
    for s in local_subs {
        let Some(id) = id_string(&s) else { continue };
        if subtask_tombs.contains(&id) {
            by_id.remove(&id);
            continue;
        }
        match by_id.get(&id) {
            Some(existing) if updated_at(existing) >= updated_at(&s) => {}
            _ => {
                by_id.insert(id, s);
            }
        }
    }
    let mut out: Vec<Value> = by_id.into_values().collect();
    out.sort_by_key(|s| s.get("sortOrder").and_then(|v| v.as_i64()).unwrap_or(0));
    out
}

// 复用 util 模块的实现，保持三处（push / pull / api）行为一致。
use crate::util::id_string;

fn updated_at(v: &Value) -> &str {
    v.get("updatedAt").and_then(|x| x.as_str()).unwrap_or("")
}

/// 远端 sync-data 不存在或没有 settings 时使用的最小合法对象。
/// 与 `pc/src-tauri/src/db/models.rs::AppSettings` 的必填字段对齐，剩下字段
/// 都有 `serde(default)` 兜底，PC 反序列化时会自动填默认值。
fn default_app_settings_value() -> Value {
    json!({
        "isFixed": false,
        "windowPosition": null,
        "windowSize": null,
    })
}

fn gzip(data: &[u8]) -> anyhow::Result<Vec<u8>> {
    let mut enc = GzEncoder::new(Vec::new(), Compression::default());
    enc.write_all(data)
        .map_err(|e| anyhow::anyhow!("gzip write: {}", e))?;
    enc.finish()
        .map_err(|e| anyhow::anyhow!("gzip finish: {}", e))
}

fn gunzip(data: &[u8]) -> anyhow::Result<String> {
    use std::io::Read as _;
    let mut dec = flate2::read::GzDecoder::new(data);
    let mut out = String::new();
    dec.read_to_string(&mut out)
        .map_err(|e| anyhow::anyhow!("gunzip: {}", e))?;
    Ok(out)
}

// =============================================================================
// 图片 push
// =============================================================================

fn push_dirty_images(cfg: &Config, db: &Db) -> anyhow::Result<()> {
    let raw = db.with_conn(|conn| repo::get_meta(conn, "dirty_images"));
    let names: Vec<String> = raw
        .as_deref()
        .and_then(|s| serde_json::from_str::<Vec<String>>(s).ok())
        .unwrap_or_default();
    if names.is_empty() {
        return Ok(());
    }

    let client = WebDavClient::new(&cfg.webdav_url, &cfg.webdav_username, &cfg.webdav_password)?;
    let _ = client.ensure_dir(REMOTE_IMAGES_DIR);

    let mut remaining: Vec<String> = Vec::new();
    for name in &names {
        let local_path = cfg.images_dir.join(name);
        if !local_path.exists() {
            // 本地不见了，跳过；不挂回 dirty
            warn!(target: "minitodo_cloud::push", "dirty image {} missing locally, drop", name);
            continue;
        }
        let bytes = match std::fs::read(&local_path) {
            Ok(b) => b,
            Err(e) => {
                warn!(target: "minitodo_cloud::push", "read {} failed: {}", local_path.display(), e);
                remaining.push(name.clone());
                continue;
            }
        };
        let ct = guess_image_content_type(name);
        let remote_path = format!("{}/{}", REMOTE_IMAGES_DIR, name);
        match client.put(&remote_path, &bytes, ct, None) {
            Ok(put) if (200..300).contains(&put.status_code) => {
                info!(target: "minitodo_cloud::push", "uploaded image {}", name);
            }
            Ok(put) => {
                warn!(target: "minitodo_cloud::push", "PUT {} returned {}", name, put.status_code);
                remaining.push(name.clone());
            }
            Err(e) => {
                warn!(target: "minitodo_cloud::push", "PUT {} failed: {:#}", name, e);
                remaining.push(name.clone());
            }
        }
    }

    db.with_conn(|conn| -> rusqlite::Result<()> {
        let new_raw = serde_json::to_string(&remaining).unwrap_or_else(|_| "[]".to_string());
        repo::set_meta(conn, "dirty_images", &new_raw)?;
        Ok(())
    })?;

    Ok(())
}

fn guess_image_content_type(name: &str) -> &'static str {
    match std::path::Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .as_deref()
    {
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("webp") => "image/webp",
        Some("gif") => "image/gif",
        Some("bmp") => "image/bmp",
        Some("svg") => "image/svg+xml",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_subtasks_lww_keeps_newer() {
        let remote = vec![json!({"id": 1, "title": "old", "updatedAt": "2026-05-13 10:00:00"})];
        let local = vec![json!({"id": 1, "title": "new", "updatedAt": "2026-05-13 11:00:00"})];
        let tombs = HashSet::new();
        let out = merge_subtasks_into(remote, local, &tombs);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0]["title"].as_str(), Some("new"));
    }

    #[test]
    fn merge_subtasks_lww_keeps_remote_when_newer() {
        let remote =
            vec![json!({"id": 1, "title": "remote-new", "updatedAt": "2026-05-13 12:00:00"})];
        let local =
            vec![json!({"id": 1, "title": "local-old", "updatedAt": "2026-05-13 11:00:00"})];
        let tombs = HashSet::new();
        let out = merge_subtasks_into(remote, local, &tombs);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0]["title"].as_str(), Some("remote-new"));
    }

    #[test]
    fn merge_subtasks_tombstone_removes() {
        let remote = vec![json!({"id": 1, "title": "remote", "updatedAt": "2026-05-13 12:00:00"})];
        let local = vec![];
        let mut tombs = HashSet::new();
        tombs.insert("1".to_string());
        let out = merge_subtasks_into(remote, local, &tombs);
        assert!(out.is_empty(), "tombstone should suppress remote record");
    }

    #[test]
    fn merge_subtasks_union_when_disjoint() {
        let remote = vec![json!({"id": 1, "title": "a", "updatedAt": "2026-05-13 10:00:00"})];
        let local = vec![json!({"id": 2, "title": "b", "updatedAt": "2026-05-13 10:00:00"})];
        let tombs = HashSet::new();
        let out = merge_subtasks_into(remote, local, &tombs);
        assert_eq!(out.len(), 2);
        let ids: std::collections::HashSet<_> = out.iter().filter_map(id_string).collect();
        assert!(ids.contains("1"));
        assert!(ids.contains("2"));
    }

    #[test]
    fn id_string_handles_numeric_and_string() {
        assert_eq!(id_string(&json!({"id": 42})), Some("42".to_string()));
        assert_eq!(id_string(&json!({"id": "abc"})), Some("abc".to_string()));
        assert_eq!(id_string(&json!({})), None);
    }

    #[test]
    fn updated_at_default_empty() {
        assert_eq!(updated_at(&json!({})), "");
        assert_eq!(updated_at(&json!({"updatedAt": "x"})), "x");
    }

    #[test]
    fn gzip_roundtrip() {
        let body = b"hello, world!";
        let compressed = gzip(body).unwrap();
        let decompressed = gunzip(&compressed).unwrap();
        assert_eq!(decompressed.as_bytes(), body);
    }

    #[test]
    fn guess_content_type_matches_extension() {
        assert_eq!(guess_image_content_type("a.png"), "image/png");
        assert_eq!(guess_image_content_type("b.JPG"), "image/jpeg");
        assert_eq!(
            guess_image_content_type("c.bin"),
            "application/octet-stream"
        );
    }

    #[test]
    fn default_settings_has_required_fields() {
        // PC 端 AppSettings 必填字段：isFixed / windowPosition / windowSize
        // 这些在 PC `serde` 反序列化时没有 default，缺失会导致 import 失败。
        let v = default_app_settings_value();
        assert!(v.get("isFixed").is_some());
        assert!(v.get("windowPosition").is_some());
        assert!(v.get("windowSize").is_some());
    }
}
