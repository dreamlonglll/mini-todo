//! `el-color-picker size="small"`（Element Plus 2.13，不带 alpha，可选预设色）：有状态视图，构造宿主视图时创建。
//!
//! 触发器 24×24：`padding 4px`、1px `#DCDFE6` 边（悬停 `#C0C4CC`、展开 `#409EFF`）、圆角 4；内部色块 1px `#909399` 边、
//! 圆角 2，中间白色 ArrowDown（缩放 0.8）；无值时显示灰色 Close；禁用时底 `#F5F7FA`、色块 `opacity .3`、`cursor: not-allowed`。
//! 弹层（[`ep_popper`]，`offset 0`、无箭头、默认在触发器下方居中，放不下翻到上方）：浅色弹层、`padding 12px`、内容宽 300：
//! SV 面板 280×180（色相底 + 白→透明横向渐变 + 透明→黑纵向渐变，4px 圆形光标）+ 右侧竖向色相条 12×180（上下 2px 内边距，
//! 4px 高滑块）；有预设色时下方 8px 处一排 20×20、间距 8、圆角 4（与草稿同色的加 `box-shadow: 0 0 3px 2px #409EFF`）；
//! 底部 12px 处 hex 输入框（160px、small，回车 / 失焦解析）与 `Clear`（文字按钮）、`OK`（plain 按钮）——项目没配置
//! Element Plus 语言，内置文案是英文。拖动 / 点选只改草稿；点 `OK` 发 [`EpColorPickerEvent::Change`]（小写 `#rrggbb`，
//! 与 EP 的 TinyColor 输出一致），点 `Clear` 发 `Change(None)`；点外面 / Esc 收起并放弃草稿。
//!
//! ```ignore
//! let picker = cx.new(|cx| EpColorPicker::new("bg-color", window, cx).predefine(colors));
//! cx.subscribe_in(&picker, window, |this, _, e: &EpColorPickerEvent, window, cx| match e {
//!     EpColorPickerEvent::Change(color) => this.set_color(color.clone().unwrap_or("#000000".into()), window, cx),
//! }).detach();
//! picker.update(cx, |p, cx| p.set_value(Some("#1E293B".into()), cx));
//! ```

use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::{
    AppContext as _, ClickEvent, Context, CursorStyle, DragMoveEvent, ElementId, Empty, Entity,
    EventEmitter, FocusHandle, InteractiveElement as _, IntoElement, KeyDownEvent, MouseButton,
    MouseDownEvent, ParentElement as _, Pixels, Point, Render, StatefulInteractiveElement as _,
    Styled as _, Subscription, Window, div, linear_color_stop, linear_gradient,
    prelude::FluentBuilder as _, px,
};

use crate::assets::EpIcon;
use crate::theme::{self, ep};
use crate::ui::button::EpButton;
use crate::ui::ep_popper::{self, Anchor, Placement, PopperOptions};
use crate::ui::icon::icon;
use crate::ui::input::EpInput;

/// 确认（`Some("#rrggbb")`）或清空（`None`）
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EpColorPickerEvent {
    Change(Option<String>),
}

const SV_W: f32 = 280.;
const SV_H: f32 = 180.;
const HUE_W: f32 = 12.;
/// 弹层外框宽：内容 300 + padding 24 + 边框 2
const PANEL_WIDTH: f32 = 326.;

/// 弹层外框高度：边框 2 + padding 24 + SV 180 +（预设色 8 + 20）+ 页脚（12 + 24）
fn panel_height(has_predefine: bool) -> f32 {
    2. + 24. + SV_H + if has_predefine { 28. } else { 0. } + 36.
}

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

/// `#RGB` / `#RRGGBB` → HSV
pub fn hex_to_hsv(hex: &str) -> Option<(f32, f32, f32)> {
    let (r, g, b) = theme::parse_hex_color(hex)?;
    Some(rgb_to_hsv(r, g, b))
}

/// HSV → 小写 `#rrggbb`（EP 的输出格式）
pub fn hsv_to_hex(h: f32, s: f32, v: f32) -> String {
    let (r, g, b) = hsv_to_rgb(h, s, v);
    format!("#{r:02x}{g:02x}{b:02x}")
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
    /// 已确认的值
    value: Option<String>,
    predefine: Vec<String>,
    disabled: bool,
    open: bool,
    /// 草稿（HSV）
    hue: f32,
    sat: f32,
    val: f32,
    anchor: Anchor,
    sv_bounds: Anchor,
    hue_bounds: Anchor,
    hex_input: Entity<InputState>,
    focus: FocusHandle,
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
            value: None,
            predefine: Vec::new(),
            disabled: false,
            open: false,
            hue: 0.,
            sat: 0.,
            val: 0.,
            anchor: Anchor::new(),
            sv_bounds: Anchor::new(),
            hue_bounds: Anchor::new(),
            hex_input,
            focus: cx.focus_handle(),
            _subscriptions: subscriptions,
        }
    }

    /// 预设色（`:predefine`）
    pub fn predefine(mut self, colors: Vec<String>) -> Self {
        self.predefine = colors;
        self
    }

    pub fn value(&self) -> Option<String> {
        self.value.clone()
    }

    /// 设置已确认的值（不发事件）
    pub fn set_value(&mut self, value: Option<String>, cx: &mut Context<Self>) {
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

    pub fn is_open(&self) -> bool {
        self.open
    }

    /// 收起并放弃草稿（宿主处理 Esc 时用）
    pub fn close(&mut self, cx: &mut Context<Self>) {
        if self.open {
            self.open = false;
            cx.notify();
        }
    }

    fn draft_hex(&self) -> String {
        hsv_to_hex(self.hue, self.sat, self.val)
    }

    fn set_draft(&mut self, (h, s, v): (f32, f32, f32)) {
        // 灰度色没有色相：保留上次的色相，避免色相条跳回红色
        if s > 0. {
            self.hue = h;
        }
        self.sat = s;
        self.val = v;
    }

    fn sync_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let hex = self.draft_hex();
        self.hex_input
            .update(cx, |s, cx| s.set_value(hex, window, cx));
    }

    fn toggle(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.open {
            self.close(cx);
            return;
        }
        let draft = self
            .value
            .as_deref()
            .and_then(hex_to_hsv)
            .unwrap_or((self.hue, 0., 0.));
        self.set_draft(draft);
        self.open = true;
        self.sync_input(window, cx);
        self.focus.focus(window, cx);
        cx.notify();
    }

    fn apply_hex_text(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        if !self.open {
            return;
        }
        match hex_to_hsv(text) {
            Some(hsv) => {
                self.set_draft(hsv);
                cx.notify();
            }
            None => self.sync_input(window, cx),
        }
    }

    fn pick_sv(&mut self, position: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(bounds) = self.sv_bounds.bounds() else {
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
        let Some(bounds) = self.hue_bounds.bounds() else {
            return;
        };
        let height = f32::from(bounds.size.height).max(1.);
        let y = f32::from(position.y - bounds.top()).clamp(0., height);
        self.hue = (y / height * 360.).min(360.);
        self.sync_input(window, cx);
        cx.notify();
    }

    fn confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // 输入框里还没回车的文字也算（EP 失焦即解析）
        let text = self.hex_input.read(cx).value();
        if let Some(hsv) = hex_to_hsv(&text) {
            self.set_draft(hsv);
        }
        let hex = self.draft_hex();
        self.value = Some(hex.clone());
        self.close(cx);
        let _ = window;
        cx.emit(EpColorPickerEvent::Change(Some(hex)));
    }

    fn clear(&mut self, cx: &mut Context<Self>) {
        self.value = None;
        self.close(cx);
        cx.emit(EpColorPickerEvent::Change(None));
    }

    fn render_sv(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let ep = ep();
        let id = cx.entity_id();
        let (hr, hg, hb) = hsv_to_rgb(self.hue, 1., 1.);
        let white = theme::hex(0xFFFFFF);
        let black = theme::hex(0x000000);
        div()
            .id("sv")
            .relative()
            .w(px(SV_W))
            .h(px(SV_H))
            .bg(theme::rgba_css(hr, hg, hb, 1.))
            .cursor_pointer()
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
                    .left(px(self.sat * SV_W - 2.))
                    .top(px((1. - self.val) * SV_H - 2.))
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
                cx.listener(|this, event: &MouseDownEvent, window, cx| {
                    this.pick_sv(event.position, window, cx)
                }),
            )
            // 拖动用 GPUI 的拖拽：指针离开面板后仍然跟随
            .on_drag(SvDrag(id), |_, _, _, cx| cx.new(|_| DragGhost))
            .on_drag_move::<SvDrag>(cx.listener(
                move |this, event: &DragMoveEvent<SvDrag>, window, cx| {
                    if event.drag(cx).0 == id {
                        this.pick_sv(event.event.position, window, cx);
                    }
                },
            ))
    }

    fn render_hue(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let ep = ep();
        let id = cx.entity_id();
        // 6 段双色渐变拼接（GPUI 的线性渐变只有两个色标）
        let stops: [u32; 7] = [
            0xFF0000, 0xFFFF00, 0x00FF00, 0x00FFFF, 0x0000FF, 0xFF00FF, 0xFF0000,
        ];
        let segments = stops.windows(2).map(|pair| {
            div().flex_1().w_full().bg(linear_gradient(
                180.,
                linear_color_stop(theme::hex(pair[0]), 0.),
                linear_color_stop(theme::hex(pair[1]), 1.),
            ))
        });
        let thumb_top = self.hue / 360. * (SV_H - 4.);
        div()
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
                    .flex()
                    .flex_col()
                    .child(self.hue_bounds.measure())
                    .children(segments)
                    .child(
                        div()
                            .absolute()
                            .left_0()
                            .top(px(thumb_top))
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
                cx.listener(|this, event: &MouseDownEvent, window, cx| {
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
            ))
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
                    .id(("predefine", ix))
                    .size(px(20.))
                    .rounded(theme::radius::SM)
                    .bg(theme::todo_color(color, ep.white))
                    .cursor_pointer()
                    .when(selected, |this| {
                        this.shadow(vec![theme::shadow(0., 0., 3., 2., ep.primary)])
                    })
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        if let Some(hsv) = hex_to_hsv(&value) {
                            this.set_draft(hsv);
                            this.sync_input(window, cx);
                            cx.notify();
                        }
                    }))
            })
            .collect();
        let has_predefine = !swatches.is_empty();
        let footer =
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
                        .child(
                            EpButton::new("cp-clear")
                                .text()
                                .small()
                                .label("Clear")
                                .on_click(cx.listener(|this, _, _, cx| this.clear(cx))),
                        )
                        .child(
                            EpButton::new("cp-ok").plain().small().label("OK").on_click(
                                cx.listener(|this, _, window, cx| this.confirm(window, cx)),
                            ),
                        ),
                );
        ep_popper::frame()
            .id("cp-panel")
            .w(px(PANEL_WIDTH))
            .p(px(12.))
            .font(theme::ui_font())
            .child(
                div()
                    .w(px(300.))
                    .flex()
                    .justify_between()
                    .child(sv)
                    .child(hue),
            )
            // EP：`__wrapper` 下边距 6 与预设色上边距 8 折叠成 8，与页脚上边距 12 折叠成 12
            .when(has_predefine, |this| {
                this.child(
                    div()
                        .mt(px(8.))
                        .w(px(SV_W))
                        .flex()
                        .flex_wrap()
                        .gap(px(8.))
                        .children(swatches),
                )
            })
            .child(footer)
    }
}

impl Render for EpColorPicker {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let ep = ep();
        let open = self.open;
        let disabled = self.disabled;
        let inner = div()
            .size_full()
            .rounded(px(2.))
            .border_1()
            .border_color(ep.text_secondary)
            .flex()
            .items_center()
            .justify_center()
            .when(disabled, |this| this.opacity(0.3));
        let inner = match self.value.as_deref() {
            Some(color) => inner.bg(theme::todo_color(color, ep.white)).child(icon(
                EpIcon::ArrowDown,
                9.6,
                ep.white,
            )),
            None => inner.child(icon(EpIcon::Close, 9.6, ep.text_secondary)),
        };
        let popper = if open {
            let entity = cx.entity().downgrade();
            ep_popper::popper(
                &self.anchor,
                window,
                PopperOptions {
                    placement: Placement::Bottom,
                    flip: true,
                    // EP 颜色选择器 `:offset="0"`、`:show-arrow="false"`
                    gap: 0.,
                    arrow: false,
                    width: PANEL_WIDTH,
                    height: panel_height(!self.predefine.is_empty()),
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
            .id(self.id.clone())
            .track_focus(&self.focus)
            .relative()
            .flex_none()
            .size(px(24.))
            .p(px(4.))
            .border_1()
            .border_color(if open { ep.primary } else { ep.border })
            .rounded(theme::radius::SM)
            .when(disabled, |this| {
                this.bg(ep.fill_light)
                    .cursor(CursorStyle::OperationNotAllowed)
            })
            .when(!disabled, |this| {
                this.cursor_pointer()
                    .when(!open, |this| {
                        this.hover(move |s| s.border_color(ep.border_hover))
                    })
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, window, cx| {
                            this.toggle(window, cx);
                            cx.stop_propagation();
                        }),
                    )
            })
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape" && this.open {
                    this.close(cx);
                    cx.stop_propagation();
                }
            }))
            .child(self.anchor.measure())
            .child(inner)
            .children(popper)
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
        assert_eq!(hsv_to_rgb(120., 1., 1.), (0, 255, 0));
    }

    #[test]
    fn hex_hsv_round_trip_is_lower_case() {
        for hex in [
            "#ef4444", "#10b981", "#3b82f6", "#000000", "#ffffff", "#eab308",
        ] {
            let (h, s, v) = hex_to_hsv(hex).unwrap();
            assert_eq!(hsv_to_hex(h, s, v), hex, "{hex}");
        }
        let (h, s, v) = hex_to_hsv("#1E293B").unwrap();
        assert_eq!(hsv_to_hex(h, s, v), "#1e293b");
        assert_eq!(hex_to_hsv("red"), None);
        assert_eq!(hex_to_hsv("#FF0000"), Some((0., 1., 1.)));
    }

    #[test]
    fn panel_height_accounts_for_predefine_row() {
        assert_eq!(panel_height(false), 242.);
        assert_eq!(panel_height(true), 270.);
    }
}
