//! 启动时建表。Schema 设计参考 prd：4 张表，KV + JSON1。

use rusqlite::Connection;

pub fn init(conn: &Connection) -> anyhow::Result<()> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS todos (
            id          TEXT PRIMARY KEY,
            data_json   TEXT NOT NULL,
            updated_at  TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS subtasks (
            id          TEXT PRIMARY KEY,
            todo_id     TEXT NOT NULL,
            data_json   TEXT NOT NULL,
            updated_at  TEXT NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_subtasks_todo_id ON subtasks(todo_id);

        CREATE TABLE IF NOT EXISTS settings (
            key   TEXT PRIMARY KEY,
            value TEXT
        );

        CREATE TABLE IF NOT EXISTS meta (
            key   TEXT PRIMARY KEY,
            value TEXT
        );
        "#,
    )
    .map_err(|e| anyhow::anyhow!("初始化 schema 失败: {}", e))?;
    Ok(())
}
