//! 端到端同步场景测试：真实的 pull / push / 图片代码 + 进程内 mock WebDAV（`mock_dav`）。
//!
//! 全部是普通 `#[test]`：被测代码用 reqwest blocking，不能跑在 tokio runtime 线程里
//! （mock 服务自己跑在独立线程的 runtime 上）。

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use chrono_tz::Tz;
use serde_json::{json, Value};
use tempfile::TempDir;

use crate::config::Config;
use crate::db::repo::{self, meta_keys as mk};
use crate::db::Db;
use crate::sync::doc::{placeholder_settings, CLOUD_DEVICE_ID};
use crate::sync::mock_dav::{MockDav, XmlStyle};
use crate::sync::{gzip, images, pull, push, SyncCtx, SYNC_DATA_FILE};
use crate::time::now_local_string;

struct Env {
    mock: MockDav,
    ctx: Arc<SyncCtx>,
    db: Db,
    _tmp: TempDir,
}

fn env(mock: MockDav) -> Env {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let images_dir = tmp.path().join("images");
    std::fs::create_dir_all(&data_dir).unwrap();
    std::fs::create_dir_all(&images_dir).unwrap();
    let mut cfg = Config::for_tests(
        "test-api-key-1234567890abcdef",
        data_dir.clone(),
        images_dir,
    );
    cfg.webdav_url = mock.base_url.clone();
    let db = Db::open(&data_dir.join("data.db")).unwrap();
    let ctx = SyncCtx::new(Arc::new(cfg), db.clone());
    Env {
        mock,
        ctx,
        db,
        _tmp: tmp,
    }
}

fn tz() -> Tz {
    "Asia/Shanghai".parse().unwrap()
}

/// 规范格式的"现在 + offset 分钟"（墓碑必须落在 30 天保留期内，所以用相对时间）。
fn ts(offset_minutes: i64) -> String {
    (chrono::Utc::now() + chrono::TimeDelta::minutes(offset_minutes))
        .with_timezone(&tz())
        .format("%Y-%m-%d %H:%M:%S")
        .to_string()
}

/// PC 端形态的完整 todo 记录（字段与 `pc/src-tauri/src/db/models.rs::Todo` 一致）。
fn pc_todo(id: i64, title: &str, updated_at: &str, subtasks: Vec<Value>) -> Value {
    json!({
        "id": id,
        "title": title,
        "description": null,
        "color": "#10B981",
        "quadrant": 4,
        "notifyAt": null,
        "notifyBefore": 0,
        "notified": false,
        "completed": false,
        "sortOrder": id,
        "startTime": null,
        "endTime": null,
        "createdAt": "2026-01-01 08:00:00",
        "updatedAt": updated_at,
        "repeatEnabled": false,
        "repeatType": null,
        "repeatInterval": 1,
        "repeatWeekdays": null,
        "repeatMonthDay": null,
        "subtasks": subtasks,
    })
}

fn pc_subtask(id: i64, parent_id: i64, title: &str, updated_at: &str) -> Value {
    json!({
        "id": id,
        "parentId": parent_id,
        "title": title,
        "content": null,
        "completed": false,
        "sortOrder": 0,
        "createdAt": "2026-01-01 08:00:00",
        "updatedAt": updated_at,
    })
}

/// 新版 PC 写出的文档（带 tombstones / settingsUpdatedAt）。
fn pc_doc(todos: Vec<Value>, tombstones: Vec<Value>) -> Value {
    json!({
        "version": "4.0",
        "deviceId": "dev_pc",
        "updatedAt": "2026-10-02T10:00:00+08:00",
        "todos": todos,
        "settings": {"isFixed": false, "windowPosition": null, "windowSize": null, "textTheme": "dark"},
        "settingsUpdatedAt": "2026-10-01 09:00:00",
        "images": [],
        "tombstones": tombstones,
    })
}

/// 与 API `POST /todos` 写路径相同的落库动作（upsert + 分配 seq + 标脏）。
fn api_create_todo(db: &Db, id: i64, title: &str) {
    let now = now_local_string(tz());
    let body = json!({
        "id": id, "title": title, "color": "#EF4444", "quadrant": 1, "completed": false,
        "notified": false, "notifyBefore": 0, "sortOrder": 0, "createdAt": now, "updatedAt": now
    });
    db.with_conn(|c| {
        repo::upsert_todo(c, &id.to_string(), &body.to_string(), &now).unwrap();
        repo::assign_seq(c, &id.to_string()).unwrap();
        repo::mark_dirty(c).unwrap();
    });
}

/// 与 API `DELETE /todos/:id` 写路径相同的落库动作（级联删除 + 墓碑 + 标脏）。
fn api_delete_todo(db: &Db, id: &str) {
    let now = now_local_string(tz());
    db.with_conn(|c| {
        let tx = c.transaction().unwrap();
        let subs: Vec<String> = repo::list_subtasks_for_todo(&tx, id)
            .unwrap()
            .into_iter()
            .map(|r| r.id)
            .collect();
        assert!(repo::delete_todo_cascade(&tx, id).unwrap());
        repo::add_tombstone(&tx, "todo", id, &now).unwrap();
        for s in subs {
            repo::add_tombstone(&tx, "subtask", &s, &now).unwrap();
        }
        repo::delete_seq(&tx, id).unwrap();
        repo::mark_dirty(&tx).unwrap();
        tx.commit().unwrap();
    });
}

fn todo_title(db: &Db, id: &str) -> Option<String> {
    db.with_conn(|c| {
        repo::get_todo(c, id).unwrap().map(|row| {
            serde_json::from_str::<Value>(&row.data_json).unwrap()["title"]
                .as_str()
                .unwrap()
                .to_string()
        })
    })
}

fn meta(db: &Db, key: &str) -> Option<String> {
    db.with_conn(|c| repo::get_meta(c, key)).unwrap()
}

fn is_dirty(db: &Db) -> bool {
    db.with_conn(|c| repo::is_dirty(c)).unwrap()
}

fn remote_ids(doc: &Value) -> Vec<i64> {
    let mut ids: Vec<i64> = doc["todos"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["id"].as_i64().unwrap())
        .collect();
    ids.sort();
    ids
}

fn remote_todo(doc: &Value, id: i64) -> Option<Value> {
    doc["todos"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == json!(id))
        .cloned()
}

fn has_tombstone(doc: &Value, entity_type: &str, id: i64) -> bool {
    doc["tombstones"]
        .as_array()
        .unwrap()
        .iter()
        .any(|t| t["entityType"] == json!(entity_type) && t["entityId"] == json!(id))
}

fn sync_data_calls(mock: &MockDav) -> Vec<(String, u16)> {
    mock.requests()
        .into_iter()
        .filter(|r| r.path == SYNC_DATA_FILE)
        .map(|r| (r.method, r.status))
        .collect()
}

// =============================================================================
// K4：写前合并、基准、条件 PUT
// =============================================================================

/// 审查 A4 回归：push 先合并远端再上传，缓存 = 上传内容；基准取自 PUT 响应，
/// PUT 之后没有整包 GET；之后的 pull 304 不会让缓存停在旧版本。
#[test]
fn push_merges_remote_first_and_cache_matches_upload() {
    let e = env(MockDav::start());
    e.mock.put_doc(&pc_doc(
        vec![pc_todo(1, "A", "2026-10-01 10:00:00", vec![])],
        vec![],
    ));
    pull::pull_once(&e.ctx).unwrap();
    assert_eq!(todo_title(&e.db, "1").as_deref(), Some("A"));
    assert_eq!(
        meta(&e.db, mk::BASE_ETAG),
        e.mock.with(|s| s.etag_of(SYNC_DATA_FILE))
    );
    assert!(meta(&e.db, mk::BASE_LAST_MODIFIED).is_some());

    // PC 改了 1、新建了 2；与此同时 AI 通过云端 API 新建了 100
    e.mock.put_doc(&pc_doc(
        vec![
            pc_todo(1, "A-pc", "2026-10-01 11:00:00", vec![]),
            pc_todo(2, "B-pc", "2026-10-01 11:00:00", vec![]),
        ],
        vec![],
    ));
    let v2 = e.mock.with(|s| s.etag_of(SYNC_DATA_FILE));
    api_create_todo(&e.db, 100, "from AI");
    e.mock.clear_log();

    let report = push::push_once(&e.ctx).unwrap();
    assert!(report.pushed);
    assert!(report.dirty_cleared);
    assert_eq!(report.attempts, 1);
    assert!(!is_dirty(&e.db));

    let remote = e.mock.doc().unwrap();
    assert_eq!(remote_ids(&remote), vec![1, 2, 100]);
    assert_eq!(remote_todo(&remote, 1).unwrap()["title"], json!("A-pc"));
    // 缓存已经合并了 PC 的修改（旧实现缓存停在 "A"，pull 随后永远 304）
    assert_eq!(todo_title(&e.db, "1").as_deref(), Some("A-pc"));
    assert_eq!(todo_title(&e.db, "2").as_deref(), Some("B-pc"));

    let reqs = e.mock.requests();
    let put_idx = reqs.iter().position(|r| r.method == "PUT").unwrap();
    assert_eq!(reqs[put_idx].if_match, v2, "PUT 用刚合并的版本做 If-Match");
    assert!(
        reqs[put_idx + 1..]
            .iter()
            .all(|r| r.method != "GET" && r.method != "PROPFIND"),
        "PUT 响应带了校验器，之后不该再有 GET / PROPFIND: {:?}",
        &reqs[put_idx + 1..]
    );
    assert_eq!(e.mock.count("MKCOL", ""), 0, "目录已存在时不 MKCOL");
    assert_eq!(
        meta(&e.db, mk::BASE_ETAG),
        e.mock.with(|s| s.etag_of(SYNC_DATA_FILE)),
        "基准 = 刚 PUT 的版本"
    );
    assert!(meta(&e.db, mk::LAST_PUSH_OK_AT).is_some());

    // 之后 pull：304，缓存不变
    e.mock.clear_log();
    let r = pull::pull_once(&e.ctx).unwrap();
    assert!(!r.changed);
    assert_eq!(sync_data_calls(&e.mock), vec![("GET".to_string(), 304)]);

    // PC 在云端上传的文档基础上再改：照常合并进来
    let mut pc_next = remote.clone();
    for t in pc_next["todos"].as_array_mut().unwrap() {
        if t["id"] == json!(1) {
            t["title"] = json!("A-pc2");
            t["updatedAt"] = json!("2026-10-01 12:00:00");
        }
    }
    e.mock.put_doc(&pc_next);
    let r = pull::pull_once(&e.ctx).unwrap();
    assert!(r.changed);
    assert_eq!(todo_title(&e.db, "1").as_deref(), Some("A-pc2"));
    assert_eq!(todo_title(&e.db, "100").as_deref(), Some("from AI"));
}

/// 另一个写入方在云端 GET 与 PUT 之间写入 → If-Match 412 → 无条件重新 GET → 合并 → 重试成功。
#[test]
fn concurrent_write_between_get_and_put_triggers_412_merge_and_retry() {
    let e = env(MockDav::start());
    e.mock.put_doc(&pc_doc(
        vec![pc_todo(1, "A", "2026-10-01 10:00:00", vec![])],
        vec![],
    ));
    pull::pull_once(&e.ctx).unwrap();
    api_create_todo(&e.db, 100, "from AI");

    let pc_v2 = pc_doc(
        vec![
            pc_todo(1, "A", "2026-10-01 10:00:00", vec![]),
            pc_todo(2, "B-pc", "2026-10-01 11:00:00", vec![]),
        ],
        vec![],
    );
    e.mock
        .with(|s| s.before_put = Some(Box::new(move |s| s.write_doc(&pc_v2))));
    e.mock.clear_log();

    let report = push::push_once(&e.ctx).unwrap();
    assert_eq!(report.attempts, 2);
    assert_eq!(
        sync_data_calls(&e.mock),
        vec![
            ("GET".to_string(), 304),
            ("PUT".to_string(), 412),
            ("GET".to_string(), 200),
            ("PUT".to_string(), 204),
        ]
    );
    let after_412_get = e
        .mock
        .requests()
        .into_iter()
        .filter(|r| r.method == "GET")
        .nth(1)
        .unwrap();
    assert!(
        after_412_get.if_none_match.is_none() && after_412_get.if_modified_since.is_none(),
        "412 之后必须无条件 GET"
    );
    let remote = e.mock.doc().unwrap();
    assert_eq!(remote_ids(&remote), vec![1, 2, 100]);
    assert_eq!(todo_title(&e.db, "2").as_deref(), Some("B-pc"));
    assert!(!e.ctx.if_match_unreliable());
}

/// Apache：一秒内的弱 ETag 不能用于 If-Match（必然 412）→ 用 If-Unmodified-Since；
/// PUT 响应不带校验器 → 恰好一次 PROPFIND Depth 0，不整包 GET；ETag 变强后升级为 If-Match。
#[test]
fn apache_weak_etag_uses_if_unmodified_since_and_propfind_after_put() {
    let e = env(MockDav::start_with(|s| {
        s.dirs.insert("/mini-todo".into());
        s.dirs.insert("/mini-todo/images".into());
        s.weak_etag_window = Duration::from_secs(3600);
        s.put_returns_validators = false;
    }));
    e.mock.put_doc(&pc_doc(
        vec![pc_todo(1, "A", "2026-10-01 10:00:00", vec![])],
        vec![],
    ));
    pull::pull_once(&e.ctx).unwrap();
    let weak = meta(&e.db, mk::BASE_ETAG).unwrap();
    assert!(weak.starts_with("W/"), "{}", weak);
    assert!(
        meta(&e.db, mk::BASE_LAST_MODIFIED).is_some(),
        "ETag 与 Last-Modified 必须一起存"
    );

    api_create_todo(&e.db, 100, "from AI");
    e.mock.clear_log();
    push::push_once(&e.ctx).unwrap();
    let reqs = e.mock.requests();
    // GET 用弱 ETag 做 If-None-Match（弱比较）→ 304
    assert_eq!(reqs[0].method, "GET");
    assert_eq!(reqs[0].status, 304);
    assert_eq!(reqs[0].if_none_match.as_deref(), Some(weak.as_str()));
    let put_idx = reqs.iter().position(|r| r.method == "PUT").unwrap();
    let put = &reqs[put_idx];
    assert!(put.if_match.is_none(), "弱 ETag 不能做 If-Match");
    assert!(put.if_unmodified_since.is_some());
    assert!(matches!(put.status, 201 | 204), "{}", put.status);
    let after: Vec<_> = reqs[put_idx + 1..].iter().collect();
    assert_eq!(after.len(), 1, "{:?}", after);
    assert_eq!(after[0].method, "PROPFIND");
    assert_eq!(after[0].depth.as_deref(), Some("0"));
    let new_base = meta(&e.db, mk::BASE_ETAG).unwrap();
    assert_eq!(
        Some(new_base.clone()),
        e.mock.with(|s| s.etag_of(SYNC_DATA_FILE))
    );
    assert!(meta(&e.db, mk::BASE_LAST_MODIFIED).is_some());

    // 一秒后 Apache 的 ETag 变强：304 响应带回强 ETag，基准随之升级，之后的 PUT 用 If-Match
    e.mock.with(|s| s.weak_etag_window = Duration::ZERO);
    pull::pull_once(&e.ctx).unwrap();
    let strong = meta(&e.db, mk::BASE_ETAG).unwrap();
    assert!(!strong.starts_with("W/"), "{}", strong);
    api_create_todo(&e.db, 101, "again");
    e.mock.clear_log();
    push::push_once(&e.ctx).unwrap();
    let put = e
        .mock
        .requests()
        .into_iter()
        .find(|r| r.method == "PUT")
        .unwrap();
    assert_eq!(put.if_match.as_deref(), Some(strong.as_str()));
    assert_eq!(remote_ids(&e.mock.doc().unwrap()), vec![1, 100, 101]);
}

/// Apache + If-Unmodified-Since：别人在之后的某一秒写入 → 412 → 合并重试。
#[test]
fn if_unmodified_since_conflict_is_merged_and_retried() {
    let e = env(MockDav::start_with(|s| {
        s.dirs.insert("/mini-todo".into());
        s.weak_etag_window = Duration::from_secs(3600);
        s.put_returns_validators = false;
    }));
    e.mock.put_doc(&pc_doc(
        vec![pc_todo(1, "A", "2026-10-01 10:00:00", vec![])],
        vec![],
    ));
    pull::pull_once(&e.ctx).unwrap();
    api_create_todo(&e.db, 100, "from AI");
    let pc_v2 = pc_doc(
        vec![
            pc_todo(1, "A", "2026-10-01 10:00:00", vec![]),
            pc_todo(2, "B-pc", "2026-10-01 11:00:00", vec![]),
        ],
        vec![],
    );
    let later = SystemTime::now() + Duration::from_secs(5);
    e.mock.with(|s| {
        s.before_put = Some(Box::new(move |s| {
            s.write_at(
                SYNC_DATA_FILE,
                gzip(pc_v2.to_string().as_bytes()).unwrap(),
                later,
            )
        }))
    });
    e.mock.clear_log();
    let report = push::push_once(&e.ctx).unwrap();
    assert_eq!(report.attempts, 2);
    let puts: Vec<_> = e
        .mock
        .requests()
        .into_iter()
        .filter(|r| r.method == "PUT")
        .collect();
    assert_eq!(puts.len(), 2);
    assert_eq!(puts[0].status, 412);
    assert!(puts
        .iter()
        .all(|p| p.if_unmodified_since.is_some() && p.if_match.is_none()));
    assert_eq!(remote_ids(&e.mock.doc().unwrap()), vec![1, 2, 100]);
}

/// nginx dav / Caddy：PUT 忽略前置条件、永远不会 412。写前 GET + 合并是唯一的保护：
/// 云端上次同步之后 PC 写入的内容不会被覆盖掉。
#[test]
fn nginx_ignoring_preconditions_still_merges_before_put() {
    let e = env(MockDav::start_with(|s| {
        s.dirs.insert("/mini-todo".into());
        s.ignore_preconditions = true;
    }));
    e.mock.put_doc(&pc_doc(
        vec![pc_todo(1, "A", "2026-10-01 10:00:00", vec![])],
        vec![],
    ));
    pull::pull_once(&e.ctx).unwrap();
    // PC 在云端下次 push 之前写入
    e.mock.put_doc(&pc_doc(
        vec![
            pc_todo(1, "A", "2026-10-01 10:00:00", vec![]),
            pc_todo(2, "B-pc", "2026-10-01 11:00:00", vec![]),
        ],
        vec![],
    ));
    api_create_todo(&e.db, 100, "from AI");
    e.mock.clear_log();
    push::push_once(&e.ctx).unwrap();
    assert_eq!(
        sync_data_calls(&e.mock),
        vec![("GET".to_string(), 200), ("PUT".to_string(), 204)]
    );
    assert_eq!(remote_ids(&e.mock.doc().unwrap()), vec![1, 2, 100]);
}

/// 服务端被证实错误处理 If-Match（远端没变却 412）→ 退回 If-Unmodified-Since。
#[test]
fn server_rejecting_valid_if_match_falls_back_to_if_unmodified_since() {
    let e = env(MockDav::start());
    e.mock.put_doc(&pc_doc(
        vec![pc_todo(1, "A", "2026-10-01 10:00:00", vec![])],
        vec![],
    ));
    pull::pull_once(&e.ctx).unwrap();
    api_create_todo(&e.db, 100, "from AI");
    e.mock.add_fault("PUT", "sync-data", 412, 1);
    e.mock.clear_log();
    push::push_once(&e.ctx).unwrap();
    assert!(e.ctx.if_match_unreliable());
    let puts: Vec<_> = e
        .mock
        .requests()
        .into_iter()
        .filter(|r| r.method == "PUT")
        .collect();
    assert_eq!(puts.len(), 2);
    assert!(puts[0].if_match.is_some());
    assert!(puts[1].if_match.is_none() && puts[1].if_unmodified_since.is_some());
    assert_eq!(remote_ids(&e.mock.doc().unwrap()), vec![1, 100]);
}

/// 读不懂的远端绝不覆盖；读失败的 GET 不记为基准。
#[test]
fn unreadable_remote_is_never_overwritten_nor_recorded_as_base() {
    let e = env(MockDav::start());
    e.mock.put_doc(&pc_doc(
        vec![pc_todo(1, "A", "2026-10-01 10:00:00", vec![])],
        vec![],
    ));
    pull::pull_once(&e.ctx).unwrap();
    let base = meta(&e.db, mk::BASE_ETAG);

    e.mock
        .with(|s| s.write(SYNC_DATA_FILE, b"definitely not gzip".to_vec()));
    let garbage = e.mock.file(SYNC_DATA_FILE).unwrap();
    assert!(pull::pull_once(&e.ctx).is_err());
    assert_eq!(
        meta(&e.db, mk::BASE_ETAG),
        base,
        "没合并的 GET 不能记为基准"
    );
    assert!(meta(&e.db, mk::LAST_PULL_ERROR).is_some());

    api_create_todo(&e.db, 100, "from AI");
    e.mock.clear_log();
    assert!(push::push_once(&e.ctx).is_err());
    assert_eq!(e.mock.count("PUT", ""), 0);
    assert_eq!(e.mock.file(SYNC_DATA_FILE).unwrap(), garbage);
    assert!(is_dirty(&e.db));
    assert!(meta(&e.db, mk::LAST_PUSH_ERROR).is_some());

    // 顶层不是对象的 JSON 同样拒绝
    e.mock
        .with(|s| s.write(SYNC_DATA_FILE, gzip(b"[1,2,3]").unwrap()));
    assert!(push::push_once(&e.ctx).is_err());
    assert_eq!(e.mock.count("PUT", ""), 0);
}

/// 父目录不存在时 PUT 404/409 → MKCOL 链 → 重试；目录已在时不再 MKCOL。
#[test]
fn mkcol_only_when_put_reports_missing_parent() {
    let e = env(MockDav::start_empty());
    api_create_todo(&e.db, 100, "from AI");
    push::push_once(&e.ctx).unwrap();
    assert_eq!(
        sync_data_calls(&e.mock),
        vec![
            ("GET".to_string(), 404),
            ("PUT".to_string(), 409),
            ("PUT".to_string(), 201),
        ]
    );
    assert_eq!(e.mock.count("MKCOL", ""), 1);
    let put = e
        .mock
        .requests()
        .into_iter()
        .find(|r| r.method == "PUT")
        .unwrap();
    assert!(
        put.if_match.is_none() && put.if_unmodified_since.is_none(),
        "远端不存在时 PUT 不带前置条件"
    );

    api_create_todo(&e.db, 101, "again");
    e.mock.clear_log();
    push::push_once(&e.ctx).unwrap();
    assert_eq!(e.mock.count("MKCOL", ""), 0);
    assert_eq!(remote_ids(&e.mock.doc().unwrap()), vec![100, 101]);
}

// =============================================================================
// K2：以远端文档为底，未知顶层键 / settings 原样保留
// =============================================================================

#[test]
fn push_preserves_settings_unknown_keys_and_pc_records_even_on_304() {
    let e = env(MockDav::start());
    let todo1 = pc_todo(
        1,
        "A",
        "2026-10-01 10:00:00",
        vec![pc_subtask(11, 1, "child", "2026-10-01 10:00:00")],
    );
    let mut doc = pc_doc(vec![todo1.clone()], vec![]);
    doc["futureKey"] = json!({"nested": [1, {"x": true}]});
    doc["images"] = json!(["1715000000000_abc123.png"]);
    e.mock.put_doc(&doc);
    pull::pull_once(&e.ctx).unwrap();

    api_create_todo(&e.db, 100, "from AI");
    e.mock.clear_log();
    push::push_once(&e.ctx).unwrap();
    assert_eq!(
        sync_data_calls(&e.mock)[0],
        ("GET".to_string(), 304),
        "远端没变：靠存下来的信封重建文档"
    );

    let remote = e.mock.doc().unwrap();
    assert_eq!(remote["settings"], doc["settings"]);
    assert_eq!(remote["settingsUpdatedAt"], doc["settingsUpdatedAt"]);
    assert_eq!(remote["futureKey"], doc["futureKey"]);
    assert_eq!(remote["version"], json!("4.0"));
    assert_eq!(remote["deviceId"], json!(CLOUD_DEVICE_ID));
    assert_ne!(remote["updatedAt"], doc["updatedAt"]);
    assert_eq!(remote["tombstones"], json!([]));
    assert!(remote["images"]
        .as_array()
        .unwrap()
        .contains(&json!("1715000000000_abc123.png")));
    // PC 记录（含嵌套子任务）原样往返
    assert_eq!(remote_todo(&remote, 1).unwrap(), todo1);
}

#[test]
fn first_push_to_empty_remote_writes_placeholder_settings_without_timestamp() {
    let e = env(MockDav::start());
    let r = pull::pull_once(&e.ctx).unwrap();
    assert!(!r.remote_exists);
    api_create_todo(&e.db, 100, "from AI");
    push::push_once(&e.ctx).unwrap();
    let remote = e.mock.doc().unwrap();
    assert_eq!(remote["settings"], placeholder_settings());
    assert!(remote.get("settingsUpdatedAt").is_none());
    assert_eq!(remote_ids(&remote), vec![100]);
    for key in [
        "version",
        "deviceId",
        "updatedAt",
        "todos",
        "settings",
        "images",
        "tombstones",
    ] {
        assert!(remote.get(key).is_some(), "missing {}", key);
    }
}

// =============================================================================
// K3：墓碑双向传播
// =============================================================================

#[test]
fn cloud_delete_propagates_as_tombstone_and_remote_tombstones_are_kept() {
    let e = env(MockDav::start());
    let recent = ts(-60);
    e.mock.put_doc(&pc_doc(
        vec![
            pc_todo(
                1,
                "A",
                "2026-10-01 10:00:00",
                vec![pc_subtask(11, 1, "child", "2026-10-01 10:00:00")],
            ),
            pc_todo(2, "B", "2026-10-01 10:00:00", vec![]),
        ],
        vec![json!({"entityType": "subtask", "entityId": 99, "deletedAt": recent})],
    ));
    pull::pull_once(&e.ctx).unwrap();
    api_delete_todo(&e.db, "1");
    push::push_once(&e.ctx).unwrap();

    let remote = e.mock.doc().unwrap();
    assert_eq!(remote_ids(&remote), vec![2]);
    assert!(has_tombstone(&remote, "todo", 1));
    assert!(has_tombstone(&remote, "subtask", 11));
    assert!(has_tombstone(&remote, "subtask", 99), "远端墓碑并集保留");
    for t in remote["tombstones"].as_array().unwrap() {
        let d = t["deletedAt"].as_str().unwrap();
        assert_eq!(d.len(), 19);
        assert_eq!(&d[10..11], " ", "deletedAt 必须是规范格式: {}", d);
        assert!(t["entityId"].is_i64());
    }
}

#[test]
fn pull_applies_remote_tombstones_and_keeps_edit_after_delete() {
    let e = env(MockDav::start());
    e.mock.put_doc(&pc_doc(
        vec![
            pc_todo(1, "A", "2026-01-01 10:00:00", vec![]),
            pc_todo(2, "B", "2026-01-01 10:00:00", vec![]),
            pc_todo(3, "C", "2026-01-01 10:00:00", vec![]),
        ],
        vec![],
    ));
    pull::pull_once(&e.ctx).unwrap();
    db_assign_seq(&e.db, "1");

    // PC 删了 1 和 3；但 3 在删除之后又被另一端编辑过
    let deleted_at = ts(-10);
    e.mock.put_doc(&pc_doc(
        vec![
            pc_todo(2, "B", "2026-01-01 10:00:00", vec![]),
            pc_todo(3, "C-edited", &ts(0), vec![]),
        ],
        vec![
            json!({"entityType": "todo", "entityId": 1, "deletedAt": deleted_at}),
            json!({"entityType": "todo", "entityId": 3, "deletedAt": deleted_at}),
        ],
    ));
    let r = pull::pull_once(&e.ctx).unwrap();
    assert_eq!(r.stats.todos_deleted, 1);
    assert!(todo_title(&e.db, "1").is_none());
    assert_eq!(todo_title(&e.db, "3").as_deref(), Some("C-edited"));
    let stones = e.db.with_conn(|c| repo::list_tombstones(c)).unwrap();
    assert_eq!(stones.len(), 2);

    // 墓碑随后的 push 继续传播（并集）
    api_create_todo(&e.db, 100, "from AI");
    push::push_once(&e.ctx).unwrap();
    let remote = e.mock.doc().unwrap();
    assert_eq!(remote_ids(&remote), vec![2, 3, 100]);
    assert!(has_tombstone(&remote, "todo", 1));
    assert!(has_tombstone(&remote, "todo", 3));
}

fn db_assign_seq(db: &Db, id: &str) {
    db.with_conn(|c| repo::assign_seq(c, id)).unwrap();
}

/// 旧版 PC 写的文档（没有 tombstones 键）+ 云端不 dirty → 沿用缺席清理；
/// 云端 dirty 时不清理。
#[test]
fn legacy_document_cleanup_only_when_not_dirty() {
    let e = env(MockDav::start());
    let legacy = |todos: Vec<Value>| {
        json!({"version": "4.0", "deviceId": "old_pc", "updatedAt": "x", "todos": todos,
               "settings": {"isFixed": false, "windowPosition": null, "windowSize": null}, "images": []})
    };
    e.mock.put_doc(&legacy(vec![
        pc_todo(1, "A", "2026-01-01 10:00:00", vec![]),
        pc_todo(2, "B", "2026-01-01 10:00:00", vec![]),
    ]));
    pull::pull_once(&e.ctx).unwrap();
    e.mock.put_doc(&legacy(vec![pc_todo(
        1,
        "A",
        "2026-01-01 10:00:00",
        vec![],
    )]));
    let r = pull::pull_once(&e.ctx).unwrap();
    assert!(r.stats.legacy_cleanup);
    assert!(todo_title(&e.db, "2").is_none());

    api_create_todo(&e.db, 100, "from AI");
    e.mock.put_doc(&legacy(vec![pc_todo(
        1,
        "A",
        "2026-01-01 10:00:00",
        vec![],
    )]));
    let r = pull::pull_once(&e.ctx).unwrap();
    assert!(!r.stats.legacy_cleanup);
    assert!(todo_title(&e.db, "100").is_some(), "dirty 时不得清理");
}

// =============================================================================
// K5：图片
// =============================================================================

/// 上传期间（不持 DB 锁）新入队的图片不能丢（审查 A8）。
#[test]
fn image_enqueued_during_upload_is_not_lost() {
    let e = env(MockDav::start());
    let images_dir = e.ctx.cfg.images_dir.clone();
    std::fs::write(images_dir.join("a.png"), b"AAA").unwrap();
    std::fs::write(images_dir.join("b.png"), b"BBB").unwrap();
    e.db.with_conn(|c| {
        repo::enqueue_dirty_image(c, "a.png").unwrap();
        repo::mark_dirty(c).unwrap();
    });
    e.mock.with(|s| {
        s.delays.push((
            "PUT".into(),
            "/mini-todo/images/a.png".into(),
            Duration::from_millis(800),
        ))
    });

    let ctx = e.ctx.clone();
    let worker = std::thread::spawn(move || push::push_once(&ctx));
    // 等 a.png 的上传真正开始（push 已经读过队列），再模拟 API 上传 b.png
    assert!(e
        .mock
        .wait_started("PUT", "/mini-todo/images/a.png", Duration::from_secs(10)));
    // 与 API `POST /images` 相同的入队动作
    e.db.with_conn(|c| {
        repo::enqueue_dirty_image(c, "b.png").unwrap();
        repo::mark_dirty(c).unwrap();
    });
    let report = worker.join().unwrap().unwrap();
    assert_eq!(report.images.uploaded, 1);
    assert_eq!(
        e.db.with_conn(|c| repo::dirty_image_queue(c)).unwrap(),
        vec!["b.png".to_string()],
        "上传期间入队的 b.png 必须留在队列里"
    );
    assert_eq!(e.mock.file("/mini-todo/images/a.png").unwrap(), b"AAA");
    // sync-data 的 images 清单不列还没上传的图片
    let listed = e.mock.doc().unwrap()["images"].clone();
    assert_eq!(listed, json!(["a.png"]));
    // 图片先于 sync-data 上传
    let order: Vec<String> = e
        .mock
        .requests()
        .into_iter()
        .filter(|r| r.method == "PUT")
        .map(|r| r.path)
        .collect();
    assert_eq!(order, vec!["/mini-todo/images/a.png", SYNC_DATA_FILE]);

    push::push_once(&e.ctx).unwrap();
    assert!(e
        .db
        .with_conn(|c| repo::dirty_image_queue(c))
        .unwrap()
        .is_empty());
    assert_eq!(e.mock.file("/mini-todo/images/b.png").unwrap(), b"BBB");
    assert_eq!(meta(&e.db, mk::IMAGE_QUEUE_SINCE), None);
    assert_eq!(e.mock.doc().unwrap()["images"], json!(["a.png", "b.png"]));
}

#[test]
fn remote_image_listing_works_for_all_xml_styles() {
    for style in [
        XmlStyle::NginxMultiLine,
        XmlStyle::GoSingleLine,
        XmlStyle::SabreLower,
        XmlStyle::DefaultNamespace,
    ] {
        let e = env(MockDav::start_with(|s| s.xml_style = style));
        e.mock.with(|s| {
            s.write("/mini-todo/images/a.png", b"A".to_vec());
            s.write("/mini-todo/images/b.jpg", b"B".to_vec());
            s.write("/mini-todo/images/bad name.png", b"X".to_vec());
            s.write("/mini-todo/images/sub/c.png", b"C".to_vec());
        });
        let names = images::list_remote_images(e.ctx.dav().unwrap()).unwrap();
        assert_eq!(names, vec!["a.png", "b.jpg"], "{:?}", style);
    }
}

/// pull 拿到新文档后镜像缺失图片；单行 XML（x/net/webdav）也能列出全部文件。
#[test]
fn mirror_downloads_missing_images() {
    let e = env(MockDav::start_with(|s| {
        s.xml_style = XmlStyle::GoSingleLine
    }));
    e.mock.with(|s| {
        s.write("/mini-todo/images/a.png", b"A".to_vec());
        s.write("/mini-todo/images/b.jpg", b"B".to_vec());
        s.write("/mini-todo/images/bad name.png", b"X".to_vec());
    });
    let images_dir = e.ctx.cfg.images_dir.clone();
    std::fs::write(images_dir.join("b.jpg"), b"local").unwrap();

    let r = images::mirror_once(&e.ctx).unwrap();
    assert!(r.listing_error.is_none());
    assert_eq!(r.remote_count, 2);
    assert_eq!(r.downloaded, 1);
    assert_eq!(r.failed, 0);
    assert_eq!(std::fs::read(images_dir.join("a.png")).unwrap(), b"A");
    assert_eq!(std::fs::read(images_dir.join("b.jpg")).unwrap(), b"local");
    assert!(!images_dir.join("bad name.png").exists());
    let leftovers: Vec<_> = std::fs::read_dir(&images_dir)
        .unwrap()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().ends_with(".part"))
        .collect();
    assert!(leftovers.is_empty());

    // 再跑一次什么都不下载
    let r = images::mirror_once(&e.ctx).unwrap();
    assert_eq!(r.downloaded, 0);
}

/// PROPFIND 失败时退化为按 sync-data 的 images 清单下载（非法名照样跳过）。
#[test]
fn mirror_falls_back_to_document_image_list() {
    let e = env(MockDav::start());
    e.mock
        .with(|s| s.write("/mini-todo/images/a.png", b"A".to_vec()));
    let mut doc = pc_doc(vec![], vec![]);
    doc["images"] = json!(["a.png", "../evil.png", "gone.png"]);
    e.mock.put_doc(&doc);
    pull::pull_once(&e.ctx).unwrap();
    e.mock.add_fault("PROPFIND", "/mini-todo/images", 500, 1);

    let r = images::mirror_once(&e.ctx).unwrap();
    assert!(r.listing_error.is_some());
    assert_eq!(r.remote_count, 2);
    assert_eq!(r.downloaded, 1);
    assert_eq!(r.missing, 1);
    assert!(e.ctx.cfg.images_dir.join("a.png").exists());
    assert_eq!(e.mock.count("GET", "evil"), 0);
}
