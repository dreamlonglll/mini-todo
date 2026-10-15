//! 每个窗口 Root 上的浮层插件 [`UiLayer`]：承载 `ElMessage`（[`super::message`]）与
//! `ElMessageBox.confirm`（[`super::message_box`]）。`main` 里 `UiLayer::register(cx)` 一次，之后每个用
//! `gpui_kit::base::Root` 包装的窗口都会自动挂上（GPUI 没有 z-index，插件画在应用内容之上）。

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use gpui_kit::base::{Root, RootPlugin};
use gpui_kit::{
    Animation, AnimationExt as _, AnyElement, App, Context, Entity, FocusHandle,
    InteractiveElement as _, IntoElement, KeyDownEvent, MouseButton, ParentElement as _, Render,
    SharedString, StatefulInteractiveElement as _, Styled as _, Window, div,
    prelude::FluentBuilder as _, px,
};

use crate::assets::EpIcon;
use crate::theme::{self, ep};
use crate::ui::button::EpButton;
use crate::ui::icon::icon;
use crate::ui::message::{MessageKind, kind_colors, kind_icon};
use crate::ui::message_box::{BoxKind, ConfirmCallback, ConfirmOptions};

/// 消息离场动画时长
const LEAVE_DURATION: Duration = Duration::from_millis(300);
/// 计时轮询间隔（悬停暂停期间）
const TICK: Duration = Duration::from_millis(250);

struct Toast {
    id: u64,
    kind: MessageKind,
    text: SharedString,
    deadline: Instant,
    /// 悬停时暂停：剩余时长
    paused: Option<Duration>,
    leaving: bool,
}

struct PendingConfirm {
    id: u64,
    options: ConfirmOptions,
    callback: Option<ConfirmCallback>,
}

/// 窗口浮层（消息 + 确认框）
pub struct UiLayer {
    toasts: Vec<Toast>,
    confirms: VecDeque<PendingConfirm>,
    next_id: u64,
    confirm_focus: FocusHandle,
    restore_focus: Option<FocusHandle>,
}

impl RootPlugin for UiLayer {}

impl UiLayer {
    /// 注册为 Root 插件（`gpui_kit::init` 之后、开窗之前调用一次）
    pub fn register(cx: &mut App) {
        Root::register_plugin::<UiLayer>(cx, |_window, cx| UiLayer {
            toasts: Vec::new(),
            confirms: VecDeque::new(),
            next_id: 0,
            confirm_focus: cx.focus_handle(),
            restore_focus: None,
        });
    }

    /// 该窗口的浮层（窗口根不是 `Root` 时为 `None`）
    pub fn of(window: &Window, cx: &App) -> Option<Entity<UiLayer>> {
        window
            .root::<Root>()
            .flatten()?
            .read(cx)
            .plugin::<UiLayer>()
    }

    pub fn has_confirm(&self) -> bool {
        !self.confirms.is_empty()
    }

    pub(crate) fn push_message(
        &mut self,
        kind: MessageKind,
        text: SharedString,
        duration: Duration,
        cx: &mut Context<Self>,
    ) {
        let id = self.next_id;
        self.next_id += 1;
        self.toasts.push(Toast {
            id,
            kind,
            text,
            deadline: Instant::now() + duration,
            paused: None,
            leaving: false,
        });
        cx.notify();
        cx.spawn(async move |this, cx| {
            loop {
                let Ok(Some(left)) = this.update(cx, |layer, _| layer.time_left(id)) else {
                    return;
                };
                if left.is_zero() {
                    break;
                }
                cx.background_executor().timer(left.min(TICK)).await;
            }
            let _ = this.update(cx, |layer, cx| {
                if let Some(toast) = layer.toasts.iter_mut().find(|t| t.id == id) {
                    toast.leaving = true;
                    cx.notify();
                }
            });
            cx.background_executor().timer(LEAVE_DURATION).await;
            let _ = this.update(cx, |layer, cx| {
                layer.toasts.retain(|t| t.id != id);
                cx.notify();
            });
        })
        .detach();
    }

    /// 剩余显示时长；悬停中返回一个轮询间隔；消息已不在返回 `None`
    fn time_left(&self, id: u64) -> Option<Duration> {
        let toast = self.toasts.iter().find(|t| t.id == id)?;
        Some(match toast.paused {
            Some(_) => TICK,
            None => toast.deadline.saturating_duration_since(Instant::now()),
        })
    }

    fn set_toast_hover(&mut self, id: u64, hovered: bool) {
        let Some(toast) = self.toasts.iter_mut().find(|t| t.id == id) else {
            return;
        };
        let now = Instant::now();
        match (hovered, toast.paused) {
            (true, None) => toast.paused = Some(toast.deadline.saturating_duration_since(now)),
            (false, Some(left)) => {
                toast.deadline = now + left;
                toast.paused = None;
            }
            _ => {}
        }
    }

    pub(crate) fn push_confirm(
        &mut self,
        options: ConfirmOptions,
        callback: ConfirmCallback,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let id = self.next_id;
        self.next_id += 1;
        if self.confirms.is_empty() {
            self.restore_focus = window.focused(cx);
            window.focus(&self.confirm_focus, cx);
        }
        self.confirms.push_back(PendingConfirm {
            id,
            options,
            callback: Some(callback),
        });
        cx.notify();
    }

    fn resolve(&mut self, confirmed: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(mut pending) = self.confirms.pop_front() else {
            return;
        };
        if self.confirms.is_empty() {
            if let Some(previous) = self.restore_focus.take() {
                window.focus(&previous, cx);
            }
        } else {
            window.focus(&self.confirm_focus, cx);
        }
        cx.notify();
        if let Some(callback) = pending.callback.take() {
            // 回调里可能再弹消息 / 确认框（会再次更新本实体），延后到本次更新之外执行
            window.defer(cx, move |window, cx| callback(confirmed, window, cx));
        }
    }

    fn render_toast(&self, toast: &Toast, max_width: f32, cx: &mut Context<Self>) -> AnyElement {
        let (bg, border, fg) = kind_colors(toast.kind);
        let id = toast.id;
        let body = div()
            .id(("ui-message", id))
            .occlude()
            .relative()
            .flex()
            .items_center()
            .gap(px(8.))
            .max_w(px(max_width))
            .px(px(15.))
            .py(px(11.))
            .bg(bg)
            .border_1()
            .border_color(border)
            .rounded(theme::radius::SM)
            .text_size(px(14.))
            .line_height(px(14.))
            .text_color(fg)
            .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                this.set_toast_hover(id, *hovered);
                cx.notify();
            }))
            .child(icon(kind_icon(toast.kind), 14., fg))
            .child(div().min_w_0().child(toast.text.clone()));
        if toast.leaving {
            body.with_animation(
                ("ui-message-leave", id),
                Animation::new(LEAVE_DURATION),
                |el, delta| el.opacity(1. - delta).top(px(-20. * delta)),
            )
            .into_any_element()
        } else {
            body.with_animation(
                ("ui-message-enter", id),
                Animation::new(theme::duration::SLOW),
                |el, delta| el.opacity(delta).top(px(-20. * (1. - delta))),
            )
            .into_any_element()
        }
    }

    fn render_confirm(
        &self,
        pending: &PendingConfirm,
        width: f32,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let ep = ep();
        let options = &pending.options;
        let status_icon = match options.kind {
            BoxKind::Warning => Some(icon(EpIcon::WarningFilled, 24., ep.warning)),
            BoxKind::Info => Some(icon(EpIcon::InfoFilled, 24., ep.info)),
            BoxKind::None => None,
        };
        let dialog = div()
            .id(("ui-confirm", pending.id))
            .occlude()
            .track_focus(&self.confirm_focus)
            .key_context("UiConfirm")
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                match event.keystroke.key.as_str() {
                    "escape" => this.resolve(false, window, cx),
                    "enter" => this.resolve(true, window, cx),
                    _ => return,
                }
                cx.stop_propagation();
            }))
            .relative()
            .w(px(width))
            .p(px(12.))
            .bg(ep.white)
            .rounded(theme::radius::SM)
            .shadow(ep.box_shadow())
            .child(
                div()
                    .pb(px(12.))
                    .pr(px(28.))
                    .text_size(px(18.))
                    .line_height(px(24.))
                    .text_color(ep.text_primary)
                    .child(options.title.clone()),
            )
            .child(
                div()
                    .id("ui-confirm-close")
                    .group("ui-confirm-close")
                    .absolute()
                    .top_0()
                    .right_0()
                    .size(px(40.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _, window, cx| this.resolve(false, window, cx)))
                    .child(
                        icon(EpIcon::Close, 16., ep.text_secondary)
                            .group_hover("ui-confirm-close", |s| s.text_color(ep.primary)),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .text_size(px(14.))
                    .line_height(px(24.))
                    .text_color(ep.text_regular)
                    .children(status_icon)
                    // 正文按对话框宽度折行（长文本 / 长单词都不溢出）
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .whitespace_normal()
                            .overflow_hidden()
                            .child(options.message.clone()),
                    ),
            )
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(px(12.))
                    .pt(px(12.))
                    .child(
                        EpButton::new("ui-confirm-cancel")
                            .label(options.cancel_text.clone())
                            .on_click(
                                cx.listener(|this, _, window, cx| this.resolve(false, window, cx)),
                            ),
                    )
                    .child(
                        EpButton::new("ui-confirm-ok")
                            .primary()
                            .label(options.confirm_text.clone())
                            .on_click(
                                cx.listener(|this, _, window, cx| this.resolve(true, window, cx)),
                            ),
                    ),
            )
            .with_animation(
                ("ui-confirm-in", pending.id),
                Animation::new(theme::duration::SLOW),
                |el, delta| el.opacity(delta).top(px(-20. * (1. - delta))),
            );

        div()
            .id(("ui-confirm-overlay", pending.id))
            .absolute()
            .inset_0()
            .occlude()
            .bg(ep.overlay)
            .flex()
            .items_center()
            .justify_center()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| this.resolve(false, window, cx)),
            )
            .child(dialog)
            .with_animation(
                ("ui-confirm-overlay-in", pending.id),
                Animation::new(theme::duration::BASE),
                |el, delta| el.opacity(delta),
            )
            .into_any_element()
    }
}

impl Render for UiLayer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let viewport = window.viewport_size();
        let width = f32::from(viewport.width);
        let confirm_width = (width - 32.).clamp(0., 420.);
        let confirm = self
            .confirms
            .front()
            .map(|pending| self.render_confirm(pending, confirm_width, cx));
        let mut toasts = Vec::with_capacity(self.toasts.len());
        for toast in &self.toasts {
            toasts.push(self.render_toast(toast, (width - 32.).max(0.), cx));
        }
        div()
            .size_full()
            .font(theme::ui_font())
            .children(confirm)
            .when(!toasts.is_empty(), |this| {
                this.child(
                    div()
                        .absolute()
                        .top(px(16.))
                        .left_0()
                        .right_0()
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap(px(16.))
                        .children(toasts),
                )
            })
    }
}
