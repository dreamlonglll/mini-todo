//! 列表视图（旧 `TodoList.vue`，规格 §5、§7）：未完成待办按 `sortOrder`；拖拽中按会话顺序渲染（不按 store 重建），
//! 松手后整体落库；没有任何待办（含已完成）时显示空状态。双击行以外的空白（含空状态）新建待办由外层的内容区处理
//! （`ListWindow::on_blank_mouse_down`）。

use gpui_kit::{
    Context, DragMoveEvent, InteractiveElement as _, IntoElement, ParentElement as _, Styled as _,
    Window, div, prelude::FluentBuilder as _, px,
};

use super::todo_item::{RowPlace, rows_of};
use super::{DragKind, DraggedTodo, ListWindow, RenderCtx, logic};
use crate::assets::EpIcon;
use crate::theme::Palette;
use crate::ui::icon::icon;

impl ListWindow {
    pub(super) fn render_list(
        &mut self,
        rc: &RenderCtx,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let list_id = self.list_id();
        let state = self.state.read(cx);
        let total = state.todo_count_in(list_id).total;
        let rows = match self.drag.as_ref().map(|d| &d.kind) {
            Some(DragKind::List { order, .. }) => {
                let order = order.clone();
                self.rows_for(&order, cx)
            }
            _ => rows_of(&state.pending_todos_in(list_id)),
        };
        let empty = rows.is_empty() && total == 0;
        let mut children = Vec::with_capacity(rows.len());
        for row in &rows {
            children.push(self.render_row(row, RowPlace::List, rc, window, cx));
        }
        // 锁定时没有 FAB：空状态只提示双击 / 快捷键
        let fab_visible = self.fab_visible(cx);

        div()
            .id("todo-list")
            .on_drag_move::<DraggedTodo>(cx.listener(
                |this, event: &DragMoveEvent<DraggedTodo>, window, cx| {
                    this.drag_move_list(event, window, cx)
                },
            ))
            .children(children)
            .when(empty, |this| {
                this.child(render_empty(&rc.palette, rc, fab_visible))
            })
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

/// 空状态：`Document` 48px + `暂无待办事项` + 提示（深色下底 `rgba(0,0,0,0.1)`）。提示见 [`empty_hints`]
fn render_empty(palette: &Palette, rc: &RenderCtx, fab_visible: bool) -> impl IntoElement {
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
        .children(
            empty_hints(fab_visible)
                .into_iter()
                .map(|hint| div().mt(px(4.)).text_size(px(12.)).child(hint)),
        )
}

/// 空状态的提示行：没锁定时先提示 FAB（旧版文案），再提示双击空白 / 快捷键；锁定时没有 FAB，只提示后者
fn empty_hints(fab_visible: bool) -> Vec<String> {
    if fab_visible {
        vec![
            "点击右下角悬浮按钮添加待办项".to_string(),
            format!("也可以双击空白处或按 {}", super::NEW_TODO_SHORTCUT),
        ]
    } else {
        vec![format!(
            "双击空白处或按 {} 添加待办项",
            super::NEW_TODO_SHORTCUT
        )]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_hints_follow_the_fab() {
        let shown = empty_hints(true);
        assert_eq!(shown[0], "点击右下角悬浮按钮添加待办项");
        assert!(
            shown[1].contains("双击空白处") && shown[1].contains(super::super::NEW_TODO_SHORTCUT)
        );
        // 锁定：没有 FAB，不再提"右下角悬浮按钮"
        let locked = empty_hints(false);
        assert_eq!(locked.len(), 1);
        assert!(!locked[0].contains("悬浮按钮"));
        assert!(locked[0].contains("双击空白处"));
    }
}
