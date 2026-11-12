//! 日历窗口的纯逻辑（无 GPUI）：日历网格与跨天条（逐字移植自旧前端 `CalendarView.vue`）、日历窗口第一次显示时的位置。
//! 都带单测。

use chrono::{Datelike, Duration, NaiveDate, NaiveTime};
use mini_todo_core::db::Todo;
use mini_todo_core::db::time::{DefaultTime, parse_local_datetime_with};

use crate::platform::PhysRect;
use crate::util::datetime::format_storage_date_time;
use crate::windows::block::DEFAULT_LIST_ID;
use crate::windows::drag::js_round;

// ----------------------------------------------------------------------
// 日历网格与跨天条
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
pub fn calendar_bars(cells: &[CalendarCell], todos: &[&Todo]) -> Vec<TodoBar> {
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

// ----------------------------------------------------------------------
// 双击某天新建待办
// ----------------------------------------------------------------------

/// 在日历上双击某天新建的待办建到哪个清单：清单筛选（`None` = 全部）**恰好只选了一个**、且它还在 → 它；
/// 其余（全部、选了多个、选的清单已不在）→ 默认清单
pub fn new_todo_list_id(filter: Option<&[i64]>, exists: impl Fn(i64) -> bool) -> i64 {
    match filter {
        Some([only]) if exists(*only) => *only,
        _ => DEFAULT_LIST_ID,
    }
}

/// 在日历上双击某天新建的待办的开始时间：那天 00:00（规范存储格式）。日历按开始时间把待办放进格子
/// （[`todo_start_date`]），保存后就出现在那天
pub fn new_todo_start_time(date: NaiveDate) -> String {
    format_storage_date_time(&date.and_time(NaiveTime::default()))
}

// ----------------------------------------------------------------------
// 日历窗口的位置
// ----------------------------------------------------------------------

/// 日历窗口的最小客户区（逻辑像素）：6 行日格各至少 60 高 + 星期头 + 标题栏 + 内边距放得下
pub const MIN_WIDTH: f32 = 420.;
pub const MIN_HEIGHT: f32 = 480.;

/// 日历窗口第一次显示时的宽度（逻辑像素）：按高度让日格大致是方的（以前打开日历时主窗口加宽的公式，
/// 换成独立窗口自己的标题栏 52、星期头 34、内边距 24），不小于 [`MIN_WIDTH`]
pub fn default_width(height: f64) -> f64 {
    const TITLE_BAR: f64 = 52.;
    const WEEKDAYS: f64 = 34.;
    const PADDING: f64 = 24.;
    let cell = (height - TITLE_BAR - WEEKDAYS - PADDING) / 6.;
    (cell * 7. + PADDING).round().max(f64::from(MIN_WIDTH))
}

/// 日历窗口第一次显示、当前屏幕组合下没有布局时的客户区（物理像素，PRD R4）：贴在默认清单窗口（`list`）右侧、与它等高，
/// 宽度见 [`default_width`]；右边放不下就放到左侧，两边都放不下就夹回 `work`（默认清单窗口所在显示器的工作区）
pub fn first_rect(list: PhysRect, work: PhysRect, scale: f64) -> PhysRect {
    let scale = if scale.is_finite() && scale > 0. {
        scale
    } else {
        1.
    };
    let height = list.height.max(js_round(f64::from(MIN_HEIGHT) * scale));
    let width = js_round(default_width(f64::from(height) / scale) * scale);
    let right = list.right();
    let left = list.x - width;
    let x = if right + width <= work.right() {
        right
    } else if left >= work.x {
        left
    } else {
        // 两边都放不下：尽量靠右、不出工作区（窗口比工作区还宽时贴左边）
        (work.right() - width).max(work.x)
    };
    PhysRect::new(x, list.y, width, height)
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
            list_id: Some(1),
            list_updated_at: None,
            subtasks: vec![],
        }
    }

    #[test]
    fn new_todo_goes_to_the_only_selected_list() {
        let exists = |ids: &'static [i64]| move |id| ids.contains(&id);
        // 全部清单：默认清单
        assert_eq!(new_todo_list_id(None, exists(&[1, 7, 8])), DEFAULT_LIST_ID);
        // 恰好选了一个：它
        assert_eq!(new_todo_list_id(Some(&[7]), exists(&[1, 7, 8])), 7);
        assert_eq!(new_todo_list_id(Some(&[1]), exists(&[1, 7])), 1);
        // 选了多个：默认清单（哪怕默认清单不在里面）
        assert_eq!(
            new_todo_list_id(Some(&[7, 8]), exists(&[1, 7, 8])),
            DEFAULT_LIST_ID
        );
        // 选的那个清单已不在（刚被删除、内存副本晚一步）：默认清单
        assert_eq!(
            new_todo_list_id(Some(&[9]), exists(&[1, 7])),
            DEFAULT_LIST_ID
        );
        // 空数组（存储里不会出现，按"没有唯一的清单"处理）：默认清单
        assert_eq!(new_todo_list_id(Some(&[]), exists(&[1])), DEFAULT_LIST_ID);
    }

    #[test]
    fn new_todo_starts_at_midnight_of_the_clicked_day() {
        let day = NaiveDate::from_ymd_opt(2026, 11, 12).unwrap();
        assert_eq!(new_todo_start_time(day), "2026-11-12 00:00:00");
        // 灰色的上 / 下月格子按那天算；保存后日历把它放回这一天
        let prev = NaiveDate::from_ymd_opt(2026, 10, 26).unwrap();
        let mut t = todo(1);
        t.start_time = Some(new_todo_start_time(prev));
        assert_eq!(todo_start_date(&t), Some(prev));
        let cells = month_cells(2026, 11, day);
        let bars = calendar_bars(&cells, &[&t]);
        assert_eq!(bars.len(), 1);
        assert_eq!(cells[bars[0].row * 7 + bars[0].start_col].date, prev);
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
        let bars = calendar_bars(&cells, &[&a, &b, &c, &out_of_range, &long]);
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
    fn default_width_keeps_the_cells_roughly_square() {
        // 600 高：(600 - 110) / 6 × 7 + 24 ≈ 596
        assert_eq!(default_width(600.), 596.);
        assert_eq!(default_width(800.), 829.);
        assert_eq!(default_width(400.), f64::from(MIN_WIDTH), "不小于最小宽度");
    }

    #[test]
    fn the_calendar_first_shows_next_to_the_default_list() {
        let work = PhysRect::new(0, 0, 1920, 1040);
        // 默认清单窗口在左边：日历贴它右侧、等高
        let list = PhysRect::new(192, 104, 380, 600);
        assert_eq!(
            first_rect(list, work, 1.),
            PhysRect::new(572, 104, 596, 600)
        );
        // 右边放不下：放左侧
        let list = PhysRect::new(1400, 104, 380, 600);
        assert_eq!(
            first_rect(list, work, 1.),
            PhysRect::new(804, 104, 596, 600)
        );
        // 两边都放不下（清单窗口很宽）：靠右夹回工作区
        let list = PhysRect::new(300, 0, 1300, 1000);
        let rect = first_rect(list, work, 1.);
        assert_eq!(rect.height, 1000);
        assert_eq!(rect.right(), work.right());
        // 150%：宽高都按物理像素（600 逻辑高 = 900 物理）
        let work = PhysRect::new(0, 0, 2880, 1560);
        let list = PhysRect::new(288, 156, 570, 900);
        assert_eq!(
            first_rect(list, work, 1.5),
            PhysRect::new(858, 156, 894, 900)
        );
        // 清单窗口比日历的最小高度还矮：日历按最小高度
        let list = PhysRect::new(0, 0, 320, 400);
        assert_eq!(
            first_rect(list, PhysRect::new(0, 0, 1920, 1040), 1.).height,
            480
        );
    }
}
