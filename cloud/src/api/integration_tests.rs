//! API 集成测试：用 axum Router + tower::ServiceExt::oneshot 直接打 in-process
//! 请求，覆盖鉴权、健康检查、todos / subtasks / images 全部 CRUD 路径，外加
//! 过滤、排序、分页、merge PATCH、cascade DELETE、tombstones、X-Sync-Status header。
//!
//! 这些测试**不**起 tokio 后台 worker（pull/push/images bootstrap），所以
//! 全程不会触碰真实网络；`Db` 用临时目录里的 SQLite 文件、`images_dir` 也用
//! tempdir，测试结束自动清理。

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
    let tmp = TempDir::new().expect("tempdir");
    let data_dir = tmp.path().join("data");
    let images_dir = tmp.path().join("images");
    std::fs::create_dir_all(&data_dir).unwrap();
    std::fs::create_dir_all(&images_dir).unwrap();

    let cfg = Arc::new(Config::for_tests(API_KEY, data_dir.clone(), images_dir));
    let db = Db::open(&data_dir.join("data.db")).expect("open db");

    let state = AppState {
        config: cfg,
        db: db.clone(),
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
async fn auth_missing_token_returns_401_with_sync_header() {
    let fx = fixture();
    let (status, headers, body) = send(&fx.router, req_no_auth(Method::GET, "/health")).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    // 即使是 401 也要有 X-Sync-Status（外层 middleware 注入）
    assert!(
        headers.contains_key("x-sync-status"),
        "401 response must still carry x-sync-status header"
    );
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
    let (status, _, _) = send(&fx.router, req(Method::GET, "/health", None)).await;
    assert_eq!(status, StatusCode::OK);
}

// =============================================================================
// /health
// =============================================================================

#[tokio::test]
async fn health_offline_when_no_pull() {
    let fx = fixture();
    let (status, headers, body) = send(&fx.router, req(Method::GET, "/health", None)).await;
    assert_eq!(status, StatusCode::OK);
    let v = json_body(&body);
    assert_eq!(v["status"], "healthy");
    assert_eq!(v["sync"], "offline");
    assert!(v["lastPullAt"].is_null());
    assert_eq!(headers.get("x-sync-status").unwrap(), "offline");
    // offline 时附 Warning header
    assert!(headers.contains_key("warning"));
}

#[tokio::test]
async fn health_healthy_after_meta_set() {
    let fx = fixture();
    let now = now_local_string(fx.state.config.timezone_offset);
    fx.state
        .db
        .with_conn(|conn| repo::set_meta(conn, "last_pull_at", &now).unwrap());

    let (status, headers, body) = send(&fx.router, req(Method::GET, "/health", None)).await;
    assert_eq!(status, StatusCode::OK);
    let v = json_body(&body);
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

#[tokio::test]
async fn create_todo_passes_through_user_fields() {
    let fx = fixture();
    let body = json!({
        "title": "with extras",
        "priority": "high",
        "quadrant": 1,
        "color": "#EF4444",
        "dueDate": "2026-05-20",
        "notes": "free text"
    });
    let (status, _, raw) = send(&fx.router, req(Method::POST, "/todos", Some(body))).await;
    assert_eq!(status, StatusCode::CREATED);
    let v = json_body(&raw);
    assert_eq!(v["priority"], "high");
    assert_eq!(v["quadrant"], 1);
    assert_eq!(v["color"], "#EF4444");
    assert_eq!(v["dueDate"], "2026-05-20");
    assert_eq!(v["notes"], "free text");
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
    let dirty = fx.state.db.with_conn(|c| repo::get_meta(c, "dirty"));
    assert_eq!(dirty.as_deref(), Some("true"));
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
        json!({"title": "old", "priority": "low", "color": "#000000"}),
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
            Some(json!({"title": "new", "priority": "high"})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let v = json_body(&raw);
    assert_eq!(v["title"], "new");
    assert_eq!(v["priority"], "high");
    // 未提及字段保留
    assert_eq!(v["color"], "#000000");
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
    let t = create_todo(&fx, json!({"title": "x", "notes": "kept"})).await;
    let id = todo_id_path(&t);
    let (status, _, raw) = send(
        &fx.router,
        req(
            Method::PATCH,
            &format!("/todos/{}", id),
            Some(json!({"notes": null})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let v = json_body(&raw);
    assert!(v["notes"].is_null());
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
    let dirty_images = fx.state.db.with_conn(|c| repo::get_meta(c, "dirty_images"));
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
    let now = now_local_string(fx.state.config.timezone_offset);
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

#[tokio::test]
async fn error_body_shape_is_consistent() {
    let fx = fixture();
    let (_, _, body) = send(&fx.router, req(Method::GET, "/todos/999999", None)).await;
    let v = json_body(&body);
    assert!(v["error"].is_string());
    assert!(v["detail"].is_string());
}
