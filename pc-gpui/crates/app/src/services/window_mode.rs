//! 主窗口的窗口模式与几何（旧版 `commands/window.rs` 的主窗口部分 + 前端 `appStore` 的
//! `applyFixedMode` / `applyNormalMode` / `saveWindowState` / `initSettings` 定位）。**W1**。
//!
//! | 模式 | `classic_fixed` | `desktop` | owner | ex style | `WS_MINIMIZEBOX` | 几何锁定 | 置顶 |
//! |---|---|---|---|---|---|---|---|
//! | 普通 | false | false | 0 | APPWINDOW | 有 | 否 | 否 |
//! | 经典固定 | true | false | 0 | TOOLWINDOW | 有 | 是 | 仅贴边 / 托盘唤起时 |
//! | 固定 + 嵌入桌面（仅 Windows） | false | true | Progman | TOOLWINDOW | 无 | 是 | 永远置底 |
//!
//! `AppState.is_fixed` 对两种固定都为 true；"嵌入"不是第三种模式，而是 `is_fixed && fixed_embed_desktop`
//! （`.trellis/spec/backend/window-modes.md` §3）。GPUI 建窗后不再改写 ex style，旧版为对抗 tao `apply_diff`
//! 写的排队重申不再需要：切换时同步设置一次即可（`research/backend-port.md` §5.3）；200ms 轮询只保留
//! Explorer 重启重挂、经典固定的最小化守护、贴边隐藏，以及廉价的样式自愈（防御）。
//!
//! **线程 / 借用约束**：所有 Win32 写操作都在 `cx.spawn` 的 async 块里、`cx.update` 闭包之外执行
//! （`platform::Hwnd` 原始调用会同步给 GPUI 窗口过程发消息，App 被借用时 GPUI 处理不了）。

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use gpui_kit::{AnyWindowHandle, App, AppContext as _, AsyncApp, Global, Task, Window};
use mini_todo_core::db::{AppSettings, SaveScreenConfigRequest, WindowPosition, WindowSize};
use mini_todo_core::settings;
use mini_todo_core::window_logic::{
    self, AutoHideState, AutoHideTransition, MonitorBounds, WindowRect,
};

use crate::db::Db;
use crate::platform::{self, Hwnd, MonitorInfo, PhysRect};
use crate::state::AppState;
use crate::windows::WindowManager;

/// 贴边隐藏 / 嵌入守护的轮询间隔（旧版 200ms 轮询线程）
pub const TICK_INTERVAL: Duration = Duration::from_millis(200);
/// 移动 / 缩放后保存窗口状态的防抖
pub const SAVE_DEBOUNCE: Duration = Duration::from_millis(500);

/// 主窗口的窗口模式运行时状态（旧版 `IS_FIXED_MODE` / `IS_DESKTOP_MODE` / `DESKTOP_OWNER` /
/// `AUTO_HIDE_STATE` / `TOP_ON_WAKE` / `DESIRED_TOPMOST` / `PENDING_AUTO_HIDE_RELEASE`）
pub struct WindowMode {
    /// 主窗口句柄（只有 Windows 有）
    pub main_hwnd: Option<Hwnd>,
    /// 经典固定模式
    pub classic_fixed: bool,
    /// 固定 + 嵌入桌面
    pub desktop: bool,
    pub auto_hide: AutoHideState,
    top_on_wake: AtomicBool,
    /// 期望的置顶态（贴边唤起 / 托盘唤起）
    pub desired_topmost: bool,
    /// 偏好被关掉后欠一次窗口收尾（拉回藏起的窗口、撤置顶），下个 tick 处理
    pending_release: bool,
    /// 保存窗口状态的防抖代次
    save_generation: u64,
    /// 200ms 轮询是否在跑（主窗口重开时不重复启动）
    tick_running: bool,
}

impl Default for WindowMode {
    fn default() -> Self {
        Self {
            main_hwnd: None,
            classic_fixed: false,
            desktop: false,
            auto_hide: AutoHideState::default(),
            top_on_wake: AtomicBool::new(true),
            desired_topmost: false,
            pending_release: false,
            save_generation: 0,
            tick_running: false,
        }
    }
}

impl Global for WindowMode {}

fn mode(cx: &mut App) -> &mut WindowMode {
    cx.default_global::<WindowMode>()
}

/// 当前是否处在某种固定模式（窗口侧的真实状态）
pub fn is_window_fixed(cx: &App) -> bool {
    cx.try_global::<WindowMode>()
        .is_some_and(|m| m.classic_fixed || m.desktop)
}

/// 主窗口建好后调用：缓存句柄、启动 200ms 轮询
pub fn on_main_opened(cx: &mut App, hwnd: Option<Hwnd>) {
    let mode = mode(cx);
    mode.main_hwnd = hwnd;
    mode.classic_fixed = false;
    mode.desktop = false;
    mode.auto_hide = AutoHideState::default();
    mode.desired_topmost = false;
    if hwnd.is_some() && !mode.tick_running {
        mode.tick_running = true;
        start_tick(cx);
    }
}

/// 主窗口关闭后调用
pub fn on_main_closed(cx: &mut App) {
    let mode = mode(cx);
    mode.main_hwnd = None;
    mode.classic_fixed = false;
    mode.desktop = false;
    mode.auto_hide = AutoHideState::default();
}

/// 运行时偏好变化（设置窗口改了贴边隐藏 / 唤起置顶、导入、同步应用了远端设置）：刷新状态机，
/// 有功能被关掉时下个 tick 收尾（旧 `reload_runtime_prefs`）
pub fn apply_runtime_prefs(cx: &mut App, top_on_wake: bool, auto_hide_enabled: bool) {
    let mode = mode(cx);
    let active = auto_hide_enabled && mode.classic_fixed;
    let WindowMode {
        auto_hide,
        top_on_wake: cell,
        ..
    } = mode;
    if window_logic::apply_runtime_prefs(auto_hide, cell, top_on_wake, active) {
        mode.pending_release = true;
    }
}

// ----------------------------------------------------------------------
// 模式切换
// ----------------------------------------------------------------------

/// 目标模式
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetMode {
    Normal,
    ClassicFixed,
    Desktop,
}

impl TargetMode {
    /// `applyFixedMode` / `applyNormalMode` 的选择：固定时 Windows 且开了"嵌入桌面"→ 嵌入，否则经典固定
    pub fn for_state(is_fixed: bool, embed_desktop: bool) -> TargetMode {
        match (is_fixed, embed_desktop && cfg!(windows)) {
            (false, _) => TargetMode::Normal,
            (true, true) => TargetMode::Desktop,
            (true, false) => TargetMode::ClassicFixed,
        }
    }
}

/// 一组要在 App 借用之外执行的窗口操作
#[derive(Debug, Default)]
struct Ops {
    steps: Vec<Op>,
}

#[derive(Debug, Clone, Copy)]
enum Op {
    MoveTo(i32, i32),
    Topmost(bool),
    ToolWindow(bool),
    LockGeometry(bool),
    Attach,
    Detach { tool_window: bool },
    Restore,
}

impl Ops {
    fn push(&mut self, op: Op) {
        self.steps.push(op);
    }

    /// 执行（立即执行的 Win32；只在 async 块里、`cx.update` 之外调用）
    fn run(self, hwnd: Hwnd) {
        for op in self.steps {
            run_op(hwnd, op);
        }
    }
}

fn run_op(hwnd: Hwnd, op: Op) -> bool {
    match op {
        Op::MoveTo(x, y) => hwnd.set_client_position(x, y),
        Op::Topmost(on) => hwnd.set_topmost(on),
        Op::ToolWindow(tool) => {
            hwnd.set_tool_window(tool);
            true
        }
        Op::LockGeometry(locked) => {
            hwnd.set_geometry_locked(locked);
            true
        }
        Op::Attach => platform::embed::attach_to_desktop(hwnd),
        Op::Detach { tool_window } => {
            platform::embed::detach_from_desktop(hwnd, tool_window);
            true
        }
        Op::Restore => {
            if hwnd.is_minimized() {
                hwnd.restore();
            }
            if !hwnd.is_visible() {
                hwnd.show();
            }
            true
        }
    }
}

/// 计算切到 `target` 要做的事，并更新运行时状态（在 `cx.update` 里调用）
fn plan_mode(cx: &mut App, target: TargetMode) -> Ops {
    let prefs = AppState::global(cx).read(cx).runtime_prefs;
    let mode = mode(cx);
    let mut ops = Ops::default();
    mode.top_on_wake.store(prefs.top_on_wake, Ordering::SeqCst);
    match target {
        TargetMode::ClassicFixed => {
            if mode.desktop {
                mode.desktop = false;
                // 嵌入 → 经典固定：保持 TOOLWINDOW，不闪任务栏
                ops.push(Op::Detach { tool_window: true });
            }
            mode.classic_fixed = true;
            if let Some(anchor) = mode.auto_hide.enter_fixed_mode(prefs.auto_hide_enabled) {
                ops.push(Op::MoveTo(anchor.x, anchor.y));
            }
            ops.push(Op::LockGeometry(true));
            ops.push(Op::ToolWindow(true));
            ops.push(Op::Topmost(mode.desired_topmost));
        }
        TargetMode::Desktop => {
            if mode.classic_fixed {
                // 先撤置顶再退出经典固定（否则解锁后一直置顶）
                ops.push(Op::Topmost(false));
                mode.desired_topmost = false;
                mode.classic_fixed = false;
            }
            if let Some(anchor) = mode.auto_hide.leave_fixed_mode() {
                ops.push(Op::MoveTo(anchor.x, anchor.y));
            }
            mode.desktop = true;
            ops.push(Op::LockGeometry(true));
            ops.push(Op::Attach);
        }
        TargetMode::Normal => {
            if mode.classic_fixed || mode.desired_topmost {
                ops.push(Op::Topmost(false));
            }
            mode.desired_topmost = false;
            mode.classic_fixed = false;
            if let Some(anchor) = mode.auto_hide.leave_fixed_mode() {
                ops.push(Op::MoveTo(anchor.x, anchor.y));
            }
            if mode.desktop {
                mode.desktop = false;
                ops.push(Op::Detach { tool_window: false });
            }
            ops.push(Op::LockGeometry(false));
            ops.push(Op::ToolWindow(false));
        }
    }
    ops
}

/// 把主窗口切到 `target`（异步：Win32 部分在借用之外执行）。主窗口不存在时 `Err("主窗口不存在")`
pub fn apply_mode(cx: &mut App, target: TargetMode) -> Task<Result<(), String>> {
    cx.spawn(async move |cx| {
        let planned = cx.update(|cx| {
            WindowManager::main(cx)?;
            let hwnd = mode(cx).main_hwnd;
            let ops = plan_mode(cx, target);
            crate::services::tray::set_fixed_checked(cx, target != TargetMode::Normal);
            Some((hwnd, ops))
        });
        let Some((hwnd, ops)) = planned else {
            return Err("主窗口不存在".to_string());
        };
        log::info!("[window] 主窗口切换到 {target:?}");
        if let Some(hwnd) = hwnd {
            ops.run(hwnd);
        }
        Ok(())
    })
}

/// 切换固定模式（旧 `toggleFixedMode`）：`fixed` 是切换后的 `is_fixed`。切完保存窗口状态；
/// 失败提示 `切换固定模式失败：…`
pub fn toggle_fixed(cx: &mut App, fixed: bool) {
    let embed = AppState::global(cx).read(cx).fixed_embed_desktop;
    let task = apply_mode(cx, TargetMode::for_state(fixed, embed));
    cx.spawn(async move |cx| {
        let result = task.await;
        cx.update(|cx| match result {
            Ok(()) => save_window_state(cx),
            Err(e) => {
                log::error!("[window] 切换固定模式失败: {e}");
                if let Some(main) = WindowManager::main(cx) {
                    crate::ui::message::show_in(
                        main,
                        cx,
                        crate::ui::message::MessageKind::Error,
                        crate::ui::message::error_text("切换固定模式失败", &e),
                    );
                }
            }
        });
    })
    .detach();
}

/// "嵌入桌面"偏好在已固定时被切换（设置窗口 / 同步）：直接在两种固定之间切换，不保存窗口状态
/// （旧 `set_fixed_embed_desktop` 的后端部分）
pub fn on_embed_preference_changed(cx: &mut App, enabled: bool) {
    let is_fixed = AppState::global(cx).read(cx).is_fixed;
    if !is_fixed || !cfg!(windows) {
        return;
    }
    let mode = cx.try_global::<WindowMode>();
    let currently_desktop = mode.is_some_and(|m| m.desktop);
    if enabled == currently_desktop {
        return;
    }
    apply_mode(cx, TargetMode::for_state(true, enabled)).detach();
}

// ----------------------------------------------------------------------
// 200ms 轮询（Windows）：嵌入重挂 / 经典固定守护 / 贴边隐藏
// ----------------------------------------------------------------------

fn start_tick(cx: &mut App) {
    cx.spawn(async move |cx| {
        loop {
            cx.background_executor().timer(TICK_INTERVAL).await;
            let planned = cx.update(plan_tick);
            let Some(plan) = planned else {
                cx.update(|cx| mode(cx).tick_running = false);
                break;
            };
            run_tick(cx, plan);
        }
    })
    .detach();
}

struct TickPlan {
    hwnd: Hwnd,
    ops: Ops,
    /// 贴边隐藏的移动（失败要回滚状态）
    auto_hide: Option<AutoHideMove>,
}

enum AutoHideMove {
    Hide { to: WindowPosition },
    Restore { to: WindowPosition, topmost: bool },
}

/// 读状态、算出这一 tick 要做的事（在 `cx.update` 里）；主窗口没了返回 `None`（结束轮询）
fn plan_tick(cx: &mut App) -> Option<TickPlan> {
    let hwnd = mode(cx).main_hwnd?;
    if !hwnd.is_alive() {
        return None;
    }
    let monitors = platform::monitors(cx);
    let mode = mode(cx);
    let mut ops = Ops::default();
    let mut auto_hide = None;
    if mode.desktop {
        if platform::embed::needs_reattach(hwnd) {
            ops.push(Op::Attach);
        }
    } else if mode.classic_fixed {
        if hwnd.is_minimized() {
            // 固定模式没有任务栏入口：被 Win+D / Win+M 最小化后立刻还原
            ops.push(Op::Restore);
        }
        if window_logic::lacks_tool_window_style(hwnd.ex_style()) {
            ops.push(Op::ToolWindow(true));
        }
        if mode.pending_release {
            mode.pending_release = false;
            if let Some(anchor) = window_logic::take_disabled_hidden_anchor(&mut mode.auto_hide) {
                ops.push(Op::MoveTo(anchor.x, anchor.y));
            }
            if !mode.top_on_wake.load(Ordering::SeqCst) || !mode.auto_hide.enabled {
                mode.desired_topmost = false;
                ops.push(Op::Topmost(false));
            }
        }
        if mode.auto_hide.enabled
            && let Some(rect) = hwnd.client_rect()
        {
            let window = WindowRect {
                x: rect.x,
                y: rect.y,
                width: rect.width,
                height: rect.height,
            };
            let monitor = monitor_for_rect(&monitors, rect)
                .map(|m| monitor_bounds(&m))
                .or(mode.auto_hide.monitor_bounds);
            if let Some(monitor) = monitor {
                let cursor = platform::cursor_position_physical();
                match window_logic::evaluate_auto_hide_transition(
                    &mut mode.auto_hide,
                    window,
                    monitor,
                    cursor,
                    Instant::now(),
                ) {
                    AutoHideTransition::None => {}
                    AutoHideTransition::Hide { hidden, .. } => {
                        auto_hide = Some(AutoHideMove::Hide { to: hidden });
                    }
                    AutoHideTransition::Restore { anchor } => {
                        auto_hide = Some(AutoHideMove::Restore {
                            to: anchor,
                            topmost: mode.top_on_wake.load(Ordering::SeqCst),
                        });
                    }
                }
            }
        }
    } else if mode.pending_release {
        mode.pending_release = false;
    }
    Some(TickPlan {
        hwnd,
        ops,
        auto_hide,
    })
}

fn run_tick(cx: &mut AsyncApp, plan: TickPlan) {
    let TickPlan {
        hwnd,
        ops,
        auto_hide,
    } = plan;
    for op in ops.steps {
        if let Op::Attach = op {
            // 嵌入已被退出（排队期间切走了）时不再挂
            if !cx.update(|cx| mode(cx).desktop) {
                continue;
            }
        }
        run_op(hwnd, op);
    }
    match auto_hide {
        None => {}
        Some(AutoHideMove::Hide { to }) => {
            if hwnd.set_client_position(to.x, to.y) {
                hwnd.set_topmost(false);
                cx.update(|cx| mode(cx).desired_topmost = false);
            } else {
                // 移动失败：回滚为显示态
                cx.update(|cx| {
                    let state = &mut mode(cx).auto_hide;
                    state.hidden = false;
                    state.edge_stick_started_at = None;
                });
            }
        }
        Some(AutoHideMove::Restore { to, topmost }) => {
            if hwnd.set_client_position(to.x, to.y) {
                if topmost {
                    hwnd.set_topmost(true);
                    cx.update(|cx| mode(cx).desired_topmost = true);
                }
            } else {
                cx.update(|cx| mode(cx).auto_hide.hidden = true);
            }
        }
    }
}

/// 显示器整块区域（贴边判断用整块显示器，不是工作区）
fn monitor_bounds(m: &MonitorInfo) -> MonitorBounds {
    MonitorBounds {
        x: m.rect.x,
        y: m.rect.y,
        width: m.rect.width,
        height: m.rect.height,
    }
}

/// 矩形中心所在的显示器；中心不在任何显示器上时取面积重叠最大的
fn monitor_for_rect(monitors: &[MonitorInfo], rect: PhysRect) -> Option<MonitorInfo> {
    let cx = rect.x + rect.width / 2;
    let cy = rect.y + rect.height / 2;
    monitors
        .iter()
        .find(|m| m.rect.contains(cx, cy))
        .or_else(|| {
            monitors.iter().max_by_key(|m| {
                let w = (rect.right().min(m.rect.right()) - rect.x.max(m.rect.x)).max(0) as i64;
                let h = (rect.bottom().min(m.rect.bottom()) - rect.y.max(m.rect.y)).max(0) as i64;
                w * h
            })
        })
        .cloned()
}

// ----------------------------------------------------------------------
// 托盘唤起 / 重置窗口
// ----------------------------------------------------------------------

/// 显示并激活主窗口（托盘左键、单实例唤起；旧 `bring_main_window_to_front`）：嵌入桌面只显示 + 聚焦；
/// 其它模式：贴边隐藏开着时给 3s 宽限并拉回 → 还原 → 临时置顶 600ms（贴边唤起态需要保持置顶时不撤）→ 激活
pub fn bring_main_to_front(cx: &mut App) {
    let Some(main) = WindowManager::main(cx) else {
        crate::windows::main_window::open(cx);
        return;
    };
    cx.spawn(async move |cx| {
        let plan = cx.update(|cx| {
            let mode = mode(cx);
            let hwnd = mode.main_hwnd;
            if mode.desktop {
                return (hwnd, true, None);
            }
            let anchor = mode.auto_hide.wake_from_tray(Instant::now());
            mode.desired_topmost = true;
            (hwnd, false, anchor)
        });
        let (hwnd, desktop, anchor) = plan;
        if let Some(hwnd) = hwnd {
            if let Some(anchor) = &anchor {
                hwnd.set_client_position(anchor.x, anchor.y);
            }
            run_op(hwnd, Op::Restore);
            if !desktop {
                hwnd.set_topmost(true);
            }
            hwnd.set_foreground();
        }
        let _ = cx.update_window(main, |_, window, _| window.activate_window());
        if desktop || hwnd.is_none() {
            return;
        }
        let generation = window_logic::begin_tray_wake();
        cx.background_executor()
            .timer(window_logic::TRAY_WAKE_TOPMOST_DURATION)
            .await;
        let release = cx.update(|cx| {
            let mode = mode(cx);
            let stay = mode
                .auto_hide
                .should_stay_on_top(mode.top_on_wake.load(Ordering::SeqCst));
            let release = window_logic::is_latest_tray_wake(generation) && !stay;
            if release {
                mode.desired_topmost = false;
            }
            release
        });
        if release && let Some(hwnd) = hwnd {
            hwnd.set_topmost(false);
        }
    })
    .detach();
}

/// 托盘「重置位置」（旧 `reset_window_impl` + 前端 `tray-reset-window`）：清贴边状态、撤置顶、
/// 主显示器左上起 10% 边距、380×600（逻辑）；固定中则退回普通模式；然后保存窗口状态
pub fn reset_main_window(cx: &mut App) {
    let Some(main) = WindowManager::main(cx) else {
        crate::windows::main_window::open(cx);
        return;
    };
    let primary = platform::primary_monitor(cx);
    cx.spawn(async move |cx| {
        let hwnd = cx.update(|cx| {
            let mode = mode(cx);
            mode.auto_hide.clear_runtime();
            mode.desired_topmost = false;
            mode.main_hwnd
        });
        if let (Some(hwnd), Some(monitor)) = (hwnd, &primary) {
            let position =
                window_logic::reset_window_position(monitor_bounds(monitor), monitor.scale);
            let width = (window_logic::RESET_WINDOW_WIDTH * monitor.scale).round() as i32;
            let height = (window_logic::RESET_WINDOW_HEIGHT * monitor.scale).round() as i32;
            hwnd.set_topmost(false);
            hwnd.set_client_rect(PhysRect::new(position.x, position.y, width, height));
        } else {
            let _ = cx.update_window(main, |_, window, _| {
                window.resize(gpui_kit::size(
                    gpui_kit::px(window_logic::RESET_WINDOW_WIDTH as f32),
                    gpui_kit::px(window_logic::RESET_WINDOW_HEIGHT as f32),
                ));
            });
        }
        cx.update(|cx| {
            let state = AppState::global(cx);
            if state.read(cx).is_fixed {
                // 固定模式不允许挪窗口：重置后退回普通模式（toggle_fixed 内会保存）
                state.update(cx, |state, cx| state.toggle_fixed_mode(cx));
            } else {
                save_window_state(cx);
            }
        });
    })
    .detach();
}

// ----------------------------------------------------------------------
// 保存 / 恢复窗口状态
// ----------------------------------------------------------------------

/// 移动 / 缩放后防抖 500ms 保存（旧 `debouncedSaveState`）
pub fn schedule_save(cx: &mut App) {
    let generation = {
        let mode = mode(cx);
        mode.save_generation += 1;
        mode.save_generation
    };
    cx.spawn(async move |cx| {
        cx.background_executor().timer(SAVE_DEBOUNCE).await;
        cx.update(|cx| {
            if mode(cx).save_generation == generation {
                save_window_state(cx);
            }
        });
    })
    .detach();
}

/// 立即保存（取消进行中的防抖）
pub fn save_window_state(cx: &mut App) {
    mode(cx).save_generation += 1;
    let Some(main) = WindowManager::main(cx) else {
        return;
    };
    let hwnd = mode(cx).main_hwnd;
    cx.spawn(async move |cx| {
        let rect = match hwnd.and_then(Hwnd::client_rect) {
            Some(rect) => Some(rect),
            None => cx
                .update_window(main, |_, window, _| platform::client_rect_physical(window))
                .ok()
                .flatten(),
        };
        let Some(rect) = rect else {
            return;
        };
        cx.update(|cx| save_window_state_with(cx, rect));
    })
    .detach();
}

/// 用已经读好的客户区几何保存（主窗口关闭时窗口马上就没了，几何要在关闭回调里同步读）。
///
/// 设置还没加载完（`AppState.initialized` 为假）时等加载完再写：`save_window_settings` 会写主题 / 底色 / 贴边等
/// 9 个键，用默认值写就把用户设置覆盖了（旧版 `initSettings` 也是先读设置再保存）
pub fn save_window_state_with(cx: &mut App, rect: PhysRect) {
    if !AppState::global(cx).read(cx).initialized {
        cx.spawn(async move |cx| {
            for _ in 0..200 {
                cx.background_executor()
                    .timer(Duration::from_millis(50))
                    .await;
                if cx.update(|cx| AppState::global(cx).read(cx).initialized) {
                    cx.update(|cx| save_window_state_with(cx, rect));
                    return;
                }
            }
            log::warn!("[window] 设置迟迟没有加载完，放弃这次保存窗口状态");
        })
        .detach();
        return;
    }
    let persist = mode(cx).auto_hide.persist_state(
        WindowPosition {
            x: rect.x,
            y: rect.y,
        },
        WindowSize {
            width: rect.width.max(0) as u32,
            height: rect.height.max(0) as u32,
        },
    );
    let state = AppState::global(cx);
    let screen_id = {
        let current = state.read(cx).current_screen_config_id.clone();
        if current.is_empty() {
            let id = platform::screen_config_id(cx);
            state.update(cx, |state, _| state.current_screen_config_id = id.clone());
            id
        } else {
            current
        }
    };
    let s = state.read(cx);
    let request = SaveScreenConfigRequest {
        config_id: screen_id.clone(),
        display_name: Some(settings::screen_config_display_name(&screen_id)),
        window_x: persist.position.x,
        window_y: persist.position.y,
        window_width: persist.size.width as i32,
        window_height: persist.size.height as i32,
        is_fixed: s.is_fixed,
    };
    let app_settings = AppSettings {
        is_fixed: s.is_fixed,
        fixed_embed_desktop: s.fixed_embed_desktop,
        window_position: Some(persist.position.clone()),
        window_size: Some(persist.size.clone()),
        auto_hide_enabled: s.auto_hide_enabled,
        top_on_wake: s.top_on_wake,
        window_bg_color: s.window_bg.color.clone(),
        window_bg_alpha: s.window_bg.alpha,
        text_theme: if s.is_dark_theme { "light" } else { "dark" }.to_string(),
        show_calendar: s.show_calendar,
        view_mode: s.view_mode.as_str().to_string(),
        notification_type: s.notification_type.as_str().to_string(),
    };
    let db = Db::get(cx);
    let task = cx.background_executor().spawn(async move {
        settings::save_screen_config(&db, &request)?;
        settings::save_window_settings(&db, &app_settings)?;
        Ok::<_, String>(settings::runtime_prefs(&db))
    });
    cx.spawn(async move |cx| match task.await {
        Ok(prefs) => cx.update(|cx| {
            state.update(cx, |state, cx| {
                state.window_position = Some(persist.position);
                state.window_size = Some(persist.size);
                state.runtime_prefs = prefs;
                cx.notify();
            });
            apply_runtime_prefs(cx, prefs.top_on_wake, prefs.auto_hide_enabled);
        }),
        Err(e) => log::warn!("[window] 保存窗口状态失败: {e}"),
    })
    .detach();
}

/// 启动时恢复主窗口用的信息（建窗前同步读出：一次按主键的查询，避免窗口先居中再跳）
#[derive(Debug, Clone, Default)]
pub struct StartupGeometry {
    pub screen_config_id: String,
    /// 该屏幕组合记下的客户区（物理像素）
    pub saved: Option<PhysRect>,
    pub is_fixed: bool,
    pub embed_desktop: bool,
}

/// 读当前屏幕组合的记录（旧 `initSettings` 的 `get_screen_config`）
pub fn read_startup_geometry(cx: &App) -> StartupGeometry {
    let screen_config_id = platform::screen_config_id(cx);
    let db = Db::get(cx);
    let saved = settings::get_screen_config(&db, &screen_config_id)
        .map_err(|e| log::warn!("[window] 读取屏幕配置失败: {e}"))
        .ok()
        .flatten();
    StartupGeometry {
        embed_desktop: settings::fixed_embed_desktop(&db),
        is_fixed: saved.as_ref().is_some_and(|c| c.is_fixed),
        saved: saved
            .filter(|c| c.window_width > 0 && c.window_height > 0)
            .map(|c| PhysRect::new(c.window_x, c.window_y, c.window_width, c.window_height)),
        screen_config_id,
    }
}

/// 物理客户区 → 建窗用的逻辑矩形与显示器（中心不在任何显示器上返回 `None`，调用方居中后再挪）
pub fn logical_bounds_for(
    monitors: &[MonitorInfo],
    rect: PhysRect,
) -> Option<(gpui_kit::Bounds<gpui_kit::Pixels>, gpui_kit::DisplayId)> {
    let cx = rect.x + rect.width / 2;
    let cy = rect.y + rect.height / 2;
    let monitor = monitors.iter().find(|m| m.rect.contains(cx, cy))?;
    Some((rect.to_logical(monitor.scale), monitor.display_id))
}

/// 主窗口关闭请求：同步读几何并保存（旧 `onCloseRequested`）
pub fn on_main_close_requested(window: &Window, cx: &mut App) {
    if let Some(rect) = platform::client_rect_physical(window) {
        save_window_state_with(cx, rect);
    }
}

/// 主窗口句柄（托盘 / 单实例用）
pub fn main_window(cx: &App) -> Option<AnyWindowHandle> {
    WindowManager::main(cx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::DisplayId;

    fn monitor(x: i32, y: i32, w: i32, h: i32, scale: f64) -> MonitorInfo {
        MonitorInfo {
            display_id: DisplayId::new((x + 1) as u64),
            rect: PhysRect::new(x, y, w, h),
            work: PhysRect::new(x, y, w, h - 40),
            scale,
            primary: x == 0,
        }
    }

    #[test]
    fn target_mode_follows_embed_preference_only_on_windows() {
        assert_eq!(TargetMode::for_state(false, true), TargetMode::Normal);
        assert_eq!(TargetMode::for_state(true, false), TargetMode::ClassicFixed);
        let embedded = TargetMode::for_state(true, true);
        if cfg!(windows) {
            assert_eq!(embedded, TargetMode::Desktop);
        } else {
            // 非 Windows：同步来的"嵌入桌面"偏好不能让固定失败，回退为经典固定
            assert_eq!(embedded, TargetMode::ClassicFixed);
        }
    }

    #[test]
    fn monitor_lookup_uses_center_then_overlap() {
        let monitors = vec![
            monitor(0, 0, 1920, 1080, 1.0),
            monitor(1920, 0, 2560, 1440, 1.25),
        ];
        let on_second = PhysRect::new(2000, 100, 475, 750);
        assert_eq!(monitor_for_rect(&monitors, on_second).unwrap().rect.x, 1920);
        // 中心在屏幕外（被拖出一半）：取重叠最大的
        let off = PhysRect::new(-300, -500, 400, 600);
        assert_eq!(monitor_for_rect(&monitors, off).unwrap().rect.x, 0);

        let (bounds, display) = logical_bounds_for(&monitors, on_second).unwrap();
        assert_eq!(display, DisplayId::new(1921));
        assert_eq!(f32::from(bounds.origin.x), 1600.);
        assert_eq!(f32::from(bounds.size.width), 380.);
        assert!(logical_bounds_for(&monitors, PhysRect::new(5000, 5000, 100, 100)).is_none());
    }
}
