//! 测试用进程内 mock WebDAV 服务：axum 跑在独立线程 + 独立 current-thread runtime 上，
//! 被测的同步代码（阻塞 reqwest）可以直接在普通 `#[test]` 线程里调用。
//!
//! 支持：
//! - GET / HEAD：ETag + Last-Modified；`If-None-Match`（弱比较）/ `If-Modified-Since` → 304
//! - PUT：`If-Match`（强比较）/ `If-Unmodified-Since` → 412；父目录缺失 → 409
//! - PROPFIND Depth 0 / 1：四种 XML 风格（nginx 多行、x/net/webdav 单行、SabreDAV 小写
//!   前缀 + 绝对 URL、默认命名空间）
//! - MKCOL：已存在 405、父目录缺失 409
//!
//! 可模拟的服务端行为：
//! - Apache：修改后 `weak_etag_window` 内给弱 ETag；`put_returns_validators = false`
//!   时 PUT 响应不带 ETag / Last-Modified
//! - nginx dav / Caddy webdav：`ignore_preconditions = true` 时 PUT 忽略前置条件
//! - 故障注入（方法 + 路径片段 → 状态码）、请求延迟、"下一次 PUT sync-data 之前"的
//!   并发写入钩子

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use axum::body::{Body, Bytes};
use axum::extract::State;
use axum::http::{HeaderMap, Method, Uri};
use axum::response::Response;
use axum::Router;
use chrono::{DateTime, TimeZone, Utc};
use serde_json::Value;
use tokio::sync::oneshot;

use crate::sync::{gunzip, gzip, SYNC_DATA_FILE};

/// mock 服务挂载的根路径（客户端的 `webdav_url` = `http://addr/dav`）。
pub const ROOT: &str = "/dav";

pub struct MockFile {
    pub body: Vec<u8>,
    pub version: u64,
    /// HTTP 层面的修改时间（Last-Modified，秒级）。
    pub modified: SystemTime,
    /// 真实修改时刻（判断弱 ETag 窗口用）。
    pub modified_at: Instant,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum XmlStyle {
    NginxMultiLine,
    GoSingleLine,
    SabreLower,
    DefaultNamespace,
}

#[derive(Clone, Debug)]
pub struct LoggedRequest {
    pub method: String,
    pub path: String,
    pub if_match: Option<String>,
    pub if_none_match: Option<String>,
    pub if_unmodified_since: Option<String>,
    pub if_modified_since: Option<String>,
    pub depth: Option<String>,
    pub status: u16,
}

pub struct Fault {
    pub method: String,
    pub path_contains: String,
    pub status: u16,
    pub remaining: usize,
}

pub type Hook = Box<dyn FnOnce(&mut MockState) + Send>;

pub struct MockState {
    pub files: BTreeMap<String, MockFile>,
    pub dirs: BTreeSet<String>,
    next_version: u64,
    pub ignore_preconditions: bool,
    pub weak_etag_window: Duration,
    pub put_returns_validators: bool,
    pub xml_style: XmlStyle,
    pub log: Vec<LoggedRequest>,
    /// 收到请求时（注入延迟之前）记录的 (方法, 路径)，测试用来等"请求已经开始"。
    pub started: Vec<(String, String)>,
    pub faults: Vec<Fault>,
    /// (方法, 路径片段, 延迟)
    pub delays: Vec<(String, String, Duration)>,
    /// 下一次 PUT sync-data 之前执行一次（模拟另一个写入方抢先写入）。
    pub before_put: Option<Hook>,
}

impl Default for MockState {
    fn default() -> Self {
        MockState {
            files: BTreeMap::new(),
            dirs: BTreeSet::new(),
            next_version: 0,
            ignore_preconditions: false,
            weak_etag_window: Duration::ZERO,
            put_returns_validators: true,
            xml_style: XmlStyle::NginxMultiLine,
            log: Vec::new(),
            started: Vec::new(),
            faults: Vec::new(),
            delays: Vec::new(),
            before_put: None,
        }
    }
}

fn parent(path: &str) -> String {
    match path.trim_end_matches('/').rfind('/') {
        Some(0) | None => String::new(),
        Some(i) => path[..i].to_string(),
    }
}

fn epoch_secs(t: SystemTime) -> i64 {
    t.duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

pub fn http_date(t: SystemTime) -> String {
    Utc.timestamp_opt(epoch_secs(t), 0)
        .single()
        .expect("valid timestamp")
        .format("%a, %d %b %Y %H:%M:%S GMT")
        .to_string()
}

fn parse_http_date(s: &str) -> Option<i64> {
    DateTime::parse_from_rfc2822(s.trim())
        .ok()
        .map(|d| d.timestamp())
}

fn strip_weak(tag: &str) -> &str {
    let t = tag.trim();
    t.strip_prefix("W/")
        .or_else(|| t.strip_prefix("w/"))
        .unwrap_or(t)
}

fn is_weak(tag: &str) -> bool {
    let t = tag.trim();
    t.starts_with("W/") || t.starts_with("w/")
}

fn weak_eq(a: &str, b: &str) -> bool {
    strip_weak(a) == strip_weak(b)
}

fn strong_eq(a: &str, b: &str) -> bool {
    !is_weak(a) && !is_weak(b) && a.trim() == b.trim()
}

fn encode_path(path: &str) -> String {
    path.split('/')
        .map(|seg| urlencoding::encode(seg).into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

struct PropEntry {
    href: String,
    collection: bool,
    etag: Option<String>,
    last_modified: Option<String>,
    content_length: Option<usize>,
}

impl MockState {
    /// 直接写文件（测试准备数据 / 模拟其它写入方）。自动补齐父目录。
    pub fn write(&mut self, path: &str, body: Vec<u8>) {
        self.write_at(path, body, SystemTime::now());
    }

    /// 同 `write`，但指定 Last-Modified（模拟"之后某一秒"的写入）。
    pub fn write_at(&mut self, path: &str, body: Vec<u8>, modified: SystemTime) {
        let mut p = parent(path);
        while !p.is_empty() {
            self.dirs.insert(p.clone());
            p = parent(&p);
        }
        self.next_version += 1;
        self.files.insert(
            path.to_string(),
            MockFile {
                body,
                version: self.next_version,
                modified,
                modified_at: Instant::now(),
            },
        );
    }

    /// 像 PC 一样写 sync-data（gzip JSON）。
    pub fn write_doc(&mut self, doc: &Value) {
        self.write(SYNC_DATA_FILE, gzip(doc.to_string().as_bytes()).unwrap());
    }

    pub fn etag(&self, f: &MockFile) -> String {
        if f.modified_at.elapsed() < self.weak_etag_window {
            format!("W/\"v{}\"", f.version)
        } else {
            format!("\"v{}\"", f.version)
        }
    }

    pub fn etag_of(&self, path: &str) -> Option<String> {
        self.files.get(path).map(|f| self.etag(f))
    }

    fn take_fault(&mut self, method: &str, path: &str) -> Option<u16> {
        let i = self
            .faults
            .iter()
            .position(|f| f.method == method && path.contains(&f.path_contains))?;
        let status = self.faults[i].status;
        self.faults[i].remaining = self.faults[i].remaining.saturating_sub(1);
        if self.faults[i].remaining == 0 {
            self.faults.remove(i);
        }
        Some(status)
    }

    fn dispatch(
        &mut self,
        method: &Method,
        path: &str,
        headers: &HeaderMap,
        body: Bytes,
    ) -> (u16, Vec<(&'static str, String)>, Vec<u8>) {
        let header = |name: &str| {
            headers
                .get(name)
                .and_then(|v| v.to_str().ok())
                .map(str::to_string)
        };
        if let Some(status) = self.take_fault(method.as_str(), path) {
            return (status, vec![], vec![]);
        }
        match method.as_str() {
            "GET" | "HEAD" => {
                let Some(f) = self.files.get(path) else {
                    return (404, vec![], vec![]);
                };
                let etag = self.etag(f);
                let lm = http_date(f.modified);
                let validators = vec![("etag", etag.clone()), ("last-modified", lm)];
                if let Some(inm) = header("if-none-match") {
                    if inm.trim() == "*" || inm.split(',').any(|t| weak_eq(t, &etag)) {
                        return (304, validators, vec![]);
                    }
                } else if let Some(ims) = header("if-modified-since") {
                    if parse_http_date(&ims).is_some_and(|d| epoch_secs(f.modified) <= d) {
                        return (304, validators, vec![]);
                    }
                }
                let body = if method == Method::HEAD {
                    vec![]
                } else {
                    f.body.clone()
                };
                (200, validators, body)
            }
            "PUT" => {
                if path == SYNC_DATA_FILE {
                    if let Some(hook) = self.before_put.take() {
                        hook(self);
                    }
                }
                let par = parent(path);
                if !par.is_empty() && !self.dirs.contains(&par) {
                    return (409, vec![], vec![]);
                }
                if self.dirs.contains(path) {
                    return (405, vec![], vec![]);
                }
                if !self.ignore_preconditions {
                    if let Some(im) = header("if-match") {
                        let ok = match self.files.get(path) {
                            None => false,
                            Some(f) => {
                                let current = self.etag(f);
                                im.trim() == "*" || im.split(',').any(|t| strong_eq(t, &current))
                            }
                        };
                        if !ok {
                            return (412, vec![], vec![]);
                        }
                    }
                    if let Some(ius) = header("if-unmodified-since") {
                        if let (Some(f), Some(d)) = (self.files.get(path), parse_http_date(&ius)) {
                            if epoch_secs(f.modified) > d {
                                return (412, vec![], vec![]);
                            }
                        }
                    }
                }
                let existed = self.files.contains_key(path);
                self.write(path, body.to_vec());
                let mut out_headers = vec![];
                if self.put_returns_validators {
                    let f = &self.files[path];
                    out_headers.push(("etag", self.etag(f)));
                    out_headers.push(("last-modified", http_date(f.modified)));
                }
                (if existed { 204 } else { 201 }, out_headers, vec![])
            }
            "MKCOL" => {
                if path.is_empty() || self.dirs.contains(path) || self.files.contains_key(path) {
                    return (405, vec![], vec![]);
                }
                let par = parent(path);
                if !par.is_empty() && !self.dirs.contains(&par) {
                    return (409, vec![], vec![]);
                }
                self.dirs.insert(path.to_string());
                (201, vec![], vec![])
            }
            "PROPFIND" => {
                let depth = header("depth").unwrap_or_else(|| "infinity".to_string());
                let mut entries = Vec::new();
                if let Some(f) = self.files.get(path) {
                    entries.push(self.file_entry(path, f));
                } else if path.is_empty() || self.dirs.contains(path) {
                    entries.push(PropEntry {
                        href: format!("{}{}/", ROOT, encode_path(path)),
                        collection: true,
                        etag: None,
                        last_modified: None,
                        content_length: None,
                    });
                    if depth.trim() != "0" {
                        for d in self.dirs.iter().filter(|d| parent(d) == path) {
                            entries.push(PropEntry {
                                href: format!("{}{}/", ROOT, encode_path(d)),
                                collection: true,
                                etag: None,
                                last_modified: None,
                                content_length: None,
                            });
                        }
                        for (p, f) in self.files.iter().filter(|(p, _)| parent(p) == path) {
                            entries.push(self.file_entry(p, f));
                        }
                    }
                } else {
                    return (404, vec![], vec![]);
                }
                let xml = render_multistatus(self.xml_style, &entries);
                (
                    207,
                    vec![("content-type", "application/xml; charset=utf-8".into())],
                    xml.into_bytes(),
                )
            }
            _ => (405, vec![], vec![]),
        }
    }

    fn file_entry(&self, path: &str, f: &MockFile) -> PropEntry {
        PropEntry {
            href: format!("{}{}", ROOT, encode_path(path)),
            collection: false,
            etag: Some(self.etag(f)),
            last_modified: Some(http_date(f.modified)),
            content_length: Some(f.body.len()),
        }
    }
}

fn render_multistatus(style: XmlStyle, entries: &[PropEntry]) -> String {
    let (p, nl, open, escape_quote): (&str, &str, String, &str) = match style {
        XmlStyle::NginxMultiLine => ("D:", "\n", r#"<D:multistatus xmlns:D="DAV:">"#.into(), "\""),
        XmlStyle::GoSingleLine => (
            "D:",
            "",
            r#"<D:multistatus xmlns:D="DAV:">"#.into(),
            "&#34;",
        ),
        XmlStyle::SabreLower => (
            "d:",
            "",
            r#"<d:multistatus xmlns:d="DAV:" xmlns:s="http://sabredav.org/ns">"#.into(),
            "&quot;",
        ),
        XmlStyle::DefaultNamespace => ("", "\n", r#"<multistatus xmlns="DAV:">"#.into(), "&quot;"),
    };
    let mut out = format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>{}{}{}",
        nl, open, nl
    );
    for e in entries {
        let href = if style == XmlStyle::SabreLower {
            format!("http://localhost{}", e.href)
        } else {
            e.href.clone()
        };
        out.push_str(&format!(
            "<{p}response>{nl}<{p}href>{href}</{p}href>{nl}<{p}propstat>{nl}<{p}prop>{nl}"
        ));
        if e.collection {
            let col = if style == XmlStyle::GoSingleLine {
                r#"<D:collection xmlns:D="DAV:"/>"#.to_string()
            } else {
                format!("<{p}collection/>")
            };
            out.push_str(&format!("<{p}resourcetype>{col}</{p}resourcetype>{nl}"));
        } else {
            out.push_str(&format!("<{p}resourcetype/>{nl}"));
        }
        if let Some(etag) = &e.etag {
            out.push_str(&format!(
                "<{p}getetag>{}</{p}getetag>{nl}",
                etag.replace('"', escape_quote)
            ));
        }
        if let Some(lm) = &e.last_modified {
            out.push_str(&format!(
                "<{p}getlastmodified>{lm}</{p}getlastmodified>{nl}"
            ));
        }
        if let Some(n) = e.content_length {
            out.push_str(&format!(
                "<{p}getcontentlength>{n}</{p}getcontentlength>{nl}"
            ));
        }
        out.push_str(&format!(
            "</{p}prop>{nl}<{p}status>HTTP/1.1 200 OK</{p}status>{nl}</{p}propstat>{nl}</{p}response>{nl}"
        ));
    }
    out.push_str(&format!("</{p}multistatus>{nl}"));
    out
}

type Shared = Arc<Mutex<MockState>>;

async fn handle(
    State(st): State<Shared>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let raw = uri.path();
    let decoded = urlencoding::decode(raw)
        .map(|c| c.into_owned())
        .unwrap_or_else(|_| raw.to_string());
    let delay = {
        let mut s = st.lock().unwrap();
        s.started.push((method.to_string(), decoded.clone()));
        s.delays
            .iter()
            .find(|(m, p, _)| m == method.as_str() && decoded.contains(p.as_str()))
            .map(|(_, _, d)| *d)
    };
    if let Some(d) = delay {
        tokio::time::sleep(d).await;
    }
    let mut s = st.lock().unwrap();
    let (status, out_headers, out_body) = match decoded.strip_prefix(ROOT) {
        Some(rel) => {
            let path = rel.trim_end_matches('/').to_string();
            s.dispatch(&method, &path, &headers, body)
        }
        None => (404, vec![], vec![]),
    };
    let header = |name: &str| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string)
    };
    let logged_path = decoded
        .strip_prefix(ROOT)
        .unwrap_or(&decoded)
        .trim_end_matches('/')
        .to_string();
    s.log.push(LoggedRequest {
        method: method.to_string(),
        path: logged_path,
        if_match: header("if-match"),
        if_none_match: header("if-none-match"),
        if_unmodified_since: header("if-unmodified-since"),
        if_modified_since: header("if-modified-since"),
        depth: header("depth"),
        status,
    });
    let mut builder = Response::builder().status(status);
    for (k, v) in out_headers {
        builder = builder.header(k, v);
    }
    builder.body(Body::from(out_body)).unwrap()
}

/// 运行中的 mock 服务。drop 时通知服务线程退出。
pub struct MockDav {
    /// 给被测代码用的 `webdav_url`。
    pub base_url: String,
    state: Shared,
    shutdown: Option<oneshot::Sender<()>>,
}

impl MockDav {
    /// 预建好 `/mini-todo` 与 `/mini-todo/images` 目录。
    pub fn start() -> Self {
        Self::start_with(|s| {
            s.dirs.insert("/mini-todo".into());
            s.dirs.insert("/mini-todo/images".into());
        })
    }

    /// 空服务器（连 `/mini-todo` 都没有），测 MKCOL 用。
    pub fn start_empty() -> Self {
        Self::start_with(|_| {})
    }

    pub fn start_with(init: impl FnOnce(&mut MockState)) -> Self {
        let mut state = MockState::default();
        init(&mut state);
        let state: Shared = Arc::new(Mutex::new(state));
        let std_listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind mock dav");
        std_listener.set_nonblocking(true).unwrap();
        let addr = std_listener.local_addr().unwrap();
        let (tx, rx) = oneshot::channel::<()>();
        let shared = state.clone();
        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            rt.block_on(async move {
                let listener = tokio::net::TcpListener::from_std(std_listener).unwrap();
                let app = Router::new().fallback(handle).with_state(shared);
                let _ = axum::serve(listener, app)
                    .with_graceful_shutdown(async {
                        let _ = rx.await;
                    })
                    .await;
            });
        });
        MockDav {
            base_url: format!("http://{}{}", addr, ROOT),
            state,
            shutdown: Some(tx),
        }
    }

    pub fn with<R>(&self, f: impl FnOnce(&mut MockState) -> R) -> R {
        let mut s = self.state.lock().unwrap();
        f(&mut s)
    }

    /// 像 PC 一样写 sync-data。
    pub fn put_doc(&self, doc: &Value) {
        self.with(|s| s.write_doc(doc));
    }

    /// 读出当前 sync-data（gunzip + JSON）。
    pub fn doc(&self) -> Option<Value> {
        let bytes = self.with(|s| s.files.get(SYNC_DATA_FILE).map(|f| f.body.clone()))?;
        let json = gunzip(&bytes).ok()?;
        serde_json::from_str(&json).ok()
    }

    pub fn file(&self, path: &str) -> Option<Vec<u8>> {
        self.with(|s| s.files.get(path).map(|f| f.body.clone()))
    }

    pub fn requests(&self) -> Vec<LoggedRequest> {
        self.with(|s| s.log.clone())
    }

    pub fn clear_log(&self) {
        self.with(|s| s.log.clear());
    }

    pub fn count(&self, method: &str, path_contains: &str) -> usize {
        self.with(|s| {
            s.log
                .iter()
                .filter(|r| r.method == method && r.path.contains(path_contains))
                .count()
        })
    }

    /// 轮询等待某个请求开始（最多 `timeout`），返回是否等到。
    pub fn wait_started(&self, method: &str, path_contains: &str, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            let seen = self.with(|s| {
                s.started
                    .iter()
                    .any(|(m, p)| m == method && p.contains(path_contains))
            });
            if seen {
                return true;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        false
    }

    pub fn add_fault(&self, method: &str, path_contains: &str, status: u16, times: usize) {
        self.with(|s| {
            s.faults.push(Fault {
                method: method.to_string(),
                path_contains: path_contains.to_string(),
                status,
                remaining: times,
            })
        });
    }
}

impl Drop for MockDav {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mock_http_date_round_trips() {
        let t = UNIX_EPOCH + Duration::from_secs(1_790_000_000);
        let s = http_date(t);
        assert_eq!(s, "Mon, 21 Sep 2026 14:13:20 GMT");
        assert_eq!(parse_http_date(&s), Some(1_790_000_000));
    }

    #[test]
    fn etag_comparisons() {
        assert!(weak_eq("W/\"v1\"", "\"v1\""));
        assert!(!strong_eq("W/\"v1\"", "\"v1\""));
        assert!(strong_eq("\"v1\"", " \"v1\" "));
    }
}
