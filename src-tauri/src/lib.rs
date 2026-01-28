mod db;
mod commands;
mod services;

use db::Database;
use services::NotificationService;
use tauri::Manager;
use commands::{
    get_todos, create_todo, update_todo, delete_todo, reorder_todos,
    create_subtask, update_subtask, delete_subtask,
    get_settings, save_settings, set_window_fixed_mode,
    export_data, import_data,
};

#[cfg(target_os = "windows")]
fn setup_window_rounded_corners(window: &tauri::WebviewWindow) {
    use raw_window_handle::HasWindowHandle;
    use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND};
    use windows::Win32::Foundation::HWND;

    if let Ok(handle) = window.window_handle() {
        if let raw_window_handle::RawWindowHandle::Win32(win32_handle) = handle.as_raw() {
            let hwnd = HWND(win32_handle.hwnd.get() as *mut _);
            unsafe {
                let preference = DWMWCP_ROUND;
                let _ = DwmSetWindowAttribute(
                    hwnd,
                    DWMWA_WINDOW_CORNER_PREFERENCE,
                    &preference as *const _ as *const _,
                    std::mem::size_of_val(&preference) as u32,
                );
            }
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // 初始化数据库
    let database = Database::new().expect("Failed to initialize database");

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .manage(database)
        .setup(|app| {
            #[cfg(target_os = "windows")]
            {
                if let Some(window) = app.get_webview_window("main") {
                    setup_window_rounded_corners(&window);
                }
            }
            
            // 启动通知调度器
            NotificationService::start_scheduler(app.handle().clone());
            
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            // TODO 命令
            get_todos,
            create_todo,
            update_todo,
            delete_todo,
            reorder_todos,
            // 子任务命令
            create_subtask,
            update_subtask,
            delete_subtask,
            // 窗口设置命令
            get_settings,
            save_settings,
            set_window_fixed_mode,
            // 数据导入导出命令
            export_data,
            import_data,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
