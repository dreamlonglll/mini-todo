mod commands;
mod db;
mod services;

use db::Database;
use services::NotificationService;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{Emitter, Manager};
use tauri_plugin_autostart::ManagerExt;

use commands::{
    close_all_notification_windows, close_notification_window, create_subtask, create_todo,
    delete_screen_config, delete_subtask, delete_todo, export_data, export_data_to_file,
    fetch_holidays, get_auto_hide_enabled, get_change_seq, get_fixed_embed_desktop, get_images_dir,
    get_notification_type, get_screen_config, get_setting, get_show_calendar, get_subtask,
    get_sync_settings, get_system_fonts, get_text_theme, get_todo, get_todo_font_family,
    get_todo_font_size, get_todos, get_top_on_wake, get_window_background,
    get_window_persist_state, import_data, import_data_from_file, import_subtasks_from_paths,
    is_desktop_mode, is_fixed_mode, list_screen_configs, reorder_subtasks, reorder_todos,
    reset_window, save_screen_config, save_settings, save_subtask_image, save_sync_settings,
    set_auto_hide_cursor_inside, set_auto_hide_enabled, set_fixed_embed_desktop,
    set_notification_type, set_setting, set_show_calendar, set_text_theme, set_todo_font_family,
    set_todo_font_size, set_top_on_wake, set_window_background, set_window_desktop_mode,
    set_window_fixed_mode, sync_auto_start_state, update_screen_config_name, update_subtask,
    update_todo, webdav_force_pull, webdav_force_push, webdav_sync, webdav_test_connection,
};

/// 开机自启动时附带的命令行参数（见 autostart 插件初始化）
const AUTOSTART_ARG: &str = "--autostart";

/// 托盘左键连点去抖：双击会先后触发两次单击抬起事件，第二下不必再唤起一遍
const TRAY_CLICK_DEBOUNCE: Duration = Duration::from_millis(300);

/// 上一次被处理的托盘左键点击时间（`Instant` 单调递增，不受系统时钟回拨影响）
static LAST_TRAY_CLICK: Mutex<Option<Instant>> = Mutex::new(None);

/// 这次托盘点击是否需要处理；处理时记下时间
fn accept_tray_click(last: &Mutex<Option<Instant>>, now: Instant) -> bool {
    let mut last = last.lock().unwrap_or_else(|e| e.into_inner());
    let accept = last.is_none_or(|prev| now.saturating_duration_since(prev) >= TRAY_CLICK_DEBOUNCE);
    if accept {
        *last = Some(now);
    }
    accept
}

#[cfg(target_os = "windows")]
fn setup_window_rounded_corners(window: &tauri::WebviewWindow) {
    use raw_window_handle::HasWindowHandle;
    use windows::Win32::Foundation::HWND;
    use windows::Win32::Graphics::Dwm::{
        DwmSetWindowAttribute, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND,
    };

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

#[cfg(target_os = "macos")]
fn setup_macos_transparent_webview(window: &tauri::WebviewWindow) {
    use tauri::webview::Color;

    // 把 WKWebView 底色置空，让 CSS 控制最终显示：深色模式透明透出桌面，浅色模式由 .app-container 填白。
    if let Err(e) = window.set_background_color(Some(Color(0, 0, 0, 0))) {
        log::warn!(
            "Failed to set macOS webview background transparent: {:?}",
            e
        );
    }
}

/// 日志：应用日志目录下的 `mini-todo.log`（约 2MB 轮转、保留 1 份旧日志）+ 标准输出，Info 级。
///
/// release 版是 Windows 子系统程序，没有控制台，以前 `eprintln!` 的同步 / 迁移日志全部丢失（C4）。
/// Windows 上日志目录为 `%LOCALAPPDATA%\com.tauri-app.mini-todo\logs`。
fn log_plugin<R: tauri::Runtime>() -> tauri::plugin::TauriPlugin<R> {
    use tauri_plugin_log::{RotationStrategy, Target, TargetKind, TimezoneStrategy};

    tauri_plugin_log::Builder::new()
        .targets([
            Target::new(TargetKind::LogDir { file_name: None }),
            Target::new(TargetKind::Stdout),
        ])
        .level(log::LevelFilter::Info)
        .max_file_size(2 * 1024 * 1024)
        .rotation_strategy(RotationStrategy::KeepSome(1))
        .timezone_strategy(TimezoneStrategy::UseLocal)
        .build()
}

/// 打开数据库；失败时不 panic：记日志、提示用户（Windows 弹框，其它平台写 stderr）后以退出码 1 结束。
///
/// 放在 setup 里（日志插件已初始化）而不是 Builder 之前：迁移 / 备份的日志才能落盘，
/// 重复启动的第二个实例也会在单实例插件里先退出，不会去碰数据库。
fn open_database_or_exit() -> Database {
    match Database::new() {
        Ok(database) => database,
        Err(e) => {
            let path = db::paths::db_path();
            log::error!("[db] 数据库初始化失败（{}）: {}", path.display(), e);
            show_fatal_error(&format!(
                "Mini Todo 无法打开数据库，程序将退出。\n\n数据库位置：{}\n错误：{}\n\n升级前的自动备份在同目录的 backups 文件夹中。",
                path.display(),
                e
            ));
            std::process::exit(1);
        }
    }
}

#[cfg(target_os = "windows")]
fn show_fatal_error(message: &str) {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};

    let wide = |s: &str| -> Vec<u16> { s.encode_utf16().chain(std::iter::once(0)).collect() };
    let text = wide(message);
    let caption = wide("Mini Todo");
    unsafe {
        MessageBoxW(
            HWND::default(),
            PCWSTR(text.as_ptr()),
            PCWSTR(caption.as_ptr()),
            MB_OK | MB_ICONERROR,
        );
    }
}

#[cfg(not(target_os = "windows"))]
fn show_fatal_error(message: &str) {
    // 这里刻意直接写 stderr：日志之外，终端启动时用户也要能看到
    eprintln!("{}", message);
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // 单实例必须第一个注册：重复启动的进程在这里就退出，不会初始化其它插件、打开数据库，
        // 也不会再起一套提醒调度与同步循环（以前每条提醒弹两次、两个同步互相 412）
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            // 开机自启动拉起的重复实例不打扰用户
            if argv.iter().any(|arg| arg == AUTOSTART_ARG) {
                return;
            }
            log::info!("[app] 检测到重复启动，唤起已运行的窗口");
            commands::bring_main_window_to_front(app);
        }))
        .plugin(log_plugin())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec![AUTOSTART_ARG]),
        ))
        .setup(|app| {
            // 第一件事：管理数据库状态。setup 跑在主线程，返回前前端的任何 IPC 都处理不了，
            // 所以命令不会在 State<Database> 就位之前被调用
            let database = open_database_or_exit();
            // 轮询线程拿不到 State<Database>，启动时先把 top_on_wake 等灌进运行时缓存
            commands::reload_runtime_prefs(&database);
            app.manage(database);

            if let Some(window) = app.get_webview_window("main") {
                commands::remember_main_window(&window);

                #[cfg(target_os = "windows")]
                setup_window_rounded_corners(&window);

                #[cfg(target_os = "macos")]
                setup_macos_transparent_webview(&window);
            }

            // 创建系统托盘菜单项
            let toggle_fixed = CheckMenuItem::with_id(
                app,
                "toggle_fixed",
                "固定模式",
                true,
                is_fixed_mode(),
                None::<&str>,
            )?;
            let reset = MenuItem::with_id(app, "reset", "重置位置", true, None::<&str>)?;
            let add_todo = MenuItem::with_id(app, "add_todo", "添加待办项", true, None::<&str>)?;
            let open_settings =
                MenuItem::with_id(app, "open_settings", "打开设置", true, None::<&str>)?;
            // 自启自愈：is_enabled 只判断注册表键是否存在，不校验路径。便携版目录被移动/
            // 换成安装版后，旧记录仍在但指向失效路径，这里重新 enable 一次刷新为当前 exe。
            let auto_start_enabled = {
                let autolaunch = app.autolaunch();
                let enabled = autolaunch.is_enabled().unwrap_or(false);
                if enabled {
                    if let Err(e) = autolaunch.enable() {
                        log::warn!("Failed to refresh autostart entry: {e}");
                    }
                }
                enabled
            };
            let auto_start = CheckMenuItem::with_id(
                app,
                "auto_start",
                "开机自启动",
                true,
                auto_start_enabled,
                None::<&str>,
            )?;
            let separator1 = PredefinedMenuItem::separator(app)?;
            let separator2 = PredefinedMenuItem::separator(app)?;
            let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;

            let menu = Menu::with_items(
                app,
                &[
                    &add_todo,
                    &separator1,
                    &toggle_fixed,
                    &reset,
                    &open_settings,
                    &auto_start,
                    &separator2,
                    &quit,
                ],
            )?;

            // 保存托盘菜单项引用，供 set_window_fixed_mode / set_window_desktop_mode 同步勾选状态
            // （嵌入桌面的固定模式对托盘来说同样是"固定模式"）
            commands::set_tray_toggle_fixed_item(toggle_fixed.clone());
            // 保存自启菜单项引用，供设置面板切换后同步勾选状态
            commands::set_tray_auto_start_item(auto_start.clone());

            let _tray = TrayIconBuilder::new()
                .icon(app.default_window_icon().unwrap().clone())
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(move |app: &tauri::AppHandle, event| {
                    match event.id().as_ref() {
                        "toggle_fixed" => {
                            if let Some(window) = app.get_webview_window("main") {
                                let _ = window.emit::<()>("tray-toggle-fixed", ());
                            }
                        }
                        "reset" => {
                            // 重置窗口位置
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
                        "open_settings" => {
                            // 发送事件给前端打开设置窗口
                            if let Some(window) = app.get_webview_window("main") {
                                let _ = window.show();
                                let _ = window.set_focus();
                                let _ = window.emit::<()>("tray-open-settings", ());
                            }
                        }
                        "auto_start" => {
                            // 切换开机自启动
                            let autolaunch = app.autolaunch();
                            let currently_enabled = autolaunch.is_enabled().unwrap_or(false);
                            let result = if currently_enabled {
                                autolaunch.disable()
                            } else {
                                autolaunch.enable()
                            };
                            if let Err(e) = result {
                                log::warn!("Failed to toggle autostart: {e}");
                            }
                            // 不依赖 CheckMenuItem 的自动翻转：它只翻显示不看结果，
                            // 与设置面板交叉操作或 enable/disable 失败时会显示反转，
                            // 这里按注册表实际状态回写勾选
                            let enabled_now = autolaunch.is_enabled().unwrap_or(false);
                            commands::sync_tray_auto_start_checked(enabled_now);
                        }
                        "quit" => {
                            app.exit(0);
                        }
                        _ => {}
                    }
                })
                .on_tray_icon_event(|tray: &tauri::tray::TrayIcon, event| {
                    // 左键单击：把主窗口显示并抬到最前（issue #10）。固定模式下窗口不在任务栏，
                    // 托盘是唯一入口；新建待办在右键菜单里。以前的"双击"分支与单击做的事完全一样，已合并
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        if accept_tray_click(&LAST_TRAY_CLICK, Instant::now()) {
                            commands::bring_main_window_to_front(tray.app_handle());
                        }
                    }
                })
                .build(app)?;

            // 启动通知调度器
            NotificationService::start_scheduler(app.handle().clone());

            // 启动窗口模式监听器（固定模式：最小化守护 + 贴边隐藏；桌面模式：宿主存活检测）
            let handle = app.handle().clone();
            std::thread::spawn(move || {
                loop {
                    std::thread::sleep(std::time::Duration::from_millis(200));

                    if is_desktop_mode() {
                        // 桌面模式下窗口不可最小化、也不贴边，只需盯着 Explorer 重启后重新挂载
                        if let Some(window) = handle.get_webview_window("main") {
                            commands::tick_desktop_mode(&window);
                        }
                    } else if is_fixed_mode() {
                        if let Some(window) = handle.get_webview_window("main") {
                            // 被最小化就立刻还原：固定模式下没有任务栏入口可以点回来
                            commands::restore_if_minimized(&window);

                            // 固定模式下贴边自动隐藏/唤起
                            commands::tick_auto_hide(&window);
                        }
                    }
                }
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            // TODO 命令
            get_todos,
            get_todo,
            get_change_seq,
            create_todo,
            update_todo,
            delete_todo,
            reorder_todos,
            reorder_subtasks,
            // 子任务命令
            create_subtask,
            update_subtask,
            delete_subtask,
            import_subtasks_from_paths,
            // 图片命令
            get_images_dir,
            get_subtask,
            save_subtask_image,
            // 窗口设置命令
            save_settings,
            // 白名单设置项（视图模式）
            get_setting,
            set_setting,
            get_text_theme,
            set_text_theme,
            set_window_fixed_mode,
            set_window_desktop_mode,
            get_auto_hide_enabled,
            set_auto_hide_enabled,
            get_top_on_wake,
            set_top_on_wake,
            get_fixed_embed_desktop,
            set_fixed_embed_desktop,
            get_window_background,
            set_window_background,
            set_auto_hide_cursor_inside,
            get_window_persist_state,
            reset_window,
            sync_auto_start_state,
            // 屏幕配置命令
            get_screen_config,
            save_screen_config,
            list_screen_configs,
            delete_screen_config,
            update_screen_config_name,
            // 日历设置命令
            get_show_calendar,
            set_show_calendar,
            // 字体设置命令
            get_system_fonts,
            get_todo_font_family,
            set_todo_font_family,
            get_todo_font_size,
            set_todo_font_size,
            // 数据导入导出命令
            export_data,
            import_data,
            export_data_to_file,
            import_data_from_file,
            // 节假日命令
            fetch_holidays,
            // 通知设置命令
            get_notification_type,
            set_notification_type,
            // 通知窗口命令
            close_notification_window,
            close_all_notification_windows,
            // WebDAV 同步命令
            get_sync_settings,
            save_sync_settings,
            webdav_test_connection,
            webdav_sync,
            webdav_force_pull,
            webdav_force_push,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|_app_handle, _event| {
            // 事件监听（保留空实现）
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tray_clicks_are_debounced_with_a_monotonic_clock() {
        let last = Mutex::new(None);
        let t0 = Instant::now();
        assert!(accept_tray_click(&last, t0), "第一下总是处理");
        assert!(
            !accept_tray_click(&last, t0 + Duration::from_millis(120)),
            "双击的第二下不重复唤起"
        );
        assert!(accept_tray_click(&last, t0 + TRAY_CLICK_DEBOUNCE));
        // 时间"倒退"（理论上 Instant 不会，但不能因此 panic 或永久拒绝）
        assert!(!accept_tray_click(&last, t0));
        assert!(accept_tray_click(
            &last,
            t0 + TRAY_CLICK_DEBOUNCE * 2 + Duration::from_millis(1)
        ));
    }
}
