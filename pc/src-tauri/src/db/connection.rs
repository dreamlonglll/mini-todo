use rusqlite::{Connection, Result, Transaction, TransactionBehavior};
use std::path::PathBuf;
use std::sync::Mutex;

use super::{migrations, paths};

pub struct Database {
    conn: Mutex<Connection>,
}

impl Database {
    pub fn new() -> Result<Self> {
        let db_path = Self::get_db_path();

        // 确保目录存在
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent).ok();
        }

        let conn = Connection::open(&db_path)?;

        // 启用外键约束
        conn.execute("PRAGMA foreign_keys = ON", [])?;

        let db = Self {
            conn: Mutex::new(conn),
        };

        // 运行迁移
        db.run_migrations()?;

        Ok(db)
    }

    fn get_db_path() -> PathBuf {
        paths::db_path()
    }

    fn run_migrations(&self) -> Result<()> {
        let conn = self.lock_conn();
        migrations::run_migrations(&conn)
    }

    pub fn with_connection<F, R>(&self, f: F) -> Result<R>
    where
        F: FnOnce(&Connection) -> Result<R>,
    {
        let conn = self.lock_conn();
        f(&conn)
    }

    /// 在单个 `BEGIN IMMEDIATE` 事务里执行 `f`：返回 `Ok` 时提交，返回 `Err` 或 panic 时
    /// 由 `Transaction` 的 drop 自动回滚。多语句写入一律走这里，不要手写 BEGIN / COMMIT。
    ///
    /// `with_connection` 只给 `&Connection`，用不了要求 `&mut` 的 `conn.transaction()`，
    /// 因此走 `Transaction::new_unchecked`；闭包拿到 `&mut Transaction` 以便按需开 savepoint。
    pub fn with_transaction<F, R, E>(&self, f: F) -> std::result::Result<R, E>
    where
        F: FnOnce(&mut Transaction<'_>) -> std::result::Result<R, E>,
        E: From<rusqlite::Error>,
    {
        let conn = self.lock_conn();
        let mut tx = Transaction::new_unchecked(&conn, TransactionBehavior::Immediate)?;
        let value = f(&mut tx)?;
        tx.commit()?;
        Ok(value)
    }

    /// 拿锁时忽略中毒标记：持锁线程 panic 只影响那一次操作，SQLite 连接本身
    /// 仍然可用；若沿用 `unwrap()`，一次 panic 会让之后所有 DB 调用永久 panic。
    fn lock_conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// 测试专用：内存库 + 完整迁移，不触碰真实用户数据文件。
    #[cfg(test)]
    pub fn new_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        conn.execute("PRAGMA foreign_keys = ON", [])?;
        let db = Self {
            conn: Mutex::new(conn),
        };
        db.run_migrations()?;
        Ok(db)
    }
}
