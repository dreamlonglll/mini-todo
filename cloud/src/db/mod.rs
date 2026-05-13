//! SQLite 持久化层。
//!
//! Schema 采用 KV-style：`todos(id, data_json, updated_at)` /
//! `subtasks(id, todo_id, data_json, updated_at)` / `settings(key, value)` /
//! `meta(key, value)`。整条 PC 端 todo / subtask 对象 JSON 原样塞进
//! `data_json`，列表/过滤用 SQLite JSON1 `json_extract` 完成。这样 PC 端
//! 加新字段不影响云端代码。

pub mod repo;
pub mod schema;

use std::path::Path;
use std::sync::{Arc, Mutex};

use rusqlite::Connection;

/// 线程安全的 SQLite 包装。axum handler + 后台 worker 共用。
#[derive(Clone)]
pub struct Db {
    inner: Arc<Mutex<Connection>>,
}

impl Db {
    /// 打开 SQLite 文件（自动建父目录），运行迁移。
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| anyhow::anyhow!("创建数据目录 {} 失败: {}", parent.display(), e))?;
        }
        let conn = Connection::open(path)
            .map_err(|e| anyhow::anyhow!("打开 SQLite {} 失败: {}", path.display(), e))?;
        // 启用 WAL 提高并发；mini-todo 单文件 + 一个 process 的读写也不构成压力
        conn.pragma_update(None, "journal_mode", "WAL").ok();
        conn.pragma_update(None, "foreign_keys", "ON").ok();

        schema::init(&conn)?;

        Ok(Db {
            inner: Arc::new(Mutex::new(conn)),
        })
    }

    /// 在 lock 内同步执行一段逻辑。所有数据库操作都走这里。
    pub fn with_conn<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&mut Connection) -> R,
    {
        let mut guard = self.inner.lock().expect("db mutex poisoned");
        f(&mut guard)
    }
}
