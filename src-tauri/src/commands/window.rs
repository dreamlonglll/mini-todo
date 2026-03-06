use crate::db::{
    AppSettings, Database, SaveScreenConfigRequest, ScreenConfig, WindowPosition, WindowSize,
};
use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use tauri::{Manager, State, WebviewWindow, Window};

#[cfg(target_os = "macos")]
use window_vibrancy::{
    apply_vibrancy, clear_vibrancy, NSVisualEffectMaterial, NSVisualEffectState,
};

#[cfg(target_os = "windows")]
use windows::Win32::Foundation::{HWND, POINT};
#[cfg(target_os = "windows")]
use windows::Win32::UI::WindowsAndMessaging::{
    GetCursorPos, GetWindowLongW, SetWindowLongW, GWL_EXSTYLE, WS_EX_APPWINDOW, WS_EX_TOOLWINDOW,
};

/// 全局固定模式状态
pub static IS_FIXED_MODE: AtomicBool = AtomicBool::new(false);

/// 托盘"固定模式"勾选菜单项引用，用于跨模块同步状态
static TRAY_TOGGLE_FIXED: OnceLock<tauri::menu::CheckMenuItem<tauri::Wry>> = OnceLock::new();

/// 检查当前是否处于固定模式
pub fn is_fixed_mode() -> bool {
    IS_FIXED_MODE.load(Ordering::SeqCst)
}

/// 保存托盘"固定模式"勾选菜单项引用（在 setup 阶段调用一次）
pub fn set_tray_toggle_fixed_item(item: tauri::menu::CheckMenuItem<tauri::Wry>) {
    let _ = TRAY_TOGGLE_FIXED.set(item);
}

/// 同步更新托盘菜单项勾选状态
fn sync_tray_fixed_checked(fixed: bool) {
    if let Some(item) = TRAY_TOGGLE_FIXED.get() {
        let _ = item.set_checked(fixed);
    }
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

const EDGE_SNAP_THRESHOLD_PX: i32 = 10;
const EDGE_HIDE_DELAY: Duration = Duration::from_millis(420);
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
        return AutoHideTransition::None;
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

        let text_theme: String = conn
            .query_row(
                "SELECT value FROM settings WHERE key = 'text_theme'",
                [],
                |row| row.get(0),
            )
            .unwrap_or_else(|_| "dark".to_string());

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
            window_position,
            window_size,
            auto_hide_enabled,
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
    db.with_connection(|conn| {
        // 保存 is_fixed
        conn.execute(
            "INSERT OR REPLACE INTO settings (key, value, updated_at) VALUES ('is_fixed', ?, datetime('now', 'localtime'))",
            [if settings.is_fixed { "true" } else { "false" }],
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

        // 保存文本主题
        conn.execute(
            "INSERT OR REPLACE INTO settings (key, value, updated_at) VALUES ('text_theme', ?, datetime('now', 'localtime'))",
            [&settings.text_theme],
        )?;

        Ok(())
    })
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_window_fixed_mode(
    window: Window,
    db: State<Database>,
    fixed: bool,
) -> Result<(), String> {
    // 更新全局固定模式状态
    IS_FIXED_MODE.store(fixed, Ordering::SeqCst);
    let auto_hide_enabled = get_auto_hide_enabled_value(&db);

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

    #[cfg(target_os = "windows")]
    {
        use raw_window_handle::HasWindowHandle;

        if let Ok(handle) = window.window_handle() {
            if let raw_window_handle::RawWindowHandle::Win32(win32_handle) = handle.as_raw() {
                let hwnd = HWND(win32_handle.hwnd.get() as *mut _);

                unsafe {
                    let ex_style = GetWindowLongW(hwnd, GWL_EXSTYLE);

                    if fixed {
                        // 设置为工具窗口样式，不显示在任务栏
                        let new_style = (ex_style as u32 | WS_EX_TOOLWINDOW.0) & !WS_EX_APPWINDOW.0;
                        SetWindowLongW(hwnd, GWL_EXSTYLE, new_style as i32);
                    } else {
                        // 恢复为普通窗口样式
                        let new_style = (ex_style as u32 & !WS_EX_TOOLWINDOW.0) | WS_EX_APPWINDOW.0;
                        SetWindowLongW(hwnd, GWL_EXSTYLE, new_style as i32);
                    }
                }
            }
        }
    }

    #[cfg(target_os = "macos")]
    {
        if fixed {
            let _ = apply_vibrancy(
                &window,
                NSVisualEffectMaterial::HudWindow,
                Some(NSVisualEffectState::FollowsWindowActiveState),
                Some(10.0),
            );
        } else {
            let _ = clear_vibrancy(&window);
        }
    }

    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        let _ = (window, fixed);
    }

    sync_tray_fixed_checked(fixed);

    Ok(())
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
            let _ = main_window.show();
        }
    }

    Ok(())
}

#[tauri::command]
pub fn get_window_persist_state(window: Window) -> Result<WindowPersistState, String> {
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
#[tauri::command]
pub fn reset_window(window: Window) -> Result<(), String> {
    reset_window_impl(&window)
}

/// 重置 WebviewWindow 位置和大小（用于托盘菜单）
pub fn reset_webview_window(window: WebviewWindow) -> Result<(), String> {
    reset_window_impl(&window)
}

/// 内部重置窗口实现
fn reset_window_impl<T: tauri::Runtime>(window: &impl WindowExt<T>) -> Result<(), String> {
    clear_auto_hide_runtime_state();

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

    Ok(())
}

/// 窗口扩展 trait
trait WindowExt<R: tauri::Runtime> {
    fn primary_monitor(&self) -> tauri::Result<Option<tauri::Monitor>>;
    fn set_position(&self, position: tauri::Position) -> tauri::Result<()>;
    fn set_size(&self, size: tauri::Size) -> tauri::Result<()>;
    fn set_resizable(&self, resizable: bool) -> tauri::Result<()>;
}

impl<R: tauri::Runtime> WindowExt<R> for Window<R> {
    fn primary_monitor(&self) -> tauri::Result<Option<tauri::Monitor>> {
        Window::primary_monitor(self)
    }
    fn set_position(&self, position: tauri::Position) -> tauri::Result<()> {
        Window::set_position(self, position)
    }
    fn set_size(&self, size: tauri::Size) -> tauri::Result<()> {
        Window::set_size(self, size)
    }
    fn set_resizable(&self, resizable: bool) -> tauri::Result<()> {
        Window::set_resizable(self, resizable)
    }
}

impl<R: tauri::Runtime> WindowExt<R> for WebviewWindow<R> {
    fn primary_monitor(&self) -> tauri::Result<Option<tauri::Monitor>> {
        WebviewWindow::primary_monitor(self)
    }
    fn set_position(&self, position: tauri::Position) -> tauri::Result<()> {
        WebviewWindow::set_position(self, position)
    }
    fn set_size(&self, size: tauri::Size) -> tauri::Result<()> {
        WebviewWindow::set_size(self, size)
    }
    fn set_resizable(&self, resizable: bool) -> tauri::Result<()> {
        WebviewWindow::set_resizable(self, resizable)
    }
}

// ============ 屏幕配置相关命令 ============

/// 根据屏幕配置标识获取保存的窗口配置
#[tauri::command]
pub fn get_screen_config(
    db: State<Database>,
    config_id: String,
) -> Result<Option<ScreenConfig>, String> {
    db.with_connection(|conn| {
        let result = conn.query_row(
            "SELECT id, config_id, display_name, window_x, window_y, window_width, window_height, 
                    is_fixed, created_at, updated_at 
             FROM screen_configs WHERE config_id = ?",
            [&config_id],
            |row| {
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
            },
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
            "SELECT id, config_id, display_name, window_x, window_y, window_width, window_height, 
                    is_fixed, created_at, updated_at 
             FROM screen_configs WHERE config_id = ?",
            [&config.config_id],
            |row| {
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
            },
        )
    })
    .map_err(|e| e.to_string())
}

/// 获取所有屏幕配置列表
#[tauri::command]
pub fn list_screen_configs(db: State<Database>) -> Result<Vec<ScreenConfig>, String> {
    db.with_connection(|conn| {
        let mut stmt = conn.prepare(
            "SELECT id, config_id, display_name, window_x, window_y, window_width, window_height, 
                    is_fixed, created_at, updated_at 
             FROM screen_configs ORDER BY updated_at DESC",
        )?;

        let configs = stmt.query_map([], |row| {
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
        })?;

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
