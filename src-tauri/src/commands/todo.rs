use base64::{engine::general_purpose, Engine};
use crate::db::{
    CreateSubTaskRequest, CreateTodoRequest, Database, SubTask, Todo, UpdateSubTaskRequest,
    UpdateTodoRequest, subtask_from_row, todo_from_row, SUBTASK_COLUMNS, TODO_COLUMNS,
};
use std::path::PathBuf;
use tauri::State;

#[tauri::command]
pub fn get_todos(db: State<Database>) -> Result<Vec<Todo>, String> {
    db.with_connection(|conn| {
        // 获取所有待办
        let sql = format!(
            "SELECT {} FROM todos ORDER BY completed ASC, sort_order ASC, created_at DESC",
            TODO_COLUMNS
        );
        let mut stmt = conn.prepare(&sql)?;

        let todo_iter = stmt.query_map([], |row| todo_from_row(row))?;

        let mut todos: Vec<Todo> = todo_iter.filter_map(|t| t.ok()).collect();

        // 获取每个待办的子任务
        for todo in &mut todos {
            let subtask_sql = format!(
                "SELECT {} FROM subtasks WHERE parent_id = ? ORDER BY sort_order ASC",
                SUBTASK_COLUMNS
            );
            let mut subtask_stmt = conn.prepare(&subtask_sql)?;

            let subtask_iter = subtask_stmt.query_map([todo.id], |row| subtask_from_row(row))?;

            todo.subtasks = subtask_iter.filter_map(|s| s.ok()).collect();
        }

        Ok(todos)
    })
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn create_todo(db: State<Database>, data: CreateTodoRequest) -> Result<Todo, String> {
    db.with_connection(|conn| {
        // 获取最大排序值
        let max_order: i32 = conn
            .query_row(
                "SELECT COALESCE(MAX(sort_order), -1) FROM todos WHERE completed = 0",
                [],
                |row| row.get(0),
            )
            .unwrap_or(-1);

        conn.execute(
            "INSERT INTO todos (title, description, color, quadrant, notify_at, notify_before, start_time, end_time, sort_order, agent_id, agent_project_path) 
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            (
                &data.title,
                &data.description,
                &data.color,
                data.quadrant,
                &data.notify_at,
                data.notify_before.unwrap_or(0),
                &data.start_time,
                &data.end_time,
                max_order + 1,
                &data.agent_id,
                &data.agent_project_path,
            ),
        )?;

        let id = conn.last_insert_rowid();

        let sql = format!("SELECT {} FROM todos WHERE id = ?", TODO_COLUMNS);
        conn.query_row(&sql, [id], |row| todo_from_row(row))
    })
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn update_todo(db: State<Database>, id: i64, data: UpdateTodoRequest) -> Result<Todo, String> {
    db.with_connection(|conn| {
        let mut updates = Vec::new();
        let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

        if let Some(ref title) = data.title {
            updates.push("title = ?");
            params.push(Box::new(title.clone()));
        }
        if let Some(ref desc) = data.description {
            updates.push("description = ?");
            params.push(Box::new(desc.clone()));
        }
        if let Some(ref color) = data.color {
            updates.push("color = ?");
            params.push(Box::new(color.clone()));
        }
        if let Some(quadrant) = data.quadrant {
            updates.push("quadrant = ?");
            params.push(Box::new(quadrant));
        }
        // 明确清除通知时间
        if data.clear_notify_at {
            updates.push("notify_at = NULL");
            updates.push("notified = 0");
        } else if let Some(ref notify_at) = data.notify_at {
            updates.push("notify_at = ?");
            params.push(Box::new(notify_at.clone()));
            // 设置新通知时间时，重置已通知状态
            updates.push("notified = 0");
        }
        if let Some(notify_before) = data.notify_before {
            updates.push("notify_before = ?");
            params.push(Box::new(notify_before));
        }
        if let Some(completed) = data.completed {
            updates.push("completed = ?");
            params.push(Box::new(if completed { 1 } else { 0 }));
        }
        if let Some(sort_order) = data.sort_order {
            updates.push("sort_order = ?");
            params.push(Box::new(sort_order));
        }
        // 开始时间
        if data.clear_start_time {
            updates.push("start_time = NULL");
        } else if let Some(ref start_time) = data.start_time {
            updates.push("start_time = ?");
            params.push(Box::new(start_time.clone()));
        }
        // 截止时间
        if data.clear_end_time {
            updates.push("end_time = NULL");
        } else if let Some(ref end_time) = data.end_time {
            updates.push("end_time = ?");
            params.push(Box::new(end_time.clone()));
        }
        // Agent 绑定
        if data.clear_agent {
            updates.push("agent_id = NULL");
            updates.push("agent_project_path = NULL");
        } else {
            if let Some(agent_id) = data.agent_id {
                updates.push("agent_id = ?");
                params.push(Box::new(agent_id));
            }
            if let Some(ref path) = data.agent_project_path {
                updates.push("agent_project_path = ?");
                params.push(Box::new(path.clone()));
            }
        }
        if let Some(ref post_action) = data.post_action {
            updates.push("post_action = ?");
            params.push(Box::new(post_action.clone()));
        }

        if updates.is_empty() {
            return Err(rusqlite::Error::InvalidParameterName(
                "No fields to update".to_string(),
            ));
        }

        updates.push("updated_at = datetime('now', 'localtime')");

        let sql = format!("UPDATE todos SET {} WHERE id = ?", updates.join(", "));
        params.push(Box::new(id));

        let params_refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|p| p.as_ref()).collect();
        conn.execute(&sql, params_refs.as_slice())?;

        let todo_sql = format!("SELECT {} FROM todos WHERE id = ?", TODO_COLUMNS);
        let mut todo = conn.query_row(&todo_sql, [id], |row| todo_from_row(row))?;

        let subtask_sql = format!(
            "SELECT {} FROM subtasks WHERE parent_id = ? ORDER BY sort_order ASC",
            SUBTASK_COLUMNS
        );
        let mut subtask_stmt = conn.prepare(&subtask_sql)?;
        let subtask_iter = subtask_stmt.query_map([id], |row| subtask_from_row(row))?;
        todo.subtasks = subtask_iter.filter_map(|s| s.ok()).collect();

        Ok(todo)
    })
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn delete_todo(db: State<Database>, id: i64) -> Result<(), String> {
    db.with_connection(|conn| {
        conn.execute("DELETE FROM todos WHERE id = ?", [id])?;
        Ok(())
    })
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn reorder_todos(db: State<Database>, ids: Vec<i64>) -> Result<(), String> {
    db.with_connection(|conn| {
        for (index, id) in ids.iter().enumerate() {
            conn.execute(
                "UPDATE todos SET sort_order = ?, updated_at = datetime('now', 'localtime') WHERE id = ?",
                (index as i32, id),
            )?;
        }
        Ok(())
    })
    .map_err(|e| e.to_string())
}

// 子任务操作
#[tauri::command]
pub fn create_subtask(db: State<Database>, data: CreateSubTaskRequest) -> Result<SubTask, String> {
    db.with_connection(|conn| {
        let max_order: i32 = conn
            .query_row(
                "SELECT COALESCE(MAX(sort_order), -1) FROM subtasks WHERE parent_id = ?",
                [data.parent_id],
                |row| row.get(0),
            )
            .unwrap_or(-1);

        conn.execute(
            "INSERT INTO subtasks (parent_id, title, content, sort_order) VALUES (?1, ?2, ?3, ?4)",
            (data.parent_id, &data.title, &data.content, max_order + 1),
        )?;

        let id = conn.last_insert_rowid();

        let sql = format!("SELECT {} FROM subtasks WHERE id = ?", SUBTASK_COLUMNS);
        conn.query_row(&sql, [id], |row| subtask_from_row(row))
    })
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn update_subtask(
    db: State<Database>,
    id: i64,
    data: UpdateSubTaskRequest,
) -> Result<SubTask, String> {
    db.with_connection(|conn| {
        let mut updates = Vec::new();
        let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

        if let Some(ref title) = data.title {
            updates.push("title = ?");
            params.push(Box::new(title.clone()));
        }
        if let Some(ref content) = data.content {
            updates.push("content = ?");
            params.push(Box::new(content.clone()));
        }
        if let Some(completed) = data.completed {
            updates.push("completed = ?");
            params.push(Box::new(if completed { 1 } else { 0 }));
        }
        if let Some(sort_order) = data.sort_order {
            updates.push("sort_order = ?");
            params.push(Box::new(sort_order));
        }

        if updates.is_empty() {
            return Err(rusqlite::Error::InvalidParameterName(
                "No fields to update".to_string(),
            ));
        }

        updates.push("updated_at = datetime('now', 'localtime')");

        let sql = format!("UPDATE subtasks SET {} WHERE id = ?", updates.join(", "));
        params.push(Box::new(id));

        let params_refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|p| p.as_ref()).collect();
        conn.execute(&sql, params_refs.as_slice())?;

        let sql = format!("SELECT {} FROM subtasks WHERE id = ?", SUBTASK_COLUMNS);
        conn.query_row(&sql, [id], |row| subtask_from_row(row))
    })
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_subtask(db: State<Database>, id: i64) -> Result<SubTask, String> {
    db.with_connection(|conn| {
        let sql = format!("SELECT {} FROM subtasks WHERE id = ?", SUBTASK_COLUMNS);
        conn.query_row(&sql, [id], |row| subtask_from_row(row))
    })
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn delete_subtask(db: State<Database>, id: i64) -> Result<(), String> {
    db.with_connection(|conn| {
        conn.execute("DELETE FROM subtasks WHERE id = ?", [id])?;
        Ok(())
    })
    .map_err(|e| e.to_string())
}

fn get_images_dir_path() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("mini-todo")
        .join("images")
}

#[tauri::command]
pub fn get_images_dir() -> Result<String, String> {
    let dir = get_images_dir_path();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    dir.to_str()
        .map(|s| s.to_string())
        .ok_or_else(|| "Invalid path".to_string())
}

#[tauri::command]
pub fn save_subtask_image(image_data: String, file_name: String) -> Result<String, String> {
    use std::io::Write;

    let dir = get_images_dir_path();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;

    let raw = if image_data.contains(',') {
        image_data.splitn(2, ',').nth(1).unwrap_or("").to_string()
    } else {
        image_data
    };
    let bytes = general_purpose::STANDARD
        .decode(&raw)
        .map_err(|e| e.to_string())?;

    let file_path = dir.join(&file_name);
    let mut file = std::fs::File::create(&file_path).map_err(|e| e.to_string())?;
    file.write_all(&bytes).map_err(|e| e.to_string())?;

    file_path
        .to_str()
        .map(|s| s.to_string())
        .ok_or_else(|| "Invalid path".to_string())
}
