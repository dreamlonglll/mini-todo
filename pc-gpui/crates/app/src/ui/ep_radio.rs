//! `el-radio-group` + `el-radio-button`（按钮式单选组）。
//!
//! 按钮水平拼接：每格 1px `#DCDFE6` 边框（相邻格不重复左边框）、白底、文字 `#606266`；small 字号 12、padding
//! `5px 11px`（可用 [`EpRadioButtons::padding`] 覆盖，设置页「通知方式」是 `6px 12px`）、行高 1；首格左圆角 4、末格
//! 右圆角 4；悬停文字 `#409EFF`；选中：底 / 边框 `#409EFF`、白字，左侧再补 1px 主色（EP 的 `box-shadow: -1px 0 0 0`）；
//! 禁用：文字 `#A8ABB2`、不可点。受控组件：点击未选中的项回调 `on_change(value)`。
//!
//! ```ignore
//! EpRadioButtons::new("notify-type")
//!     .small()
//!     .padding(6., 12.)
//!     .option("system", "系统通知")
//!     .option("app", "软件通知")
//!     .selected(Some("system"))
//!     .on_change(cx.listener(|this, value: &SharedString, window, cx| ..))
//! ```

use std::rc::Rc;

use gpui_kit::{
    App, CursorStyle, ElementId, InteractiveElement as _, IntoElement, ParentElement as _,
    RenderOnce, SharedString, StatefulInteractiveElement as _, Styled as _, Window, div,
    prelude::FluentBuilder as _, px,
};

use crate::theme::{self, ep};

type ChangeHandler = Rc<dyn Fn(&SharedString, &mut Window, &mut App)>;

/// 按钮式单选组
#[derive(IntoElement)]
pub struct EpRadioButtons {
    id: ElementId,
    options: Vec<(SharedString, SharedString)>,
    selected: Option<SharedString>,
    small: bool,
    padding: Option<(f32, f32)>,
    disabled: bool,
    on_change: Option<ChangeHandler>,
}

impl EpRadioButtons {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self {
            id: id.into(),
            options: Vec::new(),
            selected: None,
            small: false,
            padding: None,
            disabled: false,
            on_change: None,
        }
    }

    /// 追加一项（值、文案）
    pub fn option(
        mut self,
        value: impl Into<SharedString>,
        label: impl Into<SharedString>,
    ) -> Self {
        self.options.push((value.into(), label.into()));
        self
    }

    pub fn selected(mut self, value: Option<impl Into<SharedString>>) -> Self {
        self.selected = value.map(Into::into);
        self
    }

    /// `size="small"`（字号 12）
    pub fn small(mut self) -> Self {
        self.small = true;
        self
    }

    /// 覆盖内边距（上下, 左右）
    pub fn padding(mut self, y: f32, x: f32) -> Self {
        self.padding = Some((y, x));
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn on_change(mut self, f: impl Fn(&SharedString, &mut Window, &mut App) + 'static) -> Self {
        self.on_change = Some(Rc::new(f));
        self
    }
}

impl RenderOnce for EpRadioButtons {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let ep = ep();
        let font = if self.small { 12. } else { 14. };
        let (pad_y, pad_x) = self
            .padding
            .unwrap_or(if self.small { (5., 11.) } else { (8., 15.) });
        let count = self.options.len();
        let disabled = self.disabled;
        let radius = theme::radius::SM;
        div()
            .id(self.id.clone())
            .flex()
            .flex_none()
            .items_center()
            .children(
                self.options
                    .into_iter()
                    .enumerate()
                    .map(|(index, (value, label))| {
                        let checked = self.selected.as_ref() == Some(&value);
                        let first = index == 0;
                        let last = index + 1 == count;
                        let text = if disabled {
                            ep.disabled_text
                        } else if checked {
                            ep.white
                        } else {
                            ep.text_regular
                        };
                        let on_change = self.on_change.clone();
                        div()
                            .id(("radio", index))
                            .relative()
                            .flex_none()
                            .px(px(pad_x))
                            .py(px(pad_y))
                            .text_size(px(font))
                            .line_height(px(font))
                            .whitespace_nowrap()
                            .border_t_1()
                            .border_b_1()
                            .border_r_1()
                            .when(first, |this| this.border_l_1().rounded_l(radius))
                            .when(last, |this| this.rounded_r(radius))
                            .border_color(if checked && !disabled {
                                ep.primary
                            } else {
                                ep.border
                            })
                            .bg(if checked && !disabled {
                                ep.primary
                            } else {
                                ep.white
                            })
                            .text_color(text)
                            .when(checked && !disabled && !first, |this| {
                                // EP：`box-shadow: -1px 0 0 0 var(--el-color-primary)` 盖住前一格的右边框
                                this.child(
                                    div()
                                        .absolute()
                                        .top(px(-1.))
                                        .bottom(px(-1.))
                                        .left(px(-1.))
                                        .w(px(1.))
                                        .bg(ep.primary),
                                )
                            })
                            .when(disabled, |this| {
                                this.cursor(CursorStyle::OperationNotAllowed)
                            })
                            .when(!disabled, |this| {
                                this.cursor_pointer().when(!checked, |this| {
                                    this.hover(move |s| s.text_color(ep.primary))
                                })
                            })
                            .when_some(
                                on_change.filter(|_| !disabled && !checked),
                                |this, handler| {
                                    let value = value.clone();
                                    this.on_click(move |_, window, cx| handler(&value, window, cx))
                                },
                            )
                            .child(label)
                    }),
            )
    }
}
