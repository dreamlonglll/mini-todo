//! 同步引擎测试：内存库 + 进程内 tiny_http mock WebDAV（Apache / nginx 实测语义）。

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
    fn respond(code: u16, body: Vec<u8>, headers: &[(&str, &str)]) -> Response<Cursor<Vec<u8>>> {
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

/// 按相反的键顺序手写 JSON（模拟键顺序不同的其它写入方；与 serde_json 是否开 `preserve_order` 无关）
fn reversed_keys_json(v: &Value) -> String {
    match v {
        Value::Object(map) => {
            let parts: Vec<String> = map
                .iter()
                .rev()
                .map(|(k, v)| format!("{}:{}", Value::String(k.clone()), reversed_keys_json(v)))
                .collect();
            format!("{{{}}}", parts.join(","))
        }
        Value::Array(items) => {
            let parts: Vec<String> = items.iter().map(reversed_keys_json).collect();
            format!("[{}]", parts.join(","))
        }
        other => other.to_string(),
    }
}

/// 远端文档的键顺序（其它写入方、serde_json `preserve_order` 开关——workspace 里 gpui 会打开它）不影响合并、
/// "是否需要上传"的判断与未知顶层键的保留：判断都按记录 id / updatedAt / 墓碑集合做，不比较 JSON 文本
#[test]
fn remote_key_order_does_not_affect_merge_or_upload() {
    let fx = Fixture::new("key-order");
    add_local_todo(&fx.db, 1, "本地", "2026-09-01 10:00:00");
    fx.engine().sync(false).expect("首次上传");

    let mut remote = fx.remote();
    remote["todos"]
        .as_array_mut()
        .unwrap()
        .push(todo_json(2, "远端", "2026-09-02 10:00:00"));
    remote["zzFuture"] = json!({"y": [1, {"q": 1, "p": 2}], "x": null});
    let text = reversed_keys_json(&remote);
    assert_ne!(
        text,
        serde_json::to_string(&remote).unwrap(),
        "前提：键顺序确实不同"
    );
    fx.server()
        .write(SYNC_DATA_FILE, gzip_compress(text.as_bytes()).unwrap());

    fx.clear_log();
    let report = fx.engine().sync(true).expect("合并");
    assert_eq!(report.status, SyncStatus::Pulled);
    assert_eq!(report.todos_inserted, 1);
    assert_eq!(
        local_titles(&fx.db),
        vec![(1, "本地".to_string()), (2, "远端".to_string())]
    );
    assert_eq!(
        fx.count(PUT_DOC),
        0,
        "内容已等价，只是键顺序不同：不重新上传"
    );

    // 本地修改后上传：未知顶层键按值原样带回
    fx.db
        .with_connection(|c| {
            c.execute(
                "UPDATE todos SET title = '本地改', updated_at = '2026-09-03 10:00:00' WHERE id = 1",
                [],
            )
        })
        .unwrap();
    fx.clear_log();
    let report = fx.engine().sync(false).expect("上传");
    assert_eq!(report.status, SyncStatus::Pushed);
    assert_eq!(fx.count(PUT_DOC), 1);
    let after = fx.remote();
    assert_eq!(after["zzFuture"], remote["zzFuture"]);
    assert_eq!(remote_ids(&after), vec![1, 2]);
    let first = after["todos"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == 1)
        .unwrap();
    assert_eq!(first["title"], "本地改");
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
        .with_connection(|c| set_setting(c, KEY_REMOTE_ETAG, &format!("W/{}", current)).map(|_| ()))
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

    // 同一秒内另一台设备改了标题与 updatedAt，写回的文件与原文件等长：先构造一个压缩后比原文件小的改写，
    // 再用 gzip 头（FNAME）补齐到等长。
    // 原文件里的 deviceId（随机）与文档 updatedAt（运行时刻）几乎不可压缩，改写方把这两个元信息字段换成空串
    // （它们不参与合并判断），压缩后一定比原文件小几十字节，与标题改动无关。不能指望"碰巧"压缩得更小：
    // 键顺序随 serde_json 的 `preserve_order` 特性变化（workspace 里 gpui 会打开它，Value 重新序列化时保持原顺序；
    // 单独编译 core 时按字母序重排），两种情况下压缩率不同。
    let original = fx.remote();
    let title = "bbbb".to_string();
    let mut doc = original.clone();
    doc["deviceId"] = json!("");
    doc["updatedAt"] = json!("");
    doc["todos"][0]["title"] = json!(title);
    doc["todos"][0]["updatedAt"] = json!("2026-09-02 10:00:00");
    let rewritten = gzip_padded_to(&serde_json::to_vec(&doc).unwrap(), base_len)
        .expect("去掉元信息后的改写压缩后应比原文件小");
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
    cloud["settings"] = json!({"viewMode": "quadrant", "windowSize": {"width": 1, "height": 1}});
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
    assert!(
        !text.contains("todoLists") && !text.contains("listTombstones"),
        "没有清单信息的文档原样写回也没有：不凭空造出\"远端清单为空\""
    );
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
    local1["subtasks"] = json!([{"id": 11, "title": "本地子", "updatedAt": "2026-09-01 00:00:00"}]);
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

// ------------------------------------------------------------------
// 待办清单（v31，契约 .trellis/tasks/11-09-multi-list-windows/contract.md）
// ------------------------------------------------------------------

use crate::lists;
use crate::window_prefs::{self, UpdateWindowPrefs, WindowKey};

const LIFE: i64 = 1_731_000_000_000_456;

fn list_json(id: i64, name: &str, updated_at: &str) -> Value {
    json!({"id": id, "name": name, "sortOrder": 1,
           "createdAt": "2026-01-01 00:00:00", "updatedAt": updated_at})
}

fn list_names(db: &Database) -> Vec<String> {
    lists::list(db)
        .unwrap()
        .into_iter()
        .map(|l| l.name)
        .collect()
}

fn list_ids_of(doc: &Value) -> Vec<i64> {
    let mut ids: Vec<i64> = doc["todoLists"]
        .as_array()
        .expect("todoLists 是数组")
        .iter()
        .filter_map(|l| l["id"].as_i64())
        .collect();
    ids.sort();
    ids
}

fn list_tombstone_ids(doc: &Value) -> Vec<i64> {
    let mut ids: Vec<i64> = doc["listTombstones"]
        .as_array()
        .expect("listTombstones 是数组")
        .iter()
        .map(|t| t["entityId"].as_i64().expect("entityId 是整数"))
        .collect();
    ids.sort();
    ids
}

fn todo_of(doc: &Value, id: i64) -> Value {
    doc["todos"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == id)
        .cloned()
        .unwrap_or_else(|| panic!("远端没有待办 #{id}"))
}

/// 本地存储的归属；`None` = 未知
fn local_list_id(db: &Database, todo_id: i64) -> Option<i64> {
    db.with_connection(|c| {
        c.query_row("SELECT list_id FROM todos WHERE id = ?1", [todo_id], |r| {
            r.get(0)
        })
    })
    .unwrap()
}

/// 比 `previous` 严格更新的版本时间（模拟另一台设备在之后改过）
fn later_than(previous: &Value) -> String {
    crate::db::time::superseding(&now_local(), previous.as_str().unwrap())
}

/// 上传文档带上全部清单与清单墓碑，待办带 listId；远端还没有清单信息、本机只有没改过的默认清单时不为它多传一次
#[test]
fn sync_uploads_lists_and_only_adds_list_info_when_it_carries_something() {
    let fx = Fixture::new("lists-upload");
    // 旧版本写的远端（没有 todoLists）、本机刚升级、没有本地变更：不必上传
    add_local_todo(&fx.db, 1, "本地", "2026-09-01 10:00:00");
    fx.engine().sync(false).expect("首次同步");
    let first = fx.remote();
    assert_eq!(
        first["todoLists"],
        json!([{"id": 1, "name": "待办清单", "sortOrder": 0,
                "createdAt": "2000-01-01 00:00:00", "updatedAt": "2000-01-01 00:00:00"}])
    );
    assert_eq!(first["listTombstones"], json!([]));
    assert!(
        todo_of(&first, 1).get("listId").is_none(),
        "归属未知的待办（旧版本写进库的）不写 listId"
    );
    let mut old = first.clone();
    old.as_object_mut().unwrap().remove("todoLists");
    old.as_object_mut().unwrap().remove("listTombstones");
    fx.set_remote(&old);
    fx.clear_log();
    assert_eq!(
        fx.engine().sync(false).unwrap().status,
        SyncStatus::NoChanges
    );
    assert_eq!(fx.count(PUT_DOC), 0, "默认清单的种子状态不算要补的信息");

    // 本机有清单：照常上传
    let work = lists::create(&fx.db, "工作").unwrap();
    assert_eq!(fx.engine().sync(false).unwrap().status, SyncStatus::Pushed);
    assert_eq!(list_ids_of(&fx.remote()), vec![1, work.id]);

    // 远端被不认识清单的写入方整个覆盖掉（例如旧版本对看不懂的远端强制推送）：本机没有变更也补传
    let mut lost = fx.remote();
    lost.as_object_mut().unwrap().remove("todoLists");
    lost.as_object_mut().unwrap().remove("listTombstones");
    fx.set_remote(&lost);
    fx.clear_log();
    assert_eq!(fx.engine().sync(false).unwrap().status, SyncStatus::Pushed);
    assert_eq!(list_ids_of(&fx.remote()), vec![1, work.id]);
    fx.clear_log();
    assert_eq!(
        fx.engine().sync(false).unwrap().status,
        SyncStatus::NoChanges
    );
    assert_eq!(fx.count(PUT_DOC), 0);
}

/// 清单的新建 / 改名 / 删除双向传播；同步来的删除清掉本机的窗口记录、不把清单传回去
#[test]
fn lists_and_list_deletions_sync_in_both_directions() {
    let fx = Fixture::new("lists-both-ways");
    let work = lists::create(&fx.db, "工作").unwrap();
    add_local_todo(&fx.db, 1, "工作待办", "2026-09-01 10:00:00");
    crate::todo::move_to_list(&fx.db, 1, work.id).unwrap();
    fx.engine().sync(false).unwrap();
    let remote = fx.remote();
    assert_eq!(list_ids_of(&remote), vec![1, work.id]);
    assert_eq!(todo_of(&remote, 1)["listId"], work.id);

    // 另一台设备：新建"生活"、把待办移过去、给默认清单改名
    let mut other = fx.remote();
    let renamed_at = later_than(&other["todoLists"][0]["updatedAt"]);
    other["todoLists"][0]["name"] = json!("收件箱");
    other["todoLists"][0]["updatedAt"] = json!(renamed_at);
    let mut life = list_json(LIFE, "生活", "2026-10-01 10:00:00");
    life["sortOrder"] = json!(2);
    other["todoLists"].as_array_mut().unwrap().push(life);
    let todo = other["todos"][0].clone();
    other["todos"][0]["listId"] = json!(LIFE);
    other["todos"][0]["updatedAt"] = json!(later_than(&todo["updatedAt"]));
    fx.other_device_writes(&other);
    let report = fx.engine().sync(false).unwrap();
    assert_eq!((report.lists_inserted, report.lists_updated), (1, 1));
    assert!(report.lists_changed() && report.local_changed());
    assert_eq!(report.status, SyncStatus::Pulled);
    assert_eq!(list_names(&fx.db), vec!["收件箱", "工作", "生活"]);
    assert_eq!(local_list_id(&fx.db, 1), Some(LIFE));

    // 另一台设备删除"工作"：本机随之删除，窗口记录一并清理；远端已是结果，不再上传
    window_prefs::update_prefs(
        &fx.db,
        WindowKey::List(work.id),
        &UpdateWindowPrefs {
            dark: Some(true),
            ..Default::default()
        },
    )
    .unwrap()
    .unwrap();
    let mut other = fx.remote();
    other["todoLists"]
        .as_array_mut()
        .unwrap()
        .retain(|l| l["id"] != work.id);
    other["listTombstones"] = json!([{"entityId": work.id, "deletedAt": now_local()}]);
    fx.other_device_writes(&other);
    fx.clear_log();
    let report = fx.engine().sync(false).unwrap();
    assert_eq!(report.lists_deleted, 1);
    assert!(lists::get(&fx.db, work.id).is_err());
    assert_eq!(
        window_prefs::get_prefs(&fx.db, WindowKey::List(work.id)).unwrap(),
        None,
        "同步来的清单删除：删窗口记录"
    );
    assert_eq!(fx.count(PUT_DOC), 0);

    // 本机删除"生活"：待办移到默认清单，清单墓碑随上传传播；已删的"工作"不会复活
    assert_eq!(lists::delete(&fx.db, LIFE).unwrap(), 1);
    assert_eq!(fx.engine().sync(false).unwrap().status, SyncStatus::Pushed);
    let remote = fx.remote();
    assert_eq!(list_ids_of(&remote), vec![1]);
    let mut deleted = vec![work.id, LIFE];
    deleted.sort();
    assert_eq!(list_tombstone_ids(&remote), deleted);
    assert_eq!(todo_of(&remote, 1)["listId"], 1);
    assert_eq!(
        remote["tombstones"],
        json!([]),
        "清单墓碑不进共用的 tombstones"
    );
}

/// 混跑旧版本：旧设备（≤ 3.0.x，强类型、不认识清单）同步一次——todoLists / listTombstones 经它的 extra 原样带回，
/// 清单不丢、已删清单不复活；它按自己的版本重新序列化了**全部**待办、全都丢了 listId（契约 §12：缺失 = 未知）。
/// 新设备不受影响：它改过的待办内容取它的、归属沿用本地清单；随后补传一次把远端的 listId 补回来（已知的默认清单 1 也补）
#[test]
fn old_devices_keep_list_keys_and_their_edits_keep_the_local_list() {
    #[derive(Serialize, Deserialize)]
    struct OldSyncData {
        #[serde(default)]
        todos: Vec<Value>,
        #[serde(default)]
        tombstones: Vec<Value>,
        #[serde(flatten)]
        extra: Map<String, Value>,
    }

    let fx = Fixture::new("lists-old-device");
    let work = lists::create(&fx.db, "工作").unwrap();
    let gone = lists::create(&fx.db, "已删").unwrap();
    for id in [1, 2] {
        add_local_todo(&fx.db, id, "工作待办", "2026-09-01 10:00:00");
        crate::todo::move_to_list(&fx.db, id, work.id).unwrap();
    }
    add_local_todo(&fx.db, 3, "默认清单待办", "2026-09-01 10:00:00");
    crate::todo::move_to_list(&fx.db, 3, 1).unwrap();
    lists::delete(&fx.db, gone.id).unwrap();
    fx.engine().sync(false).unwrap();
    assert_eq!(
        todo_of(&fx.remote(), 3)["listId"],
        1,
        "已知的默认清单照常写出"
    );

    // 旧设备：未知顶层键进 extra；待办按旧模型重新序列化（没有 listId），其中一条它真的改过
    let mut old: OldSyncData = serde_json::from_value(fx.remote()).unwrap();
    assert!(
        old.extra.contains_key("todoLists") && old.extra.contains_key("listTombstones"),
        "新顶层键在旧版本里是未知键"
    );
    for todo in &mut old.todos {
        todo.as_object_mut().unwrap().remove("listId");
        if todo["id"] == 2 {
            todo["title"] = json!("旧设备改的标题");
            todo["updatedAt"] = json!(later_than(&todo["updatedAt"]));
        }
    }
    fx.other_device_writes(&serde_json::to_value(&old).unwrap());

    fx.clear_log();
    let report = fx.engine().sync(false).unwrap();
    assert_eq!(report.lists_deleted, 0, "清单定义没被旧设备抹掉");
    assert_eq!(
        list_names(&fx.db),
        vec!["待办清单", "工作"],
        "已删清单没复活"
    );
    assert_eq!(
        local_list_id(&fx.db, 1),
        Some(work.id),
        "旧设备没改过：留在原清单"
    );
    assert_eq!(
        local_list_id(&fx.db, 2),
        Some(work.id),
        "旧设备改过：内容取它的，归属沿用本地清单"
    );
    assert_eq!(crate::todo::get(&fx.db, 2).unwrap().title, "旧设备改的标题");
    assert_eq!(local_list_id(&fx.db, 3), Some(1));

    // 远端丢了全部 listId：本机上传一次补回（含已知的 1）；之后不再来回
    assert_eq!(fx.count(PUT_DOC), 1);
    let remote = fx.remote();
    assert_eq!(todo_of(&remote, 1)["listId"], work.id);
    assert_eq!(todo_of(&remote, 2)["listId"], work.id);
    assert_eq!(todo_of(&remote, 3)["listId"], 1);
    assert_eq!(list_ids_of(&remote), vec![1, work.id]);
    assert_eq!(list_tombstone_ids(&remote), vec![gone.id]);
    fx.clear_log();
    assert_eq!(
        fx.engine().sync(false).unwrap().status,
        SyncStatus::NoChanges
    );
    assert_eq!(fx.count(PUT_DOC), 0);
}

/// 先跑旧版本、后升级的设备（契约 §12）：迁移后存量待办的归属是未知（NULL）、updatedAt 与远端相同，基准 ETag 被作废。
/// 升级后第一次同步是无条件 GET；平局也采纳远端的 listId，学到真实归属，且这次采纳不触发上传。之后它上传时，
/// 文档里的 listId 保持各自的真实值（不会被改成 1），两边都未知的待办仍不写 listId
#[test]
fn upgraded_old_device_learns_list_ids_without_uploading() {
    let fx = Fixture::new("lists-upgraded-device");
    // 另一台新版本设备写过远端：#1 在"生活"、#2 明确在默认清单、#3 归属未知
    let new_db = Database::new_in_memory().unwrap();
    let new_device = SyncEngine {
        db: &new_db,
        client: &fx.client,
        images_dir: fx.dir.clone(),
        device_id: "new-pc".to_string(),
        retry_delay: fx.retry_delay,
    };
    let life = lists::create(&new_db, "生活").unwrap();
    for id in 1..=3 {
        add_local_todo(&new_db, id, "待办", "2026-09-01 10:00:00");
    }
    crate::todo::move_to_list(&new_db, 1, life.id).unwrap();
    crate::todo::move_to_list(&new_db, 2, 1).unwrap();
    new_device.sync(false).unwrap();

    // 本机以前跑旧版本、同步过同一批待办（同一版本）；迁移 v31 后归属全部未知、基准 ETag 作废、没有未同步的变更
    fx.engine().sync(false).unwrap();
    fx.db
        .with_transaction(|tx| -> rusqlite::Result<()> {
            tx.execute("UPDATE todos SET list_id = NULL", [])?;
            let seq = sync_store::local_seq(tx)?;
            sync_store::set_synced_seq(tx, seq)?;
            set_setting(tx, KEY_REMOTE_ETAG, "")?;
            Ok(())
        })
        .unwrap();
    let versions_before: Vec<String> = (1..=3)
        .map(|id| crate::todo::get(&fx.db, id).unwrap().updated_at)
        .collect();
    assert!(
        !fx.engine().read_pre_state().unwrap().dirty(),
        "前提：没有未同步的变更"
    );

    fx.clear_log();
    let report = fx.engine().sync(false).unwrap();
    assert_eq!(
        fx.server().get_conditions.last(),
        Some(&(None, None)),
        "升级后第一次同步是无条件 GET"
    );
    assert_eq!(report.todos_updated, 2, "学到 #1 / #2 的归属");
    assert_eq!(report.status, SyncStatus::Pulled);
    assert_eq!(local_list_id(&fx.db, 1), Some(life.id));
    assert_eq!(local_list_id(&fx.db, 2), Some(1));
    assert_eq!(local_list_id(&fx.db, 3), None, "远端也未知：仍是未知");
    let versions_after: Vec<String> = (1..=3)
        .map(|id| crate::todo::get(&fx.db, id).unwrap().updated_at)
        .collect();
    assert_eq!(versions_after, versions_before, "采纳归属不改 updated_at");
    assert_eq!(fx.count(PUT_DOC), 0, "采纳远端本来就有的 listId 不触发上传");
    fx.clear_log();
    assert_eq!(
        fx.engine().sync(false).unwrap().status,
        SyncStatus::NoChanges
    );
    assert_eq!(fx.count(PUT_DOC), 0);

    // 之后本机改了 #3（只改标题，归属保持未知）并上传：远端的 listId 保持真实值，#3 仍不写 listId
    let mut upd: crate::db::UpdateTodoRequest = serde_json::from_value(json!({})).unwrap();
    upd.title = Some("升级后的设备改的".to_string());
    crate::todo::update(&fx.db, 3, &upd).unwrap();
    assert_eq!(fx.engine().sync(false).unwrap().status, SyncStatus::Pushed);
    let remote = fx.remote();
    assert_eq!(todo_of(&remote, 1)["listId"], life.id);
    assert_eq!(todo_of(&remote, 2)["listId"], 1);
    let third = todo_of(&remote, 3);
    assert_eq!(third["title"], "升级后的设备改的");
    assert!(third.get("listId").is_none(), "未知不写出");
}

/// 两台设备离线各建一个清单（甚至同名）：id 不撞，同步后两边都有两个（同步造成的重名容忍）
#[test]
fn two_devices_creating_lists_offline_keep_both() {
    let fx = Fixture::new("lists-two-devices");
    let other_db = Database::new_in_memory().unwrap();
    let other = SyncEngine {
        db: &other_db,
        client: &fx.client,
        images_dir: fx.dir.clone(),
        device_id: "test-pc-2".to_string(),
        retry_delay: fx.retry_delay,
    };
    let a = lists::create(&fx.db, "工作").unwrap();
    // 不同毫秒：毫秒分量不同，id 一定不同（同一毫秒靠随机后缀，测试里不赌概率）
    std::thread::sleep(std::time::Duration::from_millis(3));
    let b = lists::create(&other_db, "工作").unwrap();
    assert_ne!(a.id, b.id);

    fx.engine().sync(false).unwrap();
    other.sync(false).unwrap();
    fx.engine().sync(false).unwrap();
    for db in [&fx.db, &other_db] {
        let mut ids: Vec<i64> = lists::list(db).unwrap().iter().map(|l| l.id).collect();
        ids.sort();
        let mut expected = vec![1, a.id, b.id];
        expected.sort();
        assert_eq!(ids, expected);
        assert_eq!(list_names(db).iter().filter(|n| *n == "工作").count(), 2);
    }
}

#[test]
fn force_push_and_force_pull_cover_lists() {
    let fx = Fixture::new("lists-force");
    let mut remote = doc(vec![]);
    remote["todoLists"] = json!([
        list_json(1, "远端默认", "2026-09-01 10:00:00"),
        list_json(LIFE, "远端独有", "2026-09-01 10:00:00")
    ]);
    fx.set_remote(&remote);
    let work = lists::create(&fx.db, "本地清单").unwrap();

    // 强制推送：远端独有的清单写墓碑；本地清单（含默认清单）胜出
    assert_eq!(fx.engine().force_push().unwrap().status, SyncStatus::Pushed);
    let pushed = fx.remote();
    assert_eq!(list_ids_of(&pushed), vec![1, work.id]);
    assert_eq!(list_tombstone_ids(&pushed), vec![LIFE]);
    let default_list = &pushed["todoLists"][0];
    assert_eq!(default_list["name"], "待办清单");
    assert!(default_list["updatedAt"].as_str().unwrap() > "2026-09-01 10:00:00");

    // 强制拉取：本地清单 = 远端（本地独有的删掉、不写墓碑，窗口记录一并清理）
    window_prefs::update_prefs(
        &fx.db,
        WindowKey::List(work.id),
        &UpdateWindowPrefs {
            hidden: Some(true),
            ..Default::default()
        },
    )
    .unwrap()
    .unwrap();
    let mut cloud = doc(vec![]);
    cloud["todoLists"] = json!([
        list_json(1, "云端默认", "2026-09-05 10:00:00"),
        list_json(55, "云端", "2026-09-05 10:00:00")
    ]);
    cloud["listTombstones"] = json!([]);
    fx.set_remote(&cloud);
    let report = fx.engine().force_pull().unwrap();
    assert_eq!(
        (
            report.lists_inserted,
            report.lists_updated,
            report.lists_deleted
        ),
        (1, 1, 1)
    );
    assert_eq!(list_names(&fx.db), vec!["云端默认", "云端"]);
    assert_eq!(
        window_prefs::get_prefs(&fx.db, WindowKey::List(work.id)).unwrap(),
        None
    );
    let tombs = fx
        .db
        .with_connection(|c| sync_store::load_list_tombstones(c, "2000-01-01 00:00:00"))
        .unwrap();
    assert!(tombs.is_empty(), "清单墓碑表替换为远端");

    // 远端没有清单信息：强制拉取不动本地清单
    fx.set_remote(&doc(vec![]));
    fx.engine().force_pull().unwrap();
    assert_eq!(list_names(&fx.db), vec!["云端默认", "云端"]);
}

#[test]
fn decode_reads_list_keys_as_fields_not_extra() {
    let parsed = decode_sync_doc(
        br#"{"todos": [], "futureKey": 1,
             "todoLists": [{"id": 1, "name": "x", "updatedAt": "2026-05-01 10:00:00"}, "garbage"],
             "listTombstones": [{"entityId": 1, "deletedAt": "2026-05-01"},
                                {"entityId": 5, "deletedAt": "2026-05-01T10:00"},
                                {"bogus": true}]}"#,
    )
    .unwrap();
    assert_eq!(
        parsed.todo_lists.as_ref().map(Vec::len),
        Some(2),
        "原样保留，坏记录在合并时计数"
    );
    assert_eq!(
        parsed.list_tombstones,
        Some(vec![ListTombstone {
            entity_id: 5,
            deleted_at: "2026-05-01 10:00:00".to_string()
        }]),
        "默认清单的墓碑与看不懂的墓碑丢弃"
    );
    assert!(!parsed.extra.contains_key("todoLists"));
    assert!(!parsed.extra.contains_key("listTombstones"));
    assert_eq!(parsed.extra.get("futureKey"), Some(&json!(1)));

    let missing = decode_sync_doc(br#"{"todos": [], "todoLists": null}"#).unwrap();
    assert_eq!(missing.todo_lists, None, "null 同缺失：远端没有清单信息");
    assert_eq!(missing.list_tombstones, None);
    for bad in [
        &br#"{"todoLists": "x"}"#[..],
        br#"{"todoLists": {"id": 1}}"#,
        br#"{"listTombstones": {}}"#,
    ] {
        assert!(
            decode_sync_doc(bad).is_err(),
            "{}",
            String::from_utf8_lossy(bad)
        );
    }
}

#[test]
fn doc_differs_covers_lists_and_list_tombstones() {
    let db = Database::new_in_memory().unwrap();
    let cutoff = "2000-01-01 00:00:00";
    let local = local_state(&db);
    let uploaded = build_sync_doc(&local, None, DocMode::Merge, "pc", cutoff);
    let remote: SyncData =
        serde_json::from_value(serde_json::to_value(&uploaded).unwrap()).unwrap();
    let again = build_sync_doc(&local, Some(&remote), DocMode::Merge, "pc", cutoff);
    assert!(!doc_differs(&again, &remote, cutoff));

    // 远端没有清单信息、本机只有种子状态的默认清单：等价
    let mut old = remote.clone();
    old.todo_lists = None;
    old.list_tombstones = None;
    let doc = build_sync_doc(&local, Some(&old), DocMode::Merge, "pc", cutoff);
    assert!(!doc_differs(&doc, &old, cutoff));

    // 远端缺本机的清单 → 需要上传
    lists::create(&db, "工作").unwrap();
    let doc = build_sync_doc(
        &local_state(&db),
        Some(&remote),
        DocMode::Merge,
        "pc",
        cutoff,
    );
    assert!(doc_differs(&doc, &remote, cutoff));

    // 本机多了清单墓碑 → 需要上传
    let db = Database::new_in_memory().unwrap();
    db.with_transaction(|tx| sync_store::record_list_tombstone(tx, 42, "2026-09-01 00:00:00"))
        .unwrap();
    let doc = build_sync_doc(
        &local_state(&db),
        Some(&remote),
        DocMode::Merge,
        "pc",
        cutoff,
    );
    assert!(doc_differs(&doc, &remote, cutoff));
    assert_eq!(
        doc.list_tombstones,
        Some(vec![ListTombstone {
            entity_id: 42,
            deleted_at: "2026-09-01 00:00:00".to_string()
        }])
    );
}

#[test]
fn list_value_merge_follows_lww_tombstones_and_union() {
    let mut tombs = ListTombstoneIndex::new();
    tombs.insert(4, "2026-09-04 00:00:00".to_string());
    let local = vec![list_json(1, "本地较新", "2026-09-05 00:00:00")];
    let remote = vec![
        list_json(1, "远端较旧", "2026-09-01 00:00:00"),
        json!({"id": 2, "name": 123, "updatedAt": "2026-09-01 00:00:00"}),
        json!("无 id 的垃圾"),
        list_json(4, "已删除", "2026-09-03 00:00:00"),
        list_json(5, "远端新增", "2026-09-03 00:00:00"),
    ];
    let merged = merge_list_values(local, &remote, &tombs);
    let names: Vec<Value> = merged.iter().map(|l| l["name"].clone()).collect();
    assert_eq!(
        names,
        vec![
            json!("本地较新"),
            json!(123),
            Value::Null,
            json!("远端新增")
        ],
        "看不懂的远端记录原样带上，被墓碑压制的丢掉"
    );
}
