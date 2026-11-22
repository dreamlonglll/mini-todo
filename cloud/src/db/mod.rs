//! SQLite 持久化层。
//!
//! Schema 采用 KV-style：`todos(id, data_json, updated_at)` /
//! `subtasks(id, todo_id, data_json, updated_at)` / `settings(key, value)` /
//! `meta(key, value)`。整条 PC 端 todo / subtask 对象 JSON 原样塞进
//! `data_json`，列表/过滤用 SQLite JSON1 `json_extract` 完成。这样 PC 端
//! 加新字段不影响云端代码。

pub mod normalize;
pub mod repo;
pub mod schema;

use std::path::Path;
use std::sync::{Arc, Mutex};

use rusqlite::{Connection, OpenFlags};

/// 线程安全的 SQLite 包装。axum handler + 后台 worker 共用。
///
/// 文件库在 WAL 模式下额外持有一只只读连接：后台同步持有写连接进行长事务合并时，
/// API 读请求与同步状态头查询可完全并发执行，不被写锁阻塞。
#[derive(Clone)]
pub struct Db {
    inner: Arc<Mutex<Connection>>,
    reader: Option<Arc<Mutex<Connection>>>,
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
        // 启用 WAL + synchronous=NORMAL + busy_timeout 提高并发读写稳定性
        conn.pragma_update(None, "busy_timeout", 5000).ok();
        conn.pragma_update(None, "temp_store", "MEMORY").ok();
        conn.pragma_update(None, "journal_mode", "WAL").ok();
        conn.pragma_update(None, "synchronous", "NORMAL").ok();
        conn.pragma_update(None, "foreign_keys", "ON").ok();

        schema::init(&conn)?;

        // 打开独立只读连接（WAL 下与写连接并发读）；若打开失败（如特殊路径）自动退回共用主连接
        let reader = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .ok()
        .map(|ro| {
            ro.pragma_update(None, "busy_timeout", 5000).ok();
            ro.pragma_update(None, "temp_store", "MEMORY").ok();
            ro.pragma_update(None, "query_only", "ON").ok();
            Arc::new(Mutex::new(ro))
        });

        Ok(Db {
            inner: Arc::new(Mutex::new(conn)),
            reader,
        })
    }

    /// 在写连接 lock 内同步执行一段逻辑。
    ///
    /// 拿锁时忽略中毒标记：持锁闭包 panic 只影响那一次请求，SQLite 连接本身
    /// 仍然可用；若沿用 `expect()`，一次 panic 会让之后所有请求和 sync tick
    /// 永久 panic（服务不退出但全 500，不可恢复）。
    pub fn with_conn<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&mut Connection) -> R,
    {
        let mut guard = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        f(&mut guard)
    }

    /// 在只读连接（若可用，否则退回主连接）内同步执行只读查询。
    pub fn with_read_conn<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&Connection) -> R,
    {
        let target = self.reader.as_ref().unwrap_or(&self.inner);
        let guard = target
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        f(&guard)
    }

    /// 在 Tokio `spawn_blocking` 线程池中执行只读 SQLite 查询，避免在 async worker 线程同步争锁。
    pub async fn run_read<F, R>(&self, f: F) -> Result<R, tokio::task::JoinError>
    where
        F: FnOnce(&Connection) -> R + Send + 'static,
        R: Send + 'static,
    {
        let db = self.clone();
        tokio::task::spawn_blocking(move || db.with_read_conn(f)).await
    }

    /// 在 Tokio `spawn_blocking` 线程池中执行 SQLite 写事务，避免在 async worker 线程同步争锁。
    pub async fn run_write<F, R>(&self, f: F) -> Result<R, tokio::task::JoinError>
    where
        F: FnOnce(&mut Connection) -> R + Send + 'static,
        R: Send + 'static,
    {
        let db = self.clone();
        tokio::task::spawn_blocking(move || db.with_conn(f)).await
    }
}
