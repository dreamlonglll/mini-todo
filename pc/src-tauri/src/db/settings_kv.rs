//! `settings` 键值表读写助手。
//!
//! 写入只在值真正变化时才更新 `value` 与 `updated_at`：同步把"参与同步的设置项的
//! `max(updated_at)`"当作本地设置版本（K3-5），`INSERT OR REPLACE` 式的无条件写会让每次
//! 窗口移动 / 重复保存都把版本推到"现在"，远端设置就再也应用不进来，本地还会无谓上传。
//!
//! 所有 settings 写入都应走这里（data.rs / sync_cmd.rs / settings_cmd.rs 已改，window.rs 由 R2 改）。
//! 多键写入的事务由调用方负责。

use rusqlite::{Connection, OptionalExtension, Result};

/// 布尔值在 settings 表里的存储形式
pub fn bool_str(value: bool) -> &'static str {
    if value {
        "true"
    } else {
        "false"
    }
}

/// 读取原始值；键不存在返回 `Ok(None)`
pub fn get_setting(conn: &Connection, key: &str) -> Result<Option<String>> {
    conn.query_row("SELECT value FROM settings WHERE key = ?1", [key], |row| {
        row.get(0)
    })
    .optional()
}

/// 读取字符串值，键不存在或读取失败时返回 `default`
pub fn get_setting_or(conn: &Connection, key: &str, default: &str) -> String {
    match get_setting(conn, key) {
        Ok(Some(value)) => value,
        _ => default.to_string(),
    }
}

/// 读取布尔值：存在时 `"true"` 为真、其余为假；键不存在或读取失败时返回 `default`
pub fn get_bool_setting(conn: &Connection, key: &str, default: bool) -> bool {
    match get_setting(conn, key) {
        Ok(Some(value)) => value == "true",
        _ => default,
    }
}

/// 写入值；只有新增或值变化时才写，并把 `updated_at` 设为当前本地时间。
/// 返回是否真的发生了变化。
pub fn set_setting(conn: &Connection, key: &str, value: &str) -> Result<bool> {
    let changed = conn.execute(
        "INSERT INTO settings (key, value, updated_at)
         VALUES (?1, ?2, datetime('now', 'localtime'))
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at
         WHERE settings.value IS NOT excluded.value",
        [key, value],
    )?;
    Ok(changed > 0)
}

/// 同 [`set_setting`]，但 `updated_at` 用给定的时间戳（应用远端设置时用远端的
/// `settingsUpdatedAt`，避免"应用远端"本身把本地版本推到比远端还新）。
pub fn set_setting_at(conn: &Connection, key: &str, value: &str, updated_at: &str) -> Result<bool> {
    let changed = conn.execute(
        "INSERT INTO settings (key, value, updated_at)
         VALUES (?1, ?2, ?3)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at
         WHERE settings.value IS NOT excluded.value",
        [key, value, updated_at],
    )?;
    Ok(changed > 0)
}

/// 写入布尔值，语义同 [`set_setting`]
pub fn set_bool_setting(conn: &Connection, key: &str, value: bool) -> Result<bool> {
    set_setting(conn, key, bool_str(value))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;

    fn updated_at(conn: &Connection, key: &str) -> String {
        conn.query_row(
            "SELECT updated_at FROM settings WHERE key = ?1",
            [key],
            |r| r.get(0),
        )
        .unwrap()
    }

    #[test]
    fn set_setting_only_bumps_updated_at_when_value_changes() {
        let db = Database::new_in_memory().unwrap();
        db.with_connection(|conn| {
            // 新键：插入即变化
            assert!(set_setting(conn, "probe_key", "a")?);
            // 把时间戳拨回过去，方便观察是否被刷新
            conn.execute(
                "UPDATE settings SET updated_at = '2000-01-01 00:00:00' WHERE key = 'probe_key'",
                [],
            )?;

            // 同值：不算变化，时间戳不动
            assert!(!set_setting(conn, "probe_key", "a")?);
            assert_eq!(updated_at(conn, "probe_key"), "2000-01-01 00:00:00");

            // 新值：变化，时间戳刷新到现在
            assert!(set_setting(conn, "probe_key", "b")?);
            assert_eq!(get_setting(conn, "probe_key")?.as_deref(), Some("b"));
            assert!(updated_at(conn, "probe_key").as_str() > "2000-01-01 00:00:00");
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn set_setting_at_uses_given_timestamp_only_on_change() {
        let db = Database::new_in_memory().unwrap();
        db.with_connection(|conn| {
            assert!(set_setting_at(
                conn,
                "probe_key",
                "x",
                "2026-01-01 08:00:00"
            )?);
            assert_eq!(updated_at(conn, "probe_key"), "2026-01-01 08:00:00");
            assert!(!set_setting_at(
                conn,
                "probe_key",
                "x",
                "2026-02-01 08:00:00"
            )?);
            assert_eq!(updated_at(conn, "probe_key"), "2026-01-01 08:00:00");
            assert!(set_setting_at(
                conn,
                "probe_key",
                "y",
                "2026-02-01 08:00:00"
            )?);
            assert_eq!(updated_at(conn, "probe_key"), "2026-02-01 08:00:00");
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn readers_fall_back_to_defaults() {
        let db = Database::new_in_memory().unwrap();
        db.with_connection(|conn| {
            assert_eq!(get_setting(conn, "missing_key")?, None);
            assert_eq!(get_setting_or(conn, "missing_key", "d"), "d");
            assert!(get_bool_setting(conn, "missing_key", true));
            assert!(set_bool_setting(conn, "flag_key", true)?);
            assert!(get_bool_setting(conn, "flag_key", false));
            set_setting(conn, "flag_key", "garbage")?;
            assert!(!get_bool_setting(conn, "flag_key", true));
            Ok(())
        })
        .unwrap();
    }
}
