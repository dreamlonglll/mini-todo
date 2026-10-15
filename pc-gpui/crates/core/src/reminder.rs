//! 提醒调度的纯逻辑（原 `services/notification.rs` 去掉 Tauri 部分）。
//!
//! app 层的调度循环：
//!
//! ```text
//! 启动后等 STARTUP_DELAY → catch_up_missed_repeats(db, deliver)      // 补发已过点的重复提醒
//! loop { sleep(duration_until_next_minute(now)) → run_due(db, deliver) }  // 对齐整分钟
//! ```
//!
//! `deliver` 由 app 提供：按 [`notification_kind`] 发系统通知或开应用内通知窗口，返回 `Ok` 表示已发出。
//! 发送失败只记日志、**不推进**（下一分钟重试）；成功后重复提醒推进到下一次，否则标记已通知。
//! 单条提醒出错不影响同一轮的其它提醒。全部函数阻塞（读写数据库），放后台线程执行。
//!
//! 应用内通知窗口的摆放（右下角向上堆叠、满列向左换列）也在这里：[`notification_position`]、
//! 槽位池 [`NOTIFICATION_SLOTS`]。

use crate::db::time::{
    format_canonical, parse_local_datetime_with, DefaultTime, SQL_SET_UPDATED_AT,
};
use crate::db::Database;
use chrono::{Datelike, Local, NaiveDate, NaiveDateTime, NaiveTime};
use std::collections::BTreeSet;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// 启动后多久开始第一次检查（原调度器先等应用初始化完成）
pub const STARTUP_DELAY: Duration = Duration::from_secs(5);

/// 没有描述时通知正文的默认文字
pub const DEFAULT_BODY: &str = "待办事项提醒";

// 通知窗口计数器（用于生成唯一的窗口标签）
static NOTIFICATION_COUNTER: AtomicU32 = AtomicU32::new(0);

/// 新的应用内通知窗口标签 `notification_{n}`（进程内递增）
pub fn next_window_label() -> String {
    format!(
        "notification_{}",
        NOTIFICATION_COUNTER.fetch_add(1, Ordering::SeqCst)
    )
}

/// 正在显示的应用内通知窗口占用的堆叠槽位。
///
/// 新通知取最小的空闲槽位，窗口销毁时归还：中间某条先关掉后，下一条补进它的空位，
/// 不会叠在仍在显示的通知上（单纯计数做不到这一点）。
pub static NOTIFICATION_SLOTS: SlotPool = SlotPool::new();

/// 应用内通知窗口的堆叠槽位池：新通知取最小空闲槽位，窗口关闭时归还
pub struct SlotPool(Mutex<BTreeSet<u32>>);

impl SlotPool {
    pub const fn new() -> Self {
        Self(Mutex::new(BTreeSet::new()))
    }

    /// 取一个空闲槽位（最小的未占用编号）
    pub fn acquire(&self) -> u32 {
        acquire_slot(&self.0)
    }

    /// 归还槽位（重复归还无害）
    pub fn release(&self, slot: u32) {
        release_slot(&self.0, slot)
    }
}

impl Default for SlotPool {
    fn default() -> Self {
        Self::new()
    }
}

// 通知窗口尺寸与间距（逻辑像素）
pub const NOTIFICATION_WIDTH: f64 = 320.0;
pub const NOTIFICATION_HEIGHT: f64 = 120.0;
pub const NOTIFICATION_MARGIN: f64 = 20.0;
pub const NOTIFICATION_SPACING: f64 = 10.0;

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
pub struct WorkArea {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// 取不到主显示器时按 1920×1080 摆放
pub const FALLBACK_WORK_AREA: WorkArea = WorkArea {
    x: 0.0,
    y: 0.0,
    width: 1920.0,
    height: 1080.0,
};

/// 通知窗口的逻辑尺寸
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NotificationSize {
    pub width: f64,
    pub height: f64,
}

/// 请求的窗口尺寸。真实尺寸可能更大（平台强加最小尺寸，如 Linux / GTK 把 320×120 撑成 320×200），
/// 定位与堆叠都以真实尺寸为准，见 [`fit_to_actual_size`]。
pub const REQUESTED_SIZE: NotificationSize = NotificationSize {
    width: NOTIFICATION_WIDTH,
    height: NOTIFICATION_HEIGHT,
};

/// 最近一次观测到的通知窗口真实尺寸：后续通知直接按它摆放，不必先按请求尺寸放下再挪位置
static LAST_ACTUAL_SIZE: Mutex<Option<NotificationSize>> = Mutex::new(None);

impl NotificationSize {
    /// 物理像素 → 逻辑像素；尺寸为 0 或缩放比例不合法时返回 `None`
    pub fn from_physical(width: u32, height: u32, scale: f64) -> Option<Self> {
        if width == 0 || height == 0 || !(scale.is_finite() && scale > 0.0) {
            return None;
        }
        Some(Self {
            width: width as f64 / scale,
            height: height as f64 / scale,
        })
    }

    /// 相差超过半个逻辑像素才算不同（物理像素换算的舍入误差不算）
    pub fn differs_from(self, other: Self) -> bool {
        (self.width - other.width).abs() >= 0.5 || (self.height - other.height).abs() >= 0.5
    }
}

/// 上次观测到的通知窗口真实尺寸；新通知先按它摆（没有时按 [`REQUESTED_SIZE`]）
pub fn last_actual_size() -> Option<NotificationSize> {
    *LAST_ACTUAL_SIZE.lock().unwrap_or_else(|e| e.into_inner())
}

/// 记下通知窗口的真实尺寸
pub fn remember_actual_size(size: NotificationSize) {
    *LAST_ACTUAL_SIZE.lock().unwrap_or_else(|e| e.into_inner()) = Some(size);
}

/// 第 `slot` 个通知窗口（逻辑尺寸 `size`）的左上角逻辑坐标：从工作区右下角往上堆叠，
/// 一列放不下就往左开新列；整个工作区都放满时从头复用位置（取模），永远落在工作区内。
/// 行高、列宽都按窗口真实尺寸算——按请求尺寸算的话，被系统撑高的窗口底部会被工作区边缘切掉。
/// 全程浮点运算，不会出现 u32 下溢。
pub fn notification_position(area: WorkArea, size: NotificationSize, slot: u32) -> (f64, f64) {
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

/// 按窗口的真实尺寸校正位置（原 `fit_to_actual_size`）：记下真实尺寸供后续通知直接使用；与摆放时假设的
/// 尺寸 `placed_for` 不同就返回本槽位按真实尺寸重算的坐标（逻辑像素），调用方把窗口移过去
pub fn fit_to_actual_size(
    area: WorkArea,
    slot: u32,
    placed_for: NotificationSize,
    actual: NotificationSize,
) -> Option<(f64, f64)> {
    remember_actual_size(actual);
    actual
        .differs_from(placed_for)
        .then(|| notification_position(area, actual, slot))
}

/// 解析 `notify_at`：兼容规范空格格式与旧的 `T` 格式（K1）；仅日期按 09:00 处理
fn parse_notify_at(s: &str) -> Option<NaiveDateTime> {
    parse_local_datetime_with(s, DefaultTime::Notify)
}

/// 待发送提醒的待办
#[derive(Debug, Clone, PartialEq)]
pub struct Reminder {
    pub id: i64,
    pub title: String,
    pub description: Option<String>,
    pub repeat_enabled: bool,
    pub repeat_type: Option<String>,
    pub repeat_interval: i32,
    pub repeat_weekdays: Option<String>,
    pub repeat_month_day: Option<i32>,
    pub notify_at: Option<String>,
}

impl Reminder {
    /// 通知正文：描述，没有时为 [`DEFAULT_BODY`]
    pub fn body(&self) -> &str {
        self.description.as_deref().unwrap_or(DEFAULT_BODY)
    }
}

/// 通知方式（settings `notification_type`）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationKind {
    /// 系统通知（默认）
    System,
    /// 应用内通知窗口
    App,
}

/// 当前通知方式：`app` → 应用内窗口，其它（含读库失败）→ 系统通知。每轮检查前重新读取
pub fn notification_kind(db: &Database) -> NotificationKind {
    let value = db
        .with_connection(|conn| {
            Ok(crate::db::settings_kv::get_setting_or(
                conn,
                "notification_type",
                "system",
            ))
        })
        .unwrap_or_else(|_| "system".to_string());
    if value == "app" {
        NotificationKind::App
    } else {
        NotificationKind::System
    }
}

const PENDING_COLUMNS: &str = "id, title, description, repeat_enabled, repeat_type, \
     repeat_interval, repeat_weekdays, repeat_month_day, notify_at";

/// 重复提醒相关列类型不对时按"未设置"处理：坏掉的可选字段不该让整条提醒发不出去
fn pending_from_row(row: &rusqlite::Row) -> rusqlite::Result<Reminder> {
    Ok(Reminder {
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
fn query_pending(db: &Database, sql: &str) -> Result<Vec<Reminder>, String> {
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

/// 等到下一个整分（本地时间与 UNIX 时间的分钟边界一致）的时长；正好在整分上时为 0
pub fn duration_until_next_minute(now: SystemTime) -> Duration {
    let since_epoch = now
        .duration_since(UNIX_EPOCH)
        .unwrap_or_else(|_| Duration::from_secs(0));
    let secs = since_epoch.as_secs();
    let nanos = since_epoch.subsec_nanos();
    let remainder = secs % 60;

    if remainder == 0 && nanos == 0 {
        return Duration::ZERO;
    }

    let mut wait_secs = 59 - remainder;
    let mut wait_nanos = 1_000_000_000 - nanos;
    if wait_nanos == 1_000_000_000 {
        wait_secs += 1;
        wait_nanos = 0;
    }
    Duration::new(wait_secs, wait_nanos)
}

/// 检查并发出到期的提醒（到点扣除提前量、未完成、未通知，按提醒时间排序），逐条交给 `deliver`。
/// 一条失败只记日志，不影响同一轮的其它提醒。返回 `Err` 只表示查询本身失败。
pub fn run_due(
    db: &Database,
    deliver: &mut dyn FnMut(&Reminder) -> Result<(), String>,
) -> Result<(), String> {
    for todo in query_pending(db, &due_reminders_sql())? {
        deliver_one(db, &todo, deliver);
    }
    Ok(())
}

/// 启动时补发错过的重复提醒（已过点、未通知），规则同 [`run_due`]
pub fn catch_up_missed_repeats(
    db: &Database,
    deliver: &mut dyn FnMut(&Reminder) -> Result<(), String>,
) -> Result<(), String> {
    for todo in query_pending(db, &missed_repeats_sql())? {
        deliver_one(db, &todo, deliver);
    }
    Ok(())
}

/// 发出一条提醒并推进它的状态。
///
/// 发送失败：只记日志、不推进，下一分钟重试（什么都没弹出来，不会刷屏）。
/// 推进失败：记日志；重复提醒的时间无法解析时直接标记已通知，避免每分钟弹一次。
fn deliver_one(
    db: &Database,
    todo: &Reminder,
    deliver: &mut dyn FnMut(&Reminder) -> Result<(), String>,
) {
    if let Err(e) = deliver(todo) {
        log::error!("[notify] 待办 #{} 的提醒发送失败: {}", todo.id, e);
        return;
    }

    let advanced = if todo.repeat_enabled {
        advance_repeat(db, todo)
    } else {
        mark_as_notified(db, todo.id)
    };
    if let Err(e) = advanced {
        log::error!("[notify] 更新待办 #{} 的提醒状态失败: {}", todo.id, e);
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
fn advance_repeat(db: &Database, todo: &Reminder) -> Result<(), String> {
    let Some(notify_at) = todo.notify_at.as_deref() else {
        return mark_as_notified(db, todo.id);
    };
    let Some(current) = parse_notify_at(notify_at) else {
        mark_as_notified(db, todo.id)?;
        return Err(format!(
            "无法解析重复提醒时间 {:?}，已停止该提醒（重新编辑提醒时间即可恢复）",
            notify_at
        ));
    };

    let now = Local::now().naive_local();
    let Some(next) = calc_next_occurrence(current, now, todo) else {
        return mark_as_notified(db, todo.id);
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
    todo: &Reminder,
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
            "weekly" => next_weekly(candidate, interval, todo.repeat_weekdays.as_deref(), time)?,
            "monthly" => next_monthly(candidate, interval, month_day, time)?,
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

    fn rule(kind: &str, interval: i32, weekdays: Option<&str>, month_day: Option<i32>) -> Reminder {
        Reminder {
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
    fn next(from: &str, todo: &Reminder) -> String {
        let from = dt(from);
        calc_next_occurrence(from, from, todo)
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
        let got =
            calc_next_occurrence(dt("2026-05-01 09:00:00"), dt("2026-05-10 12:00:00"), &daily);
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
        let got = calc_next_occurrence(
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

    fn pending(db: &Database, id: i64) -> Reminder {
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

        advance_repeat(&db, &todo).expect("推进");
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

        mark_as_notified(&db, 1).unwrap();
        assert_eq!(row_state(&db, 1).3, "2099-01-01 08:00:01");
        mark_as_notified(&db, 1).unwrap();
        assert_eq!(
            row_state(&db, 1).3,
            "2099-01-01 08:00:02",
            "同一秒内也严格递增"
        );

        let todo = pending(&db, 2);
        advance_repeat(&db, &todo).unwrap();
        assert_eq!(row_state(&db, 2).3, "2099-01-01 08:00:01");
    }

    #[test]
    fn unparsable_repeat_time_is_marked_notified_instead_of_firing_every_minute() {
        let db = Database::new_in_memory().unwrap();
        insert_reminder(&db, 1, "next tuesday", "daily");
        let todo = pending(&db, 1);
        assert!(advance_repeat(&db, &todo).is_err());
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

    // ---- 调度入口（原调度循环里的逻辑，投递改为回调）----

    #[test]
    fn next_minute_wait_aligns_to_the_unix_minute() {
        let at = |secs: u64, nanos: u32| UNIX_EPOCH + Duration::new(secs, nanos);
        assert_eq!(duration_until_next_minute(at(120, 0)), Duration::ZERO);
        assert_eq!(
            duration_until_next_minute(at(121, 0)),
            Duration::from_secs(59)
        );
        assert_eq!(
            duration_until_next_minute(at(179, 999_999_999)),
            Duration::from_nanos(1)
        );
        assert_eq!(
            duration_until_next_minute(at(150, 500_000_000)),
            Duration::from_millis(29_500)
        );
    }

    #[test]
    fn delivered_reminders_advance_and_failed_ones_are_retried_next_minute() {
        let db = Database::new_in_memory().unwrap();
        insert_reminder(&db, 1, "2020-01-01 09:00:00", "");
        insert_reminder(&db, 2, "2020-01-02 09:00:00", "");
        // 重复提醒最多向后推进 5 年，过期太久的会被直接标记已通知：用昨天
        let yesterday = format_canonical(&(Local::now().naive_local() - chrono::Duration::days(1)));
        insert_reminder(&db, 3, &yesterday, "daily");
        db.with_connection(|c| c.execute("UPDATE todos SET description = 'd' WHERE id = 3", []))
            .unwrap();

        let mut seen = Vec::new();
        run_due(&db, &mut |r: &Reminder| {
            seen.push((r.id, r.body().to_string()));
            if r.id == 2 {
                Err("通知服务不可用".to_string())
            } else {
                Ok(())
            }
        })
        .unwrap();
        assert_eq!(
            seen,
            vec![
                (1, DEFAULT_BODY.to_string()),
                (2, DEFAULT_BODY.to_string()),
                (3, "d".to_string())
            ]
        );
        assert!(row_state(&db, 1).1, "已发出：标记已通知");
        assert!(!row_state(&db, 2).1, "发送失败：不推进，下一分钟重试");
        let (notify_at, notified, _, _) = row_state(&db, 3);
        assert!(!notified, "重复提醒推进到下一次");
        assert!(dt(&notify_at.unwrap()) > Local::now().naive_local());

        // 下一轮只剩发送失败的那条
        let mut retried = Vec::new();
        run_due(&db, &mut |r: &Reminder| {
            retried.push(r.id);
            Ok(())
        })
        .unwrap();
        assert_eq!(retried, vec![2]);
    }

    #[test]
    fn startup_catch_up_only_picks_missed_repeats() {
        let db = Database::new_in_memory().unwrap();
        insert_reminder(&db, 1, "2020-01-01 09:00:00", "");
        insert_reminder(&db, 2, "2020-01-01 09:00:00", "weekly");
        insert_reminder(&db, 3, "2999-01-01 09:00:00", "daily");
        let mut seen = Vec::new();
        catch_up_missed_repeats(&db, &mut |r: &Reminder| {
            seen.push(r.id);
            Ok(())
        })
        .unwrap();
        assert_eq!(seen, vec![2]);
    }

    #[test]
    fn notification_kind_defaults_to_system() {
        let db = Database::new_in_memory().unwrap();
        assert_eq!(notification_kind(&db), NotificationKind::System);
        crate::settings::set_notification_type(&db, "app").unwrap();
        assert_eq!(notification_kind(&db), NotificationKind::App);
    }

    #[test]
    fn slot_pool_and_size_correction() {
        let pool = SlotPool::new();
        assert_eq!((pool.acquire(), pool.acquire()), (0, 1));
        pool.release(0);
        assert_eq!(pool.acquire(), 0);

        let area = WorkArea {
            x: 0.0,
            y: 0.0,
            width: 800.0,
            height: 500.0,
        };
        assert_eq!(
            fit_to_actual_size(area, 0, REQUESTED_SIZE, REQUESTED_SIZE),
            None,
            "尺寸符合预期不用挪"
        );
        let taller = NotificationSize {
            width: 320.0,
            height: 200.0,
        };
        assert_eq!(
            fit_to_actual_size(area, 0, REQUESTED_SIZE, taller),
            Some((460.0, 280.0))
        );
        assert_eq!(last_actual_size(), Some(taller));
        assert!(next_window_label().starts_with("notification_"));
    }
}
