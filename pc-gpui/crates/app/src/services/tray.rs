//! 系统托盘 / 菜单栏图标（Windows 与 macOS：`tray-icon` + 其自带的 muda 菜单；旧版 `lib.rs` L256-426）。
//!
//! 菜单（旧版顺序与文案）：添加待办项、分隔、新增收纳箱、显示/隐藏所有收纳箱、重置收纳箱位置、分隔、
//! 固定模式（勾选）、重置位置、打开设置、开机自启动（勾选）、分隔、退出。收纳箱三项（连同其后的分隔）只在 Windows 上有
//! （旧版 mac 托盘同样没有）。左键单击 300ms 去抖后唤起主窗口；右键弹菜单；开发隔离时 tooltip 为 `Mini Todo（开发版）`。
//! 图标：Windows 取 exe 资源 ID 1；macOS 用单色模板图标（`tray/mac_icon.rs`，随菜单栏深浅色自动着色）。
//!
//! 托盘 / 菜单事件在窗口过程（Windows）/ AppKit 事件（macOS，主线程）里触发：回调里**不碰** GPUI `App`，只把事件投进
//! channel，由前台任务处理（旧版"菜单回调里不能直接建窗"的约束同样适用）。"固定模式"勾选状态由窗口模式切换时显式回写
//! （[`set_fixed_checked`]），不依赖 muda 的自动翻转。macOS 上 tray-icon / muda 必须在主线程、run loop 已运行后创建：
//! [`start`] 在 `Application::run` 的回调里调用（GPUI 在 `applicationDidFinishLaunching:` 里执行它），满足要求。
//! Linux 不建托盘（只用于 CI 与截图）。

use gpui_kit::App;

#[cfg(target_os = "macos")]
mod mac_icon;

/// 启动托盘（`run()` 里调用；Linux 什么也不做）
pub fn start(cx: &mut App) {
    imp::start(cx)
}

/// 回写"固定模式"勾选（经典固定与嵌入桌面都显示为勾选）
pub fn set_fixed_checked(cx: &mut App, checked: bool) {
    imp::set_fixed_checked(cx, checked)
}

/// 回写"开机自启动"勾选（设置页切换开机自启后调用；旧 `sync_auto_start_state`）
pub fn set_auto_start_checked(cx: &mut App, checked: bool) {
    imp::set_auto_start_checked(cx, checked)
}

/// 托盘菜单项 id
pub mod menu_id {
    pub const ADD_TODO: &str = "add_todo";
    pub const NEW_FENCE: &str = "new_fence";
    pub const TOGGLE_FENCES: &str = "toggle_fences";
    pub const RESET_FENCES: &str = "reset_fences";
    pub const TOGGLE_FIXED: &str = "toggle_fixed";
    pub const RESET: &str = "reset";
    pub const OPEN_SETTINGS: &str = "open_settings";
    pub const AUTO_START: &str = "auto_start";
    pub const QUIT: &str = "quit";
}

/// 托盘提示文字（开发隔离时标出"开发版"）
pub fn tooltip_text() -> &'static str {
    if mini_todo_core::db::paths::dev_data_dir_override().is_some() {
        "Mini Todo（开发版）"
    } else {
        "Mini Todo"
    }
}

/// 处理一个菜单项（前台、App 未被窗口借用）
pub fn handle_menu(cx: &mut App, id: &str) {
    use crate::services::window_mode;
    use crate::state::AppState;
    use crate::windows::{ChildOf, WindowManager, editor, settings};

    log::info!("[tray] 菜单: {id}");
    match id {
        menu_id::ADD_TODO => {
            activate_app(cx);
            // 以主窗口为模态打开方，但不设 owner（独立顶层窗口）
            let parent = WindowManager::main(cx).map(|main| ChildOf::handle(main, cx).unowned());
            editor::open(cx, parent, None, editor::EditorMode::Edit);
        }
        menu_id::NEW_FENCE => fences::create(cx),
        menu_id::TOGGLE_FENCES => fences::toggle_visibility(cx),
        menu_id::RESET_FENCES => fences::reset_positions(cx),
        menu_id::TOGGLE_FIXED => {
            let state = AppState::global(cx);
            state.update(cx, |state, cx| state.toggle_fixed_mode(cx));
            // muda 点击时自己翻转了勾选；按真实状态回写（主窗口不存在时切换会失败）
            let fixed = state.read(cx).is_fixed;
            set_fixed_checked(cx, fixed);
        }
        menu_id::RESET => window_mode::reset_main_window(cx),
        menu_id::OPEN_SETTINGS => {
            activate_app(cx);
            window_mode::bring_main_to_front(cx);
            if let Some(main) = WindowManager::main(cx) {
                let parent = ChildOf::handle(main, cx);
                settings::open(cx, Some(parent), settings::SettingsTab::General);
            }
        }
        menu_id::AUTO_START => {
            let task = cx.background_executor().spawn(async move {
                use crate::services::autostart;
                let result = match autostart::is_enabled() {
                    Ok(true) => autostart::disable(),
                    Ok(false) => autostart::enable(),
                    Err(e) => Err(e),
                };
                if let Err(e) = result {
                    log::warn!("[tray] 切换开机自启动失败: {e}");
                }
                autostart::is_enabled().unwrap_or(false)
            });
            cx.spawn(async move |cx| {
                let enabled = task.await;
                cx.update(|cx| set_auto_start_checked(cx, enabled));
            })
            .detach();
        }
        menu_id::QUIT => {
            log::info!("[tray] 退出");
            cx.quit();
        }
        other => log::warn!("[tray] 未知菜单项: {other}"),
    }
}

/// 托盘左键单击（去抖后唤起主窗口）
pub fn handle_left_click(cx: &mut App) {
    use std::sync::Mutex;
    use std::time::Instant;
    static LAST_CLICK: Mutex<Option<Instant>> = Mutex::new(None);
    if mini_todo_core::window_logic::accept_tray_click(&LAST_CLICK, Instant::now()) {
        activate_app(cx);
        crate::services::window_mode::bring_main_to_front(cx);
    }
}

/// macOS：点菜单栏图标 / 菜单项不会激活本应用，新开或唤起的窗口会落在当前前台应用的窗口之后，先把应用切到前台
/// （`activateIgnoringOtherApps:`）。Windows 由窗口自己 `SetForegroundWindow`，什么也不做
fn activate_app(_cx: &mut App) {
    #[cfg(target_os = "macos")]
    _cx.activate(true);
}

/// 收纳箱三个菜单项的转发层（实现在 `windows::fence`）
mod fences {
    use gpui_kit::App;

    pub fn create(cx: &mut App) {
        crate::windows::fence::tray_create_fence(cx);
    }

    pub fn toggle_visibility(cx: &mut App) {
        crate::windows::fence::tray_toggle_fences(cx);
    }

    pub fn reset_positions(cx: &mut App) {
        crate::windows::fence::tray_reset_fences(cx);
    }
}

#[cfg(any(windows, target_os = "macos"))]
mod imp {
    use gpui_kit::{App, Global};
    use tray_icon::menu::{
        CheckMenuItem, IsMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem,
    };
    use tray_icon::{
        Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
    };

    use super::menu_id;

    /// 托盘与两个勾选项（主线程持有；drop 即移除托盘图标）
    struct Tray {
        _icon: TrayIcon,
        fixed: CheckMenuItem,
        auto_start: CheckMenuItem,
    }

    impl Global for Tray {}

    enum TrayMessage {
        Menu(String),
        LeftClick,
    }

    /// 托盘图标：Windows 取 exe 资源 ID 1（`build.rs` 嵌入的 icon.ico）
    #[cfg(windows)]
    fn tray_image() -> Result<Icon, String> {
        Icon::from_resource(1, None).map_err(|e| format!("读取 exe 图标失败: {e}"))
    }

    /// 菜单栏图标：macOS 用单色模板图标（构建托盘时再标成 template）
    #[cfg(target_os = "macos")]
    fn tray_image() -> Result<Icon, String> {
        super::mac_icon::menu_bar_icon()
    }

    pub fn start(cx: &mut App) {
        if cx.has_global::<Tray>() {
            return;
        }
        let auto_start_enabled = crate::services::autostart::is_enabled().unwrap_or(false);
        let fixed = CheckMenuItem::with_id(menu_id::TOGGLE_FIXED, "固定模式", true, false, None);
        let auto_start = CheckMenuItem::with_id(
            menu_id::AUTO_START,
            "开机自启动",
            true,
            auto_start_enabled,
            None,
        );
        let menu = Menu::new();
        let mut items: Vec<Box<dyn IsMenuItem>> = vec![
            Box::new(MenuItem::with_id(
                menu_id::ADD_TODO,
                "添加待办项",
                true,
                None,
            )),
            Box::new(PredefinedMenuItem::separator()),
        ];
        // 收纳箱三项只在 Windows 上有（桌面收纳箱是 Windows 专有功能；旧版 mac 托盘同样没有）
        #[cfg(windows)]
        {
            let fence_items: [Box<dyn IsMenuItem>; 4] = [
                Box::new(MenuItem::with_id(
                    menu_id::NEW_FENCE,
                    "新增收纳箱",
                    true,
                    None,
                )),
                Box::new(MenuItem::with_id(
                    menu_id::TOGGLE_FENCES,
                    "显示/隐藏所有收纳箱",
                    true,
                    None,
                )),
                Box::new(MenuItem::with_id(
                    menu_id::RESET_FENCES,
                    "重置收纳箱位置",
                    true,
                    None,
                )),
                Box::new(PredefinedMenuItem::separator()),
            ];
            items.extend(fence_items);
        }
        let rest: [Box<dyn IsMenuItem>; 6] = [
            Box::new(fixed.clone()),
            Box::new(MenuItem::with_id(menu_id::RESET, "重置位置", true, None)),
            Box::new(MenuItem::with_id(
                menu_id::OPEN_SETTINGS,
                "打开设置",
                true,
                None,
            )),
            Box::new(auto_start.clone()),
            Box::new(PredefinedMenuItem::separator()),
            Box::new(MenuItem::with_id(menu_id::QUIT, "退出", true, None)),
        ];
        items.extend(rest);
        for item in &items {
            if let Err(e) = menu.append(item.as_ref()) {
                log::warn!("[tray] 添加菜单项失败: {e}");
            }
        }

        let (tx, rx) = async_channel::unbounded::<TrayMessage>();
        let menu_tx = tx.clone();
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            let _ = menu_tx.try_send(TrayMessage::Menu(event.id.0));
        }));
        TrayIconEvent::set_event_handler(Some(move |event: TrayIconEvent| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                let _ = tx.try_send(TrayMessage::LeftClick);
            }
        }));

        let mut builder = TrayIconBuilder::new()
            .with_id("main")
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(false)
            .with_tooltip(super::tooltip_text());
        match tray_image() {
            Ok(icon) => builder = builder.with_icon(icon),
            Err(e) => log::warn!("[tray] {e}"),
        }
        // macOS：模板图标只取 alpha，系统按菜单栏深浅色着色
        #[cfg(target_os = "macos")]
        {
            builder = builder.with_icon_as_template(true);
        }
        let icon = match builder.build() {
            Ok(icon) => icon,
            Err(e) => {
                log::error!("[tray] 创建托盘失败: {e}");
                return;
            }
        };
        cx.set_global(Tray {
            _icon: icon,
            fixed,
            auto_start,
        });
        // 按真实状态初始化"固定模式"勾选（主窗口可能已按屏幕配置恢复为固定）
        let fixed_now = crate::services::window_mode::is_window_fixed(cx)
            || crate::state::AppState::global(cx).read(cx).is_fixed;
        set_fixed_checked(cx, fixed_now);

        cx.spawn(async move |cx| {
            while let Ok(message) = rx.recv().await {
                cx.update(|cx| match message {
                    TrayMessage::Menu(id) => super::handle_menu(cx, &id),
                    TrayMessage::LeftClick => super::handle_left_click(cx),
                });
            }
        })
        .detach();
        log::info!("[tray] 托盘已创建");
    }

    pub fn set_fixed_checked(cx: &mut App, checked: bool) {
        if let Some(tray) = cx.try_global::<Tray>() {
            tray.fixed.set_checked(checked);
        }
    }

    pub fn set_auto_start_checked(cx: &mut App, checked: bool) {
        if let Some(tray) = cx.try_global::<Tray>() {
            tray.auto_start.set_checked(checked);
        }
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
mod imp {
    use gpui_kit::App;

    pub fn start(_cx: &mut App) {}
    pub fn set_fixed_checked(_cx: &mut App, _checked: bool) {}
    pub fn set_auto_start_checked(_cx: &mut App, _checked: bool) {}
}

#[cfg(test)]
mod tests {
    /// macOS 菜单栏模板图标：36×36（18pt@2x），只有 alpha 有意义（RGB 全黑），且确有字形
    #[test]
    fn menu_bar_template_icon_is_black_alpha_glyph() {
        let bytes = include_bytes!("../../assets/tray-template.png");
        let image = image::load_from_memory_with_format(bytes, image::ImageFormat::Png)
            .expect("tray-template.png 解码失败")
            .into_rgba8();
        assert_eq!(image.dimensions(), (36, 36));
        assert!(image.pixels().all(|p| p[0] == 0 && p[1] == 0 && p[2] == 0));
        let opaque = image.pixels().filter(|p| p[3] > 128).count();
        assert!(opaque > 100, "字形太少: {opaque}");
        assert!(image.pixels().any(|p| p[3] == 0), "没有透明背景");
    }
}
