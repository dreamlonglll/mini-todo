use std::sync::Arc;
use tauri::{Emitter, Manager, State};
use crate::db::Database;
use crate::db::{scheduler_db, dependency_db};
use crate::services::scheduler::cron_manager;
use crate::services::scheduler::engine::{self, TaskScheduler};
use crate::services::scheduler::state_machine;

#[tauri::command]
pub fn update_subtask_schedule_status(
    app: tauri::AppHandle,
    db: State<Database>,
    subtask_id: i64,
    target_status: String,
) -> Result<String, String> {
    let new_status = db.with_connection(|conn| {
        let current: String = conn
            .query_row(
                "SELECT schedule_status FROM subtasks WHERE id = ?1",
                [subtask_id],
                |row| row.get(0),
            )
            .map_err(|_| rusqlite::Error::QueryReturnedNoRows)?;

        let status = state_machine::try_transition(&current, &target_status)
            .map_err(|e| rusqlite::Error::InvalidParameterName(e))?;

        scheduler_db::update_schedule_status(conn, subtask_id, &status)?;

        if &status == "pending" || &status == "none" {
            scheduler_db::update_schedule_error(conn, subtask_id, None)?;
        }

        Ok(status)
    })
    .map_err(|e| format!("更新调度状态失败: {}", e))?;

    let _ = app.emit(
        "schedule:status-changed",
        serde_json::json!({ "subtask_id": subtask_id, "status": new_status }),
    );

    Ok(new_status)
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

#[tauri::command]
pub fn validate_cron_expression(
    expression: String,
) -> Result<String, String> {
    cron_manager::validate_cron(&expression)?;
    Ok(cron_manager::describe_cron(&expression))
}

#[tauri::command]
pub fn get_next_cron_execution(
    expression: String,
) -> Result<String, String> {
    let next = cron_manager::next_execution_time(&expression)?;
    Ok(next.format("%Y-%m-%d %H:%M:%S").to_string())
}

#[tauri::command]
pub fn get_scheduled_todos(
    db: State<Database>,
) -> Result<Vec<serde_json::Value>, String> {
    db.with_connection(|conn| {
        let mut stmt = conn.prepare(
            "SELECT t.id, t.title, t.cron_expression, t.schedule_enabled, t.last_scheduled_run,
                    (SELECT COUNT(*) FROM subtasks s WHERE s.parent_id = t.id AND s.completed = 0) as pending_subtasks
             FROM todos t
             WHERE t.cron_expression IS NOT NULL AND t.cron_expression != ''
             ORDER BY t.schedule_enabled DESC, t.title ASC"
        )?;

        let rows = stmt.query_map([], |row| {
            let id: i64 = row.get(0)?;
            let title: String = row.get(1)?;
            let cron_expression: String = row.get(2)?;
            let schedule_enabled: bool = row.get::<_, i32>(3)? != 0;
            let last_scheduled_run: Option<String> = row.get(4)?;
            let pending_subtasks: i64 = row.get(5)?;

            let description = cron_manager::describe_cron(&cron_expression);
            let next_run = cron_manager::next_execution_time(&cron_expression)
                .map(|t| t.format("%Y-%m-%d %H:%M:%S").to_string())
                .unwrap_or_default();

            Ok(serde_json::json!({
                "id": id,
                "title": title,
                "cronExpression": cron_expression,
                "cronDescription": description,
                "scheduleEnabled": schedule_enabled,
                "lastScheduledRun": last_scheduled_run,
                "nextRun": next_run,
                "pendingSubtasks": pending_subtasks
            }))
        })?;

        rows.collect::<Result<Vec<_>, _>>()
    })
    .map_err(|e| format!("获取定时任务列表失败: {}", e))
}

#[tauri::command]
pub async fn approve_review(
    app: tauri::AppHandle,
    subtask_id: i64,
) -> Result<(), String> {
    let db = app.state::<Database>();

    let current: String = db
        .with_connection(|conn| {
            conn.query_row(
                "SELECT schedule_status FROM subtasks WHERE id = ?1",
                [subtask_id],
                |row| row.get(0),
            )
        })
        .map_err(|e| format!("获取子任务状态失败: {}", e))?;

    state_machine::try_transition(&current, "completed")?;

    db.with_connection(|conn| {
        scheduler_db::update_schedule_status(conn, subtask_id, "completed")
    })
    .map_err(|e| format!("更新状态失败: {}", e))?;

    let _ = app.emit(
        "schedule:status-changed",
        serde_json::json!({ "subtask_id": subtask_id, "status": "completed" }),
    );

    engine::trigger_downstream(&app, subtask_id).await;

    Ok(())
}

#[tauri::command]
pub async fn reject_review(
    app: tauri::AppHandle,
    subtask_id: i64,
    action: String,
) -> Result<(), String> {
    let db = app.state::<Database>();

    let current: String = db
        .with_connection(|conn| {
            conn.query_row(
                "SELECT schedule_status FROM subtasks WHERE id = ?1",
                [subtask_id],
                |row| row.get(0),
            )
        })
        .map_err(|e| format!("获取子任务状态失败: {}", e))?;

    let target = match action.as_str() {
        "retry" => "pending",
        _ => "failed",
    };

    state_machine::try_transition(&current, target)?;

    db.with_connection(|conn| {
        scheduler_db::update_schedule_status(conn, subtask_id, target)?;
        if target == "pending" {
            scheduler_db::update_schedule_error(conn, subtask_id, None)?;
            scheduler_db::reset_retry_count(conn, subtask_id)?;
        }
        Ok(())
    })
    .map_err(|e| format!("更新状态失败: {}", e))?;

    let _ = app.emit(
        "schedule:status-changed",
        serde_json::json!({ "subtask_id": subtask_id, "status": target }),
    );

    Ok(())
}
