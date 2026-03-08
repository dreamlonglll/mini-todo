use tauri::State;
use crate::db::Database;
use crate::db::{workflow_db, models::{WorkflowStep, WorkflowStepInput}};

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
