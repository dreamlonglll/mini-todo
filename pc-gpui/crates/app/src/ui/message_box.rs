//! `ElMessageBox.confirm`：窗口内模态确认框。
//!
//! 遮罩满窗口 `rgba(0,0,0,0.5)`（淡入 0.2s）；对话框白底、圆角 4、阴影 `--el-box-shadow`、padding 12、
//! 宽 `min(420, 窗口宽 − 32)`、水平垂直居中（淡入 + 自上 20px 落下 0.3s）。
//! 头部标题 18px / 行高 24 / `#303133`、下边距 12；右上角 40×40 关闭区（Close 16px `#909399`，hover `#409EFF`）。
//! 内容：状态图标 24px（warning `WarningFilled #E6A23C` / info `InfoFilled #909399`）+ 消息 14px / 行高 24 / `#606266`，间距 12。
//! 按钮行右对齐、上边距 12：取消（默认按钮）在左、确认（primary）在右，间距 12。
//! 点遮罩 / Esc / × = 取消；Enter = 确认。同一窗口同时请求多个时排队依次显示。
//!
//! ```ignore
//! ui::message_box::confirm(
//!     window, cx,
//!     ConfirmOptions::new("删除确认", format!("确定要删除待办\"{title}\"吗？"))
//!         .confirm_text("删除").cancel_text("取消"),
//!     move |confirmed, window, cx| if confirmed { /* … */ },
//! );
//! ```
//!
//! 回调在确认框关闭后的下一个 effect 周期执行（`window.defer`），里面可以放心再弹消息 / 确认框。

use gpui_kit::{App, SharedString, Window};

use crate::ui::layers::UiLayer;

/// 状态图标
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BoxKind {
    #[default]
    Warning,
    Info,
    /// 不显示图标
    None,
}

/// 确认框内容
#[derive(Debug, Clone, PartialEq)]
pub struct ConfirmOptions {
    pub title: SharedString,
    pub message: SharedString,
    pub confirm_text: SharedString,
    pub cancel_text: SharedString,
    pub kind: BoxKind,
}

impl ConfirmOptions {
    /// 默认按钮文案 `确定` / `取消`、warning 图标
    pub fn new(title: impl Into<SharedString>, message: impl Into<SharedString>) -> Self {
        Self {
            title: title.into(),
            message: message.into(),
            confirm_text: "确定".into(),
            cancel_text: "取消".into(),
            kind: BoxKind::Warning,
        }
    }

    pub fn confirm_text(mut self, text: impl Into<SharedString>) -> Self {
        self.confirm_text = text.into();
        self
    }

    pub fn cancel_text(mut self, text: impl Into<SharedString>) -> Self {
        self.cancel_text = text.into();
        self
    }

    pub fn kind(mut self, kind: BoxKind) -> Self {
        self.kind = kind;
        self
    }
}

pub(crate) type ConfirmCallback = Box<dyn FnOnce(bool, &mut Window, &mut App)>;

/// 弹出确认框；用户确认 / 取消后调用 `on_result(confirmed, window, cx)`
pub fn confirm(
    window: &mut Window,
    cx: &mut App,
    options: ConfirmOptions,
    on_result: impl FnOnce(bool, &mut Window, &mut App) + 'static,
) {
    match UiLayer::of(window, cx) {
        Some(layer) => layer.update(cx, |layer, cx| {
            layer.push_confirm(options, Box::new(on_result), window, cx)
        }),
        None => {
            log::warn!("[ui] 窗口没有 UiLayer，确认框按取消处理: {}", options.title);
            let on_result: ConfirmCallback = Box::new(on_result);
            window.defer(cx, move |window, cx| on_result(false, window, cx));
        }
    }
}

/// 当前窗口是否有确认框在显示（窗口自己的快捷键在此期间应当让路）
pub fn is_open(window: &Window, cx: &App) -> bool {
    UiLayer::of(window, cx).is_some_and(|layer| layer.read(cx).has_confirm())
}
