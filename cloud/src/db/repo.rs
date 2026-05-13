//! 仓储层：list / upsert / patch / delete / meta KV / settings KV / tombstones。
//!
//! Schema 是 KV-style（`todos(id, data_json, updated_at)` /
//! `subtasks(id, todo_id, data_json, updated_at)` / `settings(key, value)` /
//! `meta(key, value)`）。所有过滤 / 排序通过 SQLite JSON1 函数对 `data_json`
//! 做提取。

use rusqlite::{params, Connection, OptionalExtension};

/// 单条 todo 在 SQLite 中的快照：`data_json` 是 PC 端 todo 对象的 JSON 原样存储。
#[derive(Debug, Clone)]
pub struct TodoRow {
    #[allow(dead_code)]
    pub id: String,
    pub data_json: String,
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
    pub updated_at: String,
}

// =============================================================================
// meta KV
// =============================================================================

pub fn get_meta(conn: &Connection, key: &str) -> Option<String> {
    conn.query_row("SELECT value FROM meta WHERE key = ?1", [key], |row| {
        row.get::<_, String>(0)
    })
    .optional()
    .ok()
    .flatten()
}

pub fn set_meta(conn: &Connection, key: &str, value: &str) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO meta (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
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

#[allow(dead_code)]
pub fn get_setting(conn: &Connection, key: &str) -> Option<String> {
    conn.query_row("SELECT value FROM settings WHERE key = ?1", [key], |row| {
        row.get::<_, String>(0)
    })
    .optional()
    .ok()
    .flatten()
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

#[allow(dead_code)]
pub fn list_todos(conn: &Connection) -> rusqlite::Result<Vec<TodoRow>> {
    let mut stmt = conn.prepare(
        "SELECT id, data_json, updated_at FROM todos
         ORDER BY CAST(json_extract(data_json, '$.sortOrder') AS INTEGER) ASC, id ASC",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(TodoRow {
            id: row.get(0)?,
            data_json: row.get(1)?,
            updated_at: row.get(2)?,
        })
    })?;
    rows.collect()
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
        // 用 COALESCE（支持 N 参数）替代 IFNULL（仅 2 参）；
        // SQLite 的 IFNULL 严格只接受 2 参，3 参会直接 prepare 报错。
        sql.push_str(" AND COALESCE(json_extract(data_json, '$.dueDate'), json_extract(data_json, '$.endTime'), '') <= ?");
        args.push(Box::new(before.clone()));
    }
    if let Some(ref after) = filter.due_date_after {
        sql.push_str(" AND COALESCE(json_extract(data_json, '$.dueDate'), json_extract(data_json, '$.endTime'), '') >= ?");
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

#[allow(dead_code)]
pub fn count_todos(conn: &Connection) -> rusqlite::Result<i64> {
    conn.query_row("SELECT COUNT(*) FROM todos", [], |row| row.get(0))
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

/// per-record LWW upsert：仅在远端 `updated_at` ≥ 本地（或本地不存在）时写入。
/// 返回是否实际写入。
pub fn upsert_todo_if_newer(
    conn: &Connection,
    id: &str,
    data_json: &str,
    updated_at: &str,
) -> rusqlite::Result<bool> {
    let existing = get_todo(conn, id)?;
    let should_write = match existing {
        Some(row) => updated_at >= row.updated_at.as_str(),
        None => true,
    };
    if !should_write {
        return Ok(false);
    }
    upsert_todo(conn, id, data_json, updated_at)?;
    Ok(true)
}

/// 删除 todo 及其全部 subtasks（同一事务内）。
pub fn delete_todo_cascade(conn: &Connection, id: &str) -> rusqlite::Result<bool> {
    let n_t = conn.execute("DELETE FROM todos WHERE id = ?1", [id])?;
    conn.execute("DELETE FROM subtasks WHERE todo_id = ?1", [id])?;
    Ok(n_t > 0)
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

pub fn upsert_subtask_if_newer(
    conn: &Connection,
    id: &str,
    todo_id: &str,
    data_json: &str,
    updated_at: &str,
) -> rusqlite::Result<bool> {
    let existing = get_subtask(conn, id)?;
    let should_write = match existing {
        Some(row) => updated_at >= row.updated_at.as_str(),
        None => true,
    };
    if !should_write {
        return Ok(false);
    }
    upsert_subtask(conn, id, todo_id, data_json, updated_at)?;
    Ok(true)
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

// =============================================================================
// Tombstones（软删除标记，push worker merge 用）
// =============================================================================

pub fn add_tombstone(
    conn: &Connection,
    entity_type: &str,
    entity_id: &str,
    deleted_at: &str,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO tombstones (entity_type, entity_id, deleted_at) VALUES (?1, ?2, ?3)
         ON CONFLICT(entity_type, entity_id) DO UPDATE SET deleted_at = excluded.deleted_at",
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

/// 清理早于 `cutoff_local` 的 tombstones。push worker 每次 PUT 成功后调用。
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
        assert_eq!(get_meta(&c, "dirty"), None);
        set_meta(&c, "dirty", "true").unwrap();
        assert_eq!(get_meta(&c, "dirty").as_deref(), Some("true"));
        set_meta(&c, "dirty", "false").unwrap();
        assert_eq!(get_meta(&c, "dirty").as_deref(), Some("false"));
    }
}
