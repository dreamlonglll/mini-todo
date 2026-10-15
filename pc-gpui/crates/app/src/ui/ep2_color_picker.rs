//! `el-color-picker size="small"`（Element Plus 2.13，无透明度，带预设色）。
//!
//! 触发器 24×24：`padding 4px`、1px `#DCDFE6` 边（hover `#C0C4CC`、展开 `#409EFF`）、圆角 4；内部色块 1px `#909399`
//! 边、圆角 2，中间白色 ArrowDown（缩放 0.8）；无值时显示灰色 Close。
//! 面板（浅色弹层、无箭头、`padding 12px`、内容宽 300）：SV 面板 280×180 + 右侧竖向色相条 12×180；下方 6px 处
//! 预设色 20×20、间距 8、圆角 4（选中 `box-shadow: 0 0 3px 2px #409EFF`）；底部 12px 处 hex 输入框（160px、small）
//! 与 `Clear`（文字按钮）、`OK`（默认按钮）。拖动 / 点选只改草稿，OK 提交（小写 hex，与 EP 一致），Clear 清空，
//! 点外面关闭并放弃草稿。默认在触发器下方居中，放不下翻到上方。

use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::{
    AppContext as _, ClickEvent, Context, Entity, EventEmitter, InteractiveElement as _,
    IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent, ParentElement as _, Pixels, Point,
    Render, SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, Window, div,
    linear_color_stop, linear_gradient, prelude::FluentBuilder as _, px,
};

use crate::assets::EpIcon;
use crate::theme::{self, ep};
use crate::ui::button::EpButton;
use crate::ui::ep2_popper::{self, Anchor, Placement, PopperOptions};
use crate::ui::icon::icon;
use crate::ui::input::EpInput;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EpColorPickerEvent {
    /// 新颜色（小写 `#rrggbb`）或清空
    Change(Option<String>),
}

const SV_WIDTH: f32 = 280.;
const SV_HEIGHT: f32 = 180.;
const HUE_WIDTH: f32 = 12.;
pub const PANEL_WIDTH: f32 = 326.;
pub const PANEL_HEIGHT: f32 = 268.;

/// `#RGB` / `#RRGGBB` → HSV（h 0..360、s / v 0..1）
pub fn hex_to_hsv(hex: &str) -> Option<(f32, f32, f32)> {
    let (r, g, b) = theme::parse_hex_color(hex)?;
    let (r, g, b) = (
        f32::from(r) / 255.,
        f32::from(g) / 255.,
        f32::from(b) / 255.,
    );
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
    Some((h, s, max))
}

pub fn hsv_to_rgb(h: f32, s: f32, v: f32) -> (u8, u8, u8) {
    let c = v * s;
    let hp = (h.rem_euclid(360.)) / 60.;
    let x = c * (1. - (hp.rem_euclid(2.) - 1.).abs());
    let (r, g, b) = match hp as u32 {
        0 => (c, x, 0.),
        1 => (x, c, 0.),
        2 => (0., c, x),
        3 => (0., x, c),
        4 => (x, 0., c),
        _ => (c, 0., x),
    };
    let m = v - c;
    let to = |c: f32| ((c + m) * 255.).round().clamp(0., 255.) as u8;
    (to(r), to(g), to(b))
}

/// HSV → 小写 `#rrggbb`
pub fn hsv_to_hex(h: f32, s: f32, v: f32) -> String {
    let (r, g, b) = hsv_to_rgb(h, s, v);
    format!("#{r:02x}{g:02x}{b:02x}")
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Drag {
    Sv,
    Hue,
}

pub struct EpColorPicker {
    id: SharedString,
    value: Option<String>,
    predefine: Vec<String>,
    open: bool,
    hsv: (f32, f32, f32),
    hex_input: Entity<InputState>,
    drag: Option<Drag>,
    anchor: Anchor,
    sv_bounds: Anchor,
    hue_bounds: Anchor,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<EpColorPickerEvent> for EpColorPicker {}

impl EpColorPicker {
    pub fn new(
        id: impl Into<SharedString>,
        predefine: Vec<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let hex_input = cx.new(|cx| InputState::new(window, cx));
        let subscriptions = vec![cx.subscribe_in(
            &hex_input,
            window,
            |this, _, event: &InputEvent, window, cx| {
                if let InputEvent::PressEnter { .. } = event {
                    let text = this.hex_input.read(cx).value().to_string();
                    match hex_to_hsv(&text) {
                        Some(hsv) => this.hsv = hsv,
                        None => this.sync_hex(window, cx),
                    }
                    cx.notify();
                }
            },
        )];
        Self {
            id: id.into(),
            value: None,
            predefine,
            open: false,
            hsv: (0., 0., 0.),
            hex_input,
            drag: None,
            anchor: Anchor::new(),
            sv_bounds: Anchor::new(),
            hue_bounds: Anchor::new(),
            _subscriptions: subscriptions,
        }
    }

    pub fn value(&self) -> Option<String> {
        self.value.clone()
    }

    /// 外部赋值（不发事件）
    pub fn set_value(&mut self, value: Option<String>, cx: &mut Context<Self>) {
        self.value = value;
        cx.notify();
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn close(&mut self, cx: &mut Context<Self>) {
        if self.open {
            self.open = false;
            self.drag = None;
            cx.notify();
        }
    }

    fn draft_hex(&self) -> String {
        let (h, s, v) = self.hsv;
        hsv_to_hex(h, s, v)
    }

    fn sync_hex(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let hex = self.draft_hex();
        self.hex_input
            .update(cx, |state, cx| state.set_value(hex, window, cx));
    }

    fn toggle(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.open {
            self.close(cx);
            return;
        }
        self.hsv = self
            .value
            .as_deref()
            .and_then(hex_to_hsv)
            .unwrap_or((0., 0., 0.));
        self.open = true;
        self.sync_hex(window, cx);
        cx.notify();
    }

    fn update_from(
        &mut self,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self.drag {
            Some(Drag::Sv) => {
                if let Some(b) = self.sv_bounds.bounds() {
                    let x = (f32::from(position.x - b.origin.x) / f32::from(b.size.width))
                        .clamp(0., 1.);
                    let y = (f32::from(position.y - b.origin.y) / f32::from(b.size.height))
                        .clamp(0., 1.);
                    self.hsv.1 = x;
                    self.hsv.2 = 1. - y;
                }
            }
            Some(Drag::Hue) => {
                if let Some(b) = self.hue_bounds.bounds() {
                    let y = (f32::from(position.y - b.origin.y) / f32::from(b.size.height))
                        .clamp(0., 1.);
                    self.hsv.0 = y * 360.;
                }
            }
            None => return,
        }
        self.sync_hex(window, cx);
        cx.notify();
    }

    fn render_sv(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let (h, s, v) = self.hsv;
        let (r, g, b) = hsv_to_rgb(h, 1., 1.);
        let hue = theme::rgba_css(r, g, b, 1.);
        let white = theme::hex(0xFFFFFF);
        let black = theme::hex(0x000000);
        div()
            .id(SharedString::from(format!("{}-sv", self.id)))
            .relative()
            .w(px(SV_WIDTH))
            .h(px(SV_HEIGHT))
            .bg(hue)
            .child(self.sv_bounds.measure())
            .child(div().absolute().inset_0().bg(linear_gradient(
                90.,
                linear_color_stop(white, 0.),
                linear_color_stop(white.opacity(0.), 1.),
            )))
            .child(div().absolute().inset_0().bg(linear_gradient(
                0.,
                linear_color_stop(black, 0.),
                linear_color_stop(black.opacity(0.), 1.),
            )))
            .child(
                div()
                    .absolute()
                    .left(px(s * SV_WIDTH - 2.))
                    .top(px((1. - v) * SV_HEIGHT - 2.))
                    .size(px(4.))
                    .rounded_full()
                    .border(px(1.5))
                    .border_color(white)
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
                cx.listener(|this, event: &MouseDownEvent, window, cx| {
                    this.drag = Some(Drag::Sv);
                    this.update_from(event.position, window, cx);
                }),
            )
    }

    fn render_hue(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let stops: [u32; 7] = [
            0xFF0000, 0xFFFF00, 0x00FF00, 0x00FFFF, 0x0000FF, 0xFF00FF, 0xFF0000,
        ];
        let positions = [0., 0.17, 0.33, 0.5, 0.67, 0.83, 1.];
        let bar_height = SV_HEIGHT - 4.;
        let segments = (0..6).map(|i| {
            let h = (positions[i + 1] - positions[i]) * bar_height;
            div().w_full().h(px(h)).bg(linear_gradient(
                180.,
                linear_color_stop(theme::hex(stops[i]), 0.),
                linear_color_stop(theme::hex(stops[i + 1]), 1.),
            ))
        });
        let thumb_top = self.hsv.0 / 360. * (SV_HEIGHT - 4.);
        div()
            .id(SharedString::from(format!("{}-hue", self.id)))
            .relative()
            .w(px(HUE_WIDTH))
            .h(px(SV_HEIGHT))
            .py(px(2.))
            .bg(theme::hex(0xFF0000))
            .child(self.hue_bounds.measure())
            .children(segments)
            .child(
                div()
                    .absolute()
                    .left_0()
                    .top(px(thumb_top))
                    .w_full()
                    .h(px(4.))
                    .bg(theme::hex(0xFFFFFF))
                    .border_1()
                    .border_color(ep().border_lighter)
                    .rounded(px(1.))
                    .shadow(vec![theme::shadow(
                        0.,
                        0.,
                        2.,
                        0.,
                        theme::rgba_css(0, 0, 0, 0.6),
                    )]),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, window, cx| {
                    this.drag = Some(Drag::Hue);
                    this.update_from(event.position, window, cx);
                }),
            )
    }

    fn render_panel(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let ep = ep();
        let draft = self.draft_hex();
        let sv = self.render_sv(cx).into_any_element();
        let hue = self.render_hue(cx).into_any_element();
        let swatches: Vec<_> = self
            .predefine
            .iter()
            .enumerate()
            .map(|(ix, color)| {
                let selected = color.eq_ignore_ascii_case(&draft);
                let value = color.clone();
                div()
                    .id(SharedString::from(format!("{}-pre-{ix}", self.id)))
                    .size(px(20.))
                    .rounded(theme::radius::SM)
                    .bg(theme::todo_color(color, ep.white))
                    .cursor_pointer()
                    .when(selected, |this| {
                        this.shadow(vec![theme::shadow(0., 0., 3., 2., ep.primary)])
                    })
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        if let Some(hsv) = hex_to_hsv(&value) {
                            this.hsv = hsv;
                            this.sync_hex(window, cx);
                            cx.notify();
                        }
                    }))
            })
            .collect();
        let clear = EpButton::new(SharedString::from(format!("{}-clear", self.id)))
            .text()
            .small()
            .label("Clear")
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.open = false;
                this.value = None;
                cx.emit(EpColorPickerEvent::Change(None));
                cx.notify();
            }));
        let ok = EpButton::new(SharedString::from(format!("{}-ok", self.id)))
            .small()
            .label("OK")
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.open = false;
                let hex = this.draft_hex();
                if this.value.as_deref() != Some(hex.as_str()) {
                    this.value = Some(hex.clone());
                    cx.emit(EpColorPickerEvent::Change(Some(hex)));
                }
                cx.notify();
            }));
        ep2_popper::frame()
            .id(SharedString::from(format!("{}-panel", self.id)))
            .w(px(PANEL_WIDTH))
            .p(px(12.))
            .font(theme::ui_font())
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, window, cx| {
                if event.pressed_button == Some(MouseButton::Left) {
                    this.update_from(event.position, window, cx);
                } else if this.drag.is_some() {
                    this.drag = None;
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, _| this.drag = None),
            )
            .child(div().flex().gap(px(8.)).child(sv).child(hue))
            .child(
                div()
                    .mt(px(6.))
                    .w(px(SV_WIDTH))
                    .flex()
                    .flex_wrap()
                    .gap(px(8.))
                    .children(swatches),
            )
            .child(
                div()
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
                            .child(clear)
                            .child(ok),
                    ),
            )
    }
}

impl Render for EpColorPicker {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let ep = ep();
        let open = self.open;
        let inner = match self.value.as_deref() {
            Some(color) => div()
                .size_full()
                .rounded(px(2.))
                .border_1()
                .border_color(ep.text_secondary)
                .bg(theme::todo_color(color, ep.white))
                .flex()
                .items_center()
                .justify_center()
                .child(icon(EpIcon::ArrowDown, 9.6, ep.white)),
            None => div()
                .size_full()
                .rounded(px(2.))
                .border_1()
                .border_color(ep.text_secondary)
                .flex()
                .items_center()
                .justify_center()
                .child(icon(EpIcon::Close, 9.6, ep.text_secondary)),
        };
        let popper = if open {
            let entity = cx.entity().downgrade();
            ep2_popper::popper(
                &self.anchor,
                window,
                PopperOptions {
                    placement: Placement::Bottom,
                    flip: true,
                    gap: 12.,
                    arrow: false,
                    width: PANEL_WIDTH,
                    height: PANEL_HEIGHT,
                },
                self.render_panel(cx),
                move |_, cx| {
                    let _ = entity.update(cx, |this, cx| this.close(cx));
                },
            )
        } else {
            None
        };
        div()
            .id(SharedString::from(format!("{}-trigger", self.id)))
            .relative()
            .size(px(24.))
            .flex_none()
            .p(px(4.))
            .border_1()
            .border_color(if open { ep.primary } else { ep.border })
            .rounded(theme::radius::SM)
            .cursor_pointer()
            .when(!open, |this| {
                this.hover(|s| s.border_color(ep.border_hover))
            })
            .child(self.anchor.measure())
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    this.toggle(window, cx);
                    cx.stop_propagation();
                }),
            )
            .child(inner)
            .children(popper)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_hsv_round_trip() {
        for hex in [
            "#ef4444", "#10b981", "#3b82f6", "#000000", "#ffffff", "#eab308",
        ] {
            let (h, s, v) = hex_to_hsv(hex).unwrap();
            assert_eq!(hsv_to_hex(h, s, v), hex, "{hex}");
        }
        assert_eq!(hex_to_hsv("red"), None);
        let (h, s, v) = hex_to_hsv("#FF0000").unwrap();
        assert_eq!((h, s, v), (0., 1., 1.));
    }
}
