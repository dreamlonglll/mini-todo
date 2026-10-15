//! `el-slider`（Element Plus 2.13 默认样式）：拖动中 `on_input`，松手 / 点击后 `on_change`（与 `@input` / `@change`
//! 语义一致，适合"拖动预览、松手落库"）。
//!
//! 外层高 32（small 24）；轨道 6px 高 `#E4E7ED`、圆角 3；已选段 `#409EFF`；拇指 20×20 白底 + 2px `#409EFF` 边框圆，
//! 悬停 / 拖动放大 1.2 倍，`cursor: grab / grabbing`；点击轨道跳到该值；禁用时已选段与拇指边框 `#A8ABB2`、不可操作。
//! `tooltip`（EP 默认开启）时悬停 / 拖动在拇指上方显示深色气泡。整个 32px 高的外层都是点击区（EP 只有轨道和拇指）。
//!
//! ```ignore
//! EpSlider::new("alpha", 45.)
//!     .range(5., 100.).step(5.)
//!     .tooltip(|v| format!("{v}%").into())
//!     .on_input(cx.listener(|this, v: &f32, _, cx| { this.preview = Some(*v); cx.notify(); }))
//!     .on_change(cx.listener(|this, v: &f32, window, cx| this.save(*v, window, cx)))
//! ```

use std::rc::Rc;

use gpui_kit::component::StyledExt as _;
use gpui_kit::{
    Anchor, AnchoredPositionMode, App, AppContext as _, Bounds, CursorStyle, DragMoveEvent,
    ElementId, Empty, InteractiveElement as _, IntoElement, MouseButton, ParentElement as _,
    Pixels, Render, RenderOnce, SharedString, StatefulInteractiveElement as _, StyleRefinement,
    Styled, Window, anchored, canvas, deferred, div, point, prelude::FluentBuilder as _, px,
};

use crate::assets::AppIcon;
use crate::theme::{self, ep};
use crate::ui::icon::icon;

type ValueHandler = Rc<dyn Fn(&f32, &mut Window, &mut App)>;
type Formatter = Rc<dyn Fn(f32) -> SharedString>;

/// 运行期状态（跨帧保留）
#[derive(Default)]
struct SliderRuntime {
    /// 轨道的窗口坐标（上一帧）
    bounds: Option<Bounds<Pixels>>,
    pressed: bool,
    dragging: bool,
    hovered: bool,
    /// 最近一次回调给宿主的值
    last: Option<f32>,
}

/// 拖动标记（`on_drag_move` 只认自己的）
#[derive(Clone)]
struct SliderDrag(gpui_kit::EntityId);

struct SliderDragView;

impl Render for SliderDragView {
    fn render(
        &mut self,
        _window: &mut Window,
        _cx: &mut gpui_kit::Context<Self>,
    ) -> impl IntoElement {
        Empty
    }
}

/// Element Plus 滑块
#[derive(IntoElement)]
pub struct EpSlider {
    id: ElementId,
    value: f32,
    min: f32,
    max: f32,
    step: f32,
    small: bool,
    disabled: bool,
    tooltip: Option<Formatter>,
    on_input: Option<ValueHandler>,
    on_change: Option<ValueHandler>,
    style: StyleRefinement,
}

impl EpSlider {
    pub fn new(id: impl Into<ElementId>, value: f32) -> Self {
        Self {
            id: id.into(),
            value,
            min: 0.,
            max: 100.,
            step: 1.,
            small: false,
            disabled: false,
            tooltip: None,
            on_input: None,
            on_change: None,
            style: StyleRefinement::default(),
        }
    }

    pub fn range(mut self, min: f32, max: f32) -> Self {
        self.min = min;
        self.max = max;
        self
    }

    pub fn step(mut self, step: f32) -> Self {
        self.step = step;
        self
    }

    /// `size="small"`（外层高 24）
    pub fn small(mut self) -> Self {
        self.small = true;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// 悬停 / 拖动时的气泡文字（`format-tooltip`）；不设则不显示气泡（`:show-tooltip="false"`）
    pub fn tooltip(mut self, format: impl Fn(f32) -> SharedString + 'static) -> Self {
        self.tooltip = Some(Rc::new(format));
        self
    }

    /// 拖动 / 点击过程中值变化（`@input`）
    pub fn on_input(mut self, f: impl Fn(&f32, &mut Window, &mut App) + 'static) -> Self {
        self.on_input = Some(Rc::new(f));
        self
    }

    /// 松手（`@change`）
    pub fn on_change(mut self, f: impl Fn(&f32, &mut Window, &mut App) + 'static) -> Self {
        self.on_change = Some(Rc::new(f));
        self
    }
}

impl Styled for EpSlider {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

/// 位置（相对轨道左端的比例 0~1）→ 按步长取整后的值（步长从 `min` 起算，与 EP 相同）
pub(crate) fn value_at(ratio: f32, min: f32, max: f32, step: f32) -> f32 {
    let ratio = ratio.clamp(0., 1.);
    let raw = min + (max - min) * ratio;
    let step = if step > 0. { step } else { 1. };
    let snapped = min + ((raw - min) / step).round() * step;
    snapped.clamp(min.min(max), max.max(min))
}

/// 值 → 比例
pub(crate) fn ratio_of(value: f32, min: f32, max: f32) -> f32 {
    if max <= min {
        0.
    } else {
        ((value - min) / (max - min)).clamp(0., 1.)
    }
}

impl RenderOnce for EpSlider {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let ep = ep();
        let runtime =
            window.use_keyed_state((self.id.clone(), "rt"), cx, |_, _| SliderRuntime::default());
        let rt_id = runtime.entity_id();
        let (bounds, hovered, dragging) = {
            let rt = runtime.read(cx);
            (rt.bounds, rt.hovered, rt.dragging || rt.pressed)
        };
        let (min, max, step) = (self.min, self.max, self.step);
        let ratio = ratio_of(self.value, min, max);
        let disabled = self.disabled;
        let main_color = if disabled {
            ep.text_placeholder
        } else {
            ep.primary
        };
        let active = !disabled && (hovered || dragging);
        let thumb_size = if active { 24. } else { 20. };

        // 指针位置 → 值（回调宿主，值变化时才回调）
        let apply = {
            let runtime = runtime.clone();
            let on_input = self.on_input.clone();
            move |x: Pixels, window: &mut Window, cx: &mut App| {
                let Some(bounds) = runtime.read(cx).bounds else {
                    return;
                };
                let width = f32::from(bounds.size.width).max(1.);
                let value = value_at(f32::from(x - bounds.left()) / width, min, max, step);
                let changed = runtime.update(cx, |rt, _| {
                    let changed = rt.last != Some(value);
                    rt.last = Some(value);
                    changed
                });
                if changed && let Some(handler) = &on_input {
                    handler(&value, window, cx);
                }
            }
        };
        let finish = {
            let runtime = runtime.clone();
            let on_change = self.on_change.clone();
            move |window: &mut Window, cx: &mut App| {
                let released = runtime.update(cx, |rt, cx| {
                    let was = rt.pressed || rt.dragging;
                    rt.pressed = false;
                    rt.dragging = false;
                    cx.notify();
                    was.then_some(rt.last)
                });
                if let Some(Some(value)) = released
                    && let Some(handler) = &on_change
                {
                    handler(&value, window, cx);
                }
            }
        };

        let bubble =
            self.tooltip
                .as_ref()
                .filter(|_| active)
                .zip(bounds)
                .map(|(format, bounds)| {
                    let center_x = bounds.left() + bounds.size.width * ratio;
                    // 拇指上沿（放大后 24px）再往上 5px 是箭头尖
                    let top = bounds.top() + bounds.size.height / 2. - px(thumb_size / 2. + 5.);
                    let body = div()
                        .font(theme::ui_font())
                        .whitespace_nowrap()
                        .px(px(11.))
                        .py(px(5.))
                        .text_size(px(12.))
                        .line_height(px(20.))
                        .text_color(ep.white)
                        .bg(ep.tooltip_bg)
                        .rounded(theme::radius::SM)
                        .child(format(self.value));
                    deferred(
                        anchored()
                            .position_mode(AnchoredPositionMode::Window)
                            .position(point(center_x, top))
                            .anchor(Anchor::BottomCenter)
                            .child(
                                div().flex().flex_col().items_center().child(body).child(
                                    icon(AppIcon::ArrowDownTip, 10., ep.tooltip_bg)
                                        .w(px(10.))
                                        .h(px(5.)),
                                ),
                            ),
                    )
                    .with_priority(3)
                });

        let bounds_rt = runtime.clone();
        let hover_rt = runtime.clone();
        let down_rt = runtime.clone();
        let apply_down = apply.clone();
        let apply_move = apply;
        let finish_up = finish.clone();
        let finish_out = finish;

        div()
            .id(self.id.clone())
            .flex()
            .items_center()
            .w_full()
            .h(px(if self.small { 24. } else { 32. }))
            .when(!disabled, |this| {
                this.cursor(if dragging {
                    CursorStyle::ClosedHand
                } else {
                    CursorStyle::PointingHand
                })
                .on_mouse_down(MouseButton::Left, move |event, window, cx| {
                    down_rt.update(cx, |rt, cx| {
                        rt.pressed = true;
                        cx.notify();
                    });
                    apply_down(event.position.x, window, cx);
                })
                .on_drag(SliderDrag(rt_id), |_, _, _, cx| cx.new(|_| SliderDragView))
                .on_drag_move::<SliderDrag>(move |event: &DragMoveEvent<SliderDrag>, window, cx| {
                    if event.drag(cx).0 != rt_id {
                        return;
                    }
                    let first = !runtime.read(cx).dragging;
                    if first {
                        runtime.update(cx, |rt, cx| {
                            rt.dragging = true;
                            cx.notify();
                        });
                    }
                    apply_move(event.event.position.x, window, cx);
                })
                .on_mouse_up(MouseButton::Left, move |_, window, cx| {
                    finish_up(window, cx)
                })
                .on_mouse_up_out(MouseButton::Left, move |_, window, cx| {
                    finish_out(window, cx)
                })
            })
            .when(disabled, |this| this.cursor_default())
            .child(
                div()
                    .relative()
                    .flex_1()
                    .h(px(6.))
                    .rounded(px(3.))
                    .bg(ep.border_light)
                    .child(
                        canvas(
                            move |bounds, _, cx| {
                                bounds_rt.update(cx, |rt, _| rt.bounds = Some(bounds));
                            },
                            |_, _, _, _| {},
                        )
                        .absolute()
                        .inset_0(),
                    )
                    .child(
                        div()
                            .absolute()
                            .left_0()
                            .top_0()
                            .h_full()
                            .w(gpui_kit::relative(ratio))
                            .rounded_l(px(3.))
                            .bg(main_color),
                    )
                    .child(
                        // 36×36 的拇指包裹层（EP `.el-slider__button-wrapper`），中心对准当前值
                        div()
                            .id((self.id.clone(), "thumb"))
                            .absolute()
                            .top(px(-15.))
                            .left(gpui_kit::relative(ratio))
                            .ml(px(-18.))
                            .size(px(36.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .on_hover(move |hovered, _, cx| {
                                let hovered = *hovered;
                                hover_rt.update(cx, |rt, cx| {
                                    if rt.hovered != hovered {
                                        rt.hovered = hovered;
                                        cx.notify();
                                    }
                                });
                            })
                            .when(!disabled, |this| {
                                this.cursor(if dragging {
                                    CursorStyle::ClosedHand
                                } else {
                                    CursorStyle::OpenHand
                                })
                            })
                            .child(
                                div()
                                    .size(px(thumb_size))
                                    .rounded_full()
                                    .bg(ep.white)
                                    .border_2()
                                    .border_color(main_color),
                            ),
                    ),
            )
            .children(bubble)
            .refine_style(&self.style)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapping_follows_element_plus_steps() {
        // 透明度 5~100 步长 5
        assert_eq!(value_at(0., 5., 100., 5.), 5.);
        assert_eq!(value_at(1., 5., 100., 5.), 100.);
        assert_eq!(value_at(0.42, 5., 100., 5.), 45.);
        assert_eq!(value_at(-1., 5., 100., 5.), 5.);
        // 字号 12~20 步长 1
        assert_eq!(value_at(0.5, 12., 20., 1.), 16.);
        assert_eq!(value_at(0.26, 12., 20., 1.), 14.);
        assert!((ratio_of(45., 5., 100.) - 40. / 95.).abs() < 1e-6);
        assert_eq!(ratio_of(3., 5., 5.), 0.);
    }
}
