//! 开机自启：与旧版 `tauri-plugin-autostart`（底层 auto-launch 0.5）同一实现、同名同参数，
//! 沿用旧版写下的注册表项 `HKCU\Software\Microsoft\Windows\CurrentVersion\Run\mini-todo = "<exe>" --autostart`。
//!
//! 全部是阻塞调用（读写注册表 / 文件），放后台执行。开发专用数据隔离（`MINI_TODO_DATA_DIR`）时
//! **不要**调用 [`refresh_if_enabled`]：自启项与安装版共用，刷新会把它指向开发版 exe。

use auto_launch::{AutoLaunch, AutoLaunchBuilder};

/// 注册表值名（旧版 `package_info().name`）
pub const APP_NAME: &str = "mini-todo";
/// 开机自启拉起时带的参数：第二实例带它时静默退出
pub const AUTOSTART_ARG: &str = "--autostart";

fn launcher() -> Result<AutoLaunch, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    AutoLaunchBuilder::new()
        .set_app_name(APP_NAME)
        .set_app_path(&exe.display().to_string())
        .set_args(&[AUTOSTART_ARG])
        .build()
        .map_err(|e| e.to_string())
}

/// 是否已开启（注册表项存在）
pub fn is_enabled() -> Result<bool, String> {
    launcher()?.is_enabled().map_err(|e| e.to_string())
}

pub fn enable() -> Result<(), String> {
    launcher()?.enable().map_err(|e| e.to_string())
}

pub fn disable() -> Result<(), String> {
    launcher()?.disable().map_err(|e| e.to_string())
}

pub fn set_enabled(enabled: bool) -> Result<(), String> {
    if enabled { enable() } else { disable() }
}

/// 启动时的自愈：已开启时重新写一次，把注册表里的路径刷新为当前 exe（便携版被移动 / 换成安装版后
/// 旧记录仍在但指向失效路径）。返回是否开启。
pub fn refresh_if_enabled() -> bool {
    let enabled = is_enabled().unwrap_or(false);
    if enabled && let Err(e) = enable() {
        log::warn!("[autostart] 刷新开机自启项失败: {e}");
    }
    enabled
}
