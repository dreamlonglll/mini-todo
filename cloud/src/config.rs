//! `config.toml` 解析与运行时配置。
//!
//! 缺任意必填字段时直接以清晰错误退出（在 `main.rs` 用 `expect` / `?` 转
//! `anyhow::Error` 的 root cause 打出来），不在运行期做兜底。

use std::path::{Path, PathBuf};

use chrono::FixedOffset;
use chrono_tz::Tz;
use serde::Deserialize;

/// 解析后的运行时配置。所有字段都已校验完毕，可以直接使用。
#[derive(Debug, Clone)]
pub struct Config {
    pub webdav_url: String,
    pub webdav_username: String,
    pub webdav_password: String,
    pub api_key: String,
    pub bind: String,
    /// IANA 时区，例如 `Asia/Shanghai`。保留 `Tz` 是为了让 PR2 push worker
    /// 在 DST 切换时能重新计算 offset（PR1 还用不到，先 allow）。
    #[allow(dead_code)]
    pub timezone: Tz,
    /// `timezone` 当前时刻对应的 `FixedOffset`，用于生成与 PC SQLite
    /// `datetime('now','localtime')` 完全一致的时间戳字符串。
    pub timezone_offset: FixedOffset,
    pub pull_interval_secs: u64,
    pub data_dir: PathBuf,
    pub images_dir: PathBuf,
}

/// `config.toml` 的原始反序列化结构。任意缺字段直接报错。
#[derive(Debug, Deserialize)]
struct RawConfig {
    webdav_url: String,
    webdav_username: String,
    webdav_password: String,
    api_key: String,
    #[serde(default = "default_bind")]
    bind: String,
    #[serde(default = "default_timezone")]
    timezone: String,
    #[serde(default = "default_pull_interval")]
    pull_interval: u64,
    #[serde(default = "default_data_dir")]
    data_dir: PathBuf,
    #[serde(default = "default_images_dir")]
    images_dir: PathBuf,
}

fn default_bind() -> String {
    "127.0.0.1:8787".to_string()
}
fn default_timezone() -> String {
    "Asia/Shanghai".to_string()
}
fn default_pull_interval() -> u64 {
    60
}
fn default_data_dir() -> PathBuf {
    PathBuf::from("/var/lib/minitodo")
}
fn default_images_dir() -> PathBuf {
    PathBuf::from("/var/lib/minitodo/images")
}

impl Config {
    /// 从指定路径加载 `config.toml`。
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let body = std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("无法读取配置文件 {}: {}", path.display(), e))?;

        let raw: RawConfig = toml::from_str(&body)
            .map_err(|e| anyhow::anyhow!("解析配置文件 {} 失败: {}", path.display(), e))?;

        if raw.webdav_url.trim().is_empty() {
            anyhow::bail!("config.toml: webdav_url 不能为空");
        }
        if raw.webdav_username.trim().is_empty() {
            anyhow::bail!("config.toml: webdav_username 不能为空");
        }
        if raw.webdav_password.trim().is_empty() {
            anyhow::bail!("config.toml: webdav_password 不能为空");
        }
        if raw.api_key.trim().is_empty() {
            anyhow::bail!("config.toml: api_key 不能为空");
        }
        if raw.api_key.len() < 16 {
            anyhow::bail!("config.toml: api_key 至少需要 16 个字符（建议 32+）");
        }
        if raw.pull_interval == 0 {
            anyhow::bail!("config.toml: pull_interval 必须 > 0");
        }

        let tz: Tz = raw.timezone.parse().map_err(|_| {
            anyhow::anyhow!(
                "config.toml: timezone '{}' 不是合法的 IANA 时区名（例如 Asia/Shanghai / UTC）",
                raw.timezone
            )
        })?;
        let timezone_offset = crate::time::offset_for_tz_now(tz);

        Ok(Config {
            webdav_url: raw.webdav_url.trim_end_matches('/').to_string(),
            webdav_username: raw.webdav_username,
            webdav_password: raw.webdav_password,
            api_key: raw.api_key,
            bind: raw.bind,
            timezone: tz,
            timezone_offset,
            pull_interval_secs: raw.pull_interval,
            data_dir: raw.data_dir,
            images_dir: raw.images_dir,
        })
    }
}
