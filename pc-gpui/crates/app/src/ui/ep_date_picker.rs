//! `el-date-picker type="date"`（Element Plus 2.13，未配置 locale → 英文面板）。
//!
//! 外观：`el-input`（高 32、前缀 Calendar 图标 `#A8ABB2`、可清空：悬停且有值时右侧 CircleClose）；可直接输入
//! `YYYY-MM-DD`（回车 / 失焦时提交，非法则还原）。面板宽 322（+边框）、`padding` 由头部 12px 与内容 15px 组成；
//! 头部 `«  ‹   2026  October   ›  »`（标签 16px，点年份 / 月份切到年 / 月表）；星期表头英文 Sun…Sat、12px、
//! 下边线 `#EBEEF5`；日期格 41.7×38，数字 12px 居中于 24px 圆：今天 `#409EFF` 加粗、选中 `#409EFF` 底白字、
//! 上 / 下月 `#A8ABB2`、悬停字 `#409EFF`。type=date 没有底部按钮，点日期即选中并关闭。
//!
//! 编辑窗口里所有日期 / 时间选择器 `placement: top-start`、不翻转（面板总在上方）。

use chrono::{Datelike, Local, NaiveDate};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::{
    AppContext as _, ClickEvent, Context, Entity, EventEmitter, InteractiveElement as _,
    IntoElement, MouseButton, ParentElement as _, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, Window, div,
    prelude::FluentBuilder as _, px,
};

use crate::assets::EpIcon;
use crate::theme::{self, ep};
use crate::ui::ep2_popper::{self, Anchor, Placement, PopperOptions};
use crate::ui::icon::icon;
use crate::ui::input::EpInput;

/// 值变化（`YYYY-MM-DD` / 清空为 `None`）
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EpDatePickerEvent {
    Change(Option<String>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PanelMode {
    Date,
    Month,
    Year,
}

pub const PANEL_WIDTH: f32 = 324.;
pub const PANEL_HEIGHT: f32 = 343.;

const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];
const MONTHS_SHORT: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];
const WEEKDAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

pub struct EpDatePicker {
    id: SharedString,
    input: Entity<InputState>,
    value: Option<NaiveDate>,
    open: bool,
    mode: PanelMode,
    view_year: i32,
    view_month: u32,
    disabled: bool,
    anchor: Anchor,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<EpDatePickerEvent> for EpDatePicker {}

/// `YYYY-MM-DD`（严格两位月日）
pub fn parse_date(text: &str) -> Option<NaiveDate> {
    let text = text.trim();
    if text.len() != 10 {
        return None;
    }
    NaiveDate::parse_from_str(text, "%Y-%m-%d").ok()
}

/// 某月第一格（周日开头）对应的日期：6 行 × 7 列
pub fn month_grid_start(year: i32, month: u32) -> Option<NaiveDate> {
    let first = NaiveDate::from_ymd_opt(year, month, 1)?;
    let back = first.weekday().num_days_from_sunday();
    // EP：第一行不会整行都是上个月；当月 1 号是周日时上一行补满一整周
    let back = if back == 0 { 7 } else { back };
    first.checked_sub_days(chrono::Days::new(u64::from(back)))
}

impl EpDatePicker {
    pub fn new(
        id: impl Into<SharedString>,
        placeholder: impl Into<SharedString>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let placeholder = placeholder.into();
        let input = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder));
        let subscriptions =
            vec![
                cx.subscribe_in(&input, window, |this, _, event: &InputEvent, window, cx| {
                    match event {
                        InputEvent::Focus => {
                            if !this.disabled {
                                this.open_panel(cx);
                            }
                        }
                        InputEvent::PressEnter { .. } => {
                            this.commit_text(window, cx);
                            this.open = false;
                            cx.notify();
                        }
                        InputEvent::Blur => {
                            this.commit_text(window, cx);
                            this.open = false;
                            cx.notify();
                        }
                        InputEvent::Change => {
                            // 输入了合法日期：面板跟着跳到那个月
                            let text = this.input.read(cx).value();
                            if let Some(date) = parse_date(&text) {
                                this.view_year = date.year();
                                this.view_month = date.month();
                                cx.notify();
                            }
                        }
                    }
                }),
            ];
        let today = Local::now().date_naive();
        Self {
            id: id.into(),
            input,
            value: None,
            open: false,
            mode: PanelMode::Date,
            view_year: today.year(),
            view_month: today.month(),
            disabled: false,
            anchor: Anchor::new(),
            _subscriptions: subscriptions,
        }
    }

    /// 当前值 `YYYY-MM-DD`
    pub fn value(&self) -> Option<String> {
        self.value.map(|d| d.format("%Y-%m-%d").to_string())
    }

    /// 外部赋值（不发事件）
    pub fn set_value(&mut self, value: Option<&str>, window: &mut Window, cx: &mut Context<Self>) {
        self.value = value.and_then(parse_date);
        self.sync_text(window, cx);
        cx.notify();
    }

    pub fn set_disabled(&mut self, disabled: bool, cx: &mut Context<Self>) {
        if self.disabled != disabled {
            self.disabled = disabled;
            if disabled {
                self.open = false;
            }
            cx.notify();
        }
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    /// 关闭面板（Esc 由窗口转交）
    pub fn close(&mut self, cx: &mut Context<Self>) {
        if self.open {
            self.open = false;
            cx.notify();
        }
    }

    fn open_panel(&mut self, cx: &mut Context<Self>) {
        if self.open {
            return;
        }
        let base = self.value.unwrap_or_else(|| Local::now().date_naive());
        self.view_year = base.year();
        self.view_month = base.month();
        self.mode = PanelMode::Date;
        self.open = true;
        cx.notify();
    }

    fn sync_text(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.value().unwrap_or_default();
        self.input
            .update(cx, |state, cx| state.set_value(text, window, cx));
    }

    /// 回车 / 失焦：输入框里的文字合法则提交，否则还原
    fn commit_text(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.input.read(cx).value().to_string();
        if text.trim().is_empty() {
            if self.value.is_some() {
                self.value = None;
                cx.emit(EpDatePickerEvent::Change(None));
            }
            return;
        }
        match parse_date(&text) {
            Some(date) if Some(date) != self.value => {
                self.value = Some(date);
                self.sync_text(window, cx);
                cx.emit(EpDatePickerEvent::Change(self.value()));
            }
            Some(_) => {}
            None => self.sync_text(window, cx),
        }
    }

    fn pick(&mut self, date: NaiveDate, window: &mut Window, cx: &mut Context<Self>) {
        self.value = Some(date);
        self.open = false;
        self.sync_text(window, cx);
        cx.emit(EpDatePickerEvent::Change(self.value()));
        cx.notify();
    }

    fn clear(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.value = None;
        self.open = false;
        self.sync_text(window, cx);
        cx.emit(EpDatePickerEvent::Change(None));
        cx.notify();
    }

    fn shift_month(&mut self, delta: i32, cx: &mut Context<Self>) {
        let index = self.view_year * 12 + self.view_month as i32 - 1 + delta;
        self.view_year = index.div_euclid(12);
        self.view_month = index.rem_euclid(12) as u32 + 1;
        cx.notify();
    }

    fn render_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let ep = ep();
        let label =
            |id: &'static str, text: SharedString, mode: PanelMode, cx: &mut Context<Self>| {
                div()
                    .id(id)
                    .px(px(5.))
                    .text_size(px(16.))
                    .line_height(px(22.))
                    .text_color(ep.text_regular)
                    .cursor_pointer()
                    .hover(move |s| s.text_color(ep.primary))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.mode = mode;
                        cx.notify();
                    }))
                    .child(text)
            };
        let year_text: SharedString = match self.mode {
            PanelMode::Year => {
                let start = self.view_year - self.view_year.rem_euclid(10);
                format!("{start} - {}", start + 9).into()
            }
            _ => self.view_year.to_string().into(),
        };
        let mut center = div()
            .flex_1()
            .flex()
            .justify_center()
            .items_center()
            .h(px(30.))
            .child(label("dp-year", year_text, PanelMode::Year, cx));
        if self.mode == PanelMode::Date {
            center = center.child(label(
                "dp-month",
                MONTHS[self.view_month as usize - 1].into(),
                PanelMode::Month,
                cx,
            ));
        }
        let step = |mode: PanelMode| match mode {
            PanelMode::Year => 120,
            _ => 12,
        };
        let mode = self.mode;
        div()
            .flex()
            .items_start()
            .px(px(12.))
            .pt(px(12.))
            .child(self.icon_button(
                "dp-prev-year",
                EpIcon::DArrowLeft,
                move |this, cx| this.shift_month(-step(mode), cx),
                cx,
            ))
            .when(mode == PanelMode::Date, |this| {
                this.child(self.icon_button(
                    "dp-prev-month",
                    EpIcon::ArrowLeft,
                    |this, cx| this.shift_month(-1, cx),
                    cx,
                ))
            })
            .child(center)
            .when(mode == PanelMode::Date, |this| {
                this.child(self.icon_button(
                    "dp-next-month",
                    EpIcon::ArrowRight,
                    |this, cx| this.shift_month(1, cx),
                    cx,
                ))
            })
            .child(self.icon_button(
                "dp-next-year",
                EpIcon::DArrowRight,
                move |this, cx| this.shift_month(step(mode), cx),
                cx,
            ))
    }

    fn icon_button(
        &self,
        id: &'static str,
        source: EpIcon,
        on_click: impl Fn(&mut Self, &mut Context<Self>) + 'static,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let ep = ep();
        let group: SharedString = format!("{}-{id}", self.id).into();
        div()
            .id(id)
            .group(group.clone())
            .px(px(6.))
            .py(px(1.))
            .mt(px(8.))
            .cursor_pointer()
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| on_click(this, cx)))
            .child(
                icon(source, 12., ep.text_primary)
                    .group_hover(group, move |s| s.text_color(ep.primary)),
            )
    }

    fn render_days(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let ep = ep();
        let today = Local::now().date_naive();
        let start = month_grid_start(self.view_year, self.view_month);
        let header = div()
            .flex()
            .border_b_1()
            .border_color(ep.border_lighter)
            .children(WEEKDAYS.iter().map(|w| {
                div()
                    .flex_1()
                    .h(px(40.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(px(12.))
                    .text_color(ep.text_regular)
                    .child(*w)
            }));
        let rows = (0..6).map(|row| {
            div().flex().children((0..7).map(|col| {
                let date = start.and_then(|s| s.checked_add_days(chrono::Days::new(row * 7 + col)));
                let Some(date) = date else {
                    return div().flex_1().h(px(38.)).into_any_element();
                };
                let in_month = date.month() == self.view_month;
                let selected = self.value == Some(date);
                let is_today = date == today;
                let color = if selected {
                    ep.white
                } else if is_today {
                    ep.primary
                } else if in_month {
                    ep.text_regular
                } else {
                    ep.text_placeholder
                };
                let group: SharedString = format!("{}-d{row}-{col}", self.id).into();
                div()
                    .id(SharedString::from(format!("{}-day-{row}-{col}", self.id)))
                    .group(group.clone())
                    .flex_1()
                    .h(px(38.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        this.pick(date, window, cx)
                    }))
                    .child(
                        div()
                            .size(px(24.))
                            .rounded_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_size(px(12.))
                            .text_color(color)
                            .when(selected, |this| this.bg(ep.primary))
                            .when(is_today && !selected, |this| {
                                this.font_weight(gpui_kit::FontWeight::BOLD)
                            })
                            .when(!selected, |this| {
                                this.group_hover(group, move |s| s.text_color(ep.primary))
                            })
                            .child(date.day().to_string()),
                    )
                    .into_any_element()
            }))
        });
        div()
            .m(px(15.))
            .w(px(292.))
            .flex()
            .flex_col()
            .child(header)
            .children(rows)
    }

    fn render_months(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let ep = ep();
        let current = self.value.map(|d| (d.year(), d.month()));
        let now = Local::now().date_naive();
        div()
            .m(px(15.))
            .w(px(292.))
            .border_t_1()
            .border_color(ep.border_lighter)
            .flex()
            .flex_wrap()
            .children((1..=12u32).map(|month| {
                let selected = current == Some((self.view_year, month));
                let this_month = now.year() == self.view_year && now.month() == month;
                let color = if selected {
                    ep.white
                } else if this_month {
                    ep.primary
                } else {
                    ep.text_regular
                };
                div()
                    .id(SharedString::from(format!("{}-m{month}", self.id)))
                    .w(px(292. / 4.))
                    .h(px(68.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.view_month = month;
                        this.mode = PanelMode::Date;
                        cx.notify();
                    }))
                    .child(
                        div()
                            .w(px(54.))
                            .h(px(36.))
                            .rounded(px(18.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_size(px(12.))
                            .text_color(color)
                            .when(selected, |this| this.bg(ep.primary))
                            .when(this_month && !selected, |this| {
                                this.font_weight(gpui_kit::FontWeight::BOLD)
                            })
                            .child(MONTHS_SHORT[month as usize - 1]),
                    )
            }))
    }

    fn render_years(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let ep = ep();
        let start = self.view_year - self.view_year.rem_euclid(10);
        let current = self.value.map(|d| d.year());
        let now = Local::now().date_naive().year();
        div()
            .m(px(15.))
            .w(px(292.))
            .border_t_1()
            .border_color(ep.border_lighter)
            .flex()
            .flex_wrap()
            .children((start..start + 10).map(|year| {
                let selected = current == Some(year);
                let color = if selected {
                    ep.white
                } else if year == now {
                    ep.primary
                } else {
                    ep.text_regular
                };
                div()
                    .id(SharedString::from(format!("{}-y{year}", self.id)))
                    .w(px(292. / 4.))
                    .h(px(68.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.view_year = year;
                        this.mode = PanelMode::Month;
                        cx.notify();
                    }))
                    .child(
                        div()
                            .w(px(60.))
                            .h(px(36.))
                            .rounded(px(18.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_size(px(12.))
                            .text_color(color)
                            .when(selected, |this| this.bg(ep.primary))
                            .child(year.to_string()),
                    )
            }))
    }

    fn render_panel(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let body = match self.mode {
            PanelMode::Date => self.render_days(cx).into_any_element(),
            PanelMode::Month => self.render_months(cx).into_any_element(),
            PanelMode::Year => self.render_years(cx).into_any_element(),
        };
        ep2_popper::frame()
            .w(px(PANEL_WIDTH))
            .text_color(ep().text_regular)
            .font(theme::ui_font())
            .flex()
            .flex_col()
            .child(self.render_header(cx))
            .child(body)
    }
}

impl Render for EpDatePicker {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let ep = ep();
        let group: SharedString = format!("{}-wrap", self.id).into();
        let has_value = self.value.is_some() && !self.disabled;
        let clear = div()
            .id(SharedString::from(format!("{}-clear", self.id)))
            .flex()
            .items_center()
            .cursor_pointer()
            .invisible()
            .when(has_value, |this| {
                this.group_hover(group.clone(), |s| s.visible())
            })
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                this.clear(window, cx);
            }))
            .child(icon(EpIcon::CircleClose, 14., ep.text_placeholder));
        let input = EpInput::new(&self.input)
            .disabled(self.disabled)
            .w_full()
            .prefix(icon(EpIcon::Calendar, 14., ep.text_placeholder).mr(px(4.)))
            .suffix(clear);
        let popper = if self.open && !self.disabled {
            let entity = cx.entity().downgrade();
            ep2_popper::popper(
                &self.anchor,
                window,
                PopperOptions {
                    placement: Placement::TopStart,
                    flip: false,
                    gap: 12.,
                    arrow: true,
                    width: PANEL_WIDTH,
                    height: PANEL_HEIGHT,
                },
                self.render_panel(cx),
                move |window, cx| {
                    let _ = entity.update(cx, |this, cx| {
                        this.commit_text(window, cx);
                        this.open = false;
                        cx.notify();
                    });
                },
            )
        } else {
            None
        };
        div()
            .id(SharedString::from(format!("{}-root", self.id)))
            .group(group)
            .relative()
            .w_full()
            .child(self.anchor.measure())
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    if !this.disabled {
                        this.open_panel(cx);
                    }
                }),
            )
            .child(input)
            .children(popper)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_only_full_dates() {
        assert_eq!(
            parse_date("2026-10-13"),
            NaiveDate::from_ymd_opt(2026, 10, 13)
        );
        assert_eq!(
            parse_date(" 2026-10-13 "),
            NaiveDate::from_ymd_opt(2026, 10, 13)
        );
        assert_eq!(parse_date("2026-1-3"), None);
        assert_eq!(parse_date("2026-02-30"), None);
        assert_eq!(parse_date("tomorrow"), None);
    }

    #[test]
    fn grid_starts_on_sunday_with_leading_days() {
        // 2026-10-01 是周四：第一格是 9 月 27 日（周日）
        assert_eq!(
            month_grid_start(2026, 10),
            NaiveDate::from_ymd_opt(2026, 9, 27)
        );
        // 2026-02-01 是周日：上一行补一整周
        assert_eq!(
            month_grid_start(2026, 2),
            NaiveDate::from_ymd_opt(2026, 1, 25)
        );
    }
}
