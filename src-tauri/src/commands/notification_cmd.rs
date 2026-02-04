use tauri::Manager;

/// 关闭指定的通知窗口
#[tauri::command]
pub fn close_notification_window(app_handle: tauri::AppHandle, window_label: String) -> Result<(), String> {
    if let Some(window) = app_handle.get_webview_window(&window_label) {
        window.close().map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// 关闭所有通知窗口
#[tauri::command]
pub fn close_all_notification_windows(app_handle: tauri::AppHandle) -> Result<(), String> {
    // 获取所有窗口，关闭以 "notification_" 开头的窗口
    let windows = app_handle.webview_windows();
    for (label, window) in windows {
        if label.starts_with("notification_") {
            let _ = window.close();
        }
    }
    Ok(())
}
