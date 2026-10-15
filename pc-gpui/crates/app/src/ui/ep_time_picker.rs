//! `el-time-picker format="HH:mm"`（Element Plus 2.13，英文按钮）。
//!
//! 外观：`el-input`（前缀 Clock 图标、可清空、可直接输入 `HH:mm`）。面板宽 180（+边框）：时 / 分两列滚轮
//! （各占 50%、可视高 192、每项 32px、12px 字；选中项居中加粗 `#303133`，中间一条上下 1px `#E4E7ED` 的选中带，
//! 左右各缩 12%），底部 36px 按钮行：`Cancel`（12px `#303133`）、`OK`（`#409EFF`、800）。
//! 滚轮在列上逐项调整、点某项直接选中；面板打开期间输入框跟着显示，OK / 点外面提交，Cancel 还原。

use chrono::{Local, NaiveTime, Timelike};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::{
    AppContext as _, ClickEvent, Context, Entity, EventEmitter, FontWeight,
    InteractiveElement as _, IntoElement, MouseButton, ParentElement as _, Render, ScrollDelta,
    ScrollWheelEvent, SharedString, StatefulInteractiveElement as _, Styled as _, Subscription,
    Window, div, prelude::FluentBuilder as _, px, relative,
};

use crate::assets::EpIcon;
use crate::theme::{self, ep};
use crate::ui::ep2_popper::{self, Anchor, Placement, PopperOptions};
use crate::ui::icon::icon;
use crate::ui::input::EpInput;

/// 值变化（`HH:mm` / 清空为 `None`）
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EpTimePickerEvent {
    Change(Option<String>),
}

pub const PANEL_WIDTH: f32 = 182.;
pub const PANEL_HEIGHT: f32 = 230.;
const ITEM_HEIGHT: f32 = 32.;
const SPINNER_HEIGHT: f32 = 192.;

/// `HH:mm` 或 `HH:mm:ss`（两位时分）
pub fn parse_time(text: &str) -> Option<(u32, u32)> {
    let text = text.trim();
    let t = NaiveTime::parse_from_str(text, "%H:%M")
        .or_else(|_| NaiveTime::parse_from_str(text, "%H:%M:%S"))
        .ok()?;
    if text.len() < 5 || !text.as_bytes()[..2].iter().all(u8::is_ascii_digit) {
        return None;
    }
    Some((t.hour(), t.minute()))
}

pub fn format_time((h, m): (u32, u32)) -> String {
    format!("{h:02}:{m:02}")
}

pub struct EpTimePicker {
    id: SharedString,
    input: Entity<InputState>,
    value: Option<(u32, u32)>,
    draft: (u32, u32),
    open: bool,
    disabled: bool,
    anchor: Anchor,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<EpTimePickerEvent> for EpTimePicker {}

impl EpTimePicker {
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
                                this.open_panel(window, cx);
                            }
                        }
                        InputEvent::PressEnter { .. } => {
                            this.commit_text(window, cx);
                            this.open = false;
                            cx.notify();
                        }
                        InputEvent::Blur => {
                            if this.open {
                                this.confirm(window, cx);
                            } else {
                                this.commit_text(window, cx);
                            }
                        }
                        InputEvent::Change => {}
                    }
                }),
            ];
        Self {
            id: id.into(),
            input,
            value: None,
            draft: (0, 0),
            open: false,
            disabled: false,
            anchor: Anchor::new(),
            _subscriptions: subscriptions,
        }
    }

    pub fn value(&self) -> Option<String> {
        self.value.map(format_time)
    }

    /// 外部赋值（不发事件）
    pub fn set_value(&mut self, value: Option<&str>, window: &mut Window, cx: &mut Context<Self>) {
        self.value = value.and_then(parse_time);
        self.sync_text(self.value, window, cx);
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

    /// Esc：等同 Cancel
    pub fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.open {
            self.cancel(window, cx);
        }
    }

    fn open_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.open {
            return;
        }
        let now = Local::now();
        self.draft = self.value.unwrap_or((now.hour(), now.minute()));
        self.open = true;
        self.sync_text(Some(self.draft), window, cx);
        cx.notify();
    }

    fn sync_text(
        &mut self,
        value: Option<(u32, u32)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let text = value.map(format_time).unwrap_or_default();
        self.input
            .update(cx, |state, cx| state.set_value(text, window, cx));
    }

    fn commit_text(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.input.read(cx).value().to_string();
        if text.trim().is_empty() {
            if self.value.is_some() {
                self.value = None;
                cx.emit(EpTimePickerEvent::Change(None));
            }
            return;
        }
        match parse_time(&text) {
            Some(t) if Some(t) != self.value => {
                self.value = Some(t);
                self.sync_text(self.value, window, cx);
                cx.emit(EpTimePickerEvent::Change(self.value()));
            }
            Some(_) => {}
            None => self.sync_text(self.value, window, cx),
        }
    }

    fn confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open = false;
        let changed = self.value != Some(self.draft);
        self.value = Some(self.draft);
        self.sync_text(self.value, window, cx);
        if changed {
            cx.emit(EpTimePickerEvent::Change(self.value()));
        }
        cx.notify();
    }

    fn cancel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open = false;
        self.sync_text(self.value, window, cx);
        cx.notify();
    }

    fn clear(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open = false;
        self.value = None;
        self.sync_text(None, window, cx);
        cx.emit(EpTimePickerEvent::Change(None));
        cx.notify();
    }

    fn set_draft(&mut self, draft: (u32, u32), window: &mut Window, cx: &mut Context<Self>) {
        self.draft = draft;
        self.sync_text(Some(draft), window, cx);
        cx.notify();
    }

    fn spinner(&self, hour: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let ep = ep();
        let (selected, max) = if hour {
            (self.draft.0, 23)
        } else {
            (self.draft.1, 59)
        };
        let first = selected.saturating_sub(3);
        let last = (selected + 3).min(max);
        let center_top = (SPINNER_HEIGHT - ITEM_HEIGHT) / 2.;
        let items = (first..=last).map(|value| {
            let top = center_top + (value as f32 - selected as f32) * ITEM_HEIGHT;
            let active = value == selected;
            div()
                .id(SharedString::from(format!(
                    "{}-{}{value}",
                    self.id,
                    if hour { "h" } else { "m" }
                )))
                .absolute()
                .left_0()
                .right_0()
                .top(px(top))
                .h(px(ITEM_HEIGHT))
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(12.))
                .text_color(if active {
                    ep.text_primary
                } else {
                    ep.text_regular
                })
                .when(active, |this| this.font_weight(FontWeight::BOLD))
                .when(!active, |this| {
                    this.cursor_pointer().hover(move |s| s.bg(ep.fill_light))
                })
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                    let draft = if hour {
                        (value, this.draft.1)
                    } else {
                        (this.draft.0, value)
                    };
                    this.set_draft(draft, window, cx);
                }))
                .child(format!("{value:02}"))
        });
        div()
            .id(SharedString::from(format!(
                "{}-spin-{}",
                self.id,
                if hour { "h" } else { "m" }
            )))
            .relative()
            .w(relative(0.5))
            .h(px(SPINNER_HEIGHT))
            .overflow_hidden()
            .on_scroll_wheel(
                cx.listener(move |this, event: &ScrollWheelEvent, window, cx| {
                    let dy = match event.delta {
                        ScrollDelta::Lines(p) => p.y,
                        ScrollDelta::Pixels(p) => f32::from(p.y) / ITEM_HEIGHT,
                    };
                    if dy == 0. {
                        return;
                    }
                    let step: i32 = if dy < 0. { 1 } else { -1 };
                    let (current, max) = if hour {
                        (this.draft.0 as i32, 23)
                    } else {
                        (this.draft.1 as i32, 59)
                    };
                    let next = (current + step).clamp(0, max) as u32;
                    let draft = if hour {
                        (next, this.draft.1)
                    } else {
                        (this.draft.0, next)
                    };
                    this.set_draft(draft, window, cx);
                    cx.stop_propagation();
                }),
            )
            .children(items)
    }

    fn render_panel(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let ep = ep();
        let band_top = (SPINNER_HEIGHT - ITEM_HEIGHT) / 2.;
        let button =
            |id: &'static str, label: &'static str, confirm: bool, cx: &mut Context<Self>| {
                div()
                    .id(id)
                    .mx(px(5.))
                    .px(px(5.))
                    .h(px(28.))
                    .flex()
                    .items_center()
                    .text_size(px(12.))
                    .cursor_pointer()
                    .text_color(if confirm { ep.primary } else { ep.text_primary })
                    .when(confirm, |this| this.font_weight(FontWeight::EXTRA_BOLD))
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        if confirm {
                            this.confirm(window, cx);
                        } else {
                            this.cancel(window, cx);
                        }
                    }))
                    .child(label)
            };
        ep2_popper::frame()
            .w(px(PANEL_WIDTH))
            .font(theme::ui_font())
            .flex()
            .flex_col()
            .child(
                div()
                    .relative()
                    .flex()
                    .h(px(SPINNER_HEIGHT))
                    .child(
                        div()
                            .absolute()
                            .top(px(band_top))
                            .left(relative(0.12))
                            .right(relative(0.12))
                            .h(px(ITEM_HEIGHT))
                            .border_t_1()
                            .border_b_1()
                            .border_color(ep.border_light),
                    )
                    .child(self.spinner(true, cx))
                    .child(self.spinner(false, cx)),
            )
            .child(
                div()
                    .h(px(36.))
                    .p(px(4.))
                    .border_t_1()
                    .border_color(ep.border_light)
                    .flex()
                    .justify_end()
                    .items_center()
                    .child(button("tp-cancel", "Cancel", false, cx))
                    .child(button("tp-ok", "OK", true, cx)),
            )
    }
}

impl Render for EpTimePicker {
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
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.clear(window, cx)))
            .child(icon(EpIcon::CircleClose, 14., ep.text_placeholder));
        let input = EpInput::new(&self.input)
            .disabled(self.disabled)
            .w_full()
            .prefix(icon(EpIcon::Clock, 14., ep.text_placeholder).mr(px(4.)))
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
                    let _ = entity.update(cx, |this, cx| this.confirm(window, cx));
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
                cx.listener(|this, _, window, cx| {
                    if !this.disabled {
                        this.open_panel(window, cx);
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
    fn parses_hh_mm() {
        assert_eq!(parse_time("09:05"), Some((9, 5)));
        assert_eq!(parse_time("23:59:10"), Some((23, 59)));
        assert_eq!(parse_time("9:05"), None);
        assert_eq!(parse_time("24:00"), None);
        assert_eq!(parse_time(""), None);
        assert_eq!(format_time((7, 3)), "07:03");
    }
}
