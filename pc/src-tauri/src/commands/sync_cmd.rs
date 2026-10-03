//! WebDAV 同步命令（10-02 全面优化 R1：契约 K1–K5、K7）。
//!
//! 一次"智能同步"（[`webdav_sync`]，手动按钮 / 设置页"立即同步" / 自动同步定时器共用）：
//!
//! ```text
//! 互斥（同一时刻只允许一个同步，含强制拉取 / 推送）
//! 最多 4 轮：
//!   本地有未同步变更（local_seq ≠ synced_seq，或设置版本 > 基准里的 settingsUpdatedAt）、
//!   手动同步（`full: true`）或每第 FULL_GET_EVERY 轮 → 无条件 GET；
//!   否则基准里有 ETag 时带 If-None-Match 的条件 GET，304 → 无变化，结束
//!     （不用 If-Modified-Since：nginx 按整秒比较，同一秒内的写入会被漏掉；
//!      nginx 的 ETag 也只到秒，同秒等长的改写同样 304，所以要定期无条件 GET 兜底）
//!   404 → 远端为空；200 → 解析（失败直接报错，绝不上传覆盖读不懂的远端）
//!   单事务合并：墓碑并集 → 墓碑应用到本地 → 记录级 LWW（平局保留本地）→ 远端设置（K3-5）
//!   下载远端列出、本地缺失的图片
//!   生成上传文档：本地 ∪ 远端（墓碑压制），保留远端的未知顶层键与本地无法识别的记录
//!   不需要上传（无本地变更且文档与远端等价）→ 记基准，结束
//!   先传缺失图片，再条件 PUT（知道 ETag → If-Match: "<opaque>"，否则不带条件；
//!   绝不发 If-Unmodified-Since：Apache 拿亚秒 mtime 比较，永远 412）
//!     成功 → 记基准（PUT 响应的 ETag，缺失时依次 HEAD、PROPFIND Depth 0）、synced_seq、
//!            设置基准，结束
//!     412 → 等待 ≥1.1s（Apache 刚写完的一秒内只给弱 ETag，If-Match 必然 412）后
//!           下一轮无条件 GET + 合并；404 / 409 → 建目录后重试
//! 4 轮都冲突 → 报错
//! ```
//!
//! 基准（`webdav_remote_etag` / `webdav_last_modified` / `webdav_synced_settings_at`）只在
//! "远端内容已完整合并且本地没有需要上传的东西"或"上传成功"之后才落库：永远不会把一个
//! 没合并过的 GET 记成基准（A2 / A4）。条件请求只用 ETag；Last-Modified 只作记录。
//!
//! 删除通过墓碑传播；正常同步里没有"缺席即删除"。手动导入走 `data::import_data_raw`。

use super::data::{adopt_settings_json, read_app_settings, settings_version, EXPORT_VERSION};
use crate::db::models::Tombstone;
use crate::db::paths::{self, is_safe_image_name};
use crate::db::settings_kv::{
    get_bool_setting, get_setting, get_setting_or, set_bool_setting, set_setting,
};
use crate::db::sync_store::{self, EntityKind, LocalRecords, MergeStats, TombstoneIndex};
use crate::db::time::{normalize_datetime, now_local, DefaultTime};
use crate::db::{AppSettings, Database};
use crate::services::secret;
use crate::services::webdav::{
    choose_put_precondition, GetOutcome, Precondition, PropMeta, PutOutcome, RemoteVersion,
    WebDavClient,
};
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use tauri::{AppHandle, Emitter, Manager, State};

/// 一次同步里 GET → 合并 → PUT 的最大轮数（412 重试上限）
const MAX_ATTEMPTS: usize = 4;

/// 每隔这么多轮智能同步做一次无条件 GET（与 cloud 的 `FULL_PULL_EVERY` 同一规则）。
///
/// nginx dav 的 ETag 是 `"<mtime 秒十六进制>-<大小十六进制>"`：另一写入方在本机记下基准的同一秒内
/// 改写了 sync-data、且压缩后大小恰好不变（等长的标题修改、象限 1→2 ……）时 ETag 不变，
/// 带 `If-None-Match` 的条件 GET 得到 304，这次修改要等到下一次有人写入才会被看到。
/// 自动同步每 10 轮无条件 GET 一次兜底（盲区最多延迟 10 个周期）；手动同步（`full: true`）每次都无条件 GET。
pub const FULL_GET_EVERY: u32 = 10;

/// 412 之后、下一轮 GET 之前的等待时间。Apache mod_dav 在文件写入后约 1 秒内只给弱 ETag，
/// 这期间 `If-Match` 一定 412；等过这一秒再取到的就是可以强匹配的 ETag。
const CONFLICT_RETRY_DELAY: std::time::Duration = std::time::Duration::from_millis(1100);

const REMOTE_DIR: &str = "/mini-todo";
const SYNC_DATA_FILE: &str = "/mini-todo/sync-data.json.gz";
const REMOTE_IMAGES_DIR: &str = "/mini-todo/images";

/// 远端同步文档解压后的大小上限，防止异常 / 恶意数据把内存撑爆
const MAX_SYNC_DOC_BYTES: u64 = 256 * 1024 * 1024;

const KEY_URL: &str = "webdav_url";
const KEY_USERNAME: &str = "webdav_username";
const KEY_PASSWORD: &str = "webdav_password";
const KEY_AUTO_SYNC: &str = "webdav_auto_sync";
const KEY_SYNC_INTERVAL: &str = "webdav_sync_interval";
const KEY_LAST_SYNC_AT: &str = "webdav_last_sync_at";
const KEY_DEVICE_ID: &str = "webdav_device_id";
const KEY_REMOTE_ETAG: &str = "webdav_remote_etag";
const KEY_LAST_MODIFIED: &str = "webdav_last_modified";
const KEY_SYNCED_SETTINGS_AT: &str = "webdav_synced_settings_at";

// ============================================================================
// 同步设置（前端 DTO）
// ============================================================================

fn default_sync_interval() -> i32 {
    15
}

/// 同步设置。
///
/// - 读取（`get_sync_settings`）：`webdavPassword` 恒为空串，密码不回传 WebView；`hasPassword` 表示已保存过密码
/// - 保存（`save_sync_settings`）：`webdavPassword` 为空表示保留已保存的密码；`clearPassword: true` 显式清空。
///   `hasPassword` / `lastSyncAt` / `deviceId` 由后端维护，保存时忽略
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncSettings {
    #[serde(default)]
    pub webdav_url: String,
    #[serde(default)]
    pub webdav_username: String,
    #[serde(default)]
    pub webdav_password: String,
    #[serde(default)]
    pub has_password: bool,
    #[serde(default)]
    pub auto_sync: bool,
    #[serde(default = "default_sync_interval")]
    pub sync_interval: i32,
    #[serde(default)]
    pub last_sync_at: Option<String>,
    #[serde(default)]
    pub device_id: String,
    #[serde(default, skip_serializing)]
    pub clear_password: bool,
}

fn generate_device_id() -> String {
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or_default();
    format!("dev_{}", millis)
}

/// 读取设备 id，没有就生成并保存
fn ensure_device_id(conn: &rusqlite::Connection) -> rusqlite::Result<String> {
    if let Some(id) = get_setting(conn, KEY_DEVICE_ID)?.filter(|s| !s.is_empty()) {
        return Ok(id);
    }
    let id = generate_device_id();
    set_setting(conn, KEY_DEVICE_ID, &id)?;
    Ok(id)
}

#[tauri::command]
pub fn get_sync_settings(db: State<Database>) -> Result<SyncSettings, String> {
    read_sync_settings(&db)
}

fn read_sync_settings(db: &Database) -> Result<SyncSettings, String> {
    db.with_connection(|conn| {
        Ok(SyncSettings {
            webdav_url: get_setting_or(conn, KEY_URL, ""),
            webdav_username: get_setting_or(conn, KEY_USERNAME, ""),
            webdav_password: String::new(),
            has_password: !get_setting_or(conn, KEY_PASSWORD, "").is_empty(),
            auto_sync: get_bool_setting(conn, KEY_AUTO_SYNC, false),
            sync_interval: get_setting(conn, KEY_SYNC_INTERVAL)?
                .and_then(|v| v.parse().ok())
                .unwrap_or_else(default_sync_interval),
            last_sync_at: get_setting(conn, KEY_LAST_SYNC_AT)?.filter(|s| !s.is_empty()),
            device_id: ensure_device_id(conn)?,
            clear_password: false,
        })
    })
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn save_sync_settings(db: State<Database>, settings: SyncSettings) -> Result<(), String> {
    save_sync_settings_inner(&db, &settings)
}

fn save_sync_settings_inner(db: &Database, settings: &SyncSettings) -> Result<(), String> {
    let url = settings.webdav_url.trim().to_string();
    let username = settings.webdav_username.clone();

    // 密码：显式清空 > 新密码（加密）> 保留；保留时把旧版明文顺手加密
    let stored = db
        .with_connection(|conn| Ok(get_setting_or(conn, KEY_PASSWORD, "")))
        .map_err(|e: rusqlite::Error| e.to_string())?;
    let new_password = if settings.clear_password {
        Some(String::new())
    } else if !settings.webdav_password.is_empty() {
        Some(secret::protect(&settings.webdav_password)?)
    } else if !stored.is_empty() && !secret::is_protected(&stored) {
        match secret::protect(&stored) {
            Ok(p) if p != stored => Some(p),
            _ => None,
        }
    } else {
        None
    };
    let interval = settings.sync_interval.max(1);

    db.with_transaction(|tx| -> rusqlite::Result<()> {
        let old_url = get_setting_or(tx, KEY_URL, "");
        let old_username = get_setting_or(tx, KEY_USERNAME, "");

        set_setting(tx, KEY_URL, &url)?;
        set_setting(tx, KEY_USERNAME, &username)?;
        if let Some(password) = &new_password {
            set_setting(tx, KEY_PASSWORD, password)?;
        }
        set_bool_setting(tx, KEY_AUTO_SYNC, settings.auto_sync)?;
        set_setting(tx, KEY_SYNC_INTERVAL, &interval.to_string())?;
        ensure_device_id(tx)?;

        // 换了服务器 / 账号：旧基准属于另一个远端，清掉，下次同步完整合并一次
        if old_url != url || old_username != username {
            for key in [
                KEY_REMOTE_ETAG,
                KEY_LAST_MODIFIED,
                KEY_SYNCED_SETTINGS_AT,
                KEY_LAST_SYNC_AT,
            ] {
                set_setting(tx, key, "")?;
            }
            sync_store::set_synced_seq(tx, 0)?;
        }
        Ok(())
    })
    .map_err(|e| e.to_string())
}

fn stored_password(db: &Database) -> Result<String, String> {
    let stored = db
        .with_connection(|conn| Ok(get_setting_or(conn, KEY_PASSWORD, "")))
        .map_err(|e: rusqlite::Error| e.to_string())?;
    secret::reveal(&stored)
}

/// 测试连接。`password` 为空 / 缺省时使用已保存的密码（前端拿不到明文密码）。
#[tauri::command]
pub async fn webdav_test_connection(
    app: AppHandle,
    url: String,
    username: String,
    password: Option<String>,
) -> Result<bool, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let password = match password.filter(|p| !p.is_empty()) {
            Some(p) => p,
            None => stored_password(&app.state::<Database>())?,
        };
        WebDavClient::new(&url, &username, &password)?.test_connection()
    })
    .await
    .map_err(|e| format!("连接测试异常: {}", e))?
}

// ============================================================================
// 同步文档（K2）
// ============================================================================

/// `sync-data.json.gz` 的顶层结构（K2）。
///
/// 反序列化尽量宽容：标量字段类型不对时取默认值；单条墓碑 / 图片名无法识别时跳过；
/// 但 `todos` / `tombstones` 不是数组（结构都看不懂）时报错，同步随之中止。
/// 未声明的顶层键收进 `extra`，下次上传原样写回（前向兼容）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncData {
    #[serde(default, deserialize_with = "de_lenient_string")]
    pub version: String,
    #[serde(default, deserialize_with = "de_lenient_string")]
    pub device_id: String,
    /// 元信息：文档生成时间（ISO 8601 带偏移），不参与合并判断
    #[serde(default, deserialize_with = "de_lenient_string")]
    pub updated_at: String,
    /// 待办（嵌套 subtasks），原始 JSON
    #[serde(default, deserialize_with = "de_array")]
    pub todos: Vec<Value>,
    /// PC 的 AppSettings 对象，cloud 原样透传
    #[serde(default)]
    pub settings: Value,
    /// settings 的版本（K1 规范时间）；缺失表示旧版写入方 / cloud 占位，不会被应用
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_settings_updated_at"
    )]
    pub settings_updated_at: Option<String>,
    #[serde(default, deserialize_with = "de_images")]
    pub images: Vec<String>,
    #[serde(default, deserialize_with = "de_tombstones")]
    pub tombstones: Vec<Tombstone>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

fn de_lenient_string<'de, D: Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    Ok(match Value::deserialize(d)? {
        Value::String(s) => s,
        Value::Null => String::new(),
        other => other.to_string(),
    })
}

fn de_array<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<Value>, D::Error> {
    match Value::deserialize(d)? {
        Value::Array(items) => Ok(items),
        Value::Null => Ok(Vec::new()),
        _ => Err(serde::de::Error::custom("todos 必须是数组")),
    }
}

fn de_settings_updated_at<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    Ok(match Value::deserialize(d)? {
        Value::String(s) => {
            let normalized = normalize_datetime(&s, DefaultTime::StartOfDay);
            if normalized.is_none() {
                log::warn!("[sync] 忽略无法识别的 settingsUpdatedAt: {:?}", s);
            }
            normalized
        }
        _ => None,
    })
}

fn de_images<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<String>, D::Error> {
    Ok(match Value::deserialize(d)? {
        Value::Array(items) => items
            .into_iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect(),
        _ => Vec::new(),
    })
}

fn de_tombstones<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<Tombstone>, D::Error> {
    match Value::deserialize(d)? {
        Value::Array(items) => Ok(items
            .iter()
            .filter_map(|v| {
                let parsed = sync_store::parse_tombstone(v);
                if parsed.is_none() {
                    log::warn!("[sync] 忽略无法识别的远端墓碑: {}", v);
                }
                parsed
            })
            .collect()),
        Value::Null => Ok(Vec::new()),
        _ => Err(serde::de::Error::custom("tombstones 必须是数组")),
    }
}

fn gzip_compress(data: &[u8]) -> Result<Vec<u8>, String> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder
        .write_all(data)
        .map_err(|e| format!("压缩失败: {}", e))?;
    encoder.finish().map_err(|e| format!("压缩完成失败: {}", e))
}

fn gzip_decompress(data: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    GzDecoder::new(data)
        .take(MAX_SYNC_DOC_BYTES + 1)
        .read_to_end(&mut out)
        .map_err(|e| format!("解压失败: {}", e))?;
    if out.len() as u64 > MAX_SYNC_DOC_BYTES {
        return Err("远端同步数据过大".to_string());
    }
    Ok(out)
}

/// 解码远端 `sync-data.json.gz`（也接受未压缩的 JSON）。任何结构性问题都返回错误。
pub(crate) fn decode_sync_doc(bytes: &[u8]) -> Result<SyncData, String> {
    let json = if bytes.starts_with(&[0x1f, 0x8b]) {
        gzip_decompress(bytes)?
    } else {
        bytes.to_vec()
    };
    let value: Value =
        serde_json::from_slice(&json).map_err(|e| format!("解析远程数据失败: {}", e))?;
    if !value.is_object() {
        return Err("解析远程数据失败: 顶层不是 JSON 对象".to_string());
    }
    serde_json::from_value(value).map_err(|e| format!("解析远程数据失败: {}", e))
}

fn iso_now() -> String {
    chrono::Local::now()
        .format("%Y-%m-%dT%H:%M:%S%:z")
        .to_string()
}

// ============================================================================
// 同步结果（K7）
// ============================================================================

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncStatus {
    /// 两边都没有变化
    #[default]
    NoChanges,
    /// 只有远端变化落到了本地
    Pulled,
    /// 只有本地变化推到了远端
    Pushed,
    /// 双向都有
    Merged,
}

/// 一次同步的结果，同时作为 `sync-completed` 事件的 payload
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncReport {
    pub status: SyncStatus,
    /// 本次同步完成时间（ISO 8601 带偏移，元信息）
    pub last_sync_at: String,
    pub todos_inserted: u32,
    pub todos_updated: u32,
    pub todos_deleted: u32,
    pub subtasks_inserted: u32,
    pub subtasks_updated: u32,
    pub subtasks_deleted: u32,
    /// 无法识别 / 无法落库而跳过的远端记录（不会被当作删除）
    pub records_skipped: u32,
    pub images_uploaded: u32,
    pub images_downloaded: u32,
    pub settings_applied: bool,
}

impl SyncReport {
    fn add_merge(&mut self, stats: &MergeStats, settings_applied: bool) {
        self.todos_inserted += stats.todos_inserted;
        self.todos_updated += stats.todos_updated;
        self.todos_deleted += stats.todos_deleted;
        self.subtasks_inserted += stats.subtasks_inserted;
        self.subtasks_updated += stats.subtasks_updated;
        self.subtasks_deleted += stats.subtasks_deleted;
        // 412 重试会把同一份远端再合并一遍，跳过数取最后一次
        self.records_skipped = stats.records_skipped;
        self.settings_applied |= settings_applied;
    }

    /// 本地数据或设置是否被改动（决定是否通知前端刷新）
    pub fn local_changed(&self) -> bool {
        self.todos_inserted
            + self.todos_updated
            + self.todos_deleted
            + self.subtasks_inserted
            + self.subtasks_updated
            + self.subtasks_deleted
            > 0
            || self.settings_applied
    }

    fn finalize(&mut self, pushed: bool, last_sync_at: String, images: &ImageTransfer<'_>) {
        self.images_uploaded = images.uploaded;
        self.images_downloaded = images.downloaded;
        let pulled = self.local_changed() || self.images_downloaded > 0;
        self.status = match (pulled, pushed) {
            (false, false) => SyncStatus::NoChanges,
            (true, false) => SyncStatus::Pulled,
            (false, true) => SyncStatus::Pushed,
            (true, true) => SyncStatus::Merged,
        };
        self.last_sync_at = last_sync_at;
    }
}

// ============================================================================
// 命令
// ============================================================================

static SYNC_RUNNING: AtomicBool = AtomicBool::new(false);

/// 进程内智能同步的轮次计数（拿到同步互斥后加一，第一轮为 1）
static SMART_SYNC_ROUNDS: AtomicU32 = AtomicU32::new(0);

/// 第 `round` 轮（从 1 起）智能同步是否做无条件 GET（同 cloud `is_full_pull_round`）
fn is_full_get_round(round: u32) -> bool {
    round > 0 && round.is_multiple_of(FULL_GET_EVERY)
}

/// 这一轮是否无条件 GET：调用方明确要求（手动同步传 `full: true`）或轮到兜底轮
fn wants_full_get(requested: Option<bool>, round: u32) -> bool {
    requested == Some(true) || is_full_get_round(round)
}

/// 同步互斥：同一时刻只允许一个同步（含强制拉取 / 推送），重入直接报错
struct SyncGuard;

impl SyncGuard {
    fn acquire() -> Result<Self, String> {
        SYNC_RUNNING
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map(|_| SyncGuard)
            .map_err(|_| "同步正在进行中".to_string())
    }
}

impl Drop for SyncGuard {
    fn drop(&mut self) {
        SYNC_RUNNING.store(false, Ordering::Release);
    }
}

#[derive(Debug, Clone, Copy)]
enum SyncMode {
    /// `full`：调用方要求本轮无条件 GET（`None` = 旧调用方没传，按 false 处理）
    Smart {
        full: Option<bool>,
    },
    ForcePull,
    ForcePush,
}

/// 智能同步（K4）：手动同步、设置页"立即同步"、自动同步定时器共用。
///
/// `full: true`（标题栏同步按钮、设置页"立即同步"）：本轮第一次 GET 不带 `If-None-Match`，
/// 绕开 nginx 秒级 ETag 的盲区（见 [`FULL_GET_EVERY`]）；自动同步不传，每第 10 轮自动无条件一次。
#[tauri::command]
pub async fn webdav_sync(app: AppHandle, full: Option<bool>) -> Result<SyncReport, String> {
    run_sync_command(app, SyncMode::Smart { full }).await
}

/// 用云端覆盖本地：删除本地独有的待办 / 子任务（不生成墓碑），应用远端设置（设备相关键除外）。
/// 远端不存在时报错。
#[tauri::command]
pub async fn webdav_force_pull(app: AppHandle) -> Result<SyncReport, String> {
    run_sync_command(app, SyncMode::ForcePull).await
}

/// 用本地覆盖云端：为远端独有的待办 / 子任务生成墓碑，条件 PUT 本地数据。
#[tauri::command]
pub async fn webdav_force_push(app: AppHandle) -> Result<SyncReport, String> {
    run_sync_command(app, SyncMode::ForcePush).await
}

async fn run_sync_command(app: AppHandle, mode: SyncMode) -> Result<SyncReport, String> {
    let _guard = SyncGuard::acquire()?;
    let worker = app.clone();
    // reqwest::blocking 不能在 tokio worker 上跑，网络与数据库都放进阻塞线程池（C1）
    let report = tauri::async_runtime::spawn_blocking(move || {
        let db = worker.state::<Database>();
        run_sync_blocking(&db, mode)
    })
    .await
    .map_err(|e| format!("同步任务异常: {}", e))??;

    if report.local_changed() {
        if let Err(e) = app.emit("sync-completed", &report) {
            log::warn!("[sync] 发送 sync-completed 事件失败: {}", e);
        }
    }
    Ok(report)
}

struct ConnectionConfig {
    url: String,
    username: String,
    password: String,
    device_id: String,
}

fn load_connection(db: &Database) -> Result<ConnectionConfig, String> {
    let (url, username, stored, device_id) = db
        .with_connection(|conn| {
            Ok((
                get_setting_or(conn, KEY_URL, ""),
                get_setting_or(conn, KEY_USERNAME, ""),
                get_setting_or(conn, KEY_PASSWORD, ""),
                ensure_device_id(conn)?,
            ))
        })
        .map_err(|e: rusqlite::Error| e.to_string())?;
    if url.trim().is_empty() {
        return Err("未配置 WebDAV 服务器".to_string());
    }
    Ok(ConnectionConfig {
        url,
        username,
        password: secret::reveal(&stored)?,
        device_id,
    })
}

fn run_sync_blocking(db: &Database, mode: SyncMode) -> Result<SyncReport, String> {
    let config = load_connection(db)?;
    let client = WebDavClient::new(&config.url, &config.username, &config.password)?;
    let engine = SyncEngine {
        db,
        client: &client,
        images_dir: paths::images_dir(),
        device_id: config.device_id,
        retry_delay: CONFLICT_RETRY_DELAY,
    };
    let report = match mode {
        SyncMode::Smart { full } => {
            let round = SMART_SYNC_ROUNDS
                .fetch_add(1, Ordering::Relaxed)
                .wrapping_add(1);
            engine.sync(wants_full_get(full, round))
        }
        SyncMode::ForcePull => engine.force_pull(),
        SyncMode::ForcePush => engine.force_push(),
    }?;
    if report.settings_applied {
        // 远端设置改写了 top_on_wake / auto_hide_enabled 等：刷新窗口模块的运行时缓存
        crate::commands::window::reload_runtime_prefs(db);
    }
    Ok(report)
}

// ============================================================================
// 同步引擎
// ============================================================================

/// 同步开始前的本地状态
struct PreState {
    local_seq: i64,
    synced_seq: i64,
    /// 本地设置版本（参与同步的设置键的 max(updated_at)）
    settings_version: String,
    /// 基准版本里的 settingsUpdatedAt
    synced_settings_at: String,
    base: RemoteVersion,
    /// 从未与当前远端成功同步过（新设备 / 换了服务器）：远端设置优先
    first_sync: bool,
}

impl PreState {
    fn dirty(&self) -> bool {
        self.local_seq != self.synced_seq || self.settings_version > self.synced_settings_at
    }
}

/// 上传前的本地快照（同一把锁内读取）
struct LocalState {
    records: LocalRecords,
    settings: AppSettings,
    settings_version: Option<String>,
}

/// 上传后补基准用的远端版本探测（HEAD / PROPFIND Depth 0）
type VersionProbe = fn(&WebDavClient, &str) -> Result<Option<PropMeta>, String>;

enum PushResult {
    Done(RemoteVersion),
    Conflict,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DocMode {
    /// 智能同步：本地 ∪ 远端（LWW + 墓碑）
    Merge,
    /// 强制推送：远端 = 本地
    ForcePush,
}

struct SyncEngine<'a> {
    db: &'a Database,
    client: &'a WebDavClient,
    images_dir: PathBuf,
    device_id: String,
    /// 412 之后的等待时间（生产环境为 [`CONFLICT_RETRY_DELAY`]）
    retry_delay: std::time::Duration,
}

impl SyncEngine<'_> {
    fn read_pre_state(&self) -> Result<PreState, String> {
        self.db
            .with_connection(|conn| {
                let non_empty = |key: &str| -> rusqlite::Result<Option<String>> {
                    Ok(get_setting(conn, key)?.filter(|s| !s.trim().is_empty()))
                };
                let synced_settings_at = get_setting_or(conn, KEY_SYNCED_SETTINGS_AT, "");
                let last_sync_at = get_setting_or(conn, KEY_LAST_SYNC_AT, "");
                Ok(PreState {
                    local_seq: sync_store::local_seq(conn)?,
                    synced_seq: sync_store::synced_seq(conn)?,
                    settings_version: settings_version(conn)?.unwrap_or_default(),
                    first_sync: last_sync_at.is_empty() && synced_settings_at.is_empty(),
                    synced_settings_at,
                    base: RemoteVersion {
                        etag: non_empty(KEY_REMOTE_ETAG)?,
                        last_modified: non_empty(KEY_LAST_MODIFIED)?,
                    },
                })
            })
            .map_err(|e| format!("读取同步状态失败: {}", e))
    }

    /// 单事务合并远端记录、墓碑与设置
    fn merge(
        &self,
        remote: &SyncData,
        first_sync: bool,
        cutoff: &str,
    ) -> Result<(MergeStats, bool), String> {
        let parsed = sync_store::parse_remote_todos(&remote.todos);
        self.db
            .with_transaction(|tx| -> rusqlite::Result<(MergeStats, bool)> {
                let stats = sync_store::merge_remote(tx, &parsed, &remote.tombstones, cutoff)?;
                let applied = apply_remote_settings_if_newer(tx, remote, first_sync)?;
                Ok((stats, applied))
            })
            .map_err(|e| format!("合并远端数据失败: {}", e))
    }

    fn snapshot(&self, cutoff: &str) -> Result<LocalState, String> {
        self.db
            .with_connection(|conn| {
                Ok(LocalState {
                    records: sync_store::local_records(conn, cutoff)?,
                    settings: read_app_settings(conn),
                    settings_version: settings_version(conn)?,
                })
            })
            .map_err(|e| format!("读取本地数据失败: {}", e))
    }

    /// 同步成功收尾：记基准、synced_seq、设置基准、同步时间，清理过期墓碑
    fn finish(
        &self,
        base: &RemoteVersion,
        synced_seq: i64,
        synced_settings_at: &str,
        cutoff: &str,
    ) -> Result<String, String> {
        let at = iso_now();
        self.db
            .with_transaction(|tx| -> rusqlite::Result<()> {
                set_setting(tx, KEY_REMOTE_ETAG, base.etag.as_deref().unwrap_or(""))?;
                set_setting(
                    tx,
                    KEY_LAST_MODIFIED,
                    base.last_modified.as_deref().unwrap_or(""),
                )?;
                set_setting(tx, KEY_SYNCED_SETTINGS_AT, synced_settings_at)?;
                set_setting(tx, KEY_LAST_SYNC_AT, &at)?;
                sync_store::set_synced_seq(tx, synced_seq)?;
                sync_store::purge_tombstones_before(tx, cutoff)?;
                Ok(())
            })
            .map_err(|e| format!("保存同步状态失败: {}", e))?;
        Ok(at)
    }

    /// gzip + 条件 PUT；父目录不存在时建目录重试一次；响应没带 ETag 时 HEAD / PROPFIND 补齐
    fn put_doc(&self, doc: &SyncData, precondition: &Precondition) -> Result<PushResult, String> {
        let json = serde_json::to_vec(doc).map_err(|e| format!("序列化同步数据失败: {}", e))?;
        let body = gzip_compress(&json)?;
        let len = body.len() as u64;

        let mut outcome = self.client.put(
            SYNC_DATA_FILE,
            body.clone(),
            "application/gzip",
            precondition,
        )?;
        if outcome == PutOutcome::ParentMissing {
            self.client.ensure_dir(REMOTE_DIR)?;
            outcome = self
                .client
                .put(SYNC_DATA_FILE, body, "application/gzip", precondition)?;
        }
        match outcome {
            PutOutcome::Ok(version) if version.etag.is_some() => Ok(PushResult::Done(version)),
            PutOutcome::Ok(version) => Ok(PushResult::Done(self.version_after_put(len, version))),
            PutOutcome::PreconditionFailed => Ok(PushResult::Conflict),
            PutOutcome::ParentMissing => Err("上传失败：无法创建远端目录 /mini-todo".to_string()),
        }
    }

    /// PUT 响应没带 ETag 时补基准：先 HEAD（nginx dav 只有 HEAD / GET 给 ETag，PROPFIND 没有
    /// getetag），HEAD 也没有 ETag 再 PROPFIND Depth 0（Apache 的 getetag 是强 ETag）。
    /// 大小对不上说明期间被别人写过，宁可不记基准（下次完整 GET 一次）也不记错。
    /// Last-Modified 一并记下，但只作记录，不用于条件请求。
    fn version_after_put(&self, uploaded_len: u64, from_put: RemoteVersion) -> RemoteVersion {
        let mut last_modified = from_put.last_modified;
        let probes: [(&str, VersionProbe); 2] = [
            ("HEAD", WebDavClient::head_meta),
            ("PROPFIND", WebDavClient::propfind_meta),
        ];
        for (name, probe) in probes {
            match probe(self.client, SYNC_DATA_FILE) {
                Ok(Some(meta)) => {
                    if meta.content_length.is_some_and(|len| len != uploaded_len) {
                        log::warn!("[sync] 上传后远端大小不符（期间被其它设备写入），本次不记基准");
                        return RemoteVersion::default();
                    }
                    if meta.version.last_modified.is_some() {
                        last_modified = meta.version.last_modified;
                    }
                    if meta.version.etag.is_some() {
                        return RemoteVersion {
                            etag: meta.version.etag,
                            last_modified,
                        };
                    }
                }
                Ok(None) => {
                    log::warn!("[sync] 上传后 {} 查不到远端文件，本次不记基准", name);
                    return RemoteVersion::default();
                }
                Err(e) => log::warn!("[sync] 上传后 {} 查询远端版本失败: {}", name, e),
            }
        }
        // 拿不到 ETag：下次同步做一次无条件 GET（Last-Modified 不参与条件请求）
        RemoteVersion {
            etag: None,
            last_modified,
        }
    }

    /// 412 之后等一会儿再重新 GET：Apache 刚写完的一秒内只给弱 ETag，立刻重试必然再 412。
    /// 最后一轮之后不等（马上报错）。
    fn wait_before_retry(&self, attempt: usize) {
        if attempt + 1 < MAX_ATTEMPTS {
            std::thread::sleep(self.retry_delay);
        }
    }

    /// 智能同步（K4）。`full`：第一次 GET 也不带条件（手动同步 / 兜底轮）
    fn sync(&self, full: bool) -> Result<SyncReport, String> {
        let mut report = SyncReport::default();
        let mut images = ImageTransfer::new(self.client, &self.images_dir);

        for attempt in 0..MAX_ATTEMPTS {
            let pre = self.read_pre_state()?;
            let dirty = pre.dirty();
            let cutoff = sync_store::retention_cutoff();

            // 本地有变更时反正要拿到远端全文来合并，直接无条件 GET；手动同步 / 兜底轮也无条件
            let conditional =
                (!full && !dirty && attempt == 0 && pre.base.etag.is_some()).then_some(&pre.base);
            let (remote, version) = match self.client.get(SYNC_DATA_FILE, conditional)? {
                GetOutcome::NotModified => {
                    let at =
                        self.finish(&pre.base, pre.local_seq, &pre.synced_settings_at, &cutoff)?;
                    report.finalize(false, at, &images);
                    return Ok(report);
                }
                GetOutcome::NotFound => (None, RemoteVersion::default()),
                GetOutcome::Found { body, version } => (Some(decode_sync_doc(&body)?), version),
            };

            if let Some(remote) = &remote {
                let (stats, settings_applied) = self.merge(remote, pre.first_sync, &cutoff)?;
                report.add_merge(&stats, settings_applied);
                images.download_missing(&remote.images);
            }

            let local = self.snapshot(&cutoff)?;
            let mut doc = build_sync_doc(
                &local,
                remote.as_ref(),
                DocMode::Merge,
                &self.device_id,
                &cutoff,
            );
            let need_upload = match &remote {
                None => true,
                Some(r) => dirty || doc_differs(&doc, r, &cutoff),
            };

            if !need_upload {
                let synced_settings_at = remote
                    .as_ref()
                    .and_then(|r| r.settings_updated_at.clone())
                    .unwrap_or_default();
                let at = self.finish(&version, local.records.seq, &synced_settings_at, &cutoff)?;
                report.finalize(false, at, &images);
                return Ok(report);
            }

            // K5：先传缺失图片，再传 sync-data
            images.upload_missing();
            doc.images = images.merged_list(remote.as_ref());

            let precondition = if remote.is_some() {
                choose_put_precondition(&version)
            } else {
                Precondition::None
            };
            match self.put_doc(&doc, &precondition)? {
                PushResult::Done(new_version) => {
                    let at = self.finish(
                        &new_version,
                        local.records.seq,
                        doc.settings_updated_at.as_deref().unwrap_or(""),
                        &cutoff,
                    )?;
                    report.finalize(true, at, &images);
                    return Ok(report);
                }
                PushResult::Conflict => {
                    log::warn!(
                        "[sync] 远端在合并期间被其它设备修改（412），重新合并（第 {} 次）",
                        attempt + 1
                    );
                    self.wait_before_retry(attempt);
                }
            }
        }
        Err(format!(
            "同步失败：远端在连续 {} 次尝试中都被其它设备同时修改，请稍后重试",
            MAX_ATTEMPTS
        ))
    }

    /// 强制拉取（K7）：本地 = 远端
    fn force_pull(&self) -> Result<SyncReport, String> {
        let mut report = SyncReport::default();
        let cutoff = sync_store::retention_cutoff();
        let (remote, version) = match self.client.get(SYNC_DATA_FILE, None)? {
            GetOutcome::Found { body, version } => (decode_sync_doc(&body)?, version),
            GetOutcome::NotFound => {
                return Err("远端还没有同步数据，无法用云端覆盖本地".to_string())
            }
            GetOutcome::NotModified => return Err("远端返回了意外的 304".to_string()),
        };

        let parsed = sync_store::parse_remote_todos(&remote.todos);
        let (stats, settings_applied, seq) = self
            .db
            .with_transaction(|tx| -> rusqlite::Result<(MergeStats, bool, i64)> {
                let stats =
                    sync_store::force_pull_replace(tx, &parsed, &remote.tombstones, &cutoff)?;
                // 没有 settingsUpdatedAt 的 settings（旧版写入方 / cloud 占位）不应用
                let applied = match (
                    remote.settings_updated_at.as_deref(),
                    remote.settings.as_object(),
                ) {
                    (Some(at), Some(obj)) => adopt_settings_json(tx, obj, at)?,
                    _ => false,
                };
                Ok((stats, applied, sync_store::local_seq(tx)?))
            })
            .map_err(|e| format!("用云端覆盖本地失败: {}", e))?;
        report.add_merge(&stats, settings_applied);

        let mut images = ImageTransfer::new(self.client, &self.images_dir);
        images.download_missing(&remote.images);

        let synced_settings_at = remote.settings_updated_at.clone().unwrap_or_default();
        let at = self.finish(&version, seq, &synced_settings_at, &cutoff)?;
        report.finalize(false, at, &images);
        Ok(report)
    }

    /// 强制推送（K7）：远端 = 本地
    fn force_push(&self) -> Result<SyncReport, String> {
        let mut report = SyncReport::default();
        let mut images = ImageTransfer::new(self.client, &self.images_dir);

        for attempt in 0..MAX_ATTEMPTS {
            let cutoff = sync_store::retention_cutoff();
            let (remote, version) = match self.client.get(SYNC_DATA_FILE, None)? {
                GetOutcome::Found { body, version } => match decode_sync_doc(&body) {
                    Ok(doc) => (Some(doc), version),
                    Err(e) => {
                        // 用户明确要求用本地覆盖：看不懂的远端也直接覆盖
                        log::warn!("[sync] 强制推送：远端数据无法识别，将被本地数据覆盖: {}", e);
                        (None, version)
                    }
                },
                GetOutcome::NotFound => (None, RemoteVersion::default()),
                GetOutcome::NotModified => return Err("远端返回了意外的 304".to_string()),
            };

            if let Some(r) = &remote {
                let now = now_local();
                self.db
                    .with_transaction(|tx| {
                        sync_store::force_push_prepare(tx, &r.todos, &r.tombstones, &now)
                    })
                    .map_err(|e: rusqlite::Error| format!("准备强制推送失败: {}", e))?;
            }

            let local = self.snapshot(&cutoff)?;
            let mut doc = build_sync_doc(
                &local,
                remote.as_ref(),
                DocMode::ForcePush,
                &self.device_id,
                &cutoff,
            );
            images.upload_missing();
            doc.images = images.merged_list(remote.as_ref());

            let precondition = choose_put_precondition(&version);
            match self.put_doc(&doc, &precondition)? {
                PushResult::Done(new_version) => {
                    let at = self.finish(
                        &new_version,
                        local.records.seq,
                        doc.settings_updated_at.as_deref().unwrap_or(""),
                        &cutoff,
                    )?;
                    report.finalize(true, at, &images);
                    return Ok(report);
                }
                PushResult::Conflict => {
                    log::warn!("[sync] 强制推送遇到 412，重试（第 {} 次）", attempt + 1);
                    self.wait_before_retry(attempt);
                }
            }
        }
        Err(format!(
            "强制推送失败：远端在连续 {} 次尝试中都被其它设备同时修改，请稍后重试",
            MAX_ATTEMPTS
        ))
    }
}

/// K3-5：远端 settingsUpdatedAt 存在且比本地设置版本新（或本机首次同步）时应用远端设置
fn apply_remote_settings_if_newer(
    conn: &rusqlite::Connection,
    remote: &SyncData,
    first_sync: bool,
) -> rusqlite::Result<bool> {
    let (Some(remote_at), Some(obj)) = (
        remote.settings_updated_at.as_deref(),
        remote.settings.as_object(),
    ) else {
        return Ok(false);
    };
    if first_sync {
        let changed = adopt_settings_json(conn, obj, remote_at)?;
        // 本机设置此刻就是远端这个版本：同一事务里记下设置基准，"首次同步"到此为止。
        // 否则这次同步若在上传前失败（412 用尽、网络中断），下一次仍按首次同步无条件采用远端
        // 设置，把用户在两次同步之间改过的设置悄悄覆盖掉
        set_setting(conn, KEY_SYNCED_SETTINGS_AT, remote_at)?;
        return Ok(changed);
    }
    let local_at = settings_version(conn)?.unwrap_or_default();
    if remote_at > local_at.as_str() {
        adopt_settings_json(conn, obj, remote_at)
    } else {
        Ok(false)
    }
}

// ============================================================================
// 上传文档拼装（JSON 层面的合并，与 cloud push 同一套规则）
// ============================================================================

fn set_subtasks(v: &mut Value, subtasks: Vec<Value>) {
    if let Some(obj) = v.as_object_mut() {
        obj.insert("subtasks".to_string(), Value::Array(subtasks));
    }
}

/// 子任务列表合并：按 id LWW（平局本地），墓碑压制，没有 id 的远端项原样保留
fn merge_subtask_values(local: Vec<Value>, remote: &[Value], tombs: &TombstoneIndex) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::with_capacity(local.len() + remote.len());
    let mut index: HashMap<i64, usize> = HashMap::new();
    for v in local {
        if let Some(id) = sync_store::value_id(&v) {
            if sync_store::is_suppressed(
                tombs,
                EntityKind::Subtask,
                id,
                &sync_store::value_updated_at(&v),
            ) {
                continue;
            }
            index.insert(id, out.len());
        }
        out.push(v);
    }
    for r in remote {
        let Some(id) = sync_store::value_id(r) else {
            out.push(r.clone());
            continue;
        };
        let remote_at = sync_store::value_updated_at(r);
        match index.get(&id).copied() {
            Some(i) => {
                if remote_at > sync_store::value_updated_at(&out[i]) {
                    out[i] = r.clone();
                }
            }
            None => {
                if sync_store::is_suppressed(tombs, EntityKind::Subtask, id, &remote_at) {
                    continue;
                }
                index.insert(id, out.len());
                out.push(r.clone());
            }
        }
    }
    out
}

/// 待办列表合并（K3）：按 id LWW（平局本地），墓碑压制，并集；子任务逐条合并。
/// 本地已合并过远端，所以远端胜出的只可能是本地无法识别 / 无法落库的记录——
/// 它们被原样带进上传文档，绝不因为"本地没有"而从远端消失。
fn merge_todo_values(local: Vec<Value>, remote: &[Value], tombs: &TombstoneIndex) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::with_capacity(local.len() + remote.len());
    let mut index: HashMap<i64, usize> = HashMap::new();
    for v in local {
        if let Some(id) = sync_store::value_id(&v) {
            if sync_store::is_suppressed(
                tombs,
                EntityKind::Todo,
                id,
                &sync_store::value_updated_at(&v),
            ) {
                continue;
            }
            index.insert(id, out.len());
        }
        out.push(v);
    }
    for r in remote {
        let Some(id) = sync_store::value_id(r) else {
            out.push(r.clone());
            continue;
        };
        let remote_at = sync_store::value_updated_at(r);
        match index.get(&id).copied() {
            Some(i) => {
                let local_subtasks = sync_store::value_subtasks(&out[i]).to_vec();
                let subtasks =
                    merge_subtask_values(local_subtasks, sync_store::value_subtasks(r), tombs);
                if remote_at > sync_store::value_updated_at(&out[i]) {
                    out[i] = r.clone();
                }
                set_subtasks(&mut out[i], subtasks);
            }
            None => {
                if sync_store::is_suppressed(tombs, EntityKind::Todo, id, &remote_at) {
                    continue;
                }
                let mut v = r.clone();
                let subtasks =
                    merge_subtask_values(Vec::new(), sync_store::value_subtasks(r), tombs);
                set_subtasks(&mut v, subtasks);
                index.insert(id, out.len());
                out.push(v);
            }
        }
    }
    dedupe_subtasks(&mut out);
    out
}

/// 同一个子任务 id 出现在多个待办下（旧版 cloud 允许改 parentId）时只保留 updatedAt
/// 最新的那一份（平局保留先出现的，即本地），避免上传文档里出现重复 id
fn dedupe_subtasks(todos: &mut [Value]) {
    // 子任务 id → (待办下标, 子任务下标, updatedAt)
    let mut winners: HashMap<i64, (usize, usize, String)> = HashMap::new();
    let mut duplicated = false;
    for (ti, todo) in todos.iter().enumerate() {
        for (si, sub) in sync_store::value_subtasks(todo).iter().enumerate() {
            let Some(id) = sync_store::value_id(sub) else {
                continue;
            };
            let at = sync_store::value_updated_at(sub);
            match winners.get(&id) {
                Some((_, _, best)) => {
                    duplicated = true;
                    if at > *best {
                        winners.insert(id, (ti, si, at));
                    }
                }
                None => {
                    winners.insert(id, (ti, si, at));
                }
            }
        }
    }
    if !duplicated {
        return;
    }
    for (ti, todo) in todos.iter_mut().enumerate() {
        let subtasks = sync_store::value_subtasks(todo);
        let kept: Vec<Value> = subtasks
            .iter()
            .enumerate()
            .filter(|(si, sub)| match sync_store::value_id(sub) {
                Some(id) => winners
                    .get(&id)
                    .is_some_and(|(wt, ws, _)| *wt == ti && ws == si),
                None => true,
            })
            .map(|(_, sub)| sub.clone())
            .collect();
        if kept.len() != subtasks.len() {
            set_subtasks(todo, kept);
        }
    }
}

/// 文档里每条记录的 (类型, id) → updatedAt（同键取最大）
fn record_versions(todos: &[Value]) -> BTreeMap<(EntityKind, i64), String> {
    let mut versions = BTreeMap::new();
    let mut put = |key: (EntityKind, i64), at: String| {
        let entry: &mut String = versions.entry(key).or_default();
        if at > *entry {
            *entry = at;
        }
    };
    for todo in todos {
        if let Some(id) = sync_store::value_id(todo) {
            put((EntityKind::Todo, id), sync_store::value_updated_at(todo));
        }
        for sub in sync_store::value_subtasks(todo) {
            if let Some(id) = sync_store::value_id(sub) {
                put((EntityKind::Subtask, id), sync_store::value_updated_at(sub));
            }
        }
    }
    versions
}

fn tombstone_set(tombstones: &[Tombstone], cutoff: &str) -> BTreeSet<(String, i64, String)> {
    tombstones
        .iter()
        .filter(|t| t.deleted_at.as_str() >= cutoff)
        .map(|t| (t.entity_type.clone(), t.entity_id, t.deleted_at.clone()))
        .collect()
}

/// 上传文档与远端是否不等价：记录版本、墓碑（保留期内）、设置版本任一不同即需上传
fn doc_differs(doc: &SyncData, remote: &SyncData, cutoff: &str) -> bool {
    record_versions(&doc.todos) != record_versions(&remote.todos)
        || tombstone_set(&doc.tombstones, cutoff) != tombstone_set(&remote.tombstones, cutoff)
        || doc.settings_updated_at != remote.settings_updated_at
}

/// 生成上传文档（K2）。`images` 由调用方在上传图片后填写。
fn build_sync_doc(
    local: &LocalState,
    remote: Option<&SyncData>,
    mode: DocMode,
    device_id: &str,
    cutoff: &str,
) -> SyncData {
    // 墓碑：本地 ∪ 远端，同键取较大时间，按保留期过滤
    let mut tombs: BTreeMap<(EntityKind, i64), String> = BTreeMap::new();
    let all_tombstones = local
        .records
        .tombstones
        .iter()
        .chain(remote.map(|r| r.tombstones.as_slice()).unwrap_or(&[]));
    for t in all_tombstones {
        let Some(kind) = EntityKind::parse(&t.entity_type) else {
            continue;
        };
        if t.deleted_at.as_str() < cutoff {
            continue;
        }
        let entry = tombs.entry((kind, t.entity_id)).or_default();
        if t.deleted_at > *entry {
            *entry = t.deleted_at.clone();
        }
    }
    let index: TombstoneIndex = tombs.iter().map(|(k, v)| (*k, v.clone())).collect();

    let local_values: Vec<Value> = local
        .records
        .todos
        .iter()
        .filter_map(|t| serde_json::to_value(t).ok())
        .collect();
    let todos = match (mode, remote) {
        (DocMode::Merge, Some(r)) => merge_todo_values(local_values, &r.todos, &index),
        _ => local_values,
    };

    // 不保留会压制输出记录的墓碑（强制推送时本地版本必须胜出）
    let versions = record_versions(&todos);
    tombs.retain(|key, deleted_at| {
        versions
            .get(key)
            .is_none_or(|updated_at| deleted_at.as_str() < updated_at.as_str())
    });

    let local_settings_at = local.settings_version.clone().unwrap_or_else(now_local);
    let local_settings = serde_json::to_value(&local.settings).unwrap_or(Value::Null);
    let (settings, settings_updated_at) = match (mode, remote) {
        (DocMode::ForcePush, _) => {
            // 强制推送：让其它设备也采用本机设置
            let now = now_local();
            let at = if local_settings_at > now {
                local_settings_at
            } else {
                now
            };
            (local_settings, Some(at))
        }
        (DocMode::Merge, Some(r))
            if r.settings.is_object()
                && r.settings_updated_at
                    .as_deref()
                    .is_some_and(|remote_at| remote_at >= local_settings_at.as_str()) =>
        {
            // 远端设置不旧于本地：原样透传
            (r.settings.clone(), r.settings_updated_at.clone())
        }
        _ => (local_settings, Some(local_settings_at)),
    };

    SyncData {
        version: EXPORT_VERSION.to_string(),
        device_id: device_id.to_string(),
        updated_at: iso_now(),
        todos,
        settings,
        settings_updated_at,
        images: Vec::new(),
        tombstones: tombs
            .into_iter()
            .map(|((kind, id), deleted_at)| Tombstone {
                entity_type: kind.as_str().to_string(),
                entity_id: id,
                deleted_at,
            })
            .collect(),
        extra: remote.map(|r| r.extra.clone()).unwrap_or_default(),
    }
}

// ============================================================================
// 图片（K5）
// ============================================================================

fn remote_image_path(name: &str) -> String {
    format!("{}/{}", REMOTE_IMAGES_DIR, name)
}

/// 一次同步里的图片传输状态
struct ImageTransfer<'a> {
    client: &'a WebDavClient,
    dir: &'a Path,
    /// 已确认存在于远端的图片名（PROPFIND 列表 + 本次上传成功的）
    remote_known: BTreeSet<String>,
    upload_done: bool,
    tried_download: HashSet<String>,
    uploaded: u32,
    downloaded: u32,
}

impl<'a> ImageTransfer<'a> {
    fn new(client: &'a WebDavClient, dir: &'a Path) -> Self {
        Self {
            client,
            dir,
            remote_known: BTreeSet::new(),
            upload_done: false,
            tried_download: HashSet::new(),
            uploaded: 0,
            downloaded: 0,
        }
    }

    /// 本地图片目录里名字安全的文件
    fn local_names(&self) -> Vec<String> {
        let Ok(entries) = std::fs::read_dir(self.dir) else {
            return Vec::new();
        };
        let mut names: Vec<String> = entries
            .flatten()
            .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
            .filter_map(|e| e.file_name().to_str().map(str::to_string))
            .filter(|name| {
                let ok = is_safe_image_name(name);
                if !ok && !name.starts_with('.') {
                    log::warn!("[sync] 跳过文件名不安全的本地图片: {:?}", name);
                }
                ok
            })
            .collect();
        names.sort();
        names
    }

    /// 下载远端列出、本地缺失的图片；失败只记日志
    fn download_missing(&mut self, names: &[String]) {
        for name in names {
            if !is_safe_image_name(name) {
                log::warn!("[sync] 跳过文件名不安全的远端图片: {:?}", name);
                continue;
            }
            if !self.tried_download.insert(name.clone()) {
                continue;
            }
            let local = self.dir.join(name);
            if local.exists() {
                continue;
            }
            match self.client.download_file(&remote_image_path(name), &local) {
                Ok(true) => self.downloaded += 1,
                Ok(false) => log::warn!("[sync] 远端缺少图片 {}", name),
                Err(e) => log::warn!("[sync] 下载图片 {} 失败: {}", name, e),
            }
        }
    }

    /// 上传远端缺失的本地图片（每次同步只做一次）。远端清单用一次 PROPFIND Depth 1，
    /// 失败时退化为逐个 HEAD；单张失败只记日志，不阻断同步。
    fn upload_missing(&mut self) {
        if self.upload_done {
            return;
        }
        self.upload_done = true;

        let names = self.local_names();
        if names.is_empty() {
            return;
        }
        let listing: Option<HashSet<String>> = match self.client.list_names(REMOTE_IMAGES_DIR) {
            Ok(list) => Some(list.into_iter().collect()),
            Err(e) => {
                log::warn!("[sync] 列出远端图片失败，改为逐个检查: {}", e);
                None
            }
        };
        if let Some(list) = &listing {
            self.remote_known
                .extend(list.iter().filter(|n| is_safe_image_name(n)).cloned());
        }

        for name in names {
            let present = match &listing {
                Some(list) => list.contains(&name),
                None => match self.client.exists(&remote_image_path(&name)) {
                    Ok(present) => present,
                    Err(e) => {
                        log::warn!("[sync] 检查远端图片 {} 失败，本次跳过: {}", name, e);
                        continue;
                    }
                },
            };
            if !present {
                if let Err(e) = self.client.upload_file(
                    &remote_image_path(&name),
                    &self.dir.join(&name),
                    REMOTE_IMAGES_DIR,
                ) {
                    log::warn!("[sync] 上传图片 {} 失败: {}", name, e);
                    continue;
                }
                self.uploaded += 1;
            }
            self.remote_known.insert(name);
        }
    }

    /// 上传文档里的图片清单：远端原清单 ∪ 已知存在于远端的图片（只保留安全文件名）
    fn merged_list(&self, remote: Option<&SyncData>) -> Vec<String> {
        let mut names = self.remote_known.clone();
        if let Some(r) = remote {
            names.extend(r.images.iter().filter(|n| is_safe_image_name(n)).cloned());
        }
        names.into_iter().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::sync_store::insert_todo_row;
    use crate::db::Todo;
    use crate::services::webdav::opaque_etag;
    use serde_json::json;

    // ------------------------------------------------------------------
    // 进程内 mock WebDAV（tiny_http）
    // ------------------------------------------------------------------

    mod mock {
        use std::collections::{BTreeSet, HashMap};
        use std::io::Cursor;
        use std::sync::{Arc, Mutex};
        use std::thread::JoinHandle;
        use std::time::{Duration, Instant};
        use tiny_http::{Header, Request, Response, Server};

        #[derive(Clone)]
        pub struct MockFile {
            pub body: Vec<u8>,
            /// 强 ETag（带引号的 opaque-tag）；弱窗口内对外显示为 `W/` + 它
            pub etag: String,
            pub last_modified: String,
            pub written_at: Instant,
        }

        /// 服务端行为。默认是"理想"服务端：强 ETag、PUT 响应带校验器、条件头全检查。
        #[derive(Default)]
        pub struct State {
            pub files: HashMap<String, MockFile>,
            pub dirs: BTreeSet<String>,
            pub log: Vec<String>,
            counter: u64,
            /// 下一次 PUT sync-data 之前，模拟另一台设备抢先写入的内容
            pub concurrent_write: Option<Vec<u8>>,
            /// PUT 响应不带 ETag / Last-Modified（Apache、nginx 都如此）
            pub omit_put_headers: bool,
            /// Apache：文件写入后这段时间内 GET / HEAD 只给弱 ETag，`If-Match`（强比较）必然 412；
            /// PROPFIND 的 getetag 始终是强 ETag
            pub weak_window: Option<Duration>,
            /// Apache：`If-Unmodified-Since` 拿亚秒 mtime 与秒级日期比较，一律 412
            pub ius_always_fails: bool,
            /// nginx：PUT 不检查 `If-Match` / `If-Unmodified-Since`
            pub ignore_put_preconditions: bool,
            /// nginx：PROPFIND 不返回 getetag
            pub propfind_without_etag: bool,
            /// HEAD 不返回 ETag（用来测 PROPFIND 兜底）
            pub head_without_etag: bool,
            /// nginx：文件 mtime 只到秒，取这个测试控制的"当前秒"；
            /// ETag = `"<mtime 十六进制>-<大小十六进制>"`，`If-Modified-Since` 按整秒比较
            pub nginx_second: Option<u64>,
            /// 每次 PUT sync-data 收到的 (If-Match, If-Unmodified-Since)
            pub put_preconditions: Vec<(Option<String>, Option<String>)>,
            /// 每次 GET sync-data 收到的 (If-None-Match, If-Modified-Since)
            pub get_conditions: Vec<(Option<String>, Option<String>)>,
        }

        fn http_date(secs: u64) -> String {
            format!(
                "Thu, 01 Jan 2026 {:02}:{:02}:{:02} GMT",
                secs / 3600 % 24,
                secs / 60 % 60,
                secs % 60
            )
        }

        fn opaque(etag: &str) -> &str {
            etag.trim().trim_start_matches("W/")
        }

        impl State {
            pub fn write(&mut self, path: &str, body: Vec<u8>) {
                self.counter += 1;
                let n = self.counter;
                let (etag, last_modified) = match self.nginx_second {
                    Some(sec) => (format!("\"{:x}-{:x}\"", sec, body.len()), http_date(sec)),
                    None => (format!("\"v{}\"", n), http_date(n)),
                };
                self.files.insert(
                    path.to_string(),
                    MockFile {
                        body,
                        etag,
                        last_modified,
                        written_at: Instant::now(),
                    },
                );
            }

            fn in_weak_window(&self, f: &MockFile) -> bool {
                self.weak_window.is_some_and(|w| f.written_at.elapsed() < w)
            }

            /// GET / HEAD 对外给出的 ETag
            fn served_etag(&self, f: &MockFile) -> String {
                if self.in_weak_window(f) {
                    format!("W/{}", f.etag)
                } else {
                    f.etag.clone()
                }
            }
        }

        pub struct MockDav {
            server: Arc<Server>,
            handle: Option<JoinHandle<()>>,
            pub state: Arc<Mutex<State>>,
            pub url: String,
        }

        impl MockDav {
            pub fn start() -> Self {
                let server = Arc::new(Server::http("127.0.0.1:0").expect("启动 mock 服务失败"));
                let addr = server.server_addr().to_ip().expect("mock 服务地址");
                let state = Arc::new(Mutex::new(State::default()));
                let (srv, st) = (server.clone(), state.clone());
                let handle = std::thread::spawn(move || {
                    for mut req in srv.incoming_requests() {
                        let resp = handle(&st, &mut req);
                        let _ = req.respond(resp);
                    }
                });
                MockDav {
                    server,
                    handle: Some(handle),
                    state,
                    url: format!("http://{}/dav", addr),
                }
            }
        }

        impl Drop for MockDav {
            fn drop(&mut self) {
                self.server.unblock();
                if let Some(h) = self.handle.take() {
                    let _ = h.join();
                }
            }
        }

        /// 总是带 Content-Length（HEAD 也按文件真实大小给出，tiny_http 不发 HEAD 的响应体）
        fn respond(
            code: u16,
            body: Vec<u8>,
            headers: &[(&str, &str)],
        ) -> Response<Cursor<Vec<u8>>> {
            let mut resp = Response::from_data(body)
                .with_status_code(code)
                .with_chunked_threshold(usize::MAX);
            for (k, v) in headers {
                resp = resp.with_header(Header::from_bytes(k.as_bytes(), v.as_bytes()).unwrap());
            }
            resp
        }

        fn handle(state: &Mutex<State>, req: &mut Request) -> Response<Cursor<Vec<u8>>> {
            let method = req.method().as_str().to_string();
            let raw = req.url().split('?').next().unwrap_or("").to_string();
            let path = raw.strip_prefix("/dav").unwrap_or(&raw).to_string();
            let header = |name: &'static str| {
                req.headers()
                    .iter()
                    .find(|h| h.field.equiv(name))
                    .map(|h| h.value.as_str().to_string())
            };
            let if_match = header("If-Match");
            let if_none_match = header("If-None-Match");
            let if_unmodified_since = header("If-Unmodified-Since");
            let if_modified_since = header("If-Modified-Since");
            let depth = header("Depth");
            let mut body = Vec::new();
            req.as_reader().read_to_end(&mut body).unwrap();

            let mut st = state.lock().unwrap();
            st.log.push(format!("{} {}", method, path));
            let key = path.trim_end_matches('/').to_string();

            match method.as_str() {
                "GET" | "HEAD" => {
                    if method == "GET" && key.ends_with("sync-data.json.gz") {
                        st.get_conditions
                            .push((if_none_match.clone(), if_modified_since.clone()));
                    }
                    let Some(f) = st.files.get(&key).cloned() else {
                        return respond(404, Vec::new(), &[]);
                    };
                    // If-None-Match 是弱比较；If-Modified-Since 按整秒（nginx 的毛病：同一秒内的写入被判成未修改）
                    let not_modified = match (&if_none_match, &if_modified_since) {
                        (Some(inm), _) => opaque(inm) == opaque(&f.etag),
                        (None, Some(ims)) => *ims == f.last_modified,
                        (None, None) => false,
                    };
                    if not_modified {
                        return respond(304, Vec::new(), &[]);
                    }
                    let etag = st.served_etag(&f);
                    let mut headers = vec![("Last-Modified", f.last_modified.as_str())];
                    if !(method == "HEAD" && st.head_without_etag) {
                        headers.push(("ETag", etag.as_str()));
                    }
                    respond(200, f.body.clone(), &headers)
                }
                "PUT" => {
                    if key.ends_with("sync-data.json.gz") {
                        st.put_preconditions
                            .push((if_match.clone(), if_unmodified_since.clone()));
                        if let Some(other) = st.concurrent_write.take() {
                            st.write(&key, other);
                        }
                    }
                    let parent = key
                        .rsplit_once('/')
                        .map(|(p, _)| p.to_string())
                        .unwrap_or_default();
                    if !st.dirs.contains(&parent) {
                        return respond(409, Vec::new(), &[]);
                    }
                    let current = st.files.get(&key).cloned();
                    if !st.ignore_put_preconditions {
                        if let Some(expected) = &if_match {
                            // If-Match 是强比较：弱 ETag（含弱窗口内的文件）永远不匹配
                            let strong_match = current
                                .as_ref()
                                .is_some_and(|f| !st.in_weak_window(f) && f.etag == *expected);
                            if !strong_match {
                                return respond(412, Vec::new(), &[]);
                            }
                        }
                        if let Some(expected) = &if_unmodified_since {
                            if st.ius_always_fails
                                || current.as_ref().map(|f| &f.last_modified) != Some(expected)
                            {
                                return respond(412, Vec::new(), &[]);
                            }
                        }
                    }
                    st.write(&key, body);
                    if st.omit_put_headers {
                        respond(201, Vec::new(), &[])
                    } else {
                        let f = st.files[&key].clone();
                        respond(
                            201,
                            Vec::new(),
                            &[("ETag", &f.etag), ("Last-Modified", &f.last_modified)],
                        )
                    }
                }
                "MKCOL" => {
                    if st.dirs.contains(&key) {
                        respond(405, Vec::new(), &[])
                    } else {
                        st.dirs.insert(key);
                        respond(201, Vec::new(), &[])
                    }
                }
                "PROPFIND" if depth.as_deref() == Some("0") => match st.files.get(&key) {
                    None => respond(404, Vec::new(), &[]),
                    Some(f) => {
                        let getetag = if st.propfind_without_etag {
                            String::new()
                        } else {
                            format!("<D:getetag>{}</D:getetag>", f.etag.replace('"', "&quot;"))
                        };
                        let xml = format!(
                            r#"<?xml version="1.0"?><D:multistatus xmlns:D="DAV:"><D:response><D:href>/dav{}</D:href><D:propstat><D:prop>{}<D:getlastmodified>{}</D:getlastmodified><D:getcontentlength>{}</D:getcontentlength></D:prop></D:propstat></D:response></D:multistatus>"#,
                            key,
                            getetag,
                            f.last_modified,
                            f.body.len()
                        );
                        respond(
                            207,
                            xml.into_bytes(),
                            &[("Content-Type", "application/xml")],
                        )
                    }
                },
                "PROPFIND" => {
                    if !st.dirs.contains(&key) {
                        return respond(404, Vec::new(), &[]);
                    }
                    let mut xml = format!(
                        r#"<?xml version="1.0"?><D:multistatus xmlns:D="DAV:"><D:response><D:href>/dav{}/</D:href><D:propstat><D:prop><D:resourcetype><D:collection/></D:resourcetype></D:prop></D:propstat></D:response>"#,
                        key
                    );
                    let mut children: Vec<&String> = st
                        .files
                        .keys()
                        .filter(|p| p.rsplit_once('/').map(|(d, _)| d) == Some(key.as_str()))
                        .collect();
                    children.sort();
                    for p in children {
                        xml.push_str(&format!(
                            "\n<D:response><D:href>/dav{}</D:href><D:propstat><D:prop><D:resourcetype/></D:prop></D:propstat></D:response>",
                            p
                        ));
                    }
                    xml.push_str("</D:multistatus>");
                    respond(
                        207,
                        xml.into_bytes(),
                        &[("Content-Type", "application/xml")],
                    )
                }
                _ => respond(405, Vec::new(), &[]),
            }
        }
    }

    use mock::MockDav;

    struct Fixture {
        mock: MockDav,
        db: Database,
        client: WebDavClient,
        dir: PathBuf,
        /// 默认用生产值，Apache 弱 ETag 窗口的测试依赖它真的 ≥1.1s
        retry_delay: std::time::Duration,
    }

    impl Fixture {
        fn new(tag: &str) -> Self {
            let mock = MockDav::start();
            let client = WebDavClient::new(&mock.url, "user", "pass").unwrap();
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let dir = std::env::temp_dir().join(format!(
                "mini-todo-sync-test-{}-{}-{}",
                tag,
                std::process::id(),
                nanos
            ));
            std::fs::create_dir_all(&dir).unwrap();
            Fixture {
                mock,
                db: Database::new_in_memory().unwrap(),
                client,
                dir,
                retry_delay: CONFLICT_RETRY_DELAY,
            }
        }

        fn engine(&self) -> SyncEngine<'_> {
            SyncEngine {
                db: &self.db,
                client: &self.client,
                images_dir: self.dir.clone(),
                device_id: "test-pc".to_string(),
                retry_delay: self.retry_delay,
            }
        }

        /// 服务端行为开关
        fn server(&self) -> std::sync::MutexGuard<'_, mock::State> {
            self.mock.state.lock().unwrap()
        }

        /// 模拟另一台设备直接写远端（不经过本机）
        fn other_device_writes(&self, doc: &Value) {
            let mut st = self.server();
            st.dirs.insert(REMOTE_DIR.to_string());
            st.write(
                SYNC_DATA_FILE,
                gzip_compress(&serde_json::to_vec(doc).unwrap()).unwrap(),
            );
        }

        /// 客户端是否发过基于时间的条件头（If-Modified-Since / If-Unmodified-Since）
        fn sent_time_conditions(&self) -> bool {
            let st = self.server();
            st.get_conditions.iter().any(|(_, ims)| ims.is_some())
                || st.put_preconditions.iter().any(|(_, ius)| ius.is_some())
        }

        fn remote(&self) -> Value {
            let st = self.mock.state.lock().unwrap();
            let file = &st.files[SYNC_DATA_FILE];
            serde_json::from_slice(&gzip_decompress(&file.body).unwrap()).unwrap()
        }

        fn set_remote(&self, doc: &Value) {
            let mut st = self.mock.state.lock().unwrap();
            st.dirs.insert(REMOTE_DIR.to_string());
            st.write(
                SYNC_DATA_FILE,
                gzip_compress(&serde_json::to_vec(doc).unwrap()).unwrap(),
            );
        }

        fn log(&self) -> Vec<String> {
            self.mock.state.lock().unwrap().log.clone()
        }

        fn clear_log(&self) {
            self.mock.state.lock().unwrap().log.clear();
        }

        fn count(&self, line: &str) -> usize {
            self.log().iter().filter(|l| l.as_str() == line).count()
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    const PUT_DOC: &str = "PUT /mini-todo/sync-data.json.gz";

    fn todo_json(id: i64, title: &str, updated_at: &str) -> Value {
        json!({
            "id": id, "title": title, "description": null, "color": "#EF4444", "quadrant": 1,
            "notifyAt": null, "notifyBefore": 0, "notified": false, "completed": false,
            "sortOrder": id, "startTime": null, "endTime": null,
            "createdAt": "2026-01-01 00:00:00", "updatedAt": updated_at, "subtasks": []
        })
    }

    fn doc(todos: Vec<Value>) -> Value {
        json!({
            "version": "4.0", "deviceId": "other-device", "updatedAt": "2026-10-01T00:00:00+08:00",
            "todos": todos, "settings": null, "images": [], "tombstones": []
        })
    }

    fn add_local_todo(db: &Database, id: i64, title: &str, updated_at: &str) {
        let todo: Todo = serde_json::from_value(todo_json(id, title, updated_at)).unwrap();
        db.with_transaction(|tx| insert_todo_row(tx, &todo))
            .unwrap();
    }

    fn local_titles(db: &Database) -> Vec<(i64, String)> {
        db.with_connection(|c| {
            let mut stmt = c.prepare("SELECT id, title FROM todos ORDER BY id")?;
            let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
            rows.collect()
        })
        .unwrap()
    }

    /// gzip 压缩 `data`，再用头部的 FNAME 字段（名字 + 结尾 NUL）补到恰好 `size` 字节；
    /// 压缩流不变、解压结果不变。压缩后已超过 `size` 时返回 None
    fn gzip_padded_to(data: &[u8], size: usize) -> Option<Vec<u8>> {
        let plain = gzip_compress(data).unwrap();
        let pad = size.checked_sub(plain.len())?;
        if pad == 0 {
            return Some(plain);
        }
        let mut encoder = flate2::GzBuilder::new()
            .filename(vec![b'x'; pad - 1])
            .write(Vec::new(), Compression::default());
        encoder.write_all(data).unwrap();
        let padded = encoder.finish().unwrap();
        assert_eq!(padded.len(), size);
        Some(padded)
    }

    fn remote_ids(doc: &Value) -> Vec<i64> {
        let mut ids: Vec<i64> = doc["todos"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|t| t["id"].as_i64())
            .collect();
        ids.sort();
        ids
    }

    fn setting(db: &Database, key: &str) -> String {
        db.with_connection(|c| Ok(get_setting_or(c, key, "")))
            .unwrap()
    }

    // ------------------------------------------------------------------
    // 端到端：智能同步
    // ------------------------------------------------------------------

    #[test]
    fn sync_pushes_to_empty_remote_then_short_circuits_with_304() {
        let fx = Fixture::new("push-empty");
        add_local_todo(&fx.db, 1, "本地", "2026-09-01 10:00:00");

        let report = fx.engine().sync(false).expect("首次同步");
        assert_eq!(report.status, SyncStatus::Pushed);
        assert!(!report.last_sync_at.is_empty());
        let remote = fx.remote();
        assert_eq!(remote_ids(&remote), vec![1]);
        assert_eq!(remote["deviceId"], "test-pc");
        assert_eq!(remote["tombstones"], json!([]));
        assert!(remote["settingsUpdatedAt"].is_string());
        assert!(
            fx.log().contains(&"MKCOL /mini-todo/".to_string()),
            "父目录按需创建"
        );
        assert!(
            !setting(&fx.db, KEY_REMOTE_ETAG).is_empty(),
            "记下 PUT 返回的基准"
        );

        fx.clear_log();
        let again = fx.engine().sync(false).expect("二次同步");
        assert_eq!(again.status, SyncStatus::NoChanges);
        assert_eq!(
            fx.log(),
            vec!["GET /mini-todo/sync-data.json.gz".to_string()]
        );
    }

    #[test]
    fn sync_merges_both_ways_and_preserves_unknown_data() {
        let fx = Fixture::new("merge");
        let mut unreadable = todo_json(3, "看不懂", "2026-09-05 10:00:00");
        unreadable["quadrant"] = json!("urgent_important");
        let mut remote = doc(vec![
            todo_json(2, "远端", "2026-09-02 10:00:00"),
            unreadable,
        ]);
        remote["futureKey"] = json!({"nested": [1, 2]});
        remote["settings"] = json!({"textTheme": "light", "windowPosition": {"x": 9, "y": 9}});
        remote["settingsUpdatedAt"] = json!("2026-09-03T00:00:00");
        fx.set_remote(&remote);
        add_local_todo(&fx.db, 1, "本地", "2026-09-01 10:00:00");

        let report = fx.engine().sync(false).expect("同步");
        assert_eq!(report.status, SyncStatus::Merged);
        assert_eq!(report.todos_inserted, 1);
        assert_eq!(report.records_skipped, 1);
        assert!(report.settings_applied, "首次同步采用远端设置");
        assert_eq!(
            local_titles(&fx.db),
            vec![(1, "本地".to_string()), (2, "远端".to_string())]
        );
        assert_eq!(setting(&fx.db, "text_theme"), "light");
        assert_eq!(
            setting(&fx.db, "window_position"),
            "",
            "窗口位置永不从远端应用"
        );

        let after = fx.remote();
        assert_eq!(remote_ids(&after), vec![1, 2, 3], "看不懂的记录原样保留");
        let carried = after["todos"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["id"] == 3)
            .unwrap();
        assert_eq!(carried["quadrant"], "urgent_important");
        assert_eq!(
            after["futureKey"],
            json!({"nested": [1, 2]}),
            "未知顶层键原样保留"
        );
        assert_eq!(after["settings"]["textTheme"], "light");

        fx.clear_log();
        assert_eq!(
            fx.engine().sync(false).unwrap().status,
            SyncStatus::NoChanges
        );
        assert_eq!(fx.count(PUT_DOC), 0);
    }

    #[test]
    fn sync_retries_after_precondition_failure_and_merges_concurrent_write() {
        let fx = Fixture::new("412");
        fx.set_remote(&doc(vec![todo_json(2, "远端", "2026-09-02 10:00:00")]));
        add_local_todo(&fx.db, 1, "本地", "2026-09-01 10:00:00");
        fx.engine().sync(false).expect("建立基准");

        add_local_todo(&fx.db, 5, "本地新增", "2026-09-06 10:00:00");
        let mut concurrent = fx.remote();
        concurrent["todos"].as_array_mut().unwrap().push(todo_json(
            9,
            "并发写入",
            "2026-09-06 11:00:00",
        ));
        fx.mock.state.lock().unwrap().concurrent_write =
            Some(gzip_compress(&serde_json::to_vec(&concurrent).unwrap()).unwrap());

        fx.clear_log();
        let started = std::time::Instant::now();
        let report = fx.engine().sync(false).expect("412 后重试应成功");
        assert!(
            started.elapsed() >= CONFLICT_RETRY_DELAY,
            "412 之后要等过 Apache 的弱 ETag 窗口再重试"
        );
        assert_eq!(report.status, SyncStatus::Merged);
        assert_eq!(report.todos_inserted, 1);
        assert_eq!(fx.count(PUT_DOC), 2, "第一次 412，第二次成功");
        assert_eq!(remote_ids(&fx.remote()), vec![1, 2, 5, 9]);
        assert!(local_titles(&fx.db).iter().any(|(id, _)| *id == 9));
    }

    #[test]
    fn sync_gives_up_after_repeated_conflicts() {
        let mut fx = Fixture::new("412-loop");
        fx.retry_delay = std::time::Duration::from_millis(10);
        fx.set_remote(&doc(vec![]));
        add_local_todo(&fx.db, 1, "本地", "2026-09-01 10:00:00");
        let blob = gzip_compress(&serde_json::to_vec(&doc(vec![])).unwrap()).unwrap();

        // 每次 PUT 前都有别人抢先写入
        let state = fx.mock.state.clone();
        let stop = std::sync::Arc::new(AtomicBool::new(false));
        let stop2 = stop.clone();
        let feeder = std::thread::spawn(move || {
            while !stop2.load(Ordering::SeqCst) {
                {
                    let mut st = state.lock().unwrap();
                    if st.concurrent_write.is_none() {
                        st.concurrent_write = Some(blob.clone());
                    }
                }
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        });
        let err = fx.engine().sync(false).expect_err("连续冲突应报错");
        stop.store(true, Ordering::SeqCst);
        feeder.join().unwrap();
        assert!(err.contains("请稍后重试"), "{err}");
        assert_eq!(fx.count(PUT_DOC), MAX_ATTEMPTS);
    }

    #[test]
    fn sync_refuses_to_overwrite_unreadable_remote() {
        for body in [
            gzip_compress(b"{not json").unwrap(),
            gzip_compress(br#"{"todos": {"id": 1}}"#).unwrap(),
            gzip_compress(b"[1, 2, 3]").unwrap(),
            b"\x1f\x8b garbage".to_vec(),
        ] {
            let fx = Fixture::new("unreadable");
            {
                let mut st = fx.mock.state.lock().unwrap();
                st.dirs.insert(REMOTE_DIR.to_string());
                st.write(SYNC_DATA_FILE, body);
            }
            add_local_todo(&fx.db, 1, "本地", "2026-09-01 10:00:00");
            let err = fx.engine().sync(false).expect_err("看不懂的远端应报错");
            assert!(
                err.contains("解析远程数据失败") || err.contains("解压失败"),
                "{err}"
            );
            assert_eq!(fx.count(PUT_DOC), 0, "绝不上传覆盖");
            assert_eq!(setting(&fx.db, KEY_LAST_SYNC_AT), "");
        }
    }

    #[test]
    fn retry_policy_outlasts_apache_weak_etag_window() {
        const {
            assert!(CONFLICT_RETRY_DELAY.as_millis() >= 1100);
            assert!(MAX_ATTEMPTS >= 4);
        }
    }

    /// PUT 响应不带校验器：先 HEAD 取 ETag（nginx 只有 HEAD 给），HEAD 也没有再 PROPFIND
    #[test]
    fn put_without_version_headers_uses_head_then_propfind() {
        let fx = Fixture::new("put-no-validators");
        fx.server().omit_put_headers = true;
        add_local_todo(&fx.db, 1, "本地", "2026-09-01 10:00:00");

        fx.engine().sync(false).unwrap();
        assert_eq!(fx.count("HEAD /mini-todo/sync-data.json.gz"), 1);
        assert_eq!(
            fx.count("PROPFIND /mini-todo/sync-data.json.gz"),
            0,
            "HEAD 已给出 ETag，不必再 PROPFIND"
        );
        let current_etag = fx.server().files[SYNC_DATA_FILE].etag.clone();
        assert_eq!(setting(&fx.db, KEY_REMOTE_ETAG), current_etag);
        assert!(!setting(&fx.db, KEY_LAST_MODIFIED).is_empty());

        fx.clear_log();
        assert_eq!(
            fx.engine().sync(false).unwrap().status,
            SyncStatus::NoChanges
        );
        assert_eq!(fx.count(PUT_DOC), 0);

        // HEAD 也不给 ETag → PROPFIND getetag 兜底
        fx.server().head_without_etag = true;
        add_local_todo(&fx.db, 2, "再改", "2026-09-02 10:00:00");
        fx.clear_log();
        fx.engine().sync(false).unwrap();
        assert_eq!(fx.count("HEAD /mini-todo/sync-data.json.gz"), 1);
        assert_eq!(fx.count("PROPFIND /mini-todo/sync-data.json.gz"), 1);
        let current_etag = fx.server().files[SYNC_DATA_FILE].etag.clone();
        assert_eq!(setting(&fx.db, KEY_REMOTE_ETAG), current_etag);
        assert!(!fx.sent_time_conditions());
    }

    /// Apache mod_dav：If-Unmodified-Since 对没动过的文件也 412；刚写完的一秒内只给弱 ETag，
    /// 期间 If-Match 也 412；PUT 响应不带校验器；PROPFIND 给强 ETag。
    /// 实测故障：另一个写入方不到 1 秒前刚改过远端时，同步 3 次 412 后报错。
    #[test]
    fn apache_like_server_recovers_from_a_write_in_the_last_second() {
        // 弱 ETag 窗口与重试间隔按同一比例放大（真实值 1s / 1.1s，见上一个测试），
        // 留足余量，测试机繁忙时也不会因为调度延迟而误判
        let weak_window = std::time::Duration::from_secs(2);
        let mut fx = Fixture::new("apache");
        fx.retry_delay = weak_window + std::time::Duration::from_millis(100);
        {
            let mut st = fx.server();
            st.weak_window = Some(weak_window);
            st.ius_always_fails = true;
            st.omit_put_headers = true;
        }
        // 另一台设备刚刚写入（还在弱 ETag 窗口内）
        fx.other_device_writes(&doc(vec![todo_json(2, "远端", "2026-09-02 10:00:00")]));
        add_local_todo(&fx.db, 1, "本地", "2026-09-01 10:00:00");

        let started = std::time::Instant::now();
        let report = fx.engine().sync(false).expect("弱 ETag 窗口过后重试应成功");
        assert!(started.elapsed() >= fx.retry_delay);
        assert_eq!(report.status, SyncStatus::Merged);
        assert_eq!(remote_ids(&fx.remote()), vec![1, 2]);
        let preconditions = fx.server().put_preconditions.clone();
        assert_eq!(
            preconditions,
            vec![
                (Some("\"v1\"".to_string()), None),
                (Some("\"v1\"".to_string()), None)
            ],
            "弱 ETag 去掉 W/ 发 If-Match；窗口内 412，等过窗口后同一个 ETag 成功"
        );
        let current = fx.server().files[SYNC_DATA_FILE].etag.clone();
        assert_eq!(
            opaque_etag(&setting(&fx.db, KEY_REMOTE_ETAG)),
            current,
            "PUT 不带校验器：基准来自 HEAD"
        );

        // 本机刚写完时 HEAD 给的是弱 ETag；以它为基准的条件 GET 照样 304（弱比较）
        fx.db
            .with_connection(|c| {
                set_setting(c, KEY_REMOTE_ETAG, &format!("W/{}", current)).map(|_| ())
            })
            .unwrap();
        fx.clear_log();
        assert_eq!(
            fx.engine().sync(false).unwrap().status,
            SyncStatus::NoChanges
        );
        assert_eq!(fx.count(PUT_DOC), 0);
        assert!(!fx.sent_time_conditions(), "绝不发基于时间的条件头");
    }

    /// nginx dav：PUT 不带校验器且不检查条件头；PROPFIND 没有 getetag；只有 HEAD / GET 给强 ETag；
    /// If-Modified-Since 按整秒比较。实测故障：基准只有 Last-Modified 时，
    /// 同一秒内的远端修改被 If-Modified-Since 判成 304，变更漏掉。
    #[test]
    fn nginx_like_server_does_not_miss_writes_in_the_same_second() {
        let fx = Fixture::new("nginx");
        {
            let mut st = fx.server();
            st.nginx_second = Some(1_000);
            st.omit_put_headers = true;
            st.ignore_put_preconditions = true;
            st.propfind_without_etag = true;
        }
        add_local_todo(&fx.db, 1, "本地", "2026-09-01 10:00:00");
        fx.engine().sync(false).expect("首次上传");
        let base = setting(&fx.db, KEY_REMOTE_ETAG);
        assert_eq!(
            base,
            fx.server().files[SYNC_DATA_FILE].etag,
            "基准 ETag 来自 HEAD"
        );
        assert_eq!(fx.count("PROPFIND /mini-todo/sync-data.json.gz"), 0);

        // 同一秒内另一台设备写入：Last-Modified 不变，ETag 随大小变化
        let mut other = fx.remote();
        other["todos"].as_array_mut().unwrap().push(todo_json(
            9,
            "同一秒的修改",
            "2026-09-09 10:00:00",
        ));
        fx.other_device_writes(&other);
        assert_eq!(
            fx.server().files[SYNC_DATA_FILE].last_modified,
            setting(&fx.db, KEY_LAST_MODIFIED),
            "前提：Last-Modified 与基准相同"
        );

        let report = fx.engine().sync(false).expect("同步");
        assert_eq!(report.status, SyncStatus::Pulled);
        assert!(local_titles(&fx.db).iter().any(|(id, _)| *id == 9));
        let conditions = fx.server().get_conditions.clone();
        assert_eq!(
            conditions.last(),
            Some(&(Some(base), None)),
            "只带 If-None-Match，不带 If-Modified-Since"
        );
        assert!(!fx.sent_time_conditions());
    }

    /// nginx 的 ETag 只到秒：另一台设备在本机记下基准的同一秒内改写了 sync-data、压缩后大小又恰好
    /// 不变时 ETag 不变，普通轮次的条件 GET 得到 304、看不到这次修改（盲区，与 cloud 相同）。
    /// 手动同步（`full`）与每第 FULL_GET_EVERY 轮做无条件 GET，把它合并进来。
    #[test]
    fn nginx_same_second_same_size_rewrite_is_caught_by_a_full_get() {
        let fx = Fixture::new("nginx-blind-spot");
        {
            let mut st = fx.server();
            st.nginx_second = Some(2_000);
            st.omit_put_headers = true;
            st.ignore_put_preconditions = true;
            st.propfind_without_etag = true;
        }
        add_local_todo(&fx.db, 1, "aaaa", "2026-09-01 10:00:00");
        fx.engine().sync(false).expect("首次上传");
        let base = setting(&fx.db, KEY_REMOTE_ETAG);
        let base_len = fx.server().files[SYNC_DATA_FILE].body.len();

        // 同一秒内另一台设备改了标题与 updatedAt，写回的文件与原文件等长。文档里有 deviceId / 时间等随运行时刻
        // 变化的内容，不能指望碰巧压缩到等长：取一个压缩后不比原文件大的改写，再用 gzip 头补齐到等长
        let original = fx.remote();
        let (title, rewritten) = (1..=4)
            .flat_map(|len| (b'b'..=b'z').map(move |c| (c as char).to_string().repeat(len)))
            .find_map(|title| {
                let mut doc = original.clone();
                doc["todos"][0]["title"] = json!(title);
                doc["todos"][0]["updatedAt"] = json!("2026-09-02 10:00:00");
                let bytes = gzip_padded_to(&serde_json::to_vec(&doc).unwrap(), base_len)?;
                Some((title, bytes))
            })
            .expect("应能构造出与原文件等长的改写");
        fx.server().write(SYNC_DATA_FILE, rewritten);
        assert_eq!(
            fx.server().files[SYNC_DATA_FILE].etag,
            base,
            "前提：ETag 没变"
        );

        // 普通的自动同步轮：If-None-Match → 304，修改被盲区挡住
        let report = fx.engine().sync(false).unwrap();
        assert_eq!(report.status, SyncStatus::NoChanges);
        assert_eq!(local_titles(&fx.db), vec![(1, "aaaa".to_string())]);
        assert_eq!(
            fx.server().get_conditions.last(),
            Some(&(Some(base.clone()), None))
        );

        // 手动同步 / 兜底轮：无条件 GET，合并到这次修改；没有需要上传的
        fx.clear_log();
        let report = fx.engine().sync(true).unwrap();
        assert_eq!(report.status, SyncStatus::Pulled);
        assert_eq!(report.todos_updated, 1);
        assert_eq!(local_titles(&fx.db), vec![(1, title)]);
        assert_eq!(fx.server().get_conditions.last(), Some(&(None, None)));
        assert_eq!(fx.count(PUT_DOC), 0);
    }

    /// 兜底轮与 cloud 的 `is_full_pull_round` 同一规则：第 10、20、30… 轮；手动同步每轮都是
    #[test]
    fn every_tenth_smart_sync_round_and_every_manual_sync_do_a_full_get() {
        let full_rounds: Vec<u32> = (0..=35).filter(|r| is_full_get_round(*r)).collect();
        assert_eq!(full_rounds, vec![10, 20, 30]);
        assert_eq!(FULL_GET_EVERY, 10);

        for round in [1, 2, 9, 11, u32::MAX] {
            assert!(!wants_full_get(None, round), "自动同步第 {round} 轮");
            assert!(!wants_full_get(Some(false), round));
            assert!(wants_full_get(Some(true), round), "手动同步总是无条件");
        }
        assert!(wants_full_get(None, 10));
        assert!(wants_full_get(Some(false), 20));
    }

    /// 只有 Last-Modified 的旧基准（R1 之前 / 不给 ETag 的服务端）：做无条件 GET，不发 IMS
    #[test]
    fn base_without_etag_downloads_unconditionally() {
        let fx = Fixture::new("lm-only-base");
        add_local_todo(&fx.db, 1, "本地", "2026-09-01 10:00:00");
        fx.engine().sync(false).unwrap();
        fx.db
            .with_connection(|c| set_setting(c, KEY_REMOTE_ETAG, "").map(|_| ()))
            .unwrap();
        fx.clear_log();
        assert_eq!(
            fx.engine().sync(false).unwrap().status,
            SyncStatus::NoChanges
        );
        assert_eq!(
            fx.server().get_conditions.last(),
            Some(&(None, None)),
            "没有 ETag 就不带条件"
        );
        assert_eq!(fx.count(PUT_DOC), 0);
    }

    #[test]
    fn deletions_propagate_through_tombstones_in_both_directions() {
        let fx = Fixture::new("tombstones");
        fx.set_remote(&doc(vec![todo_json(2, "远端", "2026-09-02 10:00:00")]));
        add_local_todo(&fx.db, 1, "本地", "2026-09-01 10:00:00");
        fx.engine().sync(false).unwrap();

        // 本地删除 → 墓碑随上传传播，远端记录消失
        fx.db
            .with_transaction(|tx| sync_store::delete_todo_with_tombstones(tx, 2, &now_local()))
            .unwrap();
        let report = fx.engine().sync(false).unwrap();
        assert_eq!(report.status, SyncStatus::Pushed);
        let remote = fx.remote();
        assert_eq!(remote_ids(&remote), vec![1]);
        assert_eq!(remote["tombstones"][0]["entityType"], "todo");
        assert_eq!(remote["tombstones"][0]["entityId"], 2);

        // 另一台设备删除了 1 → 本地随之删除；不会把 1 重新传上去
        let mut other = fx.remote();
        other["todos"] = json!([]);
        other["tombstones"]
            .as_array_mut()
            .unwrap()
            .push(json!({"entityType": "todo", "entityId": 1, "deletedAt": now_local()}));
        fx.set_remote(&other);
        fx.clear_log();
        let report = fx.engine().sync(false).unwrap();
        assert_eq!(report.todos_deleted, 1);
        assert_eq!(report.status, SyncStatus::Pulled);
        assert!(local_titles(&fx.db).is_empty());
        assert_eq!(fx.count(PUT_DOC), 0);
    }

    #[test]
    fn local_only_records_are_reuploaded_when_remote_lost_them() {
        let fx = Fixture::new("reupload");
        add_local_todo(&fx.db, 1, "本地", "2026-09-01 10:00:00");
        fx.engine().sync(false).unwrap();

        // 另一个写入方（旧版本 / 不检查条件头的服务端）把远端整包覆盖掉了
        fx.set_remote(&doc(vec![todo_json(7, "别人的", "2026-09-07 10:00:00")]));
        let report = fx.engine().sync(false).unwrap();
        assert_eq!(report.status, SyncStatus::Merged);
        assert_eq!(remote_ids(&fx.remote()), vec![1, 7]);
    }

    #[test]
    fn force_push_and_force_pull() {
        let fx = Fixture::new("force");
        fx.set_remote(&doc(vec![
            todo_json(2, "远端独有", "2026-09-02 10:00:00"),
            todo_json(1, "远端较新", "2026-09-09 10:00:00"),
        ]));
        add_local_todo(&fx.db, 1, "本地", "2026-09-01 10:00:00");

        let report = fx.engine().force_push().expect("强制推送");
        assert_eq!(report.status, SyncStatus::Pushed);
        let remote = fx.remote();
        assert_eq!(remote_ids(&remote), vec![1]);
        assert_eq!(remote["todos"][0]["title"], "本地");
        assert!(remote["todos"][0]["updatedAt"].as_str().unwrap() > "2026-09-09 10:00:00");
        let tombs: Vec<i64> = remote["tombstones"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["entityId"].as_i64().unwrap())
            .collect();
        assert_eq!(tombs, vec![2]);

        let mut cloud = doc(vec![todo_json(3, "云端唯一", "2026-09-03 10:00:00")]);
        cloud["settings"] =
            json!({"viewMode": "quadrant", "windowSize": {"width": 1, "height": 1}});
        cloud["settingsUpdatedAt"] = json!("2026-09-03 10:00:00");
        fx.set_remote(&cloud);
        let report = fx.engine().force_pull().expect("强制拉取");
        assert_eq!(report.status, SyncStatus::Pulled);
        assert!(report.settings_applied);
        assert_eq!(local_titles(&fx.db), vec![(3, "云端唯一".to_string())]);
        assert_eq!(setting(&fx.db, "view_mode"), "quadrant");
        assert_eq!(setting(&fx.db, "window_size"), "", "设备相关键不应用");
        let local_tombs = fx
            .db
            .with_connection(|c| sync_store::list_tombstones(c, "2000-01-01 00:00:00"))
            .unwrap();
        assert!(local_tombs.is_empty(), "墓碑表与远端一致（远端没有墓碑）");

        // 强制拉取后再智能同步：无事可做
        fx.clear_log();
        assert_eq!(
            fx.engine().sync(false).unwrap().status,
            SyncStatus::NoChanges
        );
        assert_eq!(fx.count(PUT_DOC), 0);
    }

    /// 强制推送：远端独有的待办 / 子任务一律写墓碑（含整数 entityId），并落到本地墓碑表，
    /// 其它设备与 cloud 据此删除它们。远端记录的 updatedAt 比本机时钟还新（另一端时钟偏快 /
    /// 时区配置不同）时，墓碑时间取它，保证 deletedAt >= updatedAt 能压住。
    #[test]
    fn force_push_tombstones_every_remote_only_record() {
        let fx = Fixture::new("force-push-tombs");
        let mut remote_shared = todo_json(1, "两边都有", "2026-09-01 10:00:00");
        remote_shared["subtasks"] = json!([
            {"id": 11, "parentId": 1, "title": "两边都有的子任务", "content": null,
             "completed": false, "sortOrder": 0,
             "createdAt": "2026-09-01 10:00:00", "updatedAt": "2026-09-01 10:00:00"},
            {"id": 12, "parentId": 1, "title": "远端独有子任务", "content": null,
             "completed": false, "sortOrder": 1,
             "createdAt": "2026-09-01 10:00:00", "updatedAt": "2099-01-01 08:00:00"}
        ]);
        let mut remote_only = todo_json(1_759_400_000_000_123, "云端新建", "2099-01-01 08:00:00");
        remote_only["subtasks"] = json!([
            {"id": 1_759_400_000_000_456_i64, "parentId": 1_759_400_000_000_123_i64,
             "title": "云端子任务", "content": null, "completed": false, "sortOrder": 0,
             "createdAt": "2026-09-01 10:00:00", "updatedAt": "2026-09-01 10:00:00"}
        ]);
        fx.set_remote(&doc(vec![remote_shared, remote_only]));

        add_local_todo(&fx.db, 1, "两边都有", "2026-09-01 10:00:00");
        fx.db
            .with_transaction(|tx| {
                tx.execute(
                    "INSERT INTO subtasks (id, parent_id, title, created_at, updated_at)
                     VALUES (11, 1, '两边都有的子任务', '2026-09-01 10:00:00', '2026-09-01 10:00:00')",
                    [],
                )
            })
            .unwrap();

        fx.engine().force_push().expect("强制推送");
        let remote = fx.remote();
        assert_eq!(remote_ids(&remote), vec![1]);
        let subs: Vec<i64> = remote["todos"][0]["subtasks"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["id"].as_i64().unwrap())
            .collect();
        assert_eq!(subs, vec![11]);

        let mut tombs: Vec<(String, i64, String)> = remote["tombstones"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| {
                assert!(t["entityId"].is_i64(), "entityId 必须是整数: {t}");
                (
                    t["entityType"].as_str().unwrap().to_string(),
                    t["entityId"].as_i64().unwrap(),
                    t["deletedAt"].as_str().unwrap().to_string(),
                )
            })
            .collect();
        tombs.sort();
        let keys: Vec<(&str, i64)> = tombs.iter().map(|(t, id, _)| (t.as_str(), *id)).collect();
        assert_eq!(
            keys,
            vec![
                ("subtask", 12),
                ("subtask", 1_759_400_000_000_456),
                ("todo", 1_759_400_000_000_123)
            ]
        );
        for (kind, id, deleted_at) in &tombs {
            assert_eq!(deleted_at.len(), 19, "规范时间格式: {deleted_at}");
            if *id == 12 || *id == 1_759_400_000_000_123 {
                assert!(
                    deleted_at.as_str() >= "2099-01-01 08:00:00",
                    "{kind}#{id} 的墓碑必须压得住它的 updatedAt"
                );
            }
        }

        let local_tombs = fx
            .db
            .with_connection(|c| sync_store::list_tombstones(c, "2000-01-01 00:00:00"))
            .unwrap();
        assert_eq!(local_tombs.len(), 3, "墓碑同时落到本地墓碑表");
    }

    #[test]
    fn force_pull_requires_remote_data() {
        let fx = Fixture::new("force-pull-empty");
        assert!(fx
            .engine()
            .force_pull()
            .unwrap_err()
            .contains("远端还没有同步数据"));
    }

    #[test]
    fn images_upload_before_sync_data_and_missing_ones_download() {
        let fx = Fixture::new("images");
        std::fs::write(fx.dir.join("a.png"), b"local-image").unwrap();
        std::fs::write(fx.dir.join("bad name.png"), b"x").unwrap();
        {
            let mut st = fx.mock.state.lock().unwrap();
            st.dirs.insert(REMOTE_IMAGES_DIR.to_string());
            st.write("/mini-todo/images/b.png", b"remote-image".to_vec());
        }
        let mut remote = doc(vec![]);
        remote["images"] = json!(["b.png", "../evil.png"]);
        fx.set_remote(&remote);
        add_local_todo(&fx.db, 1, "本地", "2026-09-01 10:00:00");

        let report = fx.engine().sync(false).unwrap();
        assert_eq!(report.images_uploaded, 1);
        assert_eq!(report.images_downloaded, 1);
        assert_eq!(
            std::fs::read(fx.dir.join("b.png")).unwrap(),
            b"remote-image"
        );
        assert!(!fx.dir.parent().unwrap().join("evil.png").exists());

        let log = fx.log();
        let image_put = log
            .iter()
            .position(|l| l == "PUT /mini-todo/images/a.png")
            .expect("应上传 a.png");
        let doc_put = log
            .iter()
            .position(|l| l == PUT_DOC)
            .expect("应上传 sync-data");
        assert!(image_put < doc_put, "先传图片再传 sync-data");
        assert_eq!(
            log.iter()
                .filter(|l| l.starts_with("PROPFIND /mini-todo/images"))
                .count(),
            1
        );
        assert!(!log
            .iter()
            .any(|l| l.contains("bad name") || l.contains("evil")));
        assert_eq!(fx.remote()["images"], json!(["a.png", "b.png"]));
    }

    // ------------------------------------------------------------------
    // 文档拼装 / 解析（不走网络）
    // ------------------------------------------------------------------

    fn local_state(db: &Database) -> LocalState {
        db.with_connection(|conn| {
            Ok(LocalState {
                records: sync_store::local_records(conn, "2000-01-01 00:00:00")?,
                settings: read_app_settings(conn),
                settings_version: settings_version(conn)?,
            })
        })
        .unwrap()
    }

    #[test]
    fn decode_sync_doc_is_lenient_on_values_but_strict_on_structure() {
        let parsed = decode_sync_doc(
            br#"{"version": 4, "deviceId": null, "images": [1, "a.png"],
                 "settingsUpdatedAt": "2026-05-01T10:00",
                 "tombstones": [{"entityType": "todo", "entityId": 1, "deletedAt": "2026-05-01"},
                                {"bogus": true}],
                 "todos": null, "futureKey": 7}"#,
        )
        .expect("未压缩 JSON 也接受");
        assert_eq!(parsed.version, "4");
        assert_eq!(parsed.device_id, "");
        assert_eq!(parsed.images, vec!["a.png".to_string()]);
        assert_eq!(
            parsed.settings_updated_at.as_deref(),
            Some("2026-05-01 10:00:00")
        );
        assert_eq!(parsed.tombstones.len(), 1);
        assert!(parsed.todos.is_empty());
        assert_eq!(parsed.extra.get("futureKey"), Some(&json!(7)));

        for bad in [
            &b"[]"[..],
            br#"{"todos": "x"}"#,
            br#"{"tombstones": {}}"#,
            b"nope",
        ] {
            assert!(
                decode_sync_doc(bad).is_err(),
                "{}",
                String::from_utf8_lossy(bad)
            );
        }
    }

    #[test]
    fn sync_data_serialization_keeps_extra_keys_and_omits_missing_settings_version() {
        let mut data = decode_sync_doc(br#"{"futureKey": {"a": 1}, "todos": []}"#).unwrap();
        let text = serde_json::to_string(&data).unwrap();
        assert!(text.contains(r#""futureKey":{"a":1}"#));
        assert!(!text.contains("settingsUpdatedAt"));
        assert!(text.contains(r#""tombstones":[]"#));
        data.settings_updated_at = Some("2026-05-01 10:00:00".to_string());
        assert!(serde_json::to_string(&data)
            .unwrap()
            .contains("settingsUpdatedAt"));
    }

    #[test]
    fn doc_equivalent_to_remote_needs_no_upload() {
        let db = Database::new_in_memory().unwrap();
        add_local_todo(&db, 1, "本地", "2026-09-01 10:00:00");
        let local = local_state(&db);
        let cutoff = "2000-01-01 00:00:00";

        // 本机刚上传的文档 == 远端
        let uploaded = build_sync_doc(&local, None, DocMode::Merge, "pc", cutoff);
        let remote: SyncData =
            serde_json::from_value(serde_json::to_value(&uploaded).unwrap()).unwrap();
        let again = build_sync_doc(&local, Some(&remote), DocMode::Merge, "pc", cutoff);
        assert!(!doc_differs(&again, &remote, cutoff));

        // 远端缺本地的记录 → 需要上传
        let mut stale = remote.clone();
        stale.todos.clear();
        let doc = build_sync_doc(&local, Some(&stale), DocMode::Merge, "pc", cutoff);
        assert!(doc_differs(&doc, &stale, cutoff));

        // 本地多了墓碑 → 需要上传
        db.with_transaction(|tx| {
            sync_store::record_tombstone(tx, EntityKind::Todo, 42, "2026-09-01 00:00:00")
        })
        .unwrap();
        let doc = build_sync_doc(
            &local_state(&db),
            Some(&remote),
            DocMode::Merge,
            "pc",
            cutoff,
        );
        assert!(doc_differs(&doc, &remote, cutoff));
    }

    #[test]
    fn todo_value_merge_follows_lww_tombstones_and_union() {
        let mut tombs = TombstoneIndex::new();
        tombs.insert((EntityKind::Todo, 4), "2026-09-04 00:00:00".to_string());
        tombs.insert((EntityKind::Subtask, 12), "2026-09-04 00:00:00".to_string());

        let mut local1 = todo_json(1, "本地较新", "2026-09-05 00:00:00");
        local1["subtasks"] =
            json!([{"id": 11, "title": "本地子", "updatedAt": "2026-09-01 00:00:00"}]);
        let mut remote1 = todo_json(1, "远端较旧", "2026-09-01 00:00:00");
        remote1["subtasks"] = json!([
            {"id": 11, "title": "远端子较新", "updatedAt": "2026-09-02 00:00:00"},
            {"id": 12, "title": "已删除", "updatedAt": "2026-09-01 00:00:00"},
            {"id": 13, "title": "远端新增", "updatedAt": "2026-09-01 00:00:00"}
        ]);
        let remote = vec![
            remote1,
            json!({"id": 2, "quadrant": "看不懂", "updatedAt": "2026-09-01 00:00:00"}),
            json!("无 id 的垃圾"),
            todo_json(4, "已删除", "2026-09-03 00:00:00"),
        ];
        let merged = merge_todo_values(vec![local1], &remote, &tombs);

        assert_eq!(merged.len(), 3);
        assert_eq!(merged[0]["title"], "本地较新");
        let subs: Vec<(i64, String)> = merged[0]["subtasks"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| {
                (
                    s["id"].as_i64().unwrap(),
                    s["title"].as_str().unwrap().to_string(),
                )
            })
            .collect();
        assert_eq!(
            subs,
            vec![(11, "远端子较新".to_string()), (13, "远端新增".to_string())]
        );
        assert_eq!(merged[1]["quadrant"], "看不懂");
        assert_eq!(merged[2], json!("无 id 的垃圾"));
    }

    /// 回归：远端同 id 记录的 updatedAt 是乱码时，以前按原串比较（`"zzz" > "2026-…"`），
    /// 上传文档里留的是远端那份看不懂的版本，本地合法的修改永远传不出去。
    /// 现在乱码视为最旧（与 cloud 的 canon_ts 一致）：本地版本胜出，并触发一次上传把远端修好。
    #[test]
    fn unparseable_remote_timestamp_never_beats_the_local_version() {
        let fx = Fixture::new("garbage-ts");
        let mut garbage = todo_json(1, "远端乱码时间", "2026-09-01 10:00:00");
        garbage["updatedAt"] = json!("zzz");
        fx.set_remote(&doc(vec![garbage]));
        add_local_todo(&fx.db, 1, "本地合法版本", "2026-09-05 10:00:00");

        let report = fx.engine().sync(false).expect("同步");
        assert_eq!(report.records_skipped, 1, "看不懂的远端版本跳过");
        assert_eq!(
            local_titles(&fx.db),
            vec![(1, "本地合法版本".to_string())],
            "本地不受影响"
        );
        let remote = fx.remote();
        assert_eq!(remote["todos"][0]["title"], "本地合法版本", "远端被修好");
        assert_eq!(remote["todos"][0]["updatedAt"], "2026-09-05 10:00:00");

        fx.clear_log();
        assert_eq!(
            fx.engine().sync(false).unwrap().status,
            SyncStatus::NoChanges
        );
        assert_eq!(fx.count(PUT_DOC), 0, "修好之后不再反复上传");
    }

    #[test]
    fn subtask_listed_under_two_todos_is_kept_once() {
        let mut a = todo_json(1, "A", "2026-09-01 00:00:00");
        a["subtasks"] = json!([{"id": 9, "title": "旧位置", "updatedAt": "2026-09-01 00:00:00"}]);
        let mut b = todo_json(2, "B", "2026-09-01 00:00:00");
        b["subtasks"] = json!([
            {"id": 9, "title": "新位置", "updatedAt": "2026-09-02 00:00:00"},
            {"id": 10, "title": "其它", "updatedAt": "2026-09-01 00:00:00"}
        ]);
        let merged = merge_todo_values(vec![a], &[b], &TombstoneIndex::new());
        assert_eq!(merged[0]["subtasks"], json!([]));
        let b_subs: Vec<i64> = merged[1]["subtasks"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["id"].as_i64().unwrap())
            .collect();
        assert_eq!(b_subs, vec![9, 10]);
    }

    #[test]
    fn settings_block_prefers_newer_side_and_passes_remote_through() {
        let db = Database::new_in_memory().unwrap();
        db.with_connection(|c| {
            c.execute("UPDATE settings SET updated_at = '2026-09-01 00:00:00'", [])?;
            Ok(())
        })
        .unwrap();
        let local = local_state(&db);
        let cutoff = "2000-01-01 00:00:00";

        let mut remote = SyncData {
            settings: json!({"textTheme": "light", "somethingNew": true}),
            settings_updated_at: Some("2026-09-05 00:00:00".to_string()),
            ..Default::default()
        };
        let doc = build_sync_doc(&local, Some(&remote), DocMode::Merge, "pc", cutoff);
        assert_eq!(doc.settings, remote.settings, "远端较新：原样透传");
        assert_eq!(doc.settings_updated_at, remote.settings_updated_at);

        remote.settings_updated_at = Some("2026-08-01 00:00:00".to_string());
        let doc = build_sync_doc(&local, Some(&remote), DocMode::Merge, "pc", cutoff);
        assert_eq!(doc.settings["textTheme"], "dark", "本地较新：上传本地");
        assert_eq!(
            doc.settings_updated_at.as_deref(),
            Some("2026-09-01 00:00:00")
        );

        remote.settings_updated_at = None;
        let doc = build_sync_doc(&local, Some(&remote), DocMode::Merge, "pc", cutoff);
        assert_eq!(
            doc.settings_updated_at.as_deref(),
            Some("2026-09-01 00:00:00"),
            "远端无版本：上传本地"
        );
    }

    #[test]
    fn remote_settings_apply_only_when_newer_unless_first_sync() {
        let db = Database::new_in_memory().unwrap();
        db.with_connection(|c| {
            c.execute("UPDATE settings SET updated_at = '2026-09-01 00:00:00'", [])?;
            Ok(())
        })
        .unwrap();
        let mut remote = SyncData {
            settings: json!({"viewMode": "quadrant"}),
            settings_updated_at: Some("2026-08-01 00:00:00".to_string()),
            ..Default::default()
        };
        let apply = |remote: &SyncData, first: bool| {
            db.with_transaction(|tx| apply_remote_settings_if_newer(tx, remote, first))
                .unwrap()
        };
        assert!(!apply(&remote, false), "远端较旧不应用");
        remote.settings_updated_at = None;
        assert!(!apply(&remote, true), "没有版本的远端设置永不应用");
        remote.settings_updated_at = Some("2026-08-01 00:00:00".to_string());
        assert!(apply(&remote, true), "首次同步采用远端设置");
        assert_eq!(setting(&db, "view_mode"), "quadrant");

        remote.settings = json!({"viewMode": "list"});
        remote.settings_updated_at = Some("2026-09-02 00:00:00".to_string());
        assert!(apply(&remote, false), "远端较新应用");
        assert_eq!(setting(&db, "view_mode"), "list");
    }

    /// 回归：首次同步采用了远端设置、但在上传前失败（412 用尽 / 断网）时，以前仍处于
    /// "首次同步"状态，下次同步会再无条件采用一遍远端设置，覆盖用户在两次之间改过的设置
    #[test]
    fn first_sync_adopts_remote_settings_only_once() {
        let fx = Fixture::new("first-sync-once");
        let mut remote = doc(vec![]);
        remote["settings"] = json!({"textTheme": "light"});
        remote["settingsUpdatedAt"] = json!("2026-09-03 00:00:00");
        let remote: SyncData = serde_json::from_value(remote).unwrap();
        let cutoff = "2000-01-01 00:00:00";
        let engine = fx.engine();

        assert!(engine.read_pre_state().unwrap().first_sync);
        let (_, applied) = engine.merge(&remote, true, cutoff).unwrap();
        assert!(applied, "首次同步采用远端设置");
        assert_eq!(setting(&fx.db, "text_theme"), "light");

        // 合并之后、上传之前失败（finish 没跑）：已经不是首次同步了
        let pre = engine.read_pre_state().unwrap();
        assert!(!pre.first_sync);
        assert_eq!(pre.synced_settings_at, "2026-09-03 00:00:00");
        assert!(
            pre.settings_version <= pre.synced_settings_at,
            "采用后的设置本身不算未同步的变更"
        );

        // 用户在两次同步之间改了设置：下一次合并不能再把它覆盖回去
        fx.db
            .with_connection(|c| set_setting(c, "text_theme", "dark").map(|_| ()))
            .unwrap();
        let (_, applied) = engine.merge(&remote, pre.first_sync, cutoff).unwrap();
        assert!(!applied);
        assert_eq!(setting(&fx.db, "text_theme"), "dark");
        assert!(engine.read_pre_state().unwrap().dirty(), "新设置待上传");
    }

    // ------------------------------------------------------------------
    // 同步设置 / 互斥
    // ------------------------------------------------------------------

    fn sync_settings(url: &str, password: &str) -> SyncSettings {
        SyncSettings {
            webdav_url: url.to_string(),
            webdav_username: "user".to_string(),
            webdav_password: password.to_string(),
            has_password: false,
            auto_sync: true,
            sync_interval: 30,
            last_sync_at: None,
            device_id: String::new(),
            clear_password: false,
        }
    }

    #[test]
    fn password_is_never_returned_and_kept_when_left_empty() {
        let db = Database::new_in_memory().unwrap();
        save_sync_settings_inner(&db, &sync_settings("https://dav.example.com", "secret")).unwrap();
        let read = read_sync_settings(&db).unwrap();
        assert_eq!(read.webdav_password, "");
        assert!(read.has_password);
        assert!(!read.device_id.is_empty());
        assert_eq!(read.sync_interval, 30);
        assert!(read.auto_sync);
        assert_eq!(stored_password(&db).unwrap(), "secret");

        save_sync_settings_inner(&db, &sync_settings("https://dav.example.com", "")).unwrap();
        assert_eq!(stored_password(&db).unwrap(), "secret", "留空 = 不修改");

        let mut clear = sync_settings("https://dav.example.com", "");
        clear.clear_password = true;
        save_sync_settings_inner(&db, &clear).unwrap();
        assert!(!read_sync_settings(&db).unwrap().has_password);
    }

    #[test]
    fn changing_server_resets_sync_base() {
        let db = Database::new_in_memory().unwrap();
        save_sync_settings_inner(&db, &sync_settings("https://a.example.com", "p")).unwrap();
        db.with_transaction(|tx| -> rusqlite::Result<()> {
            set_setting(tx, KEY_REMOTE_ETAG, "\"x\"")?;
            set_setting(tx, KEY_LAST_SYNC_AT, "2026-09-01T00:00:00+08:00")?;
            sync_store::set_synced_seq(tx, 99)
        })
        .unwrap();

        save_sync_settings_inner(&db, &sync_settings("https://a.example.com", "")).unwrap();
        assert_eq!(setting(&db, KEY_REMOTE_ETAG), "\"x\"", "同一服务器不重置");

        save_sync_settings_inner(&db, &sync_settings("https://b.example.com", "")).unwrap();
        assert_eq!(setting(&db, KEY_REMOTE_ETAG), "");
        assert_eq!(setting(&db, KEY_LAST_SYNC_AT), "");
        assert_eq!(db.with_connection(sync_store::synced_seq).unwrap(), 0);
    }

    #[test]
    fn only_one_sync_at_a_time() {
        let first = SyncGuard::acquire().expect("第一个同步拿到锁");
        assert_eq!(
            SyncGuard::acquire().err().as_deref(),
            Some("同步正在进行中")
        );
        drop(first);
        assert!(SyncGuard::acquire().is_ok());
    }
}
