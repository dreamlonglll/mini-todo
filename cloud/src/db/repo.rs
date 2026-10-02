//! 仓储层：list / upsert / patch / delete / meta KV / settings KV / tombstones。
//!
//! Schema 是 KV-style（`todos(id, data_json, updated_at)` /
//! `subtasks(id, todo_id, data_json, updated_at)` / `settings(key, value)` /
//! `meta(key, value)`）。所有过滤 / 排序通过 SQLite JSON1 函数对 `data_json`
//! 做提取。

use std::collections::{HashMap, HashSet};

use rusqlite::{params, Connection, OptionalExtension};

/// `meta` 表里用到的键。集中在一处，避免 worker / API / 健康检查各写各的字符串。
pub mod meta_keys {
    /// `"true"` 表示有本地写入还没推到 WebDAV。
    pub const DIRTY: &str = "dirty";
    /// 每次 `mark_dirty` 递增；push 用它判断推送窗口期内是否又有新写入。
    pub const DIRTY_GENERATION: &str = "dirty_generation";
    /// 第一次变脏的时刻（UNIX 秒）；清 dirty 时一并删除。健康检查算积压时长用。
    pub const DIRTY_SINCE: &str = "dirty_since";
    /// 待上传图片队列（JSON 字符串数组）。
    pub const DIRTY_IMAGES: &str = "dirty_images";
    /// 图片队列第一次被 worker 观察到非空的时刻（UNIX 秒）；队列清空时删除。
    pub const IMAGE_QUEUE_SINCE: &str = "image_queue_since";
    /// `C{seq}` 短码的单调高水位：删掉最大号的 todo 后也不复用。
    pub const SEQ_HWM: &str = "seq_hwm";
    /// 最近一次成功从 WebDAV 拉取并合并（或确认未变）的本地墙钟时间。
    pub const LAST_PULL_AT: &str = "last_pull_at";
    pub const LAST_PULL_ERROR: &str = "last_pull_error";
    /// 最近一次成功把 sync-data PUT 到 WebDAV 的本地墙钟时间。
    pub const LAST_PUSH_OK_AT: &str = "last_push_ok_at";
    pub const LAST_PUSH_ERROR: &str = "last_push_error";
    /// 同步"基准"（跨端契约 K4）：最近一次**已完整合并或成功写入**的远端版本的校验器。
    pub const BASE_ETAG: &str = "base_etag";
    pub const BASE_LAST_MODIFIED: &str = "base_last_modified";
    /// 基准版本文档里除 `todos` / `tombstones` 以外的全部顶层键（JSON 对象），
    /// 远端 304 时用它重建要上传的文档，保证 `settings` / 未知顶层键原样保留。
    pub const REMOTE_ENVELOPE: &str = "remote_envelope";
}

use meta_keys as mk;

/// 单条 todo 在 SQLite 中的快照：`data_json` 是 PC 端 todo 对象的 JSON 原样存储。
#[derive(Debug, Clone)]
pub struct TodoRow {
    #[allow(dead_code)]
    pub id: String,
    pub data_json: String,
    #[allow(dead_code)]
    pub updated_at: String,
}

/// 单条 subtask 在 SQLite 中的快照。同 `TodoRow`。
#[derive(Debug, Clone)]
pub struct SubtaskRow {
    #[allow(dead_code)]
    pub id: String,
    #[allow(dead_code)]
    pub todo_id: String,
    pub data_json: String,
    #[allow(dead_code)]
    pub updated_at: String,
}

// =============================================================================
// meta KV
// =============================================================================

/// 读 meta 值。`Ok(None)` 只表示"键不存在"，DB 错误按 `Err` 上抛——
/// 早期版本用 `.ok().flatten()` 把两者混成 `None`，导致 pull 读 dirty 失败时
/// 被当成"不脏"而执行孤儿清理，可能误删还没 push 的本地记录。
pub fn get_meta(conn: &Connection, key: &str) -> rusqlite::Result<Option<String>> {
    conn.query_row("SELECT value FROM meta WHERE key = ?1", [key], |row| {
        row.get::<_, String>(0)
    })
    .optional()
}

pub fn set_meta(conn: &Connection, key: &str, value: &str) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO meta (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
}

pub fn delete_meta(conn: &Connection, key: &str) -> rusqlite::Result<()> {
    conn.execute("DELETE FROM meta WHERE key = ?1", [key])?;
    Ok(())
}

/// 键不存在时才写入（`dirty_since` 这类"第一次发生时刻"用）。
pub fn set_meta_if_absent(conn: &Connection, key: &str, value: &str) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO meta (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO NOTHING",
        params![key, value],
    )?;
    Ok(())
}

/// 当前是否 dirty。
pub fn is_dirty(conn: &Connection) -> rusqlite::Result<bool> {
    Ok(get_meta(conn, mk::DIRTY)?.as_deref() == Some("true"))
}

/// 标脏：置 `dirty=true` 并把 `dirty_generation` 计数 +1。
///
/// 所有写路径（todos / subtasks / images 的增删改）都必须走这里。generation
/// 是 push worker 的并发判据：push 开始时记下 `g0`，PUT 成功后仅当计数仍为
/// `g0` 才清 dirty；否则说明推送窗口期内又有新写入，dirty 保留给下一轮。
/// 第一次变脏时顺带记录 `dirty_since`（UNIX 秒，SQLite 自己取时间，调用方不必
/// 关心时区），供健康检查计算积压时长。
pub fn mark_dirty(conn: &Connection) -> rusqlite::Result<()> {
    set_meta(conn, mk::DIRTY, "true")?;
    let next = get_dirty_generation(conn)? + 1;
    set_meta(conn, mk::DIRTY_GENERATION, &next.to_string())?;
    conn.execute(
        "INSERT INTO meta (key, value) VALUES (?1, CAST(strftime('%s', 'now') AS TEXT))
         ON CONFLICT(key) DO NOTHING",
        [mk::DIRTY_SINCE],
    )?;
    Ok(())
}

/// 读 `dirty_generation` 计数；键不存在或值非法均视为 0。
pub fn get_dirty_generation(conn: &Connection) -> rusqlite::Result<i64> {
    Ok(get_meta(conn, mk::DIRTY_GENERATION)?
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(0))
}

/// PUT 成功后的收尾：generation 仍等于 `g0` 才置 dirty=false（并删 `dirty_since`），
/// 返回是否清除。
///
/// generation 变了说明推送窗口期内又有新写入（这些改动不在刚 PUT 的快照里），
/// dirty 必须保留给下一轮。读 + 判 + 写在同一个连接调用序列里完成，调用方持有
/// `Db` 的 Mutex，与写路径的 `mark_dirty` 互斥。
pub fn clear_dirty_if_unchanged(conn: &Connection, g0: i64) -> rusqlite::Result<bool> {
    if get_dirty_generation(conn)? != g0 {
        return Ok(false);
    }
    set_meta(conn, mk::DIRTY, "false")?;
    delete_meta(conn, mk::DIRTY_SINCE)?;
    Ok(true)
}

// =============================================================================
// 待上传图片队列（meta.dirty_images，JSON 字符串数组）
//
// 读改写都在调用方持有的同一个连接调用序列里完成（`Db::with_conn` 的 Mutex 保证
// 与其它写入互斥）。push worker 上传期间**不**持锁，所以上传结束后必须重新读取
// 队列、只移除这次真正处理掉的名字——不能拿上传前读到的快照整体覆盖，否则上传
// 期间新入队的图片会丢（审查 A8）。
// =============================================================================

/// 读出队列；值损坏时视为空队列（不让一条坏数据卡死整个 push）。
pub fn dirty_image_queue(conn: &Connection) -> rusqlite::Result<Vec<String>> {
    Ok(get_meta(conn, mk::DIRTY_IMAGES)?
        .and_then(|raw| serde_json::from_str::<Vec<String>>(&raw).ok())
        .unwrap_or_default())
}

/// 入队（已在队列中则忽略）。
pub fn enqueue_dirty_image(conn: &Connection, name: &str) -> rusqlite::Result<()> {
    let mut queue = dirty_image_queue(conn)?;
    if !queue.iter().any(|n| n == name) {
        queue.push(name.to_string());
    }
    write_image_queue(conn, &queue)
}

/// 从**当前**队列中移除 `done` 里的名字，返回剩余队列。
pub fn remove_dirty_images(
    conn: &Connection,
    done: &HashSet<String>,
) -> rusqlite::Result<Vec<String>> {
    let remaining: Vec<String> = dirty_image_queue(conn)?
        .into_iter()
        .filter(|n| !done.contains(n))
        .collect();
    write_image_queue(conn, &remaining)?;
    Ok(remaining)
}

fn write_image_queue(conn: &Connection, queue: &[String]) -> rusqlite::Result<()> {
    let raw = serde_json::to_string(queue).unwrap_or_else(|_| "[]".to_string());
    set_meta(conn, mk::DIRTY_IMAGES, &raw)
}

/// 给 `todo_id` 分配 / 取得 cloud-only 短码 `seq`。
///
/// - 已分配 → 返回现有 seq（幂等，多次调用不重复分配）。
/// - 未分配 → 取 `max(高水位, MAX(seq)) + 1`（首条为 1），写入 `todo_seq` 表并推高水位。
///
/// 高水位存在 `meta.seq_hwm`：只增不减，所以删掉当前最大号的 todo 之后新 todo
/// 也不会复用那个号（早期 `MAX(seq)+1` 会复用，AI 拿旧短码操作就会打到新 todo 上）。
///
/// 详见 `schema.rs` 中 `todo_seq` 注释：seq 不进 data_json，cloud 独立维护。
pub fn assign_seq(conn: &Connection, todo_id: &str) -> rusqlite::Result<i64> {
    if let Some(seq) = get_seq(conn, todo_id)? {
        return Ok(seq);
    }
    let max_in_table: i64 =
        conn.query_row("SELECT COALESCE(MAX(seq), 0) FROM todo_seq", [], |row| {
            row.get(0)
        })?;
    let hwm = get_meta(conn, mk::SEQ_HWM)?
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(0);
    let next = max_in_table.max(hwm) + 1;
    conn.execute(
        "INSERT INTO todo_seq (todo_id, seq) VALUES (?1, ?2)",
        params![todo_id, next],
    )?;
    set_meta(conn, mk::SEQ_HWM, &next.to_string())?;
    Ok(next)
}

/// 查 `todo_id` 对应的 seq；未分配返回 None。
pub fn get_seq(conn: &Connection, todo_id: &str) -> rusqlite::Result<Option<i64>> {
    conn.query_row(
        "SELECT seq FROM todo_seq WHERE todo_id = ?1",
        [todo_id],
        |row| row.get::<_, i64>(0),
    )
    .optional()
}

/// 反查：按 seq 找 todo_id；找不到返回 None。
pub fn get_todo_id_by_seq(conn: &Connection, seq: i64) -> rusqlite::Result<Option<String>> {
    conn.query_row(
        "SELECT todo_id FROM todo_seq WHERE seq = ?1",
        [seq],
        |row| row.get::<_, String>(0),
    )
    .optional()
}

/// 删除 todo 时连带清理 `todo_seq`。seq 不复用——高水位见 `assign_seq`。
pub fn delete_seq(conn: &Connection, todo_id: &str) -> rusqlite::Result<()> {
    conn.execute("DELETE FROM todo_seq WHERE todo_id = ?1", [todo_id])?;
    Ok(())
}

/// 列出所有"在 todos 表里但 todo_seq 没分配"的 todo_id，供 pull 后回填用。
pub fn todo_ids_without_seq(conn: &Connection) -> rusqlite::Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT t.id FROM todos t
         LEFT JOIN todo_seq s ON s.todo_id = t.id
         WHERE s.seq IS NULL",
    )?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
    rows.collect()
}

/// 批量取 (todo_id → seq) 映射；list_todos 拼响应时一次性 join。
pub fn seq_map_for_todos(
    conn: &Connection,
    ids: &[String],
) -> rusqlite::Result<std::collections::HashMap<String, i64>> {
    let mut map = std::collections::HashMap::new();
    if ids.is_empty() {
        return Ok(map);
    }
    // SQLite 没有数组绑定，用 IN (?,?,...) 拼。
    let placeholders = std::iter::repeat_n("?", ids.len())
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!(
        "SELECT todo_id, seq FROM todo_seq WHERE todo_id IN ({})",
        placeholders
    );
    let mut stmt = conn.prepare(&sql)?;
    let id_refs: Vec<&dyn rusqlite::ToSql> =
        ids.iter().map(|s| s as &dyn rusqlite::ToSql).collect();
    let rows = stmt.query_map(rusqlite::params_from_iter(id_refs), |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
    })?;
    for r in rows {
        let (id, seq) = r?;
        map.insert(id, seq);
    }
    Ok(map)
}

// =============================================================================
// settings KV（与 SyncData.settings JSON 字段对应）
// =============================================================================

pub fn set_setting(conn: &Connection, key: &str, value: &str) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
}

// =============================================================================
// todos / subtasks
// =============================================================================

pub fn get_todo(conn: &Connection, id: &str) -> rusqlite::Result<Option<TodoRow>> {
    conn.query_row(
        "SELECT id, data_json, updated_at FROM todos WHERE id = ?1",
        [id],
        |row| {
            Ok(TodoRow {
                id: row.get(0)?,
                data_json: row.get(1)?,
                updated_at: row.get(2)?,
            })
        },
    )
    .optional()
}

/// 列表查询的过滤参数。所有字段为 `Option`；`None` 表示不过滤。
#[derive(Debug, Default, Clone)]
pub struct ListTodosFilter {
    pub completed: Option<bool>,
    pub priority: Option<String>,
    pub quadrant: Option<i64>,
    pub due_date_before: Option<String>,
    pub due_date_after: Option<String>,
    pub start_date: Option<String>,
    pub q: Option<String>,
    /// `(field, asc)`。例如 `("dueDate", true)` 对应 `+dueDate`，`("priority", false)` 对应 `-priority`。
    pub sort: Option<(String, bool)>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

/// 列表查询。返回原始 `TodoRow`，filter / sort / pagination 都已在 SQL 里完成。
///
/// 排序字段白名单：`dueDate` / `startTime` / `priority` / `quadrant` / `sortOrder`
/// / `updatedAt` / `createdAt` / `title`；非白名单 fallback 到 sortOrder asc。
pub fn list_todos_filtered(
    conn: &Connection,
    filter: &ListTodosFilter,
) -> rusqlite::Result<Vec<TodoRow>> {
    let mut sql = String::from("SELECT id, data_json, updated_at FROM todos WHERE 1=1");
    let mut args: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

    if let Some(c) = filter.completed {
        // JSON1 取出来通常是 1/0 / true/false；这里用 IFNULL 兜底 0
        // SQLite 中 boolean 实际就是 0/1，我们既兼容数字 1/0 也兼容字符串
        sql.push_str(
            " AND (\n              CAST(IFNULL(json_extract(data_json, '$.completed'), 0) AS INTEGER) = ?\n            )",
        );
        args.push(Box::new(if c { 1i64 } else { 0i64 }));
    }
    if let Some(ref p) = filter.priority {
        sql.push_str(" AND IFNULL(json_extract(data_json, '$.priority'), '') = ?");
        args.push(Box::new(p.clone()));
    }
    if let Some(q) = filter.quadrant {
        sql.push_str(" AND CAST(IFNULL(json_extract(data_json, '$.quadrant'), 0) AS INTEGER) = ?");
        args.push(Box::new(q));
    }
    if let Some(ref before) = filter.due_date_before {
        // 用 NULLIF(..., '') 把"空字符串"也当作 NULL，COALESCE 不再用空串兜底——
        // 否则无 dueDate/endTime 的 todo 在 SQL 比较时 `'' <= '<任何日期>'` 是 TRUE，
        // 会被错误地纳入"过期"分类（详见 due_date_before_excludes_todos_without_anchor 用例）。
        sql.push_str(" AND COALESCE(NULLIF(json_extract(data_json, '$.dueDate'), ''), NULLIF(json_extract(data_json, '$.endTime'), '')) <= ?");
        args.push(Box::new(before.clone()));
    }
    if let Some(ref after) = filter.due_date_after {
        sql.push_str(" AND COALESCE(NULLIF(json_extract(data_json, '$.dueDate'), ''), NULLIF(json_extract(data_json, '$.endTime'), '')) >= ?");
        args.push(Box::new(after.clone()));
    }
    if let Some(ref sd) = filter.start_date {
        sql.push_str(
            " AND substr(COALESCE(json_extract(data_json, '$.startTime'), json_extract(data_json, '$.startDate'), ''), 1, 10) = ?",
        );
        args.push(Box::new(sd.clone()));
    }
    if let Some(ref q) = filter.q {
        sql.push_str(
            " AND (\n              IFNULL(json_extract(data_json, '$.title'), '') LIKE ?\n           OR IFNULL(json_extract(data_json, '$.description'), '') LIKE ?\n           OR IFNULL(json_extract(data_json, '$.notes'), '') LIKE ?\n            )",
        );
        let like = format!("%{}%", q);
        args.push(Box::new(like.clone()));
        args.push(Box::new(like.clone()));
        args.push(Box::new(like));
    }

    let (sort_field_sql, asc) = match &filter.sort {
        Some((field, asc)) => (sort_expr(field.as_str()), *asc),
        None => (
            "CAST(IFNULL(json_extract(data_json, '$.sortOrder'), 0) AS INTEGER)".to_string(),
            true,
        ),
    };
    sql.push_str(&format!(
        " ORDER BY {} {}, id ASC",
        sort_field_sql,
        if asc { "ASC" } else { "DESC" }
    ));

    if let Some(l) = filter.limit {
        sql.push_str(" LIMIT ?");
        args.push(Box::new(l));
        if let Some(o) = filter.offset {
            sql.push_str(" OFFSET ?");
            args.push(Box::new(o));
        }
    } else if let Some(o) = filter.offset {
        sql.push_str(" LIMIT -1 OFFSET ?");
        args.push(Box::new(o));
    }

    let mut stmt = conn.prepare(&sql)?;
    let params_refs: Vec<&dyn rusqlite::ToSql> = args.iter().map(|b| b.as_ref()).collect();
    let rows = stmt.query_map(rusqlite::params_from_iter(params_refs), |row| {
        Ok(TodoRow {
            id: row.get(0)?,
            data_json: row.get(1)?,
            updated_at: row.get(2)?,
        })
    })?;
    rows.collect()
}

fn sort_expr(field: &str) -> String {
    match field {
        "dueDate" | "endTime" => {
            // COALESCE 支持 3 参，IFNULL 不支持
            "COALESCE(json_extract(data_json, '$.dueDate'), json_extract(data_json, '$.endTime'), '')".to_string()
        }
        "startTime" | "startDate" => {
            "COALESCE(json_extract(data_json, '$.startTime'), json_extract(data_json, '$.startDate'), '')".to_string()
        }
        "priority" => {
            // 让 high > medium > low：用 CASE 把字符串映射成可比的数字
            "CASE IFNULL(json_extract(data_json, '$.priority'), '') \
                 WHEN 'high' THEN 3 \
                 WHEN 'medium' THEN 2 \
                 WHEN 'low' THEN 1 \
                 ELSE 0 END"
                .to_string()
        }
        "quadrant" => {
            "CAST(IFNULL(json_extract(data_json, '$.quadrant'), 0) AS INTEGER)".to_string()
        }
        "sortOrder" => {
            "CAST(IFNULL(json_extract(data_json, '$.sortOrder'), 0) AS INTEGER)".to_string()
        }
        "updatedAt" => "updated_at".to_string(),
        "createdAt" => "IFNULL(json_extract(data_json, '$.createdAt'), '')".to_string(),
        "title" => "IFNULL(json_extract(data_json, '$.title'), '')".to_string(),
        _ => "CAST(IFNULL(json_extract(data_json, '$.sortOrder'), 0) AS INTEGER)".to_string(),
    }
}

/// 直接 upsert（无 LWW）。CRUD 写路径用。
pub fn upsert_todo(
    conn: &Connection,
    id: &str,
    data_json: &str,
    updated_at: &str,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO todos (id, data_json, updated_at) VALUES (?1, ?2, ?3)
         ON CONFLICT(id) DO UPDATE SET
            data_json = excluded.data_json,
            updated_at = excluded.updated_at",
        params![id, data_json, updated_at],
    )?;
    Ok(())
}

/// 删除 todo 及其全部 subtasks（同一事务内）。
pub fn delete_todo_cascade(conn: &Connection, id: &str) -> rusqlite::Result<bool> {
    let n_t = conn.execute("DELETE FROM todos WHERE id = ?1", [id])?;
    conn.execute("DELETE FROM subtasks WHERE todo_id = ?1", [id])?;
    Ok(n_t > 0)
}

/// 同步合并用：删除 todo + 全部 subtasks + `todo_seq` 行，返回删掉的 subtask 数。
/// 不写墓碑（墓碑由调用方按 K3 规则维护）。
pub fn delete_todo_with_children(conn: &Connection, id: &str) -> rusqlite::Result<usize> {
    let n_subs = conn.execute("DELETE FROM subtasks WHERE todo_id = ?1", [id])?;
    conn.execute("DELETE FROM todo_seq WHERE todo_id = ?1", [id])?;
    conn.execute("DELETE FROM todos WHERE id = ?1", [id])?;
    Ok(n_subs)
}

/// 同步合并用：`id → updated_at`（原样，比较前由调用方规范化）。
pub fn todo_timestamps(conn: &Connection) -> rusqlite::Result<HashMap<String, String>> {
    let mut stmt = conn.prepare("SELECT id, updated_at FROM todos")?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    rows.collect()
}

/// 同步合并用：`id → (todo_id, updated_at)`。
pub fn subtask_timestamps(
    conn: &Connection,
) -> rusqlite::Result<HashMap<String, (String, String)>> {
    let mut stmt = conn.prepare("SELECT id, todo_id, updated_at FROM subtasks")?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            (row.get::<_, String>(1)?, row.get::<_, String>(2)?),
        ))
    })?;
    rows.collect()
}

pub fn list_subtasks_for_todo(
    conn: &Connection,
    todo_id: &str,
) -> rusqlite::Result<Vec<SubtaskRow>> {
    let mut stmt = conn.prepare(
        "SELECT id, todo_id, data_json, updated_at FROM subtasks WHERE todo_id = ?1
         ORDER BY CAST(json_extract(data_json, '$.sortOrder') AS INTEGER) ASC, id ASC",
    )?;
    let rows = stmt.query_map([todo_id], |row| {
        Ok(SubtaskRow {
            id: row.get(0)?,
            todo_id: row.get(1)?,
            data_json: row.get(2)?,
            updated_at: row.get(3)?,
        })
    })?;
    rows.collect()
}

pub fn count_subtasks_for_todo(conn: &Connection, todo_id: &str) -> rusqlite::Result<i64> {
    conn.query_row(
        "SELECT COUNT(*) FROM subtasks WHERE todo_id = ?1",
        [todo_id],
        |row| row.get(0),
    )
}

pub fn get_subtask(conn: &Connection, id: &str) -> rusqlite::Result<Option<SubtaskRow>> {
    conn.query_row(
        "SELECT id, todo_id, data_json, updated_at FROM subtasks WHERE id = ?1",
        [id],
        |row| {
            Ok(SubtaskRow {
                id: row.get(0)?,
                todo_id: row.get(1)?,
                data_json: row.get(2)?,
                updated_at: row.get(3)?,
            })
        },
    )
    .optional()
}

/// 直接 upsert subtask（CRUD 写路径）。
pub fn upsert_subtask(
    conn: &Connection,
    id: &str,
    todo_id: &str,
    data_json: &str,
    updated_at: &str,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO subtasks (id, todo_id, data_json, updated_at) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(id) DO UPDATE SET
            todo_id   = excluded.todo_id,
            data_json = excluded.data_json,
            updated_at = excluded.updated_at",
        params![id, todo_id, data_json, updated_at],
    )?;
    Ok(())
}

pub fn delete_subtask(conn: &Connection, id: &str) -> rusqlite::Result<bool> {
    let n = conn.execute("DELETE FROM subtasks WHERE id = ?1", [id])?;
    Ok(n > 0)
}

/// 全表枚举所有 todos（push worker merge 用）。
pub fn all_todos(conn: &Connection) -> rusqlite::Result<Vec<TodoRow>> {
    let mut stmt = conn.prepare("SELECT id, data_json, updated_at FROM todos")?;
    let rows = stmt.query_map([], |row| {
        Ok(TodoRow {
            id: row.get(0)?,
            data_json: row.get(1)?,
            updated_at: row.get(2)?,
        })
    })?;
    rows.collect()
}

/// 全表枚举所有 subtasks（push worker merge 用）。
pub fn all_subtasks(conn: &Connection) -> rusqlite::Result<Vec<SubtaskRow>> {
    let mut stmt = conn.prepare("SELECT id, todo_id, data_json, updated_at FROM subtasks")?;
    let rows = stmt.query_map([], |row| {
        Ok(SubtaskRow {
            id: row.get(0)?,
            todo_id: row.get(1)?,
            data_json: row.get(2)?,
            updated_at: row.get(3)?,
        })
    })?;
    rows.collect()
}

/// 删除 id 不在 `keep` 集合内的 todos + 对应 subtasks + todo_seq。
/// 仅供 pull 的旧协议兼容路径（远端文档没有 `tombstones` 键且本地不 dirty）使用。
pub fn delete_todos_not_in(conn: &Connection, keep: &HashSet<String>) -> rusqlite::Result<usize> {
    let mut stmt = conn.prepare("SELECT id FROM todos")?;
    let local_ids: Vec<String> = stmt
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<_>>()?;

    let mut n = 0usize;
    for id in &local_ids {
        if !keep.contains(id) {
            conn.execute("DELETE FROM subtasks WHERE todo_id = ?1", [id])?;
            conn.execute("DELETE FROM todo_seq WHERE todo_id = ?1", [id])?;
            conn.execute("DELETE FROM todos WHERE id = ?1", [id])?;
            n += 1;
        }
    }
    Ok(n)
}

/// 删除 id 不在 `keep` 集合内的 subtasks（旧协议兼容路径用）。
pub fn delete_subtasks_not_in(
    conn: &Connection,
    keep: &HashSet<String>,
) -> rusqlite::Result<usize> {
    let mut stmt = conn.prepare("SELECT id FROM subtasks")?;
    let local_ids: Vec<String> = stmt
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<_>>()?;

    let mut n = 0usize;
    for id in &local_ids {
        if !keep.contains(id) {
            conn.execute("DELETE FROM subtasks WHERE id = ?1", [id])?;
            n += 1;
        }
    }
    Ok(n)
}

// =============================================================================
// Tombstones（软删除标记，K2/K3：双向传播）
// =============================================================================

/// 写入墓碑；同键已存在时保留较大的 `deleted_at`（K3-4：墓碑集合同键取较大值）。
/// `deleted_at` 必须是规范格式，字符串比较即时间比较。
pub fn add_tombstone(
    conn: &Connection,
    entity_type: &str,
    entity_id: &str,
    deleted_at: &str,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO tombstones (entity_type, entity_id, deleted_at) VALUES (?1, ?2, ?3)
         ON CONFLICT(entity_type, entity_id) DO UPDATE
         SET deleted_at = MAX(tombstones.deleted_at, excluded.deleted_at)",
        params![entity_type, entity_id, deleted_at],
    )?;
    Ok(())
}

/// 返回所有 tombstones 的 `(entity_type, entity_id, deleted_at)` 列表。
pub fn list_tombstones(conn: &Connection) -> rusqlite::Result<Vec<(String, String, String)>> {
    let mut stmt = conn.prepare("SELECT entity_type, entity_id, deleted_at FROM tombstones")?;
    let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?;
    rows.collect()
}

/// 一次查询：墓碑是否存在。供 push merge 路径以外（如调试 / 未来路由）使用。
#[allow(dead_code)]
pub fn has_tombstone(
    conn: &Connection,
    entity_type: &str,
    entity_id: &str,
) -> rusqlite::Result<bool> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM tombstones WHERE entity_type = ?1 AND entity_id = ?2",
        params![entity_type, entity_id],
        |row| row.get(0),
    )?;
    Ok(count > 0)
}

/// 清理早于 `cutoff_local` 的 tombstones（保留期 30 天，合并与 PUT 成功后调用）。
pub fn purge_tombstones_before(conn: &Connection, cutoff_local: &str) -> rusqlite::Result<usize> {
    let n = conn.execute(
        "DELETE FROM tombstones WHERE deleted_at < ?1",
        [cutoff_local],
    )?;
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn fresh() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        crate::db::schema::init(&c).unwrap();
        c
    }

    fn insert_todo(c: &Connection, id: &str, data: &str, updated_at: &str) {
        upsert_todo(c, id, data, updated_at).unwrap();
    }

    #[test]
    fn filter_by_completed() {
        let c = fresh();
        insert_todo(
            &c,
            "1",
            r#"{"id":1,"title":"a","completed":true}"#,
            "2026-05-13 10:00:00",
        );
        insert_todo(
            &c,
            "2",
            r#"{"id":2,"title":"b","completed":false}"#,
            "2026-05-13 10:00:00",
        );
        let rows = list_todos_filtered(
            &c,
            &ListTodosFilter {
                completed: Some(false),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "2");
    }

    /// Regression test: 早期版本写成 `IFNULL(a, b, '')` 三参，SQLite 在 prepare
    /// 阶段就报 "wrong number of arguments to function IFNULL()"。改成 COALESCE
    /// 后此类 query 必须能正常返回数据。
    /// Regression: 无 dueDate / endTime 的 todo **不应**被 dueDateBefore 命中。
    /// 早期版本 COALESCE(..., ..., '') 拿空串兜底，导致 `'' <= '2026-...'` 在 SQL
    /// 字典序里恒为 TRUE，所有"未设截止时间"的未完成 todo 被误判为过期，
    /// `today` 子命令的 overdue 分支因此把"买洗内裤的"这种纯任务也卷进去推送。
    #[test]
    fn due_date_before_excludes_todos_without_anchor() {
        let c = fresh();
        // 一条真正过期
        insert_todo(
            &c,
            "1",
            r#"{"id":1,"title":"overdue","dueDate":"2026-05-10","completed":false}"#,
            "2026-05-10 10:00:00",
        );
        // 一条无任何时间锚，仅 quadrant=4
        insert_todo(
            &c,
            "2",
            r#"{"id":2,"title":"no time","quadrant":4,"completed":false}"#,
            "2026-05-14 10:00:00",
        );
        // 一条 dueDate 是空字符串（用户改过后清空）—— 也应被排除
        insert_todo(
            &c,
            "3",
            r#"{"id":3,"title":"empty dueDate","dueDate":"","endTime":"","completed":false}"#,
            "2026-05-14 11:00:00",
        );

        let rows = list_todos_filtered(
            &c,
            &ListTodosFilter {
                due_date_before: Some("2026-05-14T00:00:00".to_string()),
                ..Default::default()
            },
        )
        .expect("query must succeed");
        // 只应命中真正过期那条
        assert_eq!(
            rows.len(),
            1,
            "should only match the genuinely overdue todo"
        );
        assert_eq!(rows[0].id, "1");
    }

    #[test]
    fn filter_by_due_date_before_uses_coalesce() {
        let c = fresh();
        insert_todo(
            &c,
            "1",
            r#"{"id":1,"title":"due","dueDate":"2026-05-13"}"#,
            "2026-05-13 10:00:00",
        );
        insert_todo(
            &c,
            "2",
            r#"{"id":2,"title":"end","endTime":"2026-05-14"}"#,
            "2026-05-13 10:00:00",
        );
        let rows = list_todos_filtered(
            &c,
            &ListTodosFilter {
                due_date_before: Some("2026-05-13".to_string()),
                ..Default::default()
            },
        )
        .expect("query must succeed; 3-arg IFNULL would error here");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "1");
    }

    #[test]
    fn filter_by_due_date_after_uses_coalesce() {
        let c = fresh();
        insert_todo(
            &c,
            "1",
            r#"{"id":1,"title":"due","dueDate":"2026-05-13"}"#,
            "2026-05-13 10:00:00",
        );
        let rows = list_todos_filtered(
            &c,
            &ListTodosFilter {
                due_date_after: Some("2026-05-12".to_string()),
                ..Default::default()
            },
        )
        .expect("query must succeed");
        assert_eq!(rows.len(), 1);
    }

    #[test]
    fn filter_by_start_date_uses_coalesce() {
        let c = fresh();
        insert_todo(
            &c,
            "1",
            r#"{"id":1,"title":"a","startTime":"2026-05-13 09:00:00"}"#,
            "2026-05-13 10:00:00",
        );
        insert_todo(
            &c,
            "2",
            r#"{"id":2,"title":"b","startDate":"2026-05-14"}"#,
            "2026-05-13 10:00:00",
        );
        let rows = list_todos_filtered(
            &c,
            &ListTodosFilter {
                start_date: Some("2026-05-13".to_string()),
                ..Default::default()
            },
        )
        .expect("query must succeed");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "1");
    }

    #[test]
    fn sort_by_due_date_desc_uses_coalesce() {
        let c = fresh();
        insert_todo(
            &c,
            "1",
            r#"{"id":1,"title":"a","dueDate":"2026-05-13"}"#,
            "2026-05-13 10:00:00",
        );
        insert_todo(
            &c,
            "2",
            r#"{"id":2,"title":"b","dueDate":"2026-05-14"}"#,
            "2026-05-13 10:00:00",
        );
        let rows = list_todos_filtered(
            &c,
            &ListTodosFilter {
                sort: Some(("dueDate".to_string(), false)),
                ..Default::default()
            },
        )
        .expect("query must succeed");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].id, "2"); // 2026-05-14 排前
        assert_eq!(rows[1].id, "1");
    }

    #[test]
    fn tombstone_insert_list_purge() {
        let c = fresh();
        add_tombstone(&c, "todo", "1", "2026-05-13 10:00:00").unwrap();
        add_tombstone(&c, "subtask", "2", "2026-05-13 11:00:00").unwrap();
        let all = list_tombstones(&c).unwrap();
        assert_eq!(all.len(), 2);
        assert!(has_tombstone(&c, "todo", "1").unwrap());
        // purge_before：清理早于 cutoff 的
        let n = purge_tombstones_before(&c, "2026-05-13 10:30:00").unwrap();
        assert_eq!(n, 1); // 只清理掉 "10:00:00" 那条
        let remaining = list_tombstones(&c).unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].1, "2");
    }

    #[test]
    fn meta_kv_roundtrip() {
        let c = fresh();
        assert_eq!(get_meta(&c, "dirty").unwrap(), None);
        set_meta(&c, "dirty", "true").unwrap();
        assert_eq!(get_meta(&c, "dirty").unwrap().as_deref(), Some("true"));
        set_meta(&c, "dirty", "false").unwrap();
        assert_eq!(get_meta(&c, "dirty").unwrap().as_deref(), Some("false"));
    }

    /// DB 错误不得被吞成 `None`——否则调用方无法区分"键不存在"和"读失败"。
    #[test]
    fn get_meta_propagates_db_error() {
        let c = fresh();
        c.execute_batch("DROP TABLE meta").unwrap();
        assert!(
            get_meta(&c, "dirty").is_err(),
            "meta 表不存在时必须报错而不是返回 None"
        );
    }

    #[test]
    fn mark_dirty_sets_flag_and_bumps_generation() {
        let c = fresh();
        assert_eq!(get_dirty_generation(&c).unwrap(), 0);
        mark_dirty(&c).unwrap();
        assert_eq!(get_meta(&c, "dirty").unwrap().as_deref(), Some("true"));
        assert_eq!(get_dirty_generation(&c).unwrap(), 1);
        mark_dirty(&c).unwrap();
        assert_eq!(get_dirty_generation(&c).unwrap(), 2);
    }

    #[test]
    fn get_dirty_generation_defaults_to_zero_on_garbage() {
        let c = fresh();
        set_meta(&c, "dirty_generation", "not-a-number").unwrap();
        assert_eq!(get_dirty_generation(&c).unwrap(), 0);
    }

    #[test]
    fn mark_dirty_records_dirty_since_once_and_clear_removes_it() {
        let c = fresh();
        mark_dirty(&c).unwrap();
        let since = get_meta(&c, mk::DIRTY_SINCE).unwrap().expect("dirty_since");
        assert!(since.parse::<i64>().is_ok());
        set_meta(&c, mk::DIRTY_SINCE, "123").unwrap();
        mark_dirty(&c).unwrap();
        assert_eq!(
            get_meta(&c, mk::DIRTY_SINCE).unwrap().as_deref(),
            Some("123"),
            "已经脏着时不能刷新起始时间"
        );
        let g = get_dirty_generation(&c).unwrap();
        assert!(clear_dirty_if_unchanged(&c, g).unwrap());
        assert!(!is_dirty(&c).unwrap());
        assert_eq!(get_meta(&c, mk::DIRTY_SINCE).unwrap(), None);
    }

    /// `C{seq}` 高水位：删掉当前最大号后新 todo 也不复用（旧实现 MAX(seq)+1 会复用）。
    #[test]
    fn seq_never_reuses_deleted_max() {
        let c = fresh();
        assert_eq!(assign_seq(&c, "a").unwrap(), 1);
        assert_eq!(assign_seq(&c, "b").unwrap(), 2);
        assert_eq!(assign_seq(&c, "b").unwrap(), 2, "幂等");
        delete_seq(&c, "b").unwrap();
        assert_eq!(assign_seq(&c, "c").unwrap(), 3);
        delete_seq(&c, "a").unwrap();
        delete_seq(&c, "c").unwrap();
        assert_eq!(assign_seq(&c, "d").unwrap(), 4, "表清空后仍从高水位继续");
        assert_eq!(get_meta(&c, mk::SEQ_HWM).unwrap().as_deref(), Some("4"));
    }

    /// 旧库没有 seq_hwm：从表里现有最大值起步。
    #[test]
    fn seq_hwm_bootstraps_from_existing_table() {
        let c = fresh();
        c.execute_batch("INSERT INTO todo_seq (todo_id, seq) VALUES ('x', 7)")
            .unwrap();
        assert_eq!(assign_seq(&c, "y").unwrap(), 8);
    }

    #[test]
    fn image_queue_remove_keeps_names_added_meanwhile() {
        let c = fresh();
        assert!(dirty_image_queue(&c).unwrap().is_empty());
        enqueue_dirty_image(&c, "a.png").unwrap();
        enqueue_dirty_image(&c, "a.png").unwrap();
        assert_eq!(dirty_image_queue(&c).unwrap(), vec!["a.png"]);
        // 上传 a.png 期间又来了 b.png
        enqueue_dirty_image(&c, "b.png").unwrap();
        let done: HashSet<String> = ["a.png".to_string()].into_iter().collect();
        assert_eq!(remove_dirty_images(&c, &done).unwrap(), vec!["b.png"]);
        assert_eq!(dirty_image_queue(&c).unwrap(), vec!["b.png"]);
        // 损坏的队列值视为空队列
        set_meta(&c, mk::DIRTY_IMAGES, "not json").unwrap();
        assert!(dirty_image_queue(&c).unwrap().is_empty());
    }

    #[test]
    fn add_tombstone_keeps_latest_deleted_at() {
        let c = fresh();
        add_tombstone(&c, "todo", "1", "2026-05-13 10:00:00").unwrap();
        add_tombstone(&c, "todo", "1", "2026-05-12 10:00:00").unwrap();
        assert_eq!(list_tombstones(&c).unwrap()[0].2, "2026-05-13 10:00:00");
        add_tombstone(&c, "todo", "1", "2026-05-14 10:00:00").unwrap();
        assert_eq!(list_tombstones(&c).unwrap()[0].2, "2026-05-14 10:00:00");
    }

    #[test]
    fn delete_todo_with_children_removes_subtasks_and_seq() {
        let c = fresh();
        insert_todo(&c, "1", r#"{"id":1}"#, "2026-05-13 10:00:00");
        upsert_subtask(&c, "11", "1", r#"{"id":11}"#, "2026-05-13 10:00:00").unwrap();
        upsert_subtask(&c, "12", "1", r#"{"id":12}"#, "2026-05-13 10:00:00").unwrap();
        assign_seq(&c, "1").unwrap();
        assert_eq!(delete_todo_with_children(&c, "1").unwrap(), 2);
        assert!(get_todo(&c, "1").unwrap().is_none());
        assert_eq!(count_subtasks_for_todo(&c, "1").unwrap(), 0);
        assert_eq!(get_seq(&c, "1").unwrap(), None);
        assert!(tombstone_free(&c));
    }

    fn tombstone_free(c: &Connection) -> bool {
        list_tombstones(c).unwrap().is_empty()
    }

    #[test]
    fn timestamp_indexes() {
        let c = fresh();
        insert_todo(&c, "1", r#"{"id":1}"#, "2026-05-13 10:00:00");
        upsert_subtask(&c, "11", "1", r#"{"id":11}"#, "2026-05-13 11:00:00").unwrap();
        assert_eq!(
            todo_timestamps(&c).unwrap().get("1").map(String::as_str),
            Some("2026-05-13 10:00:00")
        );
        assert_eq!(
            subtask_timestamps(&c).unwrap().get("11"),
            Some(&("1".to_string(), "2026-05-13 11:00:00".to_string()))
        );
    }
}
