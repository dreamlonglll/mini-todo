//! 日历视图（旧 `CalendarView.vue`，规格 §9）：星期头 + 6×7 日格（农历 / 节气 / 节日、休 / 班角标、今天圆点）
//! + 覆盖在网格上的跨天待办条（全部待办，含已完成；按行拆段、泳道避让、悬停联动、点击打开详情）。
//!
//! 节假日后台加载（每年最多 3 次重试，结果为空时 5s / 10s / 15s 后重载，切月作废旧加载）。

use std::collections::HashMap;
use std::time::Duration;

use chrono::{Datelike, Local, NaiveDate, Timelike};
use gpui_kit::{
    Context, InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, Styled as _, Task, Window, div, prelude::FluentBuilder as _,
    px, relative,
};
use mini_todo_core::lunar::{LunarTextKind, lunar_display_text};

use super::logic::{self, CalendarCell, TodoBar};
use super::{MainWindow, RenderCtx};
use crate::theme::{self, main_window as dims};

/// 节假日每年最多尝试 3 次，间隔 2s × 次数
const HOLIDAY_ATTEMPTS: u32 = 3;
const HOLIDAY_RETRY_BASE: Duration = Duration::from_millis(2000);
/// 合并结果为空时延迟 5s × (n+1) 重载，最多 3 次
const HOLIDAY_RELOAD_BASE: Duration = Duration::from_millis(5000);
const HOLIDAY_MAX_RELOADS: u32 = 3;

/// 日格 + 农历文字
#[derive(Clone)]
struct CellView {
    cell: CalendarCell,
    lunar: SharedString,
    kind: LunarTextKind,
}

pub(crate) struct CalendarState {
    pub year: i32,
    /// 1~12
    pub month: u32,
    pub today: NaiveDate,
    /// 日期 → 是否放假（true 休 / false 班）
    holidays: HashMap<NaiveDate, bool>,
    /// 按年的内存缓存（只缓存成功且非空的结果）
    cache: HashMap<i32, HashMap<NaiveDate, bool>>,
    load_seq: u64,
    loaded_for: Option<(i32, u32)>,
    retry: Option<Task<()>>,
    pub hovered_todo: Option<i64>,
    cells: Option<((i32, u32, NaiveDate), Vec<CellView>)>,
}

fn local_today() -> NaiveDate {
    Local::now().date_naive()
}

impl CalendarState {
    pub fn new() -> Self {
        let today = local_today();
        Self {
            year: today.year(),
            month: today.month(),
            today,
            holidays: HashMap::new(),
            cache: HashMap::new(),
            load_seq: 0,
            loaded_for: None,
            retry: None,
            hovered_todo: None,
            cells: None,
        }
    }

    /// 午夜 + 1s 刷新"今天"并重新排程（系统休眠导致的延后由窗口聚焦刷新兜底）
    pub fn start(&mut self, cx: &mut Context<MainWindow>) {
        cx.spawn(async move |this, cx| {
            loop {
                let now = Local::now();
                let secs_left = 86_400 - now.num_seconds_from_midnight() as u64;
                cx.background_executor()
                    .timer(Duration::from_secs(secs_left + 1))
                    .await;
                if this
                    .update(cx, |this, cx| this.calendar.refresh_today(cx))
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
    }

    pub fn refresh_today(&mut self, cx: &mut Context<MainWindow>) {
        let today = local_today();
        if today != self.today {
            self.today = today;
            cx.notify();
        }
    }

    pub fn month_text(&self) -> String {
        logic::month_text(self.year, self.month)
    }

    pub fn prev_month(&mut self, _cx: &mut Context<MainWindow>) {
        (self.year, self.month) = logic::shift_month(self.year, self.month, -1);
    }

    pub fn next_month(&mut self, _cx: &mut Context<MainWindow>) {
        (self.year, self.month) = logic::shift_month(self.year, self.month, 1);
    }

    pub fn go_to_today(&mut self, cx: &mut Context<MainWindow>) {
        self.refresh_today(cx);
        self.year = self.today.year();
        self.month = self.today.month();
    }

    fn cells(&mut self) -> Vec<CellView> {
        let key = (self.year, self.month, self.today);
        if let Some((cached_key, cells)) = &self.cells
            && *cached_key == key
        {
            return cells.clone();
        }
        let cells: Vec<CellView> = logic::month_cells(self.year, self.month, self.today)
            .into_iter()
            .map(|cell| {
                let (text, kind) = lunar_display_text(cell.date);
                CellView {
                    cell,
                    lunar: text.into(),
                    kind,
                }
            })
            .collect();
        self.cells = Some((key, cells.clone()));
        cells
    }
}

impl MainWindow {
    /// 年月变化时重新加载节假日（作废进行中的旧加载与重试）
    fn ensure_holidays(&mut self, cx: &mut Context<Self>) {
        let key = (self.calendar.year, self.calendar.month);
        if self.calendar.loaded_for == Some(key) {
            return;
        }
        self.calendar.loaded_for = Some(key);
        self.load_holidays(0, cx);
    }

    fn load_holidays(&mut self, reload_count: u32, cx: &mut Context<Self>) {
        let calendar = &mut self.calendar;
        calendar.retry = None;
        calendar.load_seq += 1;
        let seq = calendar.load_seq;
        let years = logic::holiday_years(calendar.year, calendar.month);
        cx.spawn(async move |this, cx| {
            let mut all: HashMap<NaiveDate, bool> = HashMap::new();
            for year in years {
                let cached = this
                    .update(cx, |this, _| this.calendar.cache.get(&year).cloned())
                    .ok()
                    .flatten();
                let map = match cached {
                    Some(map) => map,
                    None => {
                        let mut map = HashMap::new();
                        for attempt in 1..=HOLIDAY_ATTEMPTS {
                            let result = cx
                                .background_executor()
                                .spawn(async move { mini_todo_core::holiday::fetch(year) })
                                .await;
                            match result {
                                Ok(list) => {
                                    map = list
                                        .into_iter()
                                        .filter_map(|h| {
                                            NaiveDate::parse_from_str(&h.date, "%Y-%m-%d")
                                                .ok()
                                                .map(|d| (d, h.is_off_day))
                                        })
                                        .collect();
                                    break;
                                }
                                Err(e) => {
                                    log::warn!(
                                        "[holiday] 第 {attempt}/{HOLIDAY_ATTEMPTS} 次获取 {year} 年节假日失败: {e}"
                                    );
                                    if attempt < HOLIDAY_ATTEMPTS {
                                        cx.background_executor()
                                            .timer(HOLIDAY_RETRY_BASE * attempt)
                                            .await;
                                    }
                                }
                            }
                        }
                        if !map.is_empty() {
                            let cached = map.clone();
                            let _ = this.update(cx, |this, _| {
                                this.calendar.cache.insert(year, cached);
                            });
                        }
                        map
                    }
                };
                let stale = this
                    .update(cx, |this, _| this.calendar.load_seq != seq)
                    .unwrap_or(true);
                if stale {
                    return;
                }
                all.extend(map);
            }
            let _ = this.update(cx, |this, cx| {
                if this.calendar.load_seq != seq {
                    return;
                }
                let empty = all.is_empty();
                this.calendar.holidays = all;
                cx.notify();
                if empty && reload_count < HOLIDAY_MAX_RELOADS {
                    log::warn!("[holiday] 节假日数据为空，稍后重新加载");
                    this.calendar.retry = Some(cx.spawn(async move |this, cx| {
                        cx.background_executor()
                            .timer(HOLIDAY_RELOAD_BASE * (reload_count + 1))
                            .await;
                        let _ = this.update(cx, |this, cx| {
                            if this.calendar.load_seq == seq {
                                this.load_holidays(reload_count + 1, cx);
                            }
                        });
                    }));
                }
            });
        })
        .detach();
    }

    pub(super) fn render_calendar(
        &mut self,
        rc: &RenderCtx,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        self.ensure_holidays(cx);
        let palette = rc.palette;
        let line = rc.colors.calendar_line;
        let cells = self.calendar.cells();
        let bars = {
            let state = self.state.read(cx);
            let plain: Vec<CalendarCell> = cells.iter().map(|c| c.cell).collect();
            logic::calendar_bars(&plain, &state.todos)
        };
        let todo_info: HashMap<i64, (SharedString, gpui_kit::Hsla, bool)> = {
            let state = self.state.read(cx);
            state
                .todos
                .iter()
                .map(|t| {
                    (
                        t.id,
                        (
                            SharedString::from(t.title.clone()),
                            theme::todo_color(&t.color, palette.primary),
                            t.completed,
                        ),
                    )
                })
                .collect()
        };

        let weekdays = div().flex().flex_none().children(
            ["一", "二", "三", "四", "五", "六", "日"]
                .into_iter()
                .map(|name| {
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .justify_center()
                        .px(px(4.))
                        .py(px(8.))
                        .text_size(px(12.))
                        .line_height(relative(theme::LINE_HEIGHT))
                        .font_weight(theme::WEIGHT_MEDIUM)
                        .text_color(palette.text_secondary)
                        .child(name)
                }),
        );

        let rows = (0..6).map(|row| {
            div()
                .flex_1()
                .min_h(px(60.))
                .flex()
                .children((0..7).map(|col| {
                    let view = &cells[row * 7 + col];
                    self.render_calendar_cell(view, row, col, rc, line)
                }))
        });

        let mut bar_elements = Vec::with_capacity(bars.len());
        for bar in bars.iter().filter(|bar| bar.end_col >= bar.start_col) {
            if let Some((title, color, completed)) = todo_info.get(&bar.todo_id).cloned() {
                bar_elements.push(self.render_bar(bar, title, color, completed, cx));
            }
        }

        div()
            .flex()
            .flex_col()
            .size_full()
            .rounded(theme::radius::MD)
            .overflow_hidden()
            .child(weekdays)
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .border_t_1()
                    .border_color(line)
                    .overflow_hidden()
                    .children(rows)
                    .children(bar_elements),
            )
    }

    fn render_calendar_cell(
        &self,
        view: &CellView,
        row: usize,
        col: usize,
        rc: &RenderCtx,
        line: gpui_kit::Hsla,
    ) -> impl IntoElement {
        let palette = rc.palette;
        let cell = view.cell;
        let faded = |el: gpui_kit::Div| {
            if cell.is_current_month {
                el
            } else {
                el.opacity(0.4)
            }
        };
        let date = if cell.is_today {
            div()
                .size(px(20.))
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .rounded_full()
                .bg(palette.primary)
                .text_color(theme::hex(0xFFFFFF))
                .text_size(px(13.))
                .line_height(relative(1.4))
                .font_weight(theme::WEIGHT_MEDIUM)
                .child(cell.date.day().to_string())
        } else {
            faded(
                div()
                    .flex_none()
                    .text_size(px(13.))
                    .line_height(relative(1.4))
                    .font_weight(theme::WEIGHT_MEDIUM)
                    .text_color(palette.text_primary)
                    .child(cell.date.day().to_string()),
            )
        };
        let badge = self.calendar.holidays.get(&cell.date).map(|off| {
            let (text, bg) = if *off {
                ("休", theme::hex(0xEF4444))
            } else {
                ("班", theme::hex(0xF59E0B))
            };
            faded(
                div()
                    .flex_none()
                    .px(px(2.))
                    .py(px(1.))
                    .rounded(px(2.))
                    .bg(bg)
                    .text_color(theme::hex(0xFFFFFF))
                    .text_size(px(9.))
                    .line_height(px(9.))
                    .font_weight(theme::WEIGHT_SEMIBOLD)
                    .child(text),
            )
        });
        let (lunar_color, lunar_weight) = match view.kind {
            LunarTextKind::Festival => (palette.error, theme::WEIGHT_MEDIUM),
            LunarTextKind::SolarTerm => (palette.success, theme::WEIGHT_MEDIUM),
            LunarTextKind::Lunar => (palette.text_tertiary, gpui_kit::FontWeight::NORMAL),
        };
        div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .px(px(4.))
            .py(px(3.))
            .when(col < 6, |this| this.border_r_1().border_color(line))
            .when(row < 5, |this| this.border_b_1().border_color(line))
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_between()
                    .gap(px(2.))
                    .child(
                        div()
                            .flex()
                            .flex_none()
                            .items_center()
                            .gap(px(2.))
                            .child(date)
                            .children(badge),
                    )
                    .child(faded(
                        div()
                            .min_w_0()
                            .flex()
                            .justify_end()
                            .text_size(px(10.))
                            .line_height(relative(1.4))
                            .font_weight(lunar_weight)
                            .text_color(lunar_color)
                            .child(div().min_w_0().truncate().child(view.lunar.clone())),
                    )),
            )
    }

    fn render_bar(
        &self,
        bar: &TodoBar,
        title: SharedString,
        color: gpui_kit::Hsla,
        completed: bool,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        let id = bar.todo_id;
        let hovered = self.calendar.hovered_todo == Some(id);
        let opacity = match (completed, hovered) {
            (false, false) => 1.,
            (false, true) => 0.85,
            (true, false) => 0.5,
            (true, true) => 0.65,
        };
        let span = (bar.end_col - bar.start_col + 1) as f32;
        let top = dims::BAR_TOP_OFFSET + bar.lane as f32 * (dims::BAR_HEIGHT + dims::BAR_GAP);
        div()
            .absolute()
            .left(relative(bar.start_col as f32 / 7.))
            .w(relative(span / 7.))
            .top(relative(bar.row as f32 / 6.))
            .child(
                div()
                    .id(SharedString::from(format!(
                        "calendar-bar-{}-{}-{}",
                        bar.row, bar.start_col, id
                    )))
                    .mx(px(2.))
                    .mt(px(top))
                    .h(px(dims::BAR_HEIGHT))
                    .px(px(6.))
                    .flex()
                    .items_center()
                    .rounded(theme::radius::SM)
                    .overflow_hidden()
                    .bg(color)
                    .opacity(opacity)
                    .cursor_pointer()
                    .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                        if *hovered {
                            this.calendar.hovered_todo = Some(id);
                        } else if this.calendar.hovered_todo == Some(id) {
                            this.calendar.hovered_todo = None;
                        }
                        cx.notify();
                    }))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        this.open_todo(id, window, cx);
                    }))
                    .when(bar.is_start, |this| {
                        let text = div()
                            .min_w_0()
                            .truncate()
                            .text_size(px(11.))
                            .text_color(theme::hex(0xFFFFFF))
                            .child(title);
                        this.child(if completed {
                            super::todo_item::strike(text, theme::hex(0xFFFFFF))
                        } else {
                            text
                        })
                    }),
            )
            .into_any_element()
    }
}
