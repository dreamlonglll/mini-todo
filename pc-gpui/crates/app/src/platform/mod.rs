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
//!
//! ## 外框 ≠ 客户区：一律按客户区摆窗
//!
//! GPUI（gpui-pre-windows 0.3.9）的无标题栏窗口在左 / 右 / 下有系统的不可见边框（96 DPI 下各 8px；不可缩放的窗口也有，
//! 因为没有 `WS_POPUP` 的顶层窗口会被系统补上 `WS_CAPTION`），只有顶边被 GPUI 去掉。界面画在客户区里，所以**可见区域 =
//! 客户区**；旧版 Tauri 无边框窗口外框就是客户区，库里存的主窗口 / 收纳箱几何都是这个可见区域。
//! GPUI 建窗时 `calculate_window_rect` 还把只在底部的边框高度对半分到上下，按 `WindowOptions.window_bounds` 建出来的客户区
//! 整体比请求的高 `height_offset / 2`（96 DPI 下 4px）。因此：
//!
//! - 建窗：[`crate::windows::open_window`] 建好后统一按客户区校正一次（[`Hwnd::set_client_rect`]），见 `WindowSpec::client_rect`；
//! - 运行时摆窗：[`set_client_rect_physical`] / [`set_client_position_physical`]（延后执行）或 [`Hwnd::set_client_rect`] /
//!   [`Hwnd::set_client_position`]（立即执行）；读几何用 [`client_rect_physical`] / [`Hwnd::client_rect`]；
//! - [`window_rect_physical`] / [`Hwnd::rect`] / [`Hwnd::set_rect`] 是外框（含不可见边框），只用于换算与诊断。

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

    /// 逻辑矩形在缩放为 `scale` 的显示器上的物理矩形（与 GPUI 建窗时 `Bounds::to_device_pixels` 相同的 f32 四舍五入）
    pub fn from_logical(bounds: Bounds<Pixels>, scale: f64) -> Self {
        let s = if scale > 0.0 { scale as f32 } else { 1.0 };
        let r = |v: Pixels| (f32::from(v) * s).round() as i32;
        Self::new(
            r(bounds.origin.x),
            r(bounds.origin.y),
            r(bounds.size.width),
            r(bounds.size.height),
        )
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

/// 外框与客户区之间的边框厚度（物理像素）。由同一时刻读到的外框与客户区得出，用来把"客户区应在哪里"换算成
/// `SetWindowPos` 要的外框；跨 DPI 移动后边框厚度会变，要重新读
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct FrameInsets {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl FrameInsets {
    pub fn between(outer: PhysRect, client: PhysRect) -> Self {
        Self {
            left: client.x - outer.x,
            top: client.y - outer.y,
            right: outer.right() - client.right(),
            bottom: outer.bottom() - client.bottom(),
        }
    }

    /// 让客户区正好是 `client` 的外框
    pub fn outer_for(&self, client: PhysRect) -> PhysRect {
        PhysRect::new(
            client.x - self.left,
            client.y - self.top,
            client.width + self.left + self.right,
            client.height + self.top + self.bottom,
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

/// 窗口外框（物理像素，含不可见边框；界面几何用 [`client_rect_physical`]）
pub fn window_rect_physical(window: &Window) -> Option<PhysRect> {
    imp::window_rect_physical(window)
}

/// 按客户区（= 可见区域）一次性设置位置与尺寸（物理像素；换算成外框后 `SetWindowPos(SWP_NOZORDER | SWP_NOACTIVATE)`，
/// 跨 DPI 时再校正；延后执行）。其它平台 no-op（GPUI 在 Linux 上不能移动窗口，需要改尺寸时调用方自己 `window.resize`）
pub fn set_client_rect_physical(window: &Window, cx: &App, rect: PhysRect) {
    imp::set_client_rect_physical(window, cx, rect)
}

/// 按客户区左上角移动（物理像素，不改尺寸；延后执行）
pub fn set_client_position_physical(window: &Window, cx: &App, x: i32, y: i32) {
    imp::set_client_position_physical(window, cx, x, y)
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
    /// 外框（含不可见边框）
    pub fn rect(self) -> Option<PhysRect> {
        imp::raw::rect(self)
    }
    /// 按外框摆窗（摆可见区域请用 [`Hwnd::set_client_rect`]）
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
// 客户区几何（主窗口位置记忆、贴边隐藏）与固定模式的几何锁定
// ----------------------------------------------------------------------

/// 客户区（物理像素，虚拟屏幕坐标）= 界面的可见区域。
///
/// GPUI 窗口（可缩放与否都一样）在左 / 右 / 下有系统不可见的边框，`GetWindowRect` 比界面大一圈（见模块文档）；
/// 旧版 Tauri 无边框窗口的外框就是客户区，库里存的位置 / 尺寸也是客户区——主窗口的保存、恢复与贴边计算、收纳箱的
/// 几何落库都用它。其它平台 = `window_rect_physical`
pub fn client_rect_physical(window: &Window) -> Option<PhysRect> {
    imp::client_rect_physical(window)
}

/// 固定模式的几何锁定：边框缩放热区、标题栏拖动区、系统菜单 / 键盘的移动与缩放一律不响应
/// （Windows 子类化 `WM_NCHITTEST` / `WM_SYSCOMMAND`；GPUI 的缩放热区按建窗时的 `is_resizable`，没有运行时开关）。
/// 延后执行；其它平台 no-op（拖动区由界面自己不渲染）
pub fn set_geometry_locked(window: &Window, cx: &App, locked: bool) {
    imp::set_geometry_locked(window, cx, locked)
}

/// 立即执行的原始调用（同样只在 App 未被借用时调用）
impl Hwnd {
    /// 客户区（物理像素）
    pub fn client_rect(self) -> Option<PhysRect> {
        imp::raw::client_rect(self)
    }
    /// 按客户区设置位置与尺寸：已经是目标就什么都不做；否则按当前边框换算成外框 `SetWindowPos(SWP_NOZORDER |
    /// SWP_NOACTIVATE)`，跨 DPI 移动（`WM_DPICHANGED` 改了尺寸 / 边框厚度）时按新边框再校正，最多 3 次。
    /// 返回 `SetWindowPos` 是否成功
    pub fn set_client_rect(self, rect: PhysRect) -> bool {
        imp::raw::set_client_rect(self, rect)
    }
    /// 按客户区左上角移动（不改尺寸；同样会校正跨 DPI 的偏差）
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

#[cfg(test)]
mod tests {
    use super::*;

    /// gpui-pre-windows 0.3.9 `calculate_window_rect`：边框差值对半分到两侧（含只在底部的高度差）
    fn gpui_outer_rect(client: PhysRect, width_offset: i32, height_offset: i32) -> PhysRect {
        let (left, top) = (width_offset / 2, height_offset / 2);
        PhysRect::new(
            client.x - left,
            client.y - top,
            client.width + width_offset,
            client.height + height_offset,
        )
    }

    /// 系统实际的客户区：顶边被 GPUI 的 `WM_NCCALCSIZE` 去掉，左 / 右 / 下保留不可见边框
    fn system_client(outer: PhysRect, border: i32) -> PhysRect {
        PhysRect::new(
            outer.x + border,
            outer.y,
            outer.width - 2 * border,
            outer.height - border,
        )
    }

    #[test]
    fn gpui_window_bounds_put_the_client_area_half_a_border_too_high() {
        // 96 DPI：左右下各 8px。请求客户区 (0,240) 380×600 → 实际 (0,236)（e2e N02 / 主窗口居中 y=236 的成因）
        let requested = PhysRect::new(0, 240, 380, 600);
        let outer = gpui_outer_rect(requested, 16, 8);
        let actual = system_client(outer, 8);
        assert_eq!(actual, PhysRect::new(0, 236, 380, 600));

        // 按实测边框换算回外框，客户区正好落在请求的位置
        let insets = FrameInsets::between(outer, actual);
        assert_eq!(
            insets,
            FrameInsets {
                left: 8,
                top: 0,
                right: 8,
                bottom: 8
            }
        );
        let corrected = insets.outer_for(requested);
        assert_eq!(corrected, PhysRect::new(-8, 240, 396, 608));
        assert_eq!(system_client(corrected, 8), requested);
    }

    #[test]
    fn fence_geometry_is_the_visible_client_area() {
        // e2e F07：按外框摆放时可见区域小 16×8、左缘右移 8px；按客户区摆放后与库里的几何一致
        let stored = PhysRect::new(1528, 24, 552, 504);
        let as_outer = system_client(stored, 8);
        assert_eq!(as_outer, PhysRect::new(1536, 24, 536, 496));
        let insets = FrameInsets::between(stored, as_outer);
        assert_eq!(system_client(insets.outer_for(stored), 8), stored);
        // 无边框（其它平台 / 外框 = 客户区）时换算是恒等的
        assert_eq!(FrameInsets::default().outer_for(stored), stored);
    }

    #[test]
    fn logical_bounds_round_like_gpui() {
        let bounds = Bounds::new(point(px(770.), px(240.)), size(px(380.), px(600.)));
        assert_eq!(
            PhysRect::from_logical(bounds, 1.0),
            PhysRect::new(770, 240, 380, 600)
        );
        // 125%：(1536 − 320 − 20, 825.6 − 120 − 20) → 物理按 f32 四舍五入
        let bounds = Bounds::new(point(px(1196.), px(685.6)), size(px(320.), px(120.)));
        assert_eq!(
            PhysRect::from_logical(bounds, 1.25),
            PhysRect::new(1495, 857, 400, 150)
        );
        // 非法缩放按 1
        assert_eq!(
            PhysRect::from_logical(bounds, 0.0),
            PhysRect::new(1196, 686, 320, 120)
        );
    }
}
