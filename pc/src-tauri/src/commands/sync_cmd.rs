//! WebDAV 同步命令。
//!
//! 自 PR3 起，PC 端上传使用条件 PUT（`If-Unmodified-Since`）：
//! - 上传成功 → 用 server 返回的 `Last-Modified` 更新 `webdav_last_modified` setting，
//!   作为下次条件 PUT 的依据
//! - 412 Precondition Failed → 表示远端被另一端（cloud / 其它 PC）改过；自动拉取最新
//!   远端，做 per-record LWW merge 到本地 SQLite，再重新 PUT，最多重试 3 次
//!
//! sync 下载路径（`webdav_apply_remote`、`webdav_auto_sync`）使用 per-record merge +
//! 孤儿清理：先 LWW 合并远端 todos/subtasks，再删除"本地有但远端没有"的记录，
//! 最后写入远端 settings。`import_data_raw` 仅供手动文件导入使用。

use super::data::{export_data_internal, write_app_settings};
use crate::db::{Database, SubTask, Todo};
use crate::services::webdav::{UploadOutcome, WebDavClient};
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use std::io::{Read as _, Write as _};
use std::path::PathBuf;
use tauri::State;

const MAX_UPLOAD_RETRY: u32 = 3;

const REMOTE_DIR: &str = "/mini-todo";
const SYNC_DATA_FILE: &str = "/mini-todo/sync-data.json.gz";
const REMOTE_IMAGES_DIR: &str = "/mini-todo/images";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncSettings {
    pub webdav_url: String,
    pub webdav_username: String,
    pub webdav_password: String,
    pub auto_sync: bool,
    pub sync_interval: i32,
    pub last_sync_at: Option<String>,
    pub device_id: String,
}

impl Default for SyncSettings {
    fn default() -> Self {
        Self {
            webdav_url: String::new(),
            webdav_username: String::new(),
            webdav_password: String::new(),
            auto_sync: false,
            sync_interval: 15,
            last_sync_at: None,
            device_id: generate_device_id(),
        }
    }
}

fn generate_device_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis();
    format!("dev_{}", ts)
}

fn get_images_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("mini-todo")
        .join("images")
}

fn get_setting(conn: &rusqlite::Connection, key: &str) -> Option<String> {
    conn.query_row("SELECT value FROM settings WHERE key = ?1", [key], |row| {
        row.get(0)
    })
    .ok()
}

fn set_setting(conn: &rusqlite::Connection, key: &str, value: &str) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO settings (key, value, updated_at) VALUES (?1, ?2, datetime('now', 'localtime'))",
        [key, value],
    )?;
    Ok(())
}

#[tauri::command]
pub fn get_sync_settings(db: State<Database>) -> Result<SyncSettings, String> {
    read_sync_settings(&db)
}

#[tauri::command]
pub fn save_sync_settings(db: State<Database>, settings: SyncSettings) -> Result<(), String> {
    db.with_connection(|conn| {
        set_setting(conn, "webdav_url", &settings.webdav_url)?;
        set_setting(conn, "webdav_username", &settings.webdav_username)?;
        set_setting(conn, "webdav_password", &settings.webdav_password)?;
        set_setting(
            conn,
            "webdav_auto_sync",
            if settings.auto_sync { "true" } else { "false" },
        )?;
        set_setting(
            conn,
            "webdav_sync_interval",
            &settings.sync_interval.to_string(),
        )?;
        if let Some(ref last) = settings.last_sync_at {
            set_setting(conn, "webdav_last_sync_at", last)?;
        }
        set_setting(conn, "webdav_device_id", &settings.device_id)?;
        Ok(())
    })
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn webdav_test_connection(
    url: String,
    username: String,
    password: String,
) -> Result<bool, String> {
    let client = WebDavClient::new(&url, &username, &password);
    client.test_connection()
}

/// WebDAV 同步数据格式。
///
/// v2.0 起不再包含 AI Agent 相关数据。反序列化对历史远端 v3.0 数据仍兼容
/// （多余字段被 serde 忽略），因此可以从已升级到 v2.0 的另一端拉到旧数据
/// 并平滑过渡。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncData {
    pub version: String,
    pub device_id: String,
    pub updated_at: String,
    pub todos: Vec<serde_json::Value>,
    pub settings: serde_json::Value,
    pub images: Vec<String>,
}

#[tauri::command]
pub fn webdav_upload_sync(db: State<Database>) -> Result<String, String> {
    let sync_settings = get_sync_settings_internal(&db)?;
    if sync_settings.webdav_url.is_empty() {
        return Err("未配置 WebDAV 服务器".to_string());
    }

    let client = WebDavClient::new(
        &sync_settings.webdav_url,
        &sync_settings.webdav_username,
        &sync_settings.webdav_password,
    );

    // Ensure remote directories
    client.ensure_dir(REMOTE_DIR)?;
    client.ensure_dir(REMOTE_IMAGES_DIR)?;

    // Collect image list once（merge 重试不影响图片列表）
    let images_dir = get_images_dir();
    let mut image_files: Vec<String> = Vec::new();
    if images_dir.exists() {
        if let Ok(entries) = std::fs::read_dir(&images_dir) {
            for entry in entries.flatten() {
                if let Some(name) = entry.file_name().to_str() {
                    image_files.push(name.to_string());
                }
            }
        }
    }

    // 条件 PUT 重试循环：412 → 拉远端 → per-record merge → 重新 PUT，最多 MAX_UPLOAD_RETRY 次
    let mut retry: u32 = 0;
    let now;
    loop {
        // 每轮都重新 export 一次：merge 后 SQLite 已含远端新数据，必须重新生成 sync-data
        let export_json = export_data_internal(&db)?;
        let export_data: serde_json::Value =
            serde_json::from_str(&export_json).map_err(|e| e.to_string())?;

        let attempt_now = chrono::Local::now()
            .format("%Y-%m-%dT%H:%M:%S%:z")
            .to_string();
        let sync_data = SyncData {
            version: export_data
                .get("version")
                .and_then(|v| v.as_str())
                .unwrap_or("4.0")
                .to_string(),
            device_id: sync_settings.device_id.clone(),
            updated_at: attempt_now.clone(),
            todos: export_data
                .get("todos")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default(),
            settings: export_data
                .get("settings")
                .cloned()
                .unwrap_or(serde_json::Value::Null),
            images: image_files.clone(),
        };

        let sync_json = serde_json::to_string(&sync_data).map_err(|e| e.to_string())?;
        let compressed = gzip_compress(sync_json.as_bytes())?;

        // 读取最新 webdav_last_modified（merge 路径会刷新这个值）
        let last_modified = db
            .with_connection(|conn| Ok(get_setting(conn, "webdav_last_modified")))
            .map_err(|e: rusqlite::Error| e.to_string())?
            .filter(|s: &String| !s.is_empty());

        match client.upload_bytes(
            SYNC_DATA_FILE,
            &compressed,
            "application/gzip",
            last_modified.as_deref(),
        )? {
            UploadOutcome::Ok(new_last_modified) => {
                // 成功；用 PUT response 的 Last-Modified 更新 setting，下次 PUT 用最新值
                if let Some(ref lm) = new_last_modified {
                    db.with_connection(|conn| {
                        set_setting(conn, "webdav_last_modified", lm)?;
                        Ok(())
                    })
                    .map_err(|e| e.to_string())?;
                }
                now = attempt_now;
                break;
            }
            UploadOutcome::PreconditionFailed => {
                if retry >= MAX_UPLOAD_RETRY {
                    return Err(format!(
                        "WebDAV 同步冲突，{} 次重试后仍失败",
                        MAX_UPLOAD_RETRY
                    ));
                }
                retry += 1;

                // 拉远端，把 Last-Modified 更新到 setting；并 per-record merge 进本地 SQLite
                let remote_bytes_opt = client.download_bytes(SYNC_DATA_FILE)?;
                if let Some((compressed, remote_last_modified)) = remote_bytes_opt {
                    let remote_json = gzip_decompress(&compressed)?;
                    let remote_data: SyncData = serde_json::from_str(&remote_json)
                        .map_err(|e| format!("解析远程数据失败: {}", e))?;

                    db.with_connection(|conn| {
                        let lm = remote_last_modified.unwrap_or_default();
                        set_setting(conn, "webdav_last_modified", &lm)?;
                        Ok(())
                    })
                    .map_err(|e| e.to_string())?;

                    merge_remote_into_local(&db, &remote_data)?;
                } else {
                    // 412 但又下载不到？理论上不应该发生；清空 Last-Modified 让下一轮裸 PUT
                    db.with_connection(|conn| {
                        set_setting(conn, "webdav_last_modified", "")?;
                        Ok(())
                    })
                    .map_err(|e| e.to_string())?;
                }
                // 循环继续，下一轮重新 export + PUT
            }
        }
    }

    // Upload images (skip if already exists on remote)
    for img_name in &image_files {
        let local_path = images_dir.join(img_name);
        if local_path.exists() {
            let remote_path = format!("{}/{}", REMOTE_IMAGES_DIR, img_name);
            if !client.exists(&remote_path).unwrap_or(false) {
                client.upload_file(&remote_path, &local_path)?;
            }
        }
    }

    // Update last sync time
    db.with_connection(|conn| {
        set_setting(conn, "webdav_last_sync_at", &now)?;
        Ok(())
    })
    .map_err(|e| e.to_string())?;

    Ok(now)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncDownloadResult {
    pub has_remote: bool,
    pub remote_data: Option<SyncData>,
    pub local_updated_at: Option<String>,
    pub remote_updated_at: Option<String>,
    pub has_conflict: bool,
}

#[tauri::command]
pub fn webdav_download_sync(db: State<Database>) -> Result<SyncDownloadResult, String> {
    let sync_settings = get_sync_settings_internal(&db)?;
    if sync_settings.webdav_url.is_empty() {
        return Err("未配置 WebDAV 服务器".to_string());
    }

    let client = WebDavClient::new(
        &sync_settings.webdav_url,
        &sync_settings.webdav_username,
        &sync_settings.webdav_password,
    );

    // Download and decompress sync data
    let remote_bytes = client.download_bytes(SYNC_DATA_FILE)?;

    if remote_bytes.is_none() {
        // 远端没有 sync-data → 清空 webdav_last_modified
        db.with_connection(|conn| {
            set_setting(conn, "webdav_last_modified", "")?;
            Ok(())
        })
        .map_err(|e| e.to_string())?;
        return Ok(SyncDownloadResult {
            has_remote: false,
            remote_data: None,
            local_updated_at: sync_settings.last_sync_at.clone(),
            remote_updated_at: None,
            has_conflict: false,
        });
    }

    let (compressed, remote_last_modified) = remote_bytes.unwrap();

    // 记录 Last-Modified，用于下次条件 PUT
    db.with_connection(|conn| {
        let lm = remote_last_modified.unwrap_or_default();
        set_setting(conn, "webdav_last_modified", &lm)?;
        Ok(())
    })
    .map_err(|e| e.to_string())?;

    let remote_json = gzip_decompress(&compressed)?;
    let remote_data: SyncData =
        serde_json::from_str(&remote_json).map_err(|e| format!("解析远程数据失败: {}", e))?;

    // Check for conflict
    let has_local_changes = check_local_changes(&db, &sync_settings)?;
    let remote_is_newer = is_remote_newer(&sync_settings, &remote_data.updated_at);
    let has_conflict = has_local_changes && remote_is_newer;

    Ok(SyncDownloadResult {
        has_remote: true,
        remote_updated_at: Some(remote_data.updated_at.clone()),
        local_updated_at: sync_settings.last_sync_at.clone(),
        remote_data: Some(remote_data),
        has_conflict,
    })
}

#[tauri::command]
pub fn webdav_apply_remote(db: State<Database>, sync_data_json: String) -> Result<String, String> {
    let sync_data: SyncData =
        serde_json::from_str(&sync_data_json).map_err(|e| format!("解析数据失败: {}", e))?;

    sync_apply_remote(&db, &sync_data)?;

    // Download images
    let sync_settings = get_sync_settings_internal(&db)?;
    let client = WebDavClient::new(
        &sync_settings.webdav_url,
        &sync_settings.webdav_username,
        &sync_settings.webdav_password,
    );

    let images_dir = get_images_dir();
    std::fs::create_dir_all(&images_dir).ok();

    for img_name in &sync_data.images {
        let local_path = images_dir.join(img_name);
        if !local_path.exists() {
            let remote_path = format!("{}/{}", REMOTE_IMAGES_DIR, img_name);
            let _ = client.download_file(&remote_path, &local_path);
        }
    }

    // Update last sync time
    let now = chrono::Local::now()
        .format("%Y-%m-%dT%H:%M:%S%:z")
        .to_string();
    db.with_connection(|conn| {
        set_setting(conn, "webdav_last_sync_at", &now)?;
        Ok(())
    })
    .map_err(|e| e.to_string())?;

    Ok(now)
}

#[tauri::command]
pub fn webdav_auto_sync(db: State<Database>) -> Result<String, String> {
    let sync_settings = get_sync_settings_internal(&db)?;
    if sync_settings.webdav_url.is_empty() || !sync_settings.auto_sync {
        return Err("自动同步未启用".to_string());
    }

    let client = WebDavClient::new(
        &sync_settings.webdav_url,
        &sync_settings.webdav_username,
        &sync_settings.webdav_password,
    );

    let has_local_changes = check_local_changes(&db, &sync_settings)?;

    let remote_bytes = client.download_bytes(SYNC_DATA_FILE).ok().flatten();
    if let Some((compressed, remote_last_modified)) = remote_bytes {
        // 记录 Last-Modified（无论是否要 apply 远端，都更新这个值；下一次 PUT 用它）
        let lm = remote_last_modified.unwrap_or_default();
        db.with_connection(|conn| {
            set_setting(conn, "webdav_last_modified", &lm)?;
            Ok(())
        })
        .map_err(|e| e.to_string())?;

        if let Ok(remote_json) = gzip_decompress(&compressed) {
            if let Ok(remote_data) = serde_json::from_str::<SyncData>(&remote_json) {
                let remote_is_newer = is_remote_newer(&sync_settings, &remote_data.updated_at);

                if remote_is_newer && !has_local_changes {
                    sync_apply_remote(&db, &remote_data)?;

                    let images_dir = get_images_dir();
                    std::fs::create_dir_all(&images_dir).ok();
                    for img_name in &remote_data.images {
                        let local_path = images_dir.join(img_name);
                        if !local_path.exists() {
                            let remote_path = format!("{}/{}", REMOTE_IMAGES_DIR, img_name);
                            let _ = client.download_file(&remote_path, &local_path);
                        }
                    }

                    let now = chrono::Local::now()
                        .format("%Y-%m-%dT%H:%M:%S%:z")
                        .to_string();
                    db.with_connection(|conn| {
                        set_setting(conn, "webdav_last_sync_at", &now)?;
                        Ok(())
                    })
                    .map_err(|e| e.to_string())?;
                    return Ok(now);
                }

                if remote_is_newer && has_local_changes {
                    return Ok("conflict".to_string());
                }
            }
        }
    }

    if has_local_changes {
        return webdav_upload_sync(db);
    }

    Ok("no_changes".to_string())
}

fn get_sync_settings_internal(db: &State<Database>) -> Result<SyncSettings, String> {
    read_sync_settings(db)
}

fn read_sync_settings(db: &Database) -> Result<SyncSettings, String> {
    db.with_connection(|conn| {
        let settings = SyncSettings {
            webdav_url: get_setting(conn, "webdav_url").unwrap_or_default(),
            webdav_username: get_setting(conn, "webdav_username").unwrap_or_default(),
            webdav_password: get_setting(conn, "webdav_password").unwrap_or_default(),
            auto_sync: get_setting(conn, "webdav_auto_sync")
                .map(|v| v == "true")
                .unwrap_or(false),
            sync_interval: get_setting(conn, "webdav_sync_interval")
                .and_then(|v| v.parse().ok())
                .unwrap_or(15),
            last_sync_at: get_setting(conn, "webdav_last_sync_at"),
            device_id: get_setting(conn, "webdav_device_id").unwrap_or_else(generate_device_id),
        };
        Ok(settings)
    })
    .map_err(|e| e.to_string())
}

fn check_local_changes(db: &State<Database>, settings: &SyncSettings) -> Result<bool, String> {
    if settings.last_sync_at.is_none() {
        return Ok(true);
    }

    let last_sync = settings.last_sync_at.as_ref().unwrap();

    db.with_connection(|conn| {
        let todo_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM todos WHERE updated_at > ?1",
                [last_sync],
                |row| row.get(0),
            )
            .unwrap_or(0);
        let subtask_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM subtasks WHERE updated_at > ?1",
                [last_sync],
                |row| row.get(0),
            )
            .unwrap_or(0);
        Ok(todo_count > 0 || subtask_count > 0)
    })
    .map_err(|e| e.to_string())
}

fn is_remote_newer(settings: &SyncSettings, remote_updated_at: &str) -> bool {
    match &settings.last_sync_at {
        Some(last) => remote_updated_at > last.as_str(),
        None => true,
    }
}

fn gzip_compress(data: &[u8]) -> Result<Vec<u8>, String> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder
        .write_all(data)
        .map_err(|e| format!("压缩失败: {}", e))?;
    encoder.finish().map_err(|e| format!("压缩完成失败: {}", e))
}

fn gzip_decompress(data: &[u8]) -> Result<String, String> {
    let mut decoder = GzDecoder::new(data);
    let mut result = String::new();
    decoder
        .read_to_string(&mut result)
        .map_err(|e| format!("解压失败: {}", e))?;
    Ok(result)
}

/// 单次 merge 的统计。`sync_apply_remote` 会把它连同孤儿删除数一起打进日志，
/// 让"远端权威删除"这条静默数据删除路径有迹可查。
#[derive(Debug, Default)]
pub struct MergeStats {
    pub todos_updated: u32,
    pub todos_inserted: u32,
    pub todos_deleted: u32,
    pub subtasks_updated: u32,
    pub subtasks_inserted: u32,
    pub subtasks_deleted: u32,
}

/// per-record LWW merge：把远端 `SyncData` 合并进本地 SQLite。
///
/// 语义：
/// - 远端 todo / subtask 的 `updatedAt` ≥ 本地 → upsert 远端字段
/// - 本地 `updatedAt` > 远端 → 保留本地（不动）
/// - 远端有、本地无 → 直接 INSERT（用远端 id）
/// - 本地有、远端无 → **保留本地**（不删；412 冲突路径需保留本地新增）
/// - settings：本函数**不动 settings**
///
/// 孤儿清理和 settings 写入由上层 `sync_apply_remote` 负责（sync 下载路径），
/// 412 冲突路径直接调本函数不做清理。
///
/// 整个 merge 包在单事务里；中间任何一步失败回滚。
pub fn merge_remote_into_local(db: &Database, remote: &SyncData) -> Result<MergeStats, String> {
    // 把远端 Vec<serde_json::Value> 反序列化为 Vec<Todo>；Todo 自带 subtasks 嵌套
    let remote_todos: Vec<Todo> = remote
        .todos
        .iter()
        .filter_map(|v| serde_json::from_value::<Todo>(v.clone()).ok())
        .collect();

    let stats = db
        .with_connection(|conn| {
            // 单事务封装：savepoint 在 with_connection 内不暴露，直接用 immediate transaction
            conn.execute("BEGIN IMMEDIATE", [])?;
            let result = merge_todos_inner(conn, &remote_todos);
            match result {
                Ok(stats) => {
                    conn.execute("COMMIT", [])?;
                    Ok(stats)
                }
                Err(e) => {
                    let _ = conn.execute("ROLLBACK", []);
                    Err(e)
                }
            }
        })
        .map_err(|e| e.to_string())?;

    Ok(stats)
}

/// `merge_remote_into_local` 的实际实现，假设外部已经包了事务。
fn merge_todos_inner(
    conn: &rusqlite::Connection,
    remote_todos: &[Todo],
) -> rusqlite::Result<MergeStats> {
    let mut stats = MergeStats::default();

    for remote_todo in remote_todos {
        let todo_id = remote_todo.id;
        let local_updated_at: Option<String> = conn
            .query_row(
                "SELECT updated_at FROM todos WHERE id = ?1",
                [todo_id],
                |row| row.get::<_, String>(0),
            )
            .ok();

        let should_apply = match &local_updated_at {
            Some(local) => remote_todo.updated_at.as_str() >= local.as_str(),
            None => true,
        };

        if should_apply {
            let notified_i = if remote_todo.notified { 1i32 } else { 0 };
            let completed_i = if remote_todo.completed { 1i32 } else { 0 };
            let repeat_enabled_i = if remote_todo.repeat_enabled { 1i32 } else { 0 };

            if local_updated_at.is_some() {
                conn.execute(
                    "UPDATE todos SET
                        title = ?1, description = ?2, color = ?3, quadrant = ?4,
                        notify_at = ?5, notify_before = ?6, notified = ?7,
                        completed = ?8, sort_order = ?9, start_time = ?10, end_time = ?11,
                        created_at = ?12, updated_at = ?13,
                        repeat_enabled = ?14, repeat_type = ?15, repeat_interval = ?16,
                        repeat_weekdays = ?17, repeat_month_day = ?18
                     WHERE id = ?19",
                    params![
                        remote_todo.title,
                        remote_todo.description,
                        remote_todo.color,
                        remote_todo.quadrant,
                        remote_todo.notify_at,
                        remote_todo.notify_before,
                        notified_i,
                        completed_i,
                        remote_todo.sort_order,
                        remote_todo.start_time,
                        remote_todo.end_time,
                        remote_todo.created_at,
                        remote_todo.updated_at,
                        repeat_enabled_i,
                        remote_todo.repeat_type,
                        remote_todo.repeat_interval,
                        remote_todo.repeat_weekdays,
                        remote_todo.repeat_month_day,
                        todo_id,
                    ],
                )?;
                stats.todos_updated += 1;
            } else {
                conn.execute(
                    "INSERT INTO todos (id, title, description, color, quadrant,
                                        notify_at, notify_before, notified, completed,
                                        sort_order, start_time, end_time, created_at, updated_at,
                                        repeat_enabled, repeat_type, repeat_interval,
                                        repeat_weekdays, repeat_month_day)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14,
                             ?15, ?16, ?17, ?18, ?19)",
                    params![
                        todo_id,
                        remote_todo.title,
                        remote_todo.description,
                        remote_todo.color,
                        remote_todo.quadrant,
                        remote_todo.notify_at,
                        remote_todo.notify_before,
                        notified_i,
                        completed_i,
                        remote_todo.sort_order,
                        remote_todo.start_time,
                        remote_todo.end_time,
                        remote_todo.created_at,
                        remote_todo.updated_at,
                        repeat_enabled_i,
                        remote_todo.repeat_type,
                        remote_todo.repeat_interval,
                        remote_todo.repeat_weekdays,
                        remote_todo.repeat_month_day,
                    ],
                )?;
                stats.todos_inserted += 1;
            }
        }

        // subtasks 同样 per-record LWW
        for remote_sub in &remote_todo.subtasks {
            merge_subtask(conn, remote_sub, &mut stats)?;
        }
    }

    Ok(stats)
}

fn merge_subtask(
    conn: &rusqlite::Connection,
    remote_sub: &SubTask,
    stats: &mut MergeStats,
) -> rusqlite::Result<()> {
    let local_updated_at: Option<String> = conn
        .query_row(
            "SELECT updated_at FROM subtasks WHERE id = ?1",
            [remote_sub.id],
            |row| row.get::<_, String>(0),
        )
        .ok();

    let should_apply = match &local_updated_at {
        Some(local) => remote_sub.updated_at.as_str() >= local.as_str(),
        None => true,
    };

    if !should_apply {
        return Ok(());
    }

    let completed_i = if remote_sub.completed { 1i32 } else { 0 };

    if local_updated_at.is_some() {
        conn.execute(
            "UPDATE subtasks SET
                parent_id = ?1, title = ?2, content = ?3, completed = ?4,
                sort_order = ?5, created_at = ?6, updated_at = ?7
             WHERE id = ?8",
            params![
                remote_sub.parent_id,
                remote_sub.title,
                remote_sub.content,
                completed_i,
                remote_sub.sort_order,
                remote_sub.created_at,
                remote_sub.updated_at,
                remote_sub.id,
            ],
        )?;
        stats.subtasks_updated += 1;
    } else {
        conn.execute(
            "INSERT INTO subtasks (id, parent_id, title, content, completed,
                                   sort_order, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                remote_sub.id,
                remote_sub.parent_id,
                remote_sub.title,
                remote_sub.content,
                completed_i,
                remote_sub.sort_order,
                remote_sub.created_at,
                remote_sub.updated_at,
            ],
        )?;
        stats.subtasks_inserted += 1;
    }

    Ok(())
}

/// sync 下载统一入口：per-record merge + 孤儿清理 + settings 写入。
///
/// 供 `webdav_apply_remote` 和 `webdav_auto_sync` 共用。与 `merge_remote_into_local`
/// 的区别：本函数在 merge 后会删除"本地有但远端没有"的 todo/subtask（远端权威），
/// 并写入远端 settings。412 冲突路径仍直接调 `merge_remote_into_local`（不删孤儿）。
fn sync_apply_remote(db: &Database, remote: &SyncData) -> Result<(), String> {
    let mut stats = merge_remote_into_local(db, remote)?;
    let (todos_deleted, subtasks_deleted) = delete_orphan_todos(db, remote)?;
    stats.todos_deleted = todos_deleted;
    stats.subtasks_deleted = subtasks_deleted;
    eprintln!(
        "[sync] 应用远端完成: todos 新增 {} / 更新 {} / 删除 {}, subtasks 新增 {} / 更新 {} / 删除 {}",
        stats.todos_inserted,
        stats.todos_updated,
        stats.todos_deleted,
        stats.subtasks_inserted,
        stats.subtasks_updated,
        stats.subtasks_deleted
    );

    // settings：从远端 SyncData 解析并写入
    if !remote.settings.is_null() {
        if let Ok(settings) =
            serde_json::from_value::<crate::db::AppSettings>(remote.settings.clone())
        {
            db.with_connection(|conn| write_app_settings(conn, &settings))
                .map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// 删除"本地有但远端没有"的 todos + 对应 subtasks，返回 `(todos_deleted, subtasks_deleted)`。
///
/// 从 `serde_json::Value` 直接提取 id，不依赖完整 `Todo` 反序列化——
/// 如果某条远端 todo 格式异常无法反序列化为 `Todo`，仍能保住其 id，
/// 避免本地正常记录被误判为孤儿。
fn delete_orphan_todos(db: &Database, remote: &SyncData) -> Result<(u32, u32), String> {
    fn extract_i64_id(v: &serde_json::Value) -> Option<i64> {
        let raw = v.get("id")?;
        raw.as_i64()
    }

    let mut remote_todo_ids = std::collections::HashSet::new();
    let mut remote_subtask_ids = std::collections::HashSet::new();

    for todo in &remote.todos {
        if let Some(id) = extract_i64_id(todo) {
            remote_todo_ids.insert(id);
        }
        if let Some(subs) = todo.get("subtasks").and_then(|v| v.as_array()) {
            for sub in subs {
                if let Some(sid) = extract_i64_id(sub) {
                    remote_subtask_ids.insert(sid);
                }
            }
        }
    }

    db.with_connection(|conn| {
        conn.execute("BEGIN IMMEDIATE", [])?;

        let mut stmt = conn.prepare("SELECT id FROM todos")?;
        let local_todo_ids: Vec<i64> = stmt
            .query_map([], |row| row.get::<_, i64>(0))?
            .filter_map(|r| r.ok())
            .collect();

        let mut todos_deleted = 0u32;
        let mut subtasks_deleted = 0u32;

        for id in local_todo_ids {
            if !remote_todo_ids.contains(&id) {
                subtasks_deleted +=
                    conn.execute("DELETE FROM subtasks WHERE parent_id = ?1", [id])? as u32;
                todos_deleted += conn.execute("DELETE FROM todos WHERE id = ?1", [id])? as u32;
            }
        }

        let mut sub_stmt = conn.prepare("SELECT id FROM subtasks")?;
        let local_sub_ids: Vec<i64> = sub_stmt
            .query_map([], |row| row.get::<_, i64>(0))?
            .filter_map(|r| r.ok())
            .collect();

        for id in local_sub_ids {
            if !remote_subtask_ids.contains(&id) {
                subtasks_deleted +=
                    conn.execute("DELETE FROM subtasks WHERE id = ?1", [id])? as u32;
            }
        }

        conn.execute("COMMIT", [])?;
        Ok((todos_deleted, subtasks_deleted))
    })
    .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{Database, SubTask, Todo};

    fn test_db() -> Database {
        Database::new_in_memory().expect("打开内存库失败")
    }

    fn make_todo(id: i64, title: &str, updated_at: &str) -> Todo {
        Todo {
            id,
            title: title.to_string(),
            description: None,
            color: "#EF4444".to_string(),
            quadrant: 4,
            notify_at: None,
            notify_before: 0,
            notified: false,
            completed: false,
            sort_order: 0,
            start_time: None,
            end_time: None,
            created_at: "2026-01-01 00:00:00".to_string(),
            updated_at: updated_at.to_string(),
            repeat_enabled: false,
            repeat_type: None,
            repeat_interval: 1,
            repeat_weekdays: None,
            repeat_month_day: None,
            subtasks: Vec::new(),
        }
    }

    fn make_subtask(id: i64, parent_id: i64, title: &str, updated_at: &str) -> SubTask {
        SubTask {
            id,
            parent_id,
            title: title.to_string(),
            content: None,
            completed: false,
            sort_order: 0,
            created_at: "2026-01-01 00:00:00".to_string(),
            updated_at: updated_at.to_string(),
        }
    }

    fn sync_data(todos: Vec<Todo>) -> SyncData {
        SyncData {
            version: "4.0".to_string(),
            device_id: "test-dev".to_string(),
            updated_at: "2026-01-02 00:00:00".to_string(),
            todos: todos
                .iter()
                .map(|t| serde_json::to_value(t).expect("序列化 Todo 失败"))
                .collect(),
            settings: serde_json::Value::Null,
            images: Vec::new(),
        }
    }

    fn todo_title(db: &Database, id: i64) -> Option<String> {
        db.with_connection(|conn| {
            Ok(conn
                .query_row("SELECT title FROM todos WHERE id = ?1", [id], |r| {
                    r.get::<_, String>(0)
                })
                .ok())
        })
        .unwrap()
    }

    fn count_subtasks(db: &Database) -> i64 {
        db.with_connection(|conn| {
            conn.query_row("SELECT COUNT(*) FROM subtasks", [], |r| r.get(0))
        })
        .unwrap()
    }

    #[test]
    fn merge_inserts_new_remote_todo_and_subtask() {
        let db = test_db();
        let mut t = make_todo(1, "远端新增", "2026-01-02 10:00:00");
        t.subtasks
            .push(make_subtask(11, 1, "子任务", "2026-01-02 10:00:00"));

        let stats = merge_remote_into_local(&db, &sync_data(vec![t])).unwrap();

        assert_eq!(stats.todos_inserted, 1);
        assert_eq!(stats.subtasks_inserted, 1);
        assert_eq!(todo_title(&db, 1).as_deref(), Some("远端新增"));
        assert_eq!(count_subtasks(&db), 1);
    }

    #[test]
    fn merge_remote_newer_overwrites_local() {
        let db = test_db();
        merge_remote_into_local(&db, &sync_data(vec![make_todo(1, "旧标题", "2026-01-01 10:00:00")]))
            .unwrap();

        let stats = merge_remote_into_local(
            &db,
            &sync_data(vec![make_todo(1, "新标题", "2026-01-03 10:00:00")]),
        )
        .unwrap();

        assert_eq!(stats.todos_updated, 1);
        assert_eq!(todo_title(&db, 1).as_deref(), Some("新标题"));
    }

    #[test]
    fn merge_local_newer_is_kept() {
        let db = test_db();
        merge_remote_into_local(
            &db,
            &sync_data(vec![make_todo(1, "本地较新", "2026-01-05 10:00:00")]),
        )
        .unwrap();

        let stats = merge_remote_into_local(
            &db,
            &sync_data(vec![make_todo(1, "远端较旧", "2026-01-02 10:00:00")]),
        )
        .unwrap();

        assert_eq!(stats.todos_updated, 0);
        assert_eq!(todo_title(&db, 1).as_deref(), Some("本地较新"));
    }

    /// 412 冲突路径语义：merge 本身不删"本地有远端无"的记录。
    #[test]
    fn merge_keeps_local_only_records() {
        let db = test_db();
        merge_remote_into_local(
            &db,
            &sync_data(vec![make_todo(1, "本地独有", "2026-01-01 10:00:00")]),
        )
        .unwrap();

        merge_remote_into_local(&db, &sync_data(vec![make_todo(2, "远端", "2026-01-01 10:00:00")]))
            .unwrap();

        assert!(todo_title(&db, 1).is_some());
        assert!(todo_title(&db, 2).is_some());
    }

    /// sync 下载路径语义：远端权威，本地孤儿（连带子任务）被删除。
    #[test]
    fn apply_remote_deletes_local_orphans() {
        let db = test_db();
        let mut t1 = make_todo(1, "会被删", "2026-01-01 10:00:00");
        t1.subtasks
            .push(make_subtask(11, 1, "随父删", "2026-01-01 10:00:00"));
        merge_remote_into_local(
            &db,
            &sync_data(vec![t1, make_todo(2, "保留", "2026-01-01 10:00:00")]),
        )
        .unwrap();

        sync_apply_remote(&db, &sync_data(vec![make_todo(2, "保留", "2026-01-01 10:00:00")]))
            .unwrap();

        assert!(todo_title(&db, 1).is_none());
        assert!(todo_title(&db, 2).is_some());
        assert_eq!(count_subtasks(&db), 0);
    }

    #[test]
    fn orphan_cleanup_returns_deletion_counts() {
        let db = test_db();
        let mut t1 = make_todo(1, "删", "2026-01-01 10:00:00");
        t1.subtasks
            .push(make_subtask(11, 1, "子", "2026-01-01 10:00:00"));
        merge_remote_into_local(&db, &sync_data(vec![t1])).unwrap();

        let (todos_deleted, subtasks_deleted) =
            delete_orphan_todos(&db, &sync_data(vec![])).unwrap();

        assert_eq!(todos_deleted, 1);
        assert_eq!(subtasks_deleted, 1);
    }

    /// 远端某条 todo 格式异常（无法反序列化为 Todo）但带 id 时，
    /// 该 id 仍参与孤儿判定，对应本地记录不能被误删。
    #[test]
    fn orphan_cleanup_spares_undeserializable_remote_todo() {
        let db = test_db();
        merge_remote_into_local(
            &db,
            &sync_data(vec![make_todo(42, "格式异常保护", "2026-01-01 10:00:00")]),
        )
        .unwrap();

        let mut data = sync_data(vec![]);
        data.todos
            .push(serde_json::json!({"id": 42, "malformed": true}));
        sync_apply_remote(&db, &data).unwrap();

        assert!(todo_title(&db, 42).is_some());
    }

    /// 父 todo 保留、但远端不再包含其中某个 subtask 时，该 subtask 单独被删。
    #[test]
    fn apply_remote_deletes_orphan_subtask_of_kept_todo() {
        let db = test_db();
        let mut t = make_todo(1, "父", "2026-01-01 10:00:00");
        t.subtasks
            .push(make_subtask(11, 1, "留", "2026-01-01 10:00:00"));
        t.subtasks
            .push(make_subtask(12, 1, "删", "2026-01-01 10:00:00"));
        merge_remote_into_local(&db, &sync_data(vec![t])).unwrap();

        let mut t2 = make_todo(1, "父", "2026-01-01 10:00:00");
        t2.subtasks
            .push(make_subtask(11, 1, "留", "2026-01-01 10:00:00"));
        sync_apply_remote(&db, &sync_data(vec![t2])).unwrap();

        assert_eq!(count_subtasks(&db), 1);
    }
}
