use crate::db::{
    AppSettings, Database, ExportData, Todo, WindowPosition, WindowSize,
    subtask_from_row, todo_from_row, SUBTASK_COLUMNS, TODO_COLUMNS,
};
use chrono::Local;
use rusqlite::params;
use std::io::{Read as _, Write as _};
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

fn read_app_settings(conn: &rusqlite::Connection) -> AppSettings {
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

    AppSettings {
        is_fixed,
        window_position,
        window_size,
        auto_hide_enabled,
        text_theme,
        show_calendar,
        view_mode,
        notification_type,
    }
}

fn write_app_settings(conn: &rusqlite::Connection, settings: &AppSettings) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO settings (key, value, updated_at) VALUES ('is_fixed', ?1, datetime('now', 'localtime'))",
        [if settings.is_fixed { "true" } else { "false" }],
    )?;
    if let Some(pos) = &settings.window_position {
        let pos_json = serde_json::to_string(pos).unwrap_or_default();
        conn.execute(
            "INSERT OR REPLACE INTO settings (key, value, updated_at) VALUES ('window_position', ?1, datetime('now', 'localtime'))",
            [&pos_json],
        )?;
    }
    if let Some(size) = &settings.window_size {
        let size_json = serde_json::to_string(size).unwrap_or_default();
        conn.execute(
            "INSERT OR REPLACE INTO settings (key, value, updated_at) VALUES ('window_size', ?1, datetime('now', 'localtime'))",
            [&size_json],
        )?;
    }
    conn.execute(
        "INSERT OR REPLACE INTO settings (key, value, updated_at) VALUES ('auto_hide_enabled', ?1, datetime('now', 'localtime'))",
        [if settings.auto_hide_enabled { "true" } else { "false" }],
    )?;
    conn.execute(
        "INSERT OR REPLACE INTO settings (key, value, updated_at) VALUES ('text_theme', ?1, datetime('now', 'localtime'))",
        [&settings.text_theme],
    )?;
    conn.execute(
        "INSERT OR REPLACE INTO settings (key, value, updated_at) VALUES ('show_calendar', ?1, datetime('now', 'localtime'))",
        [if settings.show_calendar { "true" } else { "false" }],
    )?;
    conn.execute(
        "INSERT OR REPLACE INTO settings (key, value, updated_at) VALUES ('view_mode', ?1, datetime('now', 'localtime'))",
        [&settings.view_mode],
    )?;
    conn.execute(
        "INSERT OR REPLACE INTO settings (key, value, updated_at) VALUES ('notification_type', ?1, datetime('now', 'localtime'))",
        [&settings.notification_type],
    )?;
    Ok(())
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

        let settings = read_app_settings(conn);

        Ok((todos, settings))
    });

    match result {
        Ok((todos, settings)) => {
            let export_data = ExportData {
                version: "4.0".to_string(),
                exported_at: Local::now().format("%Y-%m-%dT%H:%M:%S%:z").to_string(),
                todos,
                settings,
            };
            serde_json::to_string_pretty(&export_data).map_err(|e| e.to_string())
        }
        Err(e) => Err(e.to_string()),
    }
}

/// 导入备份数据。
///
/// 兼容 v3.0 与 v4.0 两个版本：v3.0 备份内的 agent_configs / workflow_steps /
/// task_dependencies / prompt_templates / agent_executions 字段以及 todo / subtask
/// 上的 agent / 调度 / 工作流字段，会被 serde 在反序列化阶段静默忽略
/// （`ExportData` / `Todo` / `SubTask` 在 v2.0 后不再声明这些字段）。
pub fn import_data_raw(db: &Database, json_data: &str) -> Result<(), String> {
    let import: ExportData =
        serde_json::from_str(json_data).map_err(|e| format!("Invalid JSON format: {}", e))?;

    db.with_connection(|conn| {
        conn.execute("DELETE FROM subtasks", [])?;
        conn.execute("DELETE FROM todos", [])?;

        for todo in &import.todos {
            let notified_i = if todo.notified { 1i32 } else { 0 };
            let completed_i = if todo.completed { 1i32 } else { 0 };
            let repeat_enabled_i = if todo.repeat_enabled { 1i32 } else { 0 };

            conn.execute(
                "INSERT INTO todos (title, description, color, quadrant, notify_at, notify_before,
                                    notified, completed, sort_order, start_time, end_time, created_at, updated_at,
                                    repeat_enabled, repeat_type, repeat_interval, repeat_weekdays, repeat_month_day)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13,
                         ?14, ?15, ?16, ?17, ?18)",
                params![
                    todo.title, todo.description, todo.color, todo.quadrant,
                    todo.notify_at, todo.notify_before,
                    notified_i, completed_i,
                    todo.sort_order, todo.start_time, todo.end_time,
                    todo.created_at, todo.updated_at,
                    repeat_enabled_i, todo.repeat_type, todo.repeat_interval,
                    todo.repeat_weekdays, todo.repeat_month_day,
                ],
            )?;

            let new_todo_id = conn.last_insert_rowid();

            for subtask in &todo.subtasks {
                let sub_completed_i = if subtask.completed { 1i32 } else { 0 };
                conn.execute(
                    "INSERT INTO subtasks (parent_id, title, content, completed, sort_order, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![
                        new_todo_id, subtask.title, subtask.content,
                        sub_completed_i,
                        subtask.sort_order, subtask.created_at, subtask.updated_at,
                    ],
                )?;
            }
        }

        write_app_settings(conn, &import.settings)?;

        Ok(())
    })
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn export_data(db: State<Database>) -> Result<String, String> {
    export_data_internal(&*db)
}

#[tauri::command]
pub fn import_data(db: State<Database>, json_data: String) -> Result<(), String> {
    import_data_raw(&*db, &json_data)
}

#[tauri::command]
pub fn export_data_to_file(db: State<Database>, file_path: String) -> Result<(), String> {
    let json_data = export_data_internal(&*db)?;

    let file = std::fs::File::create(&file_path)
        .map_err(|e| format!("创建文件失败: {}", e))?;
    let mut zip = zip::ZipWriter::new(file);

    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);

    zip.start_file("data.json", options)
        .map_err(|e| format!("写入 ZIP 失败: {}", e))?;
    zip.write_all(json_data.as_bytes())
        .map_err(|e| format!("写入数据失败: {}", e))?;

    zip.finish().map_err(|e| format!("完成 ZIP 失败: {}", e))?;
    Ok(())
}

#[tauri::command]
pub fn import_data_from_file(db: State<Database>, file_path: String) -> Result<(), String> {
    let file_bytes = std::fs::read(&file_path)
        .map_err(|e| format!("读取文件失败: {}", e))?;

    // ZIP magic bytes: PK (0x50, 0x4B)
    let is_zip = file_bytes.len() >= 2 && file_bytes[0] == 0x50 && file_bytes[1] == 0x4B;

    if is_zip {
        let cursor = std::io::Cursor::new(&file_bytes);
        let mut archive = zip::ZipArchive::new(cursor)
            .map_err(|e| format!("解析 ZIP 失败: {}", e))?;

        let mut json_data = String::new();
        let mut data_file = archive.by_name("data.json")
            .map_err(|e| format!("ZIP 中未找到 data.json: {}", e))?;
        data_file.read_to_string(&mut json_data)
            .map_err(|e| format!("读取 data.json 失败: {}", e))?;

        import_data_raw(&*db, &json_data)
    } else {
        let json_data = String::from_utf8(file_bytes)
            .map_err(|e| format!("文件编码错误: {}", e))?;
        import_data_raw(&*db, &json_data)
    }
}
