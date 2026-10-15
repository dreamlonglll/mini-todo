//! WebDAV 同步（原 `commands/sync_cmd.rs`，10-02 全面优化 R1：契约 K1–K5、K7）。
//!
//! 对外入口：[`run`]（智能同步 / 强制拉取 / 强制推送，阻塞，内含单同步互斥）、[`get_settings`] /
//! [`save_settings`] / [`test_connection`]。原来的 `sync-completed` 事件改为返回值：调用方在
//! [`SyncReport::local_changed`] 时通知各窗口刷新，在 `settings_applied` 时刷新运行时偏好。
//! 契约见 `.trellis/spec/backend/sync-protocol.md`。
//!
//! 一次"智能同步"（`run(db, SyncMode::Smart { .. })`，手动按钮 / 设置页"立即同步" / 自动同步定时器共用）：
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
//! 删除通过墓碑传播；正常同步里没有"缺席即删除"。手动导入走 [`crate::data::import_json`]。

use crate::data::{adopt_settings_json, read_app_settings, settings_version, EXPORT_VERSION};
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
// 同步设置（界面 DTO）
// ============================================================================

fn default_sync_interval() -> i32 {
    15
}

/// 同步设置。
///
/// - 读取（[`get_settings`]）：`webdav_password` 恒为空串，明文密码不离开 core；`has_password` 表示已保存过密码
/// - 保存（[`save_settings`]）：`webdav_password` 为空表示保留已保存的密码；`clear_password: true` 显式清空。
///   `has_password` / `last_sync_at` / `device_id` 由 core 维护，保存时忽略
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

/// 读取同步设置（`get_sync_settings`）；设备 id 不存在时生成并保存
pub fn get_settings(db: &Database) -> Result<SyncSettings, String> {
    read_sync_settings(db)
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

/// 保存同步设置（`save_sync_settings`）。密码：`clear_password` > 新密码（Windows 上 DPAPI 加密）> 保留
/// （旧版明文顺手加密）；换了服务器 / 账号时清掉同步基准与 `synced_seq`，下次同步完整合并一次
pub fn save_settings(db: &Database, settings: &SyncSettings) -> Result<(), String> {
    save_sync_settings_inner(db, settings)
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

/// 测试连接（`webdav_test_connection`）。`password` 为空 / 缺省时使用已保存的密码（界面拿不到明文密码）。
/// 阻塞（网络），放后台线程执行。
pub fn test_connection(
    db: &Database,
    url: &str,
    username: &str,
    password: Option<&str>,
) -> Result<bool, String> {
    let password = match password.filter(|p| !p.is_empty()) {
        Some(p) => p.to_string(),
        None => stored_password(db)?,
    };
    WebDavClient::new(url, username, &password)?.test_connection()
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

/// 一次同步的结果（原 `sync-completed` 事件的 payload）
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

    /// 本地数据或设置是否被改动（决定是否通知各窗口刷新，原来据此 emit `sync-completed`）
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

/// 同步方式
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncMode {
    /// 智能同步（K4，原 `webdav_sync`）：手动同步、设置页"立即同步"、自动同步定时器共用。
    ///
    /// `full: true`（标题栏同步按钮、设置页"立即同步"）：本轮第一次 GET 不带 `If-None-Match`，
    /// 绕开 nginx 秒级 ETag 的盲区（见 [`FULL_GET_EVERY`]）；自动同步传 false，每第 10 轮自动无条件一次。
    Smart { full: bool },
    /// 用云端覆盖本地（原 `webdav_force_pull`）：删除本地独有的待办 / 子任务（不生成墓碑），应用远端设置
    /// （设备相关键除外）。远端不存在时报错。
    ForcePull,
    /// 用本地覆盖云端（原 `webdav_force_push`）：为远端独有的待办 / 子任务生成墓碑，条件 PUT 本地数据。
    ForcePush,
}

/// 执行一次同步（阻塞：网络 + 数据库，放后台线程执行；不能在 tokio runtime 上下文里调用）。
///
/// 同一时刻只允许一个同步（含强制拉取 / 推送）：已有同步在跑时立即返回 `Err("同步正在进行中")`。
/// 返回后调用方：[`SyncReport::local_changed`] → 通知各窗口刷新（原 `sync-completed` 事件）；
/// `settings_applied` → 重新读取运行时偏好（原 `reload_runtime_prefs`）。
pub fn run(db: &Database, mode: SyncMode) -> Result<SyncReport, String> {
    let _guard = SyncGuard::acquire()?;
    run_sync_blocking(db, mode)
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
            engine.sync(wants_full_get(Some(full), round))
        }
        SyncMode::ForcePull => engine.force_pull(),
        SyncMode::ForcePush => engine.force_push(),
    }?;
    // 远端设置改写了 top_on_wake / auto_hide_enabled 等时（report.settings_applied），
    // 由调用方刷新运行时偏好：core 不持有窗口状态
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
mod tests;
