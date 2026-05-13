//! 启动时建表。Schema 设计参考 prd：4 张表 + tombstones 表。

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

        -- 软删除墓碑：DELETE /todos/:id /subtasks/:id 时写入；push worker
        -- merge 时使用——本地有 tombstone 而远端有 record → 删除（防止远端
        -- 陈旧数据复活已删除的本地 record）。
        --
        -- entity_type ∈ {'todo', 'subtask'}；deleted_at 用 PC 风格的本地时间
        -- 字符串。过期清理在 push 完成后做（>7 天移除）。
        CREATE TABLE IF NOT EXISTS tombstones (
            entity_type TEXT NOT NULL,
            entity_id   TEXT NOT NULL,
            deleted_at  TEXT NOT NULL,
            PRIMARY KEY (entity_type, entity_id)
        );
        "#,
    )
    .map_err(|e| anyhow::anyhow!("初始化 schema 失败: {}", e))?;
    Ok(())
}
