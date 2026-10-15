//! `ElMessage`（toast）：显示在**当前窗口内部**顶部居中，距顶 16px，多条向下叠放（间隔 16px）；
//! 默认 3000ms 自动消失（鼠标悬停时暂停计时），无关闭按钮；`padding 11px 15px`、`gap 8px`、1px 边、圆角 4、
//! 无阴影；内容 14px、行高 1；宽度自适应，最大 `窗口宽 − 32`。淡入并从上方滑入。
//!
//! | 类型 | 背景 | 边框 | 文字 / 图标 | 图标 |
//! |---|---|---|---|---|
//! | success | `#F0F9EB` | `#E1F3D8` | `#67C23A` | SuccessFilled |
//! | info | `#F4F4F5` | `#E9E9EB` | `#909399` | InfoFilled |
//! | warning | `#FDF6EC` | `#FAECD8` | `#E6A23C` | WarningFilled |
//! | error | `#FEF0F0` | `#FDE2E2` | `#F56C6C` | CircleCloseFilled |
//!
//! ```ignore
//! ui::message::success(window, cx, "待办已保存");
//! ui::message::error_with(window, cx, "保存待办失败", &err);      // → 保存待办失败：<详情>
//! ui::message::show_for(window, cx, MessageKind::Info, text, Duration::from_secs(5));
//! ui::message::show_in(main_window_handle, cx, MessageKind::Success, "数据导入成功"); // 只有句柄时
//! ```
//!
//! 每个窗口的 Root 上都挂了 [`crate::ui::layers::UiLayer`] 浮层插件，窗口视图不需要自己渲染任何东西。

use std::time::Duration;

use gpui_kit::{AnyWindowHandle, App, AppContext as _, Hsla, SharedString, Window};

use crate::assets::EpIcon;
use crate::theme::ep;
use crate::ui::layers::UiLayer;

/// 默认显示时长
pub const DEFAULT_DURATION: Duration = Duration::from_millis(3000);
/// 错误详情最长 200 字（超出截断并加 `…`）
pub const MAX_DETAIL_LENGTH: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageKind {
    Success,
    Info,
    Warning,
    Error,
}

/// 某类型的配色：(背景, 边框, 文字 / 图标)
pub fn kind_colors(kind: MessageKind) -> (Hsla, Hsla, Hsla) {
    let ep = ep();
    match kind {
        MessageKind::Success => (ep.success_light_9, ep.success_light_8, ep.success),
        MessageKind::Info => (ep.info_light_9, ep.info_light_8, ep.info),
        MessageKind::Warning => (ep.warning_light_9, ep.warning_light_8, ep.warning),
        MessageKind::Error => (ep.danger_light_9, ep.danger_light_8, ep.danger),
    }
}

pub fn kind_icon(kind: MessageKind) -> EpIcon {
    match kind {
        MessageKind::Success => EpIcon::SuccessFilled,
        MessageKind::Info => EpIcon::InfoFilled,
        MessageKind::Warning => EpIcon::WarningFilled,
        MessageKind::Error => EpIcon::CircleCloseFilled,
    }
}

/// 旧 `utils/notify.ts errorMessage`：trim，超过 200 字截断并加 `…`
pub fn error_detail(detail: &str) -> String {
    let trimmed = detail.trim();
    if trimmed.chars().count() > MAX_DETAIL_LENGTH {
        let cut: String = trimmed.chars().take(MAX_DETAIL_LENGTH).collect();
        format!("{cut}…")
    } else {
        trimmed.to_string()
    }
}

/// 旧 `notifyError` 的文案：`操作描述：错误详情`（全角冒号；详情为空时只有操作描述）
pub fn error_text(prefix: &str, detail: &str) -> String {
    let detail = error_detail(detail);
    if detail.is_empty() {
        prefix.to_string()
    } else {
        format!("{prefix}：{detail}")
    }
}

/// 在当前窗口显示一条消息（默认 3s）
pub fn show(window: &mut Window, cx: &mut App, kind: MessageKind, text: impl Into<SharedString>) {
    show_for(window, cx, kind, text, DEFAULT_DURATION);
}

/// 指定显示时长
pub fn show_for(
    window: &mut Window,
    cx: &mut App,
    kind: MessageKind,
    text: impl Into<SharedString>,
    duration: Duration,
) {
    let text = text.into();
    match UiLayer::of(window, cx) {
        Some(layer) => layer.update(cx, |layer, cx| layer.push_message(kind, text, duration, cx)),
        None => log::warn!("[ui] 窗口没有 UiLayer，消息未显示: {text}"),
    }
}

pub fn success(window: &mut Window, cx: &mut App, text: impl Into<SharedString>) {
    show(window, cx, MessageKind::Success, text);
}

pub fn info(window: &mut Window, cx: &mut App, text: impl Into<SharedString>) {
    show(window, cx, MessageKind::Info, text);
}

pub fn warning(window: &mut Window, cx: &mut App, text: impl Into<SharedString>) {
    show(window, cx, MessageKind::Warning, text);
}

pub fn error(window: &mut Window, cx: &mut App, text: impl Into<SharedString>) {
    show(window, cx, MessageKind::Error, text);
}

/// 旧 `notifyError(e, prefix)`：记日志 + `prefix：详情` 错误消息
pub fn error_with(window: &mut Window, cx: &mut App, prefix: &str, detail: &str) {
    log::error!("{prefix}: {detail}");
    error(window, cx, error_text(prefix, detail));
}

/// 只有窗口句柄时（例如后台任务完成后）在该窗口显示消息；窗口已关闭则忽略
pub fn show_in(
    handle: AnyWindowHandle,
    cx: &mut App,
    kind: MessageKind,
    text: impl Into<SharedString>,
) {
    let text = text.into();
    let _ = cx.update_window(handle, |_, window, cx| show(window, cx, kind, text));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_text_matches_notify_ts() {
        assert_eq!(error_text("同步失败", "  网络错误 "), "同步失败：网络错误");
        assert_eq!(error_text("自动同步失败", ""), "自动同步失败");
        let long = "错".repeat(250);
        let text = error_text("导入失败", &long);
        assert!(text.ends_with('…'));
        assert_eq!(text.chars().count(), "导入失败：".chars().count() + 200 + 1);
    }

    #[test]
    fn colors_follow_element_plus() {
        let (bg, border, fg) = kind_colors(MessageKind::Error);
        assert_eq!(bg, crate::theme::hex(0xFEF0F0));
        assert_eq!(border, crate::theme::hex(0xFDE2E2));
        assert_eq!(fg, crate::theme::hex(0xF56C6C));
        assert_eq!(kind_icon(MessageKind::Warning), EpIcon::WarningFilled);
    }
}
