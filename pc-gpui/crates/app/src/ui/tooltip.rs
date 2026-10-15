//! 两种 tooltip：
//!
//! 1. [`native`]：旧版大量使用的原生 `title` 属性（系统 tooltip，约 0.5s 延迟、出现在光标右下）。
//!    用 GPUI 自带的 `.tooltip(..)`（定位就是"光标 + 1px"，默认 500ms 延迟）：
//!    ```ignore
//!    div().id("settings-btn").tooltip(ui::tooltip::native("设置"))
//!    ```
//! 2. [`EpTooltip`]：Element Plus 深色 tooltip（`el-tooltip`，默认 effect dark）：底 `#303133`、白字 12px、
//!    行高 20px、padding `5px 11px`、圆角 4、宽 10 高 5 的箭头、距触发元素 12px、默认上方弹出（放不下翻到下方），
//!    `show-after` 按调用（300 / 500ms）、`hide-after` 200ms、淡入 0.2s：
//!    ```ignore
//!    EpTooltip::new("repeat-tip-3", "每天重复", badge).show_after_ms(300)
//!    ```

use std::time::Duration;

use gpui_kit::component::StyledExt as _;
use gpui_kit::{
    Anchor, AnchoredPositionMode, Animation, AnimationExt as _, AnyElement, AnyView, App,
    AppContext as _, Bounds, Context, ElementId, InteractiveElement as _, IntoElement,
    ParentElement as _, Pixels, Render, RenderOnce, SharedString, StatefulInteractiveElement as _,
    StyleRefinement, Styled, Window, anchored, canvas, deferred, div, point,
    prelude::FluentBuilder as _, px,
};

use crate::assets::AppIcon;
use crate::theme::{self, ep, hex};
use crate::ui::icon::icon;

/// 原生 `title` 风格的 tooltip 构造器（交给 `StatefulInteractiveElement::tooltip`）
pub fn native(
    text: impl Into<SharedString>,
) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    let text = text.into();
    move |_window, cx| {
        let text = text.clone();
        cx.new(|_| NativeTooltip { text }).into()
    }
}

/// 近似 Windows 11 系统 tooltip：浅底、细边、12px 文字
pub struct NativeTooltip {
    text: SharedString,
}

impl Render for NativeTooltip {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .font(theme::ui_font())
            .text_size(px(12.))
            .line_height(px(16.))
            .px(px(8.))
            .py(px(5.))
            .bg(hex(0xF9F9F9))
            .border_1()
            .border_color(theme::rgba_css(0, 0, 0, 0.15))
            .rounded(px(4.))
            .shadow(vec![theme::shadow(
                0.,
                2.,
                6.,
                0.,
                theme::rgba_css(0, 0, 0, 0.12),
            )])
            .text_color(hex(0x1A1A1A))
            .whitespace_nowrap()
            .child(self.text.clone())
    }
}

/// 弹出方向
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Placement {
    #[default]
    Top,
    Bottom,
}

/// 箭头尺寸与间距（EP：10×10 旋转 45° 的方块，外凸 5px；offset 12px）
const ARROW_W: f32 = 10.;
const ARROW_H: f32 = 5.;
const OFFSET: f32 = 12.;
/// 估算的单行气泡高度（5 + 20 + 5 + 边框 2），用来判断上方放不放得下
const BUBBLE_EST_H: f32 = 32.;

#[derive(Default)]
struct TipState {
    visible: bool,
    epoch: u64,
    trigger: Option<Bounds<Pixels>>,
}

/// Element Plus 深色 tooltip 包装：悬停在 `child` 上 `show_after` 后显示气泡
#[derive(IntoElement)]
pub struct EpTooltip {
    id: ElementId,
    content: SharedString,
    child: AnyElement,
    placement: Placement,
    show_after: Duration,
    max_width: Option<Pixels>,
    style: StyleRefinement,
}

impl EpTooltip {
    pub fn new(
        id: impl Into<ElementId>,
        content: impl Into<SharedString>,
        child: impl IntoElement,
    ) -> Self {
        Self {
            id: id.into(),
            content: content.into(),
            child: child.into_any_element(),
            placement: Placement::Top,
            show_after: Duration::ZERO,
            max_width: None,
            style: StyleRefinement::default(),
        }
    }

    pub fn placement(mut self, placement: Placement) -> Self {
        self.placement = placement;
        self
    }

    /// `show-after`（毫秒）
    pub fn show_after_ms(mut self, ms: u64) -> Self {
        self.show_after = Duration::from_millis(ms);
        self
    }

    /// 气泡最大宽度（超出折行；默认不限，单行）
    pub fn max_width(mut self, width: Pixels) -> Self {
        self.max_width = Some(width);
        self
    }
}

impl Styled for EpTooltip {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl RenderOnce for EpTooltip {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state(self.id.clone(), cx, |_, _| TipState::default());
        let (visible, trigger) = {
            let s = state.read(cx);
            (s.visible, s.trigger)
        };
        let show_after = self.show_after;
        let hover_state = state.clone();
        let bounds_state = state.clone();

        let bubble = trigger.filter(|_| visible).map(|trigger| {
            let below = self.placement == Placement::Bottom
                || f32::from(trigger.top()) < OFFSET + ARROW_H + BUBBLE_EST_H;
            let center_x = trigger.center().x;
            let (position, anchor) = if below {
                (
                    point(center_x, trigger.bottom() + px(OFFSET - ARROW_H)),
                    Anchor::TopCenter,
                )
            } else {
                (
                    point(center_x, trigger.top() - px(OFFSET - ARROW_H)),
                    Anchor::BottomCenter,
                )
            };
            let arrow = |tip: AppIcon| {
                icon(tip, ARROW_W, ep().tooltip_bg)
                    .h(px(ARROW_H))
                    .w(px(ARROW_W))
            };
            let body = div()
                .font(theme::ui_font())
                .min_w(px(10.))
                .when_some(self.max_width, |this, w| this.max_w(w))
                .when(self.max_width.is_none(), |this| this.whitespace_nowrap())
                .px(px(11.))
                .py(px(5.))
                .text_size(px(12.))
                .line_height(px(20.))
                .text_color(ep().white)
                .bg(ep().tooltip_bg)
                .border_1()
                .border_color(ep().tooltip_bg)
                .rounded(theme::radius::SM)
                .child(self.content.clone());
            let column = div()
                .flex()
                .flex_col()
                .items_center()
                .when(below, |this| this.child(arrow(AppIcon::ArrowUpTip)))
                .child(body)
                .when(!below, |this| this.child(arrow(AppIcon::ArrowDownTip)))
                .with_animation(
                    (self.id.clone(), "fade"),
                    Animation::new(theme::duration::BASE),
                    |el, delta| el.opacity(delta),
                );
            deferred(
                anchored()
                    .position_mode(AnchoredPositionMode::Window)
                    .position(position)
                    .anchor(anchor)
                    .child(column),
            )
            .with_priority(3)
        });

        div()
            .id(self.id)
            .flex()
            .relative()
            .child(self.child)
            .child(
                canvas(
                    move |bounds, _window, cx| {
                        bounds_state.update(cx, |s, _| s.trigger = Some(bounds));
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .inset_0(),
            )
            .on_hover(move |hovered, _window, cx| {
                let hovered = *hovered;
                hover_state.update(cx, |s, cx| {
                    s.epoch += 1;
                    let epoch = s.epoch;
                    let delay = if hovered {
                        show_after
                    } else {
                        theme::TOOLTIP_HIDE_AFTER
                    };
                    cx.spawn(async move |this, cx| {
                        if !delay.is_zero() {
                            cx.background_executor().timer(delay).await;
                        }
                        let _ = this.update(cx, |s, cx| {
                            if s.epoch == epoch && s.visible != hovered {
                                s.visible = hovered;
                                cx.notify();
                            }
                        });
                    })
                    .detach();
                });
            })
            .children(bubble)
            .refine_style(&self.style)
    }
}
