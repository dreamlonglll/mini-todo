//! 同步在 SQLite 侧的存储与合并（K2 / K3）。
//!
//! - `sync_meta`：本地变更计数 `local_seq`（触发器递增）与最近一次成功同步时的 `synced_seq`
//! - `tombstones`：删除墓碑，30 天保留；`list_tombstones`：清单删除墓碑，永不过期（契约 §13-10）
//! - 远端记录合并：记录级 LWW（平局保留本地）+ 墓碑压制 + 并集，**没有**"缺席即删除"；
//!   待办的清单归属另按 `(listId, listUpdatedAt)` 做字段级 LWW（[`ListAssignment`]，契约 §13）
//! - 强制拉取 / 强制推送的本地侧操作
//! - 本地快照（一次查询待办、一次查询子任务，无 N+1）
//!
//! 网络与 JSON 文档拼装在 `sync` 模块；本模块只认已解析的记录和 SQLite。

use rusqlite::{Connection, OptionalExtension, Result};

mod merge;
mod parse;
#[cfg(test)]
mod tests;
mod tombstones;

pub use merge::{
    force_pull_replace, force_pull_replace_lists, force_push_prepare, force_push_prepare_lists,
    insert_list_row, insert_subtask_row, insert_todo_row, load_lists, load_todos_with_subtasks,
    local_records, merge_remote, merge_remote_lists, retention_cutoff, update_list_row,
    update_subtask_row, update_todo_row, LocalRecords, MergeStats,
};
pub use parse::{
    normalize_list, normalize_list_lenient, normalize_subtask, normalize_todo,
    normalize_todo_lenient, parse_list_tombstone, parse_remote_lists, parse_remote_todos,
    parse_tombstone, value_id, value_subtasks, value_updated_at, ListAssignment, RemoteList,
    RemoteSubtask, RemoteTodo,
};
pub(crate) use tombstones::deletion_time;
pub use tombstones::{
    delete_subtask_with_tombstone, delete_todo_with_tombstones, is_list_suppressed, is_suppressed,
    list_tombstones, load_list_tombstones, purge_tombstones_before, record_list_tombstone,
    record_tombstone, remove_list_tombstone, remove_tombstone, ListTombstoneIndex, TombstoneIndex,
};

/// 墓碑保留天数（K2）
pub const TOMBSTONE_RETENTION_DAYS: i64 = 30;

/// 墓碑 / 记录的实体类型
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum EntityKind {
    Todo,
    Subtask,
}

impl EntityKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Todo => "todo",
            Self::Subtask => "subtask",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "todo" => Some(Self::Todo),
            "subtask" => Some(Self::Subtask),
            _ => None,
        }
    }
}

// ============================================================================
// sync_meta
// ============================================================================

fn get_meta(conn: &Connection, key: &str) -> Result<i64> {
    conn.query_row("SELECT value FROM sync_meta WHERE key = ?1", [key], |r| {
        r.get(0)
    })
    .optional()
    .map(|v| v.unwrap_or(0))
}

/// 本地变更计数：todos / subtasks 任意增删改都会递增（v28 触发器）
pub fn local_seq(conn: &Connection) -> Result<i64> {
    get_meta(conn, "local_seq")
}

/// 最近一次成功同步时的 `local_seq`
pub fn synced_seq(conn: &Connection) -> Result<i64> {
    get_meta(conn, "synced_seq")
}

pub fn set_synced_seq(conn: &Connection, value: i64) -> Result<()> {
    conn.execute(
        "INSERT INTO sync_meta (key, value) VALUES ('synced_seq', ?1)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        [value],
    )?;
    Ok(())
}
