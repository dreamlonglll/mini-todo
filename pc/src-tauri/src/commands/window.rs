use crate::db::{
    AppSettings, Database, SaveScreenConfigRequest, ScreenConfig, WindowPosition, WindowSize,
    DEFAULT_WINDOW_BG_ALPHA, DEFAULT_WINDOW_BG_COLOR,
};
use serde::Serialize;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use tauri::{Manager, State, WebviewWindow, Window};

#[cfg(target_os = "windows")]
use windows::Win32::Foundation::{HWND, POINT};
#[cfg(target_os = "windows")]
use windows::Win32::UI::WindowsAndMessaging::{
    GetClassNameW, GetCursorPos, GetShellWindow, GetWindowLongPtrW, GetWindowLongW, IsWindow,
    SetWindowLongPtrW, SetWindowLongW, SetWindowPos, ShowWindow, GWLP_HWNDPARENT, GWL_EXSTYLE,
    HWND_BOTTOM, HWND_NOTOPMOST, HWND_TOP, HWND_TOPMOST, SWP_FRAMECHANGED, SWP_NOACTIVATE,
    SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SW_RESTORE, SW_SHOW, WS_EX_APPWINDOW,
    WS_EX_TOOLWINDOW,
};

/// 全局固定模式状态
pub static IS_FIXED_MODE: AtomicBool = AtomicBool::new(false);

/// 全局桌面模式状态，与 `IS_FIXED_MODE` 互斥：进入其一即退出另一个。
///
/// 桌面模式 = 顶层窗口 + owner=Progman + tao `always_on_bottom`：窗口嵌在桌面图标之上、
/// 所有应用窗口之下，Win+D / Win+M 时随 Progman 一起被抬起而不是被最小化。
///
/// 对用户而言它不是第三种模式，而是"固定模式时，嵌入桌面中"开关（settings
/// `fixed_embed_desktop`）打开后的固定模式：前端 `applyFixedMode` 按开关决定调
/// `set_window_desktop_mode` 还是 `set_window_fixed_mode`，托盘「固定模式」在两种
/// 状态下都勾选。后端仍用两个原子量区分，因为 Win32 处理完全不同。
pub static IS_DESKTOP_MODE: AtomicBool = AtomicBool::new(false);

/// 桌面模式下挂上的 owner（Progman）句柄，0 表示尚未挂载。
///
/// 轮询线程拿它与 `GetShellWindow()` / 窗口当前 owner 比对：Explorer 重启后 Progman 是
/// 新句柄，旧 owner 失效，需要重新挂载。
///
/// 只有 `cfg(windows)` 的 attach / detach / tick 会读写它，其它平台上是死代码。
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
static DESKTOP_OWNER: AtomicIsize = AtomicIsize::new(0);

/// 贴边唤起时是否临时置顶。
///
/// 缓存成原子变量是因为 `tick_auto_hide` 跑在后台轮询线程里，拿不到 `State<Database>`。
/// 写入点：应用启动（`init_top_on_wake`）、`set_top_on_wake`、`set_window_fixed_mode`。
static TOP_ON_WAKE: AtomicBool = AtomicBool::new(true);

/// 固定模式下期望的置顶态。
///
/// 置顶改走 Win32 后，tao 内部记录的 `ALWAYS_ON_TOP` 会与真实 Z 序漂移，之后任何 tao 侧的
/// 窗口操作（如前端 `setResizable`）都可能按它的旧认知把置顶顺手撤掉。缓存期望值，
/// 由 `reassert_window_mode_state` 兜底补回。
static DESIRED_TOPMOST: AtomicBool = AtomicBool::new(false);

/// 托盘"固定模式"勾选菜单项引用，用于跨模块同步状态
static TRAY_TOGGLE_FIXED: OnceLock<tauri::menu::CheckMenuItem<tauri::Wry>> = OnceLock::new();

/// 检查当前是否处于固定模式
pub fn is_fixed_mode() -> bool {
    IS_FIXED_MODE.load(Ordering::SeqCst)
}

/// 检查当前是否处于桌面模式
pub fn is_desktop_mode() -> bool {
    IS_DESKTOP_MODE.load(Ordering::SeqCst)
}

/// 保存托盘"固定模式"勾选菜单项引用（在 setup 阶段调用一次）
pub fn set_tray_toggle_fixed_item(item: tauri::menu::CheckMenuItem<tauri::Wry>) {
    let _ = TRAY_TOGGLE_FIXED.set(item);
}

/// 同步更新托盘"固定模式"勾选状态。
///
/// 不依赖 CheckMenuItem 的自动翻转，按真实状态回写：普通固定与嵌入桌面（桌面模式）
/// 对托盘来说都是"固定模式"，两者之间互切时勾选保持不变。
fn sync_tray_fixed_checked(fixed: bool) {
    if let Some(item) = TRAY_TOGGLE_FIXED.get() {
        let _ = item.set_checked(fixed);
    }
}

/// 托盘"开机自启动"勾选菜单项引用，用于跨模块同步状态
static TRAY_AUTO_START: OnceLock<tauri::menu::CheckMenuItem<tauri::Wry>> = OnceLock::new();

/// 保存托盘"开机自启动"勾选菜单项引用（在 setup 阶段调用一次）
pub fn set_tray_auto_start_item(item: tauri::menu::CheckMenuItem<tauri::Wry>) {
    let _ = TRAY_AUTO_START.set(item);
}

/// 同步托盘"开机自启动"勾选状态
pub fn sync_tray_auto_start_checked(enabled: bool) {
    if let Some(item) = TRAY_AUTO_START.get() {
        let _ = item.set_checked(enabled);
    }
}

/// 设置窗口切换自启后调用：把托盘勾选同步为实际状态。
/// 托盘与设置面板是两个独立入口，不同步会出现"界面显示已开启、注册表实际已删"的状态反转。
#[tauri::command]
pub fn sync_auto_start_state(enabled: bool) {
    sync_tray_auto_start_checked(enabled);
}

fn get_auto_hide_enabled_value(db: &State<Database>) -> bool {
    db.with_connection(|conn| {
        let enabled: bool = conn
            .query_row(
                "SELECT value FROM settings WHERE key = 'auto_hide_enabled'",
                [],
                |row| {
                    let val: String = row.get(0)?;
                    Ok(val == "true")
                },
            )
            .unwrap_or(true);
        Ok(enabled)
    })
    .unwrap_or(true)
}

fn get_top_on_wake_value(db: &State<Database>) -> bool {
    db.with_connection(|conn| {
        let enabled: bool = conn
            .query_row(
                "SELECT value FROM settings WHERE key = 'top_on_wake'",
                [],
                |row| {
                    let val: String = row.get(0)?;
                    Ok(val == "true")
                },
            )
            .unwrap_or(true);
        Ok(enabled)
    })
    .unwrap_or(true)
}

/// 应用启动时把 `top_on_wake` 灌入原子缓存（轮询线程读不到数据库）
pub fn init_top_on_wake(db: &State<Database>) {
    TOP_ON_WAKE.store(get_top_on_wake_value(db), Ordering::SeqCst);
}

/// 窗口当前是否应保持置顶：仅在贴边唤起且开关打开时成立。
///
/// 托盘双击的临时置顶用它判断"到点该不该收回置顶"，避免误取消唤起态的置顶。
pub fn should_stay_on_top() -> bool {
    TOP_ON_WAKE.load(Ordering::SeqCst)
        && with_auto_hide_state(|state| state.enabled && !state.hidden && state.docked_edge.is_some())
}

/// 读取 settings 表的 text_theme
///
/// 注意该键描述的是文字颜色："light" 表示浅色文字，即前端的「深色主题开启」
fn get_text_theme_value(conn: &rusqlite::Connection) -> String {
    conn.query_row(
        "SELECT value FROM settings WHERE key = 'text_theme'",
        [],
        |row| row.get(0),
    )
    .unwrap_or_else(|_| "dark".to_string())
}

const EDGE_SNAP_THRESHOLD_PX: i32 = 10;
const EDGE_HIDE_DELAY: Duration = Duration::from_millis(420);
/// 托盘唤起后的保持显示宽限期
///
/// 没有它的话，鼠标不在窗口范围内，窗口会在 `EDGE_HIDE_DELAY` 后立刻缩回去，
/// 用户刚点完托盘就看它消失。宽限期内足够把鼠标移过去接管。
const TRAY_WAKE_KEEP_VISIBLE: Duration = Duration::from_secs(3);
/// 托盘唤起时临时置顶的持续时间
///
/// 置顶只是把窗口抬到最前的手段，到点就撤销——除非贴边唤起态本就需要保持置顶。
const TRAY_WAKE_TOPMOST_DURATION: Duration = Duration::from_millis(600);
const HIDDEN_VISIBLE_STRIP_PX: i32 = 4;
const WAKE_HOTZONE_WIDTH_PX: i32 = 2;
const WAKE_RANGE_PADDING_PX: i32 = 40;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DockEdge {
    Left,
    Right,
    Top,
    Bottom,
}

#[derive(Debug, Clone, Copy)]
struct MonitorBounds {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
}

impl MonitorBounds {
    fn right(self) -> i32 {
        self.x + self.width
    }

    fn bottom(self) -> i32 {
        self.y + self.height
    }
}

#[derive(Debug, Clone, Copy)]
struct WindowRect {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
}

impl WindowRect {
    fn right(self) -> i32 {
        self.x + self.width
    }

    fn bottom(self) -> i32 {
        self.y + self.height
    }
}

#[derive(Debug)]
struct AutoHideState {
    enabled: bool,
    hidden: bool,
    cursor_inside_window: bool,
    docked_edge: Option<DockEdge>,
    monitor_bounds: Option<MonitorBounds>,
    anchor_position: Option<WindowPosition>,
    anchor_size: Option<WindowSize>,
    edge_stick_started_at: Option<Instant>,
    /// 托盘唤起后的免隐藏截止时间
    keep_visible_until: Option<Instant>,
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

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowPersistState {
    pub position: WindowPosition,
    pub size: WindowSize,
}

#[derive(Debug, Clone)]
enum AutoHideTransition {
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

static AUTO_HIDE_STATE: OnceLock<Mutex<AutoHideState>> = OnceLock::new();

fn with_auto_hide_state<R>(f: impl FnOnce(&mut AutoHideState) -> R) -> R {
    let mutex = AUTO_HIDE_STATE.get_or_init(|| Mutex::new(AutoHideState::default()));
    let mut state = mutex.lock().unwrap_or_else(|e| e.into_inner());
    f(&mut state)
}

fn clear_auto_hide_runtime_state() {
    with_auto_hide_state(|state| {
        state.hidden = false;
        state.cursor_inside_window = true;
        state.docked_edge = None;
        state.monitor_bounds = None;
        state.anchor_position = None;
        state.anchor_size = None;
        state.edge_stick_started_at = None;
        state.keep_visible_until = None;
    });
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

fn detect_docked_edge(rect: WindowRect, monitor: MonitorBounds) -> Option<DockEdge> {
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

fn calc_anchor_and_hidden_position(
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

#[cfg(target_os = "windows")]
fn get_cursor_position() -> Option<(i32, i32)> {
    unsafe {
        let mut point = POINT::default();
        if GetCursorPos(&mut point).is_ok() {
            Some((point.x, point.y))
        } else {
            None
        }
    }
}

#[cfg(not(target_os = "windows"))]
fn get_cursor_position() -> Option<(i32, i32)> {
    None
}

fn get_window_rect(window: &WebviewWindow) -> Option<WindowRect> {
    let pos = window.outer_position().ok()?;
    let size = window.outer_size().ok()?;
    Some(WindowRect {
        x: pos.x,
        y: pos.y,
        width: size.width as i32,
        height: size.height as i32,
    })
}

fn get_monitor_bounds(window: &WebviewWindow) -> Option<MonitorBounds> {
    let monitor = window
        .current_monitor()
        .ok()
        .flatten()
        .or_else(|| window.primary_monitor().ok().flatten())?;

    Some(MonitorBounds {
        x: monitor.position().x,
        y: monitor.position().y,
        width: monitor.size().width as i32,
        height: monitor.size().height as i32,
    })
}

fn should_wake_hidden_window(cursor_x: i32, cursor_y: i32, state: &AutoHideState) -> bool {
    let (Some(edge), Some(monitor), Some(anchor), Some(size)) = (
        state.docked_edge,
        state.monitor_bounds,
        state.anchor_position.as_ref(),
        state.anchor_size.as_ref(),
    ) else {
        return false;
    };

    let vertical_min = anchor.y - WAKE_RANGE_PADDING_PX;
    let vertical_max = anchor.y + size.height as i32 + WAKE_RANGE_PADDING_PX;
    let horizontal_min = anchor.x - WAKE_RANGE_PADDING_PX;
    let horizontal_max = anchor.x + size.width as i32 + WAKE_RANGE_PADDING_PX;

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

fn evaluate_auto_hide_transition(
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

    let cursor_inside = if let Some((cursor_x, cursor_y)) = cursor {
        point_in_rect(cursor_x, cursor_y, rect)
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

/// 固定模式守护：窗口一旦被最小化就立刻还原。
///
/// 固定模式下窗口不在任务栏也不参与 Alt+Tab，被 Win+D / Win+M 最小化后就再没有入口点回来。
/// 还原走 `unminimize_window` / `show_window`，避免 tao 的样式覆写让任务栏图标闪现。
pub fn restore_if_minimized(window: &WebviewWindow) {
    if !window.is_minimized().unwrap_or(false) {
        return;
    }
    unminimize_window(window);
    show_window(window);
}

/// 固定模式轮询：处理贴边自动隐藏与边缘唤起
pub fn tick_auto_hide(window: &WebviewWindow) {
    let cursor = get_cursor_position();
    let Some(rect) = get_window_rect(window) else {
        return;
    };
    let Some(monitor) = get_monitor_bounds(window) else {
        return;
    };

    let transition = with_auto_hide_state(|state| {
        evaluate_auto_hide_transition(state, rect, monitor, cursor, Instant::now())
    });

    match transition {
        AutoHideTransition::None => {}
        AutoHideTransition::Hide {
            hidden,
            anchor,
            size,
            edge,
            monitor,
        } => {
            let result = window.set_position(tauri::Position::Physical(tauri::PhysicalPosition {
                x: hidden.x,
                y: hidden.y,
            }));
            if result.is_err() {
                with_auto_hide_state(|state| {
                    state.hidden = false;
                    state.anchor_position = Some(anchor);
                    state.anchor_size = Some(size);
                    state.docked_edge = Some(edge);
                    state.monitor_bounds = Some(monitor);
                });
            } else {
                // 收回时一并撤销置顶，否则藏起来的窗口会一直压在其它窗口之上
                set_window_always_on_top(window, false);
                // 兜底：set_position 在窗口处于最大化态时会经 apply_diff 重写 ex style
                reassert_window_mode_state(window);
            }
        }
        AutoHideTransition::Restore { anchor } => {
            let result = window.set_position(tauri::Position::Physical(tauri::PhysicalPosition {
                x: anchor.x,
                y: anchor.y,
            }));
            if result.is_err() {
                with_auto_hide_state(|state| {
                    state.hidden = true;
                });
            } else if TOP_ON_WAKE.load(Ordering::SeqCst) {
                // 唤起时置顶：只 set_position 不改 Z 序的话，窗口会被最大化/全屏窗口整个盖住
                set_window_always_on_top(window, true);
                // 兜底：set_position 在窗口处于最大化态时会经 apply_diff 重写 ex style
                reassert_window_mode_state(window);
            }
        }
    }
}

#[tauri::command]
pub fn get_settings(db: State<Database>) -> Result<AppSettings, String> {
    db.with_connection(|conn| {
        let is_fixed: bool = conn
            .query_row(
                "SELECT value FROM settings WHERE key = 'is_fixed'",
                [],
                |row| {
                    let val: String = row.get(0)?;
                    Ok(val == "true")
                },
            )
            .unwrap_or(false);

        let fixed_embed_desktop: bool = conn
            .query_row(
                "SELECT value FROM settings WHERE key = 'fixed_embed_desktop'",
                [],
                |row| {
                    let val: String = row.get(0)?;
                    Ok(val == "true")
                },
            )
            .unwrap_or(false);

        let window_position: Option<WindowPosition> = conn
            .query_row(
                "SELECT value FROM settings WHERE key = 'window_position'",
                [],
                |row| {
                    let val: String = row.get(0)?;
                    Ok(serde_json::from_str(&val).ok())
                },
            )
            .unwrap_or(None);

        let window_size: Option<WindowSize> = conn
            .query_row(
                "SELECT value FROM settings WHERE key = 'window_size'",
                [],
                |row| {
                    let val: String = row.get(0)?;
                    Ok(serde_json::from_str(&val).ok())
                },
            )
            .unwrap_or(None);

        let auto_hide_enabled: bool = conn
            .query_row(
                "SELECT value FROM settings WHERE key = 'auto_hide_enabled'",
                [],
                |row| {
                    let val: String = row.get(0)?;
                    Ok(val == "true")
                },
            )
            .unwrap_or(true);

        let top_on_wake: bool = conn
            .query_row(
                "SELECT value FROM settings WHERE key = 'top_on_wake'",
                [],
                |row| {
                    let val: String = row.get(0)?;
                    Ok(val == "true")
                },
            )
            .unwrap_or(true);

        let window_bg_color: String = conn
            .query_row(
                "SELECT value FROM settings WHERE key = 'window_bg_color'",
                [],
                |row| row.get(0),
            )
            .unwrap_or_else(|_| DEFAULT_WINDOW_BG_COLOR.to_string());

        let window_bg_alpha: f64 = conn
            .query_row(
                "SELECT value FROM settings WHERE key = 'window_bg_alpha'",
                [],
                |row| {
                    let val: String = row.get(0)?;
                    Ok(val.parse::<f64>().unwrap_or(DEFAULT_WINDOW_BG_ALPHA))
                },
            )
            .unwrap_or(DEFAULT_WINDOW_BG_ALPHA);

        let text_theme = get_text_theme_value(conn);

        let show_calendar: bool = conn
            .query_row(
                "SELECT value FROM settings WHERE key = 'show_calendar'",
                [],
                |row| {
                    let val: String = row.get(0)?;
                    Ok(val == "true")
                },
            )
            .unwrap_or(false);

        let view_mode: String = conn
            .query_row(
                "SELECT value FROM settings WHERE key = 'view_mode'",
                [],
                |row| row.get(0),
            )
            .unwrap_or_else(|_| "list".to_string());

        let notification_type: String = conn
            .query_row(
                "SELECT value FROM settings WHERE key = 'notification_type'",
                [],
                |row| row.get(0),
            )
            .unwrap_or_else(|_| "system".to_string());

        Ok(AppSettings {
            is_fixed,
            fixed_embed_desktop,
            window_position,
            window_size,
            auto_hide_enabled,
            top_on_wake,
            window_bg_color,
            window_bg_alpha,
            text_theme,
            show_calendar,
            view_mode,
            notification_type,
        })
    })
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn save_settings(db: State<Database>, settings: AppSettings) -> Result<(), String> {
    // 轮询线程读的是原子缓存，落库的同时要刷新，否则本次会话内改动不生效
    TOP_ON_WAKE.store(settings.top_on_wake, Ordering::SeqCst);
    db.with_connection(|conn| {
        // 保存 is_fixed
        conn.execute(
            "INSERT OR REPLACE INTO settings (key, value, updated_at) VALUES ('is_fixed', ?, datetime('now', 'localtime'))",
            [if settings.is_fixed { "true" } else { "false" }],
        )?;

        // 保存 fixed_embed_desktop（前端 saveWindowState 前必须先 load 真值，否则会刷成默认 false）
        conn.execute(
            "INSERT OR REPLACE INTO settings (key, value, updated_at) VALUES ('fixed_embed_desktop', ?, datetime('now', 'localtime'))",
            [if settings.fixed_embed_desktop { "true" } else { "false" }],
        )?;

        // 保存窗口位置
        if let Some(pos) = &settings.window_position {
            let pos_json = serde_json::to_string(pos).unwrap_or_default();
            conn.execute(
                "INSERT OR REPLACE INTO settings (key, value, updated_at) VALUES ('window_position', ?, datetime('now', 'localtime'))",
                [&pos_json],
            )?;
        }

        // 保存窗口尺寸
        if let Some(size) = &settings.window_size {
            let size_json = serde_json::to_string(size).unwrap_or_default();
            conn.execute(
                "INSERT OR REPLACE INTO settings (key, value, updated_at) VALUES ('window_size', ?, datetime('now', 'localtime'))",
                [&size_json],
            )?;
        }

        // 保存贴边自动隐藏设置
        conn.execute(
            "INSERT OR REPLACE INTO settings (key, value, updated_at) VALUES ('auto_hide_enabled', ?, datetime('now', 'localtime'))",
            [if settings.auto_hide_enabled { "true" } else { "false" }],
        )?;

        // 保存唤起置顶设置
        conn.execute(
            "INSERT OR REPLACE INTO settings (key, value, updated_at) VALUES ('top_on_wake', ?, datetime('now', 'localtime'))",
            [if settings.top_on_wake { "true" } else { "false" }],
        )?;

        // 保存窗口底色与透明度
        conn.execute(
            "INSERT OR REPLACE INTO settings (key, value, updated_at) VALUES ('window_bg_color', ?, datetime('now', 'localtime'))",
            [&settings.window_bg_color],
        )?;
        conn.execute(
            "INSERT OR REPLACE INTO settings (key, value, updated_at) VALUES ('window_bg_alpha', ?, datetime('now', 'localtime'))",
            [&settings.window_bg_alpha.to_string()],
        )?;

        // 保存文本主题
        conn.execute(
            "INSERT OR REPLACE INTO settings (key, value, updated_at) VALUES ('text_theme', ?, datetime('now', 'localtime'))",
            [&settings.text_theme],
        )?;

        Ok(())
    })
    .map_err(|e| e.to_string())
}

/// 单独保存文本主题
///
/// 供设置窗口调用：仅写 text_theme 键，不触碰窗口位置/尺寸/固定模式
#[tauri::command]
pub fn set_text_theme(db: State<Database>, theme: String) -> Result<(), String> {
    db.with_connection(|conn| {
        conn.execute(
            "INSERT OR REPLACE INTO settings (key, value, updated_at) VALUES ('text_theme', ?, datetime('now', 'localtime'))",
            [&theme],
        )?;
        Ok(())
    })
    .map_err(|e| e.to_string())
}

/// 读取文本主题（默认 "dark"，注意该值语义与深色主题开关相反，见 appStore.loadDarkTheme）
#[tauri::command]
pub fn get_text_theme(db: State<Database>) -> Result<String, String> {
    db.with_connection(|conn| Ok(get_text_theme_value(conn)))
        .map_err(|e| e.to_string())
}

/// 切换主窗口的固定模式
///
/// 同样固定取主窗口：固定模式的位置锁定与 WS_EX_TOOLWINDOW 样式只对主窗口有意义，
/// 按调用方窗口取值会在其他 WebView 触发时改错窗口
/// 按固定模式设置/恢复主窗口的任务栏样式。
///
/// 固定模式加 `WS_EX_TOOLWINDOW`、去 `WS_EX_APPWINDOW`，让窗口既不进任务栏也不参与 Alt+Tab；
/// 样式改完补一次 `SWP_FRAMECHANGED`，任务栏才会对可见窗口的样式变更立刻生效。
#[cfg(target_os = "windows")]
fn window_hwnd(window: &WebviewWindow) -> Option<HWND> {
    use raw_window_handle::HasWindowHandle;

    let handle = window.window_handle().ok()?;
    let raw_window_handle::RawWindowHandle::Win32(win32_handle) = handle.as_raw() else {
        return None;
    };
    Some(HWND(win32_handle.hwnd.get() as *mut _))
}

#[cfg(target_os = "windows")]
fn apply_fixed_ex_style(window: &WebviewWindow, fixed: bool) {
    let Some(hwnd) = window_hwnd(window) else {
        return;
    };

    unsafe {
        let ex_style = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
        let new_style = if fixed {
            (ex_style | WS_EX_TOOLWINDOW.0) & !WS_EX_APPWINDOW.0
        } else {
            (ex_style & !WS_EX_TOOLWINDOW.0) | WS_EX_APPWINDOW.0
        };
        if new_style != ex_style {
            SetWindowLongW(hwnd, GWL_EXSTYLE, new_style as i32);
            let _ = SetWindowPos(
                hwnd,
                HWND(std::ptr::null_mut()),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
            );
        }
    }
}

/// 固定模式下用 Win32 直接换 Z 序，返回 true 表示已处理、无需再走 tao。
///
/// tao 的 `set_always_on_top` 会触发 `WindowFlags::apply_diff`：它按 tao 自己记录的 flags
/// 重算并**整体覆写** `GWL_EXSTYLE`（tao-0.34.5 `window_state.rs:439-440`），手动打的
/// `WS_EX_TOOLWINDOW` 当场被抹掉、`WS_EX_APPWINDOW` 被加回，紧跟的 `SWP_FRAMECHANGED`
/// 让 shell 立即生效——任务栏按钮就此出现。事后补样式只能让它闪一下再消失，消不掉中间态。
///
/// 直接 `SetWindowPos` 换 Z 序只翻 `WS_EX_TOPMOST` 位，不触碰 ex style 的其余部分，
/// 也不带 `SWP_FRAMECHANGED`，因而全程不会有任务栏按钮出现。
#[cfg(target_os = "windows")]
fn win32_set_topmost(window: &WebviewWindow, on: bool) -> bool {
    if !is_fixed_mode() {
        return false;
    }
    let Some(hwnd) = window_hwnd(window) else {
        return false;
    };
    unsafe {
        let _ = SetWindowPos(
            hwnd,
            if on { HWND_TOPMOST } else { HWND_NOTOPMOST },
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        );
    }
    true
}

#[cfg(not(target_os = "windows"))]
fn win32_set_topmost(_window: &WebviewWindow, _on: bool) -> bool {
    false
}

/// 固定 / 桌面模式下用 Win32 直接显示/还原窗口，返回 true 表示已处理。
///
/// 与置顶同理：`show` / `unminimize` 也走 tao 的 flag 通路，会覆写 `GWL_EXSTYLE`。
/// `ShowWindow` 不碰 ex style，且 tao 的 `MINIMIZED` flag 由窗口过程按 `WM_SIZE` 自行同步
/// （tao-0.34.5 `event_loop.rs:1286`），绕过去不会让 tao 的状态漂移。
/// 桌面模式同样带 `WS_EX_TOOLWINDOW`，走 tao 通路会让任务栏图标闪现，因此一并绕开。
#[cfg(target_os = "windows")]
fn win32_show(window: &WebviewWindow, restore: bool) -> bool {
    if !is_fixed_mode() && !is_desktop_mode() {
        return false;
    }
    let Some(hwnd) = window_hwnd(window) else {
        return false;
    };
    unsafe {
        let _ = ShowWindow(hwnd, if restore { SW_RESTORE } else { SW_SHOW });
    }
    true
}

#[cfg(not(target_os = "windows"))]
fn win32_show(_window: &WebviewWindow, _restore: bool) -> bool {
    false
}

/// 置顶/取消置顶。固定模式下绕开 tao 的 flag 通路，避免任务栏图标闪现。
fn set_window_always_on_top(window: &WebviewWindow, on: bool) {
    DESIRED_TOPMOST.store(on, Ordering::SeqCst);
    if win32_set_topmost(window, on) {
        return;
    }
    let _ = window.set_always_on_top(on);
}

/// 显示窗口。固定模式下绕开 tao 的 flag 通路。
fn show_window(window: &WebviewWindow) {
    if win32_show(window, false) {
        return;
    }
    let _ = window.show();
}

/// 从最小化还原窗口。固定模式下绕开 tao 的 flag 通路。
///
/// 未最小化时直接返回：`SW_RESTORE` 对最大化窗口会执行"还原大小"、对普通窗口会激活它，
/// 而 tao 的 `unminimize` 在 flag 无变化时是彻底的 no-op。不加这道闸，
/// 托盘唤起会顺带把窗口从最大化缩回去。
fn unminimize_window(window: &WebviewWindow) {
    if !window.is_minimized().unwrap_or(false) {
        return;
    }
    if win32_show(window, true) {
        return;
    }
    let _ = window.unminimize();
}

// ============ 桌面模式（owner=Progman + always_on_bottom） ============

/// 桌面宿主窗口的类名是否合格。
///
/// `GetShellWindow()` 在 Explorer 作为 shell 时返回类名为 `Progman` 的 "Program Manager"；
/// Win11 24H2+ 按 Win+D 时被抬到最前的正是它，以它为 owner 的窗口会被系统一并抬起。
/// 其它类名（如第三方 shell）说明桌面结构未知，Win+D 免疫不保证。
///
/// 纯函数不带 `cfg(windows)` 是为了让单元测试在任何平台都能跑；调用方只有 Windows 的
/// `desktop_host`，其它平台上它是死代码。
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn is_desktop_host_class(name: &str) -> bool {
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
///
/// 同 `is_desktop_host_class`：纯函数留给测试，非 Windows 平台上没有调用方。
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn needs_reattach(
    cached_owner: isize,
    owner_alive: bool,
    shell_now: isize,
    current_owner: isize,
) -> bool {
    if shell_now == 0 {
        return false;
    }
    cached_owner == 0
        || !owner_alive
        || shell_now != cached_owner
        || current_owner != cached_owner
}

#[cfg(target_os = "windows")]
fn window_class_name(hwnd: HWND) -> String {
    let mut buf = [0u16; 64];
    let len = unsafe { GetClassNameW(hwnd, &mut buf) };
    if len <= 0 {
        return String::new();
    }
    String::from_utf16_lossy(&buf[..len as usize])
}

/// 桌面宿主：`GetShellWindow()` 返回的 Progman。
///
/// 类名不是 Progman 时只记录日志、仍返回句柄（PRD Decision 第 6 条）：Win10 / Win11 ≤23H2
/// 或第三方 shell 上 Win+D 期间可能被桌面盖住，但"不最小化、不进任务栏、压在底部"
/// 这几项照常成立。只有 shell 窗口根本不存在（Explorer 未启动）才返回 None。
#[cfg(target_os = "windows")]
fn desktop_host() -> Option<HWND> {
    let shell = unsafe { GetShellWindow() };
    if shell.is_invalid() {
        eprintln!("[desktop] GetShellWindow 为空，桌面宿主暂不可用");
        return None;
    }
    let class_name = window_class_name(shell);
    if !is_desktop_host_class(&class_name) {
        eprintln!(
            "[desktop] shell 窗口类名为 {:?}（非 Progman），仍以其为 owner",
            class_name
        );
    }
    Some(shell)
}

/// 把主窗口挂到桌面上：owner=Progman + 工具窗口样式 + 压到 Z 序底部。幂等，可反复调用。
///
/// 必须在主线程队列里、排在 tao 的 `set_minimizable` / `set_always_on_bottom` 之后执行：
/// 那两步都会经 `apply_diff` 整体重写 `GWL_EXSTYLE`，把这里打的 `WS_EX_TOOLWINDOW` 抹掉。
/// `GWLP_HWNDPARENT` 则不在 tao 的覆写范围内（它只重写 `GWL_STYLE` / `GWL_EXSTYLE`）。
///
/// `HWND_BOTTOM` 在有 owner=Progman 时的实际效果是"紧贴 Progman 之上"——系统保证 owned
/// 窗口永远在 owner 之上，于是窗口落在桌面图标之上、所有普通窗口之下。之后任何
/// `SetWindowPos`（包括点击激活）都会被 tao 的 `ALWAYS_ON_BOTTOM` 钩子改回 `HWND_BOTTOM`。
#[cfg(target_os = "windows")]
fn desktop_attach(window: &WebviewWindow) {
    let Some(hwnd) = window_hwnd(window) else {
        return;
    };
    let Some(host) = desktop_host() else {
        return;
    };
    let host_raw = host.0 as isize;

    unsafe {
        let owner_changed = GetWindowLongPtrW(hwnd, GWLP_HWNDPARENT) != host_raw;
        if owner_changed {
            SetWindowLongPtrW(hwnd, GWLP_HWNDPARENT, host_raw);
        }
        // 不进任务栏、不参与 Alt+Tab，与固定模式共用同一套样式
        apply_fixed_ex_style(window, true);
        // owner 刚换过才需要 SWP_FRAMECHANGED 让 shell 重新认识这个窗口；
        // 幂等重申时省掉，避免每次 reassert 都触发一轮 WM_NCCALCSIZE
        let mut flags = SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE;
        if owner_changed {
            flags |= SWP_FRAMECHANGED;
        }
        let _ = SetWindowPos(hwnd, HWND_BOTTOM, 0, 0, 0, 0, flags);
    }

    DESKTOP_OWNER.store(host_raw, Ordering::SeqCst);
}

/// 把主窗口从桌面上摘下来：清 owner、按当前模式恢复任务栏样式、抬回普通 Z 序。
///
/// 任务栏样式按执行时的 `is_fixed_mode()` 决定，而不是无条件恢复：直接从桌面模式切到
/// 固定模式时，`IS_FIXED_MODE` 在本闭包执行前已置 true，样式保持 `WS_EX_TOOLWINDOW`
/// 不动，任务栏图标不会闪一下再消失。
///
/// 必须排在 tao 的 `set_always_on_bottom(false)` 之后：否则 `HWND_TOP` 会被
/// `ALWAYS_ON_BOTTOM` 钩子改回 `HWND_BOTTOM`，窗口留在最底层。
#[cfg(target_os = "windows")]
fn desktop_detach(window: &WebviewWindow) {
    let Some(hwnd) = window_hwnd(window) else {
        return;
    };
    unsafe {
        SetWindowLongPtrW(hwnd, GWLP_HWNDPARENT, 0);
        apply_fixed_ex_style(window, is_fixed_mode());
        let _ = SetWindowPos(
            hwnd,
            HWND_TOP,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_FRAMECHANGED,
        );
    }
    DESKTOP_OWNER.store(0, Ordering::SeqCst);
}

/// 桌面模式轮询：宿主失效（Explorer 重启 / owner 被清）时重新挂载。
///
/// 只读几个 Win32 状态，代价可忽略；真正的重挂要改样式与 owner，排进主线程队列执行，
/// 与 `reassert_window_mode_state` 同一套语义。闭包里再核对一次模式：排队期间用户可能
/// 已退出桌面模式，不能把刚摘掉的 owner 又挂回去。
pub fn tick_desktop_mode(window: &WebviewWindow) {
    #[cfg(target_os = "windows")]
    {
        let Some(hwnd) = window_hwnd(window) else {
            return;
        };
        let cached_owner = DESKTOP_OWNER.load(Ordering::SeqCst);
        let (owner_alive, shell_now, current_owner) = unsafe {
            let alive = cached_owner != 0 && IsWindow(HWND(cached_owner as *mut _)).as_bool();
            (
                alive,
                GetShellWindow().0 as isize,
                GetWindowLongPtrW(hwnd, GWLP_HWNDPARENT),
            )
        };
        if needs_reattach(cached_owner, owner_alive, shell_now, current_owner) {
            let w = window.clone();
            let _ = window.run_on_main_thread(move || {
                if is_desktop_mode() {
                    desktop_attach(&w);
                }
            });
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = window;
    }
}

/// 在 tao 的窗口操作之后重申当前窗口模式的状态（任务栏样式 + 置顶态 / owner + 底部 Z 序）。
///
/// 主路径的置顶/显示已改走 Win32，正常不会再破坏样式。但仍有 tao 侧操作会走 `apply_diff`
/// 整体重写 `GWL_EXSTYLE`（典型是前端的 `setResizable`），把 `WS_EX_TOOLWINDOW` 抹掉、
/// 并按 tao 内部那份已漂移的 `ALWAYS_ON_TOP` 把置顶撤掉。此处作为兜底把两者都补回来：
/// - 固定模式：补 TOOLWINDOW + 期望的置顶态
/// - 桌面模式：幂等地重跑 `desktop_attach`（TOOLWINDOW + owner + HWND_BOTTOM）
/// - 普通模式：确保任务栏样式已恢复
///
/// 这些操作都是投递到主线程消息队列异步执行的，补状态必须排进同一条队列，
/// 才能保证发生在它们之后且无竞态；闭包里读的是执行时刻的模式，不是排队时刻的。
fn reassert_window_mode_state(window: &WebviewWindow) {
    #[cfg(target_os = "windows")]
    {
        let w = window.clone();
        let _ = window.run_on_main_thread(move || {
            if is_desktop_mode() {
                desktop_attach(&w);
                return;
            }
            let fixed = is_fixed_mode();
            // 只翻 TOOLWINDOW / APPWINDOW 两位，带 SWP_NOZORDER，不会动到 Z 序
            apply_fixed_ex_style(&w, fixed);
            if fixed {
                win32_set_topmost(&w, DESIRED_TOPMOST.load(Ordering::SeqCst));
            }
        });
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = window;
    }
}

/// 退出桌面模式的公共步骤（`set_window_desktop_mode(false)` 与进入固定模式时共用）。
///
/// 顺序不能换：先清全局状态（排队中的 tick / reassert 闭包据此跳过重挂），再走 tao 通路
/// 撤 `always_on_bottom` / 恢复可最小化（两者都会 `apply_diff`），最后排队 `desktop_detach`
/// 抬回普通 Z 序。
fn leave_desktop_mode(window: &WebviewWindow) {
    IS_DESKTOP_MODE.store(false, Ordering::SeqCst);
    let _ = window.set_always_on_bottom(false);
    let _ = window.set_minimizable(true);
    #[cfg(target_os = "windows")]
    {
        let w = window.clone();
        let _ = window.run_on_main_thread(move || desktop_detach(&w));
    }
    // 直接切到普通固定模式时，set_window_fixed_mode 末尾会再按 true 回写
    sync_tray_fixed_checked(false);
}

/// 切换主窗口的桌面模式
///
/// 固定取主窗口，理由同 `set_window_fixed_mode`。与固定模式互斥：进入桌面模式先按退出
/// 固定模式的步骤清理（撤置顶、复位贴边状态、藏起的窗口挪回锚点）。
///
/// 进入顺序：tao 的 `set_minimizable(false)` / `set_always_on_bottom(true)` 在前——它们经
/// `apply_diff` 整体重写 `GWL_EXSTYLE`——然后才置 `IS_DESKTOP_MODE`、排队 `desktop_attach`
/// 打 owner 与 TOOLWINDOW，保证样式不被随后的覆写抹掉。
#[tauri::command]
pub fn set_window_desktop_mode(
    app_handle: tauri::AppHandle,
    db: State<Database>,
    enabled: bool,
) -> Result<(), String> {
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (app_handle, db);
        // 前端 applyNormalMode 会无条件调一次 enabled=false，非 Windows 上当作 no-op，
        // 否则每次切回普通模式控制台都多一条报错
        if enabled {
            Err("桌面模式仅支持 Windows".to_string())
        } else {
            Ok(())
        }
    }

    #[cfg(target_os = "windows")]
    {
        let window = app_handle
            .get_webview_window("main")
            .ok_or_else(|| "主窗口不存在".to_string())?;

        if !enabled {
            if is_desktop_mode() {
                leave_desktop_mode(&window);
            }
            // 兜底：前端紧跟着的 setResizable 会重写 ex style，排在它之后补任务栏样式
            reassert_window_mode_state(&window);
            return Ok(());
        }

        if is_fixed_mode() {
            // 退出固定模式：撤置顶、贴边状态复位、藏起的窗口挪回锚点。
            // 撤置顶要在 IS_FIXED_MODE 置 false 之前——此时才走 Win32 的 HWND_NOTOPMOST；
            // 置 false 后会落到 tao 通路，而 tao 的 ALWAYS_ON_TOP 本就是 false，撤销成了 no-op
            set_window_always_on_top(&window, false);
            IS_FIXED_MODE.store(false, Ordering::SeqCst);
            let restore_position = with_auto_hide_state(|state| {
                let restore = if state.hidden {
                    state.anchor_position.clone()
                } else {
                    None
                };
                *state = AutoHideState::default();
                restore
            });
            if let Some(anchor) = restore_position {
                let _ = window.set_position(tauri::Position::Physical(tauri::PhysicalPosition {
                    x: anchor.x,
                    y: anchor.y,
                }));
            }
        }
        // 桌面模式不做贴边隐藏，但 top_on_wake 的缓存仍按库里的真值刷新，
        // 之后切回固定模式时 tick_auto_hide 读到的才是当前设置
        TOP_ON_WAKE.store(get_top_on_wake_value(&db), Ordering::SeqCst);

        // tao 通路在前：去 WS_MINIMIZEBOX（Win+D / Win+M 不再最小化它）、常驻 Z 序底部
        let _ = window.set_minimizable(false);
        let _ = window.set_always_on_bottom(true);
        // 状态位在这两步之后再翻：轮询线程一看到 IS_DESKTOP_MODE 就可能排队 desktop_attach，
        // 若它抢在 set_minimizable 之前入队，随后的 apply_diff 会把刚打的 TOOLWINDOW 抹掉、
        // 任务栏图标闪一下；放在后面入队的 attach 一定排在两次 apply_diff 之后
        IS_DESKTOP_MODE.store(true, Ordering::SeqCst);
        // 再排队 Win32：owner=Progman + TOOLWINDOW + HWND_BOTTOM
        reassert_window_mode_state(&window);

        // 嵌入桌面对托盘来说仍是"固定模式"
        sync_tray_fixed_checked(true);
        Ok(())
    }
}

#[tauri::command]
pub fn set_window_fixed_mode(
    app_handle: tauri::AppHandle,
    db: State<Database>,
    fixed: bool,
) -> Result<(), String> {
    let window = app_handle
        .get_webview_window("main")
        .ok_or_else(|| "主窗口不存在".to_string())?;

    // 两种模式互斥：进固定模式前先摘掉桌面模式的 owner / always_on_bottom。
    // 下方的 reassert 排在 desktop_detach 之后，会把 TOOLWINDOW 样式与置顶态补成固定模式的
    if fixed && is_desktop_mode() {
        leave_desktop_mode(&window);
    }

    // 更新全局固定模式状态
    IS_FIXED_MODE.store(fixed, Ordering::SeqCst);
    let auto_hide_enabled = get_auto_hide_enabled_value(&db);
    TOP_ON_WAKE.store(get_top_on_wake_value(&db), Ordering::SeqCst);

    // 退出固定模式时贴边逻辑随之停用，遗留的置顶要一并撤销。
    // 此处 IS_FIXED_MODE 已置 false，走的是 tao 通路——退出固定模式本就该恢复任务栏图标
    if !fixed {
        set_window_always_on_top(&window, false);
    }

    let restore_position = with_auto_hide_state(|state| {
        if fixed {
            state.enabled = auto_hide_enabled;
            state.hidden = false;
            state.docked_edge = None;
            state.edge_stick_started_at = None;
            None
        } else {
            let restore = if state.hidden {
                state.anchor_position.clone()
            } else {
                None
            };
            *state = AutoHideState::default();
            restore
        }
    });

    if let Some(anchor) = restore_position {
        let _ = window.set_position(tauri::Position::Physical(tauri::PhysicalPosition {
            x: anchor.x,
            y: anchor.y,
        }));
    }

    // 窗口状态排队补写：既覆盖此处的模式切换，也保证排在前端刚发出的
    // setResizable 等 tao 操作之后，不被其样式重写顶掉
    reassert_window_mode_state(&window);

    sync_tray_fixed_checked(fixed);

    Ok(())
}

/// 读取"固定模式时，嵌入桌面中"开关；缺键（旧库）按 false。
fn get_fixed_embed_desktop_value(db: &State<Database>) -> bool {
    db.with_connection(|conn| {
        let enabled: bool = conn
            .query_row(
                "SELECT value FROM settings WHERE key = 'fixed_embed_desktop'",
                [],
                |row| {
                    let val: String = row.get(0)?;
                    Ok(val == "true")
                },
            )
            .unwrap_or(false);
        Ok(enabled)
    })
    .unwrap_or(false)
}

#[tauri::command]
pub fn get_fixed_embed_desktop(db: State<Database>) -> Result<bool, String> {
    Ok(get_fixed_embed_desktop_value(&db))
}

/// 切换"固定模式时，嵌入桌面中"（设置窗口调用）。
///
/// 只写这一个 settings 键，不碰窗口几何与 `is_fixed`（设置窗口里绝不能连带
/// `saveWindowState`，见 spec state-management）。当前已处于固定模式时立即生效：
/// 开 → 普通固定切到嵌入桌面；关 → 嵌入桌面切回普通固定。两个方向都复用现有命令，
/// 互斥清理与托盘勾选由它们自己完成；不在固定模式时只记住偏好，下次固定再生效。
#[tauri::command]
pub fn set_fixed_embed_desktop(
    app_handle: tauri::AppHandle,
    db: State<Database>,
    enabled: bool,
) -> Result<(), String> {
    db.with_connection(|conn| {
        conn.execute(
            "INSERT OR REPLACE INTO settings (key, value, updated_at) VALUES ('fixed_embed_desktop', ?, datetime('now', 'localtime'))",
            [if enabled { "true" } else { "false" }],
        )?;
        Ok(())
    })
    .map_err(|e| e.to_string())?;

    if enabled && is_fixed_mode() {
        set_window_desktop_mode(app_handle, db, true)
    } else if !enabled && is_desktop_mode() {
        set_window_fixed_mode(app_handle, db, true)
    } else {
        Ok(())
    }
}

#[tauri::command]
pub fn set_auto_hide_cursor_inside(inside: bool) -> Result<(), String> {
    with_auto_hide_state(|state| {
        state.cursor_inside_window = inside;
    });
    Ok(())
}

#[tauri::command]
pub fn get_auto_hide_enabled(db: State<Database>) -> Result<bool, String> {
    Ok(get_auto_hide_enabled_value(&db))
}

#[tauri::command]
pub fn set_auto_hide_enabled(
    app_handle: tauri::AppHandle,
    db: State<Database>,
    enabled: bool,
) -> Result<(), String> {
    db.with_connection(|conn| {
        conn.execute(
            "INSERT OR REPLACE INTO settings (key, value, updated_at) VALUES ('auto_hide_enabled', ?, datetime('now', 'localtime'))",
            [if enabled { "true" } else { "false" }],
        )?;
        Ok(())
    })
    .map_err(|e| e.to_string())?;

    let restore_position = with_auto_hide_state(|state| {
        state.enabled = enabled && is_fixed_mode();
        if state.enabled {
            state.hidden = false;
            state.docked_edge = None;
            state.edge_stick_started_at = None;
            None
        } else {
            let restore = if state.hidden {
                state.anchor_position.clone()
            } else {
                None
            };
            state.hidden = false;
            state.docked_edge = None;
            state.monitor_bounds = None;
            state.anchor_size = None;
            state.edge_stick_started_at = None;
            restore
        }
    });

    if let Some(anchor) = restore_position {
        if let Some(main_window) = app_handle.get_webview_window("main") {
            let _ = main_window.set_position(tauri::Position::Physical(tauri::PhysicalPosition {
                x: anchor.x,
                y: anchor.y,
            }));
            show_window(&main_window);
            reassert_window_mode_state(&main_window);
        }
    }

    // 关掉贴边隐藏时，唤起态遗留的置顶必须撤销，否则窗口会一直压在其它窗口之上
    if !enabled {
        if let Some(main_window) = app_handle.get_webview_window("main") {
            set_window_always_on_top(&main_window, false);
            reassert_window_mode_state(&main_window);
        }
    }

    Ok(())
}

#[tauri::command]
pub fn get_top_on_wake(db: State<Database>) -> Result<bool, String> {
    Ok(get_top_on_wake_value(&db))
}

#[tauri::command]
pub fn set_top_on_wake(
    app_handle: tauri::AppHandle,
    db: State<Database>,
    enabled: bool,
) -> Result<(), String> {
    db.with_connection(|conn| {
        conn.execute(
            "INSERT OR REPLACE INTO settings (key, value, updated_at) VALUES ('top_on_wake', ?, datetime('now', 'localtime'))",
            [if enabled { "true" } else { "false" }],
        )?;
        Ok(())
    })
    .map_err(|e| e.to_string())?;

    TOP_ON_WAKE.store(enabled, Ordering::SeqCst);

    // 关闭时立刻撤销当前置顶，不必等窗口下一次收回
    if !enabled {
        if let Some(main_window) = app_handle.get_webview_window("main") {
            set_window_always_on_top(&main_window, false);
            reassert_window_mode_state(&main_window);
        }
    }

    Ok(())
}

/// 窗口底色与背景透明度（仅深色主题下生效）
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowBackground {
    pub color: String,
    pub alpha: f64,
}

#[tauri::command]
pub fn get_window_background(db: State<Database>) -> Result<WindowBackground, String> {
    db.with_connection(|conn| {
        let color: String = conn
            .query_row(
                "SELECT value FROM settings WHERE key = 'window_bg_color'",
                [],
                |row| row.get(0),
            )
            .unwrap_or_else(|_| DEFAULT_WINDOW_BG_COLOR.to_string());
        let alpha: f64 = conn
            .query_row(
                "SELECT value FROM settings WHERE key = 'window_bg_alpha'",
                [],
                |row| {
                    let val: String = row.get(0)?;
                    Ok(val.parse::<f64>().unwrap_or(DEFAULT_WINDOW_BG_ALPHA))
                },
            )
            .unwrap_or(DEFAULT_WINDOW_BG_ALPHA);
        Ok(WindowBackground { color, alpha })
    })
    .map_err(|e| e.to_string())
}

/// 单独保存窗口底色与透明度
///
/// 供设置窗口调用：只写这两个键，不走 saveWindowState，
/// 否则会把设置窗口的几何信息与 is_fixed 误存为主窗口状态
#[tauri::command]
pub fn set_window_background(
    db: State<Database>,
    color: String,
    alpha: f64,
) -> Result<(), String> {
    let alpha = alpha.clamp(0.0, 1.0);
    db.with_connection(|conn| {
        conn.execute(
            "INSERT OR REPLACE INTO settings (key, value, updated_at) VALUES ('window_bg_color', ?, datetime('now', 'localtime'))",
            [&color],
        )?;
        conn.execute(
            "INSERT OR REPLACE INTO settings (key, value, updated_at) VALUES ('window_bg_alpha', ?, datetime('now', 'localtime'))",
            [&alpha.to_string()],
        )?;
        Ok(())
    })
    .map_err(|e| e.to_string())
}

/// 显示并置顶主窗口（托盘双击 / 单击）
///
/// 固定模式下窗口是 `WS_EX_TOOLWINDOW`，既不在任务栏也不参与 Alt+Tab，
/// 单纯 `set_focus` 无法把它从全屏窗口后面拉出来，所以要过一次 topmost。
/// 置顶只是"抬到最前"的手段，随后撤销，除非贴边唤起态本就需要保持置顶。
///
/// 桌面模式下窗口本就常驻所有应用窗口之下，"抬到最前"与模式语义冲突，
/// 只显示 + 给焦点（点击激活同样会被 `ALWAYS_ON_BOTTOM` 钩子压回底部），不碰置顶。
pub fn bring_main_window_to_front(app: &tauri::AppHandle) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };

    if is_desktop_mode() {
        show_window(&window);
        let _ = window.set_focus();
        reassert_window_mode_state(&window);
        return;
    }

    // 贴边隐藏中：先拉回锚点，否则"叫回窗口"只会露出边缘那几个像素。
    // 同时给一段免隐藏宽限期，不然鼠标还没移过去窗口就缩回去了。
    let anchor = with_auto_hide_state(|state| {
        if !state.enabled {
            return None;
        }
        state.keep_visible_until = Some(Instant::now() + TRAY_WAKE_KEEP_VISIBLE);
        state.edge_stick_started_at = None;
        if state.hidden {
            state.hidden = false;
            state.anchor_position.clone()
        } else {
            None
        }
    });
    if let Some(anchor) = anchor {
        let _ = window.set_position(tauri::Position::Physical(tauri::PhysicalPosition {
            x: anchor.x,
            y: anchor.y,
        }));
    }

    unminimize_window(&window);
    show_window(&window);
    set_window_always_on_top(&window, true);
    // set_focus 只走 SetForegroundWindow，不碰 tao 的 flags，不会触发样式覆写
    let _ = window.set_focus();
    // 兜底：set_position 在窗口处于最大化态时会经 apply_diff 重写 ex style
    reassert_window_mode_state(&window);

    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(TRAY_WAKE_TOPMOST_DURATION);
        if !should_stay_on_top() {
            if let Some(window) = app.get_webview_window("main") {
                set_window_always_on_top(&window, false);
                reassert_window_mode_state(&window);
            }
        }
    });
}

/// 读取主窗口的位置与尺寸
///
/// 必须固定取 label 为 "main" 的窗口：设置、编辑器等独立 WebView 也会间接调用此命令，
/// 若按调用方窗口取值，会把它们的几何信息误存成主窗口状态
#[tauri::command]
pub fn get_window_persist_state(app_handle: tauri::AppHandle) -> Result<WindowPersistState, String> {
    let window = app_handle
        .get_webview_window("main")
        .ok_or_else(|| "主窗口不存在".to_string())?;
    let pos = window.outer_position().map_err(|e| e.to_string())?;
    let size = window.outer_size().map_err(|e| e.to_string())?;

    let mut persist = WindowPersistState {
        position: WindowPosition { x: pos.x, y: pos.y },
        size: WindowSize {
            width: size.width,
            height: size.height,
        },
    };

    with_auto_hide_state(|state| {
        if state.hidden {
            if let Some(anchor) = &state.anchor_position {
                persist.position = anchor.clone();
            }
            if let Some(anchor_size) = &state.anchor_size {
                persist.size = anchor_size.clone();
            }
        }
    });

    Ok(persist)
}

/// 重置窗口位置和大小（用于 Tauri 命令）
///
/// 与 `get_window_persist_state` 同理固定取 label 为 "main" 的窗口：设置、编辑器等独立
/// WebView 也可能调用此命令，按调用方窗口重置会把它们挪到主窗口该在的位置
#[tauri::command]
pub fn reset_window(window: Window) -> Result<(), String> {
    let main_window = window
        .app_handle()
        .get_webview_window("main")
        .ok_or_else(|| "主窗口不存在".to_string())?;
    reset_window_impl(&main_window)
}

/// 重置 WebviewWindow 位置和大小（用于托盘菜单）
pub fn reset_webview_window(window: WebviewWindow) -> Result<(), String> {
    reset_window_impl(&window)
}

/// 内部重置窗口实现
fn reset_window_impl(window: &WebviewWindow) -> Result<(), String> {
    clear_auto_hide_runtime_state();

    // 置顶只在 tick_auto_hide 的 Hide 分支撤销。这里刚把 docked_edge / hidden 清空，
    // 若窗口正处于唤起置顶态，Hide 分支再也不会触发，置顶就永久留下了。
    set_window_always_on_top(window, false);

    // 重置到屏幕左上角（10%边距），默认大小 380x600
    let default_width = 380.0;
    let default_height = 600.0;

    // 获取主显示器信息并计算 10% 边距位置
    if let Ok(Some(monitor)) = window.primary_monitor() {
        let scale = monitor.scale_factor();
        let size = monitor.size();
        let position = monitor.position();

        // 计算 10% 边距
        let margin_x = (size.width as f64 * 0.1 / scale) as i32;
        let margin_y = (size.height as f64 * 0.1 / scale) as i32;

        let x = position.x + margin_x;
        let y = position.y + margin_y;

        // 设置位置
        let _ = window.set_position(tauri::Position::Physical(tauri::PhysicalPosition { x, y }));

        // 设置大小
        let _ = window.set_size(tauri::Size::Logical(tauri::LogicalSize {
            width: default_width,
            height: default_height,
        }));

        // 确保可调整大小
        let _ = window.set_resizable(true);
    }

    // set_resizable 走 tao 的 flag 通路，会整体重写 GWL_EXSTYLE。
    // 固定模式下重置窗口时要把任务栏样式补回来，否则图标会一直留在任务栏上
    reassert_window_mode_state(window);

    Ok(())
}

// ============ 屏幕配置相关命令 ============

/// screen_configs 查询列，与 `screen_config_from_row` 的下标一一对应。
///
/// 三处 SELECT 共用一份列表：新增列只改这里和映射函数，不会出现某条查询漏读新列的情况。
const SCREEN_CONFIG_COLUMNS: &str = "id, config_id, display_name, window_x, window_y, \
     window_width, window_height, is_fixed, created_at, updated_at";

fn screen_config_from_row(row: &rusqlite::Row) -> rusqlite::Result<ScreenConfig> {
    Ok(ScreenConfig {
        id: row.get(0)?,
        config_id: row.get(1)?,
        display_name: row.get(2)?,
        window_x: row.get(3)?,
        window_y: row.get(4)?,
        window_width: row.get(5)?,
        window_height: row.get(6)?,
        is_fixed: row.get::<_, i32>(7)? != 0,
        created_at: row.get(8)?,
        updated_at: row.get(9)?,
    })
}

/// 根据屏幕配置标识获取保存的窗口配置
#[tauri::command]
pub fn get_screen_config(
    db: State<Database>,
    config_id: String,
) -> Result<Option<ScreenConfig>, String> {
    db.with_connection(|conn| {
        let result = conn.query_row(
            &format!(
                "SELECT {} FROM screen_configs WHERE config_id = ?",
                SCREEN_CONFIG_COLUMNS
            ),
            [&config_id],
            screen_config_from_row,
        );

        match result {
            Ok(config) => Ok(Some(config)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
        }
    })
    .map_err(|e| e.to_string())
}

/// 保存或更新屏幕配置
#[tauri::command]
pub fn save_screen_config(
    db: State<Database>,
    config: SaveScreenConfigRequest,
) -> Result<ScreenConfig, String> {
    db.with_connection(|conn| {
        // 使用 INSERT OR REPLACE 来保存或更新
        conn.execute(
            "INSERT INTO screen_configs
             (config_id, display_name, window_x, window_y, window_width, window_height, is_fixed, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, datetime('now', 'localtime'))
             ON CONFLICT(config_id) DO UPDATE SET
                display_name = COALESCE(?2, display_name),
                window_x = ?3,
                window_y = ?4,
                window_width = ?5,
                window_height = ?6,
                is_fixed = ?7,
                updated_at = datetime('now', 'localtime')",
            (
                &config.config_id,
                &config.display_name,
                config.window_x,
                config.window_y,
                config.window_width,
                config.window_height,
                if config.is_fixed { 1 } else { 0 },
            ),
        )?;

        // 返回保存后的配置
        conn.query_row(
            &format!(
                "SELECT {} FROM screen_configs WHERE config_id = ?",
                SCREEN_CONFIG_COLUMNS
            ),
            [&config.config_id],
            screen_config_from_row,
        )
    })
    .map_err(|e| e.to_string())
}

/// 获取所有屏幕配置列表
#[tauri::command]
pub fn list_screen_configs(db: State<Database>) -> Result<Vec<ScreenConfig>, String> {
    db.with_connection(|conn| {
        let mut stmt = conn.prepare(&format!(
            "SELECT {} FROM screen_configs ORDER BY updated_at DESC",
            SCREEN_CONFIG_COLUMNS
        ))?;

        let configs = stmt.query_map([], screen_config_from_row)?;

        configs.collect::<Result<Vec<_>, _>>()
    })
    .map_err(|e| e.to_string())
}

/// 删除屏幕配置
#[tauri::command]
pub fn delete_screen_config(db: State<Database>, config_id: String) -> Result<(), String> {
    db.with_connection(|conn| {
        conn.execute(
            "DELETE FROM screen_configs WHERE config_id = ?",
            [&config_id],
        )?;
        Ok(())
    })
    .map_err(|e| e.to_string())
}

/// 更新屏幕配置的显示名称
#[tauri::command]
pub fn update_screen_config_name(
    db: State<Database>,
    config_id: String,
    display_name: String,
) -> Result<(), String> {
    db.with_connection(|conn| {
        conn.execute(
            "UPDATE screen_configs SET display_name = ?, updated_at = datetime('now', 'localtime') WHERE config_id = ?",
            [&display_name, &config_id],
        )?;
        Ok(())
    })
    .map_err(|e| e.to_string())
}

// ============ 日历设置相关命令 ============

/// 获取是否显示日历
#[tauri::command]
pub fn get_show_calendar(db: State<Database>) -> Result<bool, String> {
    db.with_connection(|conn| {
        let show: bool = conn
            .query_row(
                "SELECT value FROM settings WHERE key = 'show_calendar'",
                [],
                |row| {
                    let val: String = row.get(0)?;
                    Ok(val == "true")
                },
            )
            .unwrap_or(false);
        Ok(show)
    })
    .map_err(|e| e.to_string())
}

/// 设置是否显示日历
#[tauri::command]
pub fn set_show_calendar(db: State<Database>, show: bool) -> Result<(), String> {
    db.with_connection(|conn| {
        conn.execute(
            "INSERT OR REPLACE INTO settings (key, value, updated_at) VALUES ('show_calendar', ?, datetime('now', 'localtime'))",
            [if show { "true" } else { "false" }],
        )?;
        Ok(())
    })
    .map_err(|e| e.to_string())
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
    fn waits_while_shell_window_is_absent() {
        // Explorer 重启中：没有宿主可挂，不排队重挂
        assert!(!needs_reattach(PROGMAN, false, 0, PROGMAN));
        assert!(!needs_reattach(0, false, 0, 0));
    }
}
