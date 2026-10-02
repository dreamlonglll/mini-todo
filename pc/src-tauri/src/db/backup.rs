//! 数据库快照备份（C5）。
//!
//! 用 `VACUUM INTO` 把当前库完整拷贝到 `<数据库所在目录>/backups/data-<reason>-<时间>.db`，
//! 只保留最新 [`KEEP_BACKUPS`] 份。手动导入前（R1）与执行待定迁移前（R2）共用。
//!
//! `VACUUM INTO` 不能在事务内执行，调用方需在开启事务之前调用。

use rusqlite::Connection;
use std::path::{Path, PathBuf};

/// 备份目录里保留的快照数量
pub const KEEP_BACKUPS: usize = 5;

/// 备份目录名（相对数据库文件所在目录）
pub const BACKUP_DIR_NAME: &str = "backups";

/// 为 `conn` 对应的数据库文件做一次快照，返回快照路径。
///
/// 内存库 / 临时库没有文件路径，直接返回 `Ok(None)`（测试走这条分支，不会碰真实数据目录）。
pub fn snapshot(conn: &Connection, reason: &str) -> Result<Option<PathBuf>, String> {
    let Some(db_file) = conn.path().filter(|p| !p.is_empty()) else {
        return Ok(None);
    };
    let dir = Path::new(db_file)
        .parent()
        .map(|p| p.join(BACKUP_DIR_NAME))
        .ok_or_else(|| "无法确定备份目录".to_string())?;
    snapshot_to(conn, &dir, reason).map(Some)
}

/// 把快照写到指定目录，完成后清理旧快照。
pub fn snapshot_to(conn: &Connection, dir: &Path, reason: &str) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("创建备份目录失败: {}", e))?;

    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S-%3f");
    let file = dir.join(format!("data-{}-{}.db", sanitize_reason(reason), stamp));
    let target = file
        .to_str()
        .ok_or_else(|| "备份路径包含非法字符".to_string())?
        .to_string();

    conn.execute("VACUUM INTO ?1", [&target])
        .map_err(|e| format!("备份数据库失败: {}", e))?;

    if let Err(e) = prune(dir, KEEP_BACKUPS) {
        // 清理失败不影响本次备份结果
        log::warn!("[backup] 清理旧备份失败: {}", e);
    }
    Ok(file)
}

/// 只保留 `dir` 下最新的 `keep` 个 `data-*.db`（按修改时间，其次按文件名）。
fn prune(dir: &Path, keep: usize) -> std::io::Result<()> {
    let mut files: Vec<(std::time::SystemTime, PathBuf)> = std::fs::read_dir(dir)?
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let name = path.file_name()?.to_str()?;
            if !(name.starts_with("data-") && name.ends_with(".db")) {
                return None;
            }
            let modified = entry.metadata().ok()?.modified().ok()?;
            Some((modified, path))
        })
        .collect();
    if files.len() <= keep {
        return Ok(());
    }
    files.sort();
    let excess = files.len() - keep;
    for (_, path) in files.into_iter().take(excess) {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

/// 备份文件名里只保留 `[A-Za-z0-9_-]`
fn sanitize_reason(reason: &str) -> String {
    let cleaned: String = reason
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if cleaned.is_empty() {
        "snapshot".to_string()
    } else {
        cleaned
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "mini-todo-backup-test-{}-{}-{}",
            tag,
            std::process::id(),
            nanos
        ))
    }

    #[test]
    fn in_memory_database_is_skipped() {
        let conn = Connection::open_in_memory().unwrap();
        assert_eq!(snapshot(&conn, "import").unwrap(), None);
    }

    #[test]
    fn snapshot_copies_data_and_keeps_newest_five() {
        let dir = temp_dir("keep");
        std::fs::create_dir_all(&dir).unwrap();
        let db_file = dir.join("data.db");
        let conn = Connection::open(&db_file).unwrap();
        conn.execute_batch("CREATE TABLE t (v TEXT); INSERT INTO t VALUES ('hello');")
            .unwrap();

        let mut last = None;
        for i in 0..7 {
            last = snapshot(&conn, &format!("v{} test", i)).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let last = last.expect("文件库应当产生快照");
        assert!(last.starts_with(dir.join(BACKUP_DIR_NAME)));
        assert!(last
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("data-v6_test-"));

        let count = std::fs::read_dir(dir.join(BACKUP_DIR_NAME))
            .unwrap()
            .count();
        assert_eq!(count, KEEP_BACKUPS);

        let copy = Connection::open(&last).unwrap();
        let v: String = copy.query_row("SELECT v FROM t", [], |r| r.get(0)).unwrap();
        assert_eq!(v, "hello");

        drop(copy);
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
