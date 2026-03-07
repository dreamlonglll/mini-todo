use std::sync::Arc;
use tauri::State;
use crate::db::Database;
use crate::db::{scheduler_db, dependency_db};
use crate::services::scheduler::engine::TaskScheduler;
use crate::services::scheduler::state_machine;

#[tauri::command]
pub fn update_subtask_schedule_status(
    db: State<Database>,
    subtask_id: i64,
    target_status: String,
) -> Result<String, String> {
    db.with_connection(|conn| {
        let current: String = conn
            .query_row(
                "SELECT schedule_status FROM subtasks WHERE id = ?1",
                [subtask_id],
                |row| row.get(0),
            )
            .map_err(|_| rusqlite::Error::QueryReturnedNoRows)?;

        let new_status = state_machine::try_transition(&current, &target_status)
            .map_err(|_| rusqlite::Error::QueryReturnedNoRows)?;

        scheduler_db::update_schedule_status(conn, subtask_id, &new_status)?;

        if &new_status == "pending" || &new_status == "none" {
            scheduler_db::update_schedule_error(conn, subtask_id, None)?;
        }

        Ok(new_status)
    })
    .map_err(|e| format!("更新调度状态失败: {}", e))
}

#[tauri::command]
pub fn update_subtask_priority(
    db: State<Database>,
    subtask_id: i64,
    priority_score: i64,
) -> Result<(), String> {
    db.with_connection(|conn| {
        scheduler_db::update_priority_score(conn, subtask_id, priority_score)
    })
    .map_err(|e| format!("更新优先级失败: {}", e))
}

#[tauri::command]
pub fn update_subtask_timeout(
    db: State<Database>,
    subtask_id: i64,
    timeout_secs: i64,
) -> Result<(), String> {
    db.with_connection(|conn| {
        scheduler_db::update_timeout_secs(conn, subtask_id, timeout_secs)
    })
    .map_err(|e| format!("更新超时时间失败: {}", e))
}

#[tauri::command]
pub fn update_subtask_max_retries(
    db: State<Database>,
    subtask_id: i64,
    max_retries: i64,
) -> Result<(), String> {
    db.with_connection(|conn| {
        scheduler_db::update_max_retries(conn, subtask_id, max_retries)
    })
    .map_err(|e| format!("更新最大重试次数失败: {}", e))
}

#[tauri::command]
pub fn add_task_dependency(
    db: State<Database>,
    subtask_id: i64,
    depends_on_id: i64,
    dependency_type: String,
) -> Result<i64, String> {
    if subtask_id == depends_on_id {
        return Err("不能依赖自身".to_string());
    }

    db.with_connection(|conn| {
        if dependency_db::has_circular_dependency(conn, subtask_id, depends_on_id)? {
            return Err(rusqlite::Error::QueryReturnedNoRows);
        }

        dependency_db::add_dependency(conn, subtask_id, depends_on_id, &dependency_type)
    })
    .map_err(|e| {
        if e.to_string().contains("QueryReturnedNoRows") {
            "添加依赖会形成循环".to_string()
        } else if e.to_string().contains("UNIQUE constraint") {
            "依赖关系已存在".to_string()
        } else {
            format!("添加依赖失败: {}", e)
        }
    })
}

#[tauri::command]
pub fn remove_task_dependency(
    db: State<Database>,
    dependency_id: i64,
) -> Result<(), String> {
    db.with_connection(|conn| {
        dependency_db::remove_dependency(conn, dependency_id)
    })
    .map_err(|e| format!("删除依赖失败: {}", e))
}

#[tauri::command]
pub fn get_task_dependencies(
    db: State<Database>,
    subtask_id: i64,
) -> Result<Vec<crate::db::models::TaskDependency>, String> {
    db.with_connection(|conn| {
        dependency_db::get_dependencies(conn, subtask_id)
    })
    .map_err(|e| format!("获取依赖失败: {}", e))
}

#[tauri::command]
pub fn check_dependencies_met(
    db: State<Database>,
    subtask_id: i64,
) -> Result<bool, String> {
    db.with_connection(|conn| {
        dependency_db::are_dependencies_met(conn, subtask_id)
    })
    .map_err(|e| format!("检查依赖失败: {}", e))
}

#[tauri::command]
pub fn update_todo_schedule_config(
    db: State<Database>,
    todo_id: i64,
    strategy: Option<String>,
    cron_expression: Option<String>,
    enabled: Option<bool>,
) -> Result<(), String> {
    db.with_connection(|conn| {
        if let Some(s) = &strategy {
            scheduler_db::update_todo_schedule_strategy(conn, todo_id, s)?;
        }
        if let Some(cron) = &cron_expression {
            scheduler_db::update_todo_cron(conn, todo_id, Some(cron))?;
        } else if strategy.is_some() {
            scheduler_db::update_todo_cron(conn, todo_id, None)?;
        }
        if let Some(e) = enabled {
            scheduler_db::toggle_todo_schedule(conn, todo_id, e)?;
        }
        Ok(())
    })
    .map_err(|e| format!("更新调度配置失败: {}", e))
}

#[tauri::command]
pub async fn start_scheduler(
    scheduler: State<'_, Arc<TaskScheduler>>,
) -> Result<(), String> {
    scheduler.set_running(true).await;
    Ok(())
}

#[tauri::command]
pub async fn stop_scheduler(
    scheduler: State<'_, Arc<TaskScheduler>>,
) -> Result<(), String> {
    scheduler.set_running(false).await;
    Ok(())
}

#[tauri::command]
pub async fn get_scheduler_status(
    scheduler: State<'_, Arc<TaskScheduler>>,
) -> Result<bool, String> {
    Ok(scheduler.is_running().await)
}

#[tauri::command]
pub async fn submit_task_to_scheduler(
    scheduler: State<'_, Arc<TaskScheduler>>,
    app: tauri::AppHandle,
    subtask_id: i64,
) -> Result<(), String> {
    scheduler.submit_task(&app, subtask_id).await
}
