use crate::db::time::{
    format_canonical, parse_local_datetime_with, DefaultTime, SQL_SET_UPDATED_AT,
};
use crate::db::Database;
use chrono::{Datelike, Local, NaiveDate, NaiveDateTime, NaiveTime};
use std::collections::BTreeSet;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::async_runtime;
use tauri::Manager;
use tauri::WebviewUrl;
use tauri::WebviewWindowBuilder;
use tauri_plugin_notification::NotificationExt;

// 通知窗口计数器（用于生成唯一的窗口标签）
static NOTIFICATION_COUNTER: AtomicU32 = AtomicU32::new(0);

/// 正在显示的应用内通知窗口占用的堆叠槽位。
///
/// 新通知取最小的空闲槽位，窗口销毁时归还：中间某条先关掉后，下一条补进它的空位，
/// 不会叠在仍在显示的通知上（单纯计数做不到这一点）。
static NOTIFICATION_SLOTS: Mutex<BTreeSet<u32>> = Mutex::new(BTreeSet::new());

// 通知窗口尺寸与间距（逻辑像素）
const NOTIFICATION_WIDTH: f64 = 320.0;
const NOTIFICATION_HEIGHT: f64 = 120.0;
const NOTIFICATION_MARGIN: f64 = 20.0;
const NOTIFICATION_SPACING: f64 = 10.0;

/// 取一个空闲槽位（最小的未占用编号）
fn acquire_slot(slots: &Mutex<BTreeSet<u32>>) -> u32 {
    let mut taken = slots.lock().unwrap_or_else(|e| e.into_inner());
    let slot = (0..).find(|n| !taken.contains(n)).unwrap_or(0);
    taken.insert(slot);
    slot
}

/// 归还槽位；归还未占用的槽位是无害的空操作（窗口销毁事件重复触发也不会出错）
fn release_slot(slots: &Mutex<BTreeSet<u32>>, slot: u32) {
    slots
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&slot);
}

/// 显示器工作区（逻辑像素，已扣除任务栏）
#[derive(Debug, Clone, Copy, PartialEq)]
struct WorkArea {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

/// 通知窗口的逻辑尺寸
#[derive(Debug, Clone, Copy, PartialEq)]
struct NotificationSize {
    width: f64,
    height: f64,
}

/// 向窗口构建器请求的尺寸。真实尺寸可能更大（Linux / GTK 有最小尺寸限制，320×120 实际出来是
/// 320×200），定位与堆叠都以真实尺寸为准，见 [`NotificationService::fit_to_actual_size`]。
const REQUESTED_SIZE: NotificationSize = NotificationSize {
    width: NOTIFICATION_WIDTH,
    height: NOTIFICATION_HEIGHT,
};

/// 最近一次观测到的通知窗口真实尺寸：后续通知直接按它摆放，不必先按请求尺寸放下再挪位置
static LAST_ACTUAL_SIZE: Mutex<Option<NotificationSize>> = Mutex::new(None);

impl NotificationSize {
    /// 物理像素 → 逻辑像素；尺寸为 0 或缩放比例不合法时返回 `None`
    fn from_physical(width: u32, height: u32, scale: f64) -> Option<Self> {
        if width == 0 || height == 0 || !(scale.is_finite() && scale > 0.0) {
            return None;
        }
        Some(Self {
            width: width as f64 / scale,
            height: height as f64 / scale,
        })
    }

    /// 相差超过半个逻辑像素才算不同（物理像素换算的舍入误差不算）
    fn differs_from(self, other: Self) -> bool {
        (self.width - other.width).abs() >= 0.5 || (self.height - other.height).abs() >= 0.5
    }
}

fn last_actual_size() -> Option<NotificationSize> {
    *LAST_ACTUAL_SIZE.lock().unwrap_or_else(|e| e.into_inner())
}

fn remember_actual_size(size: NotificationSize) {
    *LAST_ACTUAL_SIZE.lock().unwrap_or_else(|e| e.into_inner()) = Some(size);
}

/// 第 `slot` 个通知窗口（逻辑尺寸 `size`）的左上角逻辑坐标：从工作区右下角往上堆叠，
/// 一列放不下就往左开新列；整个工作区都放满时从头复用位置（取模），永远落在工作区内。
/// 行高、列宽都按窗口真实尺寸算——按请求尺寸算的话，被系统撑高的窗口底部会被工作区边缘切掉。
/// 全程浮点运算，不会出现 u32 下溢。
fn notification_position(area: WorkArea, size: NotificationSize, slot: u32) -> (f64, f64) {
    let row_step = size.height + NOTIFICATION_SPACING;
    let col_step = size.width + NOTIFICATION_SPACING;
    let rows = ((area.height - 2.0 * NOTIFICATION_MARGIN + NOTIFICATION_SPACING) / row_step)
        .floor()
        .max(1.0) as u32;
    let cols = ((area.width - 2.0 * NOTIFICATION_MARGIN + NOTIFICATION_SPACING) / col_step)
        .floor()
        .max(1.0) as u32;
    let slot = slot % rows.saturating_mul(cols).max(1);
    let (col, row) = (slot / rows, slot % rows);

    let x = area.x + area.width - NOTIFICATION_MARGIN - size.width - col as f64 * col_step;
    let y = area.y + area.height - NOTIFICATION_MARGIN - size.height - row as f64 * row_step;
    (x, y)
}

/// 解析 `notify_at`：兼容规范空格格式与旧的 `T` 格式（K1）；仅日期按 09:00 处理
fn parse_notify_at(s: &str) -> Option<NaiveDateTime> {
    parse_local_datetime_with(s, DefaultTime::Notify)
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

const PENDING_COLUMNS: &str = "id, title, description, repeat_enabled, repeat_type, \
     repeat_interval, repeat_weekdays, repeat_month_day, notify_at";

/// 重复提醒相关列类型不对时按"未设置"处理：坏掉的可选字段不该让整条提醒发不出去
fn pending_from_row(row: &rusqlite::Row) -> rusqlite::Result<PendingNotification> {
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
}

/// 到点（扣除提前量）且未通知的提醒。
/// notify_before 在 v1 建表时可空：NULL 按 0 处理，否则整条比较恒为 NULL、永不提醒。
fn due_reminders_sql() -> String {
    format!(
        "SELECT {PENDING_COLUMNS} FROM todos
         WHERE completed = 0
           AND notified = 0
           AND notify_at IS NOT NULL
           AND datetime(notify_at, '-' || COALESCE(notify_before, 0) || ' minutes')
               <= datetime('now', 'localtime')
         ORDER BY datetime(notify_at) ASC, id ASC"
    )
}

/// 启动补发：已过点、未通知的重复提醒
fn missed_repeats_sql() -> String {
    format!(
        "SELECT {PENDING_COLUMNS} FROM todos
         WHERE completed = 0
           AND repeat_enabled = 1
           AND notified = 0
           AND notify_at IS NOT NULL
           AND datetime(notify_at) <= datetime('now', 'localtime')
         ORDER BY datetime(notify_at) ASC, id ASC"
    )
}

/// 跑一条查询待发送提醒的 SQL；单行读取失败只记日志并跳过，不影响其它提醒
fn query_pending(db: &Database, sql: &str) -> Result<Vec<PendingNotification>, String> {
    db.with_connection(|conn| {
        let mut stmt = conn.prepare(sql)?;
        let rows = stmt.query_map([], pending_from_row)?;
        let mut out = Vec::new();
        for row in rows {
            match row {
                Ok(todo) => out.push(todo),
                Err(e) => log::error!("[notify] 读取待提醒的待办失败: {}", e),
            }
        }
        Ok(out)
    })
    .map_err(|e| e.to_string())
}

pub struct NotificationService;

impl NotificationService {
    /// 启动通知调度器，每分钟检查一次待办通知
    pub fn start_scheduler(app_handle: tauri::AppHandle) {
        async_runtime::spawn(async move {
            // 等待应用初始化完成
            tokio::time::sleep(Duration::from_secs(5)).await;

            // 启动时补发一次错过的重复提醒
            if let Err(e) = Self::catch_up_missed_repeats(&app_handle) {
                log::error!("[notify] 补发错过的重复提醒失败: {}", e);
            }

            loop {
                Self::sleep_until_next_minute().await;
                if let Err(e) = Self::check_and_send_notifications(&app_handle) {
                    log::error!("[notify] 通知检查失败: {}", e);
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

    /// 检查并发送到期的通知。逐条处理：一条失败只记日志，不影响同一轮的其它提醒。
    fn check_and_send_notifications(app_handle: &tauri::AppHandle) -> Result<(), String> {
        let db = app_handle.state::<Database>();
        let notification_type = Self::get_notification_type(&db);

        for todo in query_pending(&db, &due_reminders_sql())? {
            Self::deliver(app_handle, &db, &notification_type, &todo);
        }
        Ok(())
    }

    /// 发出一条提醒并推进它的状态。
    ///
    /// 发送失败：只记日志、不推进，下一分钟重试（什么都没弹出来，不会刷屏）。
    /// 推进失败：记日志；重复提醒的时间无法解析时直接标记已通知，避免每分钟弹一次。
    fn deliver(
        app_handle: &tauri::AppHandle,
        db: &Database,
        notification_type: &str,
        todo: &PendingNotification,
    ) {
        let sent = match notification_type {
            "app" => Self::send_app_notification(app_handle, &todo.title, &todo.description),
            _ => Self::send_system_notification(app_handle, &todo.title, &todo.description),
        };
        if let Err(e) = sent {
            log::error!("[notify] 待办 #{} 的提醒发送失败: {}", todo.id, e);
            return;
        }

        let advanced = if todo.repeat_enabled {
            Self::advance_repeat(db, todo)
        } else {
            Self::mark_as_notified(db, todo.id)
        };
        if let Err(e) = advanced {
            log::error!("[notify] 更新待办 #{} 的提醒状态失败: {}", todo.id, e);
        }
    }

    /// 获取通知类型设置
    fn get_notification_type(db: &Database) -> String {
        db.with_connection(|conn| {
            Ok(crate::db::settings_kv::get_setting_or(
                conn,
                "notification_type",
                "system",
            ))
        })
        .unwrap_or_else(|_| "system".to_string())
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

        // 右下角堆叠：新通知占最小的空闲槽位。先按上次观测到的真实尺寸摆放（首次按请求尺寸），
        // 建好后再按这扇窗口的真实尺寸校正
        let slot = acquire_slot(&NOTIFICATION_SLOTS);
        let area = Self::primary_work_area(app_handle);
        let expected = last_actual_size().unwrap_or(REQUESTED_SIZE);
        let (x, y) = notification_position(area, expected, slot);

        // URL 编码标题和描述
        let encoded_title = urlencoding::encode(title);
        let encoded_desc = urlencoding::encode(description.as_deref().unwrap_or("待办事项提醒"));
        let encoded_label = urlencoding::encode(&window_label);

        let url = format!(
            "index.html#/notification?title={}&description={}&label={}",
            encoded_title, encoded_desc, encoded_label
        );

        // position / inner_size 都是逻辑像素
        let mut window_builder =
            WebviewWindowBuilder::new(app_handle, &window_label, WebviewUrl::App(url.into()))
                .title("通知")
                .inner_size(NOTIFICATION_WIDTH, NOTIFICATION_HEIGHT)
                .position(x, y)
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

        match window_builder.build() {
            Ok(window) => {
                // 闭包里只拿 AppHandle + 标签，不持有窗口本身（窗口持有自己的事件闭包会成环）
                let app = app_handle.clone();
                let label = window_label.clone();
                window.on_window_event(move |event| match event {
                    // `Destroyed` 是窗口生命周期的权威信号，槽位只在这里归还
                    tauri::WindowEvent::Destroyed => release_slot(&NOTIFICATION_SLOTS, slot),
                    // 有的平台在窗口真正显示时才定下最终尺寸（GTK 最小尺寸），按它重新摆一次
                    tauri::WindowEvent::Resized(inner) => {
                        if let Some(w) = app.get_webview_window(&label) {
                            Self::fit_to_actual_size(&w, area, slot, expected, Some(*inner));
                        }
                    }
                    _ => {}
                });
                // build() 返回时尺寸已经定下的平台之后不一定再发 Resized：立即校正一次
                Self::fit_to_actual_size(&window, area, slot, expected, None);
                Ok(())
            }
            Err(e) => {
                release_slot(&NOTIFICATION_SLOTS, slot);
                Err(e.to_string())
            }
        }
    }

    /// 按窗口的真实尺寸校正位置：与摆放时假设的尺寸 `placed_for` 不同就重算本槽位的坐标并移过去，
    /// 同时记下真实尺寸供后续通知直接使用。`resized` 是 `Resized` 事件带来的物理尺寸，
    /// 为 `None` 时读取窗口当前的外框尺寸。
    fn fit_to_actual_size(
        window: &tauri::WebviewWindow,
        area: WorkArea,
        slot: u32,
        placed_for: NotificationSize,
        resized: Option<tauri::PhysicalSize<u32>>,
    ) {
        let Ok(scale) = window.scale_factor() else {
            return;
        };
        let physical = match resized {
            Some(size) => size,
            None => match window.outer_size() {
                Ok(size) => size,
                Err(_) => return,
            },
        };
        let Some(actual) = NotificationSize::from_physical(physical.width, physical.height, scale)
        else {
            return;
        };
        remember_actual_size(actual);
        if actual.differs_from(placed_for) {
            let (x, y) = notification_position(area, actual, slot);
            let _ = window.set_position(tauri::LogicalPosition::new(x, y));
        }
    }

    /// 主显示器工作区（逻辑像素）。
    ///
    /// `work_area` 已扣掉任务栏；它和 `size` 都是物理像素，而窗口构建器的 `position`
    /// 接受逻辑像素，必须除以缩放比例——直接用物理值时 1080p@125% 的通知整个落在屏幕外。
    fn primary_work_area(app_handle: &tauri::AppHandle) -> WorkArea {
        let Some(monitor) = app_handle.primary_monitor().ok().flatten() else {
            return WorkArea {
                x: 0.0,
                y: 0.0,
                width: 1920.0,
                height: 1080.0,
            };
        };
        let scale = if monitor.scale_factor() > 0.0 {
            monitor.scale_factor()
        } else {
            1.0
        };
        let work_area = monitor.work_area();
        // 个别平台拿不到工作区（返回 0 尺寸）时退回整块显示器
        let (position, size) = if work_area.size.width > 0 && work_area.size.height > 0 {
            (work_area.position, work_area.size)
        } else {
            (*monitor.position(), *monitor.size())
        };
        WorkArea {
            x: position.x as f64 / scale,
            y: position.y as f64 / scale,
            width: size.width as f64 / scale,
            height: size.height as f64 / scale,
        }
    }

    /// 标记待办为已通知。刷新 updated_at：提醒状态要随同步传播到其它设备。
    /// 新时间严格晚于原值（`SQL_SET_UPDATED_AT`），同一秒内的两次写入也能分出先后。
    fn mark_as_notified(db: &Database, todo_id: i64) -> Result<(), String> {
        db.with_connection(|conn| {
            conn.execute(
                &format!("UPDATE todos SET notified = 1, {SQL_SET_UPDATED_AT} WHERE id = ?"),
                [todo_id],
            )?;
            Ok(())
        })
        .map_err(|e| e.to_string())
    }

    /// 推进重复提醒到下一次（规范时间格式落库）。
    ///
    /// `notify_at` 无法解析时标记已通知并返回错误：否则它每分钟都会被选中、弹一次。
    /// 月重复没有 `repeat_month_day` 时以当前 `notify_at` 的日为锚点，并把锚点写回
    /// `repeat_month_day`：否则 31 号推进到 2 月 28 号后，锚点就永久漂移成 28 号。
    fn advance_repeat(db: &Database, todo: &PendingNotification) -> Result<(), String> {
        let Some(notify_at) = todo.notify_at.as_deref() else {
            return Self::mark_as_notified(db, todo.id);
        };
        let Some(current) = parse_notify_at(notify_at) else {
            Self::mark_as_notified(db, todo.id)?;
            return Err(format!(
                "无法解析重复提醒时间 {:?}，已停止该提醒（重新编辑提醒时间即可恢复）",
                notify_at
            ));
        };

        let now = Local::now().naive_local();
        let Some(next) = Self::calc_next_occurrence(current, now, todo) else {
            return Self::mark_as_notified(db, todo.id);
        };
        let anchor_day = (todo.repeat_type.as_deref() == Some("monthly")
            && todo.repeat_month_day.is_none())
        .then(|| current.day() as i32);

        db.with_connection(|conn| {
            conn.execute(
                &format!(
                    "UPDATE todos SET notify_at = ?1, notified = 0,
                            repeat_month_day = COALESCE(repeat_month_day, ?2),
                            {SQL_SET_UPDATED_AT}
                     WHERE id = ?3"
                ),
                rusqlite::params![format_canonical(&next), anchor_day, todo.id],
            )?;
            Ok(())
        })
        .map_err(|e| e.to_string())
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
        // 月重复的锚点日在整个推进过程中保持不变（31 号 → 2 月 28 号 → 3 月 31 号）
        let month_day = todo.repeat_month_day.unwrap_or(from.day() as i32);
        let mut candidate = from;

        for _ in 0..366 * 5 {
            candidate = match repeat_type {
                "daily" => candidate + chrono::Duration::days(interval as i64),
                "weekly" => {
                    Self::next_weekly(candidate, interval, todo.repeat_weekdays.as_deref(), time)?
                }
                "monthly" => Self::next_monthly(candidate, interval, month_day, time)?,
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

    /// 月模式：跳到下 N 个月的锚点日（该月没有这一天时取月末）
    fn next_monthly(
        current: NaiveDateTime,
        interval: i32,
        month_day: i32,
        time: NaiveTime,
    ) -> Option<NaiveDateTime> {
        let target_day = month_day.clamp(1, 31) as u32;
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

    /// 启动时补发错过的重复提醒（逐条处理，规则同 [`Self::deliver`]）
    fn catch_up_missed_repeats(app_handle: &tauri::AppHandle) -> Result<(), String> {
        let db = app_handle.state::<Database>();
        let notification_type = Self::get_notification_type(&db);

        for todo in query_pending(&db, &missed_repeats_sql())? {
            Self::deliver(app_handle, &db, &notification_type, &todo);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn dt(s: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").expect("测试时间格式")
    }

    fn rule(
        kind: &str,
        interval: i32,
        weekdays: Option<&str>,
        month_day: Option<i32>,
    ) -> PendingNotification {
        PendingNotification {
            id: 1,
            title: "t".to_string(),
            description: None,
            repeat_enabled: true,
            repeat_type: Some(kind.to_string()),
            repeat_interval: interval,
            repeat_weekdays: weekdays.map(str::to_string),
            repeat_month_day: month_day,
            notify_at: None,
        }
    }

    /// 从 `from` 推进一次（now 取 from 本身）
    fn next(from: &str, todo: &PendingNotification) -> String {
        let from = dt(from);
        NotificationService::calc_next_occurrence(from, from, todo)
            .map(|d| format_canonical(&d))
            .unwrap_or_default()
    }

    // ---- 槽位 ----

    #[test]
    fn slots_take_the_lowest_free_index() {
        let slots = Mutex::new(BTreeSet::new());
        assert_eq!(acquire_slot(&slots), 0);
        assert_eq!(acquire_slot(&slots), 1);
        assert_eq!(acquire_slot(&slots), 2);
        release_slot(&slots, 1);
        assert_eq!(acquire_slot(&slots), 1, "中间关掉的空位被复用");
        assert_eq!(acquire_slot(&slots), 3);
    }

    #[test]
    fn releasing_a_free_slot_is_a_no_op() {
        let slots = Mutex::new(BTreeSet::new());
        release_slot(&slots, 7);
        assert_eq!(acquire_slot(&slots), 0);
        release_slot(&slots, 0);
        release_slot(&slots, 0);
        assert!(slots.lock().unwrap().is_empty());
    }

    // ---- 通知窗口位置 ----

    /// 1920×1080@125% 去掉 48px 任务栏后的逻辑工作区
    const AREA_125: WorkArea = WorkArea {
        x: 0.0,
        y: 0.0,
        width: 1536.0,
        height: 825.6,
    };

    fn assert_inside(area: WorkArea, size: NotificationSize, (x, y): (f64, f64)) {
        assert!(
            x >= area.x && x + size.width <= area.x + area.width + 1e-9,
            "x={x}"
        );
        assert!(
            y >= area.y && y + size.height <= area.y + area.height + 1e-9,
            "y={y}"
        );
    }

    #[test]
    fn first_notification_sits_in_the_bottom_right_of_the_work_area() {
        let (x, y) = notification_position(AREA_125, REQUESTED_SIZE, 0);
        assert_eq!(x, 1536.0 - 20.0 - 320.0);
        assert!((y - (825.6 - 20.0 - 120.0)).abs() < 1e-9);
        assert_inside(AREA_125, REQUESTED_SIZE, (x, y));
    }

    #[test]
    fn notifications_stack_upwards_then_wrap_into_new_columns() {
        // 825.6 高：每列放得下 (825.6 - 40 + 10) / 130 = 6 个
        let (x0, y0) = notification_position(AREA_125, REQUESTED_SIZE, 0);
        let (x1, y1) = notification_position(AREA_125, REQUESTED_SIZE, 1);
        assert_eq!(x1, x0);
        assert!((y0 - y1 - 130.0).abs() < 1e-9, "向上堆叠");

        let (x6, y6) = notification_position(AREA_125, REQUESTED_SIZE, 6);
        assert_eq!(x6, x0 - 330.0, "第 7 个换到左边一列");
        assert_eq!(y6, y0);

        for slot in 0..200 {
            assert_inside(
                AREA_125,
                REQUESTED_SIZE,
                notification_position(AREA_125, REQUESTED_SIZE, slot),
            );
        }
    }

    #[test]
    fn offset_and_tiny_work_areas_stay_on_screen() {
        // 任务栏在顶部 / 左侧：工作区原点不是 (0, 0)
        let area = WorkArea {
            x: 40.0,
            y: 30.0,
            width: 1000.0,
            height: 700.0,
        };
        for slot in 0..50 {
            assert_inside(
                area,
                REQUESTED_SIZE,
                notification_position(area, REQUESTED_SIZE, slot),
            );
        }
        // 放不下一整个窗口的极小工作区：不 panic，坐标有限
        let tiny = WorkArea {
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 50.0,
        };
        let (x, y) = notification_position(tiny, REQUESTED_SIZE, u32::MAX);
        assert!(x.is_finite() && y.is_finite());
    }

    /// e2e 实测（Linux，GDK_SCALE=2）：请求 320×120，GTK 撑成 320×200；按请求尺寸算的位置
    /// 让窗口底部被工作区边缘切掉 80 个逻辑像素。按真实尺寸重算后整扇窗口都在工作区内，
    /// 堆叠也按真实行高
    #[test]
    fn taller_than_requested_windows_stay_inside_the_work_area() {
        // 1600×1000 物理 @2x，无面板
        let area = WorkArea {
            x: 0.0,
            y: 0.0,
            width: 800.0,
            height: 500.0,
        };
        let actual = NotificationSize::from_physical(640, 400, 2.0).unwrap();
        assert_eq!(
            actual,
            NotificationSize {
                width: 320.0,
                height: 200.0
            }
        );
        assert!(actual.differs_from(REQUESTED_SIZE));

        // 旧算法（按请求尺寸）：y = 360，底边 560 > 500，被切掉
        let (_, stale_y) = notification_position(area, REQUESTED_SIZE, 0);
        assert!(stale_y + actual.height > area.height);

        let (x0, y0) = notification_position(area, actual, 0);
        assert_eq!((x0, y0), (800.0 - 20.0 - 320.0, 500.0 - 20.0 - 200.0));
        let (_, y1) = notification_position(area, actual, 1);
        assert_eq!(y0 - y1, 210.0, "按真实行高向上堆叠");
        for slot in 0..50 {
            assert_inside(area, actual, notification_position(area, actual, slot));
        }
    }

    #[test]
    fn physical_sizes_convert_with_the_scale_factor_and_reject_garbage() {
        let s = NotificationSize::from_physical(400, 150, 1.25).unwrap();
        assert!(!s.differs_from(REQUESTED_SIZE), "舍入误差内视为相同: {s:?}");
        assert_eq!(NotificationSize::from_physical(0, 150, 1.0), None);
        assert_eq!(NotificationSize::from_physical(320, 120, 0.0), None);
        assert_eq!(NotificationSize::from_physical(320, 120, f64::NAN), None);
    }

    // ---- notify_at 解析 ----

    #[test]
    fn notify_at_accepts_canonical_and_legacy_formats() {
        for s in [
            "2026-05-01 09:30:00",
            "2026-05-01T09:30:00",
            "2026-05-01T09:30",
            "2026-05-01 09:30",
        ] {
            assert_eq!(parse_notify_at(s), Some(dt("2026-05-01 09:30:00")), "{s}");
        }
        assert_eq!(
            parse_notify_at("2026-05-01"),
            Some(dt("2026-05-01 09:00:00"))
        );
        assert_eq!(parse_notify_at("garbage"), None);
    }

    // ---- 重复提醒推进 ----

    #[test]
    fn daily_repeats_and_rolls_over_the_year() {
        let daily = rule("daily", 1, None, None);
        assert_eq!(next("2026-05-01 09:00:00", &daily), "2026-05-02 09:00:00");
        assert_eq!(next("2026-12-31 21:15:00", &daily), "2027-01-01 21:15:00");
        let every3 = rule("daily", 3, None, None);
        assert_eq!(next("2026-02-27 08:00:00", &every3), "2026-03-02 08:00:00");
    }

    #[test]
    fn skips_missed_occurrences_up_to_now() {
        let daily = rule("daily", 1, None, None);
        let got = NotificationService::calc_next_occurrence(
            dt("2026-05-01 09:00:00"),
            dt("2026-05-10 12:00:00"),
            &daily,
        );
        assert_eq!(got, Some(dt("2026-05-11 09:00:00")));
    }

    #[test]
    fn weekly_multiple_days_and_intervals() {
        // 2026-05-04 是周一
        let mwf = rule("weekly", 1, Some("1,3,5"), None);
        assert_eq!(next("2026-05-04 09:00:00", &mwf), "2026-05-06 09:00:00");
        assert_eq!(next("2026-05-06 09:00:00", &mwf), "2026-05-08 09:00:00");
        assert_eq!(next("2026-05-08 09:00:00", &mwf), "2026-05-11 09:00:00");

        // 隔周一、三：本周三之后跳过一整周
        let biweekly = rule("weekly", 2, Some("3,1"), None);
        assert_eq!(
            next("2026-05-04 09:00:00", &biweekly),
            "2026-05-06 09:00:00"
        );
        assert_eq!(
            next("2026-05-06 09:00:00", &biweekly),
            "2026-05-18 09:00:00"
        );

        // 跨年：2026-12-31 是周四
        let fri = rule("weekly", 1, Some("5"), None);
        assert_eq!(next("2026-12-31 07:00:00", &fri), "2027-01-01 07:00:00");

        // 星期全部非法时按整周推进
        let bogus = rule("weekly", 1, Some("0,8,x"), None);
        assert_eq!(next("2026-05-04 09:00:00", &bogus), "2026-05-11 09:00:00");
    }

    #[test]
    fn monthly_clamps_to_month_end_without_drifting() {
        // 没有 repeat_month_day：以 notify_at 的日（31 号）为锚点
        let monthly = rule("monthly", 1, None, None);
        assert_eq!(next("2026-01-31 09:00:00", &monthly), "2026-02-28 09:00:00");
        // 一次推进多个月时锚点不漂移：1/31 → 2/28 → 3/31
        let got = NotificationService::calc_next_occurrence(
            dt("2026-01-31 09:00:00"),
            dt("2026-03-01 00:00:00"),
            &monthly,
        );
        assert_eq!(got, Some(dt("2026-03-31 09:00:00")));

        // 锚点写回 repeat_month_day 后，从 2 月 28 号继续推进仍回到 31 号
        let pinned = rule("monthly", 1, None, Some(31));
        assert_eq!(next("2026-02-28 09:00:00", &pinned), "2026-03-31 09:00:00");
        assert_eq!(next("2026-03-31 09:00:00", &pinned), "2026-04-30 09:00:00");
    }

    #[test]
    fn monthly_handles_leap_years_intervals_and_year_rollover() {
        let on31 = rule("monthly", 1, None, Some(31));
        assert_eq!(
            next("2028-01-31 09:00:00", &on31),
            "2028-02-29 09:00:00",
            "闰年"
        );
        assert_eq!(
            next("2100-01-31 09:00:00", &on31),
            "2100-02-28 09:00:00",
            "整百年非闰"
        );
        assert_eq!(
            next("2000-01-31 09:00:00", &on31),
            "2000-02-29 09:00:00",
            "整四百年闰"
        );

        let on15 = rule("monthly", 1, None, Some(15));
        assert_eq!(
            next("2026-12-15 08:00:00", &on15),
            "2027-01-15 08:00:00",
            "跨年"
        );

        let every2 = rule("monthly", 2, None, Some(30));
        assert_eq!(next("2026-11-30 08:00:00", &every2), "2027-01-30 08:00:00");
        assert_eq!(next("2026-12-30 08:00:00", &every2), "2027-02-28 08:00:00");

        let every13 = rule("monthly", 13, None, Some(1));
        assert_eq!(next("2026-05-01 08:00:00", &every13), "2027-06-01 08:00:00");
    }

    // ---- 落库（内存库）----

    fn insert_reminder(db: &Database, id: i64, notify_at: &str, extra: &str) {
        db.with_connection(|c| {
            c.execute(
                &format!(
                    "INSERT INTO todos (id, title, notify_at, created_at, updated_at{})
                     VALUES (?1, 't', ?2, '2026-01-01 00:00:00', '2026-01-01 00:00:00'{})",
                    if extra.is_empty() {
                        ""
                    } else {
                        ", repeat_enabled, repeat_type"
                    },
                    if extra.is_empty() {
                        String::new()
                    } else {
                        format!(", 1, '{extra}'")
                    }
                ),
                rusqlite::params![id, notify_at],
            )
            .map(|_| ())
        })
        .unwrap();
    }

    fn row_state(db: &Database, id: i64) -> (Option<String>, bool, Option<i32>, String) {
        db.with_connection(|c| {
            c.query_row(
                "SELECT notify_at, notified, repeat_month_day, updated_at FROM todos WHERE id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get::<_, i32>(1)? != 0, r.get(2)?, r.get(3)?)),
            )
        })
        .unwrap()
    }

    fn pending(db: &Database, id: i64) -> PendingNotification {
        query_pending(
            db,
            &format!("SELECT {PENDING_COLUMNS} FROM todos WHERE id = {id}"),
        )
        .unwrap()
        .pop()
        .expect("待办存在")
    }

    #[test]
    fn due_query_treats_null_notify_before_as_zero_and_orders_by_time() {
        let db = Database::new_in_memory().unwrap();
        insert_reminder(&db, 1, "2020-01-02 09:00:00", "");
        insert_reminder(&db, 2, "2020-01-01T09:00:00", "");
        insert_reminder(&db, 3, "2999-01-01 09:00:00", "");
        db.with_connection(|c| c.execute("UPDATE todos SET notify_before = NULL", []))
            .unwrap();

        let due: Vec<i64> = query_pending(&db, &due_reminders_sql())
            .unwrap()
            .iter()
            .map(|t| t.id)
            .collect();
        assert_eq!(
            due,
            vec![2, 1],
            "NULL 提前量按 0；按提醒时间排序；未来的不选"
        );
    }

    #[test]
    fn advancing_a_legacy_monthly_reminder_writes_canonical_time_and_pins_the_anchor() {
        let db = Database::new_in_memory().unwrap();
        insert_reminder(&db, 1, "2026-01-31T09:00", "monthly");
        let todo = pending(&db, 1);
        assert!(todo.repeat_enabled);

        NotificationService::advance_repeat(&db, &todo).expect("推进");
        let (notify_at, notified, month_day, updated_at) = row_state(&db, 1);
        let notify_at = notify_at.unwrap();
        assert_eq!(notify_at.len(), 19, "规范格式: {notify_at}");
        assert!(!notify_at.contains('T'));
        assert!(notify_at.ends_with(" 09:00:00"));
        let next = dt(&notify_at);
        assert!(next > Local::now().naive_local());
        assert_eq!(
            next.day(),
            31.min(last_day_of_month(next.year(), next.month()))
        );
        assert!(!notified, "重复提醒推进后等待下一次");
        assert_eq!(month_day, Some(31), "锚点日写回，之后不再漂移");
        assert!(
            updated_at.as_str() > "2026-01-01 00:00:00",
            "推进要随同步传播"
        );
    }

    /// 提醒状态的写入同样要让 updated_at 严格递增：同步来的版本带着"未来"时间戳时，
    /// 写"现在"会输给它；同一秒内两次写入时间相同，另一端在两次之间同步过就会永远停在第一次
    #[test]
    fn reminder_writes_move_updated_at_strictly_forward() {
        let db = Database::new_in_memory().unwrap();
        insert_reminder(&db, 1, "2020-01-01 09:00:00", "");
        insert_reminder(&db, 2, "2020-01-01 09:00:00", "daily");
        db.with_connection(|c| {
            c.execute("UPDATE todos SET updated_at = '2099-01-01 08:00:00'", [])
        })
        .unwrap();

        NotificationService::mark_as_notified(&db, 1).unwrap();
        assert_eq!(row_state(&db, 1).3, "2099-01-01 08:00:01");
        NotificationService::mark_as_notified(&db, 1).unwrap();
        assert_eq!(
            row_state(&db, 1).3,
            "2099-01-01 08:00:02",
            "同一秒内也严格递增"
        );

        let todo = pending(&db, 2);
        NotificationService::advance_repeat(&db, &todo).unwrap();
        assert_eq!(row_state(&db, 2).3, "2099-01-01 08:00:01");
    }

    #[test]
    fn unparsable_repeat_time_is_marked_notified_instead_of_firing_every_minute() {
        let db = Database::new_in_memory().unwrap();
        insert_reminder(&db, 1, "next tuesday", "daily");
        let todo = pending(&db, 1);
        assert!(NotificationService::advance_repeat(&db, &todo).is_err());
        let (notify_at, notified, _, _) = row_state(&db, 1);
        assert_eq!(
            notify_at.as_deref(),
            Some("next tuesday"),
            "原值保留，便于用户重新编辑"
        );
        assert!(notified);
    }

    #[test]
    fn unknown_or_missing_repeat_type_has_no_next_occurrence() {
        let unknown = rule("yearly", 1, None, None);
        assert_eq!(next("2026-05-01 09:00:00", &unknown), "");
        let mut missing = rule("daily", 1, None, None);
        missing.repeat_type = None;
        assert_eq!(next("2026-05-01 09:00:00", &missing), "");
    }
}
