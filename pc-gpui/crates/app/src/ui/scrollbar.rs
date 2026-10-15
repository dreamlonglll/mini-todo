//! 细滚动条（旧版 `::-webkit-scrollbar` 定制：主列表宽 6px、四象限 4px）。
//!
//! 非覆盖式：内容溢出时在滚动区右侧占 `width` 宽（与 Chromium 一样，出现时内容区变窄）；
//! 轨道透明、滑块圆角 `width / 2`、悬停换色；滑块可拖动，滚轮由滚动区自己处理。
//!
//! ```ignore
//! // 视图里持有 ScrollHandle 与 ScrollbarState（构造视图时建）
//! div().flex().child(div().id("list").flex_1().overflow_y_scroll().track_scroll(&self.scroll).child(..))
//!     .children(ui::scrollbar::vertical("list-bar", &self.scroll, &self.bar_state, 6., thumb, thumb_hover))
//! ```
//!
//! 是否溢出取上一帧布局的结果；本帧布局后发现变化会请求重绘（滞回：显示中只在确实不溢出时才收起，
//! 避免"显示 → 变窄 → 溢出、收起 → 变宽 → 不溢出"来回跳）。

use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::{
    AnyElement, Hsla, InteractiveElement as _, IntoElement, MouseButton, MouseMoveEvent,
    MouseUpEvent, ParentElement as _, Pixels, ScrollHandle, Styled as _, canvas, div, point, px,
};

/// 滚动条的运行时状态（视图持有一份；`Rc` 共享给事件回调）
#[derive(Clone, Default)]
pub struct ScrollbarState {
    visible: Rc<Cell<bool>>,
    /// 拖动中：按下时的指针 y 与滚动偏移
    drag: Rc<Cell<Option<(f32, f32)>>>,
}

impl ScrollbarState {
    pub fn new() -> Self {
        Self::default()
    }

    /// 上一帧是否显示了滚动条
    pub fn is_visible(&self) -> bool {
        self.visible.get()
    }

    pub fn is_dragging(&self) -> bool {
        self.drag.get().is_some()
    }
}

/// 最小滑块高度（Chromium 约 18px）
const MIN_THUMB: f32 = 18.;

/// 滑块位置与高度（viewport / content / offset 都是正值）
fn thumb_geometry(viewport: f32, max_offset: f32, offset: f32) -> (f32, f32) {
    let content = viewport + max_offset;
    if content <= 0. || viewport <= 0. {
        return (0., 0.);
    }
    let height = (viewport * viewport / content).clamp(MIN_THUMB.min(viewport), viewport);
    let travel = (viewport - height).max(0.);
    let top = if max_offset > 0. {
        travel * (offset / max_offset).clamp(0., 1.)
    } else {
        0.
    };
    (top, height)
}

/// 垂直滚动条；不溢出时返回一个只负责"检测溢出变化"的零宽元素
pub fn vertical(
    id: &'static str,
    handle: &ScrollHandle,
    state: &ScrollbarState,
    width: f32,
    thumb: Hsla,
    thumb_hover: Hsla,
) -> AnyElement {
    let max = f32::from(handle.max_offset().y).max(0.);
    let shown = state.visible.get();
    // 溢出检测：本帧布局完成后读最新的 max_offset，与本帧的显示状态不一致就再画一帧
    let watcher = {
        let handle = handle.clone();
        let visible = state.visible.clone();
        canvas(
            move |_, window, _| {
                let overflow = f32::from(handle.max_offset().y) > 0.5;
                if overflow != visible.get() {
                    visible.set(overflow);
                    window.refresh();
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_0()
    };
    if !shown {
        return div().size_0().child(watcher).into_any_element();
    }

    let viewport = f32::from(handle.bounds().size.height);
    let offset = -f32::from(handle.offset().y);
    let (top, height) = thumb_geometry(viewport, max, offset);
    let dragging = state.drag.get().is_some();

    let drag_cell = state.drag.clone();
    let drag_events = {
        let drag = state.drag.clone();
        let handle = handle.clone();
        canvas(
            |_, _, _| {},
            move |_, _, window, _| {
                let move_drag = drag.clone();
                let move_handle = handle.clone();
                window.on_mouse_event(move |event: &MouseMoveEvent, _, window, _| {
                    let Some((start_y, start_offset)) = move_drag.get() else {
                        return;
                    };
                    if event.pressed_button != Some(MouseButton::Left) {
                        move_drag.set(None);
                        window.refresh();
                        return;
                    }
                    let viewport = f32::from(move_handle.bounds().size.height);
                    let max = f32::from(move_handle.max_offset().y).max(0.);
                    let (_, height) = thumb_geometry(viewport, max, start_offset);
                    let travel = (viewport - height).max(1.);
                    let delta = f32::from(event.position.y) - start_y;
                    let next = (start_offset + delta * max / travel).clamp(0., max);
                    move_handle.set_offset(point(move_handle.offset().x, px(-next)));
                    window.refresh();
                });
                let up_drag = drag.clone();
                window.on_mouse_event(move |_: &MouseUpEvent, _, window, _| {
                    if up_drag.take().is_some() {
                        window.refresh();
                    }
                });
            },
        )
        .absolute()
        .size_0()
    };

    div()
        .id(id)
        .relative()
        .flex_none()
        .w(px(width))
        .h_full()
        .child(watcher)
        .child(drag_events)
        .child(
            div()
                .id((id, 1usize))
                .absolute()
                .left_0()
                .top(px(top))
                .w(px(width))
                .h(px(height))
                .rounded(px(width / 2.))
                .bg(if dragging { thumb_hover } else { thumb })
                .hover(move |s| s.bg(thumb_hover))
                .on_mouse_down(MouseButton::Left, move |event, _, cx| {
                    drag_cell.set(Some((f32::from(event.position.y), offset)));
                    cx.stop_propagation();
                }),
        )
        .into_any_element()
}

/// 滚动区可视高度（逻辑像素；还没布局过时为 0）
pub fn viewport_height(handle: &ScrollHandle) -> Pixels {
    handle.bounds().size.height
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thumb_tracks_offset() {
        // 可视 100、内容 400：滑块 25 高，可走 75
        assert_eq!(thumb_geometry(100., 300., 0.), (0., 25.));
        assert_eq!(thumb_geometry(100., 300., 300.), (75., 25.));
        assert_eq!(thumb_geometry(100., 300., 150.), (37.5, 25.));
        // 内容极长：滑块不小于 18
        assert_eq!(thumb_geometry(100., 100_000., 0.).1, MIN_THUMB);
        assert_eq!(thumb_geometry(0., 10., 0.), (0., 0.));
    }
}
