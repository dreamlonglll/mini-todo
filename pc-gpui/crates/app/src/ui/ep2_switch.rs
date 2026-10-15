//! `el-switch` 与 `el-checkbox`（Element Plus 2.13，受控：组件只请求新值）。
//!
//! - 开关：整体高 32；核心 40×20、圆角 10、1px 边与底同色（关 `#DCDFE6` / 开 `#409EFF`）；白色滑块 16×16，
//!   关在左 1px、开在 `100% - 17px`（过渡 0.3s）。
//! - 复选框：高 32、右边距 30（组里最后一个 0）；方框 14×14、圆角 2、1px `#DCDFE6` 边（hover `#409EFF`）；
//!   选中底 / 边 `#409EFF` + 白勾；标签 14px `#606266`、左边距 8，选中时 `#409EFF`。

use std::rc::Rc;

use gpui_kit::{
    App, ClickEvent, ElementId, InteractiveElement as _, IntoElement, ParentElement as _,
    RenderOnce, SharedString, StatefulInteractiveElement as _, Styled as _, Window, div,
    prelude::FluentBuilder as _, px,
};

use crate::assets::EpIcon;
use crate::theme::ep;
use crate::ui::icon::icon;

type ChangeHandler = Rc<dyn Fn(bool, &mut Window, &mut App)>;

/// `el-switch`
#[derive(IntoElement)]
pub struct EpSwitch {
    id: ElementId,
    checked: bool,
    on_change: Option<ChangeHandler>,
}

impl EpSwitch {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self {
            id: id.into(),
            checked: false,
            on_change: None,
        }
    }

    pub fn checked(mut self, checked: bool) -> Self {
        self.checked = checked;
        self
    }

    pub fn on_change(mut self, f: impl Fn(bool, &mut Window, &mut App) + 'static) -> Self {
        self.on_change = Some(Rc::new(f));
        self
    }
}

impl RenderOnce for EpSwitch {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let ep = ep();
        let checked = self.checked;
        let color = if checked { ep.primary } else { ep.border };
        // 相对边框内侧（宽 38）：关 1px，开 `100% - 17px` = 21px
        let thumb_left = if checked { 21. } else { 1. };
        let on_change = self.on_change.clone();
        div()
            .id(self.id)
            .h(px(32.))
            .flex()
            .items_center()
            .cursor_pointer()
            .when_some(on_change, |this, handler| {
                this.on_click(move |_: &ClickEvent, window, cx| handler(!checked, window, cx))
            })
            .child(
                div()
                    .relative()
                    .w(px(40.))
                    .h(px(20.))
                    .rounded(px(10.))
                    .border_1()
                    .border_color(color)
                    .bg(color)
                    .child(
                        div()
                            .absolute()
                            .top(px(1.))
                            .size(px(16.))
                            .rounded_full()
                            .bg(ep.white)
                            .left(px(thumb_left)),
                    ),
            )
    }
}

/// `el-checkbox`
#[derive(IntoElement)]
pub struct EpCheckbox {
    id: ElementId,
    label: SharedString,
    checked: bool,
    last: bool,
    on_change: Option<ChangeHandler>,
}

impl EpCheckbox {
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            checked: false,
            last: false,
            on_change: None,
        }
    }

    pub fn checked(mut self, checked: bool) -> Self {
        self.checked = checked;
        self
    }

    /// 组里最后一个（没有右边距）
    pub fn last(mut self, last: bool) -> Self {
        self.last = last;
        self
    }

    pub fn on_change(mut self, f: impl Fn(bool, &mut Window, &mut App) + 'static) -> Self {
        self.on_change = Some(Rc::new(f));
        self
    }
}

impl RenderOnce for EpCheckbox {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let ep = ep();
        let checked = self.checked;
        let group: SharedString = format!("{}", self.id).into();
        let on_change = self.on_change.clone();
        div()
            .id(self.id)
            .group(group.clone())
            .h(px(32.))
            .flex()
            .items_center()
            .flex_none()
            .when(!self.last, |this| this.mr(px(30.)))
            .cursor_pointer()
            .when_some(on_change, |this, handler| {
                this.on_click(move |_: &ClickEvent, window, cx| handler(!checked, window, cx))
            })
            .child(
                div()
                    .size(px(14.))
                    .rounded(px(2.))
                    .border_1()
                    .border_color(if checked { ep.primary } else { ep.border })
                    .bg(if checked { ep.primary } else { ep.white })
                    .flex()
                    .items_center()
                    .justify_center()
                    .when(!checked, |this| {
                        this.group_hover(group.clone(), move |s| s.border_color(ep.primary))
                    })
                    .when(checked, |this| {
                        this.child(icon(EpIcon::Check, 10., ep.white))
                    }),
            )
            .child(
                div()
                    .pl(px(8.))
                    .text_size(px(14.))
                    .text_color(if checked { ep.primary } else { ep.text_regular })
                    .child(self.label),
            )
    }
}
