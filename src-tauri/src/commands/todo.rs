use tauri::State;
use crate::db::{
    Database, Todo, SubTask, CreateTodoRequest, UpdateTodoRequest,
    CreateSubTaskRequest, UpdateSubTaskRequest,
};

#[tauri::command]
pub fn get_todos(db: State<Database>) -> Result<Vec<Todo>, String> {
    db.with_connection(|conn| {
        // 获取所有待办
        let mut stmt = conn.prepare(
            "SELECT id, title, description, priority, notify_at, notify_before, 
                    notified, completed, sort_order, created_at, updated_at 
             FROM todos 
             ORDER BY completed ASC, sort_order ASC, created_at DESC"
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
                created_at: row.get(9)?,
                updated_at: row.get(10)?,
                subtasks: Vec::new(),
            })
        })?;

        let mut todos: Vec<Todo> = todo_iter.filter_map(|t| t.ok()).collect();

        // 获取每个待办的子任务
        for todo in &mut todos {
            let mut subtask_stmt = conn.prepare(
                "SELECT id, parent_id, title, completed, sort_order, created_at, updated_at 
                 FROM subtasks 
                 WHERE parent_id = ? 
                 ORDER BY sort_order ASC"
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
            "INSERT INTO todos (title, description, priority, notify_at, notify_before, sort_order) 
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            (
                &data.title,
                &data.description,
                &data.priority,
                &data.notify_at,
                data.notify_before.unwrap_or(0),
                max_order + 1,
            ),
        )?;

        let id = conn.last_insert_rowid();

        // 返回新创建的待办
        conn.query_row(
            "SELECT id, title, description, priority, notify_at, notify_before, 
                    notified, completed, sort_order, created_at, updated_at 
             FROM todos WHERE id = ?",
            [id],
            |row| {
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
                    created_at: row.get(9)?,
                    updated_at: row.get(10)?,
                    subtasks: Vec::new(),
                })
            },
        )
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
        if let Some(ref priority) = data.priority {
            updates.push("priority = ?");
            params.push(Box::new(priority.clone()));
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

        if updates.is_empty() {
            return Err(rusqlite::Error::InvalidParameterName("No fields to update".to_string()));
        }

        updates.push("updated_at = datetime('now', 'localtime')");
        
        let sql = format!("UPDATE todos SET {} WHERE id = ?", updates.join(", "));
        params.push(Box::new(id));

        let params_refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|p| p.as_ref()).collect();
        conn.execute(&sql, params_refs.as_slice())?;

        // 返回更新后的待办
        let mut todo = conn.query_row(
            "SELECT id, title, description, priority, notify_at, notify_before, 
                    notified, completed, sort_order, created_at, updated_at 
             FROM todos WHERE id = ?",
            [id],
            |row| {
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
                    created_at: row.get(9)?,
                    updated_at: row.get(10)?,
                    subtasks: Vec::new(),
                })
            },
        )?;

        // 获取子任务
        let mut subtask_stmt = conn.prepare(
            "SELECT id, parent_id, title, completed, sort_order, created_at, updated_at 
             FROM subtasks WHERE parent_id = ? ORDER BY sort_order ASC"
        )?;

        let subtask_iter = subtask_stmt.query_map([id], |row| {
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
            "INSERT INTO subtasks (parent_id, title, sort_order) VALUES (?1, ?2, ?3)",
            (data.parent_id, &data.title, max_order + 1),
        )?;

        let id = conn.last_insert_rowid();

        conn.query_row(
            "SELECT id, parent_id, title, completed, sort_order, created_at, updated_at 
             FROM subtasks WHERE id = ?",
            [id],
            |row| {
                Ok(SubTask {
                    id: row.get(0)?,
                    parent_id: row.get(1)?,
                    title: row.get(2)?,
                    completed: row.get::<_, i32>(3)? != 0,
                    sort_order: row.get(4)?,
                    created_at: row.get(5)?,
                    updated_at: row.get(6)?,
                })
            },
        )
    })
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn update_subtask(db: State<Database>, id: i64, data: UpdateSubTaskRequest) -> Result<SubTask, String> {
    db.with_connection(|conn| {
        let mut updates = Vec::new();
        let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

        if let Some(ref title) = data.title {
            updates.push("title = ?");
            params.push(Box::new(title.clone()));
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
            return Err(rusqlite::Error::InvalidParameterName("No fields to update".to_string()));
        }

        updates.push("updated_at = datetime('now', 'localtime')");
        
        let sql = format!("UPDATE subtasks SET {} WHERE id = ?", updates.join(", "));
        params.push(Box::new(id));

        let params_refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|p| p.as_ref()).collect();
        conn.execute(&sql, params_refs.as_slice())?;

        conn.query_row(
            "SELECT id, parent_id, title, completed, sort_order, created_at, updated_at 
             FROM subtasks WHERE id = ?",
            [id],
            |row| {
                Ok(SubTask {
                    id: row.get(0)?,
                    parent_id: row.get(1)?,
                    title: row.get(2)?,
                    completed: row.get::<_, i32>(3)? != 0,
                    sort_order: row.get(4)?,
                    created_at: row.get(5)?,
                    updated_at: row.get(6)?,
                })
            },
        )
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
