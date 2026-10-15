//! `el-color-picker size="small"`（不带 alpha）：有状态视图，构造宿主视图时创建。
//!
//! 触发器 24×24：1px `#DCDFE6`（悬停 `#C0C4CC`、展开 `#409EFF`）、圆角 4、padding 4；内部 14×14 色块（1px `#909399`
//! 边框、圆角 2）+ 白色 ArrowDown；禁用时底 `#F5F7FA`、色块 `opacity .3`、`cursor: not-allowed`。
//! 弹层（触发器正下方、无箭头、贴窗口边）：白底、1px `#EBEEF5`、圆角 4、浅阴影、padding 12、内容宽 300：
//! 左 SV 面板 280×180（色相底 + 白→透明横向渐变 + 透明→黑纵向渐变，4px 圆形光标）、右竖直色相条 12×180
//! （上下 2px 内边距，4px 宽滑块）；下方 12px：160 宽 small 输入框（hex，回车 / 失焦解析）+ 文字按钮 `Clear` 与
//! plain 按钮 `OK`（项目没配置 Element Plus 语言，内置文案是英文）。点 `OK` 才发 [`EpColorPickerEvent::Change`]
//! （`#RRGGBB` 大写），点 `Clear` 发 `Change(None)`；点外面 / Esc 收起并放弃未确认的修改。
//!
//! ```ignore
//! let picker = cx.new(|cx| EpColorPicker::new("bg-color", window, cx));
//! cx.subscribe_in(&picker, window, |this, _, e: &EpColorPickerEvent, window, cx| match e {
//!     EpColorPickerEvent::Change(color) => this.set_color(color.clone().unwrap_or("#000000".into()), window, cx),
//! }).detach();
//! picker.update(cx, |p, cx| p.set_value("#1E293B", cx));
//! ```

use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::{
    Anchor, AnchoredPositionMode, AppContext as _, Bounds, Context, CursorStyle, DragMoveEvent,
    ElementId, Empty, Entity, EventEmitter, Hsla, InteractiveElement as _, IntoElement,
    KeyDownEvent, MouseButton, ParentElement as _, Pixels, Point, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, Window, anchored, canvas, deferred,
    div, linear_color_stop, linear_gradient, point, prelude::FluentBuilder as _, px,
};

use crate::assets::EpIcon;
use crate::theme::{self, ep};
use crate::ui::button::EpButton;
use crate::ui::icon::icon;
use crate::ui::input::EpInput;

/// 确认（`Some("#RRGGBB")`）或清空（`None`）
#[derive(Debug, Clone, PartialEq)]
pub enum EpColorPickerEvent {
    Change(Option<SharedString>),
}

const SV_W: f32 = 280.;
const SV_H: f32 = 180.;
const HUE_W: f32 = 12.;
/// 弹层外框高度：padding 12 + SV 180 + 6 + 页脚（12 + 24）+ padding 12 + 边框 2
const PANEL_HEIGHT: f32 = 248.;

/// HSV（h 0~360，s / v 0~1）→ RGB（0~255）
pub(crate) fn hsv_to_rgb(h: f32, s: f32, v: f32) -> (u8, u8, u8) {
    let h = h.rem_euclid(360.) / 60.;
    let c = v * s;
    let x = c * (1. - (h % 2. - 1.).abs());
    let (r, g, b) = match h as u32 {
        0 => (c, x, 0.),
        1 => (x, c, 0.),
        2 => (0., c, x),
        3 => (0., x, c),
        4 => (x, 0., c),
        _ => (c, 0., x),
    };
    let m = v - c;
    let to = |f: f32| ((f + m) * 255.).round().clamp(0., 255.) as u8;
    (to(r), to(g), to(b))
}

/// RGB → HSV
pub(crate) fn rgb_to_hsv(r: u8, g: u8, b: u8) -> (f32, f32, f32) {
    let (r, g, b) = (r as f32 / 255., g as f32 / 255., b as f32 / 255.);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let d = max - min;
    let h = if d == 0. {
        0.
    } else if max == r {
        60. * ((g - b) / d).rem_euclid(6.)
    } else if max == g {
        60. * ((b - r) / d + 2.)
    } else {
        60. * ((r - g) / d + 4.)
    };
    let s = if max == 0. { 0. } else { d / max };
    (h, s, max)
}

pub(crate) fn to_hex(r: u8, g: u8, b: u8) -> String {
    format!("#{r:02X}{g:02X}{b:02X}")
}

#[derive(Clone)]
struct SvDrag(gpui_kit::EntityId);
#[derive(Clone)]
struct HueDrag(gpui_kit::EntityId);

struct DragGhost;

impl Render for DragGhost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        Empty
    }
}

/// 颜色选择器
pub struct EpColorPicker {
    id: ElementId,
    /// 已确认的值（`#RRGGBB`）
    value: SharedString,
    disabled: bool,
    open: bool,
    hovered: bool,
    hue: f32,
    sat: f32,
    val: f32,
    trigger_bounds: Option<Bounds<Pixels>>,
    sv_bounds: Option<Bounds<Pixels>>,
    hue_bounds: Option<Bounds<Pixels>>,
    hex_input: Entity<InputState>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<EpColorPickerEvent> for EpColorPicker {}

impl EpColorPicker {
    pub fn new(id: impl Into<ElementId>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let hex_input = cx.new(|cx| InputState::new(window, cx));
        let subscriptions = vec![cx.subscribe_in(
            &hex_input,
            window,
            |this, state, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                    let text = state.read(cx).value();
                    this.apply_hex_text(&text, window, cx);
                }
            },
        )];
        Self {
            id: id.into(),
            value: "#000000".into(),
            disabled: false,
            open: false,
            hovered: false,
            hue: 0.,
            sat: 0.,
            val: 0.,
            trigger_bounds: None,
            sv_bounds: None,
            hue_bounds: None,
            hex_input,
            _subscriptions: subscriptions,
        }
    }

    /// 设置已确认的值（不发事件）
    pub fn set_value(&mut self, value: impl Into<SharedString>, cx: &mut Context<Self>) {
        let value = value.into();
        if self.value != value {
            self.value = value;
            cx.notify();
        }
    }

    pub fn set_disabled(&mut self, disabled: bool, cx: &mut Context<Self>) {
        self.disabled = disabled;
        if disabled {
            self.open = false;
        }
        cx.notify();
    }

    fn current_rgb(&self) -> (u8, u8, u8) {
        hsv_to_rgb(self.hue, self.sat, self.val)
    }

    fn current_hex(&self) -> String {
        let (r, g, b) = self.current_rgb();
        to_hex(r, g, b)
    }

    fn load_from_value(&mut self) {
        let (r, g, b) = theme::hex_to_rgb_channels(&self.value);
        let (h, s, v) = rgb_to_hsv(r, g, b);
        // 灰度色没有色相：保留上次的色相，避免色相条跳回红色
        if s > 0. {
            self.hue = h;
        }
        self.sat = s;
        self.val = v;
    }

    fn sync_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let hex = self.current_hex();
        self.hex_input
            .update(cx, |s, cx| s.set_value(hex, window, cx));
    }

    fn set_open(&mut self, open: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.open == open {
            return;
        }
        self.open = open;
        if open {
            self.load_from_value();
            self.sync_input(window, cx);
        }
        cx.notify();
    }

    fn apply_hex_text(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        match theme::parse_hex_color(text) {
            Some((r, g, b)) => {
                let (h, s, v) = rgb_to_hsv(r, g, b);
                if s > 0. {
                    self.hue = h;
                }
                self.sat = s;
                self.val = v;
                cx.notify();
            }
            None => self.sync_input(window, cx),
        }
    }

    fn pick_sv(&mut self, position: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(bounds) = self.sv_bounds else {
            return;
        };
        let x = f32::from(position.x - bounds.left()).clamp(0., SV_W);
        let y = f32::from(position.y - bounds.top()).clamp(0., SV_H);
        self.sat = x / SV_W;
        self.val = 1. - y / SV_H;
        self.sync_input(window, cx);
        cx.notify();
    }

    fn pick_hue(&mut self, position: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(bounds) = self.hue_bounds else {
            return;
        };
        let height = f32::from(bounds.size.height).max(1.);
        let y = f32::from(position.y - bounds.top()).clamp(0., height);
        self.hue = (y / height * 360.).min(360.);
        self.sync_input(window, cx);
        cx.notify();
    }

    fn confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.hex_input.read(cx).value();
        if let Some((r, g, b)) = theme::parse_hex_color(&text) {
            let (h, s, v) = rgb_to_hsv(r, g, b);
            if s > 0. {
                self.hue = h;
            }
            self.sat = s;
            self.val = v;
        }
        let hex: SharedString = self.current_hex().into();
        self.value = hex.clone();
        self.set_open(false, window, cx);
        cx.emit(EpColorPickerEvent::Change(Some(hex)));
    }

    fn clear(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.set_open(false, window, cx);
        cx.emit(EpColorPickerEvent::Change(None));
    }

    fn render_panel(&self, window: &Window, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let bounds = self.trigger_bounds.filter(|_| self.open)?;
        // 下方放不下就翻到上方（EP `fallback-placements: bottom → top`）
        let viewport = window.viewport_size().height;
        let below = bounds.bottom() + px(PANEL_HEIGHT) <= viewport
            || bounds.top() - px(PANEL_HEIGHT) < px(0.);
        let ep = ep();
        let id = cx.entity_id();
        let (hr, hg, hb) = hsv_to_rgb(self.hue, 1., 1.);
        let hue_color = theme::rgba_css(hr, hg, hb, 1.);
        let white = theme::rgba_css(255, 255, 255, 1.);
        let clear_white = theme::rgba_css(255, 255, 255, 0.);
        let black = theme::rgba_css(0, 0, 0, 1.);
        let clear_black = theme::rgba_css(0, 0, 0, 0.);
        let cursor_x = self.sat * SV_W;
        let cursor_y = (1. - self.val) * SV_H;
        let hue_y = self.hue / 360. * (SV_H - 4.);
        let weak = cx.entity().downgrade();
        let weak_hue = weak.clone();

        // 色相条：6 段双色渐变拼接（GPUI 的线性渐变只有两个色标）
        let hue_stops: [(u8, u8, u8); 7] = [
            (255, 0, 0),
            (255, 255, 0),
            (0, 255, 0),
            (0, 255, 255),
            (0, 0, 255),
            (255, 0, 255),
            (255, 0, 0),
        ];
        let hue_bar = div()
            .flex()
            .flex_col()
            .size_full()
            .children(hue_stops.windows(2).map(|pair| {
                let (a, b) = (pair[0], pair[1]);
                div().flex_1().w_full().bg(linear_gradient(
                    180.,
                    linear_color_stop(theme::rgba_css(a.0, a.1, a.2, 1.), 0.),
                    linear_color_stop(theme::rgba_css(b.0, b.1, b.2, 1.), 1.),
                ))
            }));

        let sv_panel = div()
            .id("sv")
            .relative()
            .w(px(SV_W))
            .h(px(SV_H))
            .bg(hue_color)
            .cursor_pointer()
            .child(
                canvas(
                    move |bounds, _, cx| {
                        let _ = weak.update(cx, |this, _| this.sv_bounds = Some(bounds));
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .inset_0(),
            )
            .child(div().absolute().inset_0().bg(linear_gradient(
                90.,
                linear_color_stop(white, 0.),
                linear_color_stop(clear_white, 1.),
            )))
            .child(div().absolute().inset_0().bg(linear_gradient(
                0.,
                linear_color_stop(black, 0.),
                linear_color_stop(clear_black, 1.),
            )))
            .child(
                div()
                    .absolute()
                    .left(px(cursor_x - 2.))
                    .top(px(cursor_y - 2.))
                    .size(px(4.))
                    .rounded_full()
                    .border_1()
                    .border_color(ep.white)
                    .shadow(vec![theme::shadow(
                        0.,
                        0.,
                        1.,
                        2.,
                        theme::rgba_css(0, 0, 0, 0.4),
                    )]),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &gpui_kit::MouseDownEvent, window, cx| {
                    this.pick_sv(event.position, window, cx)
                }),
            )
            .on_drag(SvDrag(id), |_, _, _, cx| cx.new(|_| DragGhost))
            .on_drag_move::<SvDrag>(cx.listener(
                move |this, event: &DragMoveEvent<SvDrag>, window, cx| {
                    if event.drag(cx).0 == id {
                        this.pick_sv(event.event.position, window, cx);
                    }
                },
            ));

        let hue_slider = div()
            .id("hue")
            .relative()
            .w(px(HUE_W))
            .h(px(SV_H))
            .py(px(2.))
            .bg(theme::hex(0xFF0000))
            .cursor_pointer()
            .child(
                div()
                    .relative()
                    .size_full()
                    .child(
                        canvas(
                            move |bounds, _, cx| {
                                let _ =
                                    weak_hue.update(cx, |this, _| this.hue_bounds = Some(bounds));
                            },
                            |_, _, _, _| {},
                        )
                        .absolute()
                        .inset_0(),
                    )
                    .child(hue_bar)
                    .child(
                        div()
                            .absolute()
                            .left_0()
                            .top(px(hue_y))
                            .w_full()
                            .h(px(4.))
                            .bg(ep.white)
                            .border_1()
                            .border_color(ep.border_lighter)
                            .rounded(px(1.))
                            .shadow(vec![theme::shadow(
                                0.,
                                0.,
                                2.,
                                0.,
                                theme::rgba_css(0, 0, 0, 0.6),
                            )]),
                    ),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &gpui_kit::MouseDownEvent, window, cx| {
                    this.pick_hue(event.position, window, cx)
                }),
            )
            .on_drag(HueDrag(id), |_, _, _, cx| cx.new(|_| DragGhost))
            .on_drag_move::<HueDrag>(cx.listener(
                move |this, event: &DragMoveEvent<HueDrag>, window, cx| {
                    if event.drag(cx).0 == id {
                        this.pick_hue(event.event.position, window, cx);
                    }
                },
            ));

        let footer = div()
            .mt(px(12.))
            .flex()
            .items_center()
            .justify_between()
            .child(EpInput::new(&self.hex_input).small().w(px(160.)))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .child(
                        EpButton::new("cp-clear")
                            .text()
                            .small()
                            .label("Clear")
                            .on_click(cx.listener(|this, _, window, cx| this.clear(window, cx))),
                    )
                    .child(
                        EpButton::new("cp-ok")
                            .plain()
                            .small()
                            .label("OK")
                            .on_click(cx.listener(|this, _, window, cx| this.confirm(window, cx))),
                    ),
            );

        let panel = div()
            .id("cp-panel")
            .occlude()
            .p(px(12.))
            .bg(ep.white)
            .border_1()
            .border_color(ep.border_lighter)
            .rounded(theme::radius::SM)
            .shadow(ep.box_shadow_light())
            .font(theme::ui_font())
            .on_mouse_down_out(
                cx.listener(|this, event: &gpui_kit::MouseDownEvent, window, cx| {
                    if this
                        .trigger_bounds
                        .is_some_and(|b| b.contains(&event.position))
                    {
                        return;
                    }
                    this.set_open(false, window, cx);
                }),
            )
            .child(
                div()
                    .w(px(300.))
                    .mb(px(6.))
                    .flex()
                    .justify_between()
                    .child(sv_panel)
                    .child(hue_slider),
            )
            .child(footer);

        Some(
            deferred(
                anchored()
                    .position_mode(AnchoredPositionMode::Window)
                    .position(if below {
                        point(bounds.center().x, bounds.bottom())
                    } else {
                        point(bounds.center().x, bounds.top())
                    })
                    .anchor(if below {
                        Anchor::TopCenter
                    } else {
                        Anchor::BottomCenter
                    })
                    .snap_to_window_with_margin(px(4.))
                    .child(panel),
            )
            .with_priority(2),
        )
    }
}

impl Render for EpColorPicker {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let ep = ep();
        let disabled = self.disabled;
        let border = if self.open && !disabled {
            ep.primary
        } else if self.hovered && !disabled {
            ep.border_hover
        } else {
            ep.border
        };
        let (r, g, b) = theme::hex_to_rgb_channels(&self.value);
        let color: Hsla = theme::rgba_css(r, g, b, 1.);
        let weak = cx.entity().downgrade();
        div()
            .id(self.id.clone())
            .relative()
            .flex_none()
            .size(px(24.))
            .p(px(4.))
            .border_1()
            .border_color(border)
            .rounded(theme::radius::SM)
            .when(disabled, |this| {
                this.bg(ep.fill_light)
                    .cursor(CursorStyle::OperationNotAllowed)
            })
            .when(!disabled, |this| {
                this.cursor_pointer()
                    .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                        this.hovered = *hovered;
                        cx.notify();
                    }))
                    .on_click(cx.listener(|this, _, window, cx| {
                        let open = !this.open;
                        this.set_open(open, window, cx);
                    }))
                    .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                        if event.keystroke.key == "escape" && this.open {
                            this.set_open(false, window, cx);
                            cx.stop_propagation();
                        }
                    }))
            })
            .child(
                canvas(
                    move |bounds, _, cx| {
                        let _ = weak.update(cx, |this, _| this.trigger_bounds = Some(bounds));
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .inset_0(),
            )
            .child(
                div()
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .border_1()
                    .border_color(ep.text_secondary)
                    .rounded(px(2.))
                    .bg(color)
                    .when(disabled, |this| this.opacity(0.3))
                    .child(icon(EpIcon::ArrowDown, 10., ep.white)),
            )
            .children(self.render_panel(window, cx))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hsv_round_trips_through_rgb() {
        for &(r, g, b) in &[
            (0u8, 0u8, 0u8),
            (255, 255, 255),
            (0x1E, 0x29, 0x3B),
            (0x0F, 0x29, 0x42),
            (0x2E, 0x10, 0x65),
            (0x05, 0x2E, 0x24),
            (0x2B, 0x1A, 0x0F),
            (255, 0, 0),
        ] {
            let (h, s, v) = rgb_to_hsv(r, g, b);
            assert_eq!(hsv_to_rgb(h, s, v), (r, g, b), "{r},{g},{b}");
        }
        assert_eq!(to_hex(0x1E, 0x29, 0x3B), "#1E293B");
        assert_eq!(hsv_to_rgb(120., 1., 1.), (0, 255, 0));
    }
}
