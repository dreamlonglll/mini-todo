//! 主窗口模式相关的纯逻辑（原 `commands/window.rs` / `lib.rs` 中与 Tauri、Win32 无关的部分）。
//!
//! app 层用它们驱动自己的 Win32 调用（`SetWindowPos`、owner、ex style……），这里只做判断与状态机：
//!
//! - 贴边自动隐藏 / 唤起状态机（仅经典固定模式）：[`AutoHideState`] + [`evaluate_auto_hide_transition`]，
//!   每 200ms 由 app 喂入窗口外框、所在显示器（**整块显示器**，物理像素）、光标位置
//! - 运行时偏好刷新：[`apply_runtime_prefs`]（启动 / 导入 / 同步应用远端设置 / 保存窗口状态后）
//! - 桌面模式（owner=Progman）守护：[`needs_reattach`]、[`lacks_tool_window_style`]、[`is_desktop_host_class`]
//! - 托盘：左键连点去抖 [`accept_tray_click`]、唤起代次 [`begin_tray_wake`] / [`is_latest_tray_wake`]
//!
//! 状态由 app 持有（原来的 `AUTO_HIDE_STATE` 全局量、`TOP_ON_WAKE` 原子量都在 app 层）。
//! 时序约束与 Win32 结果见 `.trellis/spec/backend/window-modes.md`。

use crate::db::{WindowPosition, WindowSize};
use serde::Serialize;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub const EDGE_SNAP_THRESHOLD_PX: i32 = 10;
pub const EDGE_HIDE_DELAY: Duration = Duration::from_millis(420);
/// 托盘唤起后的保持显示宽限期
///
/// 没有它的话，鼠标不在窗口范围内，窗口会在 `EDGE_HIDE_DELAY` 后立刻缩回去，
/// 用户刚点完托盘就看它消失。宽限期内足够把鼠标移过去接管。
pub const TRAY_WAKE_KEEP_VISIBLE: Duration = Duration::from_secs(3);
/// 托盘唤起时临时置顶的持续时间
///
/// 置顶只是把窗口抬到最前的手段，到点就撤销——除非贴边唤起态本就需要保持置顶。
pub const TRAY_WAKE_TOPMOST_DURATION: Duration = Duration::from_millis(600);
pub const HIDDEN_VISIBLE_STRIP_PX: i32 = 4;
pub const WAKE_HOTZONE_WIDTH_PX: i32 = 2;
pub const WAKE_RANGE_PADDING_PX: i32 = 40;

/// 窗口贴住的屏幕边
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DockEdge {
    Left,
    Right,
    Top,
    Bottom,
}

/// 显示器整块区域（物理像素，虚拟屏幕坐标；贴边判断用整块显示器，不是工作区）
#[derive(Debug, Clone, Copy)]
pub struct MonitorBounds {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl MonitorBounds {
    pub fn right(self) -> i32 {
        self.x + self.width
    }

    pub fn bottom(self) -> i32 {
        self.y + self.height
    }
}

/// 窗口外框（物理像素）
#[derive(Debug, Clone, Copy)]
pub struct WindowRect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl WindowRect {
    pub fn right(self) -> i32 {
        self.x + self.width
    }

    pub fn bottom(self) -> i32 {
        self.y + self.height
    }
}

/// 贴边自动隐藏状态机（原 `AUTO_HIDE_STATE`）。由 app 持有，每 200ms 调一次
/// [`evaluate_auto_hide_transition`]；模式切换时用下面的辅助方法改状态。
#[derive(Debug)]
pub struct AutoHideState {
    pub enabled: bool,
    pub hidden: bool,
    pub cursor_inside_window: bool,
    pub docked_edge: Option<DockEdge>,
    pub monitor_bounds: Option<MonitorBounds>,
    pub anchor_position: Option<WindowPosition>,
    pub anchor_size: Option<WindowSize>,
    pub edge_stick_started_at: Option<Instant>,
    /// 托盘唤起后的免隐藏截止时间
    pub keep_visible_until: Option<Instant>,
}

impl Default for AutoHideState {
    fn default() -> Self {
        Self {
            enabled: false,
            hidden: false,
            cursor_inside_window: true,
            docked_edge: None,
            monitor_bounds: None,
            anchor_position: None,
            anchor_size: None,
            edge_stick_started_at: None,
            keep_visible_until: None,
        }
    }
}

/// 保存窗口状态时用的几何（原 `get_window_persist_state` 的返回值）
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowPersistState {
    pub position: WindowPosition,
    pub size: WindowSize,
}

/// [`evaluate_auto_hide_transition`] 的结果：调用方据此挪窗口
/// （`Hide` → 移到 `hidden` 并撤置顶，失败时把状态回滚为显示；`Restore` → 移回 `anchor`，
/// 成功且 `top_on_wake` 时置顶，失败时把 `hidden` 设回 true）
#[derive(Debug, Clone)]
pub enum AutoHideTransition {
    None,
    Hide {
        anchor: WindowPosition,
        hidden: WindowPosition,
        size: WindowSize,
        edge: DockEdge,
        monitor: MonitorBounds,
    },
    Restore {
        anchor: WindowPosition,
    },
}

impl AutoHideState {
    /// 清掉贴边运行时状态（隐藏态、贴边边、锚点、计时、托盘宽限），保留 `enabled`（原 `clear_auto_hide_runtime_state`，
    /// 重置窗口位置时调用；调用方随后撤销置顶）
    pub fn clear_runtime(&mut self) {
        self.hidden = false;
        self.cursor_inside_window = true;
        self.docked_edge = None;
        self.monitor_bounds = None;
        self.anchor_position = None;
        self.anchor_size = None;
        self.edge_stick_started_at = None;
        self.keep_visible_until = None;
    }

    /// 窗口当前是否应保持置顶：仅在贴边唤起且开关打开时成立（原 `should_stay_on_top`）。
    /// 托盘唤起的临时置顶到点时据此判断该不该撤。
    pub fn should_stay_on_top(&self, top_on_wake: bool) -> bool {
        top_on_wake && self.enabled && !self.hidden && self.docked_edge.is_some()
    }

    /// 进入（或重复进入）经典固定模式（原 `set_window_fixed_mode(true)` 的状态部分）：贴边开关取
    /// `auto_hide_enabled`，清隐藏态与贴边计时。返回藏在屏幕边缘时的锚点——调用方要把窗口拉回去
    /// （只清 hidden 不挪窗口，窗口会卡在屏幕外）。
    pub fn enter_fixed_mode(&mut self, auto_hide_enabled: bool) -> Option<WindowPosition> {
        let restore = self.hidden_anchor();
        self.enabled = auto_hide_enabled;
        self.hidden = false;
        self.docked_edge = None;
        self.edge_stick_started_at = None;
        restore
    }

    /// 退出经典固定模式（回普通模式或切到嵌入桌面）：整个状态复位。返回藏起时的锚点，调用方拉回窗口
    /// （原 `set_window_fixed_mode(false)` / `set_window_desktop_mode(true)` 的状态部分；调用方先撤置顶）。
    pub fn leave_fixed_mode(&mut self) -> Option<WindowPosition> {
        let restore = self.hidden_anchor();
        *self = AutoHideState::default();
        restore
    }

    /// 设置页切换贴边隐藏（原 `set_auto_hide_enabled` 的状态部分）。`active` = 开关打开且处于经典固定模式。
    /// 打开：贴边计时从头开始；关闭：清隐藏态，返回藏起时的锚点（调用方拉回并显示窗口、撤销置顶）。
    pub fn set_enabled(&mut self, active: bool) -> Option<WindowPosition> {
        self.enabled = active;
        if active {
            self.hidden = false;
            self.docked_edge = None;
            self.edge_stick_started_at = None;
            None
        } else {
            let restore = self.hidden_anchor();
            self.hidden = false;
            self.docked_edge = None;
            self.monitor_bounds = None;
            self.anchor_size = None;
            self.edge_stick_started_at = None;
            restore
        }
    }

    /// 托盘唤起（原 `bring_main_window_to_front` 的状态部分）：贴边隐藏开着时给 [`TRAY_WAKE_KEEP_VISIBLE`]
    /// 的免隐藏宽限期、清贴边计时；藏起中则清 hidden 并返回锚点，调用方先拉回再置顶 / 聚焦。
    pub fn wake_from_tray(&mut self, now: Instant) -> Option<WindowPosition> {
        if !self.enabled {
            return None;
        }
        self.keep_visible_until = Some(now + TRAY_WAKE_KEEP_VISIBLE);
        self.edge_stick_started_at = None;
        if self.hidden {
            self.hidden = false;
            self.anchor_position.clone()
        } else {
            None
        }
    }

    /// 保存窗口状态用的几何（原 `get_window_persist_state`）：贴边隐藏中返回锚点位置 / 尺寸，
    /// 不把屏幕外的隐藏位置存进库
    pub fn persist_state(&self, position: WindowPosition, size: WindowSize) -> WindowPersistState {
        let mut persist = WindowPersistState { position, size };
        if self.hidden {
            if let Some(anchor) = &self.anchor_position {
                persist.position = anchor.clone();
            }
            if let Some(anchor_size) = &self.anchor_size {
                persist.size = anchor_size.clone();
            }
        }
        persist
    }

    fn hidden_anchor(&self) -> Option<WindowPosition> {
        if self.hidden {
            self.anchor_position.clone()
        } else {
            None
        }
    }
}

/// 重置窗口（托盘「重置位置」）的默认尺寸（逻辑像素）
pub const RESET_WINDOW_WIDTH: f64 = 380.0;
pub const RESET_WINDOW_HEIGHT: f64 = 600.0;

/// 重置窗口的位置（物理像素，原 `reset_window_impl`）：主显示器左上角起各留 `尺寸 × 10% ÷ 缩放` 的边距。
/// `monitor` 为主显示器整块区域（物理像素），`scale` 为其缩放比例（不合法时按 1）。
pub fn reset_window_position(monitor: MonitorBounds, scale: f64) -> WindowPosition {
    let scale = if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    };
    let margin_x = (monitor.width as f64 * 0.1 / scale) as i32;
    let margin_y = (monitor.height as f64 * 0.1 / scale) as i32;
    WindowPosition {
        x: monitor.x + margin_x,
        y: monitor.y + margin_y,
    }
}

fn clamp_i32(value: i32, min: i32, max: i32) -> i32 {
    if max < min {
        return min;
    }
    value.max(min).min(max)
}

fn point_in_rect(x: i32, y: i32, rect: WindowRect) -> bool {
    x >= rect.x && x <= rect.right() && y >= rect.y && y <= rect.bottom()
}

pub fn detect_docked_edge(rect: WindowRect, monitor: MonitorBounds) -> Option<DockEdge> {
    let left_gap = (rect.x - monitor.x).abs();
    let right_gap = (monitor.right() - rect.right()).abs();
    let top_gap = (rect.y - monitor.y).abs();
    let bottom_gap = (monitor.bottom() - rect.bottom()).abs();

    let mut candidates = Vec::with_capacity(4);
    if left_gap <= EDGE_SNAP_THRESHOLD_PX {
        candidates.push((DockEdge::Left, left_gap));
    }
    if right_gap <= EDGE_SNAP_THRESHOLD_PX {
        candidates.push((DockEdge::Right, right_gap));
    }
    if top_gap <= EDGE_SNAP_THRESHOLD_PX {
        candidates.push((DockEdge::Top, top_gap));
    }
    if bottom_gap <= EDGE_SNAP_THRESHOLD_PX {
        candidates.push((DockEdge::Bottom, bottom_gap));
    }

    candidates.sort_by_key(|(_, gap)| *gap);
    candidates.first().map(|(edge, _)| *edge)
}

pub fn calc_anchor_and_hidden_position(
    rect: WindowRect,
    monitor: MonitorBounds,
    edge: DockEdge,
) -> (WindowPosition, WindowPosition, WindowSize) {
    let max_x = monitor.right() - rect.width;
    let max_y = monitor.bottom() - rect.height;
    let clamped_x = clamp_i32(rect.x, monitor.x, max_x);
    let clamped_y = clamp_i32(rect.y, monitor.y, max_y);

    let (anchor_x, anchor_y, hidden_x, hidden_y) = match edge {
        DockEdge::Left => (
            monitor.x,
            clamped_y,
            monitor.x - rect.width + HIDDEN_VISIBLE_STRIP_PX,
            clamped_y,
        ),
        DockEdge::Right => (
            max_x,
            clamped_y,
            monitor.right() - HIDDEN_VISIBLE_STRIP_PX,
            clamped_y,
        ),
        DockEdge::Top => (
            clamped_x,
            monitor.y,
            clamped_x,
            monitor.y - rect.height + HIDDEN_VISIBLE_STRIP_PX,
        ),
        DockEdge::Bottom => (
            clamped_x,
            max_y,
            clamped_x,
            monitor.bottom() - HIDDEN_VISIBLE_STRIP_PX,
        ),
    };

    (
        WindowPosition {
            x: anchor_x,
            y: anchor_y,
        },
        WindowPosition {
            x: hidden_x,
            y: hidden_y,
        },
        WindowSize {
            width: rect.width as u32,
            height: rect.height as u32,
        },
    )
}

/// 光标是否落在贴边窗口的唤起范围内：紧贴屏幕边缘、宽 `WAKE_HOTZONE_WIDTH_PX` 的热区，
/// 沿边方向在窗口范围两侧各放宽 `WAKE_RANGE_PADDING_PX`。
pub fn cursor_in_wake_range(
    cursor_x: i32,
    cursor_y: i32,
    edge: DockEdge,
    monitor: MonitorBounds,
    window: WindowRect,
) -> bool {
    let vertical_min = window.y - WAKE_RANGE_PADDING_PX;
    let vertical_max = window.bottom() + WAKE_RANGE_PADDING_PX;
    let horizontal_min = window.x - WAKE_RANGE_PADDING_PX;
    let horizontal_max = window.right() + WAKE_RANGE_PADDING_PX;

    match edge {
        DockEdge::Left => {
            cursor_x >= monitor.x
                && cursor_x <= monitor.x + WAKE_HOTZONE_WIDTH_PX
                && cursor_y >= vertical_min
                && cursor_y <= vertical_max
        }
        DockEdge::Right => {
            cursor_x <= monitor.right()
                && cursor_x >= monitor.right() - WAKE_HOTZONE_WIDTH_PX
                && cursor_y >= vertical_min
                && cursor_y <= vertical_max
        }
        DockEdge::Top => {
            cursor_y >= monitor.y
                && cursor_y <= monitor.y + WAKE_HOTZONE_WIDTH_PX
                && cursor_x >= horizontal_min
                && cursor_x <= horizontal_max
        }
        DockEdge::Bottom => {
            cursor_y <= monitor.bottom()
                && cursor_y >= monitor.bottom() - WAKE_HOTZONE_WIDTH_PX
                && cursor_x >= horizontal_min
                && cursor_x <= horizontal_max
        }
    }
}

pub fn should_wake_hidden_window(cursor_x: i32, cursor_y: i32, state: &AutoHideState) -> bool {
    let (Some(edge), Some(monitor), Some(anchor), Some(size)) = (
        state.docked_edge,
        state.monitor_bounds,
        state.anchor_position.as_ref(),
        state.anchor_size.as_ref(),
    ) else {
        return false;
    };
    let anchor_rect = WindowRect {
        x: anchor.x,
        y: anchor.y,
        width: size.width as i32,
        height: size.height as i32,
    };
    cursor_in_wake_range(cursor_x, cursor_y, edge, monitor, anchor_rect)
}

pub fn evaluate_auto_hide_transition(
    state: &mut AutoHideState,
    rect: WindowRect,
    monitor: MonitorBounds,
    cursor: Option<(i32, i32)>,
    now: Instant,
) -> AutoHideTransition {
    if !state.enabled {
        return AutoHideTransition::None;
    }

    if state.hidden {
        let should_restore = if let Some((cursor_x, cursor_y)) = cursor {
            should_wake_hidden_window(cursor_x, cursor_y, state)
        } else {
            state.cursor_inside_window
        };

        if should_restore {
            if let Some(anchor) = state.anchor_position.clone() {
                state.hidden = false;
                state.edge_stick_started_at = None;
                return AutoHideTransition::Restore { anchor };
            }
        }
        return AutoHideTransition::None;
    }

    let docked_edge = detect_docked_edge(rect, monitor);
    if docked_edge.is_none() {
        state.docked_edge = None;
        state.edge_stick_started_at = None;
        return AutoHideTransition::None;
    }

    let edge = docked_edge.unwrap();
    if state.docked_edge != Some(edge) {
        state.docked_edge = Some(edge);
        state.edge_stick_started_at = Some(now);
        return AutoHideTransition::None;
    }

    let Some(started_at) = state.edge_stick_started_at else {
        state.edge_stick_started_at = Some(now);
        return AutoHideTransition::None;
    };

    // 滞回（D2）：唤起范围沿边方向比窗口宽出 WAKE_RANGE_PADDING_PX。显示态若只认窗口矩形，
    // 光标停在这条带里时会"收起 → 立刻被唤起 → 又收起"反复抖动（每次移窗还触发前端存盘），
    // 所以显示态把唤起范围也算作"光标在内"
    let cursor_inside = if let Some((cursor_x, cursor_y)) = cursor {
        point_in_rect(cursor_x, cursor_y, rect)
            || cursor_in_wake_range(cursor_x, cursor_y, edge, monitor, rect)
    } else {
        state.cursor_inside_window
    };

    if cursor_inside {
        state.edge_stick_started_at = Some(now);
        state.keep_visible_until = None;
        return AutoHideTransition::None;
    }

    // 托盘唤起的宽限期内不隐藏，鼠标还没来得及移过来
    if let Some(until) = state.keep_visible_until {
        if now < until {
            state.edge_stick_started_at = Some(now);
            return AutoHideTransition::None;
        }
        state.keep_visible_until = None;
    }

    if now.duration_since(started_at) < EDGE_HIDE_DELAY {
        return AutoHideTransition::None;
    }

    let (anchor, hidden, size) = calc_anchor_and_hidden_position(rect, monitor, edge);
    state.hidden = true;
    state.monitor_bounds = Some(monitor);
    state.anchor_position = Some(anchor.clone());
    state.anchor_size = Some(size.clone());
    state.edge_stick_started_at = None;

    AutoHideTransition::Hide {
        anchor,
        hidden,
        size,
        edge,
        monitor,
    }
}

/// 贴边隐藏已被关掉、窗口却还藏在屏幕边缘时：清掉隐藏态（同 `set_auto_hide_enabled(false)`），
/// 返回要拉回的锚点；其它情况不动状态、返回 `None`。
pub fn take_disabled_hidden_anchor(state: &mut AutoHideState) -> Option<WindowPosition> {
    if state.enabled || !state.hidden {
        return None;
    }
    let anchor = state.anchor_position.clone();
    state.hidden = false;
    state.docked_edge = None;
    state.monitor_bounds = None;
    state.anchor_size = None;
    state.edge_stick_started_at = None;
    anchor
}

/// 把新的偏好写进运行时状态；返回是否有功能被关掉（窗口侧欠一次收尾）。
/// `auto_hide_active` = 开关打开且处于普通固定模式。
pub fn apply_runtime_prefs(
    state: &mut AutoHideState,
    top_on_wake_cell: &AtomicBool,
    top_on_wake: bool,
    auto_hide_active: bool,
) -> bool {
    let top_turned_off = top_on_wake_cell.swap(top_on_wake, Ordering::SeqCst) && !top_on_wake;
    let was_enabled = state.enabled;
    state.enabled = auto_hide_active;
    if auto_hide_active && !was_enabled {
        // 与 set_auto_hide_enabled(true) 一致：贴边计时从头开始
        state.docked_edge = None;
        state.edge_stick_started_at = None;
    }
    top_turned_off || (was_enabled && !auto_hide_active)
}

/// 桌面宿主窗口的类名是否合格。
///
/// `GetShellWindow()` 在 Explorer 作为 shell 时返回类名为 `Progman` 的 "Program Manager"；
/// Win11 24H2+ 按 Win+D 时被抬到最前的正是它，以它为 owner 的窗口会被系统一并抬起。
/// 其它类名（如第三方 shell）说明桌面结构未知，Win+D 免疫不保证（只记日志，仍以它为 owner）。
pub fn is_desktop_host_class(name: &str) -> bool {
    name == "Progman"
}

/// 桌面模式轮询是否需要重新挂载 owner。
///
/// - `cached_owner == 0`：从未挂上（进入桌面模式时 Explorer 可能还没起来）
/// - `!owner_alive`：owner 句柄已失效（Explorer 崩溃 / 重启）
/// - `shell_now != cached_owner`：Explorer 重启后 Progman 换了句柄
/// - `current_owner != cached_owner`：owner 被别处清掉
///
/// `shell_now == 0` 时 Explorer 尚未重建 shell 窗口，没有宿主可挂，等下一 tick。
/// 主窗口嵌入桌面与收纳箱窗口共用这一判定。
pub fn needs_reattach(
    cached_owner: isize,
    owner_alive: bool,
    shell_now: isize,
    current_owner: isize,
) -> bool {
    if shell_now == 0 {
        return false;
    }
    cached_owner == 0 || !owner_alive || shell_now != cached_owner || current_owner != cached_owner
}

/// `WS_EX_TOOLWINDOW` / `WS_EX_APPWINDOW` 的位值（纯函数要在所有平台上编译、测试，不依赖 windows crate）
pub const EX_TOOLWINDOW_BIT: u32 = 0x0000_0080;
pub const EX_APPWINDOW_BIT: u32 = 0x0004_0000;

/// 固定 / 嵌入桌面的窗口应当是"工具窗口、无 APPWINDOW"；返回 true 表示样式被抹掉了，需要重申。
///
/// 旧版（tao）里晚到的 `apply_diff` 会整体重写 `GWL_EXSTYLE` 抹掉 `WS_EX_TOOLWINDOW`，200ms 轮询据此自愈；
/// GPUI 建窗后不再写 ex style，这一检查变为廉价的防御（每 tick 一次 `GetWindowLongW`）。
pub fn lacks_tool_window_style(ex_style: u32) -> bool {
    ex_style & EX_TOOLWINDOW_BIT == 0 || ex_style & EX_APPWINDOW_BIT != 0
}

/// 托盘唤起的代次：每次唤起加一。"600ms 后撤置顶"的任务只在自己仍是最新一次时才撤，
/// 否则连点托盘时较早的任务会提前撤掉后一次唤起的置顶。
static TRAY_WAKE_GENERATION: AtomicU64 = AtomicU64::new(0);

/// 开始一次托盘唤起，返回它的代次
pub fn begin_tray_wake() -> u64 {
    TRAY_WAKE_GENERATION.fetch_add(1, Ordering::SeqCst) + 1
}

/// `generation` 是否仍是最近一次托盘唤起
pub fn is_latest_tray_wake(generation: u64) -> bool {
    TRAY_WAKE_GENERATION.load(Ordering::SeqCst) == generation
}

/// 托盘左键连点去抖：双击会先后触发两次单击抬起事件，第二下不必再唤起一遍
pub const TRAY_CLICK_DEBOUNCE: Duration = Duration::from_millis(300);

/// 这次托盘点击是否需要处理；处理时记下时间
pub fn accept_tray_click(last: &Mutex<Option<Instant>>, now: Instant) -> bool {
    let mut last = last.lock().unwrap_or_else(|e| e.into_inner());
    let accept = last.is_none_or(|prev| now.saturating_duration_since(prev) >= TRAY_CLICK_DEBOUNCE);
    if accept {
        *last = Some(now);
    }
    accept
}

#[cfg(test)]
mod tests {
    use super::*;

    const MONITOR: MonitorBounds = MonitorBounds {
        x: 0,
        y: 0,
        width: 1920,
        height: 1080,
    };

    /// 贴在屏幕顶部的窗口
    const DOCKED_RECT: WindowRect = WindowRect {
        x: 100,
        y: 0,
        width: 380,
        height: 600,
    };

    /// 远离窗口的光标位置
    const CURSOR_OUTSIDE: Option<(i32, i32)> = Some((1500, 900));

    fn docked_state(now: Instant) -> AutoHideState {
        AutoHideState {
            enabled: true,
            docked_edge: Some(DockEdge::Top),
            // 贴边计时早已超过 EDGE_HIDE_DELAY
            edge_stick_started_at: Some(now - EDGE_HIDE_DELAY - Duration::from_millis(100)),
            ..AutoHideState::default()
        }
    }

    #[test]
    fn hides_after_edge_delay_when_cursor_is_away() {
        let now = Instant::now();
        let mut state = docked_state(now);

        let transition =
            evaluate_auto_hide_transition(&mut state, DOCKED_RECT, MONITOR, CURSOR_OUTSIDE, now);

        assert!(matches!(transition, AutoHideTransition::Hide { .. }));
        assert!(state.hidden);
    }

    #[test]
    fn keep_visible_window_blocks_hiding() {
        let now = Instant::now();
        let mut state = docked_state(now);
        state.keep_visible_until = Some(now + Duration::from_secs(3));

        let transition =
            evaluate_auto_hide_transition(&mut state, DOCKED_RECT, MONITOR, CURSOR_OUTSIDE, now);

        assert!(matches!(transition, AutoHideTransition::None));
        assert!(!state.hidden, "托盘唤起的宽限期内不应隐藏");
    }

    #[test]
    fn hides_once_keep_visible_window_expires() {
        let now = Instant::now();
        let mut state = docked_state(now);
        state.keep_visible_until = Some(now - Duration::from_millis(1));

        let transition =
            evaluate_auto_hide_transition(&mut state, DOCKED_RECT, MONITOR, CURSOR_OUTSIDE, now);

        assert!(matches!(transition, AutoHideTransition::Hide { .. }));
        assert!(state.hidden);
        assert!(state.keep_visible_until.is_none(), "过期后应清掉宽限期");
    }

    #[test]
    fn cursor_entering_window_clears_keep_visible() {
        let now = Instant::now();
        let mut state = docked_state(now);
        state.keep_visible_until = Some(now + Duration::from_secs(3));

        // 光标落在窗口内
        let transition =
            evaluate_auto_hide_transition(&mut state, DOCKED_RECT, MONITOR, Some((200, 100)), now);

        assert!(matches!(transition, AutoHideTransition::None));
        assert!(!state.hidden);
        assert!(
            state.keep_visible_until.is_none(),
            "鼠标已接管，宽限期该让位给正常的 cursor_inside 逻辑"
        );
    }

    #[test]
    fn hidden_window_restores_when_cursor_hits_wake_hotzone() {
        let now = Instant::now();
        let mut state = AutoHideState {
            enabled: true,
            hidden: true,
            docked_edge: Some(DockEdge::Top),
            monitor_bounds: Some(MONITOR),
            anchor_position: Some(WindowPosition { x: 100, y: 0 }),
            anchor_size: Some(WindowSize {
                width: 380,
                height: 600,
            }),
            ..AutoHideState::default()
        };

        // 光标顶到屏幕上边缘、且在窗口水平范围内
        let transition =
            evaluate_auto_hide_transition(&mut state, DOCKED_RECT, MONITOR, Some((200, 0)), now);

        assert!(matches!(transition, AutoHideTransition::Restore { .. }));
        assert!(!state.hidden);
    }

    // ---- 滞回（D2）----

    /// 光标停在唤起范围的 padding 带里（顶边热区内、窗口水平范围外 40px 以内）：
    /// 隐藏态会被唤起，显示态也必须视为"在内"，否则收起 / 唤起来回抖动
    #[test]
    fn cursor_in_wake_padding_keeps_a_visible_window_visible() {
        let now = Instant::now();
        let in_padding = Some((DOCKED_RECT.right() + 20, 0));

        let mut hidden = AutoHideState {
            enabled: true,
            hidden: true,
            docked_edge: Some(DockEdge::Top),
            monitor_bounds: Some(MONITOR),
            anchor_position: Some(WindowPosition { x: 100, y: 0 }),
            anchor_size: Some(WindowSize {
                width: 380,
                height: 600,
            }),
            ..AutoHideState::default()
        };
        assert!(matches!(
            evaluate_auto_hide_transition(&mut hidden, DOCKED_RECT, MONITOR, in_padding, now),
            AutoHideTransition::Restore { .. }
        ));

        let mut visible = docked_state(now);
        let transition =
            evaluate_auto_hide_transition(&mut visible, DOCKED_RECT, MONITOR, in_padding, now);
        assert!(matches!(transition, AutoHideTransition::None));
        assert!(!visible.hidden, "唤起范围内不收起");
        assert_eq!(visible.edge_stick_started_at, Some(now), "计时重新开始");

        // 离开唤起范围（padding 之外 / 热区之下）照常收起
        for away in [
            (DOCKED_RECT.right() + WAKE_RANGE_PADDING_PX + 1, 0),
            (DOCKED_RECT.right() + 20, WAKE_HOTZONE_WIDTH_PX + 1),
        ] {
            let mut visible = docked_state(now);
            assert!(
                matches!(
                    evaluate_auto_hide_transition(
                        &mut visible,
                        DOCKED_RECT,
                        MONITOR,
                        Some(away),
                        now
                    ),
                    AutoHideTransition::Hide { .. }
                ),
                "{away:?}"
            );
        }
    }

    #[test]
    fn wake_range_follows_each_edge() {
        let window = WindowRect {
            x: 0,
            y: 200,
            width: 380,
            height: 600,
        };
        // 左边：x 在热区内、y 在窗口范围 ± padding 内
        assert!(cursor_in_wake_range(
            0,
            170,
            DockEdge::Left,
            MONITOR,
            window
        ));
        assert!(!cursor_in_wake_range(
            3,
            300,
            DockEdge::Left,
            MONITOR,
            window
        ));
        assert!(!cursor_in_wake_range(
            0,
            841,
            DockEdge::Left,
            MONITOR,
            window
        ));
        // 右边
        assert!(cursor_in_wake_range(
            1919,
            300,
            DockEdge::Right,
            MONITOR,
            window
        ));
        assert!(!cursor_in_wake_range(
            1916,
            300,
            DockEdge::Right,
            MONITOR,
            window
        ));
        // 底边：x 在窗口范围 ± padding 内
        assert!(cursor_in_wake_range(
            -40,
            1080,
            DockEdge::Bottom,
            MONITOR,
            window
        ));
        assert!(!cursor_in_wake_range(
            -41,
            1080,
            DockEdge::Bottom,
            MONITOR,
            window
        ));
    }

    // ---- 运行时偏好（导入 / 同步 / save_settings 之后刷新）----

    #[test]
    fn runtime_prefs_report_when_something_was_turned_off() {
        let top = AtomicBool::new(true);
        let mut state = AutoHideState {
            enabled: true,
            docked_edge: Some(DockEdge::Top),
            ..AutoHideState::default()
        };

        assert!(!apply_runtime_prefs(&mut state, &top, true, true), "无变化");
        assert!(
            apply_runtime_prefs(&mut state, &top, false, true),
            "唤起置顶被关"
        );
        assert!(!top.load(Ordering::SeqCst));
        assert!(
            apply_runtime_prefs(&mut state, &top, false, false),
            "贴边隐藏被关"
        );
        assert!(!state.enabled);

        // 重新打开：状态从头开始，不需要收尾
        assert!(!apply_runtime_prefs(&mut state, &top, true, true));
        assert!(state.enabled);
        assert_eq!(state.docked_edge, None);
        assert!(top.load(Ordering::SeqCst));
    }

    #[test]
    fn disabled_auto_hide_pulls_a_hidden_window_back_to_its_anchor() {
        let mut state = AutoHideState {
            enabled: false,
            hidden: true,
            docked_edge: Some(DockEdge::Left),
            monitor_bounds: Some(MONITOR),
            anchor_position: Some(WindowPosition { x: 0, y: 120 }),
            anchor_size: Some(WindowSize {
                width: 380,
                height: 600,
            }),
            ..AutoHideState::default()
        };
        let anchor = take_disabled_hidden_anchor(&mut state).expect("应拉回");
        assert_eq!((anchor.x, anchor.y), (0, 120));
        assert!(!state.hidden);
        assert_eq!(state.docked_edge, None);
        assert!(
            take_disabled_hidden_anchor(&mut state).is_none(),
            "只拉一次"
        );

        // 仍然启用 / 没藏起来：不动
        let mut enabled = AutoHideState {
            enabled: true,
            hidden: true,
            anchor_position: Some(WindowPosition { x: 0, y: 0 }),
            ..AutoHideState::default()
        };
        assert!(take_disabled_hidden_anchor(&mut enabled).is_none());
        assert!(enabled.hidden);
    }

    // ---- 托盘唤起代次 ----

    #[test]
    fn only_the_latest_tray_wake_may_drop_the_topmost() {
        let first = begin_tray_wake();
        let second = begin_tray_wake();
        assert!(second > first);
        assert!(
            !is_latest_tray_wake(first),
            "旧线程不能撤掉新一次唤起的置顶"
        );
        assert!(is_latest_tray_wake(second));
    }

    // ---- 桌面模式纯函数 ----

    #[test]
    fn progman_is_the_only_qualified_desktop_host() {
        assert!(is_desktop_host_class("Progman"));
        assert!(!is_desktop_host_class("WorkerW"), "壁纸层 WorkerW 不是宿主");
        assert!(!is_desktop_host_class(""));
        assert!(!is_desktop_host_class("progman"), "类名大小写敏感");
    }

    const PROGMAN: isize = 0x102A6;
    const NEW_PROGMAN: isize = 0x2003C;

    #[test]
    fn healthy_owner_does_not_reattach() {
        assert!(!needs_reattach(PROGMAN, true, PROGMAN, PROGMAN));
    }

    #[test]
    fn reattaches_when_never_attached() {
        assert!(needs_reattach(0, false, PROGMAN, 0));
    }

    #[test]
    fn reattaches_when_owner_window_died() {
        // Explorer 崩溃：旧句柄失效，新 Progman 已起来
        assert!(needs_reattach(PROGMAN, false, NEW_PROGMAN, PROGMAN));
    }

    #[test]
    fn reattaches_when_shell_window_changed() {
        // 句柄碰巧仍有效（被复用）但 GetShellWindow 已是另一个窗口
        assert!(needs_reattach(PROGMAN, true, NEW_PROGMAN, PROGMAN));
    }

    #[test]
    fn reattaches_when_owner_was_cleared_elsewhere() {
        assert!(needs_reattach(PROGMAN, true, PROGMAN, 0));
    }

    #[test]
    fn tool_window_style_is_repaired_only_when_wiped() {
        assert!(
            !lacks_tool_window_style(EX_TOOLWINDOW_BIT),
            "正常：工具窗口"
        );
        assert!(
            !lacks_tool_window_style(EX_TOOLWINDOW_BIT | 0x100),
            "其它位不影响"
        );
        assert!(
            lacks_tool_window_style(EX_APPWINDOW_BIT),
            "被 apply_diff 改回 APPWINDOW"
        );
        assert!(lacks_tool_window_style(0), "TOOLWINDOW 丢了");
        assert!(lacks_tool_window_style(
            EX_TOOLWINDOW_BIT | EX_APPWINDOW_BIT
        ));
    }

    #[test]
    fn waits_while_shell_window_is_absent() {
        // Explorer 重启中：没有宿主可挂，不排队重挂
        assert!(!needs_reattach(PROGMAN, false, 0, PROGMAN));
        assert!(!needs_reattach(0, false, 0, 0));
    }

    // ---- AutoHideState 的模式切换辅助（原各命令里的内联状态操作）----

    fn hidden_state() -> AutoHideState {
        AutoHideState {
            enabled: true,
            hidden: true,
            docked_edge: Some(DockEdge::Left),
            monitor_bounds: Some(MONITOR),
            anchor_position: Some(WindowPosition { x: 0, y: 120 }),
            anchor_size: Some(WindowSize {
                width: 380,
                height: 600,
            }),
            ..AutoHideState::default()
        }
    }

    #[test]
    fn entering_fixed_mode_pulls_a_hidden_window_back_and_restarts_docking() {
        let mut state = hidden_state();
        let anchor = state.enter_fixed_mode(false).expect("藏起的窗口要拉回");
        assert_eq!((anchor.x, anchor.y), (0, 120));
        assert!(!state.enabled && !state.hidden);
        assert_eq!(state.docked_edge, None);
        assert!(state.enter_fixed_mode(true).is_none(), "没藏起来不用拉");
        assert!(state.enabled);
    }

    #[test]
    fn leaving_fixed_mode_resets_everything() {
        let mut state = hidden_state();
        state.keep_visible_until = Some(Instant::now());
        assert!(state.leave_fixed_mode().is_some());
        assert!(!state.enabled && !state.hidden);
        assert!(state.anchor_position.is_none() && state.keep_visible_until.is_none());
        assert!(state.cursor_inside_window);
    }

    #[test]
    fn toggling_auto_hide_off_returns_the_anchor_once() {
        let mut state = hidden_state();
        let anchor = state.set_enabled(false).expect("关掉时拉回");
        assert_eq!((anchor.x, anchor.y), (0, 120));
        assert!(!state.enabled && !state.hidden);
        assert!(state.anchor_size.is_none() && state.monitor_bounds.is_none());
        assert!(state.set_enabled(false).is_none());
        assert!(state.set_enabled(true).is_none());
        assert!(state.enabled);
    }

    #[test]
    fn tray_wake_restores_a_hidden_window_and_grants_a_grace_period() {
        let now = Instant::now();
        let mut state = hidden_state();
        let anchor = state.wake_from_tray(now).expect("藏起时先拉回锚点");
        assert_eq!((anchor.x, anchor.y), (0, 120));
        assert!(!state.hidden);
        assert_eq!(state.keep_visible_until, Some(now + TRAY_WAKE_KEEP_VISIBLE));

        let mut disabled = AutoHideState::default();
        assert!(disabled.wake_from_tray(now).is_none());
        assert!(
            disabled.keep_visible_until.is_none(),
            "没开贴边隐藏时不碰状态"
        );
    }

    #[test]
    fn persisted_geometry_uses_the_anchor_while_hidden() {
        let state = hidden_state();
        let persist = state.persist_state(
            WindowPosition { x: -376, y: 120 },
            WindowSize {
                width: 380,
                height: 600,
            },
        );
        assert_eq!((persist.position.x, persist.position.y), (0, 120));

        let visible = AutoHideState::default();
        let persist = visible.persist_state(
            WindowPosition { x: 5, y: 6 },
            WindowSize {
                width: 1,
                height: 2,
            },
        );
        assert_eq!((persist.position.x, persist.size.height), (5, 2));
    }

    #[test]
    fn stays_on_top_only_while_docked_awake_with_the_option_on() {
        let mut state = AutoHideState {
            enabled: true,
            docked_edge: Some(DockEdge::Top),
            ..AutoHideState::default()
        };
        assert!(state.should_stay_on_top(true));
        assert!(!state.should_stay_on_top(false));
        state.hidden = true;
        assert!(!state.should_stay_on_top(true));
        state.clear_runtime();
        assert!(!state.should_stay_on_top(true));
        assert!(state.enabled, "clear_runtime 不改开关");
    }

    #[test]
    fn reset_position_keeps_a_ten_percent_margin_in_logical_units() {
        let monitor = MonitorBounds {
            x: 0,
            y: 0,
            width: 2560,
            height: 1440,
        };
        let pos = reset_window_position(monitor, 1.25);
        assert_eq!((pos.x, pos.y), (204, 115));
        let offset = MonitorBounds {
            x: -1920,
            y: 0,
            width: 1920,
            height: 1080,
        };
        let pos = reset_window_position(offset, 0.0);
        assert_eq!((pos.x, pos.y), (-1920 + 192, 108));
    }

    #[test]
    fn tray_clicks_are_debounced_with_a_monotonic_clock() {
        let last = Mutex::new(None);
        let t0 = Instant::now();
        assert!(accept_tray_click(&last, t0), "第一下总是处理");
        assert!(
            !accept_tray_click(&last, t0 + Duration::from_millis(120)),
            "双击的第二下不重复唤起"
        );
        assert!(accept_tray_click(&last, t0 + TRAY_CLICK_DEBOUNCE));
        // 时间"倒退"（理论上 Instant 不会，但不能因此 panic 或永久拒绝）
        assert!(!accept_tray_click(&last, t0));
        assert!(accept_tray_click(
            &last,
            t0 + TRAY_CLICK_DEBOUNCE * 2 + Duration::from_millis(1)
        ));
    }
}
