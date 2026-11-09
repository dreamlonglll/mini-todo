//! `/lists` CRUD（多清单；契约 `.trellis/tasks/11-09-multi-list-windows/contract.md` §5 / §6 / §10）。
//!
//! - `:id` 接受数字 id 或 URL 编码的清单名：先按数字 id 找，没有这个 id 再按名称（去首尾空白、
//!   不区分大小写）精确匹配；同名的有多个 → 409（同步造成的重名是容忍的），找不到 → 404
//! - 写入只接受 `name` / `sortOrder`（`id` / `createdAt` / `updatedAt` 忽略，其它字段 400）；名称
//!   去首尾空白、不能为空、不超过 50 个字，与其它未删除的清单不区分大小写重名 → 409
//! - 新清单 id 用 `ids::new_id`（与 PC 同一算法，撞主键换 id 重试），`sortOrder` 缺省 = 现有最大值 + 1
//! - 改写已有清单：`updatedAt = max(now, 旧值 + 1 秒)`；内容没变时不刷新、不标脏
//! - DELETE（契约 §5，单事务）：默认清单 → 400；`listId` 等于它的待办改到默认清单并刷新
//!   `updatedAt`，删清单，写清单墓碑 `deletedAt = max(now, 清单的 updatedAt)`
//! - 清单不分配 `C{seq}` 短码；写操作照常标脏，由 push worker 去抖推送

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use chrono_tz::Tz;
use rusqlite::Connection;
use serde_json::{json, Map, Value};
use tracing::warn;

use super::error::{ApiError, ApiJson};
use super::ids::{insert_with_fresh_id, new_id};
use super::AppState;
use crate::db::repo;
use crate::model::{self, WriteMode, DEFAULT_LIST_ID};
use crate::time::{bump_updated_at, deletion_time, now_local_string};

/// 一个未删除的清单（缓存里的对象原样 + 主键）。
#[derive(Debug, Clone)]
pub(crate) struct ListEntry {
    pub id: i64,
    pub obj: Map<String, Value>,
    /// `updated_at` 列（规范格式）。
    pub updated_at: String,
}

impl ListEntry {
    fn key(&self) -> String {
        self.id.to_string()
    }

    fn name(&self) -> Option<&str> {
        self.obj.get("name").and_then(Value::as_str)
    }

    fn sort_order(&self) -> i64 {
        self.obj
            .get("sortOrder")
            .and_then(Value::as_i64)
            .unwrap_or(0)
    }
}

/// 全部未删除的清单，按 `(sortOrder, id)` 升序（契约 §8）。坏行跳过；本地缺默认清单（不该
/// 发生：建库与每次合并都会补建）时按初始值补上。
pub(crate) fn load_lists(conn: &Connection) -> rusqlite::Result<Vec<ListEntry>> {
    let mut lists = Vec::new();
    for row in repo::all_list_rows(conn)? {
        match (
            row.id.parse::<i64>(),
            serde_json::from_str::<Value>(&row.data_json),
        ) {
            (Ok(id), Ok(Value::Object(obj))) => lists.push(ListEntry {
                id,
                obj,
                updated_at: row.updated_at,
            }),
            _ => warn!(target: "minitodo_cloud::api", "skip corrupt list row {}", row.id),
        }
    }
    if !lists.iter().any(|l| l.id == DEFAULT_LIST_ID) {
        lists.push(ListEntry {
            id: DEFAULT_LIST_ID,
            obj: model::default_list_record(),
            updated_at: model::DEFAULT_LIST_TIME.to_string(),
        });
    }
    lists.sort_by_key(|l| (l.sort_order(), l.id));
    Ok(lists)
}

/// 按名称找清单（契约 §6 的比较规则）：去首尾空白、不区分大小写精确匹配。
/// 没有 → 404；有多个（同步造成的重名）→ 409，并列出候选让调用方改用 id。
pub(crate) fn find_list_by_name<'a>(
    lists: &'a [ListEntry],
    name: &str,
) -> Result<&'a ListEntry, ApiError> {
    let key = model::list_name_key(name);
    let matches: Vec<&ListEntry> = lists
        .iter()
        .filter(|l| l.name().map(model::list_name_key).as_deref() == Some(key.as_str()))
        .collect();
    match matches.as_slice() {
        [] => Err(ApiError::not_found(format!(
            "{}：{}",
            model::LIST_NOT_FOUND,
            name.trim()
        ))),
        [one] => Ok(one),
        many => {
            let mut err = ApiError::conflict(format!(
                "有 {} 个清单都叫「{}」，请改用清单 id",
                many.len(),
                name.trim()
            ));
            let candidates: Vec<Value> = many
                .iter()
                .map(|l| json!({"id": l.id, "name": l.name()}))
                .collect();
            err.extra
                .insert("candidates".into(), Value::Array(candidates));
            Err(err)
        }
    }
}

/// 解析 path 里的 `:id`：数字 id 优先（存在才算），否则按名称找（见 `find_list_by_name`）。
pub(crate) fn resolve_list_ref(conn: &Connection, raw: &str) -> Result<ListEntry, ApiError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(ApiError::not_found(model::LIST_NOT_FOUND));
    }
    let lists = load_lists(conn)?;
    if let Ok(id) = trimmed.parse::<i64>() {
        if let Some(found) = lists.iter().find(|l| l.id == id) {
            return Ok(found.clone());
        }
    }
    find_list_by_name(&lists, trimmed).cloned()
}

/// 与其它未删除的清单（`except` 除外）不区分大小写重名 → 409（契约 §6）。错误文案与 PC 一致：
/// `已有同名清单「<已有清单的名称>」`。
fn ensure_unique_name(
    lists: &[ListEntry],
    name: &str,
    except: Option<i64>,
) -> Result<(), ApiError> {
    let key = model::list_name_key(name);
    let taken = lists.iter().find_map(|l| {
        l.name()
            .filter(|existing| Some(l.id) != except && model::list_name_key(existing) == key)
    });
    match taken {
        Some(existing) => Err(ApiError::conflict(format!(
            "已有同名清单「{}」",
            existing.trim()
        ))),
        None => Ok(()),
    }
}

/// 新清单排最后：现有最大 `sortOrder` + 1（契约 §8；PC 端是 i64）。
fn next_sort_order(lists: &[ListEntry]) -> i64 {
    lists
        .iter()
        .map(ListEntry::sort_order)
        .max()
        .unwrap_or(0)
        .saturating_add(1)
}

/// `POST /lists` 的落库动作（调用方负责事务）：校验重名、分配 id、插入、标脏。
/// `patch` 必须已经通过 `model::validate_list_input(.., Create)`。
pub(crate) fn create_list_tx(
    conn: &Connection,
    patch: &Map<String, Value>,
    now: &str,
) -> Result<Map<String, Value>, ApiError> {
    let name = patch
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| ApiError::bad_request("name: is required"))?;
    let lists = load_lists(conn)?;
    ensure_unique_name(&lists, name, None)?;
    let sort_order = patch
        .get("sortOrder")
        .and_then(Value::as_i64)
        .unwrap_or_else(|| next_sort_order(&lists));
    let mut record = Map::new();
    insert_with_fresh_id(new_id, |id| {
        record = model::new_list_record(id, name, sort_order, now);
        repo::insert_list(
            conn,
            &id.to_string(),
            &Value::Object(record.clone()).to_string(),
            now,
        )
    })?;
    repo::mark_dirty(conn)?;
    Ok(record)
}

/// `PATCH /lists/:id` 的落库动作（调用方负责事务）：改名查重名（不含自己）；内容有变化才刷新
/// `updatedAt` 并标脏。`patch` 必须已经通过 `model::validate_list_input(.., Update)`。
pub(crate) fn update_list_tx(
    conn: &Connection,
    entry: &ListEntry,
    patch: &Map<String, Value>,
    now: &str,
    tz: Tz,
) -> Result<Map<String, Value>, ApiError> {
    if let Some(name) = patch.get("name").and_then(Value::as_str) {
        ensure_unique_name(&load_lists(conn)?, name, Some(entry.id))?;
    }
    let base = &entry.obj;
    let mut next = base.clone();
    crate::util::merge_json_shallow(&mut next, patch);
    if next != *base {
        // 新版本必须严格晚于旧版本（同一秒内两次改名 / 清单来自时钟偏快的一端）
        let updated_at = bump_updated_at(
            now,
            &[
                &entry.updated_at,
                model::json_updated_at(base).unwrap_or(""),
            ],
            tz,
        );
        next.insert("updatedAt".into(), json!(updated_at));
        repo::upsert_list(
            conn,
            &entry.key(),
            &Value::Object(next.clone()).to_string(),
            &updated_at,
        )?;
        repo::mark_dirty(conn)?;
    }
    Ok(next)
}

/// 删除清单的结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DeletedList {
    pub id: i64,
    /// 移到默认清单的待办数。
    pub moved_todos: usize,
}

/// `DELETE /lists/:id` 的落库动作（契约 §5；调用方负责事务）：默认清单 → 400；原始
/// `listId` 等于它的待办改到默认清单并刷新 `updatedAt`；删清单；写清单墓碑；标脏。
pub(crate) fn delete_list_tx(
    conn: &Connection,
    entry: &ListEntry,
    now: &str,
    tz: Tz,
) -> Result<DeletedList, ApiError> {
    if entry.id == DEFAULT_LIST_ID {
        return Err(ApiError::bad_request("默认清单不能删除"));
    }
    let mut moved_todos = 0;
    for row in repo::todos_in_list(conn, entry.id)? {
        let Ok(Value::Object(mut obj)) = serde_json::from_str::<Value>(&row.data_json) else {
            continue;
        };
        let updated_at = bump_updated_at(
            now,
            &[&row.updated_at, model::json_updated_at(&obj).unwrap_or("")],
            tz,
        );
        obj.insert("listId".into(), json!(DEFAULT_LIST_ID));
        obj.insert("updatedAt".into(), json!(updated_at));
        repo::upsert_todo(conn, &row.id, &Value::Object(obj).to_string(), &updated_at)?;
        moved_todos += 1;
    }
    repo::delete_list(conn, &entry.key())?;
    // 墓碑时间 = max(now, 清单的 updatedAt)：清单比本端"现在"还新时，只写 now 压不住它
    let deleted_at = deletion_time(
        now,
        &[
            &entry.updated_at,
            model::json_updated_at(&entry.obj).unwrap_or(""),
        ],
        tz,
    );
    repo::add_list_tombstone(conn, &entry.key(), &deleted_at)?;
    repo::mark_dirty(conn)?;
    Ok(DeletedList {
        id: entry.id,
        moved_todos,
    })
}

// =============================================================================
// handlers
// =============================================================================

/// `GET /lists`：全部清单，按 `(sortOrder, id)`。
pub async fn list_lists(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    let lists = state.db.with_conn(|conn| load_lists(conn))?;
    Ok(Json(Value::Array(
        lists.into_iter().map(|l| Value::Object(l.obj)).collect(),
    )))
}

/// `GET /lists/:id`
pub async fn get_list(
    State(state): State<AppState>,
    Path(raw): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let entry = state.db.with_conn(|conn| resolve_list_ref(conn, &raw))?;
    Ok(Json(Value::Object(entry.obj)))
}

/// `POST /lists`：必填 `name`，可选 `sortOrder` → 201 + 新清单。
pub async fn create_list(
    State(state): State<AppState>,
    ApiJson(body): ApiJson<Value>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let patch =
        model::validate_list_input(&body, WriteMode::Create).map_err(ApiError::validation)?;
    let now = now_local_string(state.config.timezone);
    let record = state
        .db
        .with_conn(|conn| -> Result<Map<String, Value>, ApiError> {
            let tx = conn.transaction()?;
            let record = create_list_tx(&tx, &patch, &now)?;
            tx.commit()?;
            Ok(record)
        })?;
    Ok((StatusCode::CREATED, Json(Value::Object(record))))
}

/// `PATCH /lists/:id`：`name` / `sortOrder`，未提及字段保留。
pub async fn patch_list(
    State(state): State<AppState>,
    Path(raw): Path<String>,
    ApiJson(body): ApiJson<Value>,
) -> Result<Json<Value>, ApiError> {
    let tz = state.config.timezone;
    let patch =
        model::validate_list_input(&body, WriteMode::Update).map_err(ApiError::validation)?;
    let now = now_local_string(tz);
    let record = state
        .db
        .with_conn(|conn| -> Result<Map<String, Value>, ApiError> {
            let tx = conn.transaction()?;
            let entry = resolve_list_ref(&tx, &raw)?;
            let record = update_list_tx(&tx, &entry, &patch, &now, tz)?;
            tx.commit()?;
            Ok(record)
        })?;
    Ok(Json(Value::Object(record)))
}

/// `DELETE /lists/:id` → 200 `{deleted, name, movedTodos, movedTo}`（其下待办移到了默认清单）。
pub async fn delete_list(
    State(state): State<AppState>,
    Path(raw): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let tz = state.config.timezone;
    let now = now_local_string(tz);
    let (entry, deleted) =
        state
            .db
            .with_conn(|conn| -> Result<(ListEntry, DeletedList), ApiError> {
                let tx = conn.transaction()?;
                let entry = resolve_list_ref(&tx, &raw)?;
                let deleted = delete_list_tx(&tx, &entry, &now, tz)?;
                tx.commit()?;
                Ok((entry, deleted))
            })?;
    Ok(Json(json!({
        "deleted": deleted.id,
        "name": entry.name(),
        "movedTodos": deleted.moved_todos,
        "movedTo": DEFAULT_LIST_ID,
    })))
}
