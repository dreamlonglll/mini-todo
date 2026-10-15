//! `el-input-number`（Element Plus 2.13，`style="width:100%"`）。
//!
//! 高 32、1px `#DCDFE6` 边、圆角 6（el-input 覆盖值）；左右各一个 32px 宽按钮（左 Minus、右 Plus，底 `#F5F7FA`、
//! 字 `#606266`、与输入区之间 1px `#DCDFE6` 竖线；hover 字 `#409EFF`；到达边界时字 `#A8ABB2`、`not-allowed`）；
//! 中间输入区文字居中 14px；可选后缀文字（`#suffix` 插槽，`#A8ABB2`）显示在数字右侧输入区内。
//! 输入合法整数时立即生效（与 v-model 一致），回车 / 失焦时夹到 [min, max]，非法则还原。

use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::{
    AppContext as _, ClickEvent, Context, Entity, EventEmitter, Focusable as _,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, Window, div,
    prelude::FluentBuilder as _, px,
};

use crate::assets::EpIcon;
use crate::theme::{self, ep};
use crate::ui::icon::icon;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EpInputNumberEvent {
    Change(i32),
}

pub struct EpInputNumber {
    id: SharedString,
    input: Entity<InputState>,
    value: i32,
    min: i32,
    max: i32,
    suffix: Option<SharedString>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<EpInputNumberEvent> for EpInputNumber {}

impl EpInputNumber {
    pub fn new(
        id: impl Into<SharedString>,
        value: i32,
        min: i32,
        max: i32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let value = value.clamp(min, max);
        let input = cx.new(|cx| InputState::new(window, cx).default_value(value.to_string()));
        let subscriptions =
            vec![
                cx.subscribe_in(&input, window, |this, _, event: &InputEvent, window, cx| {
                    match event {
                        InputEvent::Change => {
                            let text = this.input.read(cx).value();
                            if let Ok(v) = text.trim().parse::<i32>()
                                && v != this.value
                                && (this.min..=this.max).contains(&v)
                            {
                                this.value = v;
                                cx.emit(EpInputNumberEvent::Change(v));
                                cx.notify();
                            }
                        }
                        InputEvent::PressEnter { .. } | InputEvent::Blur => this.commit(window, cx),
                        InputEvent::Focus => {}
                    }
                }),
            ];
        Self {
            id: id.into(),
            input,
            value,
            min,
            max,
            suffix: None,
            _subscriptions: subscriptions,
        }
    }

    /// 输入区内数字右侧的后缀文字
    pub fn with_suffix(mut self, suffix: impl Into<SharedString>) -> Self {
        self.suffix = Some(suffix.into());
        self
    }

    pub fn value(&self) -> i32 {
        self.value
    }

    /// 外部赋值（不发事件）
    pub fn set_value(&mut self, value: i32, window: &mut Window, cx: &mut Context<Self>) {
        self.value = value.clamp(self.min, self.max);
        let text = self.value.to_string();
        self.input
            .update(cx, |state, cx| state.set_value(text, window, cx));
        cx.notify();
    }

    fn commit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.input.read(cx).value();
        let next = text
            .trim()
            .parse::<i64>()
            .ok()
            .map(|v| v.clamp(i64::from(self.min), i64::from(self.max)) as i32);
        if let Some(v) = next
            && v != self.value
        {
            self.value = v;
            cx.emit(EpInputNumberEvent::Change(v));
        }
        self.set_value(self.value, window, cx);
    }

    fn step(&mut self, delta: i32, window: &mut Window, cx: &mut Context<Self>) {
        let next = (self.value + delta).clamp(self.min, self.max);
        if next != self.value {
            self.set_value(next, window, cx);
            cx.emit(EpInputNumberEvent::Change(next));
        }
    }

    fn button(&self, minus: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let ep = ep();
        let disabled = if minus {
            self.value <= self.min
        } else {
            self.value >= self.max
        };
        let color = if disabled {
            ep.disabled_text
        } else {
            ep.text_regular
        };
        let group: SharedString =
            format!("{}-{}", self.id, if minus { "dec" } else { "inc" }).into();
        div()
            .id(group.clone())
            .group(group.clone())
            .w(px(32.))
            .h_full()
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .bg(ep.fill_light)
            .when(minus, |this| {
                this.border_r_1().border_color(ep.border).rounded_l(px(5.))
            })
            .when(!minus, |this| {
                this.border_l_1().border_color(ep.border).rounded_r(px(5.))
            })
            .when(disabled, |this| {
                this.cursor(gpui_kit::CursorStyle::OperationNotAllowed)
            })
            .when(!disabled, |this| {
                this.cursor_pointer().on_click(cx.listener(
                    move |this, _: &ClickEvent, window, cx| {
                        this.step(if minus { -1 } else { 1 }, window, cx)
                    },
                ))
            })
            .child(
                icon(if minus { EpIcon::Minus } else { EpIcon::Plus }, 13., color)
                    .when(!disabled, |this| {
                        this.group_hover(group, move |s| s.text_color(ep.primary))
                    }),
            )
    }
}

impl Render for EpInputNumber {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let ep = ep();
        let focused = self
            .input
            .read(cx)
            .focus_handle(cx)
            .contains_focused(window, cx);
        let input = Input::new(&self.input)
            .appearance(false)
            .text_size(px(14.))
            .text_center()
            .px(px(0.))
            .py(px(0.));
        div()
            .id(SharedString::from(format!("{}-root", self.id)))
            .w_full()
            .h(px(32.))
            .flex()
            .items_center()
            .bg(ep.white)
            .border_1()
            .border_color(if focused { ep.primary } else { ep.border })
            .rounded(theme::radius::BASE)
            .when(!focused, |this| {
                this.hover(|s| s.border_color(ep.border_hover))
            })
            .child(self.button(true, cx))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .px(px(11.))
                    .flex()
                    .items_center()
                    .text_color(ep.text_regular)
                    .child(div().flex_1().min_w_0().child(input))
                    .when_some(self.suffix.clone(), |this, suffix| {
                        this.child(
                            div()
                                .flex_none()
                                .text_size(px(14.))
                                .text_color(ep.text_placeholder)
                                .child(suffix),
                        )
                    }),
            )
            .child(self.button(false, cx))
    }
}
