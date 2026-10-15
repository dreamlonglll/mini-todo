//! GPUI 没有的窗口能力（位置、显示 / 隐藏、置顶、ex style、owner、圆角、显示器物理信息、拖动窗口……）。
//!
//! - Windows：`windows.rs` 直接调 Win32（HWND 来自 `raw_window_handle`）。
//! - 其它平台（只用于 CI 与截图）：`fallback.rs` 用 GPUI 自带能力近似，做不到的是 no-op。
//!
//! ## 重要：会给窗口同步发消息的 Win32 调用要**延后**到 App 借用之外
//!
//! `SetWindowPos` / `ShowWindow` / 改 style 会同步给 GPUI 的窗口过程发 `WM_MOVE` / `WM_SIZE` / `WM_ACTIVATE`…；
//! 如果当时正处在 GPUI 的事件处理 / `update` / `render` 里（App 已被借用），GPUI 的回调拿不到 App，
//! 只能记一条错误并丢掉这次几何变化（窗口尺寸 / 位置状态就错了）。所以：
//!
//! - 本模块里接收 `(&Window, &App)` 的**写操作**一律经 `cx.spawn` 延后到下一个前台任务执行（与 GPUI 自己的
//!   `Window::resize` 做法相同），调用方无需关心；
//! - [`Hwnd`] 上的方法是**立即执行**的原始调用，只能在 App 未被借用的地方用（例如 `cx.spawn` 的 async 块里、
//!   不在 `update` 闭包内），一般用于 200ms 轮询之类的守护任务。
//!
//! 读操作（`GetWindowRect`、显示器枚举、光标位置）不发消息，可以随时调用。
//!
//! 几何单位：带 `physical` 的是**物理像素**（虚拟屏幕坐标，与数据库里存的一致）；GPUI 的 `Bounds<Pixels>` 是逻辑像素
//! （Windows 上按各显示器自己的缩放换算，多显示器混合 DPI 时逻辑坐标不连续）。

use gpui_kit::{App, Bounds, DisplayId, Pixels, Window, point, px, size};

#[cfg(windows)]
mod windows;
#[cfg(windows)]
use self::windows as imp;

#[cfg(not(windows))]
mod fallback;
#[cfg(not(windows))]
use self::fallback as imp;

pub mod embed;
pub mod fence_win;
pub mod file_dialog;
#[cfg(windows)]
mod subclass;

/// 原生窗口句柄（Windows 上是 HWND 的整数值；其它平台不会出现）
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Hwnd(pub isize);

/// 物理像素矩形（`x/y` 为虚拟屏幕坐标）
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct PhysRect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl PhysRect {
    pub fn new(x: i32, y: i32, width: i32, height: i32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub fn right(&self) -> i32 {
        self.x + self.width
    }

    pub fn bottom(&self) -> i32 {
        self.y + self.height
    }

    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x && x < self.right() && y >= self.y && y < self.bottom()
    }

    /// 按缩放换算成逻辑像素
    pub fn to_logical(&self, scale: f64) -> Bounds<Pixels> {
        let s = if scale > 0.0 { scale } else { 1.0 };
        Bounds::new(
            point(
                px((self.x as f64 / s) as f32),
                px((self.y as f64 / s) as f32),
            ),
            size(
                px((self.width as f64 / s) as f32),
                px((self.height as f64 / s) as f32),
            ),
        )
    }
}

/// 一块显示器（物理像素）
#[derive(Clone, Debug, PartialEq)]
pub struct MonitorInfo {
    /// GPUI 的显示器 id（Windows 上即 HMONITOR，插拔后会变，不要持久化）
    pub display_id: DisplayId,
    /// 整块显示器（`rcMonitor`）
    pub rect: PhysRect,
    /// 工作区（`rcWork`，不含任务栏）
    pub work: PhysRect,
    /// 缩放（DPI / 96；1.0 = 100%）
    pub scale: f64,
    pub primary: bool,
}

impl MonitorInfo {
    /// 整块显示器的逻辑像素矩形（子窗口居中用）
    pub fn logical_rect(&self) -> Bounds<Pixels> {
        self.rect.to_logical(self.scale)
    }

    /// 工作区的逻辑像素矩形（通知窗口摆放用）
    pub fn logical_work(&self) -> Bounds<Pixels> {
        self.work.to_logical(self.scale)
    }
}

/// 窗口的原生句柄（只有 Windows 有）
pub fn hwnd(window: &Window) -> Option<Hwnd> {
    imp::hwnd(window)
}

/// 程序化开始拖动窗口（例如收纳箱标题栏移动超过 3px 后才拖）。
///
/// Windows：延后到前台任务里 `ReleaseCapture` + `SendMessageW(WM_NCLBUTTONDOWN, HTCAPTION)`（与 tao
/// `drag_window` 相同；不能在事件处理里同步做，模态移动循环期间 App 处于借用状态会让前台任务 panic）；
/// 其它平台 `window.start_window_move()`。
///
/// 普通的"按住标题栏拖动"请用 [`crate::ui::title_bar::drag_region`]（Windows 上走系统 `HTCAPTION`，
/// 支持贴边分屏与双击最大化）。
pub fn begin_window_drag(window: &Window, cx: &App) {
    imp::begin_window_drag(window, cx)
}

/// 最大化 ⇄ 还原
pub fn toggle_maximize(window: &Window, cx: &App) {
    imp::toggle_maximize(window, cx)
}

/// 窗口外框（物理像素）
pub fn window_rect_physical(window: &Window) -> Option<PhysRect> {
    imp::window_rect_physical(window)
}

/// 一次性设置位置与尺寸（物理像素，`SetWindowPos(SWP_NOZORDER | SWP_NOACTIVATE)`；延后执行）
pub fn set_window_rect_physical(window: &Window, cx: &App, rect: PhysRect) {
    imp::set_window_rect_physical(window, cx, rect)
}

/// 只改位置（物理像素；延后执行）
pub fn set_window_position_physical(window: &Window, cx: &App, x: i32, y: i32) {
    imp::set_window_position_physical(window, cx, x, y)
}

/// 全部显示器（Windows：`EnumDisplayMonitors` 顺序；其它平台：GPUI 显示器，缩放按 1）
pub fn monitors(cx: &App) -> Vec<MonitorInfo> {
    imp::monitors(cx)
}

/// 主显示器
pub fn primary_monitor(cx: &App) -> Option<MonitorInfo> {
    let all = monitors(cx);
    all.iter()
        .find(|m| m.primary)
        .cloned()
        .or_else(|| all.into_iter().next())
}

/// 窗口所在显示器（取不到时主显示器）
pub fn monitor_of_window(window: &Window, cx: &App) -> Option<MonitorInfo> {
    imp::monitor_of_window(window, cx).or_else(|| primary_monitor(cx))
}

/// 光标所在显示器（取不到时主显示器）
pub fn monitor_at_cursor(cx: &App) -> Option<MonitorInfo> {
    let all = monitors(cx);
    cursor_position_physical()
        .and_then(|(x, y)| all.iter().find(|m| m.rect.contains(x, y)).cloned())
        .or_else(|| primary_monitor(cx))
}

/// 系统光标位置（物理像素；其它平台 `None`）
pub fn cursor_position_physical() -> Option<(i32, i32)> {
    imp::cursor_position_physical()
}

/// 当前屏幕组合 ID（`"{n}_{W}x{H}@{scale%}_…"`；debug 下 `MINI_TODO_SCREEN_ID_FILE` 可覆盖）
pub fn screen_config_id(cx: &App) -> String {
    let list: Vec<(u32, u32, f64)> = monitors(cx)
        .iter()
        .map(|m| {
            (
                m.rect.width.max(0) as u32,
                m.rect.height.max(0) as u32,
                m.scale,
            )
        })
        .collect();
    mini_todo_core::settings::screen_config_id(&list)
}

/// 置顶 / 取消置顶（不激活；延后执行）
pub fn set_topmost(window: &Window, cx: &App, topmost: bool) {
    imp::set_topmost(window, cx, topmost)
}

/// 显示窗口（不抢焦点；延后执行）
pub fn show_window(window: &Window, cx: &App) {
    imp::show_window(window, cx)
}

/// 隐藏窗口（延后执行）
pub fn hide_window(window: &Window, cx: &App) {
    imp::hide_window(window, cx)
}

/// 是否可见（`IsWindowVisible`；其它平台恒 true）
pub fn is_window_visible(window: &Window) -> bool {
    imp::is_window_visible(window)
}

/// `WS_EX_TOOLWINDOW`（true：不进任务栏 / Alt+Tab）⇄ `WS_EX_APPWINDOW`（延后执行）
pub fn set_tool_window(window: &Window, cx: &App, tool: bool) {
    imp::set_tool_window(window, cx, tool)
}

/// `WS_MINIMIZEBOX`（嵌入桌面时去掉，Win+D / Win+M 不会把窗口最小化；延后执行）
pub fn set_minimizable(window: &Window, cx: &App, minimizable: bool) {
    imp::set_minimizable(window, cx, minimizable)
}

/// `WS_THICKFRAME | WS_MAXIMIZEBOX`（固定模式不可缩放；延后执行）。
///
/// 注意：GPUI 的自绘缩放热区与 `WindowControlArea` 命中测试用的是**建窗时**的 `is_resizable`，
/// 去掉样式只能阻止系统边框缩放；运行时不可缩放的窗口建窗时就给 `is_resizable: false`，或另外拦截。
pub fn set_resizable_frame(window: &Window, cx: &App, resizable: bool) {
    imp::set_resizable_frame(window, cx, resizable)
}

/// Win11 DWM 圆角（`DWMWCP_ROUND` / `DWMWCP_DONOTROUND`；不发窗口消息，立即执行；Win10 无效果）
pub fn set_round_corners(window: &Window, round: bool) {
    imp::set_round_corners(window, round)
}

/// owner 窗口（`GWLP_HWNDPARENT`）：子窗口始终在 owner 之上、随 owner 最小化。设置 owner 时去掉
/// `WS_EX_APPWINDOW`（与旧版 Tauri `parent` 一样不单独占任务栏按钮），清除时加回（延后执行）
pub fn set_owner(window: &Window, cx: &App, owner: Option<Hwnd>) {
    imp::set_owner(window, cx, owner)
}

/// 最小化了就还原（延后执行）
pub fn restore_if_minimized(window: &Window, cx: &App) {
    imp::restore_if_minimized(window, cx)
}

/// 显示 + 还原 + 激活到前台（托盘唤起 / 单实例唤起用）
pub fn bring_to_front(window: &Window, cx: &App) {
    imp::bring_to_front(window, cx)
}

/// 立即执行的原始调用（Windows 实现；其它平台全部 no-op / `None`）。
///
/// **只在 App 未被借用时调用**（`cx.spawn` 的 async 块里、`update` 闭包外）。
impl Hwnd {
    pub fn rect(self) -> Option<PhysRect> {
        imp::raw::rect(self)
    }
    pub fn set_rect(self, rect: PhysRect) -> bool {
        imp::raw::set_rect(self, rect)
    }
    pub fn set_position(self, x: i32, y: i32) -> bool {
        imp::raw::set_position(self, x, y)
    }
    pub fn set_topmost(self, topmost: bool) -> bool {
        imp::raw::set_topmost(self, topmost)
    }
    /// `SW_SHOWNOACTIVATE`
    pub fn show(self) -> bool {
        imp::raw::show(self)
    }
    pub fn hide(self) -> bool {
        imp::raw::hide(self)
    }
    pub fn is_visible(self) -> bool {
        imp::raw::is_visible(self)
    }
    pub fn is_minimized(self) -> bool {
        imp::raw::is_minimized(self)
    }
    pub fn restore(self) -> bool {
        imp::raw::restore(self)
    }
    pub fn is_alive(self) -> bool {
        imp::raw::is_alive(self)
    }
    pub fn ex_style(self) -> u32 {
        imp::raw::ex_style(self)
    }
    pub fn set_tool_window(self, tool: bool) {
        imp::raw::set_tool_window(self, tool)
    }
    pub fn set_minimizable(self, minimizable: bool) {
        imp::raw::set_minimizable(self, minimizable)
    }
    pub fn set_resizable_frame(self, resizable: bool) {
        imp::raw::set_resizable_frame(self, resizable)
    }
    pub fn set_owner(self, owner: Option<Hwnd>) {
        imp::raw::set_owner(self, owner)
    }
    pub fn owner(self) -> Option<Hwnd> {
        imp::raw::owner(self)
    }
    pub fn set_round_corners(self, round: bool) {
        imp::raw::set_round_corners(self, round)
    }
    /// 窗口类名（`GetClassNameW`）
    pub fn class_name(self) -> Option<String> {
        imp::raw::class_name(self)
    }
}

// ----------------------------------------------------------------------
// W1 追加：客户区几何（主窗口位置记忆、贴边隐藏）与固定模式的几何锁定
// ----------------------------------------------------------------------

/// 客户区（物理像素，虚拟屏幕坐标）。
///
/// 可缩放的 GPUI 窗口（`is_resizable: true`）在左 / 右 / 下有系统不可见的缩放边框，`GetWindowRect` 比界面大一圈；
/// 旧版 Tauri 无边框窗口的外框就是客户区，库里存的位置 / 尺寸也是客户区——主窗口的保存、恢复与贴边计算都用它。
/// 不可缩放的窗口（收纳箱、通知）两者相同。其它平台 = `window_rect_physical`
pub fn client_rect_physical(window: &Window) -> Option<PhysRect> {
    imp::client_rect_physical(window)
}

/// 固定模式的几何锁定：边框缩放热区、标题栏拖动区、系统菜单 / 键盘的移动与缩放一律不响应
/// （Windows 子类化 `WM_NCHITTEST` / `WM_SYSCOMMAND`；GPUI 的缩放热区按建窗时的 `is_resizable`，没有运行时开关）。
/// 延后执行；其它平台 no-op（拖动区由界面自己不渲染）
pub fn set_geometry_locked(window: &Window, cx: &App, locked: bool) {
    imp::set_geometry_locked(window, cx, locked)
}

/// 立即执行的原始调用（W1 追加；同样只在 App 未被借用时调用）
impl Hwnd {
    /// 客户区（物理像素）
    pub fn client_rect(self) -> Option<PhysRect> {
        imp::raw::client_rect(self)
    }
    /// 按客户区设置位置与尺寸（换算成外框后一次 `SetWindowPos(SWP_NOZORDER | SWP_NOACTIVATE)`）
    pub fn set_client_rect(self, rect: PhysRect) -> bool {
        imp::raw::set_client_rect(self, rect)
    }
    /// 按客户区左上角移动（不改尺寸）
    pub fn set_client_position(self, x: i32, y: i32) -> bool {
        imp::raw::set_client_position(self, x, y)
    }
    /// 见 [`set_geometry_locked`]
    pub fn set_geometry_locked(self, locked: bool) {
        imp::raw::set_geometry_locked(self, locked)
    }
    /// 激活并带到前台（`SetForegroundWindow`；被前台锁限制时只会闪任务栏，调用方再配合 `activate_window`）
    pub fn set_foreground(self) -> bool {
        imp::raw::set_foreground(self)
    }
}
