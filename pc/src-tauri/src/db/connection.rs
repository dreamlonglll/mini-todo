use rusqlite::{Connection, Result, Transaction, TransactionBehavior};
use std::path::Path;
use std::sync::Mutex;

use super::{backup, migrations, paths};

pub struct Database {
    conn: Mutex<Connection>,
}

impl Database {
    /// 打开应用数据库（`paths::db_path()`），见 [`Database::open`]。
    pub fn new() -> std::result::Result<Self, String> {
        Self::open(&paths::db_path())
    }

    /// 打开指定位置的数据库并升级到最新版本：
    ///
    /// 1. 有待执行的迁移（且不是全新库）时，先 `VACUUM INTO` 备份到同目录的 `backups/`
    ///    （`data-v<旧版本>-<时间>.db`，与导入前备份共用、只保留最新 5 份）；备份失败则不升级、报错
    /// 2. 迁移期间关闭外键，每个迁移提交前做 `foreign_key_check`（见 [`migrations::migrate`]）
    /// 3. 迁移完成后打开外键
    ///
    /// 错误信息面向用户（启动失败时直接展示），不 panic。
    pub fn open(db_path: &Path) -> std::result::Result<Self, String> {
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("创建数据目录失败: {}", e))?;
        }
        let conn = Connection::open(db_path).map_err(|e| format!("打开数据库失败: {}", e))?;
        backup_before_migrations(&conn, db_path)?;
        migrations::migrate(&conn).map_err(|e| format!("数据库升级失败: {}", e))?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
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

    /// 测试专用：内存库 + 完整迁移（与启动路径相同的 [`migrations::migrate`]），不触碰真实用户数据文件。
    #[cfg(test)]
    pub fn new_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        migrations::migrate(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }
}

/// 有待执行的迁移时先给当前库拍快照（C5）。全新库（版本 0）没有可备份的数据，直接跳过。
fn backup_before_migrations(conn: &Connection, db_path: &Path) -> std::result::Result<(), String> {
    let current =
        migrations::current_version(conn).map_err(|e| format!("读取数据库版本失败: {}", e))?;
    if current > migrations::LATEST_VERSION {
        log::warn!(
            "[db] 数据库版本 v{} 比本程序支持的 v{} 新（可能被新版本打开过），按现状继续使用",
            current,
            migrations::LATEST_VERSION
        );
        return Ok(());
    }
    if current == 0 || current == migrations::LATEST_VERSION {
        return Ok(());
    }
    let dir = db_path
        .parent()
        .map(|p| p.join(backup::BACKUP_DIR_NAME))
        .ok_or_else(|| "无法确定备份目录".to_string())?;
    let file = backup::snapshot_to(conn, &dir, &format!("v{}", current))
        .map_err(|e| format!("升级前备份数据库失败，未做任何改动: {}", e))?;
    log::info!(
        "[db] 数据库将从 v{} 升级到 v{}，升级前备份：{}",
        current,
        migrations::LATEST_VERSION,
        file.display()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn temp_dir(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "mini-todo-db-test-{}-{}-{}",
            tag,
            std::process::id(),
            nanos
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn backups(dir: &Path) -> Vec<String> {
        match std::fs::read_dir(dir.join(backup::BACKUP_DIR_NAME)) {
            Ok(entries) => {
                let mut names: Vec<String> = entries
                    .flatten()
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .collect();
                names.sort();
                names
            }
            Err(_) => Vec::new(),
        }
    }

    fn foreign_keys_on(db: &Database) -> bool {
        db.with_connection(|c| c.query_row("PRAGMA foreign_keys", [], |r| r.get::<_, i64>(0)))
            .unwrap()
            == 1
    }

    #[test]
    fn fresh_database_is_migrated_without_backup_and_with_foreign_keys_on() {
        let dir = temp_dir("fresh");
        let db = Database::open(&dir.join("data.db")).unwrap();
        assert!(backups(&dir).is_empty(), "全新库没有可备份的数据");
        assert!(foreign_keys_on(&db));
        assert_eq!(
            db.with_connection(migrations::current_version).unwrap(),
            migrations::LATEST_VERSION
        );
        drop(db);

        // 已是最新版本：再次打开不备份
        let db = Database::open(&dir.join("data.db")).unwrap();
        assert!(backups(&dir).is_empty());
        drop(db);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn pending_migrations_back_up_the_old_database_first() {
        let dir = temp_dir("upgrade");
        let path = dir.join("data.db");
        {
            let conn = Connection::open(&path).unwrap();
            migrations::run_migrations_to(&conn, 27).unwrap();
            conn.execute(
                "INSERT INTO todos (id, title) VALUES (1, '升级前的数据')",
                [],
            )
            .unwrap();
        }

        let db = Database::open(&path).unwrap();
        assert_eq!(
            db.with_connection(migrations::current_version).unwrap(),
            migrations::LATEST_VERSION
        );
        assert!(foreign_keys_on(&db));
        let files = backups(&dir);
        assert_eq!(files.len(), 1, "{files:?}");
        assert!(files[0].starts_with("data-v27-"), "{files:?}");

        let copy = Connection::open(dir.join(backup::BACKUP_DIR_NAME).join(&files[0])).unwrap();
        assert_eq!(migrations::current_version(&copy).unwrap(), 27);
        let title: String = copy
            .query_row("SELECT title FROM todos WHERE id = 1", [], |r| r.get(0))
            .unwrap();
        assert_eq!(title, "升级前的数据");

        drop(copy);
        drop(db);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn open_reports_errors_instead_of_panicking() {
        let dir = temp_dir("broken");
        let path = dir.join("data.db");
        std::fs::write(&path, b"this is not a sqlite database at all, just text").unwrap();
        let err = Database::open(&path).err().expect("损坏的库应报错");
        assert!(!err.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
