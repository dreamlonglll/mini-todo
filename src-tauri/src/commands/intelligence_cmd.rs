use tauri::Manager;

use crate::db::{Database, dependency_db};
use crate::services::intelligence::{context_builder, task_splitter};

#[tauri::command]
pub async fn build_project_context(
    project_path: String,
) -> Result<context_builder::ProjectContext, String> {
    context_builder::build_context(&project_path).await
}

#[tauri::command]
pub async fn split_task(
    requirement: String,
    project_path: String,
    agent_type: Option<String>,
) -> Result<task_splitter::SplitResult, String> {
    let cli_name = match agent_type.as_deref() {
        Some("codex") => "codex",
        _ => "claude",
    };

    task_splitter::split_task(&project_path, &requirement, cli_name).await
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SplitTaskInput {
    pub title: String,
    pub description: String,
    pub prompt: String,
    pub complexity: String,
    pub recommended_agent: String,
    pub dependencies: Vec<usize>,
}

#[tauri::command]
pub async fn apply_split_result(
    app: tauri::AppHandle,
    todo_id: i64,
    tasks: Vec<SplitTaskInput>,
) -> Result<Vec<i64>, String> {
    let db = app.state::<Database>();

    let mut created_ids: Vec<i64> = Vec::new();

    for task in &tasks {
        let content = if task.prompt.is_empty() {
            task.description.clone()
        } else {
            format!("{}\n\n---\n\n**Agent 指令:**\n{}", task.description, task.prompt)
        };

        let subtask_id = db
            .with_connection(|conn| {
                let max_order: i32 = conn
                    .query_row(
                        "SELECT COALESCE(MAX(sort_order), 0) FROM subtasks WHERE parent_id = ?1",
                        [todo_id],
                        |row| row.get(0),
                    )
                    .unwrap_or(0);

                conn.execute(
                    "INSERT INTO subtasks (parent_id, title, content, completed, sort_order)
                     VALUES (?1, ?2, ?3, 0, ?4)",
                    rusqlite::params![todo_id, task.title, content, max_order + 1],
                )?;
                Ok(conn.last_insert_rowid())
            })
            .map_err(|e| format!("创建子任务失败: {}", e))?;

        created_ids.push(subtask_id);
    }

    // 创建依赖关系
    for (i, task) in tasks.iter().enumerate() {
        let subtask_id = created_ids[i];
        for &dep_idx in &task.dependencies {
            if dep_idx < created_ids.len() {
                let dep_id = created_ids[dep_idx];
                let _ = db.with_connection(|conn| {
                    dependency_db::add_dependency(conn, subtask_id, dep_id, "finish_to_start")
                });
            }
        }
    }

    Ok(created_ids)
}
