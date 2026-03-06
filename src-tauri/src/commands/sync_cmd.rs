use crate::db::Database;
use crate::services::webdav::WebDavClient;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use serde::{Deserialize, Serialize};
use std::io::{Read as _, Write as _};
use std::path::PathBuf;
use tauri::State;
use super::data::{export_data_internal, import_data_raw};

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
    conn.query_row(
        "SELECT value FROM settings WHERE key = ?1",
        [key],
        |row| row.get(0),
    )
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
    read_sync_settings(&*db)
}

#[tauri::command]
pub fn save_sync_settings(db: State<Database>, settings: SyncSettings) -> Result<(), String> {
    db.with_connection(|conn| {
        set_setting(conn, "webdav_url", &settings.webdav_url)?;
        set_setting(conn, "webdav_username", &settings.webdav_username)?;
        set_setting(conn, "webdav_password", &settings.webdav_password)?;
        set_setting(conn, "webdav_auto_sync", if settings.auto_sync { "true" } else { "false" })?;
        set_setting(conn, "webdav_sync_interval", &settings.sync_interval.to_string())?;
        if let Some(ref last) = settings.last_sync_at {
            set_setting(conn, "webdav_last_sync_at", last)?;
        }
        set_setting(conn, "webdav_device_id", &settings.device_id)?;
        Ok(())
    })
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn webdav_test_connection(url: String, username: String, password: String) -> Result<bool, String> {
    let client = WebDavClient::new(&url, &username, &password);
    client.test_connection()
}

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

    // Export data
    let export_json = export_data_internal(&*db)?;

    // Collect image list
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

    // Build sync data
    let export_data: serde_json::Value =
        serde_json::from_str(&export_json).map_err(|e| e.to_string())?;

    let now = chrono::Local::now().format("%Y-%m-%dT%H:%M:%S%:z").to_string();
    let sync_data = SyncData {
        version: "2.0".to_string(),
        device_id: sync_settings.device_id.clone(),
        updated_at: now.clone(),
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

    // Compress and upload sync data
    let compressed = gzip_compress(sync_json.as_bytes())?;
    client.upload_bytes(SYNC_DATA_FILE, &compressed, "application/gzip")?;

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
        return Ok(SyncDownloadResult {
            has_remote: false,
            remote_data: None,
            local_updated_at: sync_settings.last_sync_at.clone(),
            remote_updated_at: None,
            has_conflict: false,
        });
    }

    let compressed = remote_bytes.unwrap();
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

    // Import the data using existing import logic
    let import_json = serde_json::json!({
        "version": sync_data.version,
        "exportedAt": sync_data.updated_at,
        "todos": sync_data.todos,
        "settings": sync_data.settings,
    });

    let import_str = serde_json::to_string(&import_json).map_err(|e| e.to_string())?;
    import_data_raw(&*db, &import_str)?;

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
    let now = chrono::Local::now().format("%Y-%m-%dT%H:%M:%S%:z").to_string();
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
    if let Some(compressed) = remote_bytes {
        if let Ok(remote_json) = gzip_decompress(&compressed) {
            if let Ok(remote_data) = serde_json::from_str::<SyncData>(&remote_json) {
                let remote_is_newer = is_remote_newer(&sync_settings, &remote_data.updated_at);

                if remote_is_newer && !has_local_changes {
                    let import_json = serde_json::json!({
                        "version": remote_data.version,
                        "exportedAt": remote_data.updated_at,
                        "todos": remote_data.todos,
                        "settings": remote_data.settings,
                    });
                    let import_str = serde_json::to_string(&import_json).map_err(|e| e.to_string())?;
                    import_data_raw(&*db, &import_str)?;

                    let images_dir = get_images_dir();
                    std::fs::create_dir_all(&images_dir).ok();
                    for img_name in &remote_data.images {
                        let local_path = images_dir.join(img_name);
                        if !local_path.exists() {
                            let remote_path = format!("{}/{}", REMOTE_IMAGES_DIR, img_name);
                            let _ = client.download_file(&remote_path, &local_path);
                        }
                    }

                    let now = chrono::Local::now().format("%Y-%m-%dT%H:%M:%S%:z").to_string();
                    db.with_connection(|conn| {
                        set_setting(conn, "webdav_last_sync_at", &now)?;
                        Ok(())
                    }).map_err(|e| e.to_string())?;
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
    read_sync_settings(&**db)
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
            device_id: get_setting(conn, "webdav_device_id")
                .unwrap_or_else(generate_device_id),
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
    encoder.write_all(data).map_err(|e| format!("压缩失败: {}", e))?;
    encoder.finish().map_err(|e| format!("压缩完成失败: {}", e))
}

fn gzip_decompress(data: &[u8]) -> Result<String, String> {
    let mut decoder = GzDecoder::new(data);
    let mut result = String::new();
    decoder.read_to_string(&mut result).map_err(|e| format!("解压失败: {}", e))?;
    Ok(result)
}
