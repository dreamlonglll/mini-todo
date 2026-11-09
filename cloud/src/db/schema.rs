//! 启动时建表 + 幂等升级。Schema 设计参考 prd：4 张表 + tombstones 表 + todo_seq，
//! 多清单（PC 迁移 v31 起）另加 todo_lists + list_tombstones。

use rusqlite::Connection;

use crate::db::repo;

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

        -- 软删除墓碑（跨端契约 K2/K3）：DELETE /todos/:id /subtasks/:id 时写入，
        -- 也接收远端 sync-data `tombstones` 数组里的墓碑；双向传播。
        -- 墓碑只压制 `updated_at <= deleted_at` 的记录（删除后又被编辑的记录保留）。
        --
        -- entity_type ∈ {'todo', 'subtask'}；deleted_at 用规范本地时间字符串
        -- `YYYY-MM-DD HH:MM:SS`。保留期 30 天：合并与 push 成功后清理。
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

        -- 待办清单（多清单；契约 .trellis/tasks/11-09-multi-list-windows/contract.md）。
        -- KV-style，与 todos 同构：data_json 是同步文档 `todoLists` 里的清单对象原样
        -- （{id, name, sortOrder, createdAt, updatedAt}，未知字段原样保留），updated_at 是
        -- 规范化后的 updatedAt（LWW 比较用）。清单不分配 `C{seq}` 短码。
        -- 默认清单 id 1 可改名、不可删除；本地没有时按固定初始值补建（repo::ensure_default_list）。
        CREATE TABLE IF NOT EXISTS todo_lists (
            id          TEXT PRIMARY KEY,
            data_json   TEXT NOT NULL,
            updated_at  TEXT NOT NULL
        );

        -- 清单删除墓碑（同步文档顶层 `listTombstones`）。与 tombstones 分开存放、分开传播：
        -- 旧版本（≤ 3.0.x）的 parse_tombstone 丢弃未知 entityType，上传时又按本地表重建
        -- `tombstones`，清单墓碑放进去会被旧设备抹掉。规则与 todo 墓碑相同（同键取较大
        -- deleted_at、保留 30 天）；默认清单（entity_id = '1'）的墓碑一律不存。
        CREATE TABLE IF NOT EXISTS list_tombstones (
            entity_id   TEXT PRIMARY KEY,
            deleted_at  TEXT NOT NULL
        );
        "#,
    )
    .map_err(|e| anyhow::anyhow!("初始化 schema 失败: {}", e))?;

    upgrade(conn).map_err(|e| anyhow::anyhow!("升级 meta 失败: {}", e))?;
    repo::ensure_default_list(conn).map_err(|e| anyhow::anyhow!("补建默认清单失败: {}", e))?;
    Ok(())
}

/// 幂等的数据升级，每次启动都跑。
fn upgrade(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        r#"
        -- 旧版 push 把"PUT 之后再 GET 拿到的 ETag"存成 last_etag，而那个版本的内容
        -- 并没有合并进缓存（审查 A4），继续拿它做 If-None-Match 会让 pull 永远 304。
        -- 新版的"基准"改存 base_etag / base_last_modified / remote_envelope 三键，
        -- 旧键直接作废：升级后第一次 pull 必然全量 GET 并合并。
        DELETE FROM meta WHERE key IN ('last_etag', 'last_modified');

        -- 升级前就已经 dirty 的库补一个 dirty_since（UNIX 秒），健康检查才能算积压时长。
        INSERT INTO meta (key, value)
        SELECT 'dirty_since', CAST(strftime('%s', 'now') AS TEXT)
        WHERE EXISTS (SELECT 1 FROM meta WHERE key = 'dirty' AND value = 'true')
          AND NOT EXISTS (SELECT 1 FROM meta WHERE key = 'dirty_since');
        "#,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_is_idempotent_and_drops_obsolete_base_keys() {
        let c = Connection::open_in_memory().unwrap();
        init(&c).unwrap();
        c.execute_batch(
            "INSERT INTO meta (key, value) VALUES ('last_etag', '\"x\"'), ('dirty', 'true');",
        )
        .unwrap();
        init(&c).unwrap();
        let n: i64 = c
            .query_row(
                "SELECT COUNT(*) FROM meta WHERE key = 'last_etag'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 0, "旧的 last_etag 必须作废");
        let since: Option<String> = c
            .query_row(
                "SELECT value FROM meta WHERE key = 'dirty_since'",
                [],
                |r| r.get(0),
            )
            .ok();
        assert!(since.is_some(), "已 dirty 的库要补 dirty_since");
    }

    /// 默认清单按固定初始值补建；已有（例如已改名）时 init 不动它。
    #[test]
    fn init_creates_default_list_once() {
        let c = Connection::open_in_memory().unwrap();
        init(&c).unwrap();
        let row = repo::all_list_rows(&c).unwrap();
        assert_eq!(row.len(), 1);
        assert_eq!(row[0].id, "1");
        assert_eq!(row[0].updated_at, crate::model::DEFAULT_LIST_TIME);
        let v: serde_json::Value = serde_json::from_str(&row[0].data_json).unwrap();
        assert_eq!(
            v,
            serde_json::Value::Object(crate::model::default_list_record())
        );

        repo::upsert_list(
            &c,
            "1",
            r#"{"id":1,"name":"收件箱","sortOrder":0,"createdAt":"2000-01-01 00:00:00","updatedAt":"2026-11-01 10:00:00"}"#,
            "2026-11-01 10:00:00",
        )
        .unwrap();
        init(&c).unwrap();
        let row = repo::all_list_rows(&c).unwrap();
        assert_eq!(row.len(), 1);
        assert!(
            row[0].data_json.contains("收件箱"),
            "已有的默认清单不能被覆盖"
        );
    }
}
