mod commands;
mod db;
mod services;

use db::Database;
use services::agent::AgentManager;
use services::scheduler::engine::TaskScheduler;
use services::NotificationService;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{Emitter, Manager};
use tauri_plugin_autostart::ManagerExt;

// 记录上次点击时间（用于双击检测）
static LAST_CLICK_TIME: AtomicU64 = AtomicU64::new(0);
const DOUBLE_CLICK_THRESHOLD_MS: u64 = 500;
use commands::{
    close_all_notification_windows,
    close_notification_window,
    create_subtask,
    create_todo,
    delete_screen_config,
    delete_subtask,
    delete_todo,
    export_data,
    get_images_dir,
    get_subtask,
    get_sync_settings,
    fetch_holidays,
    get_auto_hide_enabled,
    get_notification_type,
    get_screen_config,
    get_settings,
    get_show_calendar,
    get_todos,
    get_window_persist_state,
    import_data,
    is_fixed_mode,
    list_screen_configs,
    reorder_todos,
    reset_window,
    save_screen_config,
    save_settings,
    save_subtask_image,
    save_sync_settings,
    set_auto_hide_cursor_inside,
    set_auto_hide_enabled,
    set_macos_vibrancy,
    set_notification_type,
    set_show_calendar,
    set_window_fixed_mode,
    update_screen_config_name,
    update_subtask,
    update_todo,
    webdav_apply_remote,
    webdav_auto_sync,
    webdav_download_sync,
    webdav_test_connection,
    webdav_upload_sync,
    // Agent 命令
    auto_detect_agents,
    cancel_agent_execution,
    check_agent_health,
    check_all_agents_health,
    create_agent,
    delete_agent,
    get_agent,
    get_agent_execution_by_subtask,
    get_agent_execution_state,
    get_agents,
    start_agent_execution,
    update_agent,
    // 调度器命令
    add_task_dependency,
    check_dependencies_met,
    get_scheduler_status,
    get_task_dependencies,
    remove_task_dependency,
    start_scheduler,
    stop_scheduler,
    submit_task_to_scheduler,
    update_subtask_max_retries,
    update_subtask_priority,
    update_subtask_schedule_status,
    update_subtask_timeout,
    update_todo_schedule_config,
    validate_cron_expression,
    get_next_cron_execution,
    get_scheduled_todos,
    // 审核命令
    approve_review,
    reject_review,
    // 工作流命令
    get_workflow_steps,
    set_workflow_steps,
    start_workflow,
    pause_workflow,
    reset_workflow,
    skip_workflow_step,
    get_workflow_executions,
    // Prompt 模板命令
    create_prompt_template,
    delete_prompt_template,
    get_prompt_template,
    get_prompt_templates,
    get_prompt_templates_by_category,
    render_prompt_template,
    update_prompt_template,
};

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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // 初始化数据库
    let database = Database::new().expect("Failed to initialize database");
    let agent_manager = AgentManager::new();
    let task_scheduler = std::sync::Arc::new(TaskScheduler::new());

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--autostart"]),
        ))
        .manage(database)
        .manage(agent_manager)
        .manage(task_scheduler.clone())
        .setup(|app| {
            #[cfg(target_os = "windows")]
            {
                if let Some(window) = app.get_webview_window("main") {
                    setup_window_rounded_corners(&window);
                }
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
            let auto_start_enabled = app.autolaunch().is_enabled().unwrap_or(false);
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

            // 保存托盘菜单项引用，供 set_window_fixed_mode 同步勾选状态
            commands::set_tray_toggle_fixed_item(toggle_fixed.clone());

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
                            if currently_enabled {
                                let _ = autolaunch.disable();
                            } else {
                                let _ = autolaunch.enable();
                            }
                            // CheckMenuItem 会自动切换勾选状态
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
                        let now = SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .unwrap()
                            .as_millis() as u64;
                        let last_click = LAST_CLICK_TIME.swap(now, Ordering::SeqCst);

                        let app = tray.app_handle();

                        // 检测双击
                        if now - last_click < DOUBLE_CLICK_THRESHOLD_MS {
                            // 双击：打开添加待办窗口
                            if let Some(window) = app.get_webview_window("main") {
                                let _ = window.emit::<()>("tray-add-todo", ());
                            }
                            // 重置时间避免连续触发
                            LAST_CLICK_TIME.store(0, Ordering::SeqCst);
                        } else {
                            // 单击：显示/聚焦主窗口
                            if let Some(webview_window) = app.get_webview_window("main") {
                                let _ = webview_window.unminimize();
                                let _ = webview_window.show();
                                let _ = webview_window.set_focus();
                            }
                        }
                    }
                })
                .build(app)?;

            // 启动通知调度器
            NotificationService::start_scheduler(app.handle().clone());

            // 启动任务调度引擎
            let scheduler = app.state::<std::sync::Arc<TaskScheduler>>().inner().clone();
            scheduler.start(app.handle().clone());

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
            create_todo,
            update_todo,
            delete_todo,
            reorder_todos,
            // 子任务命令
            create_subtask,
            update_subtask,
            delete_subtask,
            // 图片命令
            get_images_dir,
            get_subtask,
            save_subtask_image,
            // 窗口设置命令
            get_settings,
            save_settings,
            set_window_fixed_mode,
            get_auto_hide_enabled,
            set_auto_hide_enabled,
            set_auto_hide_cursor_inside,
            set_macos_vibrancy,
            get_window_persist_state,
            reset_window,
            // 屏幕配置命令
            get_screen_config,
            save_screen_config,
            list_screen_configs,
            delete_screen_config,
            update_screen_config_name,
            // 日历设置命令
            get_show_calendar,
            set_show_calendar,
            // 数据导入导出命令
            export_data,
            import_data,
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
            webdav_upload_sync,
            webdav_download_sync,
            webdav_apply_remote,
            webdav_auto_sync,
            // Agent 命令
            get_agents,
            get_agent,
            create_agent,
            update_agent,
            delete_agent,
            check_agent_health,
            check_all_agents_health,
            auto_detect_agents,
            start_agent_execution,
            get_agent_execution_state,
            get_agent_execution_by_subtask,
            cancel_agent_execution,
            // 调度器命令
            update_subtask_schedule_status,
            update_subtask_priority,
            update_subtask_timeout,
            update_subtask_max_retries,
            add_task_dependency,
            remove_task_dependency,
            get_task_dependencies,
            check_dependencies_met,
            update_todo_schedule_config,
            start_scheduler,
            stop_scheduler,
            get_scheduler_status,
            submit_task_to_scheduler,
            validate_cron_expression,
            get_next_cron_execution,
            get_scheduled_todos,
            // 审核命令
            approve_review,
            reject_review,
            // 工作流命令
            get_workflow_steps,
            set_workflow_steps,
            start_workflow,
            pause_workflow,
            reset_workflow,
            skip_workflow_step,
            get_workflow_executions,
            // Prompt 模板命令
            get_prompt_templates,
            get_prompt_template,
            get_prompt_templates_by_category,
            create_prompt_template,
            update_prompt_template,
            delete_prompt_template,
            render_prompt_template,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|_app_handle, _event| {
            // 事件监听（保留空实现）
        });
}
