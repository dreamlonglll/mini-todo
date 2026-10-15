//! 主窗口的纯逻辑（无 GPUI）：重复提醒文案、提醒时间、象限、拖拽排序、日历网格与跨天条、版本比较。
//! 全部逐字移植自旧前端（`TodoItem.vue`、`utils/quadrant.ts`、`utils/color.ts`、`CalendarView.vue`、
//! `appStore.compareVersions`），并带单测。

use std::cmp::Ordering;

use chrono::{Datelike, Duration, NaiveDate};
use mini_todo_core::db::Todo;
use mini_todo_core::db::time::{DefaultTime, parse_local_datetime_with};

// ----------------------------------------------------------------------
// 待办行
// ----------------------------------------------------------------------

/// 重复提醒 tooltip（`TodoItem.vue` 的 `repeatTooltip`）
pub fn repeat_tooltip(todo: &Todo) -> String {
    if !todo.repeat_enabled {
        return String::new();
    }
    let Some(kind) = todo.repeat_type.as_deref().filter(|t| !t.is_empty()) else {
        return String::new();
    };
    let interval = if todo.repeat_interval == 0 {
        1
    } else {
        todo.repeat_interval
    };
    const WEEKDAYS: [&str; 7] = ["一", "二", "三", "四", "五", "六", "日"];
    match kind {
        "daily" => {
            if interval == 1 {
                "每天重复".to_string()
            } else {
                format!("每 {interval} 天重复")
            }
        }
        "weekly" => {
            let days: Vec<String> = todo
                .repeat_weekdays
                .as_deref()
                .unwrap_or("")
                .split(',')
                .filter(|d| !d.is_empty())
                .map(|d| {
                    d.trim()
                        .parse::<usize>()
                        .ok()
                        .and_then(|n| n.checked_sub(1))
                        .and_then(|i| WEEKDAYS.get(i))
                        .map_or_else(|| d.to_string(), |name| (*name).to_string())
                })
                .collect();
            let prefix = if interval == 1 {
                "每周".to_string()
            } else {
                format!("每 {interval} 周")
            };
            if days.is_empty() {
                prefix
            } else {
                format!("{prefix} 周{}", days.join("、"))
            }
        }
        "monthly" => {
            let day = match todo.repeat_month_day {
                Some(d) if d != 0 => d,
                _ => 1,
            };
            if interval == 1 {
                format!("每月 {day} 号")
            } else {
                format!("每 {interval} 月 {day} 号")
            }
        }
        _ => "重复提醒".to_string(),
    }
}

/// 提醒时间 `MM-DD HH:mm`（仅日期补 09:00；无法解析返回 `None`，整个元素不显示）
pub fn notify_text(todo: &Todo) -> Option<String> {
    let raw = todo.notify_at.as_deref()?;
    parse_local_datetime_with(raw, DefaultTime::Notify)
        .map(|dt| dt.format("%m-%d %H:%M").to_string())
}

/// JS `string.length`（UTF-16 码元数），子任务标题 tooltip 的阈值判断用
pub fn utf16_len(text: &str) -> usize {
    text.encode_utf16().count()
}

// ----------------------------------------------------------------------
// 象限
// ----------------------------------------------------------------------

/// 象限定义（`types/todo.ts QUADRANT_INFO`）
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct QuadrantInfo {
    pub id: i32,
    pub name: &'static str,
    /// `#RRGGBB`
    pub color: &'static str,
    /// 象限色 RGB（底色是它的 10%）
    pub rgb: (u8, u8, u8),
}

pub const QUADRANTS: [QuadrantInfo; 4] = [
    QuadrantInfo {
        id: 1,
        name: "重要且紧急",
        color: "#EF4444",
        rgb: (239, 68, 68),
    },
    QuadrantInfo {
        id: 2,
        name: "重要不紧急",
        color: "#F59E0B",
        rgb: (245, 158, 11),
    },
    QuadrantInfo {
        id: 3,
        name: "紧急不重要",
        color: "#3B82F6",
        rgb: (59, 130, 246),
    },
    QuadrantInfo {
        id: 4,
        name: "不紧急不重要",
        color: "#10B981",
        rgb: (16, 185, 129),
    },
];

/// 新建待办的默认颜色
pub const DEFAULT_COLOR: &str = "#10B981";

/// 象限默认色（`getQuadrantColor`）
pub fn quadrant_color(quadrant: i32) -> &'static str {
    QUADRANTS
        .iter()
        .find(|q| q.id == quadrant)
        .map_or(DEFAULT_COLOR, |q| q.color)
}

/// HEX 颜色比较（`isSameColor`：去空白、忽略大小写）
pub fn is_same_color(a: &str, b: &str) -> bool {
    a.trim().eq_ignore_ascii_case(b.trim())
}

/// 切换象限时的颜色（`resolveQuadrantColor`）：仍是旧象限默认色 → 跟随新象限；用户自选色保留
pub fn resolve_quadrant_color(current: &str, prev: i32, next: i32) -> String {
    if is_same_color(current, quadrant_color(prev)) {
        quadrant_color(next).to_string()
    } else {
        current.to_string()
    }
}

/// 把某个象限的新相对顺序合并回全局顺序（`mergeQuadrantOrder`）
pub fn merge_quadrant_order(global: &[i64], quadrant: &[i64]) -> Vec<i64> {
    let mut queue: std::collections::VecDeque<i64> = quadrant.iter().copied().collect();
    let mut merged: Vec<i64> = global
        .iter()
        .map(|id| {
            if quadrant.contains(id) {
                queue.pop_front().unwrap_or(*id)
            } else {
                *id
            }
        })
        .collect();
    merged.extend(queue);
    merged
}

/// 非法象限归 1，返回下标 0..4
pub fn quadrant_index(quadrant: i32) -> usize {
    match quadrant {
        q @ 1..=4 => (q - 1) as usize,
        _ => 0,
    }
}

// ----------------------------------------------------------------------
// 拖拽排序
// ----------------------------------------------------------------------

/// 拖拽中占位应在的位置：`others_mid_y` 是除被拖项之外各行（按当前显示顺序）的垂直中点，
/// 返回中点在指针之上的行数。被拖项插到这个下标后，各行位置的变化不会让结果再变（不抖动）
pub fn insertion_index(others_mid_y: &[f32], pointer_y: f32) -> usize {
    others_mid_y.iter().filter(|mid| **mid < pointer_y).count()
}

/// 把 `id` 移到 `index`（`index` 按去掉 `id` 之后的列表计）；不在列表里时插入
pub fn move_to(order: &mut Vec<i64>, id: i64, index: usize) {
    order.retain(|x| *x != id);
    let index = index.min(order.len());
    order.insert(index, id);
}

// ----------------------------------------------------------------------
// 日历
// ----------------------------------------------------------------------

/// 一个日格
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CalendarCell {
    pub date: NaiveDate,
    pub is_current_month: bool,
    /// 只对本月格判断
    pub is_today: bool,
}

/// 某月的 42 格（周一开头，前补上月尾、后补下月头）；`month` 为 1~12
pub fn month_cells(year: i32, month: u32, today: NaiveDate) -> Vec<CalendarCell> {
    let Some(first) = NaiveDate::from_ymd_opt(year, month, 1) else {
        return Vec::new();
    };
    let offset = first.weekday().num_days_from_monday() as i64;
    let start = first - Duration::days(offset);
    (0..42)
        .map(|i| {
            let date = start + Duration::days(i);
            let is_current_month = date.year() == year && date.month() == month;
            CalendarCell {
                date,
                is_current_month,
                is_today: is_current_month && date == today,
            }
        })
        .collect()
}

/// 上一月 / 下一月（`month` 1~12）
pub fn shift_month(year: i32, month: u32, delta: i32) -> (i32, u32) {
    let index = year * 12 + month as i32 - 1 + delta;
    (index.div_euclid(12), (index.rem_euclid(12) + 1) as u32)
}

/// 标题栏月份文本 `2026年10月`
pub fn month_text(year: i32, month: u32) -> String {
    format!("{year}年{month}月")
}

/// 节假日要加载的年份：当年；1 月加上一年，12 月加下一年
pub fn holiday_years(year: i32, month: u32) -> Vec<i32> {
    let mut years = vec![year];
    if month == 1 {
        years.push(year - 1);
    }
    if month == 12 {
        years.push(year + 1);
    }
    years
}

/// 待办在日历上的开始日期：可解析的 `startTime`，否则 `createdAt`
pub fn todo_start_date(todo: &Todo) -> Option<NaiveDate> {
    todo.start_time
        .as_deref()
        .filter(|s| !s.is_empty())
        .and_then(|s| parse_local_datetime_with(s, DefaultTime::StartOfDay))
        .or_else(|| parse_local_datetime_with(&todo.created_at, DefaultTime::StartOfDay))
        .map(|dt| dt.date())
}

/// 截止日期：可解析的 `endTime`，否则等于开始日期
pub fn todo_end_date(todo: &Todo) -> Option<NaiveDate> {
    todo.end_time
        .as_deref()
        .filter(|s| !s.is_empty())
        .and_then(|s| parse_local_datetime_with(s, DefaultTime::EndOfDay))
        .map(|dt| dt.date())
}

/// 一行里的一段跨天条
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TodoBar {
    pub todo_id: i64,
    pub row: usize,
    pub start_col: usize,
    pub end_col: usize,
    /// 本段是开始段（只有开始段显示标题）
    pub is_start: bool,
    pub is_end: bool,
    pub lane: usize,
    start_key: NaiveDate,
    end_key: NaiveDate,
}

/// 计算跨天条（`todoBarsByRow` + `assignLanes`）。返回顺序同旧模板：按行，行内按开始 / 截止日期排序
pub fn calendar_bars(cells: &[CalendarCell], todos: &[Todo]) -> Vec<TodoBar> {
    let (Some(first), Some(last)) = (cells.first(), cells.get(41)) else {
        return Vec::new();
    };
    let (first, last) = (first.date, last.date);
    let mut rows: Vec<Vec<TodoBar>> = vec![Vec::new(); 6];
    for todo in todos {
        let Some(start) = todo_start_date(todo) else {
            continue;
        };
        let end = todo_end_date(todo).unwrap_or(start);
        if end < first || start > last {
            continue;
        }
        let find = |d: NaiveDate| cells.iter().position(|c| c.date == d);
        let start_index = find(start).or((start < first).then_some(0));
        let end_index = find(end).or((end > last).then_some(41));
        let (Some(si), Some(ei)) = (start_index, end_index) else {
            continue;
        };
        let (start_row, start_col) = (si / 7, si % 7);
        let (end_row, end_col) = (ei / 7, ei % 7);
        // 截止早于开始且跨行的不渲染；同行时宽度为负，渲染时跳过
        if start_row > end_row {
            continue;
        }
        for (row, bars) in rows
            .iter_mut()
            .enumerate()
            .take(end_row + 1)
            .skip(start_row)
        {
            bars.push(TodoBar {
                todo_id: todo.id,
                row,
                start_col: if row == start_row { start_col } else { 0 },
                end_col: if row == end_row { end_col } else { 6 },
                is_start: row == start_row && start >= first,
                is_end: row == end_row && end <= last,
                lane: 0,
                start_key: start,
                end_key: end,
            });
        }
    }
    let mut out = Vec::new();
    for mut bars in rows {
        assign_lanes(&mut bars);
        out.extend(bars);
    }
    out
}

fn overlaps(a: &TodoBar, b: &TodoBar) -> bool {
    !(a.end_col < b.start_col || b.end_col < a.start_col)
}

/// 泳道分配：按开始、截止日期排序，依次取不与已分配段重叠的最小泳道
fn assign_lanes(bars: &mut [TodoBar]) {
    bars.sort_by(|a, b| {
        a.start_key
            .cmp(&b.start_key)
            .then_with(|| a.end_key.cmp(&b.end_key))
    });
    let mut assigned: Vec<Option<usize>> = vec![None; bars.len()];
    for i in 0..bars.len() {
        let used: Vec<usize> = (0..bars.len())
            .filter(|&j| j != i)
            .filter_map(|j| assigned[j].filter(|_| overlaps(&bars[i], &bars[j])))
            .collect();
        let mut lane = 0;
        while used.contains(&lane) {
            lane += 1;
        }
        assigned[i] = Some(lane);
        bars[i].lane = lane;
    }
}

/// 打开日历时窗口的新宽度（逻辑像素；旧 MainView 公式，近似值 44 / 30 照抄）
pub fn calendar_window_width(logical_width: f32, logical_height: f32) -> f32 {
    const TITLE_BAR_H: f32 = 44.;
    const WEEKDAY_H: f32 = 30.;
    const PANEL_PADDING: f32 = 24.;
    let grid_h = logical_height - TITLE_BAR_H - WEEKDAY_H - PANEL_PADDING;
    let cell_h = grid_h / 6.;
    let grid_w = cell_h * 7.;
    (logical_width + grid_w + PANEL_PADDING).round()
}

// ----------------------------------------------------------------------
// 版本
// ----------------------------------------------------------------------

/// `compareVersions`：去掉前缀 `v`，按 `.` 逐段数值比较（非数字段按 0）
pub fn compare_versions(a: &str, b: &str) -> Ordering {
    let parts = |v: &str| -> Vec<i64> {
        v.strip_prefix('v')
            .unwrap_or(v)
            .split('.')
            .map(|p| p.trim().parse::<i64>().unwrap_or(0))
            .collect()
    };
    let (pa, pb) = (parts(a), parts(b));
    for i in 0..pa.len().max(pb.len()) {
        let (x, y) = (
            pa.get(i).copied().unwrap_or(0),
            pb.get(i).copied().unwrap_or(0),
        );
        match x.cmp(&y) {
            Ordering::Equal => {}
            other => return other,
        }
    }
    Ordering::Equal
}

#[cfg(test)]
mod tests {
    use super::*;

    fn todo(id: i64) -> Todo {
        Todo {
            id,
            title: format!("t{id}"),
            description: None,
            color: "#EF4444".into(),
            quadrant: 1,
            notify_at: None,
            notify_before: 0,
            notified: false,
            completed: false,
            sort_order: 0,
            start_time: None,
            end_time: None,
            created_at: "2026-10-01 08:00:00".into(),
            updated_at: String::new(),
            repeat_enabled: false,
            repeat_type: None,
            repeat_interval: 1,
            repeat_weekdays: None,
            repeat_month_day: None,
            subtasks: vec![],
        }
    }

    fn repeat(kind: &str, interval: i32, weekdays: Option<&str>, day: Option<i32>) -> Todo {
        Todo {
            repeat_enabled: true,
            repeat_type: Some(kind.into()),
            repeat_interval: interval,
            repeat_weekdays: weekdays.map(Into::into),
            repeat_month_day: day,
            ..todo(1)
        }
    }

    #[test]
    fn repeat_tooltip_matches_frontend() {
        assert_eq!(repeat_tooltip(&repeat("daily", 1, None, None)), "每天重复");
        assert_eq!(
            repeat_tooltip(&repeat("daily", 3, None, None)),
            "每 3 天重复"
        );
        assert_eq!(repeat_tooltip(&repeat("daily", 0, None, None)), "每天重复");
        assert_eq!(
            repeat_tooltip(&repeat("weekly", 1, Some("1,3,5"), None)),
            "每周 周一、三、五"
        );
        assert_eq!(repeat_tooltip(&repeat("weekly", 1, None, None)), "每周");
        assert_eq!(
            repeat_tooltip(&repeat("weekly", 2, Some("7,x"), None)),
            "每 2 周 周日、x"
        );
        assert_eq!(
            repeat_tooltip(&repeat("weekly", 2, Some(""), None)),
            "每 2 周"
        );
        assert_eq!(
            repeat_tooltip(&repeat("monthly", 1, None, Some(25))),
            "每月 25 号"
        );
        assert_eq!(
            repeat_tooltip(&repeat("monthly", 3, None, None)),
            "每 3 月 1 号"
        );
        assert_eq!(repeat_tooltip(&repeat("yearly", 1, None, None)), "重复提醒");
        let mut off = repeat("daily", 1, None, None);
        off.repeat_enabled = false;
        assert_eq!(repeat_tooltip(&off), "");
        let mut no_type = repeat("daily", 1, None, None);
        no_type.repeat_type = None;
        assert_eq!(repeat_tooltip(&no_type), "");
    }

    #[test]
    fn notify_text_formats_and_skips_garbage() {
        let mut t = todo(1);
        t.notify_at = Some("2026-10-15 17:30:00".into());
        assert_eq!(notify_text(&t).as_deref(), Some("10-15 17:30"));
        t.notify_at = Some("2026-10-16".into());
        assert_eq!(notify_text(&t).as_deref(), Some("10-16 09:00"));
        t.notify_at = Some("2026-10-16T06:30".into());
        assert_eq!(notify_text(&t).as_deref(), Some("10-16 06:30"));
        t.notify_at = Some("garbage".into());
        assert_eq!(notify_text(&t), None);
        t.notify_at = None;
        assert_eq!(notify_text(&t), None);
        assert_eq!(utf16_len("预约评审会议室并发出日历邀请给所有相关同事"), 21);
        assert_eq!(utf16_len("a😀"), 3);
    }

    #[test]
    fn quadrant_color_resolution_and_merge() {
        assert_eq!(resolve_quadrant_color("#ef4444", 1, 3), "#3B82F6");
        assert_eq!(resolve_quadrant_color(" #EF4444 ", 1, 2), "#F59E0B");
        assert_eq!(resolve_quadrant_color("#6366F1", 2, 4), "#6366F1");
        assert_eq!(quadrant_color(9), DEFAULT_COLOR);
        assert_eq!(
            merge_quadrant_order(&[1, 2, 3, 4, 5], &[5, 3]),
            vec![1, 2, 5, 4, 3]
        );
        assert_eq!(merge_quadrant_order(&[1, 2], &[2, 9]), vec![1, 2, 9]);
        assert_eq!(quadrant_index(0), 0);
        assert_eq!(quadrant_index(4), 3);
    }

    #[test]
    fn drag_insertion_is_stable() {
        // 三行（除被拖项外），中点 20 / 60 / 100
        assert_eq!(insertion_index(&[20., 60., 100.], 10.), 0);
        assert_eq!(insertion_index(&[20., 60., 100.], 61.), 2);
        assert_eq!(insertion_index(&[20., 60., 100.], 500.), 3);
        let mut order = vec![1, 2, 3, 4];
        move_to(&mut order, 1, 2);
        assert_eq!(order, vec![2, 3, 1, 4]);
        move_to(&mut order, 4, 0);
        assert_eq!(order, vec![4, 2, 3, 1]);
        move_to(&mut order, 9, 99);
        assert_eq!(order, vec![4, 2, 3, 1, 9]);
    }

    #[test]
    fn month_grid_starts_on_monday() {
        let today = NaiveDate::from_ymd_opt(2026, 10, 15).unwrap();
        let cells = month_cells(2026, 10, today);
        assert_eq!(cells.len(), 42);
        assert_eq!(cells[0].date, NaiveDate::from_ymd_opt(2026, 9, 28).unwrap());
        assert!(!cells[0].is_current_month);
        assert_eq!(cells[3].date.day(), 1);
        assert!(cells[3].is_current_month);
        assert!(cells[17].is_today);
        assert_eq!(
            cells[41].date,
            NaiveDate::from_ymd_opt(2026, 11, 8).unwrap()
        );
        assert_eq!(cells.iter().filter(|c| c.is_today).count(), 1);
        // 2027-02 从周一开始：第一格就是 1 号
        let feb = month_cells(2027, 2, today);
        assert_eq!(feb[0].date.day(), 1);
        assert_eq!(shift_month(2026, 1, -1), (2025, 12));
        assert_eq!(shift_month(2026, 12, 1), (2027, 1));
        assert_eq!(month_text(2026, 10), "2026年10月");
        assert_eq!(holiday_years(2026, 1), vec![2026, 2025]);
        assert_eq!(holiday_years(2026, 12), vec![2026, 2027]);
        assert_eq!(holiday_years(2026, 6), vec![2026]);
    }

    #[test]
    fn calendar_bars_split_rows_and_lanes() {
        let today = NaiveDate::from_ymd_opt(2026, 10, 15).unwrap();
        let cells = month_cells(2026, 10, today);
        let mut a = todo(1);
        a.start_time = Some("2026-10-13 09:00:00".into());
        a.end_time = Some("2026-10-15 18:00:00".into());
        let mut b = todo(2);
        b.start_time = Some("2026-10-16".into());
        b.end_time = Some("2026-10-20".into());
        let mut c = todo(3);
        c.created_at = "2026-10-13 10:00:00".into();
        let mut out_of_range = todo(4);
        out_of_range.start_time = Some("2026-12-01".into());
        let mut long = todo(5);
        long.start_time = Some("2026-08-01".into());
        long.end_time = Some("2026-09-29".into());
        let bars = calendar_bars(&cells, &[a, b, c, out_of_range, long]);
        let summary: Vec<(i64, usize, usize, usize, bool, usize)> = bars
            .iter()
            .map(|b| (b.todo_id, b.row, b.start_col, b.end_col, b.is_start, b.lane))
            .collect();
        assert_eq!(
            summary,
            vec![
                // 第 0 行：从网格之前开始的长条，开始段不在范围内 → 不显示标题
                (5, 0, 0, 1, false, 0),
                // 第 2 行（10-12 ~ 10-18）：c 13（截止更早，排前面）、a 13~15（与 c 重叠 → 泳道 1）、b 16 起到行末
                (3, 2, 1, 1, true, 0),
                (1, 2, 1, 3, true, 1),
                (2, 2, 4, 6, true, 0),
                // 第 3 行：b 的续段
                (2, 3, 0, 1, false, 0),
            ]
        );
        assert!(bars[4].is_end);
    }

    #[test]
    fn calendar_width_formula() {
        assert_eq!(calendar_window_width(380., 600.), 990.);
    }

    #[test]
    fn versions_compare_numerically() {
        assert_eq!(compare_versions("v2.6.0", "2.5.0"), Ordering::Greater);
        assert_eq!(compare_versions("v2.5", "2.5.0"), Ordering::Equal);
        assert_eq!(compare_versions("2.10.0", "2.9.9"), Ordering::Greater);
        assert_eq!(compare_versions("v2.4.9", "2.5.0"), Ordering::Less);
        assert_eq!(compare_versions("vX", "0"), Ordering::Equal);
    }
}
