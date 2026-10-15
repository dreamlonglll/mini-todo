//! 四象限视图（旧 `QuadrantView.vue`，规格 §8）：2×2 网格，每格 头部（色点 + 名称 + 计数胶囊）+ 待办列表；
//! 待办可在象限内排序、拖到别的象限（颜色仍是旧象限默认色时跟随新象限）。
//!
//! "观察到的实际效果"按旧版真实渲染还原（以参考截图为准）：
//! - 非日历模式下网格高度由内容决定（两行等高 = 最高的格子），不撑满窗口；整个主内容区滚动
//! - 日历分栏模式下撑满左栏，各格内部滚动
//! - 空象限看不到"暂无待办"（原版被 `min-height: 100%` 的列表挤出可视区），只占位 60px

use gpui_kit::{
    Context, DragMoveEvent, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, Window, canvas, div, prelude::FluentBuilder as _,
    px, relative,
};

use super::todo_item::{RowPlace, rows_of};
use super::{DragKind, DraggedTodo, MainWindow, RenderCtx, RowData, logic};
use crate::theme;

impl MainWindow {
    pub(super) fn render_quadrant(
        &mut self,
        rc: &RenderCtx,
        fill: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let lists: [Vec<RowData>; 4] = match self.drag.as_ref().map(|d| &d.kind) {
            Some(DragKind::Quadrant { lists, .. }) => {
                let lists = lists.clone();
                std::array::from_fn(|i| self.rows_for(&lists[i], cx))
            }
            _ => {
                let groups = self.state.read(cx).todos_by_quadrant();
                std::array::from_fn(|i| rows_of(&groups[i]))
            }
        };
        let mut cells = Vec::with_capacity(4);
        for (index, rows) in lists.iter().enumerate() {
            cells.push(self.render_quadrant_cell(index, rows, rc, fill, window, cx));
        }
        div()
            .id("quadrant-view")
            .w_full()
            .p(px(8.))
            .when(fill, |this| this.h_full())
            .child(
                div()
                    .grid()
                    .grid_cols(2)
                    .gap(px(8.))
                    .map(|this| {
                        if fill {
                            this.grid_rows(2).h_full()
                        } else {
                            this.grid_rows_min_content(2)
                        }
                    })
                    .children(cells),
            )
    }

    fn render_quadrant_cell(
        &mut self,
        index: usize,
        rows: &[RowData],
        rc: &RenderCtx,
        fill: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        let info = logic::QUADRANTS[index];
        let palette = rc.palette;
        let colors = rc.colors;
        let (r, g, b) = info.rgb;
        let quadrant_color = theme::rgba_css(r, g, b, 1.0);
        let (bg, hover_border) = if palette.is_dark {
            (colors.quadrant_dark_bg, colors.quadrant_border_hover_dark)
        } else {
            (theme::rgba_css(r, g, b, 0.1), quadrant_color)
        };

        let count = rows.len();
        let mut items = Vec::with_capacity(count);
        for (i, row) in rows.iter().enumerate() {
            let place = RowPlace::Quadrant {
                index,
                last: i + 1 == count,
            };
            items.push(self.render_row(row, place, rc, window, cx));
        }

        let bounds_cell = self.quadrant_bounds.clone();
        let measure = canvas(
            move |bounds, _, _| bounds_cell.borrow_mut()[index] = Some(bounds),
            |_, _, _, _| {},
        )
        .absolute()
        .inset_0();

        let header = div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(6.))
            .px(px(10.))
            .py(px(8.))
            .border_b_1()
            .border_color(colors.quadrant_header_border)
            .bg(colors.quadrant_header_bg)
            .child(
                div()
                    .size(px(8.))
                    .flex_none()
                    .rounded_full()
                    .bg(quadrant_color),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_size(px(12.))
                    .line_height(relative(theme::LINE_HEIGHT))
                    .font_weight(theme::WEIGHT_MEDIUM)
                    .text_color(colors.quadrant_title)
                    .child(info.name),
            )
            .child(
                div()
                    .flex_none()
                    .px(px(6.))
                    .py(px(1.))
                    .rounded(px(10.))
                    .bg(colors.quadrant_count_bg)
                    .text_size(px(11.))
                    .line_height(relative(theme::LINE_HEIGHT))
                    .text_color(colors.quadrant_count_fg)
                    .child(count.to_string()),
            );

        let list = div()
            .p(px(4.))
            .children(items)
            // 空象限："暂无待办"被挤出可视区，只剩它的 min-height 60px 撑高格子
            .when(count == 0 && !fill, |this| this.child(div().h(px(60.))));

        div()
            .id(("quadrant-cell", index))
            .relative()
            .flex()
            .flex_col()
            .min_w_0()
            .min_h_0()
            .rounded(theme::radius::MD)
            .overflow_hidden()
            .border_1()
            .border_color(colors.quadrant_border)
            .bg(bg)
            .hover(move |s| s.border_color(hover_border))
            .on_drag_move::<DraggedTodo>(cx.listener(
                move |this, event: &DragMoveEvent<DraggedTodo>, window, cx| {
                    if event.bounds.contains(&event.event.position) {
                        this.drag_move_quadrant(index, event, window, cx);
                    }
                },
            ))
            .child(measure)
            .child(header)
            .child(
                div()
                    .id(("quadrant-content", index))
                    .flex_1()
                    .min_h_0()
                    .overflow_x_hidden()
                    .when(fill, |this| this.overflow_y_scroll())
                    .child(list),
            )
            .into_any_element()
    }

    /// 拖动中经过某个象限：把占位移进该象限、按中点决定位置（跨象限即 vuedraggable 的 group put）
    fn drag_move_quadrant(
        &mut self,
        target: usize,
        event: &DragMoveEvent<DraggedTodo>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let pointer = event.event.position;
        self.record_drag_pointer(pointer, window, cx);
        let Some(session) = self.drag.as_ref() else {
            return;
        };
        let DragKind::Quadrant { lists, current, .. } = &session.kind else {
            return;
        };
        let id = session.id;
        let mids = self.mids_without(&lists[target], id);
        let index = logic::insertion_index(&mids, f32::from(pointer.y));
        let mut next = lists.clone();
        next[*current].retain(|x| *x != id);
        logic::move_to(&mut next[target], id, index);
        let changed = target != *current || next[target] != lists[target];
        if changed
            && let Some(super::DragSession {
                kind: DragKind::Quadrant { lists, current, .. },
                ..
            }) = self.drag.as_mut()
        {
            *lists = next;
            *current = target;
            cx.notify();
        }
    }
}
