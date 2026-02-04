use crate::db::Database;
use tauri::State;

/// 获取通知类型设置
#[tauri::command]
pub fn get_notification_type(db: State<Database>) -> Result<String, String> {
    db.with_connection(|conn| {
        let result: String = conn
            .query_row(
                "SELECT value FROM settings WHERE key = 'notification_type'",
                [],
                |row| row.get(0),
            )
            .unwrap_or_else(|_| "system".to_string());
        Ok(result)
    })
    .map_err(|e| e.to_string())
}

/// 设置通知类型
#[tauri::command]
pub fn set_notification_type(db: State<Database>, notification_type: String) -> Result<(), String> {
    // 验证通知类型
    let valid_type = match notification_type.as_str() {
        "system" | "app" => notification_type,
        _ => "system".to_string(),
    };

    db.with_connection(|conn| {
        conn.execute(
            "INSERT OR REPLACE INTO settings (key, value, updated_at) VALUES ('notification_type', ?1, datetime('now', 'localtime'))",
            [&valid_type],
        )?;
        Ok(())
    })
    .map_err(|e| e.to_string())
}
