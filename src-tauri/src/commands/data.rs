use crate::db::{
    AppSettings, Database, ExportData, Todo, WindowPosition, WindowSize,
    subtask_from_row, todo_from_row, SUBTASK_COLUMNS, TODO_COLUMNS,
};
use chrono::Local;
use tauri::State;

/// 从 settings 表读取字符串值的辅助函数
fn get_setting_string(conn: &rusqlite::Connection, key: &str, default: &str) -> String {
    conn.query_row("SELECT value FROM settings WHERE key = ?1", [key], |row| {
        row.get(0)
    })
    .unwrap_or_else(|_| default.to_string())
}

/// 从 settings 表读取布尔值的辅助函数
fn get_setting_bool(conn: &rusqlite::Connection, key: &str, default: bool) -> bool {
    conn.query_row("SELECT value FROM settings WHERE key = ?1", [key], |row| {
        let val: String = row.get(0)?;
        Ok(val == "true")
    })
    .unwrap_or(default)
}

pub fn export_data_internal(db: &Database) -> Result<String, String> {
    let result = db.with_connection(|conn| {
        let todo_sql = format!("SELECT {} FROM todos ORDER BY sort_order ASC", TODO_COLUMNS);
        let mut stmt = conn.prepare(&todo_sql)?;
        let todo_iter = stmt.query_map([], |row| todo_from_row(row))?;

        let mut todos: Vec<Todo> = todo_iter.filter_map(|t| t.ok()).collect();

        for todo in &mut todos {
            let subtask_sql = format!(
                "SELECT {} FROM subtasks WHERE parent_id = ? ORDER BY sort_order ASC",
                SUBTASK_COLUMNS
            );
            let mut subtask_stmt = conn.prepare(&subtask_sql)?;
            let subtask_iter = subtask_stmt.query_map([todo.id], |row| subtask_from_row(row))?;

            todo.subtasks = subtask_iter.filter_map(|s| s.ok()).collect();
        }

        let is_fixed = get_setting_bool(conn, "is_fixed", false);
        let window_position: Option<WindowPosition> = conn
            .query_row("SELECT value FROM settings WHERE key = 'window_position'", [], |row| {
                let val: String = row.get(0)?;
                Ok(serde_json::from_str(&val).ok())
            })
            .unwrap_or(None);
        let window_size: Option<WindowSize> = conn
            .query_row("SELECT value FROM settings WHERE key = 'window_size'", [], |row| {
                let val: String = row.get(0)?;
                Ok(serde_json::from_str(&val).ok())
            })
            .unwrap_or(None);
        let text_theme = get_setting_string(conn, "text_theme", "dark");
        let auto_hide_enabled = get_setting_bool(conn, "auto_hide_enabled", true);
        let show_calendar = get_setting_bool(conn, "show_calendar", false);
        let view_mode = get_setting_string(conn, "view_mode", "list");
        let notification_type = get_setting_string(conn, "notification_type", "system");

        let settings = AppSettings {
            is_fixed,
            window_position,
            window_size,
            auto_hide_enabled,
            text_theme,
            show_calendar,
            view_mode,
            notification_type,
        };

        Ok((todos, settings))
    });

    match result {
        Ok((todos, settings)) => {
            let export_data = ExportData {
                version: "2.0".to_string(),
                exported_at: Local::now().format("%Y-%m-%dT%H:%M:%S%:z").to_string(),
                todos,
                settings,
            };
            serde_json::to_string_pretty(&export_data).map_err(|e| e.to_string())
        }
        Err(e) => Err(e.to_string()),
    }
}

pub fn import_data_raw(db: &Database, json_data: &str) -> Result<(), String> {
    let import: ExportData =
        serde_json::from_str(json_data).map_err(|e| format!("Invalid JSON format: {}", e))?;

    db.with_connection(|conn| {
        conn.execute("DELETE FROM subtasks", [])?;
        conn.execute("DELETE FROM todos", [])?;

        for todo in &import.todos {
            conn.execute(
                "INSERT INTO todos (title, description, color, quadrant, notify_at, notify_before, 
                                   notified, completed, sort_order, start_time, end_time, created_at, updated_at) 
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
                (
                    &todo.title, &todo.description, &todo.color, todo.quadrant,
                    &todo.notify_at, todo.notify_before,
                    if todo.notified { 1 } else { 0 },
                    if todo.completed { 1 } else { 0 },
                    todo.sort_order, &todo.start_time, &todo.end_time,
                    &todo.created_at, &todo.updated_at,
                ),
            )?;

            let new_todo_id = conn.last_insert_rowid();

            for subtask in &todo.subtasks {
                conn.execute(
                    "INSERT INTO subtasks (parent_id, title, content, completed, sort_order, created_at, updated_at) 
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    (
                        new_todo_id, &subtask.title, &subtask.content,
                        if subtask.completed { 1 } else { 0 },
                        subtask.sort_order, &subtask.created_at, &subtask.updated_at,
                    ),
                )?;
            }
        }

        conn.execute(
            "INSERT OR REPLACE INTO settings (key, value, updated_at) VALUES ('is_fixed', ?, datetime('now', 'localtime'))",
            [if import.settings.is_fixed { "true" } else { "false" }],
        )?;
        if let Some(pos) = &import.settings.window_position {
            let pos_json = serde_json::to_string(pos).unwrap_or_default();
            conn.execute(
                "INSERT OR REPLACE INTO settings (key, value, updated_at) VALUES ('window_position', ?, datetime('now', 'localtime'))",
                [&pos_json],
            )?;
        }
        if let Some(size) = &import.settings.window_size {
            let size_json = serde_json::to_string(size).unwrap_or_default();
            conn.execute(
                "INSERT OR REPLACE INTO settings (key, value, updated_at) VALUES ('window_size', ?, datetime('now', 'localtime'))",
                [&size_json],
            )?;
        }
        conn.execute(
            "INSERT OR REPLACE INTO settings (key, value, updated_at) VALUES ('auto_hide_enabled', ?, datetime('now', 'localtime'))",
            [if import.settings.auto_hide_enabled { "true" } else { "false" }],
        )?;
        conn.execute(
            "INSERT OR REPLACE INTO settings (key, value, updated_at) VALUES ('text_theme', ?, datetime('now', 'localtime'))",
            [&import.settings.text_theme],
        )?;
        conn.execute(
            "INSERT OR REPLACE INTO settings (key, value, updated_at) VALUES ('show_calendar', ?, datetime('now', 'localtime'))",
            [if import.settings.show_calendar { "true" } else { "false" }],
        )?;
        conn.execute(
            "INSERT OR REPLACE INTO settings (key, value, updated_at) VALUES ('view_mode', ?, datetime('now', 'localtime'))",
            [&import.settings.view_mode],
        )?;
        conn.execute(
            "INSERT OR REPLACE INTO settings (key, value, updated_at) VALUES ('notification_type', ?, datetime('now', 'localtime'))",
            [&import.settings.notification_type],
        )?;

        Ok(())
    })
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn export_data(db: State<Database>) -> Result<String, String> {
    let result = db.with_connection(|conn| {
        let todo_sql = format!("SELECT {} FROM todos ORDER BY sort_order ASC", TODO_COLUMNS);
        let mut stmt = conn.prepare(&todo_sql)?;
        let todo_iter = stmt.query_map([], |row| todo_from_row(row))?;

        let mut todos: Vec<Todo> = todo_iter.filter_map(|t| t.ok()).collect();

        for todo in &mut todos {
            let subtask_sql = format!(
                "SELECT {} FROM subtasks WHERE parent_id = ? ORDER BY sort_order ASC",
                SUBTASK_COLUMNS
            );
            let mut subtask_stmt = conn.prepare(&subtask_sql)?;
            let subtask_iter = subtask_stmt.query_map([todo.id], |row| subtask_from_row(row))?;
            todo.subtasks = subtask_iter.filter_map(|s| s.ok()).collect();
        }

        // 获取设置
        let is_fixed = get_setting_bool(conn, "is_fixed", false);

        let window_position: Option<WindowPosition> = conn
            .query_row(
                "SELECT value FROM settings WHERE key = 'window_position'",
                [],
                |row| {
                    let val: String = row.get(0)?;
                    Ok(serde_json::from_str(&val).ok())
                },
            )
            .unwrap_or(None);

        let window_size: Option<WindowSize> = conn
            .query_row(
                "SELECT value FROM settings WHERE key = 'window_size'",
                [],
                |row| {
                    let val: String = row.get(0)?;
                    Ok(serde_json::from_str(&val).ok())
                },
            )
            .unwrap_or(None);

        let text_theme = get_setting_string(conn, "text_theme", "dark");
        let auto_hide_enabled = get_setting_bool(conn, "auto_hide_enabled", true);
        let show_calendar = get_setting_bool(conn, "show_calendar", false);
        let view_mode = get_setting_string(conn, "view_mode", "list");
        let notification_type = get_setting_string(conn, "notification_type", "system");

        let settings = AppSettings {
            is_fixed,
            window_position,
            window_size,
            auto_hide_enabled,
            text_theme,
            show_calendar,
            view_mode,
            notification_type,
        };

        Ok((todos, settings))
    });

    match result {
        Ok((todos, settings)) => {
            let export_data = ExportData {
                version: "2.0".to_string(),
                exported_at: Local::now().format("%Y-%m-%dT%H:%M:%S%:z").to_string(),
                todos,
                settings,
            };
            serde_json::to_string_pretty(&export_data).map_err(|e| e.to_string())
        }
        Err(e) => Err(e.to_string()),
    }
}

#[tauri::command]
pub fn import_data(db: State<Database>, json_data: String) -> Result<(), String> {
    let import_data: ExportData =
        serde_json::from_str(&json_data).map_err(|e| format!("Invalid JSON format: {}", e))?;

    db.with_connection(|conn| {
        // 清空现有数据
        conn.execute("DELETE FROM subtasks", [])?;
        conn.execute("DELETE FROM todos", [])?;

        // 导入待办
        for todo in &import_data.todos {
            conn.execute(
                "INSERT INTO todos (title, description, color, quadrant, notify_at, notify_before, 
                                   notified, completed, sort_order, start_time, end_time, created_at, updated_at, post_action) 
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
                (
                    &todo.title,
                    &todo.description,
                    &todo.color,
                    todo.quadrant,
                    &todo.notify_at,
                    todo.notify_before,
                    if todo.notified { 1 } else { 0 },
                    if todo.completed { 1 } else { 0 },
                    todo.sort_order,
                    &todo.start_time,
                    &todo.end_time,
                    &todo.created_at,
                    &todo.updated_at,
                    &todo.post_action,
                ),
            )?;

            let new_todo_id = conn.last_insert_rowid();

            // 导入子任务
            for subtask in &todo.subtasks {
                conn.execute(
                    "INSERT INTO subtasks (parent_id, title, content, completed, sort_order, created_at, updated_at) 
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    (
                        new_todo_id,
                        &subtask.title,
                        &subtask.content,
                        if subtask.completed { 1 } else { 0 },
                        subtask.sort_order,
                        &subtask.created_at,
                        &subtask.updated_at,
                    ),
                )?;
            }
        }

        // 导入设置
        conn.execute(
            "INSERT OR REPLACE INTO settings (key, value, updated_at) VALUES ('is_fixed', ?, datetime('now', 'localtime'))",
            [if import_data.settings.is_fixed { "true" } else { "false" }],
        )?;

        if let Some(pos) = &import_data.settings.window_position {
            let pos_json = serde_json::to_string(pos).unwrap_or_default();
            conn.execute(
                "INSERT OR REPLACE INTO settings (key, value, updated_at) VALUES ('window_position', ?, datetime('now', 'localtime'))",
                [&pos_json],
            )?;
        }

        if let Some(size) = &import_data.settings.window_size {
            let size_json = serde_json::to_string(size).unwrap_or_default();
            conn.execute(
                "INSERT OR REPLACE INTO settings (key, value, updated_at) VALUES ('window_size', ?, datetime('now', 'localtime'))",
                [&size_json],
            )?;
        }

        // 导入贴边自动隐藏设置
        conn.execute(
            "INSERT OR REPLACE INTO settings (key, value, updated_at) VALUES ('auto_hide_enabled', ?, datetime('now', 'localtime'))",
            [if import_data.settings.auto_hide_enabled { "true" } else { "false" }],
        )?;

        // 导入文本主题
        conn.execute(
            "INSERT OR REPLACE INTO settings (key, value, updated_at) VALUES ('text_theme', ?, datetime('now', 'localtime'))",
            [&import_data.settings.text_theme],
        )?;

        // 导入日历显示设置
        conn.execute(
            "INSERT OR REPLACE INTO settings (key, value, updated_at) VALUES ('show_calendar', ?, datetime('now', 'localtime'))",
            [if import_data.settings.show_calendar { "true" } else { "false" }],
        )?;

        // 导入视图模式
        conn.execute(
            "INSERT OR REPLACE INTO settings (key, value, updated_at) VALUES ('view_mode', ?, datetime('now', 'localtime'))",
            [&import_data.settings.view_mode],
        )?;

        // 导入通知类型
        conn.execute(
            "INSERT OR REPLACE INTO settings (key, value, updated_at) VALUES ('notification_type', ?, datetime('now', 'localtime'))",
            [&import_data.settings.notification_type],
        )?;

        Ok(())
    })
    .map_err(|e| e.to_string())
}
