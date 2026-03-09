use crate::db::{
    AppSettings, Database, ExportData, Todo, WindowPosition, WindowSize,
    AgentConfig, AgentExecution, WorkflowStep, TaskDependency, PromptTemplate,
    subtask_from_row, todo_from_row, SUBTASK_COLUMNS, TODO_COLUMNS,
};
use chrono::Local;
use rusqlite::params;
use std::collections::HashMap;
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

fn query_agent_configs(conn: &rusqlite::Connection) -> rusqlite::Result<Vec<AgentConfig>> {
    let mut stmt = conn.prepare(
        "SELECT id, name, agent_type, cli_path, enabled, created_at, updated_at
         FROM agent_configs ORDER BY id"
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(AgentConfig {
            id: row.get(0)?,
            name: row.get(1)?,
            agent_type: row.get(2)?,
            cli_path: row.get(3)?,
            enabled: row.get(4)?,
            created_at: row.get(5)?,
            updated_at: row.get(6)?,
        })
    })?;
    rows.collect()
}

fn query_all_workflow_steps(conn: &rusqlite::Connection) -> rusqlite::Result<Vec<WorkflowStep>> {
    let mut stmt = conn.prepare(
        "SELECT id, todo_id, step_order, step_type, subtask_id, prompt_text, status, carry_context, created_at
         FROM workflow_steps ORDER BY todo_id, step_order"
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(WorkflowStep {
            id: row.get(0)?,
            todo_id: row.get(1)?,
            step_order: row.get(2)?,
            step_type: row.get(3)?,
            subtask_id: row.get(4)?,
            prompt_text: row.get(5)?,
            status: row.get(6)?,
            carry_context: row.get(7)?,
            created_at: row.get(8)?,
        })
    })?;
    rows.collect()
}

fn query_all_task_dependencies(conn: &rusqlite::Connection) -> rusqlite::Result<Vec<TaskDependency>> {
    let mut stmt = conn.prepare(
        "SELECT id, subtask_id, depends_on_id, dependency_type, created_at
         FROM task_dependencies ORDER BY id"
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(TaskDependency {
            id: row.get(0)?,
            subtask_id: row.get(1)?,
            depends_on_id: row.get(2)?,
            dependency_type: row.get(3)?,
            created_at: row.get(4)?,
        })
    })?;
    rows.collect()
}

fn query_user_prompt_templates(conn: &rusqlite::Connection) -> rusqlite::Result<Vec<PromptTemplate>> {
    let mut stmt = conn.prepare(
        "SELECT id, name, category, description, template_content, variables, recommended_agent, is_builtin, created_at, updated_at
         FROM prompt_templates WHERE is_builtin = 0 ORDER BY name"
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(PromptTemplate {
            id: row.get(0)?,
            name: row.get(1)?,
            category: row.get(2)?,
            description: row.get(3)?,
            template_content: row.get(4)?,
            variables: row.get(5)?,
            recommended_agent: row.get(6)?,
            is_builtin: row.get::<_, i64>(7)? != 0,
            created_at: row.get(8)?,
            updated_at: row.get(9)?,
        })
    })?;
    rows.collect()
}

fn query_all_agent_executions(conn: &rusqlite::Connection) -> rusqlite::Result<Vec<AgentExecution>> {
    let mut stmt = conn.prepare(
        "SELECT id, task_id, subtask_id, agent_id, agent_type, status, logs, result_text, error,
                input_tokens, output_tokens, start_time_ms, duration_ms, created_at, session_id
         FROM agent_executions ORDER BY id"
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(AgentExecution {
            id: row.get(0)?,
            task_id: row.get(1)?,
            subtask_id: row.get(2)?,
            agent_id: row.get(3)?,
            agent_type: row.get(4)?,
            status: row.get(5)?,
            logs: row.get(6)?,
            result_text: row.get(7)?,
            error: row.get(8)?,
            input_tokens: row.get(9)?,
            output_tokens: row.get(10)?,
            start_time_ms: row.get(11)?,
            duration_ms: row.get(12)?,
            created_at: row.get(13)?,
            session_id: row.get(14)?,
        })
    })?;
    rows.collect()
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

pub fn export_data_internal(db: &Database, include_executions: bool) -> Result<String, String> {
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
        let agent_configs = query_agent_configs(conn)?;
        let workflow_steps = query_all_workflow_steps(conn)?;
        let task_dependencies = query_all_task_dependencies(conn)?;
        let prompt_templates = query_user_prompt_templates(conn)?;
        let agent_executions = if include_executions {
            query_all_agent_executions(conn)?
        } else {
            Vec::new()
        };

        Ok((todos, settings, agent_configs, workflow_steps, task_dependencies, prompt_templates, agent_executions))
    });

    match result {
        Ok((todos, settings, agent_configs, workflow_steps, task_dependencies, prompt_templates, agent_executions)) => {
            let export_data = ExportData {
                version: "3.0".to_string(),
                exported_at: Local::now().format("%Y-%m-%dT%H:%M:%S%:z").to_string(),
                todos,
                settings,
                agent_configs,
                workflow_steps,
                task_dependencies,
                prompt_templates,
                agent_executions,
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
        conn.execute("DELETE FROM workflow_steps", [])?;
        conn.execute("DELETE FROM task_dependencies", [])?;
        conn.execute("DELETE FROM subtasks", [])?;
        conn.execute("DELETE FROM todos", [])?;

        // 1. Import agent_configs with ID mapping
        let mut agent_id_map: HashMap<i64, i64> = HashMap::new();
        if !import.agent_configs.is_empty() {
            conn.execute("DELETE FROM agent_configs", [])?;
            for ac in &import.agent_configs {
                conn.execute(
                    "INSERT INTO agent_configs (name, agent_type, cli_path, enabled, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    (&ac.name, &ac.agent_type, &ac.cli_path, ac.enabled, &ac.created_at, &ac.updated_at),
                )?;
                agent_id_map.insert(ac.id, conn.last_insert_rowid());
            }
        }

        // 2. Import todos with all fields, build ID mapping
        let mut todo_id_map: HashMap<i64, i64> = HashMap::new();
        let mut subtask_id_map: HashMap<i64, i64> = HashMap::new();

        for todo in &import.todos {
            let mapped_agent_id: Option<i64> = todo.agent_id.and_then(|old_id| {
                if agent_id_map.is_empty() { Some(old_id) } else { agent_id_map.get(&old_id).copied() }
            });

            let notified_i = if todo.notified { 1i32 } else { 0 };
            let completed_i = if todo.completed { 1i32 } else { 0 };
            let sched_enabled_i = if todo.schedule_enabled { 1i32 } else { 0 };
            let wf_enabled_i = if todo.workflow_enabled { 1i32 } else { 0 };

            conn.execute(
                "INSERT INTO todos (title, description, color, quadrant, notify_at, notify_before,
                                   notified, completed, sort_order, start_time, end_time, created_at, updated_at,
                                   agent_id, agent_project_path, schedule_strategy, cron_expression,
                                   schedule_enabled, last_scheduled_run, post_action,
                                   workflow_enabled, workflow_current_step)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13,
                         ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22)",
                params![
                    todo.title, todo.description, todo.color, todo.quadrant,
                    todo.notify_at, todo.notify_before,
                    notified_i, completed_i,
                    todo.sort_order, todo.start_time, todo.end_time,
                    todo.created_at, todo.updated_at,
                    mapped_agent_id, todo.agent_project_path,
                    todo.schedule_strategy, todo.cron_expression,
                    sched_enabled_i, todo.last_scheduled_run, todo.post_action,
                    wf_enabled_i, todo.workflow_current_step,
                ],
            )?;

            let new_todo_id = conn.last_insert_rowid();
            todo_id_map.insert(todo.id, new_todo_id);

            // 3. Import subtasks with all fields, build ID mapping
            for subtask in &todo.subtasks {
                let sub_completed_i = if subtask.completed { 1i32 } else { 0 };
                conn.execute(
                    "INSERT INTO subtasks (parent_id, title, content, completed, sort_order, created_at, updated_at,
                                          schedule_status, priority_score, max_retries, retry_count, timeout_secs,
                                          scheduled_at, last_scheduled_run, schedule_error)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
                    params![
                        new_todo_id, subtask.title, subtask.content,
                        sub_completed_i,
                        subtask.sort_order, subtask.created_at, subtask.updated_at,
                        subtask.schedule_status, subtask.priority_score,
                        subtask.max_retries, subtask.retry_count, subtask.timeout_secs,
                        subtask.scheduled_at, subtask.last_scheduled_run, subtask.schedule_error,
                    ],
                )?;
                subtask_id_map.insert(subtask.id, conn.last_insert_rowid());
            }
        }

        // 4. Import workflow_steps with ID mapping
        for step in &import.workflow_steps {
            let mapped_todo_id = todo_id_map.get(&step.todo_id).copied();
            let mapped_subtask_id = step.subtask_id.and_then(|old_id| subtask_id_map.get(&old_id).copied());

            if let Some(new_todo_id) = mapped_todo_id {
                conn.execute(
                    "INSERT INTO workflow_steps (todo_id, step_order, step_type, subtask_id, prompt_text, status, carry_context, created_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    (
                        new_todo_id, step.step_order, &step.step_type,
                        mapped_subtask_id, &step.prompt_text, &step.status,
                        step.carry_context, &step.created_at,
                    ),
                )?;
            }
        }

        // 5. Import task_dependencies with ID mapping
        for dep in &import.task_dependencies {
            let mapped_subtask = subtask_id_map.get(&dep.subtask_id).copied();
            let mapped_depends = subtask_id_map.get(&dep.depends_on_id).copied();

            if let (Some(new_sub), Some(new_dep)) = (mapped_subtask, mapped_depends) {
                conn.execute(
                    "INSERT OR IGNORE INTO task_dependencies (subtask_id, depends_on_id, dependency_type, created_at)
                     VALUES (?1, ?2, ?3, ?4)",
                    (new_sub, new_dep, &dep.dependency_type, &dep.created_at),
                )?;
            }
        }

        // 6. Import user prompt_templates
        if !import.prompt_templates.is_empty() {
            conn.execute("DELETE FROM prompt_templates WHERE is_builtin = 0", [])?;
            for tpl in &import.prompt_templates {
                conn.execute(
                    "INSERT OR IGNORE INTO prompt_templates (id, name, category, description, template_content, variables, recommended_agent, is_builtin, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 0, ?8, ?9)",
                    (
                        &tpl.id, &tpl.name, &tpl.category, &tpl.description,
                        &tpl.template_content, &tpl.variables, &tpl.recommended_agent,
                        &tpl.created_at, &tpl.updated_at,
                    ),
                )?;
            }
        }

        // 7. Import agent_executions with ID mapping
        if !import.agent_executions.is_empty() {
            conn.execute("DELETE FROM agent_executions", [])?;
            for exec in &import.agent_executions {
                let mapped_subtask_id = exec.subtask_id.and_then(|old_id| {
                    if subtask_id_map.is_empty() { Some(old_id) } else { subtask_id_map.get(&old_id).copied() }
                });
                let mapped_agent_id = exec.agent_id.and_then(|old_id| {
                    if agent_id_map.is_empty() { Some(old_id) } else { agent_id_map.get(&old_id).copied() }
                });
                conn.execute(
                    "INSERT INTO agent_executions (task_id, subtask_id, agent_id, agent_type, status,
                        logs, result_text, error, input_tokens, output_tokens, start_time_ms,
                        duration_ms, created_at, session_id)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
                    params![
                        exec.task_id, mapped_subtask_id, mapped_agent_id,
                        exec.agent_type, exec.status,
                        exec.logs, exec.result_text, exec.error,
                        exec.input_tokens, exec.output_tokens, exec.start_time_ms,
                        exec.duration_ms, exec.created_at, exec.session_id,
                    ],
                )?;
            }
        }

        // 8. Import settings
        write_app_settings(conn, &import.settings)?;

        Ok(())
    })
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn export_data(db: State<Database>) -> Result<String, String> {
    export_data_internal(&*db, false)
}

#[tauri::command]
pub fn import_data(db: State<Database>, json_data: String) -> Result<(), String> {
    import_data_raw(&*db, &json_data)
}

#[tauri::command]
pub fn export_data_to_file(db: State<Database>, file_path: String, include_executions: bool) -> Result<(), String> {
    let json_data = export_data_internal(&*db, include_executions)?;

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
