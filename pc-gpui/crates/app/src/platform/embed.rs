//! 嵌入桌面层：owner = Progman + 永远置底（替代 tao `always_on_bottom`）+ `WS_EX_TOOLWINDOW`。
//! 主窗口"固定模式时嵌入桌面"与桌面收纳箱共用（旧版 `commands/window.rs` 的 `desktop_host` /
//! `desktop_attach_hwnd` / `desktop_detach` L1013-1106、`tick_embedded_windows` L1336）。
//!
//! **负责人：W1（主窗口）实现；W3（收纳箱）只调用这些入口。**
//!
//! - 置底：GPUI 不处理 `WM_WINDOWPOSCHANGING`，用 `SetWindowSubclass` 挂子类过程（`platform/subclass.rs`），
//!   开启时把每次 `SetWindowPos` 的 `hwndInsertAfter` 改成 `HWND_BOTTOM`；owned 窗口总在 owner 之上，于是
//!   `HWND_BOTTOM` 实际落在"紧贴 Progman 之上"：桌面图标之上、所有应用窗口之下，Win+D 时随 Progman 一起被抬起
//! - owner：`SetWindowLongPtrW(GWLP_HWNDPARENT, GetShellWindow())`（不用 `SetParent`：子窗口化后 DPI 与圆角退化）；
//!   类名不是 `Progman`（Win10 / Win11 ≤ 23H2 / 第三方 shell）只记日志，仍然挂上
//! - 样式：`WS_EX_TOOLWINDOW`（不进任务栏 / Alt+Tab）、去掉 `WS_MINIMIZEBOX`（Win+D / Win+M 不最小化）
//! - Explorer 重启：200ms 轮询里 [`needs_reattach`] 为真时再调 [`attach_to_desktop`]（幂等）
//!
//! **调用时机**：这里的函数都是立即执行的 Win32 调用，会同步给窗口过程发消息——只能在 App 未被借用时调用
//! （`cx.spawn` 的 async 块里、`update` 闭包之外；见 `platform` 模块文档）。非 Windows 平台全部是安全的空实现
//! （返回"未嵌入"、不改窗口）。

use super::Hwnd;

/// 桌面宿主窗口（`GetShellWindow()`，类名应为 `Progman`）；Explorer 没运行时 `None`
pub fn desktop_host() -> Option<Hwnd> {
    imp::desktop_host()
}

/// 把窗口嵌入桌面层（幂等）：打开永远置底、owner = Progman、`WS_EX_TOOLWINDOW`、去掉 `WS_MINIMIZEBOX`、
/// 压到 `HWND_BOTTOM`。返回是否挂上了（Explorer 未运行 / 非 Windows 时 false，轮询稍后重试）
pub fn attach_to_desktop(hwnd: Hwnd) -> bool {
    imp::attach_to_desktop(hwnd)
}

/// 退出桌面层：关掉永远置底、清 owner、按 `tool_window` 恢复任务栏样式（true = 仍不进任务栏，经典固定模式用）、
/// 恢复嵌入前的 `WS_MINIMIZEBOX`、抬到 `HWND_TOP`
pub fn detach_from_desktop(hwnd: Hwnd, tool_window: bool) {
    imp::detach_from_desktop(hwnd, tool_window)
}

/// 永远置底（子类化 `WM_WINDOWPOSCHANGING`）开关；[`attach_to_desktop`] / [`detach_from_desktop`] 已包含
pub fn set_always_on_bottom(hwnd: Hwnd, on: bool) {
    imp::set_always_on_bottom(hwnd, on)
}

/// 已嵌入的窗口是否需要重挂 owner（Explorer 重启后 Progman 换了 / owner 被清 / 样式被抹掉）。
/// 从未挂上过的窗口也返回 true（只要 Explorer 在运行）；Explorer 没运行时 false（等下一次）
pub fn needs_reattach(hwnd: Hwnd) -> bool {
    imp::needs_reattach(hwnd)
}

#[cfg(windows)]
mod imp {
    use std::collections::HashMap;
    use std::ffi::c_void;
    use std::sync::atomic::{AtomicIsize, Ordering};
    use std::sync::{Mutex, OnceLock};

    use mini_todo_core::window_logic;
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{
        GWL_STYLE, GWLP_HWNDPARENT, GetShellWindow, GetWindowLongPtrW, HWND_BOTTOM, HWND_TOP,
        IsWindow, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SetWindowLongPtrW,
        SetWindowPos, WS_MINIMIZEBOX,
    };

    use super::super::subclass::{FLAG_BOTTOM, set_flag};
    use super::Hwnd;

    /// 已嵌入的窗口：挂上的宿主、嵌入前是否有最小化按钮
    #[derive(Clone, Copy)]
    struct Attached {
        owner: isize,
        had_minimize_box: bool,
    }

    fn attached() -> &'static Mutex<HashMap<isize, Attached>> {
        static ATTACHED: OnceLock<Mutex<HashMap<isize, Attached>>> = OnceLock::new();
        ATTACHED.get_or_init(Default::default)
    }

    fn to_hwnd(h: isize) -> HWND {
        HWND(h as *mut c_void)
    }

    /// 上一次记过日志的宿主（同一个宿主只记一次类名）
    static LOGGED_HOST: AtomicIsize = AtomicIsize::new(0);

    pub fn desktop_host() -> Option<Hwnd> {
        let host = unsafe { GetShellWindow() };
        if host.0.is_null() {
            return None;
        }
        let host = Hwnd(host.0 as isize);
        if LOGGED_HOST.swap(host.0, Ordering::SeqCst) != host.0 {
            let class = host.class_name().unwrap_or_default();
            if window_logic::is_desktop_host_class(&class) {
                log::info!("[embed] 桌面宿主 {:#x}（{class}）", host.0);
            } else {
                log::warn!(
                    "[embed] 桌面宿主 {:#x} 的类名是 {class:?} 而不是 Progman：仍以它为 owner，Win+D 可能会盖住嵌入的窗口",
                    host.0
                );
            }
        }
        Some(host)
    }

    pub fn attach_to_desktop(hwnd: Hwnd) -> bool {
        if !hwnd.is_alive() {
            return false;
        }
        let Some(host) = desktop_host() else {
            return false;
        };
        set_flag(hwnd, FLAG_BOTTOM, true);
        let h = to_hwnd(hwnd.0);
        let owner_changed = hwnd.owner().map_or(0, |o| o.0) != host.0;
        let style = unsafe { GetWindowLongPtrW(h, GWL_STYLE) } as u32;
        {
            let mut map = attached().lock().unwrap_or_else(|e| e.into_inner());
            let had_minimize_box = map
                .get(&hwnd.0)
                .map_or(style & WS_MINIMIZEBOX.0 != 0, |a| a.had_minimize_box);
            map.insert(
                hwnd.0,
                Attached {
                    owner: host.0,
                    had_minimize_box,
                },
            );
        }
        unsafe {
            if owner_changed {
                SetWindowLongPtrW(h, GWLP_HWNDPARENT, host.0);
            }
        }
        hwnd.set_minimizable(false);
        hwnd.set_tool_window(true);
        let mut flags = SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE;
        if owner_changed {
            flags |= SWP_FRAMECHANGED;
        }
        unsafe {
            let _ = SetWindowPos(h, Some(HWND_BOTTOM), 0, 0, 0, 0, flags);
        }
        log::info!(
            "[embed] 窗口 {:#x} 已嵌入桌面（owner {:#x}{}）",
            hwnd.0,
            host.0,
            if owner_changed { "，新挂上" } else { "" }
        );
        true
    }

    pub fn detach_from_desktop(hwnd: Hwnd, tool_window: bool) {
        // 先关置底钩子，否则下面的 HWND_TOP 会被改回 HWND_BOTTOM
        set_flag(hwnd, FLAG_BOTTOM, false);
        let record = attached()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&hwnd.0);
        if !hwnd.is_alive() {
            return;
        }
        let h = to_hwnd(hwnd.0);
        unsafe {
            SetWindowLongPtrW(h, GWLP_HWNDPARENT, 0);
        }
        hwnd.set_tool_window(tool_window);
        if record.is_none_or(|r| r.had_minimize_box) {
            hwnd.set_minimizable(true);
        }
        unsafe {
            let _ = SetWindowPos(
                h,
                Some(HWND_TOP),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_FRAMECHANGED,
            );
        }
        log::info!("[embed] 窗口 {:#x} 已退出桌面层", hwnd.0);
    }

    pub fn set_always_on_bottom(hwnd: Hwnd, on: bool) {
        set_flag(hwnd, FLAG_BOTTOM, on);
    }

    pub fn needs_reattach(hwnd: Hwnd) -> bool {
        if !hwnd.is_alive() {
            return false;
        }
        let cached = attached()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&hwnd.0)
            .map_or(0, |a| a.owner);
        let owner_alive = cached != 0 && unsafe { IsWindow(Some(to_hwnd(cached))) }.as_bool();
        let shell_now = unsafe { GetShellWindow() }.0 as isize;
        let current_owner = hwnd.owner().map_or(0, |o| o.0);
        window_logic::needs_reattach(cached, owner_alive, shell_now, current_owner)
            || (shell_now != 0 && window_logic::lacks_tool_window_style(hwnd.ex_style()))
    }
}

#[cfg(not(windows))]
mod imp {
    use super::Hwnd;

    pub fn desktop_host() -> Option<Hwnd> {
        None
    }

    pub fn attach_to_desktop(_hwnd: Hwnd) -> bool {
        false
    }

    pub fn detach_from_desktop(_hwnd: Hwnd, _tool_window: bool) {}

    pub fn set_always_on_bottom(_hwnd: Hwnd, _on: bool) {}

    pub fn needs_reattach(_hwnd: Hwnd) -> bool {
        false
    }
}
