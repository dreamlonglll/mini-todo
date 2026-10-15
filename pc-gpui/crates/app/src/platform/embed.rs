//! 嵌入桌面层：owner = Progman + 永远置底（替代 tao `always_on_bottom`）+ `WS_EX_TOOLWINDOW`。
//! 主窗口"固定模式时嵌入桌面"与桌面收纳箱共用（旧版 `commands/window.rs` 的 `desktop_host` /
//! `desktop_attach_hwnd` / `desktop_detach` L1013-1106、`tick_embedded_windows` L1336）。
//!
//! **负责人：W1（主窗口）实现 Windows 版本；W3（收纳箱）只调用这些入口。** 目前是安全的空实现：
//! 返回"未嵌入"、不改窗口，保证调用方在 Linux / 未实现前都能编译运行。
//!
//! 实现要点（`.trellis/spec/backend/window-modes.md`、`desktop-fences.md` §3.1；GPUI 平台事实见
//! `research/backend-port.md` §5.3-5.4）：
//! - 置底：GPUI 不处理 `WM_WINDOWPOSCHANGING`，用 `SetWindowSubclass`（`Win32_UI_Shell`）挂子类过程，把
//!   `hwndInsertAfter` 改成 `HWND_BOTTOM`；**不要碰 `GWLP_USERDATA`**（GPUI 占用）
//! - owner：`SetWindowLongPtrW(GWLP_HWNDPARENT, GetShellWindow())`，并确认类名是 `Progman`
//!   （`mini_todo_core::window_logic::is_desktop_host_class`）
//! - 样式：`WS_EX_TOOLWINDOW`、去掉 `WS_MINIMIZEBOX`（[`super::Hwnd::set_tool_window`] /
//!   [`super::Hwnd::set_minimizable`]）
//! - Explorer 重启：200ms 轮询里 `mini_todo_core::window_logic::needs_reattach(..)` 为真时重挂
//! - 所有会发窗口消息的调用只能在 App 未被借用时执行（见 `platform` 模块文档）

use super::Hwnd;

/// 桌面宿主窗口（`GetShellWindow()`，类名应为 `Progman`）
pub fn desktop_host() -> Option<Hwnd> {
    None
}

/// 把窗口嵌入桌面层；返回是否成功（未实现时恒 false，调用方按"未嵌入"处理）
pub fn attach_to_desktop(hwnd: Hwnd) -> bool {
    log::debug!("[embed] attach_to_desktop({:#x}) 尚未实现（W1）", hwnd.0);
    false
}

/// 退出桌面层：清 owner、撤置底钩子、按 `tool_window` 恢复任务栏样式
pub fn detach_from_desktop(hwnd: Hwnd, tool_window: bool) {
    log::debug!(
        "[embed] detach_from_desktop({:#x}, tool_window = {tool_window}) 尚未实现（W1）",
        hwnd.0
    );
}

/// 永远置底（子类化 `WM_WINDOWPOSCHANGING`）开关
pub fn set_always_on_bottom(hwnd: Hwnd, on: bool) {
    log::debug!(
        "[embed] set_always_on_bottom({:#x}, {on}) 尚未实现（W1）",
        hwnd.0
    );
}

/// 已嵌入的窗口是否需要重挂 owner（Explorer 重启后 Progman 换了）
pub fn needs_reattach(_hwnd: Hwnd) -> bool {
    false
}
