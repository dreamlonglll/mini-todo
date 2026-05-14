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

        -- todo 短码（cloud-only）：给每个 todo 分配一个从 1 起单调递增的
        -- `seq`，用于 LLM / 用户反馈时的 `C{seq}` 短引用（i64 完整 id 16 位
        -- 太长不方便口语反馈）。
        --
        -- 为什么独立表而不是塞进 data_json：
        --   PC 端 Todo struct 是严格 typed、不含 seq 字段，serde 默认丢弃
        --   未知字段。若 seq 进 data_json，cloud 写回 WebDAV 后 PC pull
        --   会丢掉、再 export 又没了，下次 cloud pull 进来又判定为"无 seq"
        --   再分配新号——seq 会无限增长且不稳定。独立表 cloud 自家持有，
        --   pull merge 完全不动它，cloud SQLite 文件不删 seq 就稳定。
        CREATE TABLE IF NOT EXISTS todo_seq (
            todo_id TEXT PRIMARY KEY,
            seq     INTEGER NOT NULL UNIQUE
        );
        "#,
    )
    .map_err(|e| anyhow::anyhow!("初始化 schema 失败: {}", e))?;
    Ok(())
}
