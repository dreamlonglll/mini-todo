//! API 集成测试：用 axum Router + tower::ServiceExt::oneshot 直接打 in-process
//! 请求，覆盖鉴权、健康检查、todos / subtasks / images 全部 CRUD 路径，外加
//! 过滤、排序、分页、merge PATCH、cascade DELETE、tombstones、X-Sync-Status header。
//!
//! 这些测试**不**起后台 worker（pull / push / 图片镜像），不触碰外部网络：默认
//! WebDAV 地址不可达（127.0.0.1:0）；需要真走同步流程的用例连本进程内的 mock
//! WebDAV（`crate::sync::mock_dav`）。`Db` 用临时目录里的 SQLite 文件、`images_dir`
//! 也用 tempdir，测试结束自动清理。

use std::sync::Arc;

use axum::body::Body;
use axum::http::{header, Method, Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tempfile::TempDir;
use tower::ServiceExt;

use super::{build_router, AppState};
use crate::config::Config;
use crate::db::{repo, Db};
use crate::time::now_local_string;

const API_KEY: &str = "test-api-key-1234567890abcdef";

// =============================================================================
// 测试基础设施
// =============================================================================

/// 测试 fixture：一个 tempdir + Db + Router，调用方拿来直接 oneshot 请求。
/// TempDir 必须保留所有权直到测试结束（drop 时清理目录）。
struct Fixture {
    router: Router,
    state: AppState,
    _tmp: TempDir,
}

fn fixture() -> Fixture {
    fixture_with_webdav(None)
}

/// `webdav_url = None` 时指向不可达地址（127.0.0.1:0）；传入 mock WebDAV 的地址
/// 则 `/sync*` 端点会真的走一遍同步流程。
fn fixture_with_webdav(webdav_url: Option<&str>) -> Fixture {
    let tmp = TempDir::new().expect("tempdir");
    let data_dir = tmp.path().join("data");
    let images_dir = tmp.path().join("images");
    std::fs::create_dir_all(&data_dir).unwrap();
    std::fs::create_dir_all(&images_dir).unwrap();

    let mut cfg = Config::for_tests(API_KEY, data_dir.clone(), images_dir);
    if let Some(url) = webdav_url {
        cfg.webdav_url = url.to_string();
    }
    let cfg = Arc::new(cfg);
    let db = Db::open(&data_dir.join("data.db")).expect("open db");

    let state = AppState {
        config: cfg.clone(),
        db: db.clone(),
        sync: crate::sync::SyncCtx::new(cfg, db),
    };
    let router = build_router(state.clone());
    Fixture {
        router,
        state,
        _tmp: tmp,
    }
}

fn bearer() -> String {
    format!("Bearer {}", API_KEY)
}

/// 发请求并读完 body。
async fn send(router: &Router, req: Request<Body>) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
    let resp = router.clone().oneshot(req).await.expect("oneshot");
    let status = resp.status();
    let headers = resp.headers().clone();
    let bytes = resp
        .into_body()
        .collect()
        .await
        .expect("collect body")
        .to_bytes()
        .to_vec();
    (status, headers, bytes)
}

fn json_body(bytes: &[u8]) -> Value {
    serde_json::from_slice(bytes).expect("response body must be json")
}

fn req(method: Method, uri: &str, body: Option<Value>) -> Request<Body> {
    let mut b = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::AUTHORIZATION, bearer());
    let body = match body {
        Some(v) => {
            b = b.header(header::CONTENT_TYPE, "application/json");
            Body::from(v.to_string())
        }
        None => Body::empty(),
    };
    b.body(body).unwrap()
}

fn req_no_auth(method: Method, uri: &str) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .body(Body::empty())
        .unwrap()
}

// =============================================================================
// 鉴权
// =============================================================================

#[tokio::test]
async fn auth_missing_token_returns_401_without_sync_header() {
    let fx = fixture();
    let (status, headers, body) = send(&fx.router, req_no_auth(Method::GET, "/health")).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    // 鉴权是最外层：401 短路返回，不查库、不暴露同步状态
    assert!(
        !headers.contains_key("x-sync-status"),
        "401 response must not carry x-sync-status header"
    );
    assert!(!headers.contains_key("x-last-sync-at"));
    let v = json_body(&body);
    assert_eq!(v["error"], "unauthorized");
}

#[tokio::test]
async fn auth_wrong_token_returns_401() {
    let fx = fixture();
    let r = Request::builder()
        .method(Method::GET)
        .uri("/health")
        .header(header::AUTHORIZATION, "Bearer wrong-key")
        .body(Body::empty())
        .unwrap();
    let (status, _, body) = send(&fx.router, r).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let v = json_body(&body);
    assert_eq!(v["error"], "unauthorized");
}

#[tokio::test]
async fn auth_non_bearer_scheme_returns_401() {
    let fx = fixture();
    let r = Request::builder()
        .method(Method::GET)
        .uri("/health")
        .header(header::AUTHORIZATION, format!("Basic {}", API_KEY))
        .body(Body::empty())
        .unwrap();
    let (status, _, _) = send(&fx.router, r).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn auth_correct_token_passes() {
    let fx = fixture();
    let (status, headers, _) = send(&fx.router, req(Method::GET, "/todos", None)).await;
    assert_eq!(status, StatusCode::OK);
    // 鉴权通过的响应才带同步状态头
    assert!(headers.contains_key("x-sync-status"));
}

// =============================================================================
// /health
// =============================================================================

#[tokio::test]
async fn health_offline_when_no_pull() {
    let fx = fixture();
    let (status, headers, body) = send(&fx.router, req(Method::GET, "/health", None)).await;
    // 从未成功拉取 → 降级 → 503，并带出排查字段
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    let v = json_body(&body);
    assert_eq!(v["status"], "degraded");
    assert_eq!(v["sync"], "offline");
    assert_eq!(v["pull"], "offline");
    assert_eq!(v["push"], "healthy");
    assert!(v["lastPullAt"].is_null());
    assert_eq!(v["dirty"], false);
    assert_eq!(v["imageQueueLength"], 0);
    assert_eq!(headers.get("x-sync-status").unwrap(), "offline");
    // offline 时附 Warning header
    assert!(headers.contains_key("warning"));
}

#[tokio::test]
async fn health_healthy_after_meta_set() {
    let fx = fixture();
    let now = now_local_string(fx.state.config.timezone);
    fx.state
        .db
        .with_conn(|conn| repo::set_meta(conn, "last_pull_at", &now).unwrap());

    let (status, headers, body) = send(&fx.router, req(Method::GET, "/health", None)).await;
    assert_eq!(status, StatusCode::OK);
    let v = json_body(&body);
    assert_eq!(v["status"], "healthy");
    assert_eq!(v["sync"], "healthy");
    assert_eq!(v["lastPullAt"], json!(now));
    assert_eq!(headers.get("x-sync-status").unwrap(), "healthy");
    assert_eq!(headers.get("x-last-sync-at").unwrap(), now.as_str());
    assert!(!headers.contains_key("warning"));
}

// =============================================================================
// POST /todos
// =============================================================================

#[tokio::test]
async fn create_todo_minimal_succeeds_with_defaults() {
    let fx = fixture();
    let (status, _, body) = send(
        &fx.router,
        req(Method::POST, "/todos", Some(json!({"title": "first"}))),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let v = json_body(&body);
    assert_eq!(v["title"], "first");
    assert_eq!(v["completed"], false);
    assert_eq!(v["color"], "#10B981");
    assert_eq!(v["quadrant"], 4);
    assert_eq!(v["sortOrder"], 0);
    assert_eq!(v["notifyBefore"], 0);
    assert_eq!(v["notified"], false);
    assert!(v["createdAt"].is_string());
    assert!(v["updatedAt"].is_string());
    assert!(
        v["id"].is_number(),
        "id should be a number (i64 stringified -> parsed back)"
    );
}

/// K6 别名：priority → color、dueDate → endTime（仅日期 → 23:59:00）、notes → description；
/// 响应里的 priority 是由 color 派生的，别名字段本身不入库。
#[tokio::test]
async fn create_todo_maps_aliases_to_pc_fields() {
    let fx = fixture();
    let body = json!({
        "title": "with extras",
        "priority": "high",
        "quadrant": "urgent_important",
        "dueDate": "2026-05-20",
        "notes": "free text"
    });
    let (status, _, raw) = send(&fx.router, req(Method::POST, "/todos", Some(body))).await;
    assert_eq!(status, StatusCode::CREATED);
    let v = json_body(&raw);
    assert_eq!(v["color"], "#EF4444");
    assert_eq!(v["priority"], "high");
    assert_eq!(v["quadrant"], 1);
    assert_eq!(v["endTime"], "2026-05-20 23:59:00");
    assert_eq!(v["description"], "free text");
    assert!(v.get("dueDate").is_none() && v.get("notes").is_none());

    let stored = stored_todo(&fx, &todo_id_path(&v));
    assert!(
        stored.get("priority").is_none(),
        "派生字段不入库: {}",
        stored
    );
    assert!(stored.get("seq").is_none());
    assert_eq!(stored["color"], "#EF4444");

    // 同时给了 color：以 color 为准，派生优先级随之变化
    let v = create_todo(
        &fx,
        json!({"title": "c", "priority": "high", "color": "#3b82f6"}),
    )
    .await;
    assert_eq!(v["color"], "#3B82F6");
    assert!(v["priority"].is_null(), "自定义颜色的派生优先级为 null");
}

#[tokio::test]
async fn create_todo_missing_title_returns_400() {
    let fx = fixture();
    let (status, _, body) = send(
        &fx.router,
        req(Method::POST, "/todos", Some(json!({"completed": true}))),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let v = json_body(&body);
    assert_eq!(v["error"], "bad_request");
    assert!(
        v["detail"].as_str().unwrap().contains("title"),
        "detail should mention title"
    );
}

#[tokio::test]
async fn create_todo_blank_title_returns_400() {
    let fx = fixture();
    let (status, _, _) = send(
        &fx.router,
        req(Method::POST, "/todos", Some(json!({"title": "   "}))),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn create_todo_non_object_body_returns_400() {
    let fx = fixture();
    let (status, _, _) = send(
        &fx.router,
        req(Method::POST, "/todos", Some(json!([1, 2, 3]))),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn create_todo_sets_dirty_flag() {
    let fx = fixture();
    let _ = send(
        &fx.router,
        req(Method::POST, "/todos", Some(json!({"title": "dirty"}))),
    )
    .await;
    let dirty = fx
        .state
        .db
        .with_conn(|c| repo::get_meta(c, "dirty"))
        .unwrap();
    assert_eq!(dirty.as_deref(), Some("true"));
    // 写路径必须走 mark_dirty：generation 同步递增，push 才能判断
    // "推送窗口期内是否又有新写入"
    let gen = fx
        .state
        .db
        .with_conn(|c| repo::get_dirty_generation(c))
        .unwrap();
    assert_eq!(gen, 1, "写路径必须递增 dirty_generation");
}

// =============================================================================
// GET /todos & GET /todos/:id
// =============================================================================

async fn create_todo(fx: &Fixture, body: Value) -> Value {
    let (status, _, raw) = send(&fx.router, req(Method::POST, "/todos", Some(body))).await;
    assert_eq!(status, StatusCode::CREATED, "create failed");
    json_body(&raw)
}

fn todo_id_path(v: &Value) -> String {
    v["id"].as_i64().expect("id is number").to_string()
}

/// 缓存里实际存的 data_json。
fn stored_todo(fx: &Fixture, id: &str) -> Value {
    let row = fx
        .state
        .db
        .with_conn(|c| repo::get_todo(c, id).unwrap())
        .expect("row exists");
    serde_json::from_str(&row.data_json).unwrap()
}

#[tokio::test]
async fn list_todos_empty() {
    let fx = fixture();
    let (status, _, raw) = send(&fx.router, req(Method::GET, "/todos", None)).await;
    assert_eq!(status, StatusCode::OK);
    let v = json_body(&raw);
    assert_eq!(v, json!([]));
}

#[tokio::test]
async fn list_todos_returns_subtask_count_when_not_nested() {
    let fx = fixture();
    let t = create_todo(&fx, json!({"title": "t1"})).await;
    let id = todo_id_path(&t);
    // 加 1 个 subtask
    let (status, _, _) = send(
        &fx.router,
        req(
            Method::POST,
            &format!("/todos/{}/subtasks", id),
            Some(json!({"title": "s1"})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, _, raw) = send(&fx.router, req(Method::GET, "/todos", None)).await;
    assert_eq!(status, StatusCode::OK);
    let v = json_body(&raw);
    assert_eq!(v[0]["subtaskCount"], 1);
    assert!(
        v[0].get("subtasks").is_none(),
        "subtasks array should be stripped when not nested"
    );
}

#[tokio::test]
async fn list_todos_with_subtasks_inlines_array() {
    let fx = fixture();
    let t = create_todo(&fx, json!({"title": "t1"})).await;
    let id = todo_id_path(&t);
    let _ = send(
        &fx.router,
        req(
            Method::POST,
            &format!("/todos/{}/subtasks", id),
            Some(json!({"title": "s1"})),
        ),
    )
    .await;

    let (status, _, raw) = send(
        &fx.router,
        req(Method::GET, "/todos?withSubtasks=true", None),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let v = json_body(&raw);
    assert!(v[0]["subtasks"].is_array());
    assert_eq!(v[0]["subtasks"].as_array().unwrap().len(), 1);
    assert_eq!(v[0]["subtasks"][0]["title"], "s1");
}

#[tokio::test]
async fn list_todos_filter_completed() {
    let fx = fixture();
    let _ = create_todo(&fx, json!({"title": "done", "completed": true})).await;
    let _ = create_todo(&fx, json!({"title": "open", "completed": false})).await;

    let (_, _, raw) = send(&fx.router, req(Method::GET, "/todos?completed=true", None)).await;
    let v = json_body(&raw);
    assert_eq!(v.as_array().unwrap().len(), 1);
    assert_eq!(v[0]["title"], "done");

    let (_, _, raw) = send(&fx.router, req(Method::GET, "/todos?completed=false", None)).await;
    let v = json_body(&raw);
    assert_eq!(v.as_array().unwrap().len(), 1);
    assert_eq!(v[0]["title"], "open");
}

#[tokio::test]
async fn list_todos_filter_completed_accepts_aliases() {
    let fx = fixture();
    let _ = create_todo(&fx, json!({"title": "done", "completed": true})).await;
    for alias in &["1", "yes", "TRUE"] {
        let uri = format!("/todos?completed={}", alias);
        let (status, _, raw) = send(&fx.router, req(Method::GET, &uri, None)).await;
        assert_eq!(status, StatusCode::OK, "alias {} should be OK", alias);
        let v = json_body(&raw);
        assert_eq!(v.as_array().unwrap().len(), 1, "alias {}", alias);
    }
}

#[tokio::test]
async fn list_todos_filter_invalid_completed_returns_400() {
    let fx = fixture();
    let (status, _, body) =
        send(&fx.router, req(Method::GET, "/todos?completed=maybe", None)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let v = json_body(&body);
    assert_eq!(v["error"], "bad_request");
}

#[tokio::test]
async fn list_todos_filter_priority() {
    let fx = fixture();
    let _ = create_todo(&fx, json!({"title": "h", "priority": "high"})).await;
    let _ = create_todo(&fx, json!({"title": "l", "priority": "low"})).await;
    let (_, _, raw) = send(&fx.router, req(Method::GET, "/todos?priority=high", None)).await;
    let v = json_body(&raw);
    assert_eq!(v.as_array().unwrap().len(), 1);
    assert_eq!(v[0]["title"], "h");
}

#[tokio::test]
async fn list_todos_quadrant_numeric_and_alias() {
    let fx = fixture();
    let _ = create_todo(&fx, json!({"title": "Q1", "quadrant": 1})).await;
    let _ = create_todo(&fx, json!({"title": "Q2", "quadrant": 2})).await;

    let (_, _, raw) = send(&fx.router, req(Method::GET, "/todos?quadrant=1", None)).await;
    let v = json_body(&raw);
    assert_eq!(v.as_array().unwrap().len(), 1);
    assert_eq!(v[0]["title"], "Q1");

    let (_, _, raw) = send(
        &fx.router,
        req(Method::GET, "/todos?quadrant=urgent_important", None),
    )
    .await;
    let v = json_body(&raw);
    assert_eq!(v.as_array().unwrap().len(), 1);
    assert_eq!(v[0]["title"], "Q1");

    let (_, _, raw) = send(
        &fx.router,
        req(Method::GET, "/todos?quadrant=important_not_urgent", None),
    )
    .await;
    let v = json_body(&raw);
    assert_eq!(v.as_array().unwrap().len(), 1);
    assert_eq!(v[0]["title"], "Q2");
}

#[tokio::test]
async fn list_todos_quadrant_invalid_returns_400() {
    let fx = fixture();
    let (status, _, _) = send(&fx.router, req(Method::GET, "/todos?quadrant=foo", None)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _, _) = send(&fx.router, req(Method::GET, "/todos?quadrant=5", None)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn list_todos_due_date_before_after() {
    let fx = fixture();
    let _ = create_todo(&fx, json!({"title": "a", "dueDate": "2026-05-13"})).await;
    let _ = create_todo(&fx, json!({"title": "b", "dueDate": "2026-05-20"})).await;
    let (_, _, raw) = send(
        &fx.router,
        req(Method::GET, "/todos?dueDateBefore=2026-05-15", None),
    )
    .await;
    let v = json_body(&raw);
    assert_eq!(v.as_array().unwrap().len(), 1);
    assert_eq!(v[0]["title"], "a");

    let (_, _, raw) = send(
        &fx.router,
        req(Method::GET, "/todos?dueDateAfter=2026-05-15", None),
    )
    .await;
    let v = json_body(&raw);
    assert_eq!(v.as_array().unwrap().len(), 1);
    assert_eq!(v[0]["title"], "b");
}

/// Regression: GET /todos?dueDateBefore=... 时无截止时间的 todo 不该出现在结果里。
/// 这是 cmd_today 的 overdue 分支用同样 query 时把"买洗内裤的"误判为过期的根因。
#[tokio::test]
async fn list_todos_due_date_before_skips_todos_without_anchor() {
    let fx = fixture();
    // A: 真正过期
    let _ = create_todo(&fx, json!({"title": "overdue", "dueDate": "2026-05-10"})).await;
    // B: 无任何时间锚（典型场景：用户只填了标题）
    let _ = create_todo(&fx, json!({"title": "no anchor"})).await;
    // C: 未来到期，不该被 before 命中
    let _ = create_todo(&fx, json!({"title": "future", "dueDate": "2026-05-30"})).await;

    let (status, _, raw) = send(
        &fx.router,
        req(
            Method::GET,
            "/todos?dueDateBefore=2026-05-14T00:00:00",
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let v = json_body(&raw);
    let arr = v.as_array().unwrap();
    assert_eq!(
        arr.len(),
        1,
        "only 'overdue' should match, no-anchor must be excluded"
    );
    assert_eq!(arr[0]["title"], "overdue");
}

#[tokio::test]
async fn list_todos_search_q_matches_title_and_notes() {
    let fx = fixture();
    let _ = create_todo(&fx, json!({"title": "buy milk"})).await;
    let _ = create_todo(&fx, json!({"title": "x", "notes": "milk later"})).await;
    let _ = create_todo(&fx, json!({"title": "unrelated"})).await;
    let (_, _, raw) = send(&fx.router, req(Method::GET, "/todos?q=milk", None)).await;
    let v = json_body(&raw);
    assert_eq!(v.as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn list_todos_sort_by_priority_desc() {
    let fx = fixture();
    let _ = create_todo(&fx, json!({"title": "lo", "priority": "low"})).await;
    let _ = create_todo(&fx, json!({"title": "hi", "priority": "high"})).await;
    let _ = create_todo(&fx, json!({"title": "md", "priority": "medium"})).await;
    let (_, _, raw) = send(&fx.router, req(Method::GET, "/todos?sort=-priority", None)).await;
    let v = json_body(&raw);
    let titles: Vec<String> = v
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x["title"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(titles, vec!["hi", "md", "lo"]);
}

#[tokio::test]
async fn list_todos_limit_offset() {
    let fx = fixture();
    for i in 0..5 {
        let _ = create_todo(&fx, json!({"title": format!("t{}", i), "sortOrder": i})).await;
    }
    let (_, _, raw) = send(
        &fx.router,
        req(Method::GET, "/todos?sort=+sortOrder&limit=2&offset=1", None),
    )
    .await;
    let v = json_body(&raw);
    assert_eq!(v.as_array().unwrap().len(), 2);
    assert_eq!(v[0]["title"], "t1");
    assert_eq!(v[1]["title"], "t2");
}

#[tokio::test]
async fn get_todo_default_nests_subtasks() {
    let fx = fixture();
    let t = create_todo(&fx, json!({"title": "parent"})).await;
    let id = todo_id_path(&t);
    let _ = send(
        &fx.router,
        req(
            Method::POST,
            &format!("/todos/{}/subtasks", id),
            Some(json!({"title": "child"})),
        ),
    )
    .await;
    let (status, _, raw) = send(
        &fx.router,
        req(Method::GET, &format!("/todos/{}", id), None),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let v = json_body(&raw);
    assert_eq!(v["title"], "parent");
    assert!(v["subtasks"].is_array());
    assert_eq!(v["subtasks"][0]["title"], "child");
}

#[tokio::test]
async fn get_todo_with_subtasks_false_uses_count() {
    let fx = fixture();
    let t = create_todo(&fx, json!({"title": "parent"})).await;
    let id = todo_id_path(&t);
    let _ = send(
        &fx.router,
        req(
            Method::POST,
            &format!("/todos/{}/subtasks", id),
            Some(json!({"title": "child"})),
        ),
    )
    .await;
    let (status, _, raw) = send(
        &fx.router,
        req(
            Method::GET,
            &format!("/todos/{}?withSubtasks=false", id),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let v = json_body(&raw);
    assert!(
        v.get("subtasks").is_none(),
        "subtasks should be absent when false"
    );
    assert_eq!(v["subtaskCount"], 1);
}

#[tokio::test]
async fn get_todo_not_found_returns_404() {
    let fx = fixture();
    let (status, _, body) = send(&fx.router, req(Method::GET, "/todos/999999", None)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let v = json_body(&body);
    assert_eq!(v["error"], "not_found");
}

// =============================================================================
// PATCH /todos/:id
// =============================================================================

#[tokio::test]
async fn patch_todo_merges_fields_and_updates_updated_at() {
    let fx = fixture();
    let t = create_todo(
        &fx,
        json!({"title": "old", "priority": "low", "description": "kept", "color": "#000000"}),
    )
    .await;
    let id = todo_id_path(&t);
    let old_updated = t["updatedAt"].as_str().unwrap().to_string();

    // 等 1s 让 updated_at 至少差一秒（时间格式精度是秒）
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;

    let (status, _, raw) = send(
        &fx.router,
        req(
            Method::PATCH,
            &format!("/todos/{}", id),
            Some(json!({"title": "new", "completed": true})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let v = json_body(&raw);
    assert_eq!(v["title"], "new");
    assert_eq!(v["completed"], true);
    // 未提及字段保留
    assert_eq!(v["color"], "#000000");
    assert_eq!(v["description"], "kept");
    // updatedAt 必须前进
    assert!(
        v["updatedAt"].as_str().unwrap() > old_updated.as_str(),
        "updatedAt should advance: was {}, now {}",
        old_updated,
        v["updatedAt"]
    );
}

#[tokio::test]
async fn patch_todo_cannot_change_id() {
    let fx = fixture();
    let t = create_todo(&fx, json!({"title": "x"})).await;
    let id = todo_id_path(&t);
    let (status, _, raw) = send(
        &fx.router,
        req(
            Method::PATCH,
            &format!("/todos/{}", id),
            Some(json!({"id": 12345})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let v = json_body(&raw);
    // id 仍是原值
    assert_eq!(v["id"].as_i64().unwrap().to_string(), id);
}

#[tokio::test]
async fn patch_todo_null_value_explicitly_writes_null() {
    let fx = fixture();
    let t = create_todo(
        &fx,
        json!({"title": "x", "description": "kept", "endTime": "2026-05-20 10:00"}),
    )
    .await;
    let id = todo_id_path(&t);
    let (status, _, raw) = send(
        &fx.router,
        req(
            Method::PATCH,
            &format!("/todos/{}", id),
            Some(json!({"description": null, "endTime": ""})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let v = json_body(&raw);
    assert!(v["description"].is_null());
    assert!(v["endTime"].is_null(), "空串清空时间字段");
    assert!(v.as_object().unwrap().contains_key("description"));
}

#[tokio::test]
async fn patch_todo_not_found_returns_404() {
    let fx = fixture();
    let (status, _, _) = send(
        &fx.router,
        req(Method::PATCH, "/todos/999999", Some(json!({"title": "x"}))),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn patch_todo_non_object_body_returns_400() {
    let fx = fixture();
    let t = create_todo(&fx, json!({"title": "x"})).await;
    let id = todo_id_path(&t);
    let (status, _, _) = send(
        &fx.router,
        req(
            Method::PATCH,
            &format!("/todos/{}", id),
            Some(json!("oops")),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

// =============================================================================
// DELETE /todos/:id
// =============================================================================

#[tokio::test]
async fn delete_todo_cascades_and_writes_tombstones() {
    let fx = fixture();
    let t = create_todo(&fx, json!({"title": "parent"})).await;
    let id = todo_id_path(&t);
    let (_, _, sub_raw) = send(
        &fx.router,
        req(
            Method::POST,
            &format!("/todos/{}/subtasks", id),
            Some(json!({"title": "s"})),
        ),
    )
    .await;
    let sub_id = json_body(&sub_raw)["id"].as_i64().unwrap().to_string();

    let (status, _, _) = send(
        &fx.router,
        req(Method::DELETE, &format!("/todos/{}", id), None),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // GET 404
    let (s, _, _) = send(
        &fx.router,
        req(Method::GET, &format!("/todos/{}", id), None),
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND);

    // tombstones：todo + subtask 都该写入
    let stones = fx.state.db.with_conn(|c| repo::list_tombstones(c).unwrap());
    let kinds: Vec<(String, String)> = stones
        .iter()
        .map(|(t, i, _)| (t.clone(), i.clone()))
        .collect();
    assert!(
        kinds.contains(&("todo".to_string(), id.clone())),
        "todo tombstone missing"
    );
    assert!(
        kinds.contains(&("subtask".to_string(), sub_id)),
        "subtask tombstone missing"
    );
}

#[tokio::test]
async fn delete_todo_not_found_returns_404() {
    let fx = fixture();
    let (status, _, _) = send(&fx.router, req(Method::DELETE, "/todos/999999", None)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

// =============================================================================
// /todos/:id/subtasks (POST) + /subtasks/:id (PATCH/DELETE)
// =============================================================================

#[tokio::test]
async fn create_subtask_under_missing_todo_returns_404() {
    let fx = fixture();
    let (status, _, body) = send(
        &fx.router,
        req(
            Method::POST,
            "/todos/999999/subtasks",
            Some(json!({"title": "s"})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let v = json_body(&body);
    assert_eq!(v["error"], "not_found");
}

#[tokio::test]
async fn create_subtask_missing_title_returns_400() {
    let fx = fixture();
    let t = create_todo(&fx, json!({"title": "p"})).await;
    let id = todo_id_path(&t);
    let (status, _, _) = send(
        &fx.router,
        req(
            Method::POST,
            &format!("/todos/{}/subtasks", id),
            Some(json!({})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn create_subtask_assigns_parent_id_and_defaults() {
    let fx = fixture();
    let t = create_todo(&fx, json!({"title": "p"})).await;
    let parent = todo_id_path(&t);
    let (status, _, raw) = send(
        &fx.router,
        req(
            Method::POST,
            &format!("/todos/{}/subtasks", parent),
            Some(json!({"title": "child"})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let v = json_body(&raw);
    assert_eq!(v["title"], "child");
    assert_eq!(v["completed"], false);
    assert_eq!(v["sortOrder"], 0);
    assert!(v["content"].is_null());
    assert_eq!(v["parentId"].as_i64().unwrap().to_string(), parent);
    assert!(v["createdAt"].is_string());
}

#[tokio::test]
async fn patch_subtask_merges() {
    let fx = fixture();
    let t = create_todo(&fx, json!({"title": "p"})).await;
    let parent = todo_id_path(&t);
    let (_, _, raw) = send(
        &fx.router,
        req(
            Method::POST,
            &format!("/todos/{}/subtasks", parent),
            Some(json!({"title": "old", "content": "kept"})),
        ),
    )
    .await;
    let sub = json_body(&raw);
    let sid = sub["id"].as_i64().unwrap().to_string();

    let (status, _, raw) = send(
        &fx.router,
        req(
            Method::PATCH,
            &format!("/subtasks/{}", sid),
            Some(json!({"completed": true})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let v = json_body(&raw);
    assert_eq!(v["title"], "old");
    assert_eq!(v["completed"], true);
    assert_eq!(v["content"], "kept");
}

#[tokio::test]
async fn patch_subtask_not_found_returns_404() {
    let fx = fixture();
    let (status, _, _) = send(
        &fx.router,
        req(
            Method::PATCH,
            "/subtasks/999999",
            Some(json!({"title": "x"})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn delete_subtask_writes_tombstone() {
    let fx = fixture();
    let t = create_todo(&fx, json!({"title": "p"})).await;
    let parent = todo_id_path(&t);
    let (_, _, raw) = send(
        &fx.router,
        req(
            Method::POST,
            &format!("/todos/{}/subtasks", parent),
            Some(json!({"title": "s"})),
        ),
    )
    .await;
    let sid = json_body(&raw)["id"].as_i64().unwrap().to_string();

    let (status, _, _) = send(
        &fx.router,
        req(Method::DELETE, &format!("/subtasks/{}", sid), None),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let stones = fx.state.db.with_conn(|c| repo::list_tombstones(c).unwrap());
    assert!(stones.iter().any(|(t, i, _)| t == "subtask" && i == &sid));
}

#[tokio::test]
async fn delete_subtask_not_found_returns_404() {
    let fx = fixture();
    let (status, _, _) = send(&fx.router, req(Method::DELETE, "/subtasks/999999", None)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

// =============================================================================
// /images
// =============================================================================

fn multipart_body(boundary: &str, filename: &str, ct: &str, bytes: &[u8]) -> Vec<u8> {
    let mut body = Vec::new();
    body.extend_from_slice(format!("--{}\r\n", boundary).as_bytes());
    body.extend_from_slice(
        format!(
            "Content-Disposition: form-data; name=\"file\"; filename=\"{}\"\r\n",
            filename
        )
        .as_bytes(),
    );
    body.extend_from_slice(format!("Content-Type: {}\r\n\r\n", ct).as_bytes());
    body.extend_from_slice(bytes);
    body.extend_from_slice(format!("\r\n--{}--\r\n", boundary).as_bytes());
    body
}

#[tokio::test]
async fn upload_image_then_fetch_roundtrip() {
    let fx = fixture();
    let boundary = "----test-boundary";
    let payload = b"\x89PNG\r\n\x1a\nFAKEDATA";
    let body = multipart_body(boundary, "pic.png", "image/png", payload);

    let r = Request::builder()
        .method(Method::POST)
        .uri("/images")
        .header(header::AUTHORIZATION, bearer())
        .header(
            header::CONTENT_TYPE,
            format!("multipart/form-data; boundary={}", boundary),
        )
        .body(Body::from(body))
        .unwrap();
    let (status, _, raw) = send(&fx.router, r).await;
    assert_eq!(status, StatusCode::OK);
    let v = json_body(&raw);
    let name = v["name"].as_str().unwrap().to_string();
    assert!(name.starts_with("img_"));
    assert!(name.ends_with(".png"));

    // GET 拿回原 bytes
    let (s, headers, got) = send(
        &fx.router,
        req(Method::GET, &format!("/images/{}", name), None),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(headers.get(header::CONTENT_TYPE).unwrap(), "image/png");
    assert_eq!(got.as_slice(), payload);

    // dirty_images meta 也要记录
    let dirty_images = fx
        .state
        .db
        .with_conn(|c| repo::get_meta(c, "dirty_images"))
        .unwrap();
    let arr: Vec<String> = serde_json::from_str(dirty_images.as_deref().unwrap_or("[]")).unwrap();
    assert!(arr.contains(&name));
}

#[tokio::test]
async fn upload_image_non_image_ext_falls_back_to_bin() {
    let fx = fixture();
    let boundary = "----test-boundary2";
    let body = multipart_body(boundary, "danger.exe", "application/octet-stream", b"AAAA");
    let r = Request::builder()
        .method(Method::POST)
        .uri("/images")
        .header(header::AUTHORIZATION, bearer())
        .header(
            header::CONTENT_TYPE,
            format!("multipart/form-data; boundary={}", boundary),
        )
        .body(Body::from(body))
        .unwrap();
    let (status, _, raw) = send(&fx.router, r).await;
    assert_eq!(status, StatusCode::OK);
    let v = json_body(&raw);
    assert!(
        v["name"].as_str().unwrap().ends_with(".bin"),
        "non-image ext must fall back to .bin, got {}",
        v["name"]
    );
}

#[tokio::test]
async fn upload_image_missing_file_part_returns_400() {
    let fx = fixture();
    let boundary = "----empty-boundary";
    // multipart 完全没有任何 part
    let body = format!("--{}--\r\n", boundary);
    let r = Request::builder()
        .method(Method::POST)
        .uri("/images")
        .header(header::AUTHORIZATION, bearer())
        .header(
            header::CONTENT_TYPE,
            format!("multipart/form-data; boundary={}", boundary),
        )
        .body(Body::from(body))
        .unwrap();
    let (status, _, _) = send(&fx.router, r).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn get_image_not_found_returns_404() {
    let fx = fixture();
    let (status, _, body) = send(&fx.router, req(Method::GET, "/images/nope.png", None)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let v = json_body(&body);
    assert_eq!(v["error"], "not_found");
}

#[tokio::test]
async fn get_image_rejects_path_traversal() {
    let fx = fixture();
    // axum 把 `..` 在 router 层就归一化，所以这里测一个明显非法字符串
    // 走到 handler 的情况：包含 backslash 的 percent-encoded 名字
    let (status, _, _) = send(&fx.router, req(Method::GET, "/images/%2e%2e", None)).await;
    // 解码出来是 ".."，sanitize_filename 拒绝
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

// =============================================================================
// 通用错误体格式
// =============================================================================

// =============================================================================
// seq 短码（cloud-only `C{seq}` 引用）
// =============================================================================

#[tokio::test]
async fn create_todo_response_contains_seq_starting_from_1() {
    let fx = fixture();
    let a = create_todo(&fx, json!({"title": "a"})).await;
    let b = create_todo(&fx, json!({"title": "b"})).await;
    let c = create_todo(&fx, json!({"title": "c"})).await;
    assert_eq!(a["seq"], 1);
    assert_eq!(b["seq"], 2);
    assert_eq!(c["seq"], 3);
}

#[tokio::test]
async fn list_todos_response_carries_seq_for_each_item() {
    let fx = fixture();
    let _ = create_todo(&fx, json!({"title": "a"})).await;
    let _ = create_todo(&fx, json!({"title": "b"})).await;
    let (_, _, raw) = send(&fx.router, req(Method::GET, "/todos", None)).await;
    let v = json_body(&raw);
    let arr = v.as_array().unwrap();
    assert_eq!(arr.len(), 2);
    assert!(arr[0]["seq"].is_number());
    assert!(arr[1]["seq"].is_number());
    let seqs: Vec<i64> = arr.iter().map(|x| x["seq"].as_i64().unwrap()).collect();
    let mut sorted = seqs.clone();
    sorted.sort();
    assert_eq!(sorted, vec![1, 2]);
}

#[tokio::test]
async fn get_todo_by_c_prefix_seq() {
    let fx = fixture();
    let t = create_todo(&fx, json!({"title": "first"})).await;
    assert_eq!(t["seq"], 1);
    let (status, _, raw) = send(&fx.router, req(Method::GET, "/todos/C1", None)).await;
    assert_eq!(status, StatusCode::OK);
    let v = json_body(&raw);
    assert_eq!(v["title"], "first");
    assert_eq!(v["seq"], 1);
    assert_eq!(v["id"], t["id"]);
}

#[tokio::test]
async fn get_todo_by_c_prefix_is_case_insensitive() {
    let fx = fixture();
    let _ = create_todo(&fx, json!({"title": "first"})).await;
    let (status, _, raw) = send(&fx.router, req(Method::GET, "/todos/c1", None)).await;
    assert_eq!(status, StatusCode::OK);
    let v = json_body(&raw);
    assert_eq!(v["title"], "first");
}

#[tokio::test]
async fn get_todo_by_internal_id_still_works() {
    let fx = fixture();
    let t = create_todo(&fx, json!({"title": "x"})).await;
    let id = todo_id_path(&t);
    let (status, _, raw) = send(
        &fx.router,
        req(Method::GET, &format!("/todos/{}", id), None),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let v = json_body(&raw);
    assert_eq!(v["seq"], 1);
}

#[tokio::test]
async fn get_todo_unknown_seq_returns_404() {
    let fx = fixture();
    let _ = create_todo(&fx, json!({"title": "x"})).await;
    let (status, _, _) = send(&fx.router, req(Method::GET, "/todos/C999", None)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn get_todo_malformed_seq_returns_404() {
    let fx = fixture();
    let _ = create_todo(&fx, json!({"title": "x"})).await;
    let (status, _, _) = send(&fx.router, req(Method::GET, "/todos/Cfoo", None)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn patch_todo_by_c_prefix() {
    let fx = fixture();
    let _ = create_todo(&fx, json!({"title": "old"})).await;
    let (status, _, raw) = send(
        &fx.router,
        req(Method::PATCH, "/todos/C1", Some(json!({"title": "new"}))),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let v = json_body(&raw);
    assert_eq!(v["title"], "new");
    assert_eq!(v["seq"], 1);
}

#[tokio::test]
async fn delete_todo_by_c_prefix_removes_seq() {
    let fx = fixture();
    let _ = create_todo(&fx, json!({"title": "x"})).await;
    let (status, _, _) = send(&fx.router, req(Method::DELETE, "/todos/C1", None)).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    // 再 GET /todos/C1 → 404，且 todo_seq 表里这条 row 也已清理
    let (status, _, _) = send(&fx.router, req(Method::GET, "/todos/C1", None)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let leftover = fx
        .state
        .db
        .with_conn(|c| repo::get_todo_id_by_seq(c, 1).unwrap());
    assert!(
        leftover.is_none(),
        "todo_seq row should be cleaned up on delete"
    );
}

#[tokio::test]
async fn seq_does_not_recycle_after_delete() {
    let fx = fixture();
    let _ = create_todo(&fx, json!({"title": "a"})).await; // seq=1
    let _ = create_todo(&fx, json!({"title": "b"})).await; // seq=2
                                                           // 删 seq=1
    let (status, _, _) = send(&fx.router, req(Method::DELETE, "/todos/C1", None)).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    // 新建：seq 应该是 3，不复用 1
    let c = create_todo(&fx, json!({"title": "c"})).await;
    assert_eq!(c["seq"], 3, "seq must NOT recycle deleted numbers");
}

#[tokio::test]
async fn create_subtask_under_c_prefix_parent() {
    let fx = fixture();
    let t = create_todo(&fx, json!({"title": "p"})).await;
    let parent_id = todo_id_path(&t);
    let (status, _, raw) = send(
        &fx.router,
        req(
            Method::POST,
            "/todos/C1/subtasks",
            Some(json!({"title": "child"})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let v = json_body(&raw);
    assert_eq!(v["title"], "child");
    assert_eq!(v["parentId"].as_i64().unwrap().to_string(), parent_id);
}

#[tokio::test]
async fn pull_backfill_assigns_seq_to_pc_origin_todo() {
    // 模拟"PC 端创建的 todo 通过 pull 进入 cloud SQLite"：直接 upsert_todo，
    // 不走 API（API 才会 assign_seq）。然后调 backfill 验证它能被分配 seq。
    let fx = fixture();
    let now = now_local_string(fx.state.config.timezone);
    fx.state.db.with_conn(|conn| {
        repo::upsert_todo(conn, "42", r#"{"id":42,"title":"from PC"}"#, &now).unwrap();
    });
    // 现在没有 seq
    let before = fx.state.db.with_conn(|c| repo::get_seq(c, "42").unwrap());
    assert!(before.is_none());

    // 调回填
    let n = crate::sync::pull::backfill_missing_seq(&fx.state.db).unwrap();
    assert_eq!(n, 1);

    let after = fx.state.db.with_conn(|c| repo::get_seq(c, "42").unwrap());
    assert_eq!(after, Some(1));

    // 用 C1 也能查得到
    let (status, _, raw) = send(&fx.router, req(Method::GET, "/todos/C1", None)).await;
    assert_eq!(status, StatusCode::OK);
    let v = json_body(&raw);
    assert_eq!(v["title"], "from PC");
    assert_eq!(v["seq"], 1);
}

#[tokio::test]
async fn pull_backfill_is_idempotent() {
    let fx = fixture();
    let _ = create_todo(&fx, json!({"title": "a"})).await; // 已分配 seq=1
                                                           // 第二次回填什么也不应该改
    let n = crate::sync::pull::backfill_missing_seq(&fx.state.db).unwrap();
    assert_eq!(n, 0);
}

// =============================================================================
// POST /sync 系列
//
// 测试 Config 的 webdav_url 指向不可达地址（127.0.0.1:0），pull 必然失败；
// push 在 meta.dirty 未置位时是 no-op 不触网。借此覆盖路由注册、鉴权、
// 部分失败的 207 语义和响应 shape，全程无真实网络。
// =============================================================================

#[tokio::test]
async fn sync_route_requires_auth() {
    let fx = fixture();
    let (status, _, _) = send(&fx.router, req_no_auth(Method::POST, "/sync")).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn sync_route_reports_partial_failure_as_207() {
    let fx = fixture();
    let (status, _, body) = send(&fx.router, req(Method::POST, "/sync", None)).await;
    assert_eq!(status, StatusCode::MULTI_STATUS);
    let v = json_body(&body);
    assert_eq!(v["pull"], "error");
    assert!(v["pullError"].is_string(), "pull 失败必须带 pullError 详情");
    // db 干净（dirty 未置位）→ push 是 no-op，成功
    assert_eq!(v["push"], "ok");
    assert!(v.get("pushError").is_none() || v["pushError"].is_null());
}

#[tokio::test]
async fn sync_pull_route_returns_500_when_webdav_unreachable() {
    let fx = fixture();
    let (status, _, body) = send(&fx.router, req(Method::POST, "/sync/pull", None)).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    let v = json_body(&body);
    assert!(v.get("error").is_some());
}

#[tokio::test]
async fn sync_push_route_ok_when_not_dirty() {
    let fx = fixture();
    let (status, _, body) = send(&fx.router, req(Method::POST, "/sync/push", None)).await;
    assert_eq!(status, StatusCode::OK);
    let v = json_body(&body);
    assert_eq!(v["status"], "ok");
}

#[tokio::test]
async fn sync_push_route_fails_when_dirty_and_webdav_unreachable() {
    let fx = fixture();
    // 通过 API 创建 todo 会置 dirty=true，push 将真正尝试连 WebDAV → 失败
    let _ = create_todo(&fx, json!({"title": "trigger dirty"})).await;
    let (status, _, _) = send(&fx.router, req(Method::POST, "/sync/push", None)).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    // PUT 没成功过，dirty 必须保持 true，等下一轮重试
    let dirty = fx
        .state
        .db
        .with_conn(|c| repo::get_meta(c, "dirty"))
        .unwrap();
    assert_eq!(dirty.as_deref(), Some("true"));
}

// =============================================================================
// 示例：打印 /todos 真实响应 shape（cargo test demo_ -- --ignored --nocapture）
// =============================================================================

#[tokio::test]
#[ignore]
async fn demo_print_todos_responses() {
    let fx = fixture();

    // 1) POST /todos 最小创建
    let (_, _, raw) = send(
        &fx.router,
        req(Method::POST, "/todos", Some(json!({"title": "买菜"}))),
    )
    .await;
    let v: Value = json_body(&raw);
    println!(
        "\n==== POST /todos 最小创建响应 ====\n{}",
        serde_json::to_string_pretty(&v).unwrap()
    );

    // 2) POST /todos 富字段
    let (_, _, raw) = send(
        &fx.router,
        req(
            Method::POST,
            "/todos",
            Some(json!({
                "title": "写周报",
                "priority": "high",
                "quadrant": 1,
                "color": "#EF4444",
                "dueDate": "2026-05-20 18:00:00",
                "startTime": "2026-05-20 09:00:00",
                "notifyBefore": 30,
                "notes": "重点：本月 KPI"
            })),
        ),
    )
    .await;
    let with_extras: Value = json_body(&raw);
    let parent_id = with_extras["id"].as_i64().unwrap().to_string();
    println!(
        "\n==== POST /todos 富字段响应 ====\n{}",
        serde_json::to_string_pretty(&with_extras).unwrap()
    );

    // 3) 加两个子任务
    for (i, title) in ["收集数据", "写文档"].iter().enumerate() {
        let _ = send(
            &fx.router,
            req(
                Method::POST,
                &format!("/todos/{}/subtasks", parent_id),
                Some(json!({"title": title, "sortOrder": i})),
            ),
        )
        .await;
    }

    // 4) GET /todos 默认（不嵌套，含 subtaskCount）
    let (_, headers, raw) = send(&fx.router, req(Method::GET, "/todos", None)).await;
    let v: Value = json_body(&raw);
    println!(
        "\n==== GET /todos （默认，subtaskCount）====\nheaders: x-sync-status={:?}\n{}",
        headers.get("x-sync-status").map(|h| h.to_str().unwrap()),
        serde_json::to_string_pretty(&v).unwrap()
    );

    // 5) GET /todos?withSubtasks=true
    let (_, _, raw) = send(
        &fx.router,
        req(Method::GET, "/todos?withSubtasks=true", None),
    )
    .await;
    let v: Value = json_body(&raw);
    println!(
        "\n==== GET /todos?withSubtasks=true ====\n{}",
        serde_json::to_string_pretty(&v).unwrap()
    );

    // 6) GET /todos/:id 默认嵌套
    let (_, _, raw) = send(
        &fx.router,
        req(Method::GET, &format!("/todos/{}", parent_id), None),
    )
    .await;
    let v: Value = json_body(&raw);
    println!(
        "\n==== GET /todos/{} 默认（嵌套）====\n{}",
        parent_id,
        serde_json::to_string_pretty(&v).unwrap()
    );

    // 7) GET /todos/C2 通过短码反查
    let (status, _, raw) = send(&fx.router, req(Method::GET, "/todos/C2", None)).await;
    let v: Value = json_body(&raw);
    println!(
        "\n==== GET /todos/C2（短码反查）status={} ====\n{}",
        status,
        serde_json::to_string_pretty(&v).unwrap()
    );

    // 8) GET /todos/c2 大小写不敏感
    let (status, _, raw) = send(&fx.router, req(Method::GET, "/todos/c2", None)).await;
    let v: Value = json_body(&raw);
    println!(
        "\n==== GET /todos/c2（小写）status={} ====\n{}",
        status,
        serde_json::to_string_pretty(&v).unwrap()
    );

    // 9) PATCH /todos/C2 完成
    let (status, _, raw) = send(
        &fx.router,
        req(Method::PATCH, "/todos/C2", Some(json!({"completed": true}))),
    )
    .await;
    let v: Value = json_body(&raw);
    println!(
        "\n==== PATCH /todos/C2 完成 status={} ====\n{}",
        status,
        serde_json::to_string_pretty(&v).unwrap()
    );

    // 10) POST /todos/C2/subtasks 用短码引用父
    let (status, _, raw) = send(
        &fx.router,
        req(
            Method::POST,
            "/todos/C2/subtasks",
            Some(json!({"title": "通过 C2 引用父级"})),
        ),
    )
    .await;
    let v: Value = json_body(&raw);
    println!(
        "\n==== POST /todos/C2/subtasks status={} ====\n{}",
        status,
        serde_json::to_string_pretty(&v).unwrap()
    );

    // 11) GET /todos/C999 不存在
    let (status, _, raw) = send(&fx.router, req(Method::GET, "/todos/C999", None)).await;
    let v: Value = json_body(&raw);
    println!(
        "\n==== GET /todos/C999（不存在）status={} ====\n{}",
        status,
        serde_json::to_string_pretty(&v).unwrap()
    );

    // 12) DELETE /todos/C1
    let (status, _, _) = send(&fx.router, req(Method::DELETE, "/todos/C1", None)).await;
    println!("\n==== DELETE /todos/C1 status={} ====", status);

    // 13) 删除后再新建：seq 不复用（应该是 C3）
    let (_, _, raw) = send(
        &fx.router,
        req(
            Method::POST,
            "/todos",
            Some(json!({"title": "删除后新建（验证 seq 不复用）"})),
        ),
    )
    .await;
    let v: Value = json_body(&raw);
    println!(
        "\n==== POST 删除后新建（验证 seq 不复用）====\n{}",
        serde_json::to_string_pretty(&v).unwrap()
    );
}

/// 删掉当前最大短码的 todo 后，新 todo 也不复用那个短码（seq 高水位）。
#[tokio::test]
async fn seq_does_not_recycle_after_deleting_max() {
    let fx = fixture();
    let _ = create_todo(&fx, json!({"title": "a"})).await; // seq=1
    let _ = create_todo(&fx, json!({"title": "b"})).await; // seq=2
    let (status, _, _) = send(&fx.router, req(Method::DELETE, "/todos/C2", None)).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let c = create_todo(&fx, json!({"title": "c"})).await;
    assert_eq!(c["seq"], 3, "删除最大号后也不能复用");
    let (status, _, _) = send(&fx.router, req(Method::GET, "/todos/C2", None)).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "旧短码不能指向新 todo");
}

/// push 积压太久（WebDAV 写不进去）→ 即使 pull 正常也降级：503 + 带出排查字段。
#[tokio::test]
async fn health_degraded_when_push_backlog_is_old() {
    let fx = fixture();
    let now = now_local_string(fx.state.config.timezone);
    let long_ago = (chrono::Utc::now().timestamp() - 600).to_string();
    fx.state.db.with_conn(|conn| {
        repo::set_meta(conn, "last_pull_at", &now).unwrap();
        repo::mark_dirty(conn).unwrap();
        repo::set_meta(conn, "dirty_since", &long_ago).unwrap();
        repo::set_meta(conn, "last_push_error", "WebDAV PUT 返回状态 507").unwrap();
        repo::enqueue_dirty_image(conn, "a.png").unwrap();
    });
    let (status, headers, body) = send(&fx.router, req(Method::GET, "/health", None)).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    let v = json_body(&body);
    assert_eq!(v["status"], "degraded");
    assert_eq!(v["pull"], "healthy");
    assert_eq!(v["push"], "offline");
    assert_eq!(v["sync"], "offline");
    assert_eq!(v["dirty"], true);
    assert!(v["dirtySince"].is_string());
    assert_eq!(v["lastPushError"], "WebDAV PUT 返回状态 507");
    assert_eq!(v["imageQueueLength"], 1);
    assert_eq!(headers.get("x-sync-status").unwrap(), "offline");
    // 其它端点的同步头同样反映 push 积压
    let (_, headers, _) = send(&fx.router, req(Method::GET, "/todos", None)).await;
    assert_eq!(headers.get("x-sync-status").unwrap(), "offline");
}

/// `/sync*` 端点真的走一遍同步：API 写入 → push 到 mock WebDAV；DELETE 写出墓碑；
/// PC 的修改经 `/sync/pull` 合并回来。
#[tokio::test]
async fn sync_endpoints_round_trip_through_mock_webdav() {
    use crate::sync::mock_dav::MockDav;
    let mock = MockDav::start();
    let fx = fixture_with_webdav(Some(&mock.base_url));

    let a = create_todo(&fx, json!({"title": "from AI"})).await;
    let b = create_todo(&fx, json!({"title": "to delete"})).await;
    let (status, _, body) = send(&fx.router, req(Method::POST, "/sync/push", None)).await;
    assert_eq!(status, StatusCode::OK);
    let v = json_body(&body);
    assert_eq!(v["pushed"], true);
    assert_eq!(v["dirtyCleared"], true);
    let remote = mock.doc().expect("sync-data uploaded");
    assert_eq!(remote["todos"].as_array().unwrap().len(), 2);

    let (status, _, _) = send(
        &fx.router,
        req(
            Method::DELETE,
            &format!("/todos/{}", todo_id_path(&b)),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _, body) = send(&fx.router, req(Method::POST, "/sync", None)).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let remote = mock.doc().unwrap();
    let todos = remote["todos"].as_array().unwrap();
    assert_eq!(todos.len(), 1);
    assert_eq!(todos[0]["id"], a["id"]);
    assert!(remote["tombstones"]
        .as_array()
        .unwrap()
        .iter()
        .any(|t| t["entityType"] == "todo" && t["entityId"] == b["id"]));

    // PC 在远端改了标题 → /sync/pull 合并回来
    let mut pc = remote.clone();
    pc["todos"][0]["title"] = json!("edited on PC");
    pc["todos"][0]["updatedAt"] = json!("2999-01-01 00:00:00");
    mock.put_doc(&pc);
    let (status, _, body) = send(&fx.router, req(Method::POST, "/sync/pull", None)).await;
    assert_eq!(status, StatusCode::OK);
    let v = json_body(&body);
    assert_eq!(v["changed"], true);
    assert_eq!(v["todosUpserted"], 1);
    let (_, headers, body) = send(
        &fx.router,
        req(Method::GET, &format!("/todos/{}", todo_id_path(&a)), None),
    )
    .await;
    assert_eq!(json_body(&body)["title"], "edited on PC");
    assert_eq!(headers.get("x-sync-status").unwrap(), "healthy");
}

#[tokio::test]
async fn error_body_shape_is_consistent() {
    let fx = fixture();
    let (_, _, body) = send(&fx.router, req(Method::GET, "/todos/999999", None)).await;
    let v = json_body(&body);
    assert!(v["error"].is_string());
    assert!(v["detail"].is_string());
}

// =============================================================================
// K6：写入校验 / 别名 / 派生字段（E2）
// =============================================================================

fn mark_clean(fx: &Fixture) {
    fx.state.db.with_conn(|c| {
        let g = repo::get_dirty_generation(c).unwrap();
        assert!(repo::clear_dirty_if_unchanged(c, g).unwrap());
    });
}

fn is_dirty(fx: &Fixture) -> bool {
    fx.state.db.with_conn(|c| repo::is_dirty(c).unwrap())
}

#[tokio::test]
async fn unknown_fields_are_rejected_with_field_lists() {
    let fx = fixture();
    let (status, _, body) = send(
        &fx.router,
        req(
            Method::POST,
            "/todos",
            Some(json!({"title": "x", "tags": ["a"], "dueDat": "2026-05-20"})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let v = json_body(&body);
    assert_eq!(v["error"], "bad_request");
    let detail = v["detail"].as_str().unwrap();
    assert!(
        detail.contains("unknown field(s): dueDat, tags"),
        "{}",
        detail
    );
    assert!(detail.contains("allowed:"), "{}", detail);
    assert_eq!(v["unknownFields"], json!(["dueDat", "tags"]));
    let allowed: Vec<&str> = v["allowedFields"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a.as_str().unwrap())
        .collect();
    for f in ["title", "endTime", "color", "dueDate", "priority", "notes"] {
        assert!(allowed.contains(&f), "{} missing from {:?}", f, allowed);
    }
    let (_, _, raw) = send(&fx.router, req(Method::GET, "/todos", None)).await;
    assert_eq!(json_body(&raw), json!([]), "被拒绝的请求不落库");

    let t = create_todo(&fx, json!({"title": "keep"})).await;
    let (status, _, _) = send(
        &fx.router,
        req(
            Method::PATCH,
            &format!("/todos/{}", todo_id_path(&t)),
            Some(json!({"title": "changed", "foo": 1})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(stored_todo(&fx, &todo_id_path(&t))["title"], "keep");
}

#[tokio::test]
async fn type_errors_name_every_bad_field() {
    let fx = fixture();
    let t = create_todo(&fx, json!({"title": "x"})).await;
    let id = todo_id_path(&t);
    mark_clean(&fx);
    let (status, _, body) = send(
        &fx.router,
        req(
            Method::PATCH,
            &format!("/todos/{}", id),
            Some(json!({"quadrant": "soon", "color": null, "completed": "yes", "notifyAt": "tomorrow"})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let v = json_body(&body);
    let invalid = v["invalidFields"].as_object().unwrap();
    let mut keys: Vec<&String> = invalid.keys().collect();
    keys.sort();
    assert_eq!(keys, vec!["color", "completed", "notifyAt", "quadrant"]);
    assert!(invalid["notifyAt"]
        .as_str()
        .unwrap()
        .contains("YYYY-MM-DD HH:MM:SS"));
    assert!(!is_dirty(&fx), "失败的写入不标脏");
    assert_eq!(stored_todo(&fx, &id), {
        let mut s = t.clone();
        let o = s.as_object_mut().unwrap();
        o.remove("priority");
        o.remove("seq");
        s
    });
}

#[tokio::test]
async fn malformed_json_bodies_get_json_errors() {
    let fx = fixture();
    let r = Request::builder()
        .method(Method::POST)
        .uri("/todos")
        .header(header::AUTHORIZATION, bearer())
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from("{not json"))
        .unwrap();
    let (status, headers, body) = send(&fx.router, r).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        headers.get(header::CONTENT_TYPE).unwrap(),
        "application/json"
    );
    assert_eq!(json_body(&body)["error"], "bad_request");

    let r = Request::builder()
        .method(Method::POST)
        .uri("/todos")
        .header(header::AUTHORIZATION, bearer())
        .body(Body::from(r#"{"title":"x"}"#))
        .unwrap();
    let (status, _, body) = send(&fx.router, r).await;
    assert_eq!(status, StatusCode::UNSUPPORTED_MEDIA_TYPE);
    assert_eq!(json_body(&body)["error"], "unsupported_media_type");
}

/// 客户端把读到的对象（含 id / seq / 派生 priority / subtasks 等）改几个字段整包写回：可以。
#[tokio::test]
async fn read_modify_write_round_trip_is_accepted() {
    let fx = fixture();
    let t = create_todo(&fx, json!({"title": "x", "color": "#3B82F6"})).await;
    let id = todo_id_path(&t);
    let (_, _, raw) = send(
        &fx.router,
        req(Method::GET, &format!("/todos/{}", id), None),
    )
    .await;
    let mut v = json_body(&raw);
    assert!(v["priority"].is_null() && v["seq"] == 1 && v["subtasks"] == json!([]));
    v["title"] = json!("renamed");
    let (status, _, raw) = send(
        &fx.router,
        req(Method::PATCH, &format!("/todos/{}", id), Some(v)),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&raw));
    let after = json_body(&raw);
    assert_eq!(after["title"], "renamed");
    assert_eq!(after["id"], t["id"]);
    assert_eq!(after["createdAt"], t["createdAt"]);
    assert_eq!(after["color"], "#3B82F6");
}

/// 读出来只改 priority 再整包写回：对象里原样带着的 color 不能让这次修改被静默忽略。
#[tokio::test]
async fn read_modify_write_of_priority_alias_takes_effect() {
    let fx = fixture();
    let t = create_todo(&fx, json!({"title": "x"})).await;
    let id = todo_id_path(&t);
    let (_, _, raw) = send(
        &fx.router,
        req(Method::GET, &format!("/todos/{}", id), None),
    )
    .await;
    let mut v = json_body(&raw);
    assert_eq!(v["priority"], "low");
    v["priority"] = json!("high");
    v["dueDate"] = json!("2026-06-01");
    let (status, _, raw) = send(
        &fx.router,
        req(Method::PATCH, &format!("/todos/{}", id), Some(v)),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let after = json_body(&raw);
    assert_eq!(after["color"], "#EF4444");
    assert_eq!(after["priority"], "high");
    assert_eq!(after["endTime"], "2026-06-01 23:59:00");
}

/// 补丁不改变任何内容（原样写回）→ 不刷新 updatedAt、不标脏：无意义的回写不能在 LWW 里
/// 压过 PC 还没同步上来的编辑。
#[tokio::test]
async fn no_op_patch_does_not_bump_updated_at_or_mark_dirty() {
    let fx = fixture();
    let t = create_todo(&fx, json!({"title": "x", "endTime": "2026-05-20 10:00:00"})).await;
    let id = todo_id_path(&t);
    mark_clean(&fx);
    let row_before = fx
        .state
        .db
        .with_conn(|c| repo::get_todo(c, &id).unwrap().unwrap());
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    let (status, _, raw) = send(
        &fx.router,
        req(
            Method::PATCH,
            &format!("/todos/{}", id),
            Some(json!({"title": "x", "endTime": "2026-05-20T10:00", "priority": "low"})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json_body(&raw)["updatedAt"], t["updatedAt"]);
    assert!(!is_dirty(&fx));
    let row_after = fx
        .state
        .db
        .with_conn(|c| repo::get_todo(c, &id).unwrap().unwrap());
    assert_eq!(row_after.updated_at, row_before.updated_at);
}

#[tokio::test]
async fn datetimes_are_normalized_and_notify_change_resets_notified() {
    let fx = fixture();
    let t = create_todo(
        &fx,
        json!({
            "title": "x",
            "startTime": "2026-05-20",
            "endTime": "2026-05-20T18:30",
            "notifyAt": "2026-05-20T01:00:00Z",
            "notified": true
        }),
    )
    .await;
    assert_eq!(t["startTime"], "2026-05-20 00:00:00");
    assert_eq!(t["endTime"], "2026-05-20 18:30:00");
    assert_eq!(t["notifyAt"], "2026-05-20 09:00:00", "UTC → 配置时区墙钟");
    assert_eq!(t["notified"], true);
    let (_, _, raw) = send(
        &fx.router,
        req(
            Method::PATCH,
            &format!("/todos/{}", todo_id_path(&t)),
            Some(json!({"notifyAt": "2026-05-21"})),
        ),
    )
    .await;
    let v = json_body(&raw);
    assert_eq!(v["notifyAt"], "2026-05-21 09:00:00");
    assert_eq!(v["notified"], false, "新的提醒时间要重新提醒（与 PC 一致）");
}

#[tokio::test]
async fn subtask_writes_are_validated_and_parent_is_fixed() {
    let fx = fixture();
    let a = create_todo(&fx, json!({"title": "A"})).await;
    let b = create_todo(&fx, json!({"title": "B"})).await;
    let (a_id, b_id) = (todo_id_path(&a), todo_id_path(&b));
    let (status, _, raw) = send(
        &fx.router,
        req(
            Method::POST,
            &format!("/todos/{}/subtasks", a_id),
            Some(json!({"title": "s", "parentId": a["id"]})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let s = json_body(&raw);
    let sid = s["id"].as_i64().unwrap().to_string();

    for (body, field) in [
        (json!({"parentId": b["id"]}), "parentId"),
        (json!({"completed": "yes"}), "completed"),
        (json!({"title": ""}), "title"),
    ] {
        let (status, _, raw) = send(
            &fx.router,
            req(Method::PATCH, &format!("/subtasks/{}", sid), Some(body)),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(
            json_body(&raw)["invalidFields"].get(field).is_some(),
            "{}",
            field
        );
    }
    let (status, _, raw) = send(
        &fx.router,
        req(
            Method::PATCH,
            &format!("/subtasks/{}", sid),
            Some(json!({"done": true})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(json_body(&raw)["unknownFields"], json!(["done"]));

    // 同一个父待办原样写回可以
    let (status, _, raw) = send(
        &fx.router,
        req(
            Method::PATCH,
            &format!("/subtasks/{}", sid),
            Some(json!({"parentId": a["id"], "title": "s2"})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json_body(&raw)["parentId"], a["id"]);

    // 在 A 下面创建却声称属于 B → 400
    let (status, _, _) = send(
        &fx.router,
        req(
            Method::POST,
            &format!("/todos/{}/subtasks", a_id),
            Some(json!({"title": "x", "parentId": b["id"]})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let row = fx
        .state
        .db
        .with_conn(|c| repo::get_subtask(c, &sid).unwrap().unwrap());
    assert_eq!(row.todo_id, a_id);
    let _ = b_id;
}

#[tokio::test]
async fn priority_is_derived_from_color_for_filters_and_sorting() {
    let fx = fixture();
    let _ = create_todo(&fx, json!({"title": "red", "priority": "high"})).await;
    let _ = create_todo(&fx, json!({"title": "blue", "color": "#3B82F6"})).await;
    let _ = create_todo(&fx, json!({"title": "green"})).await; // 默认 #10B981 → low
                                                               // PC 来的记录：小写颜色、没有 priority 字段
    let now = now_local_string(fx.state.config.timezone);
    fx.state.db.with_conn(|c| {
        repo::upsert_todo(
            c,
            "7",
            r##"{"id":7,"title":"pc red","color":"#ef4444"}"##,
            &now,
        )
        .unwrap()
    });

    let (_, _, raw) = send(&fx.router, req(Method::GET, "/todos?priority=HIGH", None)).await;
    let mut titles: Vec<String> = json_body(&raw)
        .as_array()
        .unwrap()
        .iter()
        .map(|t| {
            assert_eq!(t["priority"], "high");
            t["title"].as_str().unwrap().to_string()
        })
        .collect();
    titles.sort();
    assert_eq!(titles, vec!["pc red", "red"]);

    let (status, _, _) = send(&fx.router, req(Method::GET, "/todos?priority=urgent", None)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let (_, _, raw) = send(&fx.router, req(Method::GET, "/todos?sort=-priority", None)).await;
    let list = json_body(&raw);
    let prios: Vec<Value> = list
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["priority"].clone())
        .collect();
    assert_eq!(
        prios,
        vec![json!("high"), json!("high"), json!("low"), Value::Null]
    );
}

#[tokio::test]
async fn due_date_filters_accept_k1_and_date_only_is_inclusive() {
    let fx = fixture();
    let _ = create_todo(&fx, json!({"title": "a", "endTime": "2026-05-14 10:00"})).await;
    let _ = create_todo(&fx, json!({"title": "b", "endTime": "2026-05-15 09:00"})).await;
    let titles = |raw: &[u8]| -> Vec<String> {
        json_body(raw)
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["title"].as_str().unwrap().to_string())
            .collect()
    };
    let (_, _, raw) = send(
        &fx.router,
        req(Method::GET, "/todos?dueDateBefore=2026-05-14", None),
    )
    .await;
    assert_eq!(titles(&raw), vec!["a"], "仅日期的 dueDateBefore 包含当天");
    let (_, _, raw) = send(
        &fx.router,
        req(Method::GET, "/todos?dueDateAfter=2026-05-15", None),
    )
    .await;
    assert_eq!(titles(&raw), vec!["b"]);
    let (_, _, raw) = send(
        &fx.router,
        req(Method::GET, "/todos?dueDateBefore=2026-05-14T12:00", None),
    )
    .await;
    assert_eq!(titles(&raw), vec!["a"]);
    let (status, _, _) = send(
        &fx.router,
        req(Method::GET, "/todos?dueDateBefore=next-week", None),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _, _) = send(&fx.router, req(Method::GET, "/todos?startDate=today", None)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn list_with_subtasks_groups_each_todos_children() {
    let fx = fixture();
    let mut ids = Vec::new();
    for i in 0..3 {
        let t = create_todo(&fx, json!({"title": format!("t{}", i), "sortOrder": i})).await;
        ids.push(todo_id_path(&t));
    }
    for (todo, title, order) in [(0, "a2", 2), (0, "a1", 1), (2, "c1", 0)] {
        let (status, _, _) = send(
            &fx.router,
            req(
                Method::POST,
                &format!("/todos/{}/subtasks", ids[todo]),
                Some(json!({"title": title, "sortOrder": order})),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
    }
    let (_, _, raw) = send(
        &fx.router,
        req(Method::GET, "/todos?withSubtasks=true&sort=sortOrder", None),
    )
    .await;
    let v = json_body(&raw);
    let names = |t: &Value| -> Vec<String> {
        t["subtasks"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["title"].as_str().unwrap().to_string())
            .collect()
    };
    assert_eq!(names(&v[0]), vec!["a1", "a2"]);
    assert!(names(&v[1]).is_empty());
    assert_eq!(names(&v[2]), vec!["c1"]);
    let (_, _, raw) = send(&fx.router, req(Method::GET, "/todos?sort=sortOrder", None)).await;
    let v = json_body(&raw);
    let counts: Vec<i64> = v
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["subtaskCount"].as_i64().unwrap())
        .collect();
    assert_eq!(counts, vec![2, 0, 1]);
}

/// 旧版 API 写进缓存的记录：任何一次 PATCH 都会顺带按 K6 修好。
#[tokio::test]
async fn legacy_record_is_normalized_on_patch() {
    let fx = fixture();
    let now = now_local_string(fx.state.config.timezone);
    fx.state.db.with_conn(|c| {
        repo::upsert_todo(
            c,
            "42",
            r##"{"id":42,"title":"ai","priority":"high","color":"#10B981","dueDate":"2026-05-20","notes":"n","quadrant":"urgent_important"}"##,
            &now,
        )
        .unwrap()
    });
    let (status, _, raw) = send(
        &fx.router,
        req(Method::PATCH, "/todos/42", Some(json!({"completed": true}))),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let v = json_body(&raw);
    assert_eq!(v["color"], "#EF4444");
    assert_eq!(v["priority"], "high");
    assert_eq!(v["endTime"], "2026-05-20 23:59:00");
    assert_eq!(v["description"], "n");
    assert_eq!(v["quadrant"], 1);
    let stored = stored_todo(&fx, "42");
    for gone in ["priority", "dueDate", "notes"] {
        assert!(stored.get(gone).is_none(), "{} still stored", gone);
    }
}

// =============================================================================
// 错误不泄露内部细节（E2）
// =============================================================================

#[tokio::test]
async fn storage_errors_return_generic_message() {
    let fx = fixture();
    fx.state
        .db
        .with_conn(|c| c.execute_batch("DROP TABLE todos"))
        .unwrap();
    let (status, _, body) = send(&fx.router, req(Method::GET, "/todos", None)).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    let v = json_body(&body);
    assert_eq!(v["error"], "internal");
    let text = String::from_utf8_lossy(&body);
    assert!(!text.contains("no such table"), "{}", text);
    assert!(!text.contains("sqlite"), "{}", text);
}

#[tokio::test]
async fn image_errors_do_not_leak_server_paths() {
    let fx = fixture();
    let images_dir = fx.state.config.images_dir.clone();
    // 一个同名目录：读取会失败（不是 NotFound）
    std::fs::create_dir_all(images_dir.join("broken.png")).unwrap();
    let (status, _, body) = send(&fx.router, req(Method::GET, "/images/broken.png", None)).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    let text = String::from_utf8_lossy(&body);
    assert!(
        !text.contains(&images_dir.display().to_string()),
        "{}",
        text
    );
    // K5 文件名规则
    for bad in [".hidden.png", "a%20b.png", "-x.png"] {
        let (status, _, _) = send(
            &fx.router,
            req(Method::GET, &format!("/images/{}", bad), None),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{}", bad);
    }
}

/// `/sync*` 与 `/health` 里的同步错误：WebDAV 层面的照常给出，本地存储错误只说"本地存储错误"。
#[tokio::test]
async fn sync_errors_are_sanitized() {
    use crate::sync::mock_dav::MockDav;
    let mock = MockDav::start();
    let fx = fixture_with_webdav(Some(&mock.base_url));
    mock.put_doc(&json!({"version": "4.0", "todos": [], "tombstones": []}));
    fx.state
        .db
        .with_conn(|c| c.execute_batch("DROP TABLE tombstones"))
        .unwrap();
    let (status, _, body) = send(&fx.router, req(Method::POST, "/sync/pull", None)).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    let v = json_body(&body);
    assert_eq!(v["error"], "sync_failed");
    let detail = v["detail"].as_str().unwrap();
    assert!(
        detail.contains(crate::util::LOCAL_STORAGE_ERROR),
        "{}",
        detail
    );
    assert!(!detail.contains("no such table"), "{}", detail);

    let (_, _, body) = send(&fx.router, req(Method::GET, "/health", None)).await;
    let h = json_body(&body);
    let last = h["lastPullError"].as_str().unwrap();
    assert!(!last.contains("no such table"), "{}", last);
}

// =============================================================================
// 访问日志（E2）
// =============================================================================

#[derive(Clone, Default)]
struct LogBuf(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

impl std::io::Write for LogBuf {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn access_log_records_requests_but_never_credentials() {
    let buf = LogBuf::default();
    let writer = buf.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(move || writer.clone())
        .with_ansi(false)
        .with_max_level(tracing::Level::TRACE)
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);

    let fx = fixture();
    // tracing 的 callsite 兴趣缓存是全局的：并行的其它测试线程可能在本订阅者装上之前
    // 注册了同一批 callsite 并缓存为 never。先打一个请求确保注册完成，再强制重建缓存。
    let _ = send(&fx.router, req(Method::GET, "/health", None)).await;
    tracing::callsite::rebuild_interest_cache();
    buf.0.lock().unwrap().clear();

    let _ = send(&fx.router, req(Method::GET, "/todos?q=secret-search", None)).await;
    let _ = send(&fx.router, req_no_auth(Method::GET, "/health")).await;
    let log = String::from_utf8(buf.0.lock().unwrap().clone()).unwrap();
    assert!(log.contains("path=/todos"), "{}", log);
    assert!(log.contains("status=200"), "{}", log);
    assert!(log.contains("status=401"), "未鉴权的请求也要记录: {}", log);
    assert!(!log.contains(API_KEY), "{}", log);
    assert!(
        !log.to_ascii_lowercase().contains("authorization"),
        "{}",
        log
    );
    assert!(
        !log.contains("secret-search"),
        "query 不进访问日志: {}",
        log
    );
}

// =============================================================================
// 跨端契约：云端写出的记录必须能被 PC 的强类型模型反序列化（K6）
// =============================================================================

/// 与 `pc/src-tauri/src/db/models.rs::Todo` 相同的 serde 约束（字段、类型、默认值）。
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
struct PcTodo {
    id: i64,
    title: String,
    description: Option<String>,
    #[serde(default = "pc_default_color")]
    color: String,
    #[serde(default = "pc_default_quadrant")]
    quadrant: i32,
    notify_at: Option<String>,
    #[serde(default)]
    notify_before: i32,
    #[serde(default)]
    notified: bool,
    #[serde(default)]
    completed: bool,
    #[serde(default)]
    sort_order: i32,
    start_time: Option<String>,
    end_time: Option<String>,
    #[serde(default)]
    created_at: String,
    #[serde(default)]
    updated_at: String,
    #[serde(default)]
    repeat_enabled: bool,
    #[serde(default)]
    repeat_type: Option<String>,
    #[serde(default = "pc_default_repeat_interval")]
    repeat_interval: i32,
    #[serde(default)]
    repeat_weekdays: Option<String>,
    #[serde(default)]
    repeat_month_day: Option<i32>,
    #[serde(default)]
    subtasks: Vec<PcSubTask>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
struct PcSubTask {
    id: i64,
    #[serde(default)]
    parent_id: i64,
    title: String,
    content: Option<String>,
    #[serde(default)]
    completed: bool,
    #[serde(default)]
    sort_order: i32,
    #[serde(default)]
    created_at: String,
    #[serde(default)]
    updated_at: String,
}

fn pc_default_color() -> String {
    "#F59E0B".into()
}
fn pc_default_quadrant() -> i32 {
    4
}
fn pc_default_repeat_interval() -> i32 {
    1
}

fn assert_canonical_time(v: &Option<String>, what: &str) {
    if let Some(s) = v {
        assert_eq!(s.len(), 19, "{} = {:?}", what, s);
        assert_eq!(&s[10..11], " ", "{} = {:?}", what, s);
    }
}

#[tokio::test]
async fn pushed_records_deserialize_into_pc_model() {
    use crate::sync::mock_dav::MockDav;
    let mock = MockDav::start();
    let fx = fixture_with_webdav(Some(&mock.base_url));

    // 远端已有 PC 的数据；先 pull 一次
    mock.put_doc(&json!({
        "version": "4.0", "deviceId": "pc", "updatedAt": "2026-10-02T10:00:00+08:00",
        "todos": [{"id": 1, "title": "from pc", "color": "#3B82F6", "quadrant": 2,
                   "notifyAt": "2026-05-20T09:00", "createdAt": "2026-05-01 08:00:00",
                   "updatedAt": "2026-05-01 08:00:00", "subtasks": []}],
        "settings": {"isFixed": false, "windowPosition": null, "windowSize": null},
        "images": [], "tombstones": []
    }));
    let (status, _, _) = send(&fx.router, req(Method::POST, "/sync/pull", None)).await;
    assert_eq!(status, StatusCode::OK);

    // 旧版 API 写进缓存的问题记录（在第一次成功 pull 之后才会被归一化，这里再拉一次触发）
    let now = now_local_string(fx.state.config.timezone);
    fx.state.db.with_conn(|c| {
        repo::upsert_todo(
            c,
            "77",
            r##"{"id":77,"title":"legacy","priority":"medium","color":null,"quadrant":"urgent_not_important","dueDate":"2026-06-01","completed":0,"sortOrder":"3"}"##,
            &now,
        )
        .unwrap();
        repo::upsert_subtask(c, "770", "77", r#"{"id":770,"parentId":5,"title":7,"completed":"true"}"#, &now)
            .unwrap();
        repo::delete_meta(c, repo::meta_keys::CACHE_NORMALIZED).unwrap();
    });

    // AI 通过 API 写入各种形态
    let t = create_todo(
        &fx,
        json!({"title": "ai", "priority": "low", "dueDate": "2026-05-30", "notes": "n",
               "repeatEnabled": true, "repeatType": "WEEKLY", "repeatWeekdays": [5, 1],
               "notifyAt": "2026-05-25T07:30", "quadrant": "important_not_urgent"}),
    )
    .await;
    let (status, _, _) = send(
        &fx.router,
        req(
            Method::POST,
            &format!("/todos/{}/subtasks", todo_id_path(&t)),
            Some(json!({"title": "step", "content": "**md**"})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, _, body) = send(&fx.router, req(Method::POST, "/sync", None)).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));

    let remote = mock.doc().expect("uploaded");
    let todos = remote["todos"].as_array().unwrap();
    assert_eq!(todos.len(), 3);
    for raw in todos {
        let t: PcTodo = serde_json::from_value(raw.clone())
            .unwrap_or_else(|e| panic!("PC 无法反序列化 {}: {}", raw, e));
        for (v, what) in [
            (&t.notify_at, "notifyAt"),
            (&t.start_time, "startTime"),
            (&t.end_time, "endTime"),
        ] {
            assert_canonical_time(v, what);
        }
        assert_canonical_time(&Some(t.updated_at.clone()), "updatedAt");
        for s in &t.subtasks {
            assert_eq!(s.parent_id, t.id);
            assert_canonical_time(&Some(s.updated_at.clone()), "subtask.updatedAt");
        }
        for derived in ["priority", "seq", "subtaskCount", "dueDate", "notes"] {
            assert!(
                raw.get(derived).is_none(),
                "{} leaked into sync-data: {}",
                derived,
                raw
            );
        }
    }
    let legacy = todos.iter().find(|t| t["id"] == 77).unwrap();
    assert_eq!(legacy["color"], "#F59E0B", "medium");
    assert_eq!(legacy["quadrant"], 3);
    assert_eq!(legacy["endTime"], "2026-06-01 23:59:00");
    assert_eq!(legacy["subtasks"][0]["title"], "7");
    let ai = todos.iter().find(|x| x["id"] == t["id"]).unwrap();
    assert_eq!(ai["color"], "#10B981");
    assert_eq!(ai["repeatType"], "weekly");
    assert_eq!(ai["repeatWeekdays"], "1,5");
    assert_eq!(ai["quadrant"], 2);
    let pc = todos.iter().find(|x| x["id"] == 1).unwrap();
    assert_eq!(
        pc["notifyAt"], "2026-05-20 09:00:00",
        "合并进来的时间统一成规范格式"
    );
    assert_eq!(
        pc["updatedAt"], "2026-05-01 08:00:00",
        "仅格式修正不刷新 updatedAt"
    );
}
