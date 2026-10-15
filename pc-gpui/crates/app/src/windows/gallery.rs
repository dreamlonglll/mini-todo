//! 控件样例窗口（仅调试：`MINI_TODO_DEBUG_OPEN=gallery`）：一屏列出 `ui/` 里的 Element Plus 风格控件，
//! 改控件后用截图脚本对照 Element Plus 外观。不是产品功能，任何窗口实现者都可以往里加自己新建的控件。

use gpui_kit::component::input::InputState;
use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, Bounds, Context, Entity, IntoElement,
    ParentElement as _, Render, Styled as _, Window, div, px, relative,
};

use super::{WindowKind, WindowManager, WindowSpec, logical_size};
use crate::assets::EpIcon;
use crate::theme::{self, ep};
use crate::ui::button::EpButton;
use crate::ui::icon::icon;
use crate::ui::input::EpInput;
use crate::ui::message::{self, MessageKind};
use crate::ui::message_box::{self, BoxKind, ConfirmOptions};
use crate::ui::title_bar::WindowTitleBar;
use crate::ui::tooltip::EpTooltip;

pub struct GalleryWindow {
    input: Entity<InputState>,
    input_small: Entity<InputState>,
    input_disabled: Entity<InputState>,
}

impl GalleryWindow {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            input: cx.new(|cx| InputState::new(window, cx).placeholder("请输入待办标题")),
            input_small: cx.new(|cx| InputState::new(window, cx).default_value("small 输入框")),
            input_disabled: cx.new(|cx| InputState::new(window, cx).default_value("禁用")),
        }
    }
}

fn row(label: &'static str) -> gpui_kit::Div {
    div().flex().flex_wrap().items_center().gap(px(12.)).child(
        div()
            .w(px(90.))
            .text_size(px(12.))
            .text_color(theme::light().text_tertiary)
            .child(label),
    )
}

impl Render for GalleryWindow {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .font(theme::ui_font())
            .text_size(theme::FONT_SIZE)
            .line_height(relative(theme::LINE_HEIGHT))
            .bg(theme::hex(0xFFFFFF))
            .text_color(theme::light().text_primary)
            .child(
                WindowTitleBar::new("gallery-title", "控件样例").action(
                    EpButton::new("g-edit")
                        .text()
                        .primary()
                        .icon(EpIcon::Edit)
                        .icon_size(14.)
                        .label("编辑"),
                ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(14.))
                    .p(px(20.))
                    .child(
                        row("el-button")
                            .child(EpButton::new("g-default").label("默认"))
                            .child(EpButton::new("g-primary").primary().label("保存"))
                            .child(
                                EpButton::new("g-success")
                                    .success()
                                    .plain()
                                    .label("完成任务"),
                            )
                            .child(
                                EpButton::new("g-warning")
                                    .warning()
                                    .plain()
                                    .label("重新打开"),
                            )
                            .child(EpButton::new("g-danger").danger().label("删除"))
                            .child(
                                EpButton::new("g-disabled")
                                    .primary()
                                    .disabled(true)
                                    .label("禁用"),
                            )
                            .child(
                                EpButton::new("g-loading")
                                    .primary()
                                    .loading(true)
                                    .label("同步中"),
                            ),
                    )
                    .child(
                        row("text / small")
                            .child(EpButton::new("g-text").text().label("文字按钮"))
                            .child(
                                EpButton::new("g-close")
                                    .text()
                                    .icon(EpIcon::Close)
                                    .icon_size(14.),
                            )
                            .child(EpButton::new("g-link").link().primary().label("链接"))
                            .child(EpButton::new("g-small").small().label("今天"))
                            .child(
                                EpButton::new("g-small-p")
                                    .small()
                                    .primary()
                                    .icon(EpIcon::Plus)
                                    .label("添加"),
                            )
                            .child(EpButton::new("g-circle").circle().icon(EpIcon::Search)),
                    )
                    .child(
                        row("el-input")
                            .child(EpInput::new(&self.input).w(px(220.)))
                            .child(EpInput::new(&self.input_small).small().w(px(160.)))
                            .child(
                                EpInput::new(&self.input_disabled)
                                    .disabled(true)
                                    .w(px(120.)),
                            ),
                    )
                    .child(
                        row("el-tooltip").child(
                            EpTooltip::new(
                                "g-tip",
                                "每周 周一、周三、周五",
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(4.))
                                    .child(icon(EpIcon::RefreshRight, 14., ep().primary))
                                    .child("悬停显示 tooltip"),
                            )
                            .show_after_ms(300),
                        ),
                    )
                    .child(
                        row("ElMessage")
                            .child(EpButton::new("g-m-s").small().label("success").on_click(
                                |_, w, cx| message::show(w, cx, MessageKind::Success, "待办已保存"),
                            ))
                            .child(EpButton::new("g-m-i").small().label("info").on_click(
                                |_, w, cx| {
                                    message::show(
                                        w,
                                        cx,
                                        MessageKind::Info,
                                        "已是最新，没有需要同步的更改",
                                    )
                                },
                            ))
                            .child(EpButton::new("g-m-w").small().label("warning").on_click(
                                |_, w, cx| {
                                    message::show(
                                        w,
                                        cx,
                                        MessageKind::Warning,
                                        "请先在设置中配置 WebDAV 服务器",
                                    )
                                },
                            ))
                            .child(EpButton::new("g-m-e").small().label("error").on_click(
                                |_, w, cx| message::error_with(w, cx, "同步失败", "网络连接超时"),
                            )),
                    )
                    .child(
                        row("MessageBox").child(
                            EpButton::new("g-confirm")
                                .small()
                                .label("删除确认")
                                .on_click(|_, window, cx| {
                                    message_box::confirm(
                                        window,
                                        cx,
                                        ConfirmOptions::new(
                                            "删除确认",
                                            "确定要删除待办\"示例\"吗？",
                                        )
                                        .confirm_text("删除")
                                        .kind(BoxKind::Warning),
                                        |ok, window, cx| {
                                            message::info(
                                                window,
                                                cx,
                                                if ok { "已确认" } else { "已取消" },
                                            )
                                        },
                                    )
                                }),
                        ),
                    ),
            )
    }
}

/// 打开控件样例窗口
pub fn open(cx: &mut App) -> Option<AnyWindowHandle> {
    let spec = WindowSpec::new(
        "控件样例",
        Bounds::new(Default::default(), logical_size(820., 420.)),
    );
    WindowManager::open_child(cx, None, WindowKind::Gallery, None, spec, |window, cx| {
        cx.new(|cx| GalleryWindow::new(window, cx))
    })
}
