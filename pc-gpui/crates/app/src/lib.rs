//! Mini Todo（GPUI 版 PC 端）：窗口、界面与平台集成。二进制入口在 `main.rs`（只调用 [`run`]）。
//!
//! 模块都是 `pub` 的：基础框架提供的接口在各窗口实现之前还没有调用方，作为库导出不会被当成死代码。
//!
//! 启动顺序：日志 → 单实例 → 数据库（失败弹原生消息框后退出）→ `Application`（`QuitMode::Explicit`，托盘常驻）
//! → `gpui_kit::init` → 主题 → 浮层插件 → 全局状态 → 单实例唤起监听 → 主窗口 → 加载设置与待办
//! →（debug）`MINI_TODO_DEBUG_OPEN` 指定的窗口。
//!
//! 托盘、提醒调度、自动同步、200ms 窗口守护等后台服务由各窗口实现者接到 `services/` 与对应窗口里。

pub mod assets;
pub mod db;
pub mod platform;
pub mod services;
pub mod state;
pub mod theme;
pub mod ui;
pub mod util;
pub mod windows;

use std::sync::Arc;

use gpui_kit::QuitMode;
use mini_todo_core::Database;
use mini_todo_core::db::paths;

use crate::assets::AppAssets;
use crate::state::{APP_VERSION, AppState};

/// AppUserModelID（系统通知归属）：沿用旧版 Tauri identifier
pub const APP_IDENTIFIER: &str = "com.tauri-app.mini-todo";
pub const APP_NAME: &str = "Mini Todo";

/// 进程入口（`main.rs` 只调用它）
pub fn run() {
    services::logging::init();
    let Some(_instance) = services::single_instance::acquire() else {
        return;
    };
    if let Some(dir) = paths::dev_data_dir_override() {
        log::warn!(
            "[dev] 开发专用数据隔离已启用（{}）：数据目录 {}，不启用单实例、不刷新开机自启项",
            paths::DEV_DATA_DIR_ENV,
            dir.display()
        );
    }
    log::info!("[app] Mini Todo v{APP_VERSION} 启动");

    let database = Arc::new(open_database_or_exit());
    let debug_open = debug_open_items();

    gpui_kit::application()
        .with_assets(AppAssets)
        .with_quit_mode(QuitMode::Explicit)
        .run(move |cx| {
            gpui_kit::init(cx);
            cx.set_app_identity(APP_IDENTIFIER, APP_NAME);
            gpui_kit::component::set_locale("zh-CN");
            theme::init(cx);
            ui::layers::UiLayer::register(cx);
            db::Db::init(cx, database);
            windows::WindowManager::init(cx);
            let state = AppState::init(cx);

            services::single_instance::listen(cx, windows::main_window::show_and_activate);

            // 非 Windows 没有托盘：最后一个窗口关掉就退出
            #[cfg(not(target_os = "windows"))]
            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();

            windows::main_window::open(cx);

            // 各窗口实现者的启动钩子（现为空实现）
            services::tray::start(cx); // W1：托盘
            services::auto_sync::start(cx); // W1：自动同步定时器
            services::scheduler::start(cx); // W2：提醒调度
            windows::fence::restore_all(cx); // W3：恢复收纳箱窗口

            // 开机自启自愈：已开启时把注册表里的路径刷新为当前 exe（开发隔离时跳过，自启项与安装版共用）
            if paths::dev_data_dir_override().is_none() {
                cx.background_executor()
                    .spawn(async { services::autostart::refresh_if_enabled() })
                    .detach();
            }

            let load = state.update(cx, |state, cx| state.load_initial(cx));
            cx.spawn(async move |cx| {
                load.await;
                if !debug_open.is_empty() {
                    cx.update(|cx| windows::open_debug(cx, &debug_open));
                }
            })
            .detach();
        });
}

/// `MINI_TODO_DEBUG_OPEN`（仅 debug 构建读取）：启动后额外打开的窗口，格式见 `pc-gpui/scripts/README.md`
fn debug_open_items() -> Vec<windows::DebugOpen> {
    #[cfg(debug_assertions)]
    {
        std::env::var("MINI_TODO_DEBUG_OPEN")
            .map(|spec| windows::parse_debug_open(&spec))
            .unwrap_or_default()
    }
    #[cfg(not(debug_assertions))]
    {
        Vec::new()
    }
}

/// 打开数据库；失败时记日志、提示用户（Windows 弹框，其它平台写 stderr）后以退出码 1 结束
fn open_database_or_exit() -> Database {
    match Database::new() {
        Ok(database) => database,
        Err(e) => {
            let path = paths::db_path();
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
    use ::windows::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW};
    use ::windows::core::HSTRING;

    unsafe {
        MessageBoxW(
            None,
            &HSTRING::from(message),
            &HSTRING::from(APP_NAME),
            MB_OK | MB_ICONERROR,
        );
    }
}

#[cfg(not(target_os = "windows"))]
fn show_fatal_error(message: &str) {
    use std::io::Write as _;
    // 日志之外，终端启动时用户也要能看到
    let _ = writeln!(std::io::stderr(), "{message}");
}
