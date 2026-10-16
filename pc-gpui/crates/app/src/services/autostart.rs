//! 开机自启：与旧版 `tauri-plugin-autostart`（底层 auto-launch 0.5）同一实现、同名同参数。
//!
//! - Windows：沿用旧版写下的注册表项 `HKCU\Software\Microsoft\Windows\CurrentVersion\Run\mini-todo = "<exe>" --autostart`
//! - macOS：与 2.5.0 mac 版（`MacosLauncher::LaunchAgent`）相同的 LaunchAgent `~/Library/LaunchAgents/mini-todo.plist`
//!   （`Label = mini-todo`、`ProgramArguments = [<可执行文件绝对路径>, --autostart]`、`RunAtLoad`）。路径取
//!   `current_exe().canonicalize()`，即 `/Applications/mini-todo.app/Contents/MacOS/mini-todo`（旧版 LaunchAgent 模式同样不截成 `.app`）。
//!   不用 auto-launch 默认的 AppleScript 登录项：那样每次查询都要 `osascript` 控制 System Events（弹自动化授权框）、
//!   丢掉 `--autostart`、也认不出旧版写的 plist。
//!   从隔离区直接运行（App Translocation，路径在 `…/AppTranslocation/…` 下的随机只读副本）时不写 plist：
//!   那个路径下次登录就不存在了
//! - Linux：auto-launch 的 XDG autostart（只用于 CI）
//!
//! 全部是阻塞调用（读写注册表 / 文件），放后台执行。开发专用数据隔离（`MINI_TODO_DATA_DIR`）时
//! **不要**调用 [`refresh_if_enabled`]：自启项与安装版共用，刷新会把它指向开发版 exe。
//!
//! 不依赖 GPUI：`pc-gpui/scripts/mac-check` 按 `aarch64-apple-darwin` 交叉检查本文件。

use auto_launch::{AutoLaunch, AutoLaunchBuilder};

/// 注册表值名 / LaunchAgent 文件名与 Label（旧版 `package_info().name`）
pub const APP_NAME: &str = "mini-todo";
/// 开机自启拉起时带的参数：第二实例带它时静默退出
pub const AUTOSTART_ARG: &str = "--autostart";

/// 自启项里写的程序路径
#[cfg(not(target_os = "macos"))]
fn launch_path() -> Result<String, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    Ok(exe.display().to_string())
}

/// 自启项里写的程序路径（macOS：解析符号链接后的绝对路径，与旧版插件一致）
#[cfg(target_os = "macos")]
fn launch_path() -> Result<String, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let exe = exe.canonicalize().unwrap_or(exe);
    Ok(exe.display().to_string())
}

/// macOS 的 App Translocation：从隔离区（下载后未移动、带隔离属性）直接打开的 `.app` 被系统复制到
/// `/private/var/folders/…/AppTranslocation/<UUID>/d/` 下的随机只读路径运行，这个路径重启后就失效
fn is_translocated(path: &str) -> bool {
    path.contains("/AppTranslocation/")
}

fn launcher() -> Result<AutoLaunch, String> {
    let path = launch_path()?;
    launcher_for(&path)
}

fn launcher_for(path: &str) -> Result<AutoLaunch, String> {
    let mut builder = AutoLaunchBuilder::new();
    builder
        .set_app_name(APP_NAME)
        .set_app_path(path)
        .set_args(&[AUTOSTART_ARG]);
    // LaunchAgent（与 2.5.0 同一个 plist），不用 AppleScript 登录项；这一项只在 macOS 上起作用
    #[cfg(target_os = "macos")]
    builder.set_use_launch_agent(true);
    builder.build().map_err(|e| e.to_string())
}

/// 是否已开启（Windows：注册表项存在；macOS：plist 存在）
pub fn is_enabled() -> Result<bool, String> {
    launcher()?.is_enabled().map_err(|e| e.to_string())
}

pub fn enable() -> Result<(), String> {
    let path = launch_path()?;
    if cfg!(target_os = "macos") && is_translocated(&path) {
        return Err(format!(
            "应用正从系统隔离的临时位置运行（{path}），请先把 mini-todo 拖进「应用程序」文件夹再开启开机自启"
        ));
    }
    launcher_for(&path)?.enable().map_err(|e| e.to_string())
}

pub fn disable() -> Result<(), String> {
    launcher()?.disable().map_err(|e| e.to_string())
}

pub fn set_enabled(enabled: bool) -> Result<(), String> {
    if enabled { enable() } else { disable() }
}

/// 启动时的自愈：已开启时重新写一次，把自启项里的路径刷新为当前程序（便携版被移动 / 换成安装版、
/// mac 上 `.app` 换了位置后旧记录仍在但指向失效路径）。返回是否开启。
/// macOS 从隔离区直接运行时不刷新（[`enable`] 拒绝写临时路径），保留原有记录。
pub fn refresh_if_enabled() -> bool {
    let enabled = is_enabled().unwrap_or(false);
    if enabled && let Err(e) = enable() {
        log::warn!("[autostart] 刷新开机自启项失败: {e}");
    }
    enabled
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_translocation_paths_are_detected() {
        assert!(is_translocated(
            "/private/var/folders/xy/abc123/T/AppTranslocation/3F2A-11/d/mini-todo.app/Contents/MacOS/mini-todo"
        ));
        assert!(!is_translocated(
            "/Applications/mini-todo.app/Contents/MacOS/mini-todo"
        ));
        assert!(!is_translocated(
            "C:\\Users\\me\\AppData\\Local\\mini-todo\\mini-todo.exe"
        ));
    }
}
