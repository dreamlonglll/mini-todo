//! 列表视图（旧 `TodoList.vue`，规格 §5、§7）：未完成待办按 `sortOrder`；拖拽中按会话顺序渲染（不按 store 重建），
//! 松手后整体落库；没有任何待办（含已完成）时显示空状态。

use gpui_kit::{
    Context, DragMoveEvent, InteractiveElement as _, IntoElement, ParentElement as _, Styled as _,
    Window, div, prelude::FluentBuilder as _, px,
};

use super::todo_item::{RowPlace, rows_of};
use super::{DragKind, DraggedTodo, MainWindow, RenderCtx, logic};
use crate::assets::EpIcon;
use crate::theme::Palette;
use crate::ui::icon::icon;

impl MainWindow {
    pub(super) fn render_list(
        &mut self,
        rc: &RenderCtx,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let state = self.state.read(cx);
        let total = state.todo_count().total;
        let rows = match self.drag.as_ref().map(|d| &d.kind) {
            Some(DragKind::List { order, .. }) => {
                let order = order.clone();
                self.rows_for(&order, cx)
            }
            _ => rows_of(&state.pending_todos()),
        };
        let empty = rows.is_empty() && total == 0;
        let mut children = Vec::with_capacity(rows.len());
        for row in &rows {
            children.push(self.render_row(row, RowPlace::List, rc, window, cx));
        }

        div()
            .id("todo-list")
            .on_drag_move::<DraggedTodo>(cx.listener(
                |this, event: &DragMoveEvent<DraggedTodo>, window, cx| {
                    this.drag_move_list(event, window, cx)
                },
            ))
            .children(children)
            .when(empty, |this| this.child(render_empty(&rc.palette, rc)))
    }

    /// 拖动中：按指针与其它行中点的关系实时移动占位
    fn drag_move_list(
        &mut self,
        event: &DragMoveEvent<DraggedTodo>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let pointer = event.event.position;
        self.record_drag_pointer(pointer, window, cx);
        let Some(session) = self.drag.as_ref() else {
            return;
        };
        let DragKind::List { order, .. } = &session.kind else {
            return;
        };
        let id = session.id;
        let mids = self.mids_without(order, id);
        let index = logic::insertion_index(&mids, f32::from(pointer.y));
        let current = order.iter().filter(|x| **x != id).count().min(index);
        let mut next = order.clone();
        logic::move_to(&mut next, id, current);
        if &next != order
            && let Some(super::DragSession {
                kind: DragKind::List { order, .. },
                ..
            }) = self.drag.as_mut()
        {
            *order = next;
            cx.notify();
        }
    }
}

/// 空状态：`Document` 48px + `暂无待办事项` + `点击右下角悬浮按钮添加待办项`（深色下底 `rgba(0,0,0,0.1)`）
fn render_empty(palette: &Palette, rc: &RenderCtx) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .p(px(24.))
        .text_color(palette.text_tertiary)
        .bg(rc.colors.empty_bg)
        .child(icon(EpIcon::Document, 48., palette.text_tertiary))
        .child(div().mt(px(8.)).text_size(px(14.)).child("暂无待办事项"))
        .child(
            div()
                .mt(px(4.))
                .text_size(px(12.))
                .child("点击右下角悬浮按钮添加待办项"),
        )
}
