//! `el-dropdown-menu`（下拉菜单，`trigger="click"`）的菜单面板。开合状态由宿主管理，宿主用
//! [`super::ep_popper::popper`] 定位（EP 下拉菜单默认 `bottom`、带箭头；`placement="bottom-end"` 时右对齐触发元素），
//! 点外面关闭由 popper 的 `on_dismiss` 处理，Esc 由宿主处理。
//!
//! 面板：弹层外框（[`super::ep_popper::frame_for`]：白底、`#E4E7ED` 边框、浅阴影、圆角 4）；菜单 `padding 5px 0`、
//! `max-height` 可选（超出滚动）；菜单项 `padding 5px 16px`、行高 22、14px、`#606266`、图标在左（右边距 5px）；
//! 悬停底 `#ECF5FF`、文字 `#409EFF`；`divided` 项上方是一条独立的分隔线（EP 渲染为单独的 `<li role="separator">`：
//! 1px `#EBEEF5`、上下各 6px 外边距），菜单项本身不带外边距。点击项后由宿主决定是否关闭菜单。
//!
//! 在 EP 的基础上加的能力（清单 / 日历窗口用）：
//! - [`EpDropdownMenu::dark`]：EP 暗黑模式配色（`theme::ep_menu(true)`），跟随所在窗口的日 / 夜；箭头同样要给
//!   `PopperOptions::dark`
//! - 菜单项右侧：[`EpDropdownItem::submenu`] 级联箭头（子菜单由宿主用第二个 popper 画在旁边，`Placement::RightStart`、
//!   锚点 = [`EpDropdownMenu::anchor`] 量到的本菜单外框；[`EpDropdownMenu::on_hover`] 悬停展开）、
//!   [`EpDropdownItem::note`] 说明文字（置灰项说明原因，同时作为 tooltip）、[`EpDropdownItem::checked`] 勾选（多选筛选）
//! - [`EpDropdownMenu::width`]：固定宽度，过长的文字截断加省略号；宽度可按内容估计（[`EpDropdownMenu::panel_width`]）
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
use crate::theme::{self, ep_menu};
use crate::ui::ep_popper::{self, Anchor};
use crate::ui::icon::icon;

/// 菜单项高度（5 + 22 + 5；`divided` 项另加 12 + 1）与菜单上下内边距（弹层定位时估算高度用）
pub const ITEM_HEIGHT: f32 = 32.;
pub const PADDING: f32 = 5.;
/// 菜单项文字字号与左右内边距
const FONT_SIZE: f32 = 14.;
const PADDING_X: f32 = 16.;
/// 左侧图标（14）+ 右边距 5
const ICON_WIDTH: f32 = 14. + 5.;
/// 右侧附加内容（箭头 / 勾 / 说明）与文字的间距、箭头与勾的大小、说明的字号
const SUFFIX_GAP: f32 = 12.;
const SUFFIX_ICON: f32 = 12.;
const NOTE_SIZE: f32 = 12.;

/// 文字宽度的估计（逻辑像素，定弹层宽度用）：ASCII 按 0.62 个字号（Segoe UI / 苹方的拉丁字母平均宽度略小于它，
/// 宁宽勿窄），其它字符（中文等）按一个字号
pub(crate) fn text_width(text: &str, font_size: f32) -> f32 {
    text.chars()
        .map(|c| {
            if c.is_ascii() {
                font_size * 0.62
            } else {
                font_size
            }
        })
        .sum()
}

/// 菜单项
#[derive(Debug, Clone)]
pub struct EpDropdownItem {
    pub command: SharedString,
    pub label: SharedString,
    pub icon: Option<EpIcon>,
    pub divided: bool,
    /// `disabled`：文字与图标 `--el-text-color-disabled`、`cursor: not-allowed`、没有悬停底色，点击不回调
    pub disabled: bool,
    /// 右侧画级联箭头（有子菜单；置灰时不画）
    pub submenu: bool,
    /// 右侧的说明文字（`--el-text-color-secondary`、12px；置灰项说明原因，同时作为悬停提示）
    pub note: Option<SharedString>,
    /// 勾选：文字 `--el-color-primary`、右侧画勾（同 `el-select multiple` 的已选项）
    pub checked: bool,
}

impl EpDropdownItem {
    pub fn new(command: impl Into<SharedString>, label: impl Into<SharedString>) -> Self {
        Self {
            command: command.into(),
            label: label.into(),
            icon: None,
            divided: false,
            disabled: false,
            submenu: false,
            note: None,
            checked: false,
        }
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn icon(mut self, icon: EpIcon) -> Self {
        self.icon = Some(icon);
        self
    }

    pub fn divided(mut self) -> Self {
        self.divided = true;
        self
    }

    pub fn submenu(mut self) -> Self {
        self.submenu = true;
        self
    }

    pub fn note(mut self, note: impl Into<SharedString>) -> Self {
        self.note = Some(note.into());
        self
    }

    pub fn checked(mut self, checked: bool) -> Self {
        self.checked = checked;
        self
    }

    /// 这一项要的宽度（不含面板边框）
    fn width(&self) -> f32 {
        let icon = if self.icon.is_some() { ICON_WIDTH } else { 0. };
        let suffix = match &self.note {
            Some(note) => SUFFIX_GAP + text_width(note, NOTE_SIZE),
            None if self.has_suffix_icon() => SUFFIX_GAP + SUFFIX_ICON,
            None => 0.,
        };
        PADDING_X * 2. + icon + text_width(&self.label, FONT_SIZE) + suffix
    }

    fn has_suffix_icon(&self) -> bool {
        self.checked || (self.submenu && !self.disabled)
    }
}

type SelectHandler = Rc<dyn Fn(&SharedString, &mut Window, &mut App)>;
type HoverHandler = Rc<dyn Fn(&SharedString, bool, &mut Window, &mut App)>;

/// 下拉菜单面板（含弹层外框）
#[derive(IntoElement)]
pub struct EpDropdownMenu {
    id: ElementId,
    items: Vec<EpDropdownItem>,
    max_height: Option<Pixels>,
    width: Option<f32>,
    dark: bool,
    anchor: Option<Anchor>,
    on_select: Option<SelectHandler>,
    on_hover: Option<HoverHandler>,
}

impl EpDropdownMenu {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self {
            id: id.into(),
            items: Vec::new(),
            max_height: None,
            width: None,
            dark: false,
            anchor: None,
            on_select: None,
            on_hover: None,
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

    /// 面板宽度（含边框）：过长的文字截断加省略号
    pub fn width(mut self, width: f32) -> Self {
        self.width = Some(width);
        self
    }

    /// EP 暗黑模式配色（所在窗口是深色主题时）
    pub fn dark(mut self, dark: bool) -> Self {
        self.dark = dark;
        self
    }

    /// 量取本菜单外框的窗口坐标（级联子菜单贴在它旁边；点子菜单时用它判断"点在菜单里"）
    pub fn anchor(mut self, anchor: Anchor) -> Self {
        self.anchor = Some(anchor);
        self
    }

    /// 点击菜单项：回调该项的 `command`
    pub fn on_select(mut self, f: impl Fn(&SharedString, &mut Window, &mut App) + 'static) -> Self {
        self.on_select = Some(Rc::new(f));
        self
    }

    /// 悬停进出菜单项（含置灰项）：回调该项的 `command` 与是否悬停（悬停展开级联子菜单）
    pub fn on_hover(
        mut self,
        f: impl Fn(&SharedString, bool, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_hover = Some(Rc::new(f));
        self
    }

    /// 面板外框高度估算（含上下内边距与 1px 边框）
    pub fn estimated_height(item_count: usize, divided_count: usize) -> f32 {
        PADDING * 2. + ITEM_HEIGHT * item_count as f32 + 13. * divided_count as f32 + 2.
    }

    /// 按当前菜单项估算的面板外框高度（有 `max-height` 时不超过它）
    pub fn panel_height(&self) -> f32 {
        let divided = self.items.iter().filter(|i| i.divided).count();
        let height = Self::estimated_height(self.items.len(), divided);
        match self.max_height {
            Some(max) => height.min(f32::from(max) + PADDING * 2. + 2.),
            None => height,
        }
    }

    /// 按最长的菜单项估算的面板宽度（含 1px 边框；文字宽度是估计值，见 [`text_width`]）
    pub fn panel_width(&self) -> f32 {
        self.items
            .iter()
            .map(EpDropdownItem::width)
            .fold(0., f32::max)
            + 2.
    }
}

impl RenderOnce for EpDropdownMenu {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let colors = ep_menu(self.dark);
        let truncate = self.width.is_some();
        let items = self
            .items
            .into_iter()
            .enumerate()
            .flat_map(|(index, item)| {
                let on_select = self.on_select.clone();
                let on_hover = self.on_hover.clone();
                let command = item.command.clone();
                let hover_command = item.command.clone();
                let group = SharedString::from(format!("ep-dropdown-item-{index}"));
                let separator = item.divided.then(|| {
                    div()
                        .my(px(6.))
                        .border_t_1()
                        .border_color(colors.divider)
                        .into_any_element()
                });
                let disabled = item.disabled;
                let color = if disabled {
                    colors.text_disabled
                } else if item.checked {
                    colors.primary
                } else {
                    colors.text
                };
                // 跟着悬停变色的图标（置灰项不变）
                let glyph = |source: EpIcon, size: f32| {
                    let glyph = icon(source, size, color);
                    if disabled {
                        glyph.into_any_element()
                    } else {
                        glyph
                            .group_hover(group.clone(), move |s| s.text_color(colors.primary))
                            .into_any_element()
                    }
                };
                let suffix = match &item.note {
                    Some(note) => Some(
                        div()
                            .flex_none()
                            .ml(px(SUFFIX_GAP))
                            .text_size(px(NOTE_SIZE))
                            .text_color(colors.text_secondary)
                            .child(note.clone())
                            .into_any_element(),
                    ),
                    None if item.checked => Some(
                        div()
                            .flex_none()
                            .ml(px(SUFFIX_GAP))
                            .flex()
                            .items_center()
                            .child(icon(EpIcon::Check, SUFFIX_ICON, colors.primary))
                            .into_any_element(),
                    ),
                    None if item.submenu && !disabled => Some(
                        div()
                            .flex_none()
                            .ml(px(SUFFIX_GAP))
                            .flex()
                            .items_center()
                            .child(glyph(EpIcon::ArrowRight, SUFFIX_ICON))
                            .into_any_element(),
                    ),
                    None => None,
                };
                let row = div()
                    .id(("item", index))
                    .group(group.clone())
                    .flex()
                    .items_center()
                    .px(px(PADDING_X))
                    .py(px(5.))
                    .text_size(px(FONT_SIZE))
                    .line_height(px(22.))
                    .whitespace_nowrap()
                    .text_color(color)
                    .when(disabled, |this| {
                        this.cursor(gpui_kit::CursorStyle::OperationNotAllowed)
                    })
                    .when(!disabled, |this| {
                        this.cursor_pointer()
                            .hover(move |s| s.bg(colors.hover_bg).text_color(colors.primary))
                    })
                    .when_some(item.icon, |this, source| {
                        this.child(
                            div()
                                .flex_none()
                                .mr(px(5.))
                                .flex()
                                .items_center()
                                .child(glyph(source, 14.)),
                        )
                    })
                    .map(|this| {
                        if truncate {
                            this.child(div().flex_1().min_w_0().truncate().child(item.label))
                        } else {
                            this.child(item.label)
                        }
                    })
                    .children(suffix)
                    .when_some(item.note.filter(|_| disabled), |this, note| {
                        this.tooltip(crate::ui::tooltip::native(note))
                    })
                    .when_some(on_hover, |this, handler| {
                        this.on_hover(move |hovered: &bool, window, cx| {
                            handler(&hover_command, *hovered, window, cx)
                        })
                    })
                    .when_some(on_select.filter(|_| !disabled), |this, handler| {
                        this.on_click(move |_, window, cx| handler(&command, window, cx))
                    })
                    .into_any_element();
                separator.into_iter().chain(std::iter::once(row))
            });
        ep_popper::frame_for(self.dark)
            .id(self.id)
            .font(theme::ui_font())
            .when_some(self.width, |this, width| this.w(px(width)))
            .child(
                div()
                    .id("menu")
                    .py(px(PADDING))
                    .when_some(self.max_height, |this, h| this.max_h(h).overflow_y_scroll())
                    .children(items),
            )
            .when_some(self.anchor, |this, anchor| this.child(anchor.measure()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_width_counts_wide_characters_as_one_font_size() {
        assert_eq!(text_width("清单", 14.), 28.);
        assert!((text_width("ab", 10.) - 12.4).abs() < 1e-4);
        assert_eq!(text_width("", 14.), 0.);
    }

    #[test]
    fn panel_size_follows_the_longest_item_and_its_suffix() {
        // 「移动到」带图标与级联箭头：16 + 19 + 42 + 12 + 12 + 16，再加 2px 边框
        let menu = EpDropdownMenu::new("m").item(
            EpDropdownItem::new("move", "移动到")
                .icon(EpIcon::Rank)
                .submenu(),
        );
        assert_eq!(menu.panel_width(), 119.);
        assert_eq!(menu.panel_height(), EpDropdownMenu::estimated_height(1, 0));
        // 置灰时不画箭头，换成说明文字（12px）
        let menu = EpDropdownMenu::new("m").item(
            EpDropdownItem::new("move", "移动到")
                .icon(EpIcon::Rank)
                .submenu()
                .disabled(true)
                .note("没有其它清单"),
        );
        assert_eq!(menu.panel_width(), 16. + 19. + 42. + 12. + 72. + 16. + 2.);
        // 勾选项右侧留出勾的位置；最高不超过 max-height + 内边距 + 边框
        let menu = EpDropdownMenu::new("m")
            .item(EpDropdownItem::new("all", "全部清单").checked(true))
            .item(EpDropdownItem::new("a", "工作").divided())
            .max_height(px(20.));
        assert_eq!(menu.panel_width(), 16. + 56. + 12. + 12. + 16. + 2.);
        assert_eq!(menu.panel_height(), 20. + 10. + 2.);
    }
}
