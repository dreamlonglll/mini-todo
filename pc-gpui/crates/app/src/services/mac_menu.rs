//! macOS 应用菜单（屏幕顶部菜单栏左侧的「Mini Todo / 编辑 / 窗口」）与 Dock 唤起。
//!
//! GPUI 不会自动建应用菜单：不调 `set_menus` 就没有 ⌘Q，系统文件面板（导入导出）里的 ⌘C / ⌘V 也不工作——AppKit 靠菜单项的
//! key equivalent 把这些键派给第一响应者（GPUI 的 app delegate 实现了 `cut:` / `copy:` / `paste:` / `selectAll:`）。
//!
//! | 菜单 | 项 | 快捷键 |
//! |---|---|---|
//! | Mini Todo（系统显示为 bundle 名） | 关于 Mini Todo（设置的「关于」页）、设置…、服务、隐藏 Mini Todo、隐藏其他、全部显示、退出 Mini Todo | ⌘, / ⌘H / ⌥⌘H / ⌘Q |
//! | 编辑 | 撤销、重做、剪切、拷贝、粘贴、全选 | 输入框自己的绑定（gpui-base `input::*`，⌘Z / ⇧⌘Z / ⌘X / ⌘C / ⌘V / ⌘A） |
//! | 窗口 | 最小化、关闭窗口 | ⌘M / ⌘W |
//!
//! - 编辑菜单的动作就是 gpui-component 输入框（gpui-base）的 `input::{Undo, Redo, Cut, Copy, Paste, SelectAll}`：焦点在输入框
//!   里时可用，否则 GPUI 自动置灰；快捷键沿用输入框在 macOS 上的绑定（context `Input`），这里不重复绑定。
//! - 其它动作是全局动作（[`App::on_action`]）。菜单动作在活动窗口的 `update` 里派发（窗口此刻被借用），所以碰窗口的处理
//!   一律 `cx.defer` 到这次派发之后。
//! - 关窗口（⌘W）不退出进程（`QuitMode::Explicit`，菜单栏图标常驻）；主窗口关掉后点 Dock 图标 / 在 Finder 里再次打开
//!   由 [`on_reopen`] 重新打开（GPUI 只在没有可见窗口时回调）。
//!
//! 模块全平台编译（Linux / Windows 的 clippy 也能检查这里对 GPUI API 的用法；本仓库的开发机都不是 mac），只在 macOS 上由
//! `lib.rs` 调用。

use gpui_kit::component::input;
use gpui_kit::{
    App, AppContext as _, KeyBinding, Menu, MenuItem, OsAction, SystemMenuType, Window, actions,
};

use crate::services::window_mode;
use crate::windows::settings::{self, SettingsTab};
use crate::windows::{ChildOf, WindowManager};

actions!(
    mini_todo_app,
    [
        About,
        OpenSettings,
        HideApp,
        HideOthers,
        ShowAll,
        Quit,
        Minimize,
        CloseWindow
    ]
);

/// 注册全局动作、快捷键与应用菜单（`gpui_kit::init` 之后调用一次；快捷键要先于 `set_menus` 绑定，菜单项才显示快捷键）
pub fn init(cx: &mut App) {
    cx.on_action(|_: &About, cx| cx.defer(|cx| open_settings(cx, SettingsTab::About)));
    cx.on_action(|_: &OpenSettings, cx| cx.defer(|cx| open_settings(cx, SettingsTab::General)));
    cx.on_action(|_: &HideApp, cx| cx.hide());
    cx.on_action(|_: &HideOthers, cx| cx.hide_other_apps());
    cx.on_action(|_: &ShowAll, cx| cx.unhide_other_apps());
    cx.on_action(|_: &Quit, cx| {
        log::info!("[app] 菜单退出");
        cx.quit();
    });
    cx.on_action(|_: &Minimize, cx| with_active_window(cx, |window, _| window.minimize_window()));
    cx.on_action(|_: &CloseWindow, cx| with_active_window(cx, close_window));
    cx.bind_keys([
        KeyBinding::new("cmd-,", OpenSettings, None),
        KeyBinding::new("cmd-h", HideApp, None),
        KeyBinding::new("alt-cmd-h", HideOthers, None),
        KeyBinding::new("cmd-q", Quit, None),
        KeyBinding::new("cmd-m", Minimize, None),
        KeyBinding::new("cmd-w", CloseWindow, None),
    ]);
    cx.set_menus(menus());
}

/// 应用菜单（文案按 macOS 简体中文的惯用写法）
pub fn menus() -> Vec<Menu> {
    let app = crate::APP_NAME;
    vec![
        Menu::new(app).items([
            MenuItem::action(format!("关于 {app}"), About),
            MenuItem::separator(),
            MenuItem::action("设置…", OpenSettings),
            MenuItem::separator(),
            MenuItem::os_submenu("服务", SystemMenuType::Services),
            MenuItem::separator(),
            MenuItem::action(format!("隐藏 {app}"), HideApp),
            MenuItem::action("隐藏其他", HideOthers),
            MenuItem::action("全部显示", ShowAll),
            MenuItem::separator(),
            MenuItem::action(format!("退出 {app}"), Quit),
        ]),
        Menu::new("编辑").items([
            MenuItem::os_action("撤销", input::Undo, OsAction::Undo),
            MenuItem::os_action("重做", input::Redo, OsAction::Redo),
            MenuItem::separator(),
            MenuItem::os_action("剪切", input::Cut, OsAction::Cut),
            MenuItem::os_action("拷贝", input::Copy, OsAction::Copy),
            MenuItem::os_action("粘贴", input::Paste, OsAction::Paste),
            MenuItem::os_action("全选", input::SelectAll, OsAction::SelectAll),
        ]),
        Menu::new("窗口").items([
            MenuItem::action("最小化", Minimize),
            MenuItem::action("关闭窗口", CloseWindow),
        ]),
    ]
}

/// Dock 图标被点 / 在 Finder 里再次打开（没有可见窗口时）：唤起主窗口，主窗口已关闭则重新打开
pub fn on_reopen(cx: &mut App) {
    log::info!("[app] Dock / Finder 唤起");
    window_mode::bring_main_to_front(cx);
}

/// 设置窗口（与托盘「打开设置」相同：先唤起主窗口，以它为模态打开方；主窗口不在就先打开）
fn open_settings(cx: &mut App, tab: SettingsTab) {
    window_mode::bring_main_to_front(cx);
    let parent = WindowManager::main(cx).map(|main| ChildOf::handle(main, cx));
    settings::open(cx, parent, tab);
}

/// 对当前活动窗口做点什么（延后到这次动作派发之后：派发期间活动窗口正被借用）
fn with_active_window(cx: &mut App, f: impl FnOnce(&mut Window, &mut App) + 'static) {
    let Some(handle) = cx.active_window() else {
        return;
    };
    cx.defer(move |cx| {
        let _ = cx.update_window(handle, |_, window, cx| f(window, cx));
    });
}

/// ⌘W：与点系统关闭按钮一样（主窗口先按关闭请求保存几何）；收纳箱不能关（Windows 专有，macOS 上不会出现）
fn close_window(window: &mut Window, cx: &mut App) {
    let id = window.window_handle().window_id();
    match WindowManager::kind_of(cx, id) {
        Some(crate::windows::WindowKind::Fence) => {}
        Some(crate::windows::WindowKind::Main) => {
            window_mode::on_main_close_requested(window, cx);
            window.remove_window();
        }
        _ => window.remove_window(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menus_have_app_edit_and_window() {
        let menus = menus();
        let names: Vec<&str> = menus.iter().map(|m| m.name.as_ref()).collect();
        assert_eq!(names, vec![crate::APP_NAME, "编辑", "窗口"]);
        // 编辑菜单的剪切 / 拷贝 / 粘贴 / 全选映射到系统选择器（文件面板里的 ⌘C / ⌘V 靠它）
        let os_actions = menus[1]
            .items
            .iter()
            .filter(|item| {
                matches!(
                    item,
                    MenuItem::Action {
                        os_action: Some(_),
                        ..
                    }
                )
            })
            .count();
        assert_eq!(os_actions, 6);
    }
}
