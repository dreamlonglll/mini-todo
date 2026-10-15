//! `el-dropdown-menu`（浅色下拉菜单，`trigger="click"`）的菜单面板。开合状态由宿主管理，宿主用
//! [`super::ep_popper::popper`] 定位（EP 下拉菜单默认 `bottom`、带箭头；`placement="bottom-end"` 时右对齐触发元素），
//! 点外面关闭由 popper 的 `on_dismiss` 处理，Esc 由宿主处理。
//!
//! 面板：浅色弹层外框（白底、`#E4E7ED` 边框、浅阴影、圆角 4）；菜单 `padding 5px 0`、`max-height` 可选（超出滚动）；
//! 菜单项 `padding 5px 16px`、行高 22、14px、`#606266`、图标在左（右边距 5px）；悬停底 `#ECF5FF`、文字 `#409EFF`；
//! `divided` 项上方是一条独立的分隔线（EP 渲染为单独的 `<li role="separator">`：1px `#EBEEF5`、上下各 6px 外边距），
//! 菜单项本身不带外边距。点击项后由宿主关闭菜单。
//!
//! ```ignore
//! let menu = EpDropdownMenu::new("fence-more")
//!     .item(EpDropdownItem::new("rename", "重命名").icon(EpIcon::EditPen))
//!     .item(EpDropdownItem::new("delete", "删除收纳箱").icon(EpIcon::Delete).divided())
//!     .on_select(cx.listener(|this, command: &SharedString, window, cx| ..));
//! ep_popper::popper(&self.anchor, window, PopperOptions { placement: Placement::BottomEnd, .. }, menu, dismiss)
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

/// 菜单项高度（5 + 22 + 5；`divided` 项另加 12 + 1）与菜单上下内边距（弹层定位时估算高度用）
pub const ITEM_HEIGHT: f32 = 32.;
pub const PADDING: f32 = 5.;

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

/// 下拉菜单面板（含弹层外框）
#[derive(IntoElement)]
pub struct EpDropdownMenu {
    id: ElementId,
    items: Vec<EpDropdownItem>,
    max_height: Option<Pixels>,
    on_select: Option<SelectHandler>,
}

impl EpDropdownMenu {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self {
            id: id.into(),
            items: Vec::new(),
            max_height: None,
            on_select: None,
        }
    }

    pub fn item(mut self, item: EpDropdownItem) -> Self {
        self.items.push(item);
        self
    }

    /// `max-height`（超出滚动）
    pub fn max_height(mut self, height: Pixels) -> Self {
        self.max_height = Some(height);
        self
    }

    /// 点击菜单项：回调该项的 `command`
    pub fn on_select(mut self, f: impl Fn(&SharedString, &mut Window, &mut App) + 'static) -> Self {
        self.on_select = Some(Rc::new(f));
        self
    }

    /// 面板外框高度估算（含上下内边距与 1px 边框）
    pub fn estimated_height(item_count: usize, divided_count: usize) -> f32 {
        PADDING * 2. + ITEM_HEIGHT * item_count as f32 + 13. * divided_count as f32 + 2.
    }
}

impl RenderOnce for EpDropdownMenu {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let ep = ep();
        let items = self
            .items
            .into_iter()
            .enumerate()
            .flat_map(|(index, item)| {
                let on_select = self.on_select.clone();
                let command = item.command.clone();
                let group = SharedString::from(format!("ep-dropdown-item-{index}"));
                let separator = item.divided.then(|| {
                    div()
                        .my(px(6.))
                        .border_t_1()
                        .border_color(ep.border_lighter)
                        .into_any_element()
                });
                let row = div()
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
                    .into_any_element();
                separator.into_iter().chain(std::iter::once(row))
            });
        ep_popper::frame().id(self.id).font(theme::ui_font()).child(
            div()
                .id("menu")
                .py(px(PADDING))
                .when_some(self.max_height, |this, h| this.max_h(h).overflow_y_scroll())
                .children(items),
        )
    }
}
