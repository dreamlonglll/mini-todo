use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex;

use tauri::{Emitter, Manager};

use crate::db::{Database, agent_db, dependency_db, scheduler_db, workflow_db};
use crate::services::agent::AgentManager;

use super::concurrency::ConcurrencyManager;
use super::cron_manager;
use super::priority_queue::{PriorityQueue, QueuedTask, calculate_priority};
use super::state_machine;

#[derive(serde::Serialize, Clone)]
struct ScheduleStatusChanged {
    subtask_id: i64,
    status: String,
}

pub fn update_status_and_notify(
    app: &tauri::AppHandle,
    db: &Database,
    subtask_id: i64,
    status: &str,
) {
    if let Err(e) = db.with_connection(|conn| scheduler_db::update_schedule_status(conn, subtask_id, status)) {
        eprintln!("[Scheduler] 更新子任务 {} 状态为 {} 失败: {}", subtask_id, status, e);
    }
    if let Err(e) = app.emit(
        "schedule:status-changed",
        ScheduleStatusChanged {
            subtask_id,
            status: status.to_string(),
        },
    ) {
        eprintln!("[Scheduler] 发送状态变更事件失败: {}", e);
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

pub struct TaskScheduler {
    queue: Mutex<PriorityQueue>,
    concurrency: ConcurrencyManager,
    running: Mutex<bool>,
}

impl TaskScheduler {
    pub fn new() -> Self {
        Self {
            queue: Mutex::new(PriorityQueue::new(50)),
            concurrency: ConcurrencyManager::new(3),
            running: Mutex::new(true),
        }
    }

    /// 启动调度器后台循环
    pub fn start(self: Arc<Self>, app: tauri::AppHandle) {
        let scheduler = self.clone();
        let app_clone = app.clone();

        tauri::async_runtime::spawn(async move {
            let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(5));

            loop {
                interval.tick().await;
                scheduler.tick(&app_clone).await;
            }
        });

        let scheduler2 = self.clone();
        tauri::async_runtime::spawn(async move {
            let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(30));

            loop {
                interval.tick().await;
                scheduler2.refresh_priorities(&app).await;
            }
        });
    }

    /// 单次调度 tick
    async fn tick(&self, app: &tauri::AppHandle) {
        let is_running = *self.running.lock().await;
        if !is_running {
            return;
        }

        self.check_cron_tasks(app).await;
        self.promote_pending_to_queued(app).await;
        self.dispatch_queued_tasks(app).await;
    }

    /// 启用/暂停调度器
    pub async fn set_running(&self, running: bool) {
        *self.running.lock().await = running;
    }

    pub async fn is_running(&self) -> bool {
        *self.running.lock().await
    }

    /// 将 pending 状态且依赖已满足的任务提升为 queued
    async fn promote_pending_to_queued(&self, app: &tauri::AppHandle) {
        let db = app.state::<Database>();

        let pending_tasks = match db.with_connection(|conn| {
            scheduler_db::get_pending_subtasks(conn)
        }) {
            Ok(tasks) => tasks,
            Err(_) => return,
        };

        for (subtask_id, parent_id, priority_score) in pending_tasks {
            let deps_met = db
                .with_connection(|conn| {
                    dependency_db::are_dependencies_met(conn, subtask_id)
                })
                .unwrap_or(false);

            if !deps_met {
                continue;
            }

            let quadrant = db
                .with_connection(|conn| {
                    conn.query_row(
                        "SELECT quadrant FROM todos WHERE id = ?1",
                        [parent_id],
                        |row| row.get::<_, String>(0),
                    )
                })
                .unwrap_or_default();

            let priority = calculate_priority(&quadrant, priority_score, 0, None);

            let mut queue = self.queue.lock().await;
            if queue.contains(subtask_id) {
                continue;
            }

            if queue
                .enqueue(QueuedTask {
                    subtask_id,
                    todo_id: parent_id,
                    priority,
                    enqueued_at: now_ms(),
                })
                .is_ok()
            {
                update_status_and_notify(app, &db, subtask_id, "queued");
            }
        }
    }

    /// 从队列取任务并分发执行
    async fn dispatch_queued_tasks(&self, app: &tauri::AppHandle) {
        loop {
            let task = {
                let queue = self.queue.lock().await;
                match queue.peek() {
                    Some(t) => t.clone(),
                    None => break,
                }
            };

            let db = app.state::<Database>();
            let project_path = db
                .with_connection(|conn| {
                    conn.query_row(
                        "SELECT agent_project_path FROM todos WHERE id = ?1",
                        [task.todo_id],
                        |row| row.get::<_, Option<String>>(0),
                    )
                })
                .unwrap_or(None)
                .unwrap_or_default();

            if !self.concurrency.try_acquire(&project_path).await {
                break;
            }

            let task = {
                let mut queue = self.queue.lock().await;
                match queue.dequeue() {
                    Some(t) => t,
                    None => break,
                }
            };

            self.concurrency.mark_running(&project_path).await;

            let app_clone = app.clone();
            let project_path_clone = project_path.clone();
            tauri::async_runtime::spawn(async move {
                execute_task(&app_clone, task, &project_path_clone).await;

                let scheduler = app_clone.state::<Arc<TaskScheduler>>();
                scheduler.concurrency.mark_finished(&project_path_clone).await;
            });
        }
    }

    /// 手动提交子任务到调度队列
    pub async fn submit_task(
        &self,
        app: &tauri::AppHandle,
        subtask_id: i64,
    ) -> Result<(), String> {
        let db = app.state::<Database>();

        let current_status: String = db
            .with_connection(|conn| {
                conn.query_row(
                    "SELECT schedule_status FROM subtasks WHERE id = ?1",
                    [subtask_id],
                    |row| row.get(0),
                )
            })
            .map_err(|e| format!("获取子任务状态失败: {}", e))?;

        state_machine::try_transition(&current_status, "pending")?;

        update_status_and_notify(app, &db, subtask_id, "pending");

        Ok(())
    }

    /// 检查定时任务是否需要触发
    async fn check_cron_tasks(&self, app: &tauri::AppHandle) {
        let db = app.state::<Database>();

        let scheduled_todos: Vec<(i64, String, Option<String>)> = match db.with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, cron_expression, last_scheduled_run
                 FROM todos
                 WHERE schedule_enabled = 1 AND completed = 0
                 AND cron_expression IS NOT NULL AND cron_expression != ''"
            )?;

            let rows = stmt.query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            })?;
            rows.collect()
        }) {
            Ok(todos) => todos,
            Err(_) => return,
        };

        for (todo_id, cron_expr, last_run) in scheduled_todos {
            let should_trigger = cron_manager::should_trigger(
                &cron_expr,
                last_run.as_deref(),
            )
            .unwrap_or(false);

            if !should_trigger {
                continue;
            }

            // Cron 触发时只将已标记为 pending 的子任务推入队列，
            // 不会自动将 none 状态的子任务纳入调度（需用户手动设置为 pending）
            let subtasks: Vec<(i64, String)> = db
                .with_connection(|conn| {
                    let mut stmt = conn.prepare(
                        "SELECT id, schedule_status FROM subtasks
                         WHERE parent_id = ?1 AND completed = 0
                         AND schedule_status = 'pending'"
                    )?;
                    let rows = stmt.query_map([todo_id], |row| {
                        Ok((row.get(0)?, row.get(1)?))
                    })?;
                    rows.collect()
                })
                .unwrap_or_default();

            for (subtask_id, _status) in subtasks {
                let deps_met = db
                    .with_connection(|conn| {
                        dependency_db::are_dependencies_met(conn, subtask_id)
                    })
                    .unwrap_or(false);

                if !deps_met {
                    continue;
                }

                let priority_score = db
                    .with_connection(|conn| {
                        conn.query_row(
                            "SELECT priority_score FROM subtasks WHERE id = ?1",
                            [subtask_id],
                            |row| row.get::<_, i64>(0),
                        )
                    })
                    .unwrap_or(0);

                let quadrant = db
                    .with_connection(|conn| {
                        conn.query_row(
                            "SELECT quadrant FROM todos WHERE id = ?1",
                            [todo_id],
                            |row| row.get::<_, String>(0),
                        )
                    })
                    .unwrap_or_default();

                let priority = calculate_priority(&quadrant, priority_score, 0, None);

                let mut queue = self.queue.lock().await;
                if !queue.contains(subtask_id) {
                    if queue
                        .enqueue(QueuedTask {
                            subtask_id,
                            todo_id,
                            priority,
                            enqueued_at: now_ms(),
                        })
                        .is_ok()
                    {
                        update_status_and_notify(app, &db, subtask_id, "queued");
                    }
                }
            }

            // 更新 last_scheduled_run
            let _ = db.with_connection(|conn| {
                conn.execute(
                    "UPDATE todos SET last_scheduled_run = datetime('now', 'localtime') WHERE id = ?1",
                    [todo_id],
                )
            });
        }
    }

    /// 重新计算所有排队任务的优先级，重建队列
    async fn refresh_priorities(&self, app: &tauri::AppHandle) {
        let db = app.state::<Database>();
        let mut queue = self.queue.lock().await;

        let tasks: Vec<QueuedTask> = queue.get_all().iter().map(|t| (*t).clone()).collect();
        if tasks.is_empty() {
            return;
        }

        let mut updated_tasks = Vec::with_capacity(tasks.len());

        for task in tasks {
            let quadrant = db
                .with_connection(|conn| {
                    conn.query_row(
                        "SELECT quadrant FROM todos WHERE id = ?1",
                        [task.todo_id],
                        |row| row.get::<_, String>(0),
                    )
                })
                .unwrap_or_default();

            let priority_score = db
                .with_connection(|conn| {
                    conn.query_row(
                        "SELECT priority_score FROM subtasks WHERE id = ?1",
                        [task.subtask_id],
                        |row| row.get::<_, i64>(0),
                    )
                })
                .unwrap_or(0);

            let created_minutes = db
                .with_connection(|conn| {
                    conn.query_row(
                        "SELECT CAST((julianday('now') - julianday(created_at)) * 24 * 60 AS INTEGER) FROM subtasks WHERE id = ?1",
                        [task.subtask_id],
                        |row| row.get::<_, i64>(0),
                    )
                })
                .unwrap_or(0);

            let new_priority =
                calculate_priority(&quadrant, priority_score, created_minutes, None);

            updated_tasks.push(QueuedTask {
                priority: new_priority,
                ..task
            });
        }

        // drain 并重建队列
        while queue.dequeue().is_some() {}
        for task in updated_tasks {
            let _ = queue.enqueue(task);
        }
    }

}

/// 执行单个调度任务
async fn execute_task(app: &tauri::AppHandle, task: QueuedTask, _project_path: &str) {
    let db = app.state::<Database>();

    update_status_and_notify(app, &db, task.subtask_id, "running");

    let todo_info = db.with_connection(|conn| {
        conn.query_row(
            "SELECT agent_id, agent_project_path FROM todos WHERE id = ?1",
            [task.todo_id],
            |row| {
                Ok((
                    row.get::<_, Option<i64>>(0)?,
                    row.get::<_, Option<String>>(1)?,
                ))
            },
        )
    });

    let (agent_id, project_path) = match todo_info {
        Ok((aid, pp)) => (aid, pp.unwrap_or_default()),
        Err(e) => {
            mark_failed(app, task.subtask_id, &format!("获取任务配置失败: {}", e)).await;
            return;
        }
    };

    let agent_id = match agent_id {
        Some(id) => id,
        None => {
            mark_failed(app, task.subtask_id, "父 Todo 未配置 Agent").await;
            return;
        }
    };

    let config = match db.with_connection(|conn| agent_db::get_agent_by_id(conn, agent_id)) {
        Ok(c) => c,
        Err(e) => {
            mark_failed(app, task.subtask_id, &format!("获取 Agent 配置失败: {}", e)).await;
            return;
        }
    };

    let (title, content) = db
        .with_connection(|conn| {
            conn.query_row(
                "SELECT title, content FROM subtasks WHERE id = ?1",
                [task.subtask_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                    ))
                },
            )
        })
        .unwrap_or(("".to_string(), None));

    let prompt = if let Some(ref c) = content {
        if c.is_empty() {
            title.clone()
        } else {
            format!("## 任务\n{}\n\n## 详细说明\n{}", title, c)
        }
    } else {
        title.clone()
    };

    let task_id = format!("sched-{}-{}", task.subtask_id, now_ms());

    let resume_session = db
        .with_connection(|conn| {
            let step = workflow_db::find_step_by_subtask(conn, task.subtask_id)?;
            if let Some(ref s) = step {
                if s.carry_context {
                    return Ok(workflow_db::resolve_carry_context_session(conn, s.todo_id, s.step_order));
                }
            }
            Ok(None)
        })
        .unwrap_or(None);

    let agent_manager = app.state::<AgentManager>();
    let result = agent_manager
        .start_background_execution(
            config,
            prompt,
            project_path,
            task_id.clone(),
            Some(task.subtask_id),
            app.clone(),
            resume_session,
        )
        .await;

    if let Err(e) = result {
        handle_task_failure(app, task.subtask_id, &format!("启动 Agent 失败: {}", e)).await;
        return;
    }

    // 等待执行完成（轮询 execution_states）
    let timeout_secs = db
        .with_connection(|conn| {
            conn.query_row(
                "SELECT timeout_secs FROM subtasks WHERE id = ?1",
                [task.subtask_id],
                |row| row.get::<_, i64>(0),
            )
        })
        .unwrap_or(600) as u64;

    let deadline = now_ms() + timeout_secs * 1000;

    loop {
        tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;

        if now_ms() > deadline {
            let _ = agent_manager.cancel_execution(&task_id).await;
            handle_task_failure(
                app,
                task.subtask_id,
                &format!("执行超时（{}秒），已自动终止", timeout_secs),
            )
            .await;
            return;
        }

        let state = agent_manager.get_execution_state(&task_id).await;
        match state {
            Some(s) if s.status == "completed" => {
                update_status_and_notify(app, &db, task.subtask_id, "completed");

                let is_workflow_step = db
                    .with_connection(|conn| {
                        workflow_db::find_step_by_subtask(conn, task.subtask_id)
                    })
                    .unwrap_or(None);

                if let Some(wf_step) = is_workflow_step {
                    let _ = db.with_connection(|conn| {
                        workflow_db::update_step_status(conn, wf_step.id, "completed")
                    });
                    let _ = super::workflow::advance_workflow(app, wf_step.todo_id).await;
                } else {
                    trigger_downstream(app, task.subtask_id).await;
                }
                return;
            }
            Some(s) if s.status == "failed" || s.status == "cancelled" => {
                let error_msg = s.error.unwrap_or_else(|| "执行失败".to_string());
                handle_task_failure(app, task.subtask_id, &error_msg).await;
                return;
            }
            None => {
                handle_task_failure(app, task.subtask_id, "执行状态丢失").await;
                return;
            }
            _ => continue,
        }
    }
}

/// 标记任务为失败（不重试）
async fn mark_failed(app: &tauri::AppHandle, subtask_id: i64, error: &str) {
    let db = app.state::<Database>();
    update_status_and_notify(app, &db, subtask_id, "failed");
    let _ = db.with_connection(|conn| {
        scheduler_db::update_schedule_error(conn, subtask_id, Some(error))
    });
}

/// 处理任务失败（含重试逻辑）
async fn handle_task_failure(app: &tauri::AppHandle, subtask_id: i64, error: &str) {
    let db = app.state::<Database>();

    let retry_info: Option<(i64, i64)> = db
        .with_connection(|conn| {
            conn.query_row(
                "SELECT retry_count, max_retries FROM subtasks WHERE id = ?1",
                [subtask_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
        })
        .ok();

    let should_retry = if let Some((retry_count, max_retries)) = retry_info {
        if retry_count < max_retries {
            let _ = db.with_connection(|conn| {
                scheduler_db::increment_retry_count(conn, subtask_id)?;
                scheduler_db::update_schedule_error(
                    conn,
                    subtask_id,
                    Some(&format!("第{}次重试失败: {}", retry_count + 1, error)),
                )
            });
            update_status_and_notify(app, &db, subtask_id, "pending");
            true
        } else {
            let _ = db.with_connection(|conn| {
                scheduler_db::update_schedule_error(
                    conn,
                    subtask_id,
                    Some(&format!("已达最大重试次数({}): {}", max_retries, error)),
                )
            });
            update_status_and_notify(app, &db, subtask_id, "failed");
            false
        }
    } else {
        update_status_and_notify(app, &db, subtask_id, "failed");
        false
    };

    if should_retry {
        let retry_count = db
            .with_connection(|conn| {
                conn.query_row(
                    "SELECT retry_count FROM subtasks WHERE id = ?1",
                    [subtask_id],
                    |row| row.get::<_, i64>(0),
                )
            })
            .unwrap_or(1);

        let delay_secs = 30 * 2u64.pow(retry_count.min(6) as u32);
        tokio::time::sleep(tokio::time::Duration::from_secs(delay_secs)).await;
    }
}

/// 当任务完成时，触发下游依赖任务
pub async fn trigger_downstream(app: &tauri::AppHandle, completed_subtask_id: i64) {
    let db = app.state::<Database>();

    let downstream = match db.with_connection(|conn| {
        dependency_db::get_dependents(conn, completed_subtask_id)
    }) {
        Ok(ids) => ids,
        Err(_) => return,
    };

    for downstream_id in downstream {
        let status = db
            .with_connection(|conn| {
                conn.query_row(
                    "SELECT schedule_status FROM subtasks WHERE id = ?1",
                    [downstream_id],
                    |row| row.get::<_, String>(0),
                )
            })
            .unwrap_or_default();

        if status != "pending" {
            continue;
        }

        let deps_met = db
            .with_connection(|conn| {
                dependency_db::are_dependencies_met(conn, downstream_id)
            })
            .unwrap_or(false);

        if deps_met {
            update_status_and_notify(app, &db, downstream_id, "queued");
        }
    }
}
