use crate::db::Database;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;
use tauri::async_runtime;
use tauri::{Manager, Emitter, Listener};
use tauri::WebviewUrl;
use tauri::WebviewWindowBuilder;
use tauri_plugin_notification::NotificationExt;

// 通知窗口计数器（用于生成唯一的窗口标签）
static NOTIFICATION_COUNTER: AtomicU32 = AtomicU32::new(0);
// 当前显示的通知窗口数量（用于堆叠计算）
static ACTIVE_NOTIFICATIONS: AtomicU32 = AtomicU32::new(0);

// 通知窗口尺寸
const NOTIFICATION_WIDTH: u32 = 320;
const NOTIFICATION_HEIGHT: u32 = 120;
const NOTIFICATION_MARGIN: u32 = 20;
const NOTIFICATION_SPACING: u32 = 10;

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
        
        // 获取通知类型设置
        let notification_type = Self::get_notification_type(&db);
        
        // 获取需要通知的待办
        let todos = Self::get_pending_notifications(&db)?;

        for todo in todos {
            // 根据设置发送不同类型的通知
            match notification_type.as_str() {
                "app" => {
                    Self::send_app_notification(app_handle, &todo.title, &todo.description)?;
                }
                _ => {
                    Self::send_system_notification(app_handle, &todo.title, &todo.description)?;
                }
            }
            
            // 标记为已通知
            Self::mark_as_notified(&db, todo.id)?;
        }

        Ok(())
    }

    /// 获取通知类型设置
    fn get_notification_type(db: &Database) -> String {
        db.with_connection(|conn| {
            let result: String = conn
                .query_row(
                    "SELECT value FROM settings WHERE key = 'notification_type'",
                    [],
                    |row| row.get(0),
                )
                .unwrap_or_else(|_| "system".to_string());
            Ok(result)
        })
        .unwrap_or_else(|_| "system".to_string())
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
    fn send_system_notification(app_handle: &tauri::AppHandle, title: &str, description: &Option<String>) -> Result<(), String> {
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

    /// 发送软件通知（创建通知窗口）
    fn send_app_notification(app_handle: &tauri::AppHandle, title: &str, description: &Option<String>) -> Result<(), String> {
        // 生成唯一的窗口标签
        let counter = NOTIFICATION_COUNTER.fetch_add(1, Ordering::SeqCst);
        let window_label = format!("notification_{}", counter);
        
        // 获取当前活动通知数量，用于计算堆叠位置
        let active_count = ACTIVE_NOTIFICATIONS.fetch_add(1, Ordering::SeqCst);
        
        // 获取主显示器信息以计算窗口位置
        let (screen_width, screen_height) = Self::get_primary_screen_size(app_handle);
        
        // 计算窗口位置（右下角堆叠）
        // 新通知在上方，旧通知在下方
        let x = screen_width - NOTIFICATION_WIDTH - NOTIFICATION_MARGIN;
        let y = screen_height - NOTIFICATION_HEIGHT - NOTIFICATION_MARGIN 
            - (active_count * (NOTIFICATION_HEIGHT + NOTIFICATION_SPACING));
        
        // URL 编码标题和描述
        let encoded_title = urlencoding::encode(title);
        let encoded_desc = urlencoding::encode(description.as_deref().unwrap_or("待办事项提醒"));
        let encoded_label = urlencoding::encode(&window_label);
        
        // 创建通知窗口
        let url = format!(
            "index.html#/notification?title={}&description={}&label={}",
            encoded_title,
            encoded_desc,
            encoded_label
        );
        
        let window_label_clone = window_label.clone();
        let app_handle_clone = app_handle.clone();
        
        // 在主线程创建窗口
        let _ = WebviewWindowBuilder::new(
            app_handle,
            &window_label,
            WebviewUrl::App(url.into()),
        )
        .title("通知")
        .inner_size(NOTIFICATION_WIDTH as f64, NOTIFICATION_HEIGHT as f64)
        .position(x as f64, y as f64)
        .decorations(false)
        .transparent(true)
        .always_on_top(true)
        .resizable(false)
        .skip_taskbar(true)
        .focused(false)
        .visible(true)
        .build()
        .map_err(|e| e.to_string())?;
        
        // 监听窗口关闭事件，减少活动通知计数
        let _ = app_handle.listen(format!("notification-closed-{}", window_label_clone), move |_| {
            ACTIVE_NOTIFICATIONS.fetch_sub(1, Ordering::SeqCst);
        });
        
        // 监听窗口销毁事件
        if let Some(window) = app_handle_clone.get_webview_window(&window_label_clone) {
            let app_handle_for_destroy = app_handle_clone.clone();
            let label_for_destroy = window_label_clone.clone();
            window.on_window_event(move |event| {
                if let tauri::WindowEvent::Destroyed = event {
                    ACTIVE_NOTIFICATIONS.fetch_sub(1, Ordering::SeqCst);
                    let _ = app_handle_for_destroy.emit(&format!("notification-closed-{}", label_for_destroy), ());
                }
            });
        }
        
        Ok(())
    }

    /// 获取主显示器尺寸
    fn get_primary_screen_size(app_handle: &tauri::AppHandle) -> (u32, u32) {
        // 尝试获取主显示器
        if let Some(monitor) = app_handle.primary_monitor().ok().flatten() {
            return (monitor.size().width, monitor.size().height);
        }
        
        // 回退到默认值
        (1920, 1080)
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
