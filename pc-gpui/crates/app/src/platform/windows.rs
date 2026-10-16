//! Win32 实现（`windows` 0.62，与 gpui-pre-windows 同版本）。
//!
//! 写操作的 `(&Window, &App)` 版本经 [`defer`] 延后到下一个前台任务执行（App 未被借用时），
//! 原因见 `platform` 模块文档；`raw` 子模块是立即执行的原始调用。

use std::ffi::c_void;

use gpui_kit::{App, DisplayId, Window};
use windows::Win32::Foundation::{HWND, LPARAM, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND, DWMWCP_ROUND, DwmSetWindowAttribute,
};
use windows::Win32::Graphics::Gdi::{
    ClientToScreen, EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITOR_DEFAULTTONEAREST,
    MONITORINFO, MonitorFromWindow,
};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture;
use windows::Win32::UI::WindowsAndMessaging::{
    GWL_EXSTYLE, GWL_STYLE, GWLP_HWNDPARENT, GetClassNameW, GetClientRect, GetCursorPos,
    GetWindowLongPtrW, GetWindowRect, HTCAPTION, HWND_NOTOPMOST, HWND_TOPMOST, IsIconic, IsWindow,
    IsWindowVisible, IsZoomed, SW_HIDE, SW_MAXIMIZE, SW_RESTORE, SW_SHOWNOACTIVATE,
    SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SendMessageW,
    SetForegroundWindow, SetWindowLongPtrW, SetWindowPos, ShowWindow, WM_NCLBUTTONDOWN,
    WS_EX_APPWINDOW, WS_EX_TOOLWINDOW, WS_MAXIMIZEBOX, WS_MINIMIZEBOX, WS_SYSMENU, WS_THICKFRAME,
};

use super::{FrameInsets, Hwnd, MonitorInfo, PhysRect};

const MONITORINFOF_PRIMARY: u32 = 1;

fn to_hwnd(hwnd: Hwnd) -> HWND {
    HWND(hwnd.0 as *mut c_void)
}

pub fn hwnd(window: &Window) -> Option<Hwnd> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    // UFCS：Window 还有同名固有方法 window_handle() -> AnyWindowHandle
    match HasWindowHandle::window_handle(window).ok()?.as_raw() {
        RawWindowHandle::Win32(handle) => Some(Hwnd(handle.hwnd.get())),
        _ => None,
    }
}

/// 把会给窗口发消息的 Win32 调用延后到下一个前台任务（App 未被借用时）执行
fn defer(window: &Window, cx: &App, op: impl FnOnce(Hwnd) + 'static) {
    let Some(target) = hwnd(window) else {
        return;
    };
    cx.spawn(async move |_cx| op(target)).detach();
}

pub fn begin_window_drag(window: &Window, cx: &App) {
    defer(window, cx, |h| unsafe {
        let _ = ReleaseCapture();
        SendMessageW(
            to_hwnd(h),
            WM_NCLBUTTONDOWN,
            Some(WPARAM(HTCAPTION as usize)),
            Some(LPARAM(0)),
        );
    });
}

pub fn toggle_maximize(window: &Window, cx: &App) {
    defer(window, cx, |h| unsafe {
        let hwnd = to_hwnd(h);
        let cmd = if IsZoomed(hwnd).as_bool() {
            SW_RESTORE
        } else {
            SW_MAXIMIZE
        };
        let _ = ShowWindow(hwnd, cmd);
    });
}

pub fn window_rect_physical(window: &Window) -> Option<PhysRect> {
    raw::rect(hwnd(window)?)
}

pub fn set_client_rect_physical(window: &Window, cx: &App, rect: PhysRect) {
    defer(window, cx, move |h| {
        raw::set_client_rect(h, rect);
    });
}

pub fn set_client_position_physical(window: &Window, cx: &App, x: i32, y: i32) {
    defer(window, cx, move |h| {
        raw::set_client_position(h, x, y);
    });
}

unsafe extern "system" fn collect_monitor(
    monitor: HMONITOR,
    _hdc: HDC,
    _rect: *mut RECT,
    data: LPARAM,
) -> windows::core::BOOL {
    let list = unsafe { &mut *(data.0 as *mut Vec<MonitorInfo>) };
    if let Some(info) = monitor_info(monitor) {
        list.push(info);
    }
    true.into()
}

fn rect_of(r: RECT) -> PhysRect {
    PhysRect::new(r.left, r.top, r.right - r.left, r.bottom - r.top)
}

fn monitor_info(monitor: HMONITOR) -> Option<MonitorInfo> {
    let mut info = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    if !unsafe { GetMonitorInfoW(monitor, &mut info) }.as_bool() {
        return None;
    }
    let (mut dpi_x, mut dpi_y) = (96u32, 96u32);
    let scale = if unsafe { GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) }
        .is_ok()
        && dpi_x > 0
    {
        f64::from(dpi_x) / 96.0
    } else {
        1.0
    };
    let rect = rect_of(info.rcMonitor);
    let work = if info.rcWork.right > info.rcWork.left {
        rect_of(info.rcWork)
    } else {
        rect
    };
    Some(MonitorInfo {
        display_id: DisplayId::new(monitor.0 as u64),
        rect,
        work,
        scale,
        primary: info.dwFlags & MONITORINFOF_PRIMARY != 0,
    })
}

pub fn monitors(_cx: &App) -> Vec<MonitorInfo> {
    let mut list: Vec<MonitorInfo> = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(
            None,
            None,
            Some(collect_monitor),
            LPARAM(&mut list as *mut Vec<MonitorInfo> as isize),
        );
    }
    list
}

pub fn monitor_of_window(window: &Window, _cx: &App) -> Option<MonitorInfo> {
    let h = hwnd(window)?;
    let monitor = unsafe { MonitorFromWindow(to_hwnd(h), MONITOR_DEFAULTTONEAREST) };
    if monitor.is_invalid() {
        return None;
    }
    monitor_info(monitor)
}

pub fn cursor_position_physical() -> Option<(i32, i32)> {
    let mut point = POINT::default();
    unsafe { GetCursorPos(&mut point) }.ok()?;
    Some((point.x, point.y))
}

pub fn set_topmost(window: &Window, cx: &App, topmost: bool) {
    defer(window, cx, move |h| {
        raw::set_topmost(h, topmost);
    });
}

pub fn show_window(window: &Window, cx: &App) {
    defer(window, cx, |h| {
        raw::show(h);
    });
}

pub fn hide_window(window: &Window, cx: &App) {
    defer(window, cx, |h| {
        raw::hide(h);
    });
}

pub fn is_window_visible(window: &Window) -> bool {
    hwnd(window).is_some_and(raw::is_visible)
}

pub fn set_tool_window(window: &Window, cx: &App, tool: bool) {
    defer(window, cx, move |h| raw::set_tool_window(h, tool));
}

pub fn set_minimizable(window: &Window, cx: &App, minimizable: bool) {
    defer(window, cx, move |h| raw::set_minimizable(h, minimizable));
}

pub fn set_resizable_frame(window: &Window, cx: &App, resizable: bool) {
    defer(window, cx, move |h| raw::set_resizable_frame(h, resizable));
}

pub fn client_rect_physical(window: &Window) -> Option<PhysRect> {
    raw::client_rect(hwnd(window)?)
}

pub fn set_geometry_locked(window: &Window, cx: &App, locked: bool) {
    defer(window, cx, move |h| raw::set_geometry_locked(h, locked));
}

pub fn set_round_corners(window: &Window, round: bool) {
    if let Some(h) = hwnd(window) {
        raw::set_round_corners(h, round);
    }
}

pub fn set_owner(window: &Window, cx: &App, owner: Option<Hwnd>) {
    defer(window, cx, move |h| raw::set_owner(h, owner));
}

pub fn restore_if_minimized(window: &Window, cx: &App) {
    defer(window, cx, |h| {
        if raw::is_minimized(h) {
            raw::restore(h);
        }
    });
}

pub fn bring_to_front(window: &Window, cx: &App) {
    defer(window, cx, |h| {
        if !raw::is_visible(h) {
            raw::show(h);
        }
        if raw::is_minimized(h) {
            raw::restore(h);
        }
        unsafe {
            let _ = SetForegroundWindow(to_hwnd(h));
        }
    });
    window.activate_window();
}

/// 立即执行的原始调用（只在 App 未被借用时用）
pub mod raw {
    use super::*;

    pub fn rect(h: Hwnd) -> Option<PhysRect> {
        let mut r = RECT::default();
        unsafe { GetWindowRect(to_hwnd(h), &mut r) }.ok()?;
        Some(rect_of(r))
    }

    pub fn set_rect(h: Hwnd, rect: PhysRect) -> bool {
        unsafe {
            SetWindowPos(
                to_hwnd(h),
                None,
                rect.x,
                rect.y,
                rect.width,
                rect.height,
                SWP_NOZORDER | SWP_NOACTIVATE,
            )
        }
        .map_err(|e| log::warn!("[platform] SetWindowPos 失败: {e}"))
        .is_ok()
    }

    pub fn set_position(h: Hwnd, x: i32, y: i32) -> bool {
        unsafe {
            SetWindowPos(
                to_hwnd(h),
                None,
                x,
                y,
                0,
                0,
                SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOSIZE,
            )
        }
        .is_ok()
    }

    pub fn set_topmost(h: Hwnd, topmost: bool) -> bool {
        let after = if topmost {
            HWND_TOPMOST
        } else {
            HWND_NOTOPMOST
        };
        unsafe {
            SetWindowPos(
                to_hwnd(h),
                Some(after),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            )
        }
        .is_ok()
    }

    pub fn show(h: Hwnd) -> bool {
        unsafe {
            let _ = ShowWindow(to_hwnd(h), SW_SHOWNOACTIVATE);
        }
        true
    }

    pub fn hide(h: Hwnd) -> bool {
        unsafe {
            let _ = ShowWindow(to_hwnd(h), SW_HIDE);
        }
        true
    }

    pub fn is_visible(h: Hwnd) -> bool {
        unsafe { IsWindowVisible(to_hwnd(h)) }.as_bool()
    }

    pub fn is_minimized(h: Hwnd) -> bool {
        unsafe { IsIconic(to_hwnd(h)) }.as_bool()
    }

    pub fn restore(h: Hwnd) -> bool {
        unsafe {
            let _ = ShowWindow(to_hwnd(h), SW_RESTORE);
        }
        true
    }

    pub fn is_alive(h: Hwnd) -> bool {
        unsafe { IsWindow(Some(to_hwnd(h))) }.as_bool()
    }

    pub fn ex_style(h: Hwnd) -> u32 {
        unsafe { GetWindowLongPtrW(to_hwnd(h), GWL_EXSTYLE) as u32 }
    }

    fn update_long(
        h: Hwnd,
        index: windows::Win32::UI::WindowsAndMessaging::WINDOW_LONG_PTR_INDEX,
        set: u32,
        clear: u32,
    ) {
        let hwnd = to_hwnd(h);
        unsafe {
            let old = GetWindowLongPtrW(hwnd, index) as u32;
            let new = (old & !clear) | set;
            if new != old {
                SetWindowLongPtrW(hwnd, index, new as i32 as isize);
                // 样式变化后要 SWP_FRAMECHANGED 才生效
                let _ = SetWindowPos(
                    hwnd,
                    None,
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
                );
            }
        }
    }

    pub fn set_tool_window(h: Hwnd, tool: bool) {
        if tool {
            update_long(h, GWL_EXSTYLE, WS_EX_TOOLWINDOW.0, WS_EX_APPWINDOW.0);
        } else {
            update_long(h, GWL_EXSTYLE, WS_EX_APPWINDOW.0, WS_EX_TOOLWINDOW.0);
        }
    }

    pub fn set_minimizable(h: Hwnd, minimizable: bool) {
        if minimizable {
            update_long(h, GWL_STYLE, WS_MINIMIZEBOX.0, 0);
        } else {
            update_long(h, GWL_STYLE, 0, WS_MINIMIZEBOX.0);
        }
    }

    pub fn set_resizable_frame(h: Hwnd, resizable: bool) {
        let bits = WS_THICKFRAME.0 | WS_MAXIMIZEBOX.0;
        if resizable {
            update_long(h, GWL_STYLE, bits, 0);
        } else {
            update_long(h, GWL_STYLE, 0, bits);
        }
    }

    pub fn set_system_menu(h: Hwnd, on: bool) {
        if on {
            update_long(h, GWL_STYLE, WS_SYSMENU.0, 0);
        } else {
            update_long(h, GWL_STYLE, 0, WS_SYSMENU.0);
        }
    }

    pub fn set_owner(h: Hwnd, owner: Option<Hwnd>) {
        unsafe {
            SetWindowLongPtrW(to_hwnd(h), GWLP_HWNDPARENT, owner.map_or(0, |o| o.0));
        }
        // 有 owner 时不单独占任务栏按钮（WS_EX_APPWINDOW 会强制进任务栏）
        if owner.is_some() {
            update_long(h, GWL_EXSTYLE, 0, WS_EX_APPWINDOW.0);
        } else if ex_style(h) & WS_EX_TOOLWINDOW.0 == 0 {
            update_long(h, GWL_EXSTYLE, WS_EX_APPWINDOW.0, 0);
        }
    }

    pub fn owner(h: Hwnd) -> Option<Hwnd> {
        let value = unsafe { GetWindowLongPtrW(to_hwnd(h), GWLP_HWNDPARENT) };
        (value != 0).then_some(Hwnd(value))
    }

    pub fn set_round_corners(h: Hwnd, round: bool) {
        let preference = if round {
            DWMWCP_ROUND
        } else {
            DWMWCP_DONOTROUND
        };
        unsafe {
            let _ = DwmSetWindowAttribute(
                to_hwnd(h),
                DWMWA_WINDOW_CORNER_PREFERENCE,
                &preference as *const _ as *const c_void,
                std::mem::size_of_val(&preference) as u32,
            );
        }
    }

    pub fn class_name(h: Hwnd) -> Option<String> {
        let mut buf = [0u16; 256];
        let len = unsafe { GetClassNameW(to_hwnd(h), &mut buf) };
        (len > 0).then(|| String::from_utf16_lossy(&buf[..len as usize]))
    }

    pub fn client_rect(h: Hwnd) -> Option<PhysRect> {
        let hwnd = to_hwnd(h);
        let mut client = RECT::default();
        let mut origin = POINT::default();
        unsafe {
            GetClientRect(hwnd, &mut client).ok()?;
            if !ClientToScreen(hwnd, &mut origin).as_bool() {
                return None;
            }
        }
        Some(PhysRect::new(
            origin.x,
            origin.y,
            client.right - client.left,
            client.bottom - client.top,
        ))
    }

    /// 跨 DPI 移动时 GPUI 的 `WM_DPICHANGED` 会把窗口改成系统建议的外框，边框厚度也随 DPI 变：按新边框再校正
    const CLIENT_PLACE_ATTEMPTS: usize = 3;

    /// 当前的外框、客户区与两者之间的边框
    fn frame(h: Hwnd) -> Option<(PhysRect, FrameInsets)> {
        let outer = rect(h)?;
        let client = client_rect(h)?;
        Some((client, FrameInsets::between(outer, client)))
    }

    pub fn set_client_rect(h: Hwnd, target: PhysRect) -> bool {
        if is_minimized(h) {
            // 最小化时客户区是 0×0，算不出边框；只按外框摆一次（与还原后的位置无关）
            return set_rect(h, target);
        }
        for _ in 0..CLIENT_PLACE_ATTEMPTS {
            let Some((client, insets)) = frame(h) else {
                return set_rect(h, target);
            };
            if client == target {
                return true;
            }
            if !set_rect(h, insets.outer_for(target)) {
                return false;
            }
        }
        if let Some((client, _)) = frame(h)
            && client != target
        {
            log::debug!(
                "[platform] 客户区没能摆到 {target:?}（{:#x} 实际 {client:?}）",
                h.0
            );
        }
        true
    }

    pub fn set_client_position(h: Hwnd, x: i32, y: i32) -> bool {
        if is_minimized(h) {
            return set_position(h, x, y);
        }
        for _ in 0..CLIENT_PLACE_ATTEMPTS {
            let Some((client, insets)) = frame(h) else {
                return set_position(h, x, y);
            };
            if (client.x, client.y) == (x, y) {
                return true;
            }
            let outer = insets.outer_for(PhysRect::new(x, y, client.width, client.height));
            if !set_position(h, outer.x, outer.y) {
                return false;
            }
        }
        true
    }

    pub fn set_geometry_locked(h: Hwnd, locked: bool) {
        super::super::subclass::set_flag(h, super::super::subclass::FLAG_LOCK_GEOMETRY, locked);
    }

    pub fn set_foreground(h: Hwnd) -> bool {
        unsafe { SetForegroundWindow(to_hwnd(h)) }.as_bool()
    }
}
