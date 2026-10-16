//! Linux（及其它非 Windows / macOS 平台；只用于 CI 与 Xvfb 截图）：用 GPUI 自带能力近似，做不到的是 no-op。
//! 缩放按 GPUI 的 `scale_factor` / 1.0（截图脚本设 `GPUI_X11_SCALE_FACTOR=1`）。macOS 有自己的实现（`macos/`）。

use gpui_kit::{App, Window};

use super::{Hwnd, MonitorInfo, PhysRect};

pub fn hwnd(_window: &Window) -> Option<Hwnd> {
    None
}

pub fn begin_window_drag(window: &Window, _cx: &App) {
    window.start_window_move();
}

pub fn toggle_maximize(window: &Window, _cx: &App) {
    window.zoom_window();
}

pub fn window_rect_physical(window: &Window) -> Option<PhysRect> {
    let bounds = window.bounds();
    let scale = window.scale_factor();
    Some(PhysRect::new(
        (f32::from(bounds.origin.x) * scale).round() as i32,
        (f32::from(bounds.origin.y) * scale).round() as i32,
        (f32::from(bounds.size.width) * scale).round() as i32,
        (f32::from(bounds.size.height) * scale).round() as i32,
    ))
}

/// GPUI 在 Linux 上没有移动窗口的 API：no-op（需要改尺寸时调用方自己 `window.resize(..)`）
pub fn set_client_rect_physical(_window: &Window, _cx: &App, _rect: PhysRect) {}

pub fn set_client_position_physical(_window: &Window, _cx: &App, _x: i32, _y: i32) {}

pub fn monitors(cx: &App) -> Vec<MonitorInfo> {
    let primary = cx.primary_display().map(|d| d.id());
    cx.displays()
        .iter()
        .map(|display| {
            let to_rect = |b: gpui_kit::Bounds<gpui_kit::Pixels>| {
                PhysRect::new(
                    f32::from(b.origin.x).round() as i32,
                    f32::from(b.origin.y).round() as i32,
                    f32::from(b.size.width).round() as i32,
                    f32::from(b.size.height).round() as i32,
                )
            };
            MonitorInfo {
                display_id: display.id(),
                rect: to_rect(display.bounds()),
                work: to_rect(display.visible_bounds()),
                scale: 1.0,
                primary: Some(display.id()) == primary,
            }
        })
        .collect()
}

pub fn monitor_of_window(window: &Window, cx: &App) -> Option<MonitorInfo> {
    let id = window.display(cx)?.id();
    monitors(cx).into_iter().find(|m| m.display_id == id)
}

pub fn cursor_position_physical() -> Option<(i32, i32)> {
    None
}

pub fn set_topmost(_window: &Window, _cx: &App, _topmost: bool) {}
pub fn show_window(_window: &Window, _cx: &App) {}
pub fn hide_window(_window: &Window, _cx: &App) {}
pub fn is_window_visible(_window: &Window) -> bool {
    true
}
pub fn set_tool_window(_window: &Window, _cx: &App, _tool: bool) {}
pub fn set_minimizable(_window: &Window, _cx: &App, _minimizable: bool) {}
pub fn set_resizable_frame(_window: &Window, _cx: &App, _resizable: bool) {}
pub fn set_round_corners(_window: &Window, _round: bool) {}
pub fn client_rect_physical(window: &Window) -> Option<PhysRect> {
    window_rect_physical(window)
}
pub fn set_geometry_locked(_window: &Window, _cx: &App, _locked: bool) {}
pub fn set_owner(_window: &Window, _cx: &App, _owner: Option<Hwnd>) {}
pub fn restore_if_minimized(_window: &Window, _cx: &App) {}

pub fn bring_to_front(window: &Window, _cx: &App) {
    window.activate_window();
}

/// 原始调用：其它平台没有 HWND，全部 no-op
pub mod raw {
    use super::super::{Hwnd, PhysRect};

    pub fn rect(_hwnd: Hwnd) -> Option<PhysRect> {
        None
    }
    pub fn set_rect(_hwnd: Hwnd, _rect: PhysRect) -> bool {
        false
    }
    pub fn set_position(_hwnd: Hwnd, _x: i32, _y: i32) -> bool {
        false
    }
    pub fn set_topmost(_hwnd: Hwnd, _topmost: bool) -> bool {
        false
    }
    pub fn show(_hwnd: Hwnd) -> bool {
        false
    }
    pub fn hide(_hwnd: Hwnd) -> bool {
        false
    }
    pub fn is_visible(_hwnd: Hwnd) -> bool {
        false
    }
    pub fn is_minimized(_hwnd: Hwnd) -> bool {
        false
    }
    pub fn restore(_hwnd: Hwnd) -> bool {
        false
    }
    pub fn is_alive(_hwnd: Hwnd) -> bool {
        false
    }
    pub fn ex_style(_hwnd: Hwnd) -> u32 {
        0
    }
    pub fn set_tool_window(_hwnd: Hwnd, _tool: bool) {}
    pub fn set_minimizable(_hwnd: Hwnd, _minimizable: bool) {}
    pub fn set_resizable_frame(_hwnd: Hwnd, _resizable: bool) {}
    pub fn set_system_menu(_hwnd: Hwnd, _on: bool) {}
    pub fn set_owner(_hwnd: Hwnd, _owner: Option<Hwnd>) {}
    pub fn owner(_hwnd: Hwnd) -> Option<Hwnd> {
        None
    }
    pub fn set_round_corners(_hwnd: Hwnd, _round: bool) {}
    pub fn class_name(_hwnd: Hwnd) -> Option<String> {
        None
    }
    pub fn client_rect(_hwnd: Hwnd) -> Option<PhysRect> {
        None
    }
    pub fn set_client_rect(_hwnd: Hwnd, _rect: PhysRect) -> bool {
        false
    }
    pub fn set_client_position(_hwnd: Hwnd, _x: i32, _y: i32) -> bool {
        false
    }
    pub fn set_geometry_locked(_hwnd: Hwnd, _locked: bool) {}
    pub fn set_foreground(_hwnd: Hwnd) -> bool {
        false
    }
    pub fn prepare_after_open(_hwnd: Hwnd) {}
}
