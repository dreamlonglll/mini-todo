//! macOS 实现（gpui-pre-macos 0.3.9 没有运行时摆窗 / 置顶层级 / 显示隐藏 / 改 style mask 的接口）：
//! AppKit 调用在 [`ns`]（不依赖 GPUI，可在 `scripts/mac-check` 里交叉检查），坐标换算在 `super::mac_geometry`
//! （全平台编译、有单测），这里只是把两者接到 `platform` 的跨平台签名上，尽量薄。
//!
//! - [`Hwnd`] 在 macOS 上是 NSWindow 的**窗口号**（`windowNumber`），不是对象指针：见 [`ns`] 模块文档；
//! - 写操作的 `(&Window, &App)` 版本经 [`defer`] 延后到下一个前台任务（`setFrame:` 等会同步触发 GPUI 的
//!   `windowDidMove:` / `windowDidResize:` 回调，App 被借用时 GPUI 处理不了），与 `windows.rs` 相同；`raw` 是立即执行的原始调用；
//! - 物理像素口径（统一系数）见 `mac_geometry` 模块文档。

pub mod ns;

use gpui_kit::{App, DisplayId, Window};

use super::mac_geometry::{CocoaRect, CocoaScreen, ScreenLayout};
use super::{Hwnd, MonitorInfo, PhysRect};

fn cocoa(r: ns::Rect) -> CocoaRect {
    CocoaRect {
        x: r.x,
        y: r.y,
        width: r.width,
        height: r.height,
    }
}

fn ns_rect(r: CocoaRect) -> ns::Rect {
    ns::Rect {
        x: r.x,
        y: r.y,
        width: r.width,
        height: r.height,
    }
}

/// 当前屏幕布局（每次现读：插拔显示器、改排列后立即生效）
fn layout() -> ScreenLayout {
    ScreenLayout::new(
        ns::screens()
            .into_iter()
            .map(|s| CocoaScreen {
                display_id: s.display_id,
                frame: cocoa(s.frame),
                visible: cocoa(s.visible),
                scale: s.scale,
            })
            .collect(),
    )
}

pub fn hwnd(window: &Window) -> Option<Hwnd> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    // UFCS：Window 还有同名固有方法 window_handle() -> AnyWindowHandle
    match HasWindowHandle::window_handle(window).ok()?.as_raw() {
        // SAFETY: ns_view 是这个 GPUI 窗口的内容视图，`window` 借用期间一直存活
        RawWindowHandle::AppKit(handle) => {
            unsafe { ns::window_number_of_view(handle.ns_view.as_ptr()) }.map(Hwnd)
        }
        _ => None,
    }
}

/// 把会同步触发 GPUI 窗口回调的 AppKit 调用延后到下一个前台任务（App 未被借用时）执行
fn defer(window: &Window, cx: &App, op: impl FnOnce(Hwnd) + 'static) {
    let Some(target) = hwnd(window) else {
        return;
    };
    cx.spawn(async move |_cx| op(target)).detach();
}

/// `performWindowDragWithEvent:`（GPUI 用当前事件；必须在鼠标按下 / 移动的事件处理里同步调用）
pub fn begin_window_drag(window: &Window, _cx: &App) {
    window.start_window_move();
}

pub fn toggle_maximize(window: &Window, _cx: &App) {
    window.zoom_window();
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

pub fn monitors(_cx: &App) -> Vec<MonitorInfo> {
    layout().monitors()
}

pub fn monitor_of_window(window: &Window, cx: &App) -> Option<MonitorInfo> {
    let id = hwnd(window)
        .and_then(|h| ns::window_display_id(h.0))
        .map(|id| DisplayId::new(u64::from(id)))
        .or_else(|| window.display(cx).map(|d| d.id()))?;
    monitors(cx).into_iter().find(|m| m.display_id == id)
}

pub fn cursor_position_physical() -> Option<(i32, i32)> {
    let (x, y) = ns::mouse_location()?;
    Some(layout().point_to_physical(x, y))
}

/// 统一系数（`mac_geometry::ScreenLayout::unit`）：逻辑像素（点）× 它 = 本 crate 的物理像素
pub fn physical_scale() -> f64 {
    layout().unit()
}

/// 屏幕组合 ID 的输入（各屏自己的缩放，与 2.5.0 一致）
pub fn screen_config_inputs() -> Vec<(u32, u32, f64)> {
    layout().screen_config_inputs()
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

/// macOS 没有逐窗口的"不进任务栏 / Alt+Tab"：程序坞图标与 ⌘Tab 是整个应用的（激活策略），no-op
pub fn set_tool_window(_window: &Window, _cx: &App, _tool: bool) {}

pub fn set_minimizable(window: &Window, cx: &App, minimizable: bool) {
    defer(window, cx, move |h| raw::set_minimizable(h, minimizable));
}

pub fn set_resizable_frame(window: &Window, cx: &App, resizable: bool) {
    defer(window, cx, move |h| raw::set_resizable_frame(h, resizable));
}

/// 带标题栏样式的窗口圆角由系统处理、透明窗口的圆角界面自己画：no-op
pub fn set_round_corners(_window: &Window, _round: bool) {}

pub fn client_rect_physical(window: &Window) -> Option<PhysRect> {
    window_rect_physical(window)
}

pub fn set_geometry_locked(window: &Window, cx: &App, locked: bool) {
    defer(window, cx, move |h| raw::set_geometry_locked(h, locked));
}

/// macOS 的子窗口（`addChildWindow:`）会跟着父窗口移动，语义与 Windows owner 不同：no-op（模态子窗口靠遮罩点击带到前台）
pub fn set_owner(_window: &Window, _cx: &App, _owner: Option<Hwnd>) {}

pub fn restore_if_minimized(window: &Window, cx: &App) {
    defer(window, cx, |h| {
        if raw::is_minimized(h) {
            raw::restore(h);
        }
    });
}

/// 还原（延后）+ 把应用切到前台 + 成为 key 窗口
pub fn bring_to_front(window: &Window, cx: &App) {
    defer(window, cx, |h| {
        if raw::is_minimized(h) {
            raw::restore(h);
        }
    });
    cx.activate(true);
    window.activate_window();
}

/// 立即执行的原始调用（只在 App 未被借用时用）
pub mod raw {
    use super::*;

    /// frame（GPUI 窗口带 `FullSizeContentView`：外框 = 客户区）
    pub fn rect(h: Hwnd) -> Option<PhysRect> {
        let frame = ns::window_frame(h.0)?;
        Some(layout().to_physical(cocoa(frame)))
    }

    pub fn set_rect(h: Hwnd, rect: PhysRect) -> bool {
        set_client_rect(h, rect)
    }

    pub fn set_position(h: Hwnd, x: i32, y: i32) -> bool {
        set_client_position(h, x, y)
    }

    /// `NSFloatingWindowLevel` ⇄ `NSNormalWindowLevel`
    pub fn set_topmost(h: Hwnd, topmost: bool) -> bool {
        ns::set_floating(h.0, topmost)
    }

    /// `orderFrontRegardless`（不改 key 窗口）
    pub fn show(h: Hwnd) -> bool {
        ns::order_front(h.0)
    }

    pub fn hide(h: Hwnd) -> bool {
        ns::order_out(h.0)
    }

    pub fn is_visible(h: Hwnd) -> bool {
        ns::is_visible(h.0)
    }

    pub fn is_minimized(h: Hwnd) -> bool {
        ns::is_miniaturized(h.0)
    }

    pub fn restore(h: Hwnd) -> bool {
        ns::deminiaturize(h.0)
    }

    pub fn is_alive(h: Hwnd) -> bool {
        ns::is_alive(h.0)
    }

    /// 没有 Win32 ex style：恒 0
    pub fn ex_style(_h: Hwnd) -> u32 {
        0
    }

    pub fn set_tool_window(_h: Hwnd, _tool: bool) {}

    pub fn set_minimizable(h: Hwnd, minimizable: bool) {
        ns::set_miniaturizable(h.0, minimizable);
    }

    pub fn set_resizable_frame(h: Hwnd, resizable: bool) {
        ns::set_resizable(h.0, resizable);
    }

    /// Windows 的 `WS_SYSMENU`；macOS 的系统窗口按钮（红绿灯）建窗后由 [`prepare_after_open`] 隐藏：no-op
    pub fn set_system_menu(_h: Hwnd, _on: bool) {}

    pub fn set_owner(_h: Hwnd, _owner: Option<Hwnd>) {}

    pub fn owner(_h: Hwnd) -> Option<Hwnd> {
        None
    }

    pub fn set_round_corners(_h: Hwnd, _round: bool) {}

    pub fn class_name(_h: Hwnd) -> Option<String> {
        None
    }

    pub fn client_rect(h: Hwnd) -> Option<PhysRect> {
        rect(h)
    }

    /// 已经是目标就不动（`setFrame:` 会触发 GPUI 回调与保存）
    pub fn set_client_rect(h: Hwnd, target: PhysRect) -> bool {
        let layout = layout();
        let Some(frame) = ns::window_frame(h.0) else {
            return false;
        };
        if layout.to_physical(cocoa(frame)) == target {
            return true;
        }
        ns::set_frame(h.0, ns_rect(layout.to_cocoa(target)))
    }

    /// 只挪左上角（按窗口当前的点高度换算 AppKit 的左下角原点）。
    ///
    /// 返回窗口是否真的到了目标位置：AppKit 会把带标题栏样式的窗口约束在屏幕内（顶边不能高过菜单栏），贴边隐藏往屏幕
    /// 上方藏时可能被挡回来；返回 false 时 `window_mode` 的贴边状态机会回滚成显示态
    pub fn set_client_position(h: Hwnd, x: i32, y: i32) -> bool {
        let layout = layout();
        let Some(frame) = ns::window_frame(h.0) else {
            return false;
        };
        let current = layout.to_physical(cocoa(frame));
        if (current.x, current.y) == (x, y) {
            return true;
        }
        let (ox, oy) = layout.cocoa_origin_for(x, y, frame.height);
        if !ns::set_origin(h.0, ox, oy) {
            return false;
        }
        // AppKit 会把原点对齐到所在屏的像素：统一系数大于该屏缩放时差 1 个"物理像素"以内都算到位
        let tolerance = layout.unit().ceil() as i32;
        ns::window_frame(h.0)
            .map(|f| layout.to_physical(cocoa(f)))
            .is_some_and(|r| (r.x - x).abs() <= tolerance && (r.y - y).abs() <= tolerance)
    }

    /// `setMovable:NO` + 去掉可缩放（解锁时恢复）
    pub fn set_geometry_locked(h: Hwnd, locked: bool) {
        ns::set_geometry_locked(h.0, locked);
    }

    /// 应用切到前台 + `makeKeyAndOrderFront:`
    pub fn set_foreground(h: Hwnd) -> bool {
        ns::activate_app();
        ns::make_key_and_order_front(h.0)
    }

    /// 建窗后立即调用：隐藏红绿灯、面板不随应用失活隐藏（不触发 GPUI 回调）
    pub fn prepare_after_open(h: Hwnd) {
        ns::prepare_new_window(h.0);
    }

    /// 收纳箱：桌面图标层之上一层 + 所有空间 / 不受调度中心与 ⌘H 影响（不显示；幂等）
    pub fn make_desktop_layer(h: Hwnd) -> bool {
        ns::make_desktop_layer(h.0)
    }

    /// 窗口实际的层级与行为（日志用）
    pub fn describe_layer(h: Hwnd) -> Option<String> {
        ns::describe_window(h.0)
    }
}
