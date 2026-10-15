//! `el-button`（Element Plus 2.13 默认样式 + main.scss 覆盖：圆角统一 6px）。
//!
//! ```ignore
//! EpButton::new("save").primary().label("保存").on_click(|_, window, cx| ..)
//! EpButton::new("edit").text().primary().icon(EpIcon::Edit).icon_size(14.).label("编辑")
//! EpButton::new("close").text().icon(EpIcon::Close).icon_size(14.)          // 只有图标：44×32
//! EpButton::new("today").small().label("今天")
//! EpButton::new("complete").success().plain().label("完成任务")
//! ```
//!
//! | 变体 | 默认外观 | hover | active |
//! |---|---|---|---|
//! | default | 白底、边 `#DCDFE6`、字 `#606266` | 字 `#409EFF`、底 `#ECF5FF`、边 `#C6E2FF` | 边 `#409EFF` |
//! | primary 等（实心） | 底 / 边 = 类型色、白字 | light-3 | dark-2 |
//! | `plain` | 字 = 类型色、底 light-9、边 light-5 | 底 / 边 = 类型色、白字 | dark-2 |
//! | `text` | 透明无边、字 `#606266`（或类型色） | 底 `#F5F7FA` | 底 `#F0F2F5` |
//! | `link` | 透明无边无内边距、字 `#606266`（或类型色） | 字 `#909399`（或 light-3） | 字 dark-2 |
//!
//! 尺寸：default 高 32、padding `8px 15px`、14px；small 高 24、padding `5px 11px`、12px；`circle` 宽 = 高。
//! 字重 500 在 Segoe UI / 微软雅黑上实际是 Regular。图标与文字间距默认 12px（EP 的 `:icon` 属性 +
//! `<span>` 文本：`margin-left:6px` 叠加 main.scss 的 `gap:6px`），裸文字节点用 `.icon_gap(6.)`。
//!
//! 放进拖动区（标题栏）时按钮已 `occlude()`，不会触发拖动。

use std::rc::Rc;

use gpui_kit::component::StyledExt as _;
use gpui_kit::{
    App, ClickEvent, CursorStyle, ElementId, Hsla, InteractiveElement as _, IntoElement,
    ParentElement as _, RenderOnce, SharedString, StatefulInteractiveElement as _, StyleRefinement,
    Styled, Window, div, prelude::FluentBuilder as _, px,
};

use crate::assets::EpIcon;
use crate::theme::{self, ep, hex};
use crate::ui::icon::{IconSource, icon, spinning};

/// `type`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ButtonType {
    #[default]
    Default,
    Primary,
    Success,
    Warning,
    Danger,
    Info,
}

/// `size`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ButtonSize {
    #[default]
    Default,
    Small,
}

/// 一组颜色（背景、边框、文字）
#[derive(Debug, Clone, Copy, PartialEq)]
struct Colors {
    bg: Hsla,
    border: Hsla,
    text: Hsla,
}

/// 类型色：(基色, light-3, light-5, light-9, dark-2)
fn type_colors(kind: ButtonType) -> (Hsla, Hsla, Hsla, Hsla, Hsla) {
    match kind {
        ButtonType::Default | ButtonType::Primary => (
            ep().primary,
            ep().primary_light_3,
            ep().primary_light_5,
            ep().primary_light_9,
            ep().primary_dark_2,
        ),
        ButtonType::Success => (
            ep().success,
            hex(0x95D475),
            ep().success_light_5,
            ep().success_light_9,
            ep().success_dark_2,
        ),
        ButtonType::Warning => (
            ep().warning,
            hex(0xEEBE77),
            ep().warning_light_5,
            ep().warning_light_9,
            ep().warning_dark_2,
        ),
        ButtonType::Danger => (
            ep().danger,
            hex(0xF89898),
            hex(0xFAB6B6),
            ep().danger_light_9,
            hex(0xC45656),
        ),
        ButtonType::Info => (
            ep().info,
            hex(0xB1B3B8),
            hex(0xC8C9CC),
            ep().info_light_9,
            hex(0x73767A),
        ),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Variant {
    Solid,
    Plain,
    Text,
    Link,
}

/// (normal, hover, active, disabled)
fn variant_colors(kind: ButtonType, variant: Variant) -> (Colors, Colors, Colors, Colors) {
    let ep = ep();
    let clear = theme::hexa(0x00000000);
    let (base, light3, light5, light9, dark2) = type_colors(kind);
    let typed = kind != ButtonType::Default;
    match variant {
        Variant::Solid if !typed => (
            Colors {
                bg: ep.white,
                border: ep.border,
                text: ep.text_regular,
            },
            Colors {
                bg: ep.primary_light_9,
                border: ep.primary_light_7,
                text: ep.primary,
            },
            Colors {
                bg: ep.primary_light_9,
                border: ep.primary,
                text: ep.primary,
            },
            Colors {
                bg: ep.white,
                border: ep.border_light,
                text: ep.disabled_text,
            },
        ),
        Variant::Solid => (
            Colors {
                bg: base,
                border: base,
                text: ep.white,
            },
            Colors {
                bg: light3,
                border: light3,
                text: ep.white,
            },
            Colors {
                bg: dark2,
                border: dark2,
                text: ep.white,
            },
            Colors {
                bg: light5,
                border: light5,
                text: ep.white,
            },
        ),
        Variant::Plain if !typed => (
            Colors {
                bg: ep.white,
                border: ep.border,
                text: ep.text_regular,
            },
            Colors {
                bg: ep.white,
                border: ep.primary,
                text: ep.primary,
            },
            Colors {
                bg: ep.white,
                border: ep.primary_dark_2,
                text: ep.primary_dark_2,
            },
            Colors {
                bg: ep.white,
                border: ep.border_light,
                text: ep.disabled_text,
            },
        ),
        Variant::Plain => (
            Colors {
                bg: light9,
                border: light5,
                text: base,
            },
            Colors {
                bg: base,
                border: base,
                text: ep.white,
            },
            Colors {
                bg: dark2,
                border: dark2,
                text: ep.white,
            },
            Colors {
                bg: light9,
                border: light9,
                text: light5,
            },
        ),
        Variant::Text => {
            let text = if typed { base } else { ep.text_regular };
            (
                Colors {
                    bg: clear,
                    border: clear,
                    text,
                },
                Colors {
                    bg: ep.fill_light,
                    border: clear,
                    text,
                },
                Colors {
                    bg: ep.fill,
                    border: clear,
                    text,
                },
                Colors {
                    bg: clear,
                    border: clear,
                    text: ep.disabled_text,
                },
            )
        }
        Variant::Link => {
            let (text, hover, active) = if typed {
                (base, light3, dark2)
            } else {
                (ep.text_regular, ep.text_secondary, ep.text_primary)
            };
            (
                Colors {
                    bg: clear,
                    border: clear,
                    text,
                },
                Colors {
                    bg: clear,
                    border: clear,
                    text: hover,
                },
                Colors {
                    bg: clear,
                    border: clear,
                    text: active,
                },
                Colors {
                    bg: clear,
                    border: clear,
                    text: ep.disabled_text,
                },
            )
        }
    }
}

type ClickHandler = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

/// Element Plus 按钮
#[derive(IntoElement)]
pub struct EpButton {
    id: ElementId,
    label: Option<SharedString>,
    icon: Option<IconSource>,
    icon_size: Option<f32>,
    icon_gap: f32,
    kind: ButtonType,
    size: ButtonSize,
    variant: Variant,
    circle: bool,
    disabled: bool,
    loading: bool,
    tooltip: Option<SharedString>,
    on_click: Option<ClickHandler>,
    style: StyleRefinement,
}

impl EpButton {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self {
            id: id.into(),
            label: None,
            icon: None,
            icon_size: None,
            icon_gap: 12.0,
            kind: ButtonType::Default,
            size: ButtonSize::Default,
            variant: Variant::Solid,
            circle: false,
            disabled: false,
            loading: false,
            tooltip: None,
            on_click: None,
            style: StyleRefinement::default(),
        }
    }

    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub fn icon(mut self, icon: impl Into<IconSource>) -> Self {
        self.icon = Some(icon.into());
        self
    }

    /// 图标尺寸（默认 = 字号：14 / small 12）
    pub fn icon_size(mut self, size: f32) -> Self {
        self.icon_size = Some(size);
        self
    }

    /// 图标与文字间距（默认 12px）
    pub fn icon_gap(mut self, gap: f32) -> Self {
        self.icon_gap = gap;
        self
    }

    pub fn kind(mut self, kind: ButtonType) -> Self {
        self.kind = kind;
        self
    }

    pub fn primary(self) -> Self {
        self.kind(ButtonType::Primary)
    }

    pub fn success(self) -> Self {
        self.kind(ButtonType::Success)
    }

    pub fn warning(self) -> Self {
        self.kind(ButtonType::Warning)
    }

    pub fn danger(self) -> Self {
        self.kind(ButtonType::Danger)
    }

    pub fn info(self) -> Self {
        self.kind(ButtonType::Info)
    }

    /// `plain`
    pub fn plain(mut self) -> Self {
        self.variant = Variant::Plain;
        self
    }

    /// `text`（文字按钮）
    pub fn text(mut self) -> Self {
        self.variant = Variant::Text;
        self
    }

    /// `link`
    pub fn link(mut self) -> Self {
        self.variant = Variant::Link;
        self
    }

    /// `circle`（宽 = 高的圆形按钮）
    pub fn circle(mut self) -> Self {
        self.circle = true;
        self
    }

    pub fn small(mut self) -> Self {
        self.size = ButtonSize::Small;
        self
    }

    pub fn size(mut self, size: ButtonSize) -> Self {
        self.size = size;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// `:loading`：图标换成转圈的 Loading、不可点击
    pub fn loading(mut self, loading: bool) -> Self {
        self.loading = loading;
        self
    }

    /// 原生 title 风格的 tooltip
    pub fn tooltip(mut self, text: impl Into<SharedString>) -> Self {
        self.tooltip = Some(text.into());
        self
    }

    pub fn on_click(mut self, f: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(Rc::new(f));
        self
    }
}

impl Styled for EpButton {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl RenderOnce for EpButton {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let (normal, hover, active, disabled_colors) = variant_colors(self.kind, self.variant);
        let inactive = self.disabled || self.loading;
        let colors = if self.disabled {
            disabled_colors
        } else {
            normal
        };
        let (height, pad_x, pad_y, font) = match self.size {
            ButtonSize::Default => (32., 15., 8., 14.),
            ButtonSize::Small => (24., 11., 5., 12.),
        };
        let icon_size = self.icon_size.unwrap_or(font);
        let is_link = self.variant == Variant::Link;
        let has_label = self.label.is_some();
        let icon_color = colors.text;
        let id = self.id.clone();

        let icon_el = if self.loading {
            Some(
                spinning(
                    (id.clone(), "loading"),
                    icon(EpIcon::Loading, icon_size, icon_color),
                )
                .into_any_element(),
            )
        } else {
            self.icon
                .clone()
                .map(|source| icon(source, icon_size, icon_color).into_any_element())
        };

        div()
            .id(self.id)
            .occlude()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .gap(px(self.icon_gap))
            .text_size(px(font))
            .line_height(px(font))
            .whitespace_nowrap()
            .text_color(colors.text)
            .bg(colors.bg)
            .when(!is_link, |this| {
                this.h(px(height))
                    .border_1()
                    .border_color(colors.border)
                    .rounded(theme::radius::BASE)
            })
            .when(is_link, |this| this.p(px(2.)))
            .when(!is_link && self.circle, |this| {
                this.w(px(height)).p(px(pad_y)).rounded_full()
            })
            .when(!is_link && !self.circle, |this| {
                this.px(px(pad_x)).py(px(pad_y))
            })
            .when(inactive, |this| {
                this.cursor(CursorStyle::OperationNotAllowed)
            })
            .when(!inactive, |this| {
                this.cursor_pointer()
                    .hover(move |s| {
                        s.bg(hover.bg)
                            .border_color(hover.border)
                            .text_color(hover.text)
                    })
                    .active(move |s| {
                        s.bg(active.bg)
                            .border_color(active.border)
                            .text_color(active.text)
                    })
            })
            .when_some(self.tooltip, |this, text| {
                this.tooltip(crate::ui::tooltip::native(text))
            })
            .when_some(self.on_click.filter(|_| !inactive), |this, handler| {
                this.on_click(move |event, window, cx| handler(event, window, cx))
            })
            .children(icon_el)
            .when(has_label, |this| this.children(self.label))
            .refine_style(&self.style)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_button_colors_match_element_plus() {
        let (normal, hover, active, disabled) = variant_colors(ButtonType::Default, Variant::Solid);
        assert_eq!(normal.border, hex(0xDCDFE6));
        assert_eq!(normal.text, hex(0x606266));
        assert_eq!(hover.text, hex(0x409EFF));
        assert_eq!(hover.bg, hex(0xECF5FF));
        assert_eq!(hover.border, hex(0xC6E2FF));
        assert_eq!(active.border, hex(0x409EFF));
        assert_eq!(disabled.text, hex(0xA8ABB2));
    }

    #[test]
    fn primary_and_plain_colors() {
        let (normal, hover, active, disabled) = variant_colors(ButtonType::Primary, Variant::Solid);
        assert_eq!(normal.bg, hex(0x409EFF));
        assert_eq!(hover.bg, hex(0x79BBFF));
        assert_eq!(active.bg, hex(0x337ECC));
        assert_eq!(disabled.bg, hex(0xA0CFFF));
        let (plain, plain_hover, _, _) = variant_colors(ButtonType::Success, Variant::Plain);
        assert_eq!(plain.text, hex(0x67C23A));
        assert_eq!(plain.bg, hex(0xF0F9EB));
        assert_eq!(plain.border, hex(0xB3E19D));
        assert_eq!(plain_hover.bg, hex(0x67C23A));
        let (text, text_hover, text_active, _) = variant_colors(ButtonType::Default, Variant::Text);
        assert_eq!(text.text, hex(0x606266));
        assert_eq!(text_hover.bg, hex(0xF5F7FA));
        assert_eq!(text_active.bg, hex(0xF0F2F5));
    }
}
