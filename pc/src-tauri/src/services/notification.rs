use crate::db::Database;
use chrono::{Datelike, Local, NaiveDate, NaiveDateTime, NaiveTime};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::async_runtime;
use tauri::WebviewUrl;
use tauri::WebviewWindowBuilder;
use tauri::{Emitter, Listener, Manager};
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

            // 启动时补发一次错过的重复提醒
            if let Err(e) = Self::catch_up_missed_repeats(&app_handle) {
                eprintln!("补发错过的重复提醒失败: {}", e);
            }

            loop {
                Self::sleep_until_next_minute().await;
                if let Err(e) = Self::check_and_send_notifications(&app_handle) {
                    eprintln!("通知检查失败: {}", e);
                }
            }
        });
    }

    /// 等待到下一个整分（本地时间）
    async fn sleep_until_next_minute() {
        let since_epoch = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_else(|_| Duration::from_secs(0));
        let secs = since_epoch.as_secs();
        let nanos = since_epoch.subsec_nanos();
        let remainder = secs % 60;

        if remainder == 0 && nanos == 0 {
            return;
        }

        let mut wait_secs = 59 - remainder;
        let mut wait_nanos = 1_000_000_000 - nanos;
        if wait_nanos == 1_000_000_000 {
            wait_secs += 1;
            wait_nanos = 0;
        }

        tokio::time::sleep(Duration::new(wait_secs, wait_nanos)).await;
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

            if todo.repeat_enabled {
                Self::advance_repeat(&db, &todo)?;
            } else {
                Self::mark_as_notified(&db, todo.id)?;
            }
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
                SELECT id, title, description, repeat_enabled, repeat_type, repeat_interval,
                       repeat_weekdays, repeat_month_day, notify_at
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
                    repeat_enabled: row.get::<_, i32>(3).unwrap_or(0) != 0,
                    repeat_type: row.get(4).unwrap_or(None),
                    repeat_interval: row.get(5).unwrap_or(1),
                    repeat_weekdays: row.get(6).unwrap_or(None),
                    repeat_month_day: row.get(7).unwrap_or(None),
                    notify_at: row.get(8).unwrap_or(None),
                })
            })?
            .filter_map(|r| r.ok())
            .collect();

            Ok(todos)
        })
        .map_err(|e| e.to_string())
    }

    /// 发送系统通知
    fn send_system_notification(
        app_handle: &tauri::AppHandle,
        title: &str,
        description: &Option<String>,
    ) -> Result<(), String> {
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
    fn send_app_notification(
        app_handle: &tauri::AppHandle,
        title: &str,
        description: &Option<String>,
    ) -> Result<(), String> {
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
        let y = screen_height
            - NOTIFICATION_HEIGHT
            - NOTIFICATION_MARGIN
            - (active_count * (NOTIFICATION_HEIGHT + NOTIFICATION_SPACING));

        // URL 编码标题和描述
        let encoded_title = urlencoding::encode(title);
        let encoded_desc = urlencoding::encode(description.as_deref().unwrap_or("待办事项提醒"));
        let encoded_label = urlencoding::encode(&window_label);

        // 创建通知窗口
        let url = format!(
            "index.html#/notification?title={}&description={}&label={}",
            encoded_title, encoded_desc, encoded_label
        );

        let window_label_clone = window_label.clone();
        let app_handle_clone = app_handle.clone();

        // 在主线程创建窗口
        let mut window_builder =
            WebviewWindowBuilder::new(app_handle, &window_label, WebviewUrl::App(url.into()))
                .title("通知")
                .inner_size(NOTIFICATION_WIDTH as f64, NOTIFICATION_HEIGHT as f64)
                .position(x as f64, y as f64)
                .decorations(false)
                .always_on_top(true)
                .resizable(false)
                .skip_taskbar(true)
                .focused(false)
                .visible(true);

        #[cfg(not(target_os = "macos"))]
        {
            window_builder = window_builder.transparent(true);
        }

        let _ = window_builder.build().map_err(|e| e.to_string())?;

        // 监听窗口关闭事件，减少活动通知计数
        let _ = app_handle.listen(
            format!("notification-closed-{}", window_label_clone),
            move |_| {
                ACTIVE_NOTIFICATIONS.fetch_sub(1, Ordering::SeqCst);
            },
        );

        // 监听窗口销毁事件
        if let Some(window) = app_handle_clone.get_webview_window(&window_label_clone) {
            let app_handle_for_destroy = app_handle_clone.clone();
            let label_for_destroy = window_label_clone.clone();
            window.on_window_event(move |event| {
                if let tauri::WindowEvent::Destroyed = event {
                    ACTIVE_NOTIFICATIONS.fetch_sub(1, Ordering::SeqCst);
                    let _ = app_handle_for_destroy
                        .emit(&format!("notification-closed-{}", label_for_destroy), ());
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

    /// 推进重复提醒到下一次
    fn advance_repeat(db: &Database, todo: &PendingNotification) -> Result<(), String> {
        let notify_at_str = match &todo.notify_at {
            Some(s) => s.clone(),
            None => return Self::mark_as_notified(db, todo.id),
        };

        let current_dt = NaiveDateTime::parse_from_str(&notify_at_str, "%Y-%m-%dT%H:%M:%S")
            .or_else(|_| NaiveDateTime::parse_from_str(&notify_at_str, "%Y-%m-%dT%H:%M"))
            .map_err(|e| format!("解析 notify_at 失败: {}", e))?;

        let now = Local::now().naive_local();
        let next = Self::calc_next_occurrence(current_dt, now, todo);

        match next {
            Some(next_dt) => {
                let next_str = next_dt.format("%Y-%m-%dT%H:%M:%S").to_string();
                db.with_connection(|conn| {
                    conn.execute(
                        "UPDATE todos SET notify_at = ?, notified = 0, updated_at = datetime('now', 'localtime') WHERE id = ?",
                        rusqlite::params![next_str, todo.id],
                    )?;
                    Ok(())
                })
                .map_err(|e| e.to_string())
            }
            None => Self::mark_as_notified(db, todo.id),
        }
    }

    /// 计算下一次重复时间（循环推进直到 > now）
    fn calc_next_occurrence(
        from: NaiveDateTime,
        now: NaiveDateTime,
        todo: &PendingNotification,
    ) -> Option<NaiveDateTime> {
        let repeat_type = todo.repeat_type.as_deref()?;
        let interval = todo.repeat_interval.max(1);
        let time = from.time();
        let mut candidate = from;

        for _ in 0..366 * 5 {
            candidate = match repeat_type {
                "daily" => candidate + chrono::Duration::days(interval as i64),
                "weekly" => {
                    Self::next_weekly(candidate, interval, todo.repeat_weekdays.as_deref(), time)?
                }
                "monthly" => Self::next_monthly(candidate, interval, todo.repeat_month_day, time)?,
                _ => return None,
            };
            if candidate > now {
                return Some(candidate);
            }
        }
        None
    }

    /// 周模式：找下一个匹配的星期
    fn next_weekly(
        current: NaiveDateTime,
        interval: i32,
        weekdays_str: Option<&str>,
        time: NaiveTime,
    ) -> Option<NaiveDateTime> {
        let mut weekdays: Vec<u32> = weekdays_str
            .unwrap_or("1,2,3,4,5,6,7")
            .split(',')
            .filter_map(|s| s.trim().parse::<u32>().ok())
            .filter(|&d| (1..=7).contains(&d))
            .collect();
        weekdays.sort_unstable();

        if weekdays.is_empty() {
            return Some(current + chrono::Duration::weeks(interval as i64));
        }

        let current_iso = current.date().weekday().number_from_monday();
        // 在当前周内找下一个匹配日
        for &wd in &weekdays {
            if wd > current_iso {
                let diff = wd - current_iso;
                let date = current.date() + chrono::Duration::days(diff as i64);
                return Some(NaiveDateTime::new(date, time));
            }
        }
        // 跳到 interval 周后的第一个匹配日
        let days_to_monday = 7 - current_iso + 1;
        let base_date = current.date()
            + chrono::Duration::days(days_to_monday as i64)
            + chrono::Duration::weeks((interval - 1) as i64);
        let first_wd = weekdays.iter().min().copied().unwrap_or(1);
        let date = base_date + chrono::Duration::days((first_wd - 1) as i64);
        Some(NaiveDateTime::new(date, time))
    }

    /// 月模式：跳到下 N 个月的指定日
    fn next_monthly(
        current: NaiveDateTime,
        interval: i32,
        month_day: Option<i32>,
        time: NaiveTime,
    ) -> Option<NaiveDateTime> {
        let target_day = month_day.unwrap_or(current.day() as i32).clamp(1, 31) as u32;
        let mut month = current.month() as i32 + interval;
        let mut year = current.year();

        while month > 12 {
            month -= 12;
            year += 1;
        }
        while month < 1 {
            month += 12;
            year -= 1;
        }

        let last_day = last_day_of_month(year, month as u32);
        let day = target_day.min(last_day);
        let date = NaiveDate::from_ymd_opt(year, month as u32, day)?;
        Some(NaiveDateTime::new(date, time))
    }

    /// 启动时补发错过的重复提醒
    fn catch_up_missed_repeats(app_handle: &tauri::AppHandle) -> Result<(), String> {
        let db = app_handle.state::<Database>();
        let notification_type = Self::get_notification_type(&db);

        let overdue = db
            .with_connection(|conn| {
                let mut stmt = conn.prepare(
                    r#"
                SELECT id, title, description, repeat_enabled, repeat_type, repeat_interval,
                       repeat_weekdays, repeat_month_day, notify_at
                FROM todos
                WHERE completed = 0
                  AND repeat_enabled = 1
                  AND notified = 0
                  AND notify_at IS NOT NULL
                  AND datetime(notify_at) <= datetime('now', 'localtime')
                "#,
                )?;

                let todos: Vec<PendingNotification> = stmt
                    .query_map([], |row| {
                        Ok(PendingNotification {
                            id: row.get(0)?,
                            title: row.get(1)?,
                            description: row.get::<_, Option<String>>(2)?,
                            repeat_enabled: row.get::<_, i32>(3).unwrap_or(0) != 0,
                            repeat_type: row.get(4).unwrap_or(None),
                            repeat_interval: row.get(5).unwrap_or(1),
                            repeat_weekdays: row.get(6).unwrap_or(None),
                            repeat_month_day: row.get(7).unwrap_or(None),
                            notify_at: row.get(8).unwrap_or(None),
                        })
                    })?
                    .filter_map(|r| r.ok())
                    .collect();

                Ok(todos)
            })
            .map_err(|e| e.to_string())?;

        for todo in &overdue {
            match notification_type.as_str() {
                "app" => {
                    let _ = Self::send_app_notification(app_handle, &todo.title, &todo.description);
                }
                _ => {
                    let _ =
                        Self::send_system_notification(app_handle, &todo.title, &todo.description);
                }
            }
            Self::advance_repeat(&db, todo)?;
        }

        Ok(())
    }
}

fn last_day_of_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 {
                29
            } else {
                28
            }
        }
        _ => 30,
    }
}

/// 待发送通知的待办
struct PendingNotification {
    id: i64,
    title: String,
    description: Option<String>,
    repeat_enabled: bool,
    repeat_type: Option<String>,
    repeat_interval: i32,
    repeat_weekdays: Option<String>,
    repeat_month_day: Option<i32>,
    notify_at: Option<String>,
}
