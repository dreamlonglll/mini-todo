//! 云端用的 WebDAV 客户端（reqwest blocking，全进程复用一个实例）。
//!
//! 跨端契约 K4：
//! - GET：有基准 ETag 用 `If-None-Match`，否则有 Last-Modified 用
//!   `If-Modified-Since`；304 = 自基准以来未变（响应里的新校验器会回传给调用方）
//! - PUT：前置条件由调用方选定（`If-Match` 只用于强 ETag；否则
//!   `If-Unmodified-Since`；远端不存在时不带条件），见 `sync::push::select_precondition`
//! - PUT 响应没有 ETag / Last-Modified 时（Apache 就是这样），调用方用一次
//!   `PROPFIND Depth: 0` 取校验器——不要整包 GET
//! - 只有 PUT 返回 404 / 409（父目录不存在）时才 MKCOL
//!
//! K5：`PROPFIND Depth: 1` 列目录；multistatus 解析不依赖命名空间前缀、标签大小写
//! 与换行（x/net/webdav、SabreDAV 输出单行 XML，nginx / Apache 输出多行）。
//!
//! **所有方法都会阻塞**：只能在阻塞上下文（`spawn_blocking` / 普通线程）里调用。

use std::borrow::Cow;
use std::collections::HashSet;
use std::fs;
use std::io::Write as _;
use std::path::Path;
use std::time::Duration;

use reqwest::blocking::{Client, RequestBuilder};
use reqwest::header::{
    HeaderMap, HeaderName, CONTENT_TYPE, ETAG, IF_MATCH, IF_MODIFIED_SINCE, IF_NONE_MATCH,
    IF_UNMODIFIED_SINCE, LAST_MODIFIED,
};
use reqwest::{Method, StatusCode};
use tracing::warn;

/// TCP / TLS 建连超时。
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// 普通请求（PROPFIND / MKCOL / 小文件）的总超时。
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60);
/// GET sync-data 的总超时：大小事先未知，按 8 MiB 估算。
pub const SYNC_DATA_GET_TIMEOUT: Duration = transfer_timeout(8 * 1024 * 1024);
/// 下载单张图片的总超时：按 API 允许的最大上传尺寸 32 MiB 估算。
pub const IMAGE_GET_TIMEOUT: Duration = transfer_timeout(32 * 1024 * 1024);

/// 按传输字节数放宽的总超时：基础 60s + 按 64 KiB/s 慢速链路折算，上限 30 分钟。
pub const fn transfer_timeout(bytes: u64) -> Duration {
    let secs = 60 + bytes / (64 * 1024);
    Duration::from_secs(if secs > 1800 { 1800 } else { secs })
}

/// 远端某个版本的校验器。两者都可能缺失。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Validators {
    pub etag: Option<String>,
    pub last_modified: Option<String>,
}

impl Validators {
    pub fn is_empty(&self) -> bool {
        self.etag.is_none() && self.last_modified.is_none()
    }

    fn from_headers(headers: &HeaderMap) -> Self {
        Validators {
            etag: header_string(headers, &ETAG),
            last_modified: header_string(headers, &LAST_MODIFIED),
        }
    }
}

fn header_string(headers: &HeaderMap, name: &HeaderName) -> Option<String> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn non_empty(v: &Option<String>) -> Option<&str> {
    v.as_deref().map(str::trim).filter(|s| !s.is_empty())
}

/// 强 ETag 才能用于 `If-Match`（RFC 7232 强比较：任何一方是弱 ETag 都不匹配）。
/// Apache 对一秒内刚修改的文件返回 `W/"…"`，拿它做 `If-Match` 必然 412。
pub fn is_strong_etag(etag: &str) -> bool {
    let e = etag.trim();
    !e.is_empty() && !e.starts_with("W/") && !e.starts_with("w/")
}

/// 条件 GET 的结果。
#[derive(Debug)]
pub enum GetOutcome {
    /// 304：与请求里的校验器对应的版本相同。附带响应里的（可能更新的）校验器。
    NotModified(Validators),
    /// 404：远端文件不存在。
    NotFound,
    /// 200：拿到完整内容。
    Fetched {
        body: Vec<u8>,
        validators: Validators,
    },
}

/// PUT 的前置条件。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Precondition<'a> {
    Unconditional,
    IfMatch(&'a str),
    IfUnmodifiedSince(&'a str),
}

/// PUT 的结果。412 / 404 / 409 不算错误，由调用方决定如何恢复。
#[derive(Debug, PartialEq, Eq)]
pub enum PutOutcome {
    /// 2xx。附带响应头里的校验器（可能为空）。
    Stored(Validators),
    /// 412：远端已被别的写入方修改。
    PreconditionFailed,
    /// 404 / 409：父目录不存在。
    ParentMissing(u16),
}

/// multistatus 里的一条 `<response>`。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DavEntry {
    pub href: String,
    pub is_collection: bool,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub content_length: Option<u64>,
}

const PROPFIND_META_BODY: &str = r#"<?xml version="1.0" encoding="utf-8"?><D:propfind xmlns:D="DAV:"><D:prop><D:getetag/><D:getlastmodified/><D:getcontentlength/><D:resourcetype/></D:prop></D:propfind>"#;
const PROPFIND_LIST_BODY: &str = r#"<?xml version="1.0" encoding="utf-8"?><D:propfind xmlns:D="DAV:"><D:prop><D:resourcetype/></D:prop></D:propfind>"#;

pub struct WebDavClient {
    http: Client,
    base_url: String,
    username: String,
    password: String,
}

impl WebDavClient {
    /// 构造客户端。**必须在阻塞上下文里调用**：reqwest blocking 客户端构造时会
    /// 等待内部运行时线程启动，debug 构建在 async 上下文里调用会 panic。
    pub fn new(base_url: &str, username: &str, password: &str) -> anyhow::Result<Self> {
        let http = Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(DEFAULT_TIMEOUT)
            .build()
            .map_err(|e| anyhow::anyhow!("初始化 reqwest 客户端失败: {}", e))?;
        Ok(Self {
            http,
            base_url: base_url.trim_end_matches('/').to_string(),
            username: username.to_string(),
            password: password.to_string(),
        })
    }

    fn full_url(&self, path: &str) -> String {
        format!("{}/{}", self.base_url, path.trim_start_matches('/'))
    }

    fn request(&self, method: Method, path: &str) -> RequestBuilder {
        self.http
            .request(method, self.full_url(path))
            .basic_auth(&self.username, Some(&self.password))
    }

    /// 条件 GET（sync-data 用）。`cond` 为空时是普通 GET。
    pub fn get_conditional(&self, path: &str, cond: &Validators) -> anyhow::Result<GetOutcome> {
        let mut req = self
            .request(Method::GET, path)
            .timeout(SYNC_DATA_GET_TIMEOUT);
        if let Some(etag) = non_empty(&cond.etag) {
            // If-None-Match 是弱比较，弱 ETag 也能正确得到 304
            req = req.header(IF_NONE_MATCH, etag);
        } else if let Some(lm) = non_empty(&cond.last_modified) {
            req = req.header(IF_MODIFIED_SINCE, lm);
        }
        let resp = req
            .send()
            .map_err(|e| anyhow::anyhow!("WebDAV GET {} 失败: {}", path, e))?;
        match resp.status() {
            StatusCode::NOT_MODIFIED => Ok(GetOutcome::NotModified(Validators::from_headers(
                resp.headers(),
            ))),
            StatusCode::NOT_FOUND => Ok(GetOutcome::NotFound),
            StatusCode::OK => {
                let validators = Validators::from_headers(resp.headers());
                let body = resp
                    .bytes()
                    .map_err(|e| anyhow::anyhow!("读取 WebDAV GET {} 响应体失败: {}", path, e))?
                    .to_vec();
                Ok(GetOutcome::Fetched { body, validators })
            }
            other => anyhow::bail!("WebDAV GET {} 返回状态 {}", path, other.as_u16()),
        }
    }

    /// PUT。总超时按 body 大小放宽。
    pub fn put(
        &self,
        path: &str,
        body: &[u8],
        content_type: &str,
        pre: Precondition<'_>,
    ) -> anyhow::Result<PutOutcome> {
        let mut req = self
            .request(Method::PUT, path)
            .timeout(transfer_timeout(body.len() as u64))
            .header(CONTENT_TYPE, content_type);
        req = match pre {
            Precondition::Unconditional => req,
            Precondition::IfMatch(etag) => req.header(IF_MATCH, etag),
            Precondition::IfUnmodifiedSince(lm) => req.header(IF_UNMODIFIED_SINCE, lm),
        };
        let resp = req
            .body(body.to_vec())
            .send()
            .map_err(|e| anyhow::anyhow!("WebDAV PUT {} 失败: {}", path, e))?;
        let status = resp.status().as_u16();
        match status {
            200..=299 => Ok(PutOutcome::Stored(Validators::from_headers(resp.headers()))),
            412 => Ok(PutOutcome::PreconditionFailed),
            404 | 409 => Ok(PutOutcome::ParentMissing(status)),
            other => anyhow::bail!("WebDAV PUT {} 返回状态 {}", path, other),
        }
    }

    fn propfind(
        &self,
        path: &str,
        depth: &'static str,
        body: &'static str,
    ) -> anyhow::Result<Option<String>> {
        let method = Method::from_bytes(b"PROPFIND").expect("PROPFIND is a valid method");
        let resp = self
            .request(method, path)
            .header("Depth", depth)
            .header(CONTENT_TYPE, "application/xml; charset=utf-8")
            .body(body)
            .send()
            .map_err(|e| anyhow::anyhow!("PROPFIND {} 失败: {}", path, e))?;
        match resp.status().as_u16() {
            404 => Ok(None),
            200 | 207 => Ok(Some(resp.text().map_err(|e| {
                anyhow::anyhow!("读取 PROPFIND {} 响应失败: {}", path, e)
            })?)),
            other => anyhow::bail!("PROPFIND {} 返回状态 {}", path, other),
        }
    }

    /// `PROPFIND Depth: 0`：取单个资源的 getetag / getlastmodified / getcontentlength。
    /// 资源不存在返回 `Ok(None)`。
    pub fn propfind_meta(&self, path: &str) -> anyhow::Result<Option<DavEntry>> {
        Ok(self
            .propfind(path, "0", PROPFIND_META_BODY)?
            .and_then(|xml| parse_multistatus(&xml).into_iter().next()))
    }

    /// `PROPFIND Depth: 1`：列出 `dir` 下的文件名（URL 解码后的最后一段，跳过目录）。
    /// 目录不存在返回空列表。文件名安全校验由调用方负责。
    pub fn list_names(&self, dir: &str) -> anyhow::Result<Vec<String>> {
        let path = format!("{}/", dir.trim_end_matches('/'));
        Ok(match self.propfind(&path, "1", PROPFIND_LIST_BODY)? {
            None => Vec::new(),
            Some(xml) => file_names_in_listing(&xml, dir),
        })
    }

    /// 逐级 MKCOL 创建 `path`。已存在（405）视为成功；其它异常状态只记日志，
    /// 真正的失败会在随后的 PUT 上暴露。
    pub fn ensure_dir(&self, path: &str) -> anyhow::Result<()> {
        let method = Method::from_bytes(b"MKCOL").expect("MKCOL is a valid method");
        let mut current = String::new();
        for part in path.split('/').filter(|s| !s.is_empty()) {
            current.push('/');
            current.push_str(part);
            // nginx dav 要求 MKCOL 的 URI 以 `/` 结尾
            let target = format!("{}/", current);
            let resp = self
                .request(method.clone(), &target)
                .send()
                .map_err(|e| anyhow::anyhow!("MKCOL {} 失败: {}", target, e))?;
            let status = resp.status().as_u16();
            if !(200..300).contains(&status) && status != 405 {
                warn!(target: "minitodo_cloud::webdav", "MKCOL {} 返回状态 {}", target, status);
            }
        }
        Ok(())
    }

    /// 下载单个文件到 `local`：先写同目录临时文件再原子 rename，半截文件永远不会
    /// 以正式文件名出现。远端 404 返回 `Ok(None)`。
    pub fn download_to(&self, path: &str, local: &Path) -> anyhow::Result<Option<u64>> {
        let mut resp = self
            .request(Method::GET, path)
            .timeout(IMAGE_GET_TIMEOUT)
            .send()
            .map_err(|e| anyhow::anyhow!("WebDAV GET {} 失败: {}", path, e))?;
        match resp.status() {
            StatusCode::NOT_FOUND => return Ok(None),
            StatusCode::OK => {}
            other => anyhow::bail!("WebDAV GET {} 返回状态 {}", path, other.as_u16()),
        }
        let dir = local
            .parent()
            .ok_or_else(|| anyhow::anyhow!("本地路径 {} 没有父目录", local.display()))?;
        let file_name = local
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| anyhow::anyhow!("本地路径 {} 没有文件名", local.display()))?;
        fs::create_dir_all(dir)
            .map_err(|e| anyhow::anyhow!("创建目录 {} 失败: {}", dir.display(), e))?;
        let tmp = dir.join(format!(".{}.part", file_name));
        let written = (|| -> anyhow::Result<u64> {
            let mut f = fs::File::create(&tmp)?;
            let n = resp.copy_to(&mut f)?;
            f.flush()?;
            f.sync_all()?;
            Ok(n)
        })();
        match written {
            Ok(n) => {
                fs::rename(&tmp, local).map_err(|e| {
                    let _ = fs::remove_file(&tmp);
                    anyhow::anyhow!("写入 {} 失败: {}", local.display(), e)
                })?;
                Ok(Some(n))
            }
            Err(e) => {
                let _ = fs::remove_file(&tmp);
                Err(anyhow::anyhow!("下载 {} 失败: {:#}", path, e))
            }
        }
    }
}

// =============================================================================
// multistatus 解析
// =============================================================================

#[derive(Clone, Copy, PartialEq, Eq)]
enum Field {
    Href,
    ETag,
    LastModified,
    ContentLength,
}

fn field_of(local_name: &str) -> Option<Field> {
    match local_name {
        "href" => Some(Field::Href),
        "getetag" => Some(Field::ETag),
        "getlastmodified" => Some(Field::LastModified),
        "getcontentlength" => Some(Field::ContentLength),
        _ => None,
    }
}

/// 解析 WebDAV multistatus 响应，返回每个 `<response>` 的 href / 是否集合 /
/// getetag / getlastmodified / getcontentlength。
///
/// 只认元素的本地名（去掉任意命名空间前缀、大小写不敏感），不依赖换行：
/// `<D:href>`、`<d:href>`、`<href xmlns="DAV:">`、`<lp1:getetag>` 都能识别；
/// 文本中的 XML 实体（含 x/net/webdav 输出的 `&#34;`）会被解码，CDATA 原样保留。
pub fn parse_multistatus(xml: &str) -> Vec<DavEntry> {
    let mut entries = Vec::new();
    let mut current: Option<DavEntry> = None;
    let mut capture: Option<(Field, String)> = None;
    let mut rest = xml;

    while let Some(lt) = rest.find('<') {
        if let Some((_, buf)) = capture.as_mut() {
            buf.push_str(&decode_entities(&rest[..lt]));
        }
        let tail = &rest[lt..];
        if let Some(body) = tail.strip_prefix("<!--") {
            match body.find("-->") {
                Some(end) => {
                    rest = &body[end + 3..];
                    continue;
                }
                None => break,
            }
        }
        if let Some(body) = tail.strip_prefix("<![CDATA[") {
            match body.find("]]>") {
                Some(end) => {
                    if let Some((_, buf)) = capture.as_mut() {
                        buf.push_str(&body[..end]);
                    }
                    rest = &body[end + 3..];
                    continue;
                }
                None => break,
            }
        }
        let Some(end) = tag_end(tail) else { break };
        let inner = &tail[1..end];
        rest = &tail[end + 1..];
        if inner.starts_with('?') || inner.starts_with('!') {
            continue;
        }
        let (closing, inner) = match inner.strip_prefix('/') {
            Some(i) => (true, i),
            None => (false, inner),
        };
        let self_closing = !closing && inner.trim_end().ends_with('/');
        let name = inner
            .trim_end_matches('/')
            .split(|c: char| c.is_whitespace())
            .next()
            .unwrap_or("");
        let local = name.rsplit(':').next().unwrap_or(name).to_ascii_lowercase();

        if closing {
            if local == "response" {
                if let Some(entry) = current.take() {
                    entries.push(entry);
                }
                capture = None;
            } else if let Some(field) = field_of(&local) {
                let matches = capture.as_ref().is_some_and(|(f, _)| *f == field);
                if let (true, Some((_, buf)), Some(entry)) =
                    (matches, capture.take(), current.as_mut())
                {
                    assign_field(entry, field, buf.trim());
                }
            }
            continue;
        }

        match local.as_str() {
            "response" if !self_closing => {
                current = Some(DavEntry::default());
                capture = None;
            }
            "collection" => {
                if let Some(entry) = current.as_mut() {
                    entry.is_collection = true;
                }
            }
            other => {
                if let (Some(field), false, true) =
                    (field_of(other), self_closing, current.is_some())
                {
                    capture = Some((field, String::new()));
                }
            }
        }
    }
    entries
}

fn assign_field(entry: &mut DavEntry, field: Field, value: &str) {
    if value.is_empty() {
        return;
    }
    match field {
        Field::Href if entry.href.is_empty() => entry.href = value.to_string(),
        Field::ETag if entry.etag.is_none() => entry.etag = Some(value.to_string()),
        Field::LastModified if entry.last_modified.is_none() => {
            entry.last_modified = Some(value.to_string())
        }
        Field::ContentLength if entry.content_length.is_none() => {
            entry.content_length = value.parse().ok()
        }
        _ => {}
    }
}

/// 找到结束当前标签的 `>`（跳过属性值里的引号内容）。
fn tag_end(s: &str) -> Option<usize> {
    let mut quote: Option<u8> = None;
    for (i, b) in s.bytes().enumerate().skip(1) {
        match quote {
            Some(q) if b == q => quote = None,
            Some(_) => {}
            None => match b {
                b'"' | b'\'' => quote = Some(b),
                b'>' => return Some(i),
                _ => {}
            },
        }
    }
    None
}

/// 解码 XML 预定义实体与数字字符引用；不认识的实体原样保留。
fn decode_entities(s: &str) -> Cow<'_, str> {
    if !s.contains('&') {
        return Cow::Borrowed(s);
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let after = &rest[amp + 1..];
        let decoded = after.find(';').filter(|&semi| semi <= 10).and_then(|semi| {
            let ent = &after[..semi];
            let ch = match ent {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                _ => {
                    if let Some(hex) = ent.strip_prefix("#x").or_else(|| ent.strip_prefix("#X")) {
                        u32::from_str_radix(hex, 16).ok().and_then(char::from_u32)
                    } else if let Some(dec) = ent.strip_prefix('#') {
                        dec.parse::<u32>().ok().and_then(char::from_u32)
                    } else {
                        None
                    }
                }
            };
            ch.map(|c| (c, semi))
        });
        match decoded {
            Some((c, semi)) => {
                out.push(c);
                rest = &after[semi + 1..];
            }
            None => {
                out.push('&');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    Cow::Owned(out)
}

/// href 可能是绝对路径（`/dav/mini-todo/images/a.png`）或绝对 URL
/// （`https://host/remote.php/dav/files/u/mini-todo/images/a.png`），只取路径部分。
fn href_path(href: &str) -> &str {
    let path = match href.find("://") {
        Some(i) => {
            let after = &href[i + 3..];
            match after.find('/') {
                Some(j) => &after[j..],
                None => "/",
            }
        }
        None => href,
    };
    path.split(['?', '#']).next().unwrap_or(path)
}

/// 从 `PROPFIND Depth: 1` 的响应里取出 `dir` 下的文件名（URL 解码后的最后一段），
/// 跳过目录本身与子目录，去重并保持出现顺序。
pub fn file_names_in_listing(xml: &str, dir: &str) -> Vec<String> {
    let dir_tail = dir.trim_matches('/');
    let dir_suffix = format!("/{}", dir_tail);
    let mut seen = HashSet::new();
    let mut names = Vec::new();
    for entry in parse_multistatus(xml) {
        if entry.is_collection {
            continue;
        }
        let Ok(decoded) = urlencoding::decode(href_path(&entry.href)) else {
            continue;
        };
        if decoded.ends_with('/') {
            continue;
        }
        let rel = decoded.trim_start_matches('/');
        if rel.is_empty() || rel == dir_tail || decoded.ends_with(&dir_suffix) {
            continue;
        }
        if let Some(name) = decoded.rsplit('/').next().filter(|n| !n.is_empty()) {
            if seen.insert(name.to_string()) {
                names.push(name.to_string());
            }
        }
    }
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_nginx_dav_propfind() {
        let xml = r#"<?xml version="1.0"?>
<D:multistatus xmlns:D="DAV:">
<D:response>
  <D:href>/mini-todo/images/</D:href>
</D:response>
<D:response>
  <D:href>/mini-todo/images/foo.png</D:href>
</D:response>
<D:response>
  <D:href>/mini-todo/images/bar.jpg</D:href>
</D:response>
</D:multistatus>"#;
        let files = file_names_in_listing(xml, "/mini-todo/images");
        assert_eq!(files, vec!["foo.png", "bar.jpg"]);
    }

    /// x/net/webdav（Caddy webdav）：整个响应一行，collection 带 xmlns 属性，
    /// getetag 里的引号被编码成 `&#34;`。旧解析器按行取第一个 href，这里会一个文件都拿不到。
    #[test]
    fn parse_single_line_go_webdav() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?><D:multistatus xmlns:D="DAV:"><D:response><D:href>/dav/mini-todo/images/</D:href><D:propstat><D:prop><D:resourcetype><D:collection xmlns:D="DAV:"/></D:resourcetype></D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response><D:response><D:href>/dav/mini-todo/images/1715000000000_abc123.png</D:href><D:propstat><D:prop><D:resourcetype></D:resourcetype><D:getetag>&#34;17f0a1b2c3&#34;</D:getetag></D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response><D:response><D:href>/dav/mini-todo/images/img_1_2.jpg</D:href><D:propstat><D:prop><D:resourcetype></D:resourcetype></D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response></D:multistatus>"#;
        assert_eq!(
            file_names_in_listing(xml, "/mini-todo/images"),
            vec!["1715000000000_abc123.png", "img_1_2.jpg"]
        );
        let entries = parse_multistatus(xml);
        assert_eq!(entries.len(), 3);
        assert!(entries[0].is_collection);
        assert_eq!(entries[1].etag.as_deref(), Some("\"17f0a1b2c3\""));
    }

    /// SabreDAV / Nextcloud：小写 `d:` 前缀、绝对路径 href、单行。
    #[test]
    fn parse_lowercase_prefix_and_absolute_url() {
        let xml = r#"<?xml version="1.0"?><d:multistatus xmlns:d="DAV:" xmlns:s="http://sabredav.org/ns"><d:response><d:href>/remote.php/dav/files/u/mini-todo/images/</d:href><d:propstat><d:prop><d:resourcetype><d:collection/></d:resourcetype></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response><d:response><d:href>https://cloud.example.com/remote.php/dav/files/u/mini-todo/images/a.png</d:href><d:propstat><d:prop><d:resourcetype/></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response></d:multistatus>"#;
        assert_eq!(
            file_names_in_listing(xml, "/mini-todo/images"),
            vec!["a.png"]
        );
    }

    #[test]
    fn parse_default_namespace_and_other_prefixes() {
        let xml = r#"<multistatus xmlns="DAV:">
  <response><href>/mini-todo/images</href><propstat><prop><resourcetype><collection/></resourcetype></prop></propstat></response>
  <response><href>/mini-todo/images/x.webp</href><propstat><prop><resourcetype/></prop></propstat></response>
</multistatus>
<ns0:multistatus xmlns:ns0="DAV:"><ns0:response><ns0:HREF>/mini-todo/images/y.gif</ns0:HREF></ns0:response></ns0:multistatus>"#;
        assert_eq!(
            file_names_in_listing(xml, "/mini-todo/images"),
            vec!["x.webp", "y.gif"]
        );
    }

    #[test]
    fn parse_url_encoded_names_and_skips_dir_without_slash() {
        let xml = r#"<D:multistatus xmlns:D="DAV:">
<D:response><D:href>/dav/mini-todo/images</D:href></D:response>
<D:response><D:href>/dav/mini-todo/images/a%20b.png</D:href></D:response>
<D:response><D:href>/dav/mini-todo/images/%E5%9B%BE.png</D:href></D:response>
<D:response><D:href>/dav/mini-todo/images/sub/</D:href></D:response>
<D:response><D:href>/dav/mini-todo/images/c.png?x=1</D:href></D:response>
<D:response><D:href>/dav/mini-todo/images/c.png</D:href></D:response>
</D:multistatus>"#;
        // 解码后原样返回；是否"安全"由调用方按 K5 过滤
        assert_eq!(
            file_names_in_listing(xml, "/mini-todo/images"),
            vec!["a b.png", "图.png", "c.png"]
        );
    }

    /// Apache：可选属性缺失时会多一个 404 propstat，里面是空元素；不能覆盖已解析的值。
    #[test]
    fn parse_depth0_meta_with_apache_404_propstat() {
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<D:multistatus xmlns:D="DAV:" xmlns:ns0="DAV:">
<D:response xmlns:lp1="DAV:" xmlns:lp2="http://apache.org/dav/props/">
<D:href>/dav/mini-todo/sync-data.json.gz</D:href>
<D:propstat>
<D:prop>
<lp1:getetag>W/"3-65cdc1d2"</lp1:getetag>
<lp1:getlastmodified>Fri, 02 Oct 2026 08:00:00 GMT</lp1:getlastmodified>
<lp1:getcontentlength>1234</lp1:getcontentlength>
<lp1:resourcetype/>
</D:prop>
<D:status>HTTP/1.1 200 OK</D:status>
</D:propstat>
<D:propstat>
<D:prop>
<D:getetag></D:getetag>
</D:prop>
<D:status>HTTP/1.1 404 Not Found</D:status>
</D:propstat>
</D:response>
</D:multistatus>"#;
        let entries = parse_multistatus(xml);
        assert_eq!(entries.len(), 1);
        let e = &entries[0];
        assert!(!e.is_collection);
        assert_eq!(e.etag.as_deref(), Some("W/\"3-65cdc1d2\""));
        assert_eq!(
            e.last_modified.as_deref(),
            Some("Fri, 02 Oct 2026 08:00:00 GMT")
        );
        assert_eq!(e.content_length, Some(1234));
    }

    #[test]
    fn parse_tolerates_comments_cdata_and_quoted_gt() {
        let xml = r#"<!-- a > b --><D:multistatus xmlns:D="DAV:"><D:response attr="x>y"><D:href><![CDATA[/i/a&b.png]]></D:href><D:getetag>&quot;e&amp;1&quot;</D:getetag></D:response></D:multistatus>"#;
        let entries = parse_multistatus(xml);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].href, "/i/a&b.png");
        assert_eq!(entries[0].etag.as_deref(), Some("\"e&1\""));
    }

    #[test]
    fn parse_garbage_yields_nothing() {
        assert!(parse_multistatus("").is_empty());
        assert!(parse_multistatus("<html><body>502 Bad Gateway</body></html>").is_empty());
        assert!(parse_multistatus("<D:multistatus><D:response><D:href>/x").is_empty());
    }

    #[test]
    fn decode_entities_handles_numeric_and_unknown() {
        assert_eq!(decode_entities("&#34;a&#x22;"), "\"a\"");
        assert_eq!(decode_entities("a &unknown; b & c"), "a &unknown; b & c");
        assert_eq!(decode_entities("plain"), "plain");
    }

    #[test]
    fn strong_etag_detection() {
        assert!(is_strong_etag("\"abc\""));
        assert!(is_strong_etag("abc"));
        assert!(!is_strong_etag("W/\"abc\""));
        assert!(!is_strong_etag("w/\"abc\""));
        assert!(!is_strong_etag("  "));
    }

    #[test]
    fn transfer_timeout_scales_with_size() {
        assert_eq!(transfer_timeout(0), Duration::from_secs(60));
        assert_eq!(transfer_timeout(64 * 1024 * 10), Duration::from_secs(70));
        assert_eq!(transfer_timeout(u64::MAX), Duration::from_secs(1800));
    }
}
