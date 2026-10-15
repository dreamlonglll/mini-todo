//! `el-input` 外观的单行输入：包装 gpui-component 的 `InputState` + `Input`（关掉组件自带外观，边框自己画）。
//!
//! 外观：高 32（small 24）、`padding 1px 11px`（small 7px）、白底、1px 边 `#DCDFE6`、圆角 6（main.scss 覆盖）；
//! hover 边 `#C0C4CC`；聚焦边 `#409EFF`；文字 14px `#606266`（small 12px）；placeholder `#A8ABB2`；
//! 禁用：底 `#F5F7FA`、边 `#E4E7ED`、字 `#A8ABB2`、`cursor: not-allowed`。
//! 文字 / placeholder 颜色来自 gpui-component 主题（`theme::init` 已调成 EP 值）。
//!
//! ```ignore
//! // 构造视图时创建状态（不要在 render 里 new）
//! let title = cx.new(|cx| InputState::new(window, cx).placeholder("请输入待办标题"));
//! cx.subscribe_in(&title, window, |this, state, event: &InputEvent, window, cx| { .. }).detach();
//! // render
//! EpInput::new(&self.title).w_full()
//! EpInput::new(&self.search).small().prefix(icon(EpIcon::Search, 14., ep().text_placeholder))
//! // 读写
//! let value = self.title.read(cx).value();
//! self.title.update(cx, |s, cx| s.set_value("新值", window, cx));
//! ```

use gpui_kit::component::StyledExt as _;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::{
    AnyElement, App, CursorStyle, Entity, Focusable as _, InteractiveElement as _, IntoElement,
    ParentElement as _, RenderOnce, StyleRefinement, Styled, Window, div,
    prelude::FluentBuilder as _, px,
};

use crate::theme::{self, ep};

pub use gpui_kit::component::input::InputEvent;

/// 尺寸
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InputSize {
    #[default]
    Default,
    Small,
}

/// Element Plus 外观的单行输入
#[derive(IntoElement)]
pub struct EpInput {
    state: Entity<InputState>,
    size: InputSize,
    disabled: bool,
    cleanable: bool,
    prefix: Option<AnyElement>,
    suffix: Option<AnyElement>,
    style: StyleRefinement,
}

impl EpInput {
    pub fn new(state: &Entity<InputState>) -> Self {
        Self {
            state: state.clone(),
            size: InputSize::Default,
            disabled: false,
            cleanable: false,
            prefix: None,
            suffix: None,
            style: StyleRefinement::default(),
        }
    }

    pub fn small(mut self) -> Self {
        self.size = InputSize::Small;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// `clearable`：有内容时右侧显示清空按钮
    pub fn cleanable(mut self, cleanable: bool) -> Self {
        self.cleanable = cleanable;
        self
    }

    pub fn prefix(mut self, prefix: impl IntoElement) -> Self {
        self.prefix = Some(prefix.into_any_element());
        self
    }

    pub fn suffix(mut self, suffix: impl IntoElement) -> Self {
        self.suffix = Some(suffix.into_any_element());
        self
    }
}

impl Styled for EpInput {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl RenderOnce for EpInput {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let ep = ep();
        let focused = self
            .state
            .read(cx)
            .focus_handle(cx)
            .contains_focused(window, cx);
        let (height, pad_x, font) = match self.size {
            InputSize::Default => (32., 11., 14.),
            InputSize::Small => (24., 7., 12.),
        };
        let border = if self.disabled {
            ep.disabled_border
        } else if focused {
            ep.primary
        } else {
            ep.border
        };
        let input = Input::new(&self.state)
            .appearance(false)
            .disabled(self.disabled)
            .cleanable(self.cleanable)
            .flex_1()
            .px(px(0.))
            .py(px(0.))
            .text_size(px(font));
        let input = Styled::h(input, px(height - 2.));

        div()
            .flex()
            .items_center()
            .gap(px(4.))
            .h(px(height))
            .px(px(pad_x))
            .bg(if self.disabled {
                ep.disabled_bg
            } else {
                ep.white
            })
            .border_1()
            .border_color(border)
            .rounded(theme::radius::BASE)
            .text_size(px(font))
            .when(self.disabled, |this| {
                this.cursor(CursorStyle::OperationNotAllowed)
                    .text_color(ep.disabled_text)
            })
            .when(!self.disabled && !focused, |this| {
                this.hover(|s| s.border_color(ep.border_hover))
            })
            .children(self.prefix)
            .child(input)
            .children(self.suffix)
            .refine_style(&self.style)
    }
}
