//! WebDAV 客户端（K4 / K5）。
//!
//! - 超时：连接 10s、单请求总计 60s（图片按大小放宽）；不跟随重定向（PUT 被 301/302
//!   改写成 GET 会被误判为上传成功）
//! - 条件 GET：只用 `If-None-Match`（服务端按弱比较判断，基准是弱 ETag 也可以）；304 → 未变。
//!   **不用** `If-Modified-Since`：nginx dav 按整秒比较，与基准同一秒内的写入会被判成 304，
//!   变更就此漏掉。没有 ETag 基准时做无条件 GET
//! - 条件 PUT：只用 `If-Match: "<opaque>"`（去掉 `W/` 前缀）；没有 ETag 基准时不带前置条件，
//!   靠"写前先 GET 合并"保护。**绝不**发 `If-Unmodified-Since`：Apache mod_dav 拿亚秒级的
//!   mtime 和秒级的日期比较，对两秒前就没再动过的文件也返回 412。
//!   Apache 刚写完的一秒内只给弱 ETag，此时 `If-Match` 也会 412，调用方隔 ≥1.1s 重试即可。
//!   412 → [`PutOutcome::PreconditionFailed`]，404 / 409（父目录不存在）→ [`PutOutcome::ParentMissing`]
//! - 有的服务端（nginx dav、Caddy/x/net/webdav）根本不检查条件头，调用方必须"写前先 GET 合并"，
//!   不能只指望 412
//! - PUT 响应常常不带校验器（Apache、nginx）：先 HEAD 取 ETag（nginx 只有 HEAD 给 ETag），
//!   再退化为 PROPFIND Depth 0 的 getetag（Apache 的 PROPFIND 给强 ETag）
//! - PROPFIND：Depth 0 取 getetag / getlastmodified / getcontentlength；Depth 1 列目录，
//!   XML 解析不依赖命名空间前缀、大小写与换行

use reqwest::blocking::{Client, RequestBuilder, Response};
use reqwest::header::{CONTENT_LENGTH, CONTENT_TYPE, ETAG, LAST_MODIFIED};
use reqwest::{Method, StatusCode};
use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
/// 图片上传按 50KB/s 估算的最长耗时上限
const MAX_TRANSFER_TIMEOUT: Duration = Duration::from_secs(600);
const MIN_TRANSFER_BYTES_PER_SEC: u64 = 50 * 1024;

const PROPFIND_BODY: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<D:propfind xmlns:D="DAV:"><D:prop><D:resourcetype/><D:getetag/><D:getlastmodified/><D:getcontentlength/></D:prop></D:propfind>"#;

/// 远端文件版本（条件请求的基准）。条件请求只用 `etag`；`last_modified` 只作记录。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RemoteVersion {
    pub etag: Option<String>,
    pub last_modified: Option<String>,
}

/// PROPFIND Depth 0 结果
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PropMeta {
    pub version: RemoteVersion,
    pub content_length: Option<u64>,
}

/// GET 结果
#[derive(Debug)]
pub enum GetOutcome {
    /// 304：自基准版本以来未变化
    NotModified,
    /// 404
    NotFound,
    Found {
        body: Vec<u8>,
        version: RemoteVersion,
    },
}

/// PUT 结果
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PutOutcome {
    /// 写入成功；版本取自响应头（可能为空，调用方可再 PROPFIND）
    Ok(RemoteVersion),
    /// 412：远端已被其它写入方修改
    PreconditionFailed,
    /// 404 / 409：父目录不存在
    ParentMissing,
}

/// PUT 前置条件。没有 `If-Unmodified-Since` 这一项是有意的（见模块文档）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Precondition {
    None,
    /// `If-Match` 的值：去掉 `W/` 前缀后的带引号 opaque-tag
    IfMatch(String),
}

pub fn is_weak_etag(etag: &str) -> bool {
    etag.starts_with("W/") || etag.starts_with("w/")
}

/// ETag 的 opaque-tag 部分：去掉弱标记 `W/` 与首尾空白（`W/"abc"` → `"abc"`）
pub fn opaque_etag(etag: &str) -> &str {
    let etag = etag.trim();
    match etag.get(..2) {
        Some("W/") | Some("w/") => etag[2..].trim_start(),
        _ => etag,
    }
}

/// K4：按基准选择 PUT 前置条件。知道 ETag（强弱都行）→ `If-Match: "<opaque>"`；
/// 不知道 → 不带条件。Last-Modified 不参与。
///
/// 弱 ETag 也去掉 `W/` 发强比较：Apache 只在文件刚写完的一秒内给弱 ETag，过了这一秒同一个
/// opaque-tag 就能强匹配；若恰好落在这一秒内会得到 412，调用方隔 ≥1.1s 重新 GET 合并后重试。
pub fn choose_put_precondition(base: &RemoteVersion) -> Precondition {
    match base.etag.as_deref().map(opaque_etag) {
        Some(tag) if !tag.is_empty() => Precondition::IfMatch(tag.to_string()),
        _ => Precondition::None,
    }
}

pub struct WebDavClient {
    client: Client,
    base_url: String,
    username: String,
    password: String,
}

impl WebDavClient {
    pub fn new(base_url: &str, username: &str, password: &str) -> Result<Self, String> {
        let base_url = base_url.trim().trim_end_matches('/').to_string();
        if base_url.is_empty() {
            return Err("未配置 WebDAV 服务器".to_string());
        }
        let lower = base_url.to_ascii_lowercase();
        if !(lower.starts_with("https://") || lower.starts_with("http://")) {
            return Err("WebDAV 地址必须以 http:// 或 https:// 开头".to_string());
        }
        if lower.starts_with("http://") && first_plaintext_warning(&PLAINTEXT_WARNED, &base_url) {
            log::warn!("[webdav] 警告：使用明文 http:// 连接 WebDAV，密码与数据可能被窃听");
        }

        let client = Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(REQUEST_TIMEOUT)
            .redirect(reqwest::redirect::Policy::none())
            .user_agent(concat!("mini-todo/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| format!("创建 HTTP 客户端失败: {}", e))?;

        Ok(Self {
            client,
            base_url,
            username: username.to_string(),
            password: password.to_string(),
        })
    }

    fn full_url(&self, path: &str) -> String {
        format!("{}/{}", self.base_url, path.trim_start_matches('/'))
    }

    /// 集合（目录）URL 统一带结尾斜杠：nginx 的 MKCOL、Apache 的 PROPFIND 对此敏感
    fn collection_url(&self, path: &str) -> String {
        let url = self.full_url(path);
        if url.ends_with('/') {
            url
        } else {
            format!("{}/", url)
        }
    }

    fn request(&self, method: Method, url: &str) -> RequestBuilder {
        self.client
            .request(method, url)
            .basic_auth(&self.username, Some(&self.password))
    }

    fn send(builder: RequestBuilder, action: &str) -> Result<Response, String> {
        builder
            .send()
            .map_err(|e| format!("{}失败: {}", action, describe_error(e)))
    }

    pub fn test_connection(&self) -> Result<bool, String> {
        let resp = Self::send(
            self.request(propfind(), &self.collection_url("/"))
                .header("Depth", "0")
                .header(CONTENT_TYPE, "application/xml; charset=utf-8")
                .body(PROPFIND_BODY),
            "连接",
        )?;
        match resp.status().as_u16() {
            200 | 207 | 301 | 302 => Ok(true),
            401 => Err("认证失败，请检查用户名和密码".to_string()),
            status => Err(format!("服务器返回状态码: {}", status)),
        }
    }

    /// 逐级 MKCOL 创建目录（已存在视为成功）
    pub fn ensure_dir(&self, path: &str) -> Result<(), String> {
        let mut current = String::new();
        for part in path.split('/').filter(|s| !s.is_empty()) {
            current.push('/');
            current.push_str(part);
            let resp = Self::send(
                self.request(mkcol(), &self.collection_url(&current)),
                "创建目录",
            )?;
            match resp.status().as_u16() {
                // 201 新建；405 已存在；200/204/301/302 部分服务端对已存在目录的回应
                200 | 201 | 204 | 301 | 302 | 405 => {}
                401 => return Err("认证失败，请检查用户名和密码".to_string()),
                status => return Err(format!("创建目录 {} 失败，状态码: {}", current, status)),
            }
        }
        Ok(())
    }

    /// 文件是否存在（HEAD）
    pub fn exists(&self, remote_path: &str) -> Result<bool, String> {
        let resp = Self::send(
            self.request(Method::HEAD, &self.full_url(remote_path)),
            "检查文件",
        )?;
        match resp.status() {
            s if s.is_success() => Ok(true),
            StatusCode::NOT_FOUND => Ok(false),
            s => Err(format!("检查文件失败，状态码: {}", s.as_u16())),
        }
    }

    /// GET；`base` 里有 ETag 时带 `If-None-Match`，未变化返回 [`GetOutcome::NotModified`]。
    /// 只有 Last-Modified 的基准不带条件（不用 `If-Modified-Since`，见模块文档）。
    pub fn get(
        &self,
        remote_path: &str,
        base: Option<&RemoteVersion>,
    ) -> Result<GetOutcome, String> {
        let mut req = self.request(Method::GET, &self.full_url(remote_path));
        if let Some(etag) = base
            .and_then(|b| b.etag.as_deref())
            .map(str::trim)
            .filter(|e| !e.is_empty())
        {
            req = req.header("If-None-Match", etag);
        }
        let resp = Self::send(req, "下载")?;
        match resp.status().as_u16() {
            304 => Ok(GetOutcome::NotModified),
            404 => Ok(GetOutcome::NotFound),
            200 => {
                let version = version_from_headers(&resp);
                let body = resp
                    .bytes()
                    .map_err(|e| format!("读取响应失败: {}", describe_error(e)))?;
                Ok(GetOutcome::Found {
                    body: body.to_vec(),
                    version,
                })
            }
            401 => Err("认证失败，请检查用户名和密码".to_string()),
            status => Err(format!("下载失败，状态码: {}", status)),
        }
    }

    /// HEAD：取文件的 ETag / Last-Modified / Content-Length；404 → `Ok(None)`。
    /// PUT 响应没带 ETag 时用它补基准（nginx dav 只有 HEAD / GET 给 ETag）。
    pub fn head_meta(&self, remote_path: &str) -> Result<Option<PropMeta>, String> {
        let resp = Self::send(
            self.request(Method::HEAD, &self.full_url(remote_path)),
            "查询文件信息",
        )?;
        match resp.status() {
            s if s.is_success() => Ok(Some(PropMeta {
                version: version_from_headers(&resp),
                // HEAD 没有响应体，reqwest 的 content_length() 不可靠，直接读头
                content_length: resp
                    .headers()
                    .get(CONTENT_LENGTH)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.trim().parse().ok()),
            })),
            StatusCode::NOT_FOUND => Ok(None),
            s => Err(format!("查询文件信息失败，状态码: {}", s.as_u16())),
        }
    }

    /// PUT bytes，按 `precondition` 附条件头
    pub fn put(
        &self,
        remote_path: &str,
        data: Vec<u8>,
        content_type: &str,
        precondition: &Precondition,
    ) -> Result<PutOutcome, String> {
        let timeout = transfer_timeout(data.len() as u64);
        let mut req = self
            .request(Method::PUT, &self.full_url(remote_path))
            .header(CONTENT_TYPE, content_type)
            .timeout(timeout);
        if let Precondition::IfMatch(etag) = precondition {
            req = req.header("If-Match", etag.as_str());
        }
        let resp = Self::send(req.body(data), "上传")?;
        match resp.status().as_u16() {
            200 | 201 | 204 => Ok(PutOutcome::Ok(version_from_headers(&resp))),
            412 => Ok(PutOutcome::PreconditionFailed),
            404 | 409 => Ok(PutOutcome::ParentMissing),
            401 => Err("认证失败，请检查用户名和密码".to_string()),
            507 => Err("上传失败：WebDAV 存储空间不足".to_string()),
            status => Err(format!("上传失败，状态码: {}", status)),
        }
    }

    /// 上传本地文件（图片）；父目录不存在时创建一次后重试
    pub fn upload_file(
        &self,
        remote_path: &str,
        local_path: &Path,
        remote_dir: &str,
    ) -> Result<(), String> {
        let data = std::fs::read(local_path).map_err(|e| format!("读取文件失败: {}", e))?;
        let content_type = content_type_for(local_path);
        match self.put(remote_path, data.clone(), content_type, &Precondition::None)? {
            PutOutcome::Ok(_) => Ok(()),
            PutOutcome::ParentMissing => {
                self.ensure_dir(remote_dir)?;
                match self.put(remote_path, data, content_type, &Precondition::None)? {
                    PutOutcome::Ok(_) => Ok(()),
                    other => Err(format!("上传 {} 失败: {:?}", remote_path, other)),
                }
            }
            PutOutcome::PreconditionFailed => Err(format!("上传 {} 失败: 412", remote_path)),
        }
    }

    /// 下载远端文件到本地：先写同目录下的隐藏临时文件再改名，避免半截文件被当成已下载。
    /// 返回 `Ok(false)` 表示远端不存在。
    pub fn download_file(&self, remote_path: &str, local_path: &Path) -> Result<bool, String> {
        let resp = Self::send(
            self.request(Method::GET, &self.full_url(remote_path))
                .timeout(MAX_TRANSFER_TIMEOUT),
            "下载",
        )?;
        match resp.status().as_u16() {
            404 => return Ok(false),
            200 => {}
            status => return Err(format!("下载失败，状态码: {}", status)),
        }
        let bytes = resp
            .bytes()
            .map_err(|e| format!("读取响应失败: {}", describe_error(e)))?;

        let parent = local_path
            .parent()
            .ok_or_else(|| "无效的本地路径".to_string())?;
        std::fs::create_dir_all(parent).map_err(|e| format!("创建目录失败: {}", e))?;
        let file_name = local_path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| "无效的本地路径".to_string())?;
        // 以 '.' 开头的临时名不满足安全图片名规则，崩溃残留也不会被当成图片上传
        let temp = parent.join(format!(".{}.part", file_name));
        std::fs::write(&temp, &bytes).map_err(|e| format!("写入文件失败: {}", e))?;
        std::fs::rename(&temp, local_path).map_err(|e| {
            let _ = std::fs::remove_file(&temp);
            format!("写入文件失败: {}", e)
        })?;
        Ok(true)
    }

    /// PROPFIND Depth 0：取文件的 ETag / Last-Modified / 大小；404 → `Ok(None)`
    pub fn propfind_meta(&self, remote_path: &str) -> Result<Option<PropMeta>, String> {
        let resp = Self::send(
            self.request(propfind(), &self.full_url(remote_path))
                .header("Depth", "0")
                .header(CONTENT_TYPE, "application/xml; charset=utf-8")
                .body(PROPFIND_BODY),
            "查询文件信息",
        )?;
        match resp.status().as_u16() {
            404 => Ok(None),
            200 | 207 => {
                let body = resp
                    .text()
                    .map_err(|e| format!("读取响应失败: {}", describe_error(e)))?;
                Ok(Some(parse_propfind_meta(&body)))
            }
            status => Err(format!("查询文件信息失败，状态码: {}", status)),
        }
    }

    /// PROPFIND Depth 1：列出目录下的文件名（不含子目录与目录自身，已 URL 解码）；
    /// 目录不存在 → 空列表
    pub fn list_names(&self, remote_dir: &str) -> Result<Vec<String>, String> {
        let resp = Self::send(
            self.request(propfind(), &self.collection_url(remote_dir))
                .header("Depth", "1")
                .header(CONTENT_TYPE, "application/xml; charset=utf-8")
                .body(PROPFIND_BODY),
            "列出目录",
        )?;
        match resp.status().as_u16() {
            404 => Ok(Vec::new()),
            200 | 207 => {
                let body = resp
                    .text()
                    .map_err(|e| format!("读取响应失败: {}", describe_error(e)))?;
                Ok(parse_listing(&body, remote_dir))
            }
            status => Err(format!("列出目录失败，状态码: {}", status)),
        }
    }
}

/// 已经警告过明文 http:// 的服务器地址（进程内）
static PLAINTEXT_WARNED: Mutex<BTreeSet<String>> = Mutex::new(BTreeSet::new());

/// 每个地址每个进程只警告一次：每次同步 / 测试连接都会新建客户端，以前一次同步就刷好几条
/// 同样的警告。地址只放在内存里做去重，不写进日志（可能带着 `user:pass@`）。
fn first_plaintext_warning(warned: &Mutex<BTreeSet<String>>, base_url: &str) -> bool {
    warned
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(base_url.to_string())
}

fn propfind() -> Method {
    Method::from_bytes(b"PROPFIND").expect("合法的 HTTP 方法名")
}

fn mkcol() -> Method {
    Method::from_bytes(b"MKCOL").expect("合法的 HTTP 方法名")
}

/// 错误信息里带上超时 / 连接失败的提示（不带 URL，避免把服务器地址刷进每条提示）
fn describe_error(e: reqwest::Error) -> String {
    let kind = if e.is_timeout() {
        "请求超时"
    } else if e.is_connect() {
        "无法连接服务器"
    } else {
        "网络错误"
    };
    format!("{}（{}）", kind, e.without_url())
}

fn version_from_headers(resp: &Response) -> RemoteVersion {
    let header = |name| {
        resp.headers()
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    };
    RemoteVersion {
        etag: header(ETAG),
        last_modified: header(LAST_MODIFIED),
    }
}

/// 按大小放宽上传超时：至少 60s，按 50KB/s 估算，最多 10 分钟
fn transfer_timeout(bytes: u64) -> Duration {
    let secs = bytes / MIN_TRANSFER_BYTES_PER_SEC;
    Duration::from_secs(secs)
        .max(REQUEST_TIMEOUT)
        .min(MAX_TRANSFER_TIMEOUT)
}

fn content_type_for(path: &Path) -> &'static str {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        _ => "application/octet-stream",
    }
}

// ============================================================================
// PROPFIND XML 解析（容忍任意命名空间前缀、大小写、单行 / 多行）
// ============================================================================

/// 标签名去掉命名空间前缀后的本地名
fn local_name(tag: &str) -> &str {
    tag.rsplit(':').next().unwrap_or(tag)
}

/// 返回所有本地名为 `name`（ASCII 大小写不敏感）的元素的内部文本（原始 XML 片段）。
/// 自闭合元素返回空串。
fn find_elements<'a>(xml: &'a str, name: &str) -> Vec<&'a str> {
    let mut out = Vec::new();
    let mut pos = 0;
    while let Some(offset) = xml[pos..].find('<') {
        let start = pos + offset;
        let rest = &xml[start + 1..];
        if rest.starts_with(['/', '?', '!']) {
            pos = start + 1;
            continue;
        }
        let Some(gt) = rest.find('>') else { break };
        let open_end = start + 1 + gt + 1;
        let inside = &rest[..gt];
        let name_end = inside
            .find(|c: char| c.is_whitespace() || c == '/')
            .unwrap_or(inside.len());
        if !local_name(&inside[..name_end]).eq_ignore_ascii_case(name) {
            pos = start + 1;
            continue;
        }
        if inside.ends_with('/') {
            out.push("");
            pos = open_end;
            continue;
        }
        match find_closing(xml, open_end, name) {
            Some((content_end, after)) => {
                out.push(&xml[open_end..content_end]);
                pos = after;
            }
            None => pos = open_end,
        }
    }
    out
}

/// 从 `from` 起找 `</[prefix:]name>`，返回 (内容结束位置, 闭合标签之后的位置)
fn find_closing(xml: &str, from: usize, name: &str) -> Option<(usize, usize)> {
    let mut pos = from;
    while let Some(offset) = xml[pos..].find("</") {
        let start = pos + offset;
        let rest = &xml[start + 2..];
        let gt = rest.find('>')?;
        if local_name(rest[..gt].trim()).eq_ignore_ascii_case(name) {
            return Some((start, start + 2 + gt + 1));
        }
        pos = start + 2;
    }
    None
}

/// 解码 XML 实体（`&amp;` `&lt;` `&gt;` `&quot;` `&apos;` `&#NN;` `&#xHH;`）
fn xml_unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let after = &rest[amp..];
        let Some(semi) = after.find(';') else {
            out.push_str(after);
            return out;
        };
        let entity = &after[1..semi];
        let decoded = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ => entity
                .strip_prefix("#x")
                .or_else(|| entity.strip_prefix("#X"))
                .and_then(|hex| u32::from_str_radix(hex, 16).ok())
                .or_else(|| entity.strip_prefix('#').and_then(|d| d.parse().ok()))
                .and_then(char::from_u32),
        };
        match decoded {
            Some(c) => {
                out.push(c);
                rest = &after[semi + 1..];
            }
            None => {
                out.push('&');
                rest = &after[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn first_text(xml: &str, name: &str) -> Option<String> {
    find_elements(xml, name)
        .into_iter()
        .map(|raw| xml_unescape(raw).trim().to_string())
        .find(|s| !s.is_empty())
}

/// PROPFIND 返回的 getetag 有的不带引号，统一成 HTTP 头里的带引号形式
fn normalize_etag(raw: &str) -> String {
    let raw = raw.trim();
    if raw.starts_with('"') || is_weak_etag(raw) {
        raw.to_string()
    } else {
        format!("\"{}\"", raw)
    }
}

/// 解析 PROPFIND Depth 0 响应
pub fn parse_propfind_meta(xml: &str) -> PropMeta {
    PropMeta {
        version: RemoteVersion {
            etag: first_text(xml, "getetag").map(|e| normalize_etag(&e)),
            last_modified: first_text(xml, "getlastmodified"),
        },
        content_length: first_text(xml, "getcontentlength").and_then(|s| s.parse().ok()),
    }
}

/// href → URL 解码后的路径（绝对 URL 只取路径部分）
fn href_path(href: &str) -> String {
    let href = xml_unescape(href);
    let href = href.trim();
    let path = match href.find("://") {
        Some(scheme_end) => {
            let after = &href[scheme_end + 3..];
            match after.find('/') {
                Some(slash) => &after[slash..],
                None => "/",
            }
        }
        None => href,
    };
    let path = path.split(['?', '#']).next().unwrap_or(path);
    urlencoding::decode(path)
        .map(|c| c.into_owned())
        .unwrap_or_else(|_| path.to_string())
}

/// 解析 PROPFIND Depth 1 列表，返回目录下的文件名（跳过目录自身与子目录）
pub fn parse_listing(xml: &str, collection_path: &str) -> Vec<String> {
    let collection = collection_path.trim_matches('/').to_string();
    let entries: Vec<(String, bool)> = {
        let responses = find_elements(xml, "response");
        if responses.is_empty() {
            // 非常规服务端：没有 response 包裹，只能逐个看 href
            find_elements(xml, "href")
                .into_iter()
                .map(|h| (h.to_string(), false))
                .collect()
        } else {
            responses
                .into_iter()
                .filter_map(|resp| {
                    let href = find_elements(resp, "href").into_iter().next()?;
                    let is_collection = !find_elements(resp, "collection").is_empty();
                    Some((href.to_string(), is_collection))
                })
                .collect()
        }
    };

    let mut names = Vec::new();
    for (href, is_collection) in entries {
        if is_collection {
            continue;
        }
        let path = href_path(&href);
        if path.ends_with('/') {
            continue;
        }
        let trimmed = path.trim_matches('/');
        let is_self = !collection.is_empty()
            && (trimmed == collection || trimmed.ends_with(&format!("/{}", collection)));
        if trimmed.is_empty() || is_self {
            continue;
        }
        if let Some(name) = trimmed.rsplit('/').next() {
            if !name.is_empty() && !names.iter().any(|n| n == name) {
                names.push(name.to_string());
            }
        }
    }
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(etag: Option<&str>, lm: Option<&str>) -> RemoteVersion {
        RemoteVersion {
            etag: etag.map(str::to_string),
            last_modified: lm.map(str::to_string),
        }
    }

    #[test]
    fn strong_etag_uses_if_match() {
        assert_eq!(
            choose_put_precondition(&v(
                Some("\"3-65cdc\""),
                Some("Wed, 01 Jan 2026 00:00:00 GMT")
            )),
            Precondition::IfMatch("\"3-65cdc\"".to_string())
        );
        assert_eq!(
            choose_put_precondition(&v(Some("  \"x\"  "), None)),
            Precondition::IfMatch("\"x\"".to_string()),
            "首尾空白去掉"
        );
    }

    /// Apache 刚写完的一秒内给弱 ETag：去掉 W/ 发 If-Match（过了这一秒就能强匹配），
    /// 绝不退回 If-Unmodified-Since（Apache 拿亚秒 mtime 比较，永远 412）
    #[test]
    fn weak_etag_uses_if_match_with_opaque_tag() {
        let lm = "Wed, 01 Jan 2026 00:00:00 GMT";
        assert_eq!(
            choose_put_precondition(&v(Some("W/\"3-65cdc\""), Some(lm))),
            Precondition::IfMatch("\"3-65cdc\"".to_string())
        );
        assert_eq!(
            choose_put_precondition(&v(Some("w/\"x\""), None)),
            Precondition::IfMatch("\"x\"".to_string())
        );
    }

    /// 没有 ETag：不带前置条件（Last-Modified 不参与），靠写前 GET 合并保护
    #[test]
    fn missing_etag_sends_no_precondition() {
        let lm = "Wed, 01 Jan 2026 00:00:00 GMT";
        assert_eq!(
            choose_put_precondition(&v(None, Some(lm))),
            Precondition::None
        );
        assert_eq!(
            choose_put_precondition(&v(Some("  "), Some(lm))),
            Precondition::None
        );
        assert_eq!(
            choose_put_precondition(&v(Some("W/"), None)),
            Precondition::None
        );
        assert_eq!(
            choose_put_precondition(&RemoteVersion::default()),
            Precondition::None
        );
    }

    #[test]
    fn opaque_etag_strips_weak_marker() {
        assert_eq!(opaque_etag("W/\"abc\""), "\"abc\"");
        assert_eq!(opaque_etag("w/\"abc\""), "\"abc\"");
        assert_eq!(opaque_etag(" \"abc\" "), "\"abc\"");
        assert_eq!(
            opaque_etag("\"W/abc\""),
            "\"W/abc\"",
            "引号内的 W/ 不是弱标记"
        );
        assert_eq!(opaque_etag(""), "");
    }

    const MULTILINE: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<D:multistatus xmlns:D="DAV:">
  <D:response>
    <D:href>/dav/mini-todo/images/</D:href>
    <D:propstat><D:prop><D:resourcetype><D:collection/></D:resourcetype></D:prop>
    <D:status>HTTP/1.1 200 OK</D:status></D:propstat>
  </D:response>
  <D:response>
    <D:href>/dav/mini-todo/images/1715000000000_ab12cd.png</D:href>
    <D:propstat><D:prop><D:resourcetype/><D:getetag>"abc"</D:getetag></D:prop>
    <D:status>HTTP/1.1 200 OK</D:status></D:propstat>
  </D:response>
  <D:response>
    <D:href>/dav/mini-todo/images/sub/</D:href>
    <D:propstat><D:prop><D:resourcetype><D:collection/></D:resourcetype></D:prop></D:propstat>
  </D:response>
</D:multistatus>"#;

    #[test]
    fn listing_multiline_uppercase_prefix() {
        assert_eq!(
            parse_listing(MULTILINE, "/mini-todo/images"),
            vec!["1715000000000_ab12cd.png"]
        );
    }

    #[test]
    fn listing_single_line_lowercase_prefix() {
        // x/net/webdav（Caddy）风格：整份响应一行、小写 d: 前缀
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?><d:multistatus xmlns:d="DAV:"><d:response><d:href>/mini-todo/images/</d:href><d:propstat><d:prop><d:resourcetype><d:collection xmlns:d="DAV:"/></d:resourcetype></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response><d:response><d:href>/mini-todo/images/a.png</d:href><d:propstat><d:prop><d:resourcetype></d:resourcetype></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response><d:response><d:href>/mini-todo/images/b.jpg</d:href><d:propstat><d:prop><d:resourcetype/></d:prop></d:propstat></d:response></d:multistatus>"#;
        assert_eq!(
            parse_listing(xml, "/mini-todo/images"),
            vec!["a.png", "b.jpg"]
        );
    }

    #[test]
    fn listing_without_prefix_and_other_prefixes() {
        let xml = r#"<multistatus xmlns="DAV:"><response><href>/mini-todo/images</href><propstat><prop><resourcetype><collection/></resourcetype></prop></propstat></response><response><href>/mini-todo/images/c.webp</href></response></multistatus>"#;
        assert_eq!(parse_listing(xml, "/mini-todo/images"), vec!["c.webp"]);

        let xml = r#"<lp1:multistatus xmlns:lp1="DAV:"><lp1:Response><lp1:HREF>/mini-todo/images/d.gif</lp1:HREF></lp1:Response></lp1:multistatus>"#;
        assert_eq!(parse_listing(xml, "/mini-todo/images"), vec!["d.gif"]);
    }

    #[test]
    fn listing_decodes_urls_and_absolute_hrefs() {
        let xml = r#"<D:multistatus xmlns:D="DAV:">
<D:response><D:href>https://dav.example.com/remote.php/dav/files/u/mini-todo/images/</D:href><D:propstat><D:prop><D:resourcetype><D:collection/></D:resourcetype></D:prop></D:propstat></D:response>
<D:response><D:href>https://dav.example.com/remote.php/dav/files/u/mini-todo/images/img%5F1.png</D:href></D:response>
<D:response><D:href>/remote.php/dav/files/u/mini-todo/images/%E6%88%AA%E5%9B%BE.png</D:href></D:response>
<D:response><D:href>/remote.php/dav/files/u/mini-todo/images/a&amp;b.png</D:href></D:response>
</D:multistatus>"#;
        assert_eq!(
            parse_listing(xml, "/mini-todo/images"),
            vec!["img_1.png", "截图.png", "a&b.png"]
        );
    }

    #[test]
    fn listing_falls_back_to_bare_hrefs() {
        let xml =
            "<root><href>/mini-todo/images/</href><href>/mini-todo/images/x.png</href></root>";
        assert_eq!(parse_listing(xml, "/mini-todo/images"), vec!["x.png"]);
    }

    #[test]
    fn propfind_meta_extracts_version_and_length() {
        let xml = r#"<?xml version="1.0"?><D:multistatus xmlns:D="DAV:"><D:response><D:href>/mini-todo/sync-data.json.gz</D:href><D:propstat><D:prop><D:getetag>&quot;5-65cdc&quot;</D:getetag><D:getlastmodified>Wed, 01 Jan 2026 00:00:00 GMT</D:getlastmodified><D:getcontentlength>1234</D:getcontentlength></D:prop></D:propstat></D:response></D:multistatus>"#;
        let meta = parse_propfind_meta(xml);
        assert_eq!(meta.version.etag.as_deref(), Some("\"5-65cdc\""));
        assert_eq!(
            meta.version.last_modified.as_deref(),
            Some("Wed, 01 Jan 2026 00:00:00 GMT")
        );
        assert_eq!(meta.content_length, Some(1234));

        // 不带引号的 getetag 补引号；弱 ETag 原样
        let xml = "<d:prop><d:getetag>abc</d:getetag></d:prop>";
        assert_eq!(
            parse_propfind_meta(xml).version.etag.as_deref(),
            Some("\"abc\"")
        );
        let xml = "<d:prop><d:getetag>W/\"abc\"</d:getetag><d:getlastmodified/></d:prop>";
        let meta = parse_propfind_meta(xml);
        assert_eq!(meta.version.etag.as_deref(), Some("W/\"abc\""));
        assert_eq!(meta.version.last_modified, None);
    }

    #[test]
    fn xml_unescape_handles_entities() {
        assert_eq!(
            xml_unescape("a&amp;b&lt;c&gt;&quot;&apos;&#65;&#x42;"),
            "a&b<c>\"'AB"
        );
        assert_eq!(xml_unescape("no entities"), "no entities");
        assert_eq!(xml_unescape("dangling & amp"), "dangling & amp");
    }

    #[test]
    fn transfer_timeout_scales_with_size() {
        assert_eq!(transfer_timeout(0), REQUEST_TIMEOUT);
        assert_eq!(transfer_timeout(20 * 1024 * 1024), Duration::from_secs(409));
        assert_eq!(transfer_timeout(u64::MAX), MAX_TRANSFER_TIMEOUT);
    }

    #[test]
    fn plaintext_warning_is_logged_once_per_url() {
        let warned = Mutex::new(BTreeSet::new());
        assert!(first_plaintext_warning(&warned, "http://nas.local/dav"));
        assert!(!first_plaintext_warning(&warned, "http://nas.local/dav"));
        assert!(!first_plaintext_warning(&warned, "http://nas.local/dav"));
        assert!(first_plaintext_warning(&warned, "http://other.local/dav"));
    }

    #[test]
    fn client_rejects_bad_urls() {
        assert!(WebDavClient::new("", "u", "p").is_err());
        assert!(WebDavClient::new("ftp://x", "u", "p").is_err());
        assert!(WebDavClient::new("https://dav.example.com/", "u", "p").is_ok());
    }
}
