//! 系统通知（旧 `tauri-plugin-notification`：Windows 上同样是 `tauri-winrt-notification` 发 Toast）。
//!
//! 标题 = 待办标题，正文 = 描述或 `待办事项提醒`。Windows 用与 `set_app_identity` 相同的 AppUserModelID
//! [`crate::APP_IDENTIFIER`]（安装包的开始菜单快捷方式登记了它，Toast 归属到 Mini Todo）。
//! 发送失败返回 `Err`：调度器据此不推进提醒状态，下一分钟重试。阻塞调用，只在后台线程用。

/// 发一条系统通知
pub fn show_system_notification(title: &str, body: &str) -> Result<(), String> {
    let mut notification = notify_rust::Notification::new();
    notification.summary(title).body(body);
    #[cfg(windows)]
    notification.app_id(crate::APP_IDENTIFIER);
    #[cfg(not(windows))]
    notification.appname(crate::APP_NAME);
    notification.show().map(|_| ()).map_err(|e| e.to_string())
}
