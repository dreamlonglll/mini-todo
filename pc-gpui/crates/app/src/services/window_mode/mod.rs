//! 桌面块窗口（清单窗口 / 日历窗口；现在只有主窗口 = 默认清单窗口 [`BlockKey::DEFAULT_LIST`]）的窗口模式与几何
//! （旧版 `commands/window.rs` 的主窗口部分 + 前端 `appStore` 的 `applyFixedMode` / `applyNormalMode` / `saveWindowState` /
//! `initSettings` 定位），**按窗口**：每个登记的窗口一份模式状态（[`machine::ModeState`]，纯状态机）与一份登记（句柄 /
//! 原生窗口 / 根视图 / 保存防抖）。
//!
//! | 模式 | `classic_fixed` | `desktop` | owner | ex style | `WS_MINIMIZEBOX` | 几何锁定 | 置顶 |
//! |---|---|---|---|---|---|---|---|
//! | 普通 | false | false | 0 | APPWINDOW | 有 | 否 | 否 |
//! | 经典固定 | true | false | 0 | TOOLWINDOW | 有 | 是 | 仅贴边 / 托盘唤起时 |
//! | 固定 + 嵌入桌面（仅 Windows） | false | true | Progman | TOOLWINDOW | 无 | 是 | 永远置底 |
//!
//! - **锁定按窗口**（`ModeState.fixed`，标题栏锁按钮 → [`toggle_fixed`]）；"嵌入"不是第三种模式，而是锁定 && `fixed_embed_desktop`
//!   （`.trellis/spec/backend/window-modes.md` §3）。**嵌入桌面 / 贴边隐藏 / 唤起置顶三个偏好是全局的**（`AppState.fixed_embed_desktop`、
//!   `AppState.runtime_prefs`），作用于每个锁定的窗口：本模块在第一个窗口登记时集中观察 `AppState`，偏好变了对所有窗口收尾
//!   （[`on_embed_preference_changed`] / [`apply_runtime_prefs`]），不靠各窗口各自观察（N 个窗口不会重复切换）。
//! - **200ms 轮询遍历所有登记的窗口**：Explorer 重启重挂、经典固定的最小化守护、贴边隐藏（状态机按窗口各一份），以及廉价的
//!   样式自愈（防御）。GPUI 建窗后不再改写 ex style，旧版为对抗 tao `apply_diff` 写的排队重申不再需要。
//! - **唤起与重置作用于所有登记的窗口**：托盘左键 / 单实例 / Dock 唤起 / 贴边隐藏中被激活 → [`bring_all_to_front`]；
//!   托盘「重置位置」→ [`reset_all`]（错开排列、解除锁定）。
//! - **几何与锁定按屏幕组合持久化**，经 [`store`]（core 的窗口属性表 `window_layouts`；默认清单窗口另外登记屏幕组合）。
//!
//! macOS 走同一套状态机与 200ms 轮询（`platform::Hwnd` 是 NSWindow 窗口号）：几何锁定 = `setMovable:NO` + 去掉可缩放，
//! 置顶 = `NSFloatingWindowLevel`，`ToolWindow` 是 no-op（程序坞 / ⌘Tab 是整个应用的），没有嵌入桌面；被最小化即还原、
//! 托盘唤起临时置顶、贴边隐藏照常（`NSEvent.mouseLocation` 给全局光标，比 2.5.0 只按鼠标移入移出更准）。
//! 贴边隐藏中被 Dock / ⌘Tab 激活时按托盘唤起拉回（[`on_activated`]）；往屏幕上方藏时 AppKit 可能把窗口挡在菜单栏下，
//! 平台层据实返回失败、状态机回滚成显示态（有菜单栏的屏幕上窗口本来就贴不到顶边）。
//!
//! **线程 / 借用约束**：所有 Win32 / AppKit 写操作都在 `cx.spawn` 的 async 块里、`cx.update` 闭包之外执行
//! （`platform::Hwnd` 原始调用会同步给 GPUI 窗口过程发消息，App 被借用时 GPUI 处理不了）；`cx.update` 里只调
//! `machine` 的纯函数算计划，以及只读的 `Hwnd` 查询。

pub mod machine;
pub mod store;

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, AsyncApp, EntityId, Global, Subscription, Task, Window,
};
use mini_todo_core::db::{WindowPosition, WindowSize};
use mini_todo_core::settings::{self, RuntimePrefs};
use mini_todo_core::window_logic::{self, MonitorBounds, WindowPersistState};

pub use self::machine::TargetMode;
use self::machine::{AutoHideMove, ModeState, Op, Ops, TickFacts, TickPlan};
use crate::db::Db;
use crate::platform::{self, Hwnd, MonitorInfo, PhysRect};
use crate::services::snap;
use crate::state::AppState;
use crate::ui::message::{self, MessageKind};
use crate::windows::block::BlockKey;
use crate::windows::snap::SnapKey;

/// 贴边隐藏 / 嵌入守护的轮询间隔（旧版 200ms 轮询线程）
pub const TICK_INTERVAL: Duration = Duration::from_millis(200);
/// 窗口被系统 / 程序挪动、改尺寸后保存窗口状态的防抖（用户自绘拖动 / 缩放松手后直接保存，不经防抖）
pub const SAVE_DEBOUNCE: Duration = Duration::from_millis(500);

/// 一个登记的窗口
struct Block {
    /// `None`：建窗前 [`prepare`] 已放好锁定状态、窗口还没建好（[`register`] 补上）
    handle: Option<AnyWindowHandle>,
    /// 原生窗口（Windows HWND / macOS 窗口号；Linux 没有）
    hwnd: Option<Hwnd>,
    /// 根视图（锁定变化时只通知它重绘）
    view: Option<EntityId>,
    state: ModeState,
    /// 保存窗口状态的防抖代次
    save_generation: u64,
}

impl Block {
    fn new(fixed: bool, top_on_wake: bool) -> Self {
        Self {
            handle: None,
            hwnd: None,
            view: None,
            state: ModeState::new(fixed, top_on_wake),
            save_generation: 0,
        }
    }
}

/// 所有桌面块窗口的模式运行时状态（Global）
#[derive(Default)]
pub struct WindowMode {
    blocks: BTreeMap<BlockKey, Block>,
    /// 200ms 轮询是否在跑（没有窗口时停，再有窗口登记时重新启动）
    tick_running: bool,
    /// 集中观察 `AppState` 的嵌入偏好与运行时偏好（第一个窗口登记时订阅）
    prefs_observer: Option<Subscription>,
    last_embed: bool,
    last_prefs: Option<RuntimePrefs>,
}

impl Global for WindowMode {}

fn mode(cx: &mut App) -> &mut WindowMode {
    cx.default_global::<WindowMode>()
}

fn block(cx: &App, key: BlockKey) -> Option<&Block> {
    cx.try_global::<WindowMode>()?.blocks.get(&key)
}

fn block_mut(cx: &mut App, key: BlockKey) -> Option<&mut Block> {
    mode(cx).blocks.get_mut(&key)
}

fn with_state(cx: &mut App, key: BlockKey, f: impl FnOnce(&mut ModeState)) {
    if let Some(block) = block_mut(cx, key) {
        f(&mut block.state);
    }
}

// ----------------------------------------------------------------------
// 登记
// ----------------------------------------------------------------------

/// 建窗前调用：放好这个窗口的锁定状态（`open_window` 建窗时同步画第一帧，第一帧就按锁定外观画）。已登记过的会被重置
pub fn prepare(cx: &mut App, key: BlockKey, fixed: bool) {
    let top_on_wake = AppState::global(cx).read(cx).runtime_prefs.top_on_wake;
    mode(cx).blocks.insert(key, Block::new(fixed, top_on_wake));
}

/// 窗口建好后调用：记下句柄、原生窗口与根视图，加入吸附组（与收纳箱同一组），启动 200ms 轮询（已在跑就不重复启动），
/// 开始集中观察偏好。窗口侧总是从普通模式开始：锁定的窗口由调用方随后 [`apply_mode`] 进入固定。没有 [`prepare`] 过时按未锁定登记
pub fn register(
    cx: &mut App,
    key: BlockKey,
    handle: AnyWindowHandle,
    hwnd: Option<Hwnd>,
    view: EntityId,
) {
    let top_on_wake = AppState::global(cx).read(cx).runtime_prefs.top_on_wake;
    let mode = mode(cx);
    let block = mode
        .blocks
        .entry(key)
        .or_insert_with(|| Block::new(false, top_on_wake));
    block.handle = Some(handle);
    block.hwnd = hwnd;
    block.view = Some(view);
    let start_tick = hwnd.is_some() && !mode.tick_running;
    if start_tick {
        mode.tick_running = true;
        start_tick_loop(cx);
    }
    snap::register(
        cx,
        SnapKey::Block(key),
        snap::Member {
            hwnd,
            view,
            suppressed: snap_suppressed,
        },
    );
    ensure_prefs_observer(cx);
}

/// 吸附组里清单 / 日历窗口"暂时不算目标"：贴边藏在屏幕边缘时（锁定的照样算）
fn snap_suppressed(cx: &App, key: SnapKey) -> bool {
    matches!(key, SnapKey::Block(block) if is_edge_hidden(cx, block))
}

/// 窗口关闭后调用（或建窗失败时撤掉 [`prepare`]）：之后的保存 / 切换 / 唤起都不再作用于它，并退出吸附组
pub fn unregister(cx: &mut App, key: BlockKey) {
    mode(cx).blocks.remove(&key);
    snap::unregister(cx, SnapKey::Block(key));
}

/// 已登记且窗口已建好
pub fn is_registered(cx: &App, key: BlockKey) -> bool {
    block(cx, key).is_some_and(|b| b.handle.is_some())
}

/// 已登记且窗口已建好的全部窗口（按 [`BlockKey`] 排序：默认清单在前，日历最后）
pub fn registered(cx: &App) -> Vec<BlockKey> {
    cx.try_global::<WindowMode>()
        .map(|m| {
            m.blocks
                .iter()
                .filter(|(_, b)| b.handle.is_some())
                .map(|(key, _)| *key)
                .collect()
        })
        .unwrap_or_default()
}

fn handle_of(cx: &App, key: BlockKey) -> Option<AnyWindowHandle> {
    block(cx, key)?.handle
}

/// 只通知这个窗口的根视图重绘（锁定状态不在 `AppState` 里，窗口观察不到它的变化）
fn notify_view(cx: &mut App, key: BlockKey) {
    if let Some(view) = block(cx, key).and_then(|b| b.view) {
        cx.notify(view);
    }
}

// ----------------------------------------------------------------------
// 查询
// ----------------------------------------------------------------------

/// 窗口是否锁定（用户可见的状态：标题栏锁按钮；经典固定与嵌入桌面都为 true）。没登记的窗口为 false
pub fn is_fixed(cx: &App, key: BlockKey) -> bool {
    block(cx, key).is_some_and(|b| b.state.fixed)
}

/// 窗口是否按经典固定的外观画（锁定且没有嵌入桌面：无圆角、无描边）
pub fn is_classic_fixed(cx: &App, key: BlockKey) -> bool {
    let embed = AppState::global(cx).read(cx).fixed_embed_desktop;
    machine::classic_fixed_look(is_fixed(cx, key), embed)
}

/// 窗口正藏在屏幕边缘（经典固定 + 贴边隐藏）：不算吸附目标
pub fn is_edge_hidden(cx: &App, key: BlockKey) -> bool {
    block(cx, key).is_some_and(|b| b.state.is_edge_hidden())
}

// ----------------------------------------------------------------------
// 模式切换
// ----------------------------------------------------------------------

/// 一组窗口操作（立即执行的 Win32 / AppKit；只在 async 块里、`cx.update` 之外调用）
fn run_ops(hwnd: Hwnd, ops: Ops) {
    for op in ops.steps {
        run_op(hwnd, op);
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

/// 把窗口 `key` 切到 `target`（异步：Win32 部分在借用之外执行）。窗口没登记时 `Err("… 不存在")`
pub fn apply_mode(cx: &mut App, key: BlockKey, target: TargetMode) -> Task<Result<(), String>> {
    cx.spawn(async move |cx| {
        let planned = cx.update(|cx| {
            let prefs = AppState::global(cx).read(cx).runtime_prefs;
            let block = block_mut(cx, key)?;
            block.handle?;
            let hwnd = block.hwnd;
            let ops = block.state.plan_mode(prefs, target);
            if key == BlockKey::DEFAULT_LIST {
                // 托盘「固定模式」勾选跟着默认清单窗口（两种固定都勾选；任务第 5 步去掉这一项）
                crate::services::tray::set_fixed_checked(cx, target != TargetMode::Normal);
            }
            Some((hwnd, ops))
        });
        let Some((hwnd, ops)) = planned else {
            return Err(format!("{key} 不存在"));
        };
        log::info!("[window] {key} 切换到 {target:?}");
        if let Some(hwnd) = hwnd {
            run_ops(hwnd, ops);
        }
        Ok(())
    })
}

/// 锁定 / 解除锁定窗口 `key`（标题栏锁按钮、托盘「固定模式」、重置位置）：按"嵌入桌面"偏好进入经典固定 / 嵌入桌面，
/// 或退回普通模式；切完保存窗口状态。失败在该窗口里提示 `切换固定模式失败：…`。窗口没登记 / 已是目标状态时什么都不做
pub fn set_fixed(cx: &mut App, key: BlockKey, fixed: bool) {
    let changed = match block_mut(cx, key) {
        Some(block) if block.handle.is_some() => {
            let changed = block.state.fixed != fixed;
            block.state.fixed = fixed;
            changed
        }
        _ => {
            log::warn!("[window] 切换固定模式失败：{key} 不存在");
            return;
        }
    };
    if !changed {
        return;
    }
    notify_view(cx, key);
    let embed = AppState::global(cx).read(cx).fixed_embed_desktop;
    let task = apply_mode(cx, key, TargetMode::for_state(fixed, embed));
    cx.spawn(async move |cx| {
        let result = task.await;
        cx.update(|cx| match result {
            Ok(()) => save_window_state(cx, key),
            Err(e) => {
                log::error!("[window] 切换固定模式失败: {e}");
                if let Some(handle) = handle_of(cx, key) {
                    message::show_in(
                        handle,
                        cx,
                        MessageKind::Error,
                        message::error_text("切换固定模式失败", &e),
                    );
                }
            }
        });
    })
    .detach();
}

/// 切换窗口 `key` 的锁定（旧 `toggleFixedMode`）
pub fn toggle_fixed(cx: &mut App, key: BlockKey) {
    let fixed = is_fixed(cx, key);
    set_fixed(cx, key, !fixed);
}

/// "嵌入桌面"偏好变了（设置窗口 / 同步 / 导入）：每个锁定的窗口直接在两种固定之间切换，不保存窗口状态
/// （旧 `set_fixed_embed_desktop` 的后端部分）；未锁定的窗口下次锁定时按新偏好进入
pub fn on_embed_preference_changed(cx: &mut App, enabled: bool) {
    let targets: Vec<(BlockKey, TargetMode)> = mode(cx)
        .blocks
        .iter()
        .filter(|(_, b)| b.handle.is_some())
        .filter_map(|(key, b)| b.state.embed_switch(enabled).map(|t| (*key, t)))
        .collect();
    for (key, target) in targets {
        apply_mode(cx, key, target).detach();
    }
}

/// 运行时偏好变化（设置窗口改了贴边隐藏 / 唤起置顶、导入、同步应用了远端设置、保存窗口状态后）：刷新每个窗口的状态机，
/// 有功能被关掉时下个 tick 收尾（旧 `reload_runtime_prefs`）
pub fn apply_runtime_prefs(cx: &mut App, prefs: RuntimePrefs) {
    for block in mode(cx).blocks.values_mut() {
        block.state.apply_runtime_prefs(prefs);
    }
}

/// 集中观察 `AppState`：嵌入偏好 / 运行时偏好变了就对所有窗口收尾（只订阅一次）
fn ensure_prefs_observer(cx: &mut App) {
    if mode(cx).prefs_observer.is_some() {
        return;
    }
    let state = AppState::global(cx);
    let read = |s: &AppState| (s.fixed_embed_desktop, s.runtime_prefs);
    let (embed, prefs) = read(state.read(cx));
    let subscription = cx.observe(&state, move |state, cx| {
        let (embed, prefs) = read(state.read(cx));
        let mode = mode(cx);
        let embed_changed = mode.last_embed != embed;
        let prefs_changed = mode.last_prefs != Some(prefs);
        mode.last_embed = embed;
        mode.last_prefs = Some(prefs);
        if embed_changed {
            cx.defer(move |cx| on_embed_preference_changed(cx, embed));
        }
        if prefs_changed {
            cx.defer(move |cx| apply_runtime_prefs(cx, prefs));
        }
    });
    let mode = mode(cx);
    mode.prefs_observer = Some(subscription);
    mode.last_embed = embed;
    mode.last_prefs = Some(prefs);
}

// ----------------------------------------------------------------------
// 200ms 轮询（Windows / macOS）：嵌入重挂（仅 Windows）/ 经典固定守护 / 贴边隐藏，遍历所有登记的窗口
// ----------------------------------------------------------------------

fn start_tick_loop(cx: &mut App) {
    cx.spawn(async move |cx| {
        loop {
            cx.background_executor().timer(TICK_INTERVAL).await;
            let Some(jobs) = cx.update(plan_tick) else {
                cx.update(|cx| mode(cx).tick_running = false);
                break;
            };
            for job in jobs {
                run_tick(cx, job);
            }
        }
    })
    .detach();
}

/// 一个窗口这一 tick 的计划
struct TickJob {
    key: BlockKey,
    hwnd: Hwnd,
    plan: TickPlan,
}

/// 读状态、算出这一 tick 每个窗口要做的事（在 `cx.update` 里；只用只读的 `Hwnd` 查询）。没有存活的窗口时返回 `None`（结束轮询）
fn plan_tick(cx: &mut App) -> Option<Vec<TickJob>> {
    let alive: Vec<(BlockKey, Hwnd)> = mode(cx)
        .blocks
        .iter()
        .filter_map(|(key, b)| Some((*key, b.hwnd?)))
        .filter(|(_, hwnd)| hwnd.is_alive())
        .collect();
    if alive.is_empty() {
        return None;
    }
    let monitors = platform::monitors(cx);
    let mut cursor = None;
    let now = Instant::now();
    let mut jobs = Vec::with_capacity(alive.len());
    for (key, hwnd) in alive {
        let Some(block) = block_mut(cx, key) else {
            continue;
        };
        let facts = tick_facts(&block.state, hwnd, &monitors, &mut cursor);
        let plan = block.state.plan_tick(&facts, now);
        jobs.push(TickJob { key, hwnd, plan });
    }
    Some(jobs)
}

/// 按窗口当前模式只读需要的实况（嵌入：要不要重挂；经典固定：最小化、样式、贴边隐藏要的几何与光标）
fn tick_facts(
    state: &ModeState,
    hwnd: Hwnd,
    monitors: &[MonitorInfo],
    cursor: &mut Option<Option<(i32, i32)>>,
) -> TickFacts {
    let mut facts = TickFacts::default();
    if state.desktop {
        facts.needs_reattach = platform::embed::needs_reattach(hwnd);
    } else if state.classic_fixed {
        facts.minimized = hwnd.is_minimized();
        // macOS 没有逐窗口的任务栏样式（ex_style 恒 0）：不自愈
        facts.lacks_tool_window =
            cfg!(windows) && window_logic::lacks_tool_window_style(hwnd.ex_style());
        if state.auto_hide.enabled
            && let Some(rect) = hwnd.client_rect()
        {
            facts.client_rect = Some(rect);
            facts.monitor = monitor_for_rect(monitors, rect).map(|m| monitor_bounds(&m));
            facts.cursor = *cursor.get_or_insert_with(platform::cursor_position_physical);
        }
    }
    facts
}

fn run_tick(cx: &mut AsyncApp, job: TickJob) {
    let TickJob { key, hwnd, plan } = job;
    for op in plan.ops.steps {
        // 嵌入已被退出（排队期间切走了）时不再挂
        if op == Op::Attach && !cx.update(|cx| block(cx, key).is_some_and(|b| b.state.desktop)) {
            continue;
        }
        run_op(hwnd, op);
    }
    match plan.auto_hide {
        None => {}
        Some(AutoHideMove::Hide { x, y }) => {
            if hwnd.set_client_position(x, y) {
                hwnd.set_topmost(false);
                cx.update(|cx| with_state(cx, key, ModeState::hidden));
            } else {
                // 移动失败：回滚为显示态
                cx.update(|cx| with_state(cx, key, ModeState::hide_failed));
            }
        }
        Some(AutoHideMove::Restore { x, y, topmost }) => {
            if hwnd.set_client_position(x, y) {
                if topmost {
                    hwnd.set_topmost(true);
                    cx.update(|cx| with_state(cx, key, |s| s.desired_topmost = true));
                }
            } else {
                cx.update(|cx| with_state(cx, key, ModeState::restore_failed));
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
// 唤起 / 重置（作用于所有登记的窗口）
// ----------------------------------------------------------------------

/// 托盘左键、单实例唤起、macOS Dock 唤起、贴边隐藏中被激活（旧 `bring_main_window_to_front`）：显示并激活所有登记的窗口。
/// 每个窗口按自己的模式：嵌入桌面只显示 + 聚焦；其它模式贴边隐藏开着时给 3s 宽限并拉回 → 还原 → 临时置顶 600ms（贴边唤起态
/// 需要保持置顶时不撤）→ 激活。默认清单窗口最后激活（在最上面、拿到焦点）；它不在（关掉了）时重新打开
pub fn bring_all_to_front(cx: &mut App) {
    // macOS：从菜单栏图标 / 单实例 / Dock 唤起时本应用多半不在前台，先切到前台，否则窗口会落在当前前台应用的
    // 窗口之后（重复调用无害）。Windows 由窗口自己 SetForegroundWindow
    #[cfg(target_os = "macos")]
    cx.activate(true);
    let opened = ensure_default_list(cx);
    let mut keys: Vec<BlockKey> = registered(cx)
        .into_iter()
        .filter(|key| !(opened && *key == BlockKey::DEFAULT_LIST))
        .collect();
    keys.reverse();
    wake(cx, keys);
}

/// 只唤起一个窗口（托盘「打开设置」/ 应用菜单「设置…」先把作为打开方的默认清单窗口带到前面）；
/// 默认清单窗口不在时重新打开
pub fn bring_to_front(cx: &mut App, key: BlockKey) {
    #[cfg(target_os = "macos")]
    cx.activate(true);
    if key == BlockKey::DEFAULT_LIST && ensure_default_list(cx) {
        return;
    }
    if is_registered(cx, key) {
        wake(cx, vec![key]);
    }
}

/// 默认清单窗口（原主窗口）不在（关掉了）时重新打开；返回是否刚打开（刚打开的不用再唤起）
fn ensure_default_list(cx: &mut App) -> bool {
    if is_registered(cx, BlockKey::DEFAULT_LIST) {
        return false;
    }
    crate::windows::main_window::open(cx);
    true
}

/// 按顺序唤起 `keys`（后面的在上面）：一次唤起只开一个代次，600ms 后各窗口按自己的贴边状态决定撤不撤置顶
fn wake(cx: &mut App, keys: Vec<BlockKey>) {
    if keys.is_empty() {
        return;
    }
    cx.spawn(async move |cx| {
        let now = Instant::now();
        let targets = cx.update(|cx| {
            keys.iter()
                .filter_map(|key| {
                    let block = block_mut(cx, *key)?;
                    let handle = block.handle?;
                    let hwnd = block.hwnd;
                    Some((*key, handle, hwnd, block.state.wake(now)))
                })
                .collect::<Vec<_>>()
        });
        let mut raised: Vec<(BlockKey, Hwnd)> = Vec::new();
        for (key, handle, hwnd, wake) in targets {
            if let Some(hwnd) = hwnd {
                if let Some((x, y)) = wake.anchor {
                    hwnd.set_client_position(x, y);
                }
                run_op(hwnd, Op::Restore);
                if !wake.desktop {
                    hwnd.set_topmost(true);
                    raised.push((key, hwnd));
                }
                hwnd.set_foreground();
            }
            let _ = cx.update_window(handle, |_, window, _| window.activate_window());
        }
        if raised.is_empty() {
            return;
        }
        let generation = window_logic::begin_tray_wake();
        cx.background_executor()
            .timer(window_logic::TRAY_WAKE_TOPMOST_DURATION)
            .await;
        let release = cx.update(|cx| {
            let latest = window_logic::is_latest_tray_wake(generation);
            raised
                .iter()
                .filter(|(key, _)| {
                    block_mut(cx, *key).is_some_and(|b| b.state.release_wake_topmost(latest))
                })
                .map(|(_, hwnd)| *hwnd)
                .collect::<Vec<_>>()
        });
        for hwnd in release {
            hwnd.set_topmost(false);
        }
    })
    .detach();
}

/// 窗口被激活时调用（窗口自己的 `observe_window_activation` 里 `cx.defer`）。macOS：贴边隐藏中的窗口被点 Dock 图标 /
/// ⌘Tab 切回来激活时按托盘唤起处理（拉回锚点、临时置顶，[`bring_all_to_front`]）——GPUI 只在没有可见窗口时才回调 Dock 唤起
/// （`mac_menu::on_reopen`），藏在屏幕边缘的窗口仍算可见，不处理的话窗口成了 key 窗口却停在屏幕外。
/// Windows 的经典固定没有任务栏 / Alt+Tab 入口，什么都不做
pub fn on_activated(cx: &mut App, key: BlockKey) {
    if !cfg!(target_os = "macos") {
        return;
    }
    if is_edge_hidden(cx, key) {
        log::info!("[window] 贴边隐藏中的{key}被激活，拉回");
        bring_all_to_front(cx);
    }
}

/// 托盘「重置位置」（旧 `reset_window_impl` + 前端 `tray-reset-window`）：所有登记的窗口清贴边状态、撤置顶，在主显示器左上起
/// 错开排列（第一个 = 以前的主窗口：10% 边距、380×600 逻辑，见 [`machine::reset_rect`]）；锁定的退回普通模式；然后保存。
/// 默认清单窗口不在（关掉了）时只重新打开它。
///
/// 屏幕组合 ID 只在启动时算（旧版也是：运行中插拔显示器不切换配置），旧版在重置后重跑 `initSettings` 重算；这里在保存
/// 之前重算，重置后的几何写进**当前**屏幕组合的记录（不会写到启动时那个已经过时的组合上）
pub fn reset_all(cx: &mut App) {
    if ensure_default_list(cx) {
        return;
    }
    let primary = platform::primary_monitor(cx);
    let keys = registered(cx);
    cx.spawn(async move |cx| {
        for (index, key) in keys.iter().enumerate() {
            let placed = cx.update(|cx| {
                let block = block_mut(cx, *key)?;
                block.state.reset_runtime();
                Some((block.handle?, block.hwnd))
            });
            let Some((handle, hwnd)) = placed else {
                continue;
            };
            match (hwnd, &primary) {
                (Some(hwnd), Some(monitor)) => {
                    let rect = machine::reset_rect(monitor_bounds(monitor), monitor.scale, index);
                    hwnd.set_topmost(false);
                    hwnd.set_client_rect(rect);
                }
                _ => {
                    let _ = cx.update_window(handle, |_, window, _| {
                        window.resize(gpui_kit::size(
                            gpui_kit::px(window_logic::RESET_WINDOW_WIDTH as f32),
                            gpui_kit::px(window_logic::RESET_WINDOW_HEIGHT as f32),
                        ));
                    });
                }
            }
        }
        cx.update(|cx| {
            refresh_screen_config_id(cx);
            for key in keys {
                if is_fixed(cx, key) {
                    // 锁定的窗口不允许挪：重置后退回普通模式（set_fixed 切换后会保存）
                    set_fixed(cx, key, false);
                } else {
                    save_window_state(cx, key);
                }
            }
        });
    })
    .detach();
}

/// 重新计算当前屏幕组合 ID（旧 `initSettings` 的 `generateScreenConfigId`）；之后的保存写进新组合的记录
pub fn refresh_screen_config_id(cx: &mut App) {
    let id = platform::screen_config_id(cx);
    let state = AppState::global(cx);
    let previous = state.read(cx).current_screen_config_id.clone();
    if previous == id {
        return;
    }
    log::info!("[window] 屏幕组合 ID：{previous} → {id}");
    state.update(cx, |state, cx| {
        state.current_screen_config_id = id;
        cx.notify();
    });
}

// ----------------------------------------------------------------------
// 保存 / 恢复窗口状态
// ----------------------------------------------------------------------

/// 窗口被系统 / 程序挪动、改尺寸后防抖 500ms 保存（旧 `debouncedSaveState`）；用户自绘拖动 / 缩放中不调（松手直接保存）
pub fn schedule_save(cx: &mut App, key: BlockKey) {
    let Some(generation) = block_mut(cx, key).map(|b| {
        b.save_generation += 1;
        b.save_generation
    }) else {
        return;
    };
    cx.spawn(async move |cx| {
        cx.background_executor().timer(SAVE_DEBOUNCE).await;
        cx.update(|cx| {
            if block(cx, key).is_some_and(|b| b.save_generation == generation) {
                save_window_state(cx, key);
            }
        });
    })
    .detach();
}

/// 立即保存窗口 `key` 的状态（取消进行中的防抖）
pub fn save_window_state(cx: &mut App, key: BlockKey) {
    let Some(block) = block_mut(cx, key) else {
        return;
    };
    block.save_generation += 1;
    let (Some(handle), hwnd) = (block.handle, block.hwnd) else {
        return;
    };
    cx.spawn(async move |cx| {
        let rect = match hwnd.and_then(Hwnd::client_rect) {
            Some(rect) => Some(rect),
            None => cx
                .update_window(handle, |_, window, _| {
                    platform::client_rect_physical(window)
                })
                .ok()
                .flatten(),
        };
        let Some(rect) = rect else {
            return;
        };
        cx.update(|cx| save_window_state_with(cx, key, rect));
    })
    .detach();
}

/// 用已经读好的客户区几何保存（窗口关闭时窗口马上就没了，几何要在关闭回调里同步读）：当前屏幕组合下的几何与锁定经
/// [`store`] 落库（贴边隐藏中存锚点，不把屏幕外的隐藏位置存进库）。只写窗口布局（默认清单窗口另外登记屏幕组合），
/// 不再写旧的全局设置键（`is_fixed` / `window_position` / `window_size` / 主题…，见 [`store`] 模块文档）。
///
/// 几何与锁定在调用时就取好（贴边状态、锁定此刻的值；窗口随后关掉、注销也照样保存）
pub fn save_window_state_with(cx: &mut App, key: BlockKey, rect: PhysRect) {
    let Some((persist, fixed)) = block(cx, key).map(|b| {
        let persist = b.state.auto_hide.persist_state(
            WindowPosition {
                x: rect.x,
                y: rect.y,
            },
            WindowSize {
                width: rect.width.max(0) as u32,
                height: rect.height.max(0) as u32,
            },
        );
        (persist, b.state.fixed)
    }) else {
        return;
    };
    persist_window_state(cx, key, persist, fixed);
}

/// [`save_window_state_with`] 的落库部分（`persist`：贴边隐藏中已换成锚点的几何）
fn persist_window_state(cx: &mut App, key: BlockKey, persist: WindowPersistState, fixed: bool) {
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
    let placement = PhysRect::new(
        persist.position.x,
        persist.position.y,
        persist.size.width as i32,
        persist.size.height as i32,
    );
    let db = Db::get(cx);
    let task = cx
        .background_executor()
        .spawn(async move { store::save(&db, key, &screen_id, placement, fixed) });
    cx.spawn(async move |_| {
        if let Err(e) = task.await {
            log::warn!("[window] 保存{key}的状态失败: {e}");
        }
    })
    .detach();
}

/// 启动时恢复窗口用的信息（建窗前同步读出：一次按主键的查询，避免窗口先居中再跳）
#[derive(Debug, Clone, Default)]
pub struct StartupGeometry {
    pub screen_config_id: String,
    /// 该屏幕组合记下的客户区（物理像素）
    pub saved: Option<PhysRect>,
    pub is_fixed: bool,
    pub embed_desktop: bool,
}

/// 读窗口 `key` 在当前屏幕组合下的记录（旧 `initSettings` 的 `get_screen_config`；经 [`store::load`]）
pub fn read_startup_geometry(cx: &App, key: BlockKey) -> StartupGeometry {
    let screen_config_id = platform::screen_config_id(cx);
    let db = Db::get(cx);
    let saved = store::load(&db, key, &screen_config_id)
        .map_err(|e| log::warn!("[window] 读取{key}的屏幕配置失败: {e}"))
        .ok()
        .flatten();
    StartupGeometry {
        embed_desktop: settings::fixed_embed_desktop(&db),
        is_fixed: saved.is_some_and(|p| p.fixed),
        saved: saved.and_then(|p| p.rect),
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

/// 窗口关闭请求：同步读几何并保存（旧 `onCloseRequested`）
pub fn on_close_requested(window: &Window, cx: &mut App, key: BlockKey) {
    if let Some(rect) = platform::client_rect_physical(window) {
        save_window_state_with(cx, key, rect);
    }
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
