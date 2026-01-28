use crate::db::Database;
use std::time::Duration;
use tauri::async_runtime;
use tauri::Manager;
use tauri_plugin_notification::NotificationExt;

pub struct NotificationService;

impl NotificationService {
    /// 启动通知调度器，每分钟检查一次待办通知
    pub fn start_scheduler(app_handle: tauri::AppHandle) {
        async_runtime::spawn(async move {
            // 等待应用初始化完成
            tokio::time::sleep(Duration::from_secs(5)).await;
            
            let mut interval = tokio::time::interval(Duration::from_secs(60));
            loop {
                interval.tick().await;
                if let Err(e) = Self::check_and_send_notifications(&app_handle) {
                    eprintln!("通知检查失败: {}", e);
                }
            }
        });
    }

    /// 检查并发送到期的通知
    fn check_and_send_notifications(app_handle: &tauri::AppHandle) -> Result<(), String> {
        let db = app_handle.state::<Database>();
        
        // 获取需要通知的待办
        let todos = Self::get_pending_notifications(&db)?;

        for todo in todos {
            // 发送通知
            Self::send_notification(app_handle, &todo.title, &todo.description)?;
            
            // 标记为已通知
            Self::mark_as_notified(&db, todo.id)?;
        }

        Ok(())
    }

    /// 获取需要发送通知的待办列表
    fn get_pending_notifications(db: &Database) -> Result<Vec<PendingNotification>, String> {
        db.with_connection(|conn| {
            let mut stmt = conn.prepare(
                r#"
                SELECT id, title, description
                FROM todos
                WHERE completed = 0
                  AND notified = 0
                  AND notify_at IS NOT NULL
                  AND datetime(notify_at, '-' || notify_before || ' minutes') <= datetime('now', 'localtime')
                "#
            )?;

            let todos = stmt.query_map([], |row| {
                Ok(PendingNotification {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    description: row.get::<_, Option<String>>(2)?,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();

            Ok(todos)
        })
        .map_err(|e| e.to_string())
    }

    /// 发送系统通知
    fn send_notification(app_handle: &tauri::AppHandle, title: &str, description: &Option<String>) -> Result<(), String> {
        let body = description.as_deref().unwrap_or("待办事项提醒");
        
        app_handle
            .notification()
            .builder()
            .title(title)
            .body(body)
            .show()
            .map_err(|e| e.to_string())?;

        Ok(())
    }

    /// 标记待办为已通知
    fn mark_as_notified(db: &Database, todo_id: i64) -> Result<(), String> {
        db.with_connection(|conn| {
            conn.execute(
                "UPDATE todos SET notified = 1, updated_at = datetime('now', 'localtime') WHERE id = ?",
                [todo_id],
            )?;
            Ok(())
        })
        .map_err(|e| e.to_string())
    }
}

/// 待发送通知的待办
struct PendingNotification {
    id: i64,
    title: String,
    description: Option<String>,
}
