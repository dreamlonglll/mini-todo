//! 窗口子类过程（`SetWindowSubclass`，comctl32）：补 GPUI 没有的两种运行时窗口行为。
//!
//! | 标志 | 拦截的消息 | 效果 |
//! |---|---|---|
//! | [`FLAG_BOTTOM`] | `WM_WINDOWPOSCHANGING` | 每次改 Z 序都改成 `HWND_BOTTOM`（替代 tao `always_on_bottom`；带 `SWP_NOZORDER` 的调用不受影响）。窗口以 Progman 为 owner 时，`HWND_BOTTOM` 实际落在"紧贴 Progman 之上" |
//! | [`FLAG_LOCK_GEOMETRY`] | `WM_NCHITTEST` / `WM_SYSCOMMAND` | 固定模式：边框缩放热区与标题栏拖动区一律不响应（客户区内回 `HTCLIENT`，客户区外的不可见边框回 `HTBORDER`）；拦掉键盘 / 系统菜单的移动、缩放、最大化 |
//!
//! GPUI 的缩放热区按**建窗时**的 `is_resizable` 计算、没有运行时开关，所以固定模式只能在 hit-test 这一层拦。
//! 不用 `GWLP_USERDATA`（GPUI 存窗口状态）；每个窗口的标志放在一个泄漏的 `AtomicU32` 里（经 `dwRefData` 传给
//! 子类过程，窗口销毁时从登记表移除），读写不需要加锁。
//!
//! `SetWindowSubclass` 必须在窗口所属线程（GPUI 主线程）调用；它本身不发窗口消息，可以在任何时候调用。

use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, OnceLock};

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::ScreenToClient;
use windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::{
    GetClientRect, HTBORDER, HTBOTTOM, HTBOTTOMLEFT, HTBOTTOMRIGHT, HTCAPTION, HTCLIENT, HTLEFT,
    HTMAXBUTTON, HTRIGHT, HTTOP, HTTOPLEFT, HTTOPRIGHT, HWND_BOTTOM, SC_MAXIMIZE, SC_MOVE, SC_SIZE,
    SWP_NOZORDER, WINDOWPOS, WM_NCDESTROY, WM_NCHITTEST, WM_SYSCOMMAND, WM_WINDOWPOSCHANGING,
};

use super::Hwnd;

/// 永远置底（嵌入桌面）
pub const FLAG_BOTTOM: u32 = 1;
/// 锁定位置与尺寸（固定模式）
pub const FLAG_LOCK_GEOMETRY: u32 = 2;

/// 子类 id（同一个窗口只装一次）
const SUBCLASS_ID: usize = 0x4D54_5743; // "MTWC"

fn registry() -> &'static Mutex<HashMap<isize, &'static AtomicU32>> {
    static REGISTRY: OnceLock<Mutex<HashMap<isize, &'static AtomicU32>>> = OnceLock::new();
    REGISTRY.get_or_init(Default::default)
}

fn to_hwnd(h: Hwnd) -> HWND {
    HWND(h.0 as *mut c_void)
}

/// 取窗口的标志单元；没有就装子类过程（只在窗口所属线程调用）
fn flags_of(h: Hwnd) -> Option<&'static AtomicU32> {
    let mut map = registry().lock().unwrap_or_else(|e| e.into_inner());
    if let Some(cell) = map.get(&h.0) {
        return Some(cell);
    }
    let cell: &'static AtomicU32 = Box::leak(Box::new(AtomicU32::new(0)));
    let ok = unsafe {
        SetWindowSubclass(
            to_hwnd(h),
            Some(subclass_proc),
            SUBCLASS_ID,
            cell as *const AtomicU32 as usize,
        )
    }
    .as_bool();
    if !ok {
        log::warn!("[platform] SetWindowSubclass({:#x}) 失败", h.0);
        return None;
    }
    map.insert(h.0, cell);
    Some(cell)
}

/// 打开 / 关闭某个标志
pub fn set_flag(h: Hwnd, flag: u32, on: bool) {
    if !on {
        // 没装过子类就等于没有任何标志，不必为了"关"而装
        let map = registry().lock().unwrap_or_else(|e| e.into_inner());
        if let Some(cell) = map.get(&h.0) {
            cell.fetch_and(!flag, Ordering::SeqCst);
        }
        return;
    }
    if let Some(cell) = flags_of(h) {
        cell.fetch_or(flag, Ordering::SeqCst);
    }
}

/// 命中测试结果是否是缩放 / 拖动类（固定模式要屏蔽的）
fn is_geometry_hit(code: u32) -> bool {
    matches!(
        code,
        HTCAPTION
            | HTLEFT
            | HTRIGHT
            | HTTOP
            | HTTOPLEFT
            | HTTOPRIGHT
            | HTBOTTOM
            | HTBOTTOMLEFT
            | HTBOTTOMRIGHT
            | HTMAXBUTTON
    )
}

unsafe extern "system" fn subclass_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    refdata: usize,
) -> LRESULT {
    // SAFETY: refdata 是 flags_of 泄漏出来的 &'static AtomicU32
    let flags = unsafe { &*(refdata as *const AtomicU32) }.load(Ordering::SeqCst);
    match msg {
        WM_WINDOWPOSCHANGING if flags & FLAG_BOTTOM != 0 => {
            // SAFETY: WM_WINDOWPOSCHANGING 的 lparam 是 WINDOWPOS*
            let pos = unsafe { &mut *(lparam.0 as *mut WINDOWPOS) };
            if pos.flags.0 & SWP_NOZORDER.0 == 0 {
                pos.hwndInsertAfter = HWND_BOTTOM;
            }
        }
        WM_NCHITTEST if flags & FLAG_LOCK_GEOMETRY != 0 => {
            let hit = unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) };
            let code = hit.0 as u32;
            if !is_geometry_hit(code) {
                return hit;
            }
            // 客户区内（GPUI 顶部的自绘缩放带 / 拖动区）交给界面；客户区外的不可见边框不响应
            let mut point = POINT {
                x: (lparam.0 & 0xFFFF) as i16 as i32,
                y: ((lparam.0 >> 16) & 0xFFFF) as i16 as i32,
            };
            let mut client = RECT::default();
            let inside = unsafe {
                ScreenToClient(hwnd, &mut point).as_bool()
                    && GetClientRect(hwnd, &mut client).is_ok()
            } && point.x >= client.left
                && point.x < client.right
                && point.y >= client.top
                && point.y < client.bottom;
            return LRESULT(if inside { HTCLIENT } else { HTBORDER } as isize);
        }
        WM_SYSCOMMAND if flags & FLAG_LOCK_GEOMETRY != 0 => {
            let command = (wparam.0 & 0xFFF0) as u32;
            if command == SC_MOVE || command == SC_SIZE || command == SC_MAXIMIZE {
                return LRESULT(0);
            }
        }
        WM_NCDESTROY => {
            registry()
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&(hwnd.0 as isize));
            unsafe {
                let _ = RemoveWindowSubclass(hwnd, Some(subclass_proc), SUBCLASS_ID);
            }
        }
        _ => {}
    }
    unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn geometry_hits_cover_resize_and_drag() {
        for code in [
            HTCAPTION,
            HTLEFT,
            HTRIGHT,
            HTTOP,
            HTBOTTOMRIGHT,
            HTMAXBUTTON,
        ] {
            assert!(is_geometry_hit(code));
        }
        for code in [HTCLIENT, HTBORDER, 0] {
            assert!(!is_geometry_hit(code));
        }
    }
}
