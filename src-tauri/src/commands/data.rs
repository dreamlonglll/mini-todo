use tauri::State;
use chrono::Local;
use crate::db::{Database, Todo, SubTask, ExportData, AppSettings, WindowPosition, WindowSize};

#[tauri::command]
pub fn export_data(db: State<Database>) -> Result<String, String> {
    let result = db.with_connection(|conn| {
        // 获取所有待办和子任务
        let mut stmt = conn.prepare(
            "SELECT id, title, description, priority, notify_at, notify_before, 
                    notified, completed, sort_order, start_time, end_time, created_at, updated_at 
             FROM todos ORDER BY sort_order ASC"
        )?;

        let todo_iter = stmt.query_map([], |row| {
            Ok(Todo {
                id: row.get(0)?,
                title: row.get(1)?,
                description: row.get(2)?,
                priority: row.get(3)?,
                notify_at: row.get(4)?,
                notify_before: row.get(5)?,
                notified: row.get::<_, i32>(6)? != 0,
                completed: row.get::<_, i32>(7)? != 0,
                sort_order: row.get(8)?,
                start_time: row.get(9)?,
                end_time: row.get(10)?,
                created_at: row.get(11)?,
                updated_at: row.get(12)?,
                subtasks: Vec::new(),
            })
        })?;

        let mut todos: Vec<Todo> = todo_iter.filter_map(|t| t.ok()).collect();

        // 获取每个待办的子任务
        for todo in &mut todos {
            let mut subtask_stmt = conn.prepare(
                "SELECT id, parent_id, title, completed, sort_order, created_at, updated_at 
                 FROM subtasks WHERE parent_id = ? ORDER BY sort_order ASC"
            )?;

            let subtask_iter = subtask_stmt.query_map([todo.id], |row| {
                Ok(SubTask {
                    id: row.get(0)?,
                    parent_id: row.get(1)?,
                    title: row.get(2)?,
                    completed: row.get::<_, i32>(3)? != 0,
                    sort_order: row.get(4)?,
                    created_at: row.get(5)?,
                    updated_at: row.get(6)?,
                })
            })?;

            todo.subtasks = subtask_iter.filter_map(|s| s.ok()).collect();
        }

        // 获取设置
        let is_fixed: bool = conn
            .query_row(
                "SELECT value FROM settings WHERE key = 'is_fixed'",
                [],
                |row| {
                    let val: String = row.get(0)?;
                    Ok(val == "true")
                },
            )
            .unwrap_or(false);

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

        let text_theme: String = conn
            .query_row(
                "SELECT value FROM settings WHERE key = 'text_theme'",
                [],
                |row| row.get(0),
            )
            .unwrap_or_else(|_| "dark".to_string());

        Ok((todos, is_fixed, window_position, window_size, text_theme))
    });

    match result {
        Ok((todos, is_fixed, window_position, window_size, text_theme)) => {
            let export_data = ExportData {
                version: "1.0".to_string(),
                exported_at: Local::now().format("%Y-%m-%dT%H:%M:%S%:z").to_string(),
                todos,
                settings: AppSettings {
                    is_fixed,
                    window_position,
                    window_size,
                    text_theme,
                },
            };
            serde_json::to_string_pretty(&export_data).map_err(|e| e.to_string())
        }
        Err(e) => Err(e.to_string()),
    }
}

#[tauri::command]
pub fn import_data(db: State<Database>, json_data: String) -> Result<(), String> {
    let import_data: ExportData = serde_json::from_str(&json_data)
        .map_err(|e| format!("Invalid JSON format: {}", e))?;

    db.with_connection(|conn| {
        // 清空现有数据
        conn.execute("DELETE FROM subtasks", [])?;
        conn.execute("DELETE FROM todos", [])?;

        // 导入待办
        for todo in &import_data.todos {
            conn.execute(
                "INSERT INTO todos (title, description, priority, notify_at, notify_before, 
                                   notified, completed, sort_order, start_time, end_time, created_at, updated_at) 
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                (
                    &todo.title,
                    &todo.description,
                    &todo.priority,
                    &todo.notify_at,
                    todo.notify_before,
                    if todo.notified { 1 } else { 0 },
                    if todo.completed { 1 } else { 0 },
                    todo.sort_order,
                    &todo.start_time,
                    &todo.end_time,
                    &todo.created_at,
                    &todo.updated_at,
                ),
            )?;

            let new_todo_id = conn.last_insert_rowid();

            // 导入子任务
            for subtask in &todo.subtasks {
                conn.execute(
                    "INSERT INTO subtasks (parent_id, title, completed, sort_order, created_at, updated_at) 
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    (
                        new_todo_id,
                        &subtask.title,
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

        // 导入文本主题
        conn.execute(
            "INSERT OR REPLACE INTO settings (key, value, updated_at) VALUES ('text_theme', ?, datetime('now', 'localtime'))",
            [&import_data.settings.text_theme],
        )?;

        Ok(())
    })
    .map_err(|e| e.to_string())
}
