//! `el-dropdown-menu`（Element Plus 2.13）：浅色弹层、`padding 5px 0`；菜单项 `padding 5px 16px`、行高 22、
//! 14px `#606266`，悬停底 `#ECF5FF`、字 `#409EFF`。定位与外部点击关闭由 [`super::ep2_popper::popper`] 负责
//! （EP 下拉菜单默认 `bottom`、带箭头）。

use std::rc::Rc;

use gpui_kit::{
    App, ClickEvent, InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window, div, px,
};

use crate::theme::{self, ep};
use crate::ui::ep2_popper;

/// 菜单项点击回调
pub type MenuHandler = Rc<dyn Fn(&mut Window, &mut App)>;

/// 一个菜单项
pub struct MenuItem {
    pub label: SharedString,
    pub on_click: MenuHandler,
}

impl MenuItem {
    pub fn new(
        label: impl Into<SharedString>,
        on_click: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            label: label.into(),
            on_click: Rc::new(on_click),
        }
    }
}

/// 菜单项高度（5 + 22 + 5）与上下内边距
pub const ITEM_HEIGHT: f32 = 32.;
pub const PADDING: f32 = 5.;

/// 菜单面板（含外框）
pub fn menu(id: &str, items: Vec<MenuItem>) -> impl IntoElement {
    let ep = ep();
    ep2_popper::frame()
        .py(px(PADDING))
        .font(theme::ui_font())
        .children(items.into_iter().enumerate().map(|(ix, item)| {
            let handler = item.on_click.clone();
            div()
                .id(SharedString::from(format!("{id}-{ix}")))
                .px(px(16.))
                .py(px(5.))
                .text_size(px(14.))
                .line_height(px(22.))
                .whitespace_nowrap()
                .text_color(ep.text_regular)
                .cursor_pointer()
                .hover(move |s| s.bg(ep.primary_light_9).text_color(ep.primary))
                .on_click(move |_: &ClickEvent, window, cx| handler(window, cx))
                .child(item.label)
        }))
}
