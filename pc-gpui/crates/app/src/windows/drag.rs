//! 自绘移动 / 缩放的公共部分：收纳箱（`windows/fence/`）与清单 / 日历窗口（桌面块）共用。契约见
//! `.trellis/spec/pc-gpui/gpui-conventions.md` §2.3 与 `.trellis/spec/backend/desktop-fences.md`「移动」「缩放」「吸附」。
//!
//! - 纯逻辑：拖动阈值、缩放方向与 [`resize_rect`]、JS 取整（单测向量来自旧前端 `fence.test.ts`，原在 `windows/fence/logic.rs`）
//! - GPUI 小工具：屏幕物理坐标的光标与位移（[`cursor_point`] / [`cursor_delta`]）、按客户区摆窗（[`place_client_rect`]）、
//!   空的拖动预览（[`EmptyDragView`]）、缩放手柄的位置与光标（[`resize_handle`]）、吸附高亮叠层（[`render_snap_guides`]，
//!   画在被对齐的**目标**窗口里）

use gpui_kit::{
    App, Context, CursorStyle, Div, Empty, InteractiveElement as _, IntoElement, Pixels, Point,
    Render, SharedString, Stateful, Styled as _, Window, div, prelude::FluentBuilder as _, px,
};
use mini_todo_core::fence::PxRect;

use super::snap::{SnapEdge, SnapEdges};
use crate::platform;
use crate::theme;

/// 当前光标：Windows / macOS 用屏幕物理坐标（拖动时窗口跟着动，窗口坐标不可靠），第二项为 true；Linux 取不到，
/// 用窗口逻辑坐标 + 窗口原点（第二项 false）
pub fn cursor_point(position: Point<Pixels>, window: &Window) -> ((f64, f64), bool) {
    match platform::cursor_position_physical() {
        Some((x, y)) => ((f64::from(x), f64::from(y)), true),
        None => {
            let origin = window.bounds().origin;
            (
                (
                    f64::from(f32::from(position.x + origin.x)),
                    f64::from(f32::from(position.y + origin.y)),
                ),
                false,
            )
        }
    }
}

/// 光标相对按下时（`start`，[`cursor_point`] 的结果）移动了多少物理像素：Linux 的窗口逻辑坐标乘系数换成物理像素
pub fn cursor_delta(
    start: (f64, f64),
    scale: f64,
    position: Point<Pixels>,
    window: &Window,
) -> (f64, f64) {
    let ((x, y), physical) = cursor_point(position, window);
    let factor = if physical { 1. } else { scale };
    ((x - start.0) * factor, (y - start.1) * factor)
}

/// 把窗口的客户区（= 可见区域）摆到 `rect`（物理像素）。Windows：换算成外框一次 `SetWindowPos`；macOS：`setFrame:`
/// （都延后执行，`scale` 用不到）；Linux GPUI 不能移动窗口，只按 `scale` 改尺寸（截图 / 冒烟测试用）
pub fn place_client_rect(window: &mut Window, cx: &App, rect: PxRect, scale: f64) {
    #[cfg(any(windows, target_os = "macos"))]
    {
        let _ = scale;
        platform::set_client_rect_physical(
            window,
            cx,
            platform::PhysRect::new(rect.x, rect.y, rect.width, rect.height),
        );
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = cx;
        let s = if scale > 0. { scale } else { 1. } as f32;
        window.resize(gpui_kit::size(
            px(rect.width as f32 / s),
            px(rect.height as f32 / s),
        ));
    }
}

/// GPUI 拖动要一个跟着光标画的预览视图：自绘拖动不画任何东西
pub struct EmptyDragView;

impl Render for EmptyDragView {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        Empty
    }
}

/// 一个缩放手柄（只有位置与光标；调用方挂按下 / 拖动 / 松手的监听，收纳箱另外 `occlude()`）：贴着窗口内侧的边，
/// 上下边 5px 高、左右边 5px 宽（离角 10px），四个角 10×10。放在 `relative()` 的根元素里
pub fn resize_handle(dir: ResizeDir) -> Stateful<Div> {
    let base = div()
        .id(SharedString::from(format!("resize-{}", dir.key())))
        .absolute();
    let base = match dir {
        ResizeDir::N => base.top_0().left(px(10.)).right(px(10.)).h(px(5.)),
        ResizeDir::S => base.bottom_0().left(px(10.)).right(px(10.)).h(px(5.)),
        ResizeDir::W => base.left_0().top(px(10.)).bottom(px(10.)).w(px(5.)),
        ResizeDir::E => base.right_0().top(px(10.)).bottom(px(10.)).w(px(5.)),
        ResizeDir::NW => base.top_0().left_0().size(px(10.)),
        ResizeDir::NE => base.top_0().right_0().size(px(10.)),
        ResizeDir::SW => base.bottom_0().left_0().size(px(10.)),
        ResizeDir::SE => base.bottom_0().right_0().size(px(10.)),
    };
    base.cursor(resize_cursor(dir))
}

/// 缩放手柄的光标
pub fn resize_cursor(dir: ResizeDir) -> CursorStyle {
    match dir {
        ResizeDir::N | ResizeDir::S => CursorStyle::ResizeUpDown,
        ResizeDir::W | ResizeDir::E => CursorStyle::ResizeLeftRight,
        ResizeDir::NW | ResizeDir::SE => CursorStyle::ResizeUpLeftDownRight,
        ResizeDir::NE | ResizeDir::SW => CursorStyle::ResizeUpRightDownLeft,
    }
}

/// 按下后移动超过阈值（逻辑像素，默认 3）才算拖动
pub fn exceeds_drag_threshold(dx: f64, dy: f64, threshold: f64) -> bool {
    dx.abs() > threshold || dy.abs() > threshold
}

/// 缩放方向（8 个手柄）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResizeDir {
    N,
    S,
    W,
    E,
    NW,
    NE,
    SW,
    SE,
}

impl ResizeDir {
    pub const ALL: [ResizeDir; 8] = [
        ResizeDir::N,
        ResizeDir::S,
        ResizeDir::W,
        ResizeDir::E,
        ResizeDir::NW,
        ResizeDir::NE,
        ResizeDir::SW,
        ResizeDir::SE,
    ];

    /// 拖动的是否含上边（N / NW / NE）；`has_s` / `has_w` / `has_e` 同理
    pub fn has_n(self) -> bool {
        matches!(self, ResizeDir::N | ResizeDir::NW | ResizeDir::NE)
    }
    pub fn has_s(self) -> bool {
        matches!(self, ResizeDir::S | ResizeDir::SW | ResizeDir::SE)
    }
    pub fn has_w(self) -> bool {
        matches!(self, ResizeDir::W | ResizeDir::NW | ResizeDir::SW)
    }
    pub fn has_e(self) -> bool {
        matches!(self, ResizeDir::E | ResizeDir::NE | ResizeDir::SE)
    }

    pub fn key(self) -> &'static str {
        match self {
            ResizeDir::N => "n",
            ResizeDir::S => "s",
            ResizeDir::W => "w",
            ResizeDir::E => "e",
            ResizeDir::NW => "nw",
            ResizeDir::NE => "ne",
            ResizeDir::SW => "sw",
            ResizeDir::SE => "se",
        }
    }
}

/// JS `Math.round`：逢 .5 向 +∞
pub fn js_round(v: f64) -> i32 {
    (v + 0.5).floor() as i32
}

/// 拖哪条边改哪条边，对边不动；不小于最小尺寸（拖左 / 上边缩到最小就停，右下角不被推走）
pub fn resize_rect(
    start: PxRect,
    dir: ResizeDir,
    dx: f64,
    dy: f64,
    min_w: f64,
    min_h: f64,
) -> PxRect {
    let (mut x, mut y) = (start.x as f64, start.y as f64);
    let (mut w, mut h) = (start.width as f64, start.height as f64);
    if dir.has_e() {
        w = (start.width as f64 + dx).max(min_w);
    }
    if dir.has_s() {
        h = (start.height as f64 + dy).max(min_h);
    }
    if dir.has_w() {
        w = (start.width as f64 - dx).max(min_w);
        x = start.x as f64 + start.width as f64 - w;
    }
    if dir.has_n() {
        h = (start.height as f64 - dy).max(min_h);
        y = start.y as f64 + start.height as f64 - h;
    }
    PxRect {
        x: js_round(x),
        y: js_round(y),
        width: js_round(w),
        height: js_round(h),
    }
}

/// 吸附高亮：被对齐的边画一条主题色线（`theme::snap_guide_color()`、`SNAP_GUIDE_WIDTH`），画在窗口内的边缘、跟着窗口圆角
/// `radius`（收纳箱 `radius::MD`，折叠时就是标题栏的边；经典固定的清单窗口没有圆角传 0）；没有命中区，不挡鼠标。
/// 放在窗口根元素（`relative()`）的最后
pub fn render_snap_guides(edges: SnapEdges, radius: Pixels) -> impl IntoElement {
    let width = theme::SNAP_GUIDE_WIDTH;
    div()
        .absolute()
        .inset_0()
        .rounded(radius)
        .border_color(theme::snap_guide_color())
        .when(edges.contains(SnapEdge::Top), |this| this.border_t(width))
        .when(edges.contains(SnapEdge::Bottom), |this| {
            this.border_b(width)
        })
        .when(edges.contains(SnapEdge::Left), |this| this.border_l(width))
        .when(edges.contains(SnapEdge::Right), |this| this.border_r(width))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drag_threshold() {
        assert!(!exceeds_drag_threshold(0., 0., 3.));
        assert!(!exceeds_drag_threshold(3., -3., 3.));
        assert!(exceeds_drag_threshold(4., 0., 3.));
        assert!(exceeds_drag_threshold(0., -4., 3.));
    }

    #[test]
    fn resize_vectors() {
        let start = PxRect {
            x: 100,
            y: 200,
            width: 552,
            height: 504,
        };
        let r = |dir, dx, dy| {
            let r = resize_rect(start, dir, dx, dy, 240., 240.);
            (r.x, r.y, r.width, r.height)
        };
        assert_eq!(r(ResizeDir::SE, 30., 40.), (100, 200, 582, 544));
        assert_eq!(r(ResizeDir::E, 30., 40.), (100, 200, 582, 504));
        assert_eq!(r(ResizeDir::S, 30., 40.), (100, 200, 552, 544));
        assert_eq!(r(ResizeDir::NW, -20., -10.), (80, 190, 572, 514));
        assert_eq!(r(ResizeDir::NE, 20., 10.), (100, 210, 572, 494));
        assert_eq!(r(ResizeDir::SW, 20., 10.), (120, 200, 532, 514));
        assert_eq!(r(ResizeDir::SE, -1000., -1000.), (100, 200, 240, 240));
        assert_eq!(r(ResizeDir::NW, 1000., 1000.), (412, 464, 240, 240));
        assert_eq!(r(ResizeDir::E, 10.4, 0.).2, 562);
    }
}
