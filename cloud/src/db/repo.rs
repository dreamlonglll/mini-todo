//! 仓储层：本 PR 只需 list / upsert / meta KV / settings KV，足够支撑 pull
//! worker 与 /health。后续 PR2 会再加 PATCH / DELETE / 子任务 CRUD 等。

use rusqlite::{params, Connection, OptionalExtension};

/// 单条 todo 在 SQLite 中的快照：`data_json` 是 PC 端 todo 对象的 JSON 原样存储。
///
/// `id` / `data_json` 字段当前只在 LWW 写入路径作为输入参数被消费，PR2 CRUD
/// 读取端会真正用到；先 allow dead_code 避免噪音。
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct TodoRow {
    pub id: String,
    pub data_json: String,
    pub updated_at: String,
}

/// 单条 subtask 在 SQLite 中的快照。同 `TodoRow`。
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct SubtaskRow {
    pub id: String,
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
// settings KV（与 SyncData.settings JSON 字段对应；本 PR 仅 worker 用）
// =============================================================================

#[allow(dead_code)]
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

#[allow(dead_code)]
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

#[allow(dead_code)]
pub fn count_todos(conn: &Connection) -> rusqlite::Result<i64> {
    conn.query_row("SELECT COUNT(*) FROM todos", [], |row| row.get(0))
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
    conn.execute(
        "INSERT INTO todos (id, data_json, updated_at) VALUES (?1, ?2, ?3)
         ON CONFLICT(id) DO UPDATE SET
            data_json = excluded.data_json,
            updated_at = excluded.updated_at",
        params![id, data_json, updated_at],
    )?;
    Ok(true)
}

#[allow(dead_code)]
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

#[allow(dead_code)]
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
    conn.execute(
        "INSERT INTO subtasks (id, todo_id, data_json, updated_at) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(id) DO UPDATE SET
            todo_id   = excluded.todo_id,
            data_json = excluded.data_json,
            updated_at = excluded.updated_at",
        params![id, todo_id, data_json, updated_at],
    )?;
    Ok(true)
}
