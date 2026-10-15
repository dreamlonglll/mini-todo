//! `el-dropdown-menu`（浅色下拉菜单，`trigger="click"`）的菜单面板。开合状态由宿主管理，宿主用
//! `deferred(anchored()…)` 定位（EP `offset` 12，`placement="bottom-end"` 时右对齐触发元素），并在打开期间处理
//! 点外面 / Esc 关闭。
//!
//! 面板：浅色弹层（白底、`#E4E7ED` 边框、浅阴影、圆角 4）+ 顶部箭头；菜单 `padding 5px 0`、`max-height` 可选（超出滚动）；
//! 菜单项 `padding 5px 16px`、行高 22、14px、`#606266`、图标在左（右边距 5px）；悬停底 `#ECF5FF`、文字 `#409EFF`；
//! `divided` 项上方 1px `#EBEEF5` 分隔线（上下各 6px 外边距）。点击项后由宿主关闭菜单。
//!
//! ```ignore
//! EpDropdownMenu::new("fence-more")
//!     .item(EpDropdownItem::new("rename", "重命名").icon(EpIcon::EditPen))
//!     .item(EpDropdownItem::new("delete", "删除收纳箱").icon(EpIcon::Delete).divided())
//!     .arrow_at(px(116.))
//!     .on_select(cx.listener(|this, command: &SharedString, window, cx| ..))
//! ```

use std::rc::Rc;

use gpui_kit::{
    App, ElementId, InteractiveElement as _, IntoElement, ParentElement as _, Pixels, RenderOnce,
    SharedString, StatefulInteractiveElement as _, Styled as _, Window, div,
    prelude::FluentBuilder as _, px,
};

use crate::assets::EpIcon;
use crate::theme::{self, ep};
use crate::ui::ep_popper;
use crate::ui::icon::icon;

/// 菜单项
#[derive(Debug, Clone)]
pub struct EpDropdownItem {
    pub command: SharedString,
    pub label: SharedString,
    pub icon: Option<EpIcon>,
    pub divided: bool,
}

impl EpDropdownItem {
    pub fn new(command: impl Into<SharedString>, label: impl Into<SharedString>) -> Self {
        Self {
            command: command.into(),
            label: label.into(),
            icon: None,
            divided: false,
        }
    }

    pub fn icon(mut self, icon: EpIcon) -> Self {
        self.icon = Some(icon);
        self
    }

    pub fn divided(mut self) -> Self {
        self.divided = true;
        self
    }
}

type SelectHandler = Rc<dyn Fn(&SharedString, &mut Window, &mut App)>;

/// 下拉菜单面板
#[derive(IntoElement)]
pub struct EpDropdownMenu {
    id: ElementId,
    items: Vec<EpDropdownItem>,
    arrow_at: Option<Pixels>,
    max_height: Option<Pixels>,
    on_select: Option<SelectHandler>,
}

impl EpDropdownMenu {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self {
            id: id.into(),
            items: Vec::new(),
            arrow_at: None,
            max_height: None,
            on_select: None,
        }
    }

    pub fn item(mut self, item: EpDropdownItem) -> Self {
        self.items.push(item);
        self
    }

    /// 箭头中心到面板左边框外沿的距离（不设则不画箭头）
    pub fn arrow_at(mut self, x: Pixels) -> Self {
        self.arrow_at = Some(x);
        self
    }

    /// `max-height`（超出滚动）
    pub fn max_height(mut self, height: Pixels) -> Self {
        self.max_height = Some(height);
        self
    }

    pub fn on_select(mut self, f: impl Fn(&SharedString, &mut Window, &mut App) + 'static) -> Self {
        self.on_select = Some(Rc::new(f));
        self
    }
}

impl RenderOnce for EpDropdownMenu {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let ep = ep();
        let items = self.items.into_iter().enumerate().map(|(index, item)| {
            let on_select = self.on_select.clone();
            let command = item.command.clone();
            let group = SharedString::from(format!("ep-dropdown-item-{index}"));
            div()
                .id(("item", index))
                .group(group.clone())
                .flex()
                .items_center()
                .px(px(16.))
                .py(px(5.))
                .text_size(px(14.))
                .line_height(px(22.))
                .whitespace_nowrap()
                .text_color(ep.text_regular)
                .cursor_pointer()
                .when(item.divided, |this| {
                    this.my(px(6.)).border_t_1().border_color(ep.border_lighter)
                })
                .hover(move |s| s.bg(ep.primary_light_9).text_color(ep.primary))
                .when_some(item.icon, |this, source| {
                    this.child(
                        div().mr(px(5.)).flex().items_center().child(
                            icon(source, 14., ep.text_regular)
                                .group_hover(group.clone(), move |s| s.text_color(ep.primary)),
                        ),
                    )
                })
                .when_some(on_select, |this, handler| {
                    this.on_click(move |_, window, cx| handler(&command, window, cx))
                })
                .child(item.label)
        });
        ep_popper::panel()
            .id(self.id)
            .occlude()
            .font(theme::ui_font())
            .when_some(self.arrow_at, |this, x| this.child(ep_popper::arrow_up(x)))
            .child(
                div()
                    .id("menu")
                    .py(px(5.))
                    .when_some(self.max_height, |this, h| this.max_h(h).overflow_y_scroll())
                    .children(items),
            )
    }
}
