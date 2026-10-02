//! `config.toml` 解析与运行时配置。
//!
//! 缺任意必填字段时直接以清晰错误退出（在 `main.rs` 用 `expect` / `?` 转
//! `anyhow::Error` 的 root cause 打出来），不在运行期做兜底。
//! 不致命但不安全的配置（短 api_key、明文 http WebDAV）通过 `Config::warnings`
//! 在启动时打 warn，不拒绝启动（兼容旧部署）。

use std::net::IpAddr;
use std::path::{Path, PathBuf};

use chrono_tz::Tz;
use reqwest::{Certificate, Url};
use serde::Deserialize;

/// api_key 的硬性下限（短于它拒绝启动）。
pub const API_KEY_MIN_LEN: usize = 16;
/// api_key 的建议长度（短于它启动时 warn）。
pub const API_KEY_RECOMMENDED_LEN: usize = 32;

/// 解析后的运行时配置。所有字段都已校验完毕，可以直接使用。
#[derive(Debug, Clone)]
pub struct Config {
    pub webdav_url: String,
    pub webdav_username: String,
    pub webdav_password: String,
    /// 可选：额外信任的 CA 证书文件（PEM，可含多张），用于自签证书的 WebDAV。
    pub webdav_ca_file: Option<PathBuf>,
    /// `webdav_ca_file` 解析出的证书（启动时已校验可用）。
    pub webdav_ca_certs: Vec<Certificate>,
    pub api_key: String,
    pub bind: String,
    /// IANA 时区，例如 `Asia/Shanghai`。用于生成与 PC SQLite
    /// `datetime('now','localtime')` 完全一致的时间戳字符串。
    ///
    /// 只存 `Tz`、不缓存换算后的偏移：每次取时间都按当时的偏移重新换算
    /// （见 `crate::time`），DST 时区切换后仍正确。
    pub timezone: Tz,
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
    #[serde(default)]
    webdav_ca_file: Option<PathBuf>,
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
        Self::parse(&body, path.parent())
            .map_err(|e| anyhow::anyhow!("配置文件 {}: {:#}", path.display(), e))
    }

    /// 解析配置文本。`base_dir` 用来解析相对路径的 `webdav_ca_file`（相对于配置文件所在目录）。
    fn parse(body: &str, base_dir: Option<&Path>) -> anyhow::Result<Self> {
        let raw: RawConfig =
            toml::from_str(body).map_err(|e| anyhow::anyhow!("解析失败: {}", e))?;

        let webdav_url = raw.webdav_url.trim().trim_end_matches('/').to_string();
        if webdav_url.is_empty() {
            anyhow::bail!("webdav_url 不能为空");
        }
        let url = Url::parse(&webdav_url)
            .map_err(|e| anyhow::anyhow!("webdav_url '{}' 不是合法 URL: {}", webdav_url, e))?;
        if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
            anyhow::bail!(
                "webdav_url '{}' 必须是 http:// 或 https:// 开头的完整地址",
                webdav_url
            );
        }
        if raw.webdav_username.trim().is_empty() {
            anyhow::bail!("webdav_username 不能为空");
        }
        if raw.webdav_password.trim().is_empty() {
            anyhow::bail!("webdav_password 不能为空");
        }
        if raw.api_key.trim().is_empty() {
            anyhow::bail!("api_key 不能为空");
        }
        if raw.api_key.len() < API_KEY_MIN_LEN {
            anyhow::bail!(
                "api_key 至少需要 {} 个字符（建议 {}+，例如 `openssl rand -hex 32`）",
                API_KEY_MIN_LEN,
                API_KEY_RECOMMENDED_LEN
            );
        }
        if raw.pull_interval == 0 {
            anyhow::bail!("pull_interval 必须 > 0");
        }

        let tz: Tz = raw.timezone.parse().map_err(|_| {
            anyhow::anyhow!(
                "timezone '{}' 不是合法的 IANA 时区名（例如 Asia/Shanghai / UTC）",
                raw.timezone
            )
        })?;

        let webdav_ca_file = raw.webdav_ca_file.map(|p| match base_dir {
            Some(dir) if p.is_relative() => dir.join(p),
            _ => p,
        });
        let webdav_ca_certs = match &webdav_ca_file {
            Some(p) => load_ca_file(p)?,
            None => Vec::new(),
        };

        Ok(Config {
            webdav_url,
            webdav_username: raw.webdav_username,
            webdav_password: raw.webdav_password,
            webdav_ca_file,
            webdav_ca_certs,
            api_key: raw.api_key,
            bind: raw.bind,
            timezone: tz,
            pull_interval_secs: raw.pull_interval,
            data_dir: raw.data_dir,
            images_dir: raw.images_dir,
        })
    }

    /// 不致命的配置隐患，启动时逐条 warn（不拒绝启动，兼容旧部署）。
    pub fn warnings(&self) -> Vec<String> {
        let mut out = Vec::new();
        if self.api_key.len() < API_KEY_RECOMMENDED_LEN {
            out.push(format!(
                "api_key 只有 {} 个字符，建议至少 {} 个随机字符（例如 `openssl rand -hex 32`）",
                self.api_key.len(),
                API_KEY_RECOMMENDED_LEN
            ));
        }
        if let Ok(url) = Url::parse(&self.webdav_url) {
            if url.scheme() == "http" && !is_loopback_host(&url) {
                out.push(format!(
                    "webdav_url 使用明文 http://（{}）：WebDAV 密码与待办数据会以明文经过网络，建议改用 https://",
                    url.host_str().unwrap_or("")
                ));
            }
        }
        out
    }

    /// 测试用构造器：跳过 `config.toml` 读盘，直接拼一个最小可用的 `Config`。
    /// `images_dir` / `data_dir` 由调用方传入（通常是 `tempfile::TempDir`），
    /// 时区固定 `Asia/Shanghai`、`pull_interval` 60s。
    #[cfg(test)]
    pub fn for_tests(api_key: &str, data_dir: PathBuf, images_dir: PathBuf) -> Self {
        let tz: Tz = "Asia/Shanghai".parse().unwrap();
        Config {
            webdav_url: "http://127.0.0.1:0/dav".to_string(),
            webdav_username: "u".to_string(),
            webdav_password: "p".to_string(),
            webdav_ca_file: None,
            webdav_ca_certs: Vec::new(),
            api_key: api_key.to_string(),
            bind: "127.0.0.1:0".to_string(),
            timezone: tz,
            pull_interval_secs: 60,
            data_dir,
            images_dir,
        }
    }
}

/// `localhost` / `*.localhost` / 回环 IP。
fn is_loopback_host(url: &Url) -> bool {
    let host = url
        .host_str()
        .unwrap_or("")
        .trim_start_matches('[')
        .trim_end_matches(']');
    match host.parse::<IpAddr>() {
        Ok(ip) => ip.is_loopback(),
        Err(_) => {
            let h = host.to_ascii_lowercase();
            h == "localhost" || h.ends_with(".localhost")
        }
    }
}

/// 读取并校验 PEM CA 文件（可含多张证书）。校验方式是真的把证书装进一个 TLS 客户端
/// （异步客户端，可以在 async 上下文里构造），坏证书在启动时就报错而不是等第一次同步。
fn load_ca_file(path: &Path) -> anyhow::Result<Vec<Certificate>> {
    let pem = std::fs::read(path)
        .map_err(|e| anyhow::anyhow!("无法读取 webdav_ca_file {}: {}", path.display(), e))?;
    let certs = Certificate::from_pem_bundle(&pem).map_err(|e| {
        anyhow::anyhow!(
            "webdav_ca_file {} 不是合法的 PEM 证书: {}",
            path.display(),
            e
        )
    })?;
    if certs.is_empty() {
        anyhow::bail!(
            "webdav_ca_file {} 里没有找到证书（需要 -----BEGIN CERTIFICATE----- 块）",
            path.display()
        );
    }
    let mut builder = reqwest::Client::builder();
    for c in &certs {
        builder = builder.add_root_certificate(c.clone());
    }
    builder.build().map_err(|e| {
        anyhow::anyhow!("webdav_ca_file {} 中的证书无法使用: {}", path.display(), e)
    })?;
    Ok(certs)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 测试用自签 CA（`openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:P-256`，
    /// CN=minitodo-test-ca，有效期 100 年）。私钥生成后即丢弃，只用于验证解析。
    const TEST_CA_PEM: &str = "-----BEGIN CERTIFICATE-----
MIIBnDCCAUOgAwIBAgIUJUrk5+DlOKkDYUzBSXw5EnowjoUwCgYIKoZIzj0EAwIw
GzEZMBcGA1UEAwwQbWluaXRvZG8tdGVzdC1jYTAgFw0yNjEwMDIxNTQwMTRaGA8y
MTI2MDkwODE1NDAxNFowGzEZMBcGA1UEAwwQbWluaXRvZG8tdGVzdC1jYTBZMBMG
ByqGSM49AgEGCCqGSM49AwEHA0IABCVlI7h4yx6PNf14BugrNh8p3qzpSXDzGYtG
6xQ1swDmxvrhxvNhSLB8sYD3P2BRqRuWu/Ek2ilXHo+Q8rAvwGOjYzBhMB0GA1Ud
DgQWBBTGYtk8gg89AKHGW3CAIsQDLsEBpzAfBgNVHSMEGDAWgBTGYtk8gg89AKHG
W3CAIsQDLsEBpzAPBgNVHRMBAf8EBTADAQH/MA4GA1UdDwEB/wQEAwIBBjAKBggq
hkjOPQQDAgNHADBEAiBCHJq6+F0OkpVbKwmpytOcpEJJ5YUAmHE8cYgmR9LKAgIg
D1/3s6Z3SrnveaIhAkNB82i+sTQdetlnViWydQqEDRU=
-----END CERTIFICATE-----
";

    fn base(extra: &str) -> String {
        format!(
            r#"
webdav_url = "https://dav.example.com/dav/"
webdav_username = "u"
webdav_password = "p"
api_key = "0123456789abcdef0123456789abcdef"
{}
"#,
            extra
        )
    }

    #[test]
    fn minimal_config_with_defaults() {
        let c = Config::parse(&base(""), None).unwrap();
        assert_eq!(c.webdav_url, "https://dav.example.com/dav");
        assert_eq!(c.bind, "127.0.0.1:8787");
        assert_eq!(c.pull_interval_secs, 60);
        assert!(c.webdav_ca_file.is_none());
        assert!(c.webdav_ca_certs.is_empty());
        assert!(c.warnings().is_empty(), "{:?}", c.warnings());
    }

    #[test]
    fn rejects_invalid_values() {
        let short = base("").replace("0123456789abcdef0123456789abcdef", "short-key-1234");
        let err = format!("{:#}", Config::parse(&short, None).unwrap_err());
        assert!(err.contains("api_key"), "{}", err);

        for bad_url in ["ftp://dav.example.com", "not a url", "https://", ""] {
            let body = base("").replace("https://dav.example.com/dav/", bad_url);
            assert!(Config::parse(&body, None).is_err(), "{:?}", bad_url);
        }
        let body = base(r#"timezone = "Mars/Olympus""#);
        assert!(Config::parse(&body, None).is_err());
        let body = base("pull_interval = 0");
        assert!(Config::parse(&body, None).is_err());
    }

    #[test]
    fn short_api_key_and_plain_http_only_warn() {
        let body = base("")
            .replace("0123456789abcdef0123456789abcdef", "0123456789abcdef")
            .replace("https://dav.example.com", "http://dav.example.com");
        let c = Config::parse(&body, None).expect("16..31 字符的 api_key 仍然可以启动");
        let w = c.warnings();
        assert_eq!(w.len(), 2, "{:?}", w);
        assert!(w[0].contains("api_key"));
        assert!(w[1].contains("http://"));
    }

    #[test]
    fn plain_http_to_loopback_does_not_warn() {
        for host in [
            "http://127.0.0.1:8080/dav",
            "http://localhost/dav",
            "http://[::1]:8080",
            "http://webdav.localhost",
        ] {
            let body = base("").replace("https://dav.example.com/dav/", host);
            let c = Config::parse(&body, None).unwrap();
            assert!(c.warnings().is_empty(), "{} -> {:?}", host, c.warnings());
        }
    }

    #[test]
    fn ca_file_is_loaded_relative_to_config_dir() {
        let tmp = tempfile::TempDir::new().unwrap();
        std::fs::write(tmp.path().join("ca.pem"), TEST_CA_PEM).unwrap();
        let cfg_path = tmp.path().join("config.toml");
        std::fs::write(&cfg_path, base(r#"webdav_ca_file = "ca.pem""#)).unwrap();
        let c = Config::load(&cfg_path).unwrap();
        assert_eq!(
            c.webdav_ca_file.as_deref(),
            Some(tmp.path().join("ca.pem").as_path())
        );
        assert_eq!(c.webdav_ca_certs.len(), 1);
        // 证书能装进 WebDAV 客户端（阻塞客户端只能在非 async 线程里构造，这里是普通 #[test]）
        crate::sync::webdav::WebDavClient::new(&c.webdav_url, "u", "p", &c.webdav_ca_certs)
            .expect("client with extra root");
    }

    #[test]
    fn bad_ca_file_fails_fast() {
        let tmp = tempfile::TempDir::new().unwrap();
        let missing = base(&format!(
            "webdav_ca_file = {:?}",
            tmp.path().join("missing.pem")
        ));
        let err = format!("{:#}", Config::parse(&missing, None).unwrap_err());
        assert!(err.contains("webdav_ca_file"), "{}", err);

        std::fs::write(tmp.path().join("empty.pem"), "no certificates here\n").unwrap();
        let empty = base(&format!(
            "webdav_ca_file = {:?}",
            tmp.path().join("empty.pem")
        ));
        assert!(Config::parse(&empty, None).is_err());

        let garbage_pem =
            "-----BEGIN CERTIFICATE-----\nbm90IGEgY2VydGlmaWNhdGU=\n-----END CERTIFICATE-----\n";
        std::fs::write(tmp.path().join("garbage.pem"), garbage_pem).unwrap();
        let garbage = base(&format!(
            "webdav_ca_file = {:?}",
            tmp.path().join("garbage.pem")
        ));
        assert!(Config::parse(&garbage, None).is_err());
    }
}
