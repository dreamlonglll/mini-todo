mod db;
mod commands;
mod services;

use db::Database;
use services::NotificationService;
use tauri::{Manager, Emitter};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use commands::{
    get_todos, create_todo, update_todo, delete_todo, reorder_todos,
    create_subtask, update_subtask, delete_subtask,
    get_settings, save_settings, set_window_fixed_mode, reset_window,
    export_data, import_data,
    is_fixed_mode,
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
            
            // 创建系统托盘菜单
            let toggle_fixed = MenuItem::with_id(app, "toggle_fixed", "固定/取消固定", true, None::<&str>)?;
            let reset = MenuItem::with_id(app, "reset", "重置位置/大小", true, None::<&str>)?;
            let add_todo = MenuItem::with_id(app, "add_todo", "添加待办项", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
            
            let menu = Menu::with_items(app, &[&toggle_fixed, &reset, &add_todo, &quit])?;
            
            let _tray = TrayIconBuilder::new()
                .icon(app.default_window_icon().unwrap().clone())
                .menu(&menu)
                .show_menu_on_left_click(true)
                .on_menu_event(move |app: &tauri::AppHandle, event| {
                    match event.id().as_ref() {
                        "toggle_fixed" => {
                            // 发送事件给前端处理
                            if let Some(window) = app.get_webview_window("main") {
                                let _ = window.emit::<()>("tray-toggle-fixed", ());
                            }
                        }
                        "reset" => {
                            // 重置窗口位置和大小
                            if let Some(window) = app.get_webview_window("main") {
                                let _ = commands::reset_webview_window(window);
                            }
                            // 发送事件通知前端更新状态
                            if let Some(window) = app.get_webview_window("main") {
                                let _ = window.emit::<()>("tray-reset-window", ());
                            }
                        }
                        "add_todo" => {
                            // 发送事件给前端打开添加待办窗口
                            if let Some(window) = app.get_webview_window("main") {
                                let _ = window.emit::<()>("tray-add-todo", ());
                            }
                        }
                        "quit" => {
                            app.exit(0);
                        }
                        _ => {}
                    }
                })
                .on_tray_icon_event(|tray: &tauri::tray::TrayIcon, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        let app = tray.app_handle();
                        if let Some(webview_window) = app.get_webview_window("main") {
                            let _ = webview_window.unminimize();
                            let _ = webview_window.show();
                            let _ = webview_window.set_focus();
                        }
                    }
                })
                .build(app)?;
            
            // 启动通知调度器
            NotificationService::start_scheduler(app.handle().clone());
            
            // 启动固定模式监听器（定时检测窗口最小化状态）
            let handle = app.handle().clone();
            std::thread::spawn(move || {
                loop {
                    std::thread::sleep(std::time::Duration::from_millis(200));
                    
                    // 只在固定模式下检测
                    if is_fixed_mode() {
                        if let Some(window) = handle.get_webview_window("main") {
                            // 检查窗口是否被最小化
                            if window.is_minimized().unwrap_or(false) {
                                let _ = window.unminimize();
                                let _ = window.show();
                            }
                        }
                    }
                }
            });
            
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
            reset_window,
            // 数据导入导出命令
            export_data,
            import_data,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|_app_handle, _event| {
            // 事件监听（保留空实现）
        });
}
