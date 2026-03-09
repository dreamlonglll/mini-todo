use tauri::State;
use crate::db::Database;
use crate::db::{workflow_db, agent_execution_db, models::{WorkflowStep, WorkflowStepInput}};
use crate::services::agent::runner::CachedLog;

#[tauri::command]
pub fn get_workflow_steps(
    db: State<Database>,
    todo_id: i64,
) -> Result<Vec<WorkflowStep>, String> {
    db.with_connection(|conn| workflow_db::get_workflow_steps(conn, todo_id))
        .map_err(|e| format!("获取工作流步骤失败: {}", e))
}

#[tauri::command]
pub fn set_workflow_steps(
    db: State<Database>,
    todo_id: i64,
    steps: Vec<WorkflowStepInput>,
) -> Result<(), String> {
    db.with_connection(|conn| workflow_db::set_workflow_steps(conn, todo_id, &steps))
        .map_err(|e| format!("保存工作流步骤失败: {}", e))
}

#[tauri::command]
pub async fn start_workflow(
    app: tauri::AppHandle,
    todo_id: i64,
) -> Result<(), String> {
    crate::services::scheduler::workflow::start_workflow(&app, todo_id).await
}

#[tauri::command]
pub fn pause_workflow(
    db: State<Database>,
    todo_id: i64,
) -> Result<(), String> {
    db.with_connection(|conn| {
        conn.execute(
            "UPDATE todos SET workflow_enabled = 0 WHERE id = ?1",
            [todo_id],
        )
    })
    .map_err(|e| format!("暂停工作流失败: {}", e))?;
    Ok(())
}

#[tauri::command]
pub fn reset_workflow(
    db: State<Database>,
    todo_id: i64,
) -> Result<(), String> {
    db.with_connection(|conn| {
        workflow_db::reset_all_steps(conn, todo_id)?;
        conn.execute(
            "UPDATE todos SET workflow_current_step = -1, workflow_enabled = 0 WHERE id = ?1",
            [todo_id],
        )?;
        Ok(())
    })
    .map_err(|e| format!("重置工作流失败: {}", e))
}

#[tauri::command]
pub async fn skip_workflow_step(
    app: tauri::AppHandle,
    todo_id: i64,
) -> Result<(), String> {
    crate::services::scheduler::workflow::skip_current_step(&app, todo_id).await
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowExecutionInfo {
    pub task_id: String,
    pub step_order: i32,
    pub agent_type: String,
    pub status: String,
    pub logs: Vec<CachedLog>,
    pub error: Option<String>,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub start_time_ms: i64,
    pub duration_ms: i64,
}

#[tauri::command]
pub fn get_workflow_executions(
    db: State<Database>,
    todo_id: i64,
) -> Result<Vec<WorkflowExecutionInfo>, String> {
    let prefix = format!("wf-{}-", todo_id);
    let records = db
        .with_connection(|conn| agent_execution_db::get_by_task_id_prefix(conn, &prefix))
        .map_err(|e| format!("查询工作流执行记录失败: {}", e))?;

    let results: Vec<WorkflowExecutionInfo> = records
        .into_iter()
        .map(|rec| {
            let step_order = rec
                .task_id
                .strip_prefix(&prefix)
                .and_then(|rest| rest.split('-').next())
                .and_then(|s| s.parse::<i32>().ok())
                .unwrap_or(-1);

            let logs: Vec<CachedLog> = serde_json::from_str(&rec.logs).unwrap_or_default();

            WorkflowExecutionInfo {
                task_id: rec.task_id,
                step_order,
                agent_type: rec.agent_type,
                status: rec.status,
                logs,
                error: rec.error,
                input_tokens: rec.input_tokens,
                output_tokens: rec.output_tokens,
                start_time_ms: rec.start_time_ms,
                duration_ms: rec.duration_ms,
            }
        })
        .collect();

    Ok(results)
}
