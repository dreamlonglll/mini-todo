//! 系统通知（旧 `tauri-plugin-notification`：Windows 上同样是 `tauri-winrt-notification` 发 Toast）。
//!
//! 标题 = 待办标题，正文 = 描述或 `待办事项提醒`。
//!
//! - Windows：notify-rust 发 Toast，AppUserModelID 与 `set_app_identity` 相同的 [`crate::APP_IDENTIFIER`]（安装包的开始菜单
//!   快捷方式登记了它，Toast 归属到 Mini Todo）。发送失败返回 `Err`：调度器据此不推进提醒状态，下一分钟重试。
//!   阻塞调用，只在后台线程用。Linux 同样走 notify-rust（只用于 CI）。
//! - macOS：GPUI 的 `show_system_notification`（`UNUserNotificationCenter`），通知归属于 `.app` 的 bundle id
//!   `com.tauri-app.mini-todo`（与 2.5.0 mac 版相同，用户给旧版的通知授权 / 显示设置沿用）。不用 notify-rust：它在 mac 上走
//!   已弃用的 `NSUserNotificationCenter`，不设 `set_application` 时以 Finder 的名义显示，设了也要靠替换 `NSBundle` 方法冒充身份。
//!   限制：
//!   - 必须在主线程调用（调度器经 channel 交给前台，见 `scheduler.rs`）
//!   - 只在从 `.app` 启动时可用（没有 bundle id 时 GPUI 只记一条日志、不发，避免系统 abort），`cargo run` 的裸二进制收不到
//!   - 第一次发通知时才请求授权；未签名的 `.app` 会被系统拒绝授权，发布包至少要 ad-hoc 签名
//!   - **拿不到投递结果**（授权被拒、发送失败都只在 GPUI 里记日志）：交给前台即视为已投递、推进提醒状态，
//!     Windows 那条"发送失败不推进、下一分钟重试"在 mac 上不成立（否则授权被拒时同一条提醒会每分钟重发）
//!   - 授权弹框出现前就投递的第一条通知可能被系统丢掉

/// 发一条系统通知（Windows Toast / Linux）。阻塞调用，只在后台线程用
#[cfg(not(target_os = "macos"))]
pub fn show_system_notification(title: &str, body: &str) -> Result<(), String> {
    let mut notification = notify_rust::Notification::new();
    notification.summary(title).body(body);
    #[cfg(windows)]
    notification.app_id(crate::APP_IDENTIFIER);
    #[cfg(not(windows))]
    notification.appname(crate::APP_NAME);
    notification.show().map(|_| ()).map_err(|e| e.to_string())
}

/// 发一条系统通知（macOS，`UNUserNotificationCenter`）。必须在主线程（前台）调用；投递结果拿不到（见模块说明）。
/// `tag` 相同的新通知会替换通知中心里的旧通知
#[cfg(target_os = "macos")]
pub fn show_system_notification(cx: &gpui_kit::App, tag: &str, title: &str, body: &str) {
    cx.show_system_notification(gpui_kit::SystemNotification {
        tag: tag.to_string().into(),
        title: title.to_string().into(),
        body: body.to_string().into(),
        actions: Vec::new(),
    });
}
