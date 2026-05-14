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
    assert!(v["id"].is_number(), "id should be a number (i64 stringified -> parsed back)");
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

    let (status, _, raw) =
        send(&fx.router, req(Method::GET, "/todos?withSubtasks=true", None)).await;
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
    let (status, _, _) =
        send(&fx.router, req(Method::GET, "/todos?quadrant=foo", None)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _, _) =
        send(&fx.router, req(Method::GET, "/todos?quadrant=5", None)).await;
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
        let _ = create_todo(
            &fx,
            json!({"title": format!("t{}", i), "sortOrder": i}),
        )
        .await;
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
    assert!(v.get("subtasks").is_none(), "subtasks should be absent when false");
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
    let t = create_todo(&fx, json!({"title": "old", "priority": "low", "color": "#000000"}))
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
        req(
            Method::PATCH,
            "/todos/999999",
            Some(json!({"title": "x"})),
        ),
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
        req(Method::PATCH, &format!("/todos/{}", id), Some(json!("oops"))),
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
    let stones = fx
        .state
        .db
        .with_conn(|c| repo::list_tombstones(c).unwrap());
    let kinds: Vec<(String, String)> =
        stones.iter().map(|(t, i, _)| (t.clone(), i.clone())).collect();
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

    let stones = fx
        .state
        .db
        .with_conn(|c| repo::list_tombstones(c).unwrap());
    assert!(stones
        .iter()
        .any(|(t, i, _)| t == "subtask" && i == &sid));
}

#[tokio::test]
async fn delete_subtask_not_found_returns_404() {
    let fx = fixture();
    let (status, _, _) =
        send(&fx.router, req(Method::DELETE, "/subtasks/999999", None)).await;
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
        .with_conn(|c| repo::get_meta(c, "dirty_images"));
    let arr: Vec<String> =
        serde_json::from_str(dirty_images.as_deref().unwrap_or("[]")).unwrap();
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
    let (status, _, body) =
        send(&fx.router, req(Method::GET, "/images/nope.png", None)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let v = json_body(&body);
    assert_eq!(v["error"], "not_found");
}

#[tokio::test]
async fn get_image_rejects_path_traversal() {
    let fx = fixture();
    // axum 把 `..` 在 router 层就归一化，所以这里测一个明显非法字符串
    // 走到 handler 的情况：包含 backslash 的 percent-encoded 名字
    let (status, _, _) = send(
        &fx.router,
        req(Method::GET, "/images/%2e%2e", None),
    )
    .await;
    // 解码出来是 ".."，sanitize_filename 拒绝
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

// =============================================================================
// 通用错误体格式
// =============================================================================

#[tokio::test]
async fn error_body_shape_is_consistent() {
    let fx = fixture();
    let (_, _, body) = send(&fx.router, req(Method::GET, "/todos/999999", None)).await;
    let v = json_body(&body);
    assert!(v["error"].is_string());
    assert!(v["detail"].is_string());
}
