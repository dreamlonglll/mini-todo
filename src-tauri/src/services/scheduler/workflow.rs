use std::time::{SystemTime, UNIX_EPOCH};
use tauri::Manager;

use crate::db::{Database, agent_db, agent_execution_db, workflow_db};
use super::engine::update_status_and_notify;
use crate::services::agent::AgentManager;

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

pub async fn start_workflow(
    app: &tauri::AppHandle,
    todo_id: i64,
) -> Result<(), String> {
    let db = app.state::<Database>();

    let (agent_id, _project_path): (Option<i64>, Option<String>) = db
        .with_connection(|conn| {
            conn.query_row(
                "SELECT agent_id, agent_project_path FROM todos WHERE id = ?1",
                [todo_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
        })
        .map_err(|e| format!("获取 Todo 配置失败: {}", e))?;

    if agent_id.is_none() {
        return Err("请先配置 Agent".to_string());
    }

    let step_count = db
        .with_connection(|conn| workflow_db::get_step_count(conn, todo_id))
        .map_err(|e| format!("获取步骤数失败: {}", e))?;

    if step_count == 0 {
        return Err("请先添加工作流步骤".to_string());
    }

    db.with_connection(|conn| {
        workflow_db::reset_all_steps(conn, todo_id)?;
        conn.execute(
            "UPDATE todos SET workflow_enabled = 1, workflow_current_step = 0 WHERE id = ?1",
            [todo_id],
        )?;
        Ok(())
    })
    .map_err(|e| format!("启动工作流失败: {}", e))?;

    execute_current_step(app, todo_id, 0).await
}

pub fn advance_workflow(
    app: &tauri::AppHandle,
    todo_id: i64,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), String>> + Send + '_>> {
    Box::pin(advance_workflow_inner(app, todo_id))
}

async fn advance_workflow_inner(
    app: &tauri::AppHandle,
    todo_id: i64,
) -> Result<(), String> {
    let db = app.state::<Database>();

    let (current_step, workflow_enabled): (i32, bool) = db
        .with_connection(|conn| {
            conn.query_row(
                "SELECT workflow_current_step, workflow_enabled FROM todos WHERE id = ?1",
                [todo_id],
                |row| Ok((row.get(0)?, row.get::<_, i32>(1)? != 0)),
            )
        })
        .map_err(|e| format!("获取工作流状态失败: {}", e))?;

    if !workflow_enabled {
        return Ok(());
    }

    if let Ok(Some(current)) = db.with_connection(|conn| {
        workflow_db::get_step_at_order(conn, todo_id, current_step)
    }) {
        if current.status == "running" {
            let _ = db.with_connection(|conn| {
                workflow_db::update_step_status(conn, current.id, "completed")
            });
        }
    }

    let next_step = current_step + 1;
    let step_count = db
        .with_connection(|conn| workflow_db::get_step_count(conn, todo_id))
        .unwrap_or(0);

    if next_step >= step_count {
        db.with_connection(|conn| {
            conn.execute(
                "UPDATE todos SET workflow_current_step = ?1, workflow_enabled = 0 WHERE id = ?2",
                rusqlite::params![next_step, todo_id],
            )
        })
        .ok();
        return Ok(());
    }

    db.with_connection(|conn| {
        conn.execute(
            "UPDATE todos SET workflow_current_step = ?1 WHERE id = ?2",
            rusqlite::params![next_step, todo_id],
        )
    })
    .ok();

    execute_current_step(app, todo_id, next_step).await
}

pub async fn skip_current_step(
    app: &tauri::AppHandle,
    todo_id: i64,
) -> Result<(), String> {
    let db = app.state::<Database>();

    let current_step: i32 = db
        .with_connection(|conn| {
            conn.query_row(
                "SELECT workflow_current_step FROM todos WHERE id = ?1",
                [todo_id],
                |row| row.get(0),
            )
        })
        .map_err(|e| format!("获取当前步骤失败: {}", e))?;

    if let Ok(Some(step)) = db.with_connection(|conn| {
        workflow_db::get_step_at_order(conn, todo_id, current_step)
    }) {
        let _ = db.with_connection(|conn| {
            workflow_db::update_step_status(conn, step.id, "skipped")
        });
    }

    advance_workflow(app, todo_id).await
}

async fn execute_current_step(
    app: &tauri::AppHandle,
    todo_id: i64,
    step_order: i32,
) -> Result<(), String> {
    let db = app.state::<Database>();

    let step = db
        .with_connection(|conn| workflow_db::get_step_at_order(conn, todo_id, step_order))
        .map_err(|e| format!("获取步骤失败: {}", e))?;

    let step = match step {
        Some(s) => s,
        None => return advance_workflow(app, todo_id).await,
    };

    let _ = db.with_connection(|conn| {
        workflow_db::update_step_status(conn, step.id, "running")
    });

    match step.step_type.as_str() {
        "subtask" => execute_subtask_step(app, todo_id, &step).await,
        "prompt" => {
            let app_clone = app.clone();
            let step_clone = step.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = execute_prompt_step(&app_clone, todo_id, &step_clone).await {
                    eprintln!("[workflow] prompt step failed: {}", e);
                    let db = app_clone.state::<Database>();
                    let _ = db.with_connection(|conn| {
                        workflow_db::update_step_status(conn, step_clone.id, "failed")
                    });
                }
            });
            Ok(())
        }
        _ => {
            let _ = db.with_connection(|conn| {
                workflow_db::update_step_status(conn, step.id, "skipped")
            });
            advance_workflow(app, todo_id).await
        }
    }
}

async fn execute_subtask_step(
    app: &tauri::AppHandle,
    _todo_id: i64,
    step: &crate::db::models::WorkflowStep,
) -> Result<(), String> {
    let db = app.state::<Database>();

    let subtask_id = match step.subtask_id {
        Some(id) => id,
        None => {
            let _ = db.with_connection(|conn| {
                workflow_db::update_step_status(conn, step.id, "skipped")
            });
            return advance_workflow(app, step.todo_id).await;
        }
    };

    let exists: bool = db
        .with_connection(|conn| {
            conn.query_row(
                "SELECT COUNT(*) FROM subtasks WHERE id = ?1",
                [subtask_id],
                |row| row.get::<_, i32>(0),
            )
        })
        .unwrap_or(0)
        > 0;

    if !exists {
        let _ = db.with_connection(|conn| {
            workflow_db::update_step_status(conn, step.id, "skipped")
        });
        return advance_workflow(app, step.todo_id).await;
    }

    update_status_and_notify(app, &db, subtask_id, "pending");

    Ok(())
}

fn resolve_prev_session_id(
    db: &Database,
    todo_id: i64,
    current_step_order: i32,
) -> Option<String> {
    if current_step_order == 0 {
        return None;
    }
    let prev_order = current_step_order - 1;
    let prev_step = db
        .with_connection(|conn| workflow_db::get_step_at_order(conn, todo_id, prev_order))
        .ok()
        .flatten()?;

    match prev_step.step_type.as_str() {
        "subtask" => {
            let subtask_id = prev_step.subtask_id?;
            db.with_connection(|conn| {
                let exec = agent_execution_db::get_latest_by_subtask(conn, subtask_id)?;
                Ok(exec.and_then(|e| e.session_id))
            })
            .ok()
            .flatten()
        }
        "prompt" => {
            let prefix = format!("wf-{}-{}-", todo_id, prev_step.step_order);
            db.with_connection(|conn| {
                agent_execution_db::get_latest_session_id_by_task_prefix(conn, &prefix)
            })
            .ok()
            .flatten()
        }
        _ => None,
    }
}

async fn execute_prompt_step(
    app: &tauri::AppHandle,
    todo_id: i64,
    step: &crate::db::models::WorkflowStep,
) -> Result<(), String> {
    let db = app.state::<Database>();

    let prompt_text = step.prompt_text.clone().unwrap_or_default();
    if prompt_text.is_empty() {
        let _ = db.with_connection(|conn| {
            workflow_db::update_step_status(conn, step.id, "skipped")
        });
        return advance_workflow(app, todo_id).await;
    }

    let (agent_id, project_path) = db
        .with_connection(|conn| {
            conn.query_row(
                "SELECT agent_id, agent_project_path FROM todos WHERE id = ?1",
                [todo_id],
                |row| {
                    Ok((
                        row.get::<_, Option<i64>>(0)?,
                        row.get::<_, Option<String>>(1)?,
                    ))
                },
            )
        })
        .map_err(|e| format!("获取 Todo 配置失败: {}", e))?;

    let agent_id = agent_id.ok_or("父 Todo 未配置 Agent")?;
    let project_path = project_path.unwrap_or_default();

    let config = db
        .with_connection(|conn| agent_db::get_agent_by_id(conn, agent_id))
        .map_err(|e| format!("获取 Agent 配置失败: {}", e))?;

    let task_id = format!("wf-{}-{}-{}", todo_id, step.step_order, now_ms());

    let resume_session = if step.carry_context {
        resolve_prev_session_id(&db, todo_id, step.step_order)
    } else {
        None
    };

    let agent_manager = app.state::<AgentManager>();
    agent_manager
        .start_background_execution(
            config,
            prompt_text,
            project_path,
            task_id.clone(),
            None,
            app.clone(),
            resume_session,
        )
        .await
        .map_err(|e| format!("启动 Agent 失败: {}", e))?;

    let timeout_secs: u64 = 600;
    let deadline = now_ms() + timeout_secs * 1000;
    let step_id = step.id;

    loop {
        tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;

        if now_ms() > deadline {
            let _ = agent_manager.cancel_execution(&task_id).await;
            let _ = db.with_connection(|conn| {
                workflow_db::update_step_status(conn, step_id, "failed")
            });
            return Err(format!("提示词步骤执行超时（{}秒）", timeout_secs));
        }

        let state = agent_manager.get_execution_state(&task_id).await;
        match state {
            Some(s) if s.status == "completed" => {
                let _ = db.with_connection(|conn| {
                    workflow_db::update_step_status(conn, step_id, "completed")
                });
                advance_workflow(app, todo_id).await?;
                return Ok(());
            }
            Some(s) if s.status == "failed" || s.status == "cancelled" => {
                let _ = db.with_connection(|conn| {
                    workflow_db::update_step_status(conn, step_id, "failed")
                });
                return Err("提示词步骤执行失败".to_string());
            }
            None => {
                let _ = db.with_connection(|conn| {
                    workflow_db::update_step_status(conn, step_id, "failed")
                });
                return Err("执行状态丢失".to_string());
            }
            _ => continue,
        }
    }
}
