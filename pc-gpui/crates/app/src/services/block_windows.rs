//! 桌面块窗口（待办清单窗口 / 日历窗口）的生命周期（任务 `11-09-multi-list-windows` 第 4 步；收纳箱的对应部分在
//! `services::fences`）。窗口模式、几何保存 / 恢复、唤起与重置在 `services::window_mode`，这里只管开 / 关 / 显示 / 隐藏。
//!
//! | 时机 | 做什么 |
//! |---|---|
//! | 启动（[`start`]） | 迁移 ui-state、**同步**读清单与窗口属性（建窗前就要知道深色 / 视图，第一帧按它画）→ 默认清单窗口（总是显示，清掉它的隐藏）→ 显示中的日历窗口 → 其它没隐藏的清单窗口；之后观察清单列表、起 5 秒变更轮询 |
//! | 关闭（Alt+F4 / ⌘W，[`request_close`]） | 保存几何、记住隐藏、原生隐藏窗口，**不注销**（唤起、重置、轮询、吸附照常找得到它；隐藏的窗口自动不算吸附目标）；有模态子窗口时不关、把子窗口带到前台。Linux 不能程序化隐藏窗口，真的关掉（最后一个窗口关掉就退出，见 `lib.rs`） |
//! | 托盘左键 / 单实例 / Dock（`window_mode::bring_all_to_front` → [`prepare_wake_all`]） | 清单窗口全部显示：还没开过窗的开窗、隐藏的清掉隐藏；日历只在「显示日历」开着时唤起（[`should_wake`]） |
//! | 清单列表变化（`AppState::lists_revision` → [`reconcile`]） | 同步 / cloud 新建、本机没有窗口记录（`window_prefs` 没有这一行）的清单自动开窗：主显示器上找空位、380×600、未锁定、浅色、列表视图（都是没有记录时的默认值）；已删除清单的窗口关掉 |
//! | 「显示日历」（设置 → 常规、托盘勾选，[`set_calendar_visible`]） | 写日历窗口的隐藏属性；第一次显示且当前屏幕组合没有布局时贴默认清单窗口右侧、与它等高（`calendar_window::logic::first_rect`） |
//!
//! **隐藏的窗口启动时不建**（第一次显示时才建：省掉用不到的窗口资源，日历隐藏时也不去拉节假日）；建好之后关闭只是隐藏。
//! 窗口登记：`WindowManager` 的 `WindowKind::List` / `WindowKind::Calendar`，key = core 窗口属性表的 key（`list:<id>` / `calendar`）。

use std::collections::HashMap;
use std::time::Duration;

use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, Context, Entity, Global, Render, SharedString,
    Subscription, WeakEntity, Window,
};
use mini_todo_core::fence::{self, PxRect};
use mini_todo_core::{lists, settings, window_prefs};

use crate::db::Db;
use crate::platform::{self, MonitorInfo, PhysRect};
use crate::services::window_mode;
use crate::state::AppState;
use crate::windows::block::{BlockKey, DEFAULT_LIST_ID};
use crate::windows::calendar_window::{self, CalendarWindow};
use crate::windows::list_window::{self, ListWindow};
use crate::windows::{WindowKind, WindowManager, WindowSpec, logical_size};

/// 变更计数轮询间隔（旧主窗口的 5s 轮询，现在是整个应用一个）
const POLL_INTERVAL: Duration = Duration::from_secs(5);
/// 默认清单窗口的系统标题（任务栏 / Alt+Tab；e2e 脚本按它找窗口）。其它清单窗口用清单名，日历窗口用 [`CALENDAR_TITLE`]
pub const DEFAULT_LIST_TITLE: &str = "Mini Todo";
pub const CALENDAR_TITLE: &str = "日历";
/// Linux（只用于 CI / 截图）不能程序化隐藏窗口：关闭就真的关掉
const CAN_HIDE: bool = cfg!(any(windows, target_os = "macos"));

/// 运行时状态（Global）
#[derive(Default)]
struct BlockWindows {
    /// 清单窗口的视图（新建清单后进入改名等要直接调用视图的地方用）
    list_views: HashMap<i64, WeakEntity<ListWindow>>,
    lists_observer: Option<Subscription>,
    last_revision: u64,
    /// 托盘「新增待办清单」进行中：给新清单找好的位置（光标所在显示器上的空位）。清单建好、`lists_revision` 变化后的对账
    /// 先于新建的回调执行，对账按它开窗（见 [`create_list`]）
    creating: Vec<PhysRect>,
}

impl Global for BlockWindows {}

fn runtime(cx: &mut App) -> &mut BlockWindows {
    cx.default_global::<BlockWindows>()
}

// ----------------------------------------------------------------------
// 启动
// ----------------------------------------------------------------------

/// 启动时（`lib.rs::run`，`AppState` / `WindowManager` 建好之后、托盘之前）：开窗、观察清单列表、起变更轮询、检查更新
pub fn start(cx: &mut App) {
    load_startup_snapshot(cx);
    // 默认清单窗口总是显示（PRD R9：它的 hidden 启动时忽略）
    open_list(cx, DEFAULT_LIST_ID, Placement::Restore);
    mark_shown(cx, BlockKey::DEFAULT_LIST);
    if AppState::global(cx).read(cx).calendar_visible() {
        open_calendar(cx);
    }
    reconcile(cx);
    observe_lists(cx);
    start_poll(cx);
    crate::services::update_check::check_for_updates(cx);
}

/// 建窗前同步读（阻塞，只在启动时一次）：迁移 ui-state.json 的展开全部 → 清单、窗口属性、全局偏好、当前屏幕组合
fn load_startup_snapshot(cx: &mut App) {
    let db = Db::get(cx);
    crate::state::migrate_legacy_ui_state(&db);
    let lists = lists::list(&db).unwrap_or_else(|e| {
        log::error!("[block] 读取待办清单失败: {e}");
        Vec::new()
    });
    let prefs = window_prefs::list_prefs(&db).unwrap_or_else(|e| {
        log::error!("[block] 读取窗口属性失败: {e}");
        Vec::new()
    });
    let runtime_prefs = settings::runtime_prefs(&db);
    let embed = settings::fixed_embed_desktop(&db);
    let screen_id = platform::screen_config_id(cx);
    AppState::global(cx).update(cx, |state, cx| {
        state.set_startup_snapshot(lists, prefs);
        state.current_screen_config_id = screen_id;
        state.fixed_embed_desktop = embed;
        state.runtime_prefs = runtime_prefs;
        state.auto_hide_enabled = runtime_prefs.auto_hide_enabled;
        state.top_on_wake = runtime_prefs.top_on_wake;
        cx.notify();
    });
}

/// 清单列表变化（加载、同步、本机新建 / 改名 / 删除）后对账开 / 关窗口
fn observe_lists(cx: &mut App) {
    let state = AppState::global(cx);
    let revision = state.read(cx).lists_revision;
    let subscription = cx.observe(&state, |state, cx| {
        let revision = state.read(cx).lists_revision;
        let rt = runtime(cx);
        if rt.last_revision != revision {
            rt.last_revision = revision;
            cx.defer(reconcile);
        }
    });
    let rt = runtime(cx);
    rt.last_revision = revision;
    rt.lists_observer = Some(subscription);
}

/// 5s 变更轮询（有清单 / 日历窗口开着模态子窗口时跳过，同以前的主窗口）
fn start_poll(cx: &mut App) {
    cx.spawn(async move |cx| {
        loop {
            cx.background_executor().timer(POLL_INTERVAL).await;
            cx.update(|cx| {
                if !WindowManager::any_block_has_modal_child(cx) {
                    AppState::global(cx).update(cx, |state, cx| state.refresh_if_changed(cx));
                }
            });
        }
    })
    .detach();
}

// ----------------------------------------------------------------------
// 开窗
// ----------------------------------------------------------------------

/// 当前屏幕组合下没有布局时窗口放哪
#[derive(Debug, Clone, Copy)]
enum Placement {
    /// 按布局恢复；没有布局时默认清单窗口在主显示器居中（同以前的主窗口），其它清单窗口在主显示器上找空位
    Restore,
    /// 没有布局时放在这里（托盘新建清单：光标所在显示器上找好的空位）
    At(PhysRect),
}

/// 打开清单 `list_id` 的窗口（已开着时返回它的视图，不重复开）
fn open_list(cx: &mut App, list_id: i64, placement: Placement) -> Option<Entity<ListWindow>> {
    let key = BlockKey::List(list_id);
    if window_mode::is_registered(cx, key) {
        return runtime(cx)
            .list_views
            .get(&list_id)
            .and_then(WeakEntity::upgrade);
    }
    let title: SharedString = if list_id == DEFAULT_LIST_ID {
        DEFAULT_LIST_TITLE.into()
    } else {
        AppState::global(cx).read(cx).list_name(list_id).into()
    };
    let fallback = match placement {
        Placement::At(rect) => rect,
        Placement::Restore if list_id == DEFAULT_LIST_ID => centered_on_primary(cx),
        Placement::Restore => empty_spot(cx, platform::primary_monitor(cx)),
    };
    let (_, view) = open_block(
        cx,
        key,
        WindowKind::List,
        title,
        (list_window::MIN_WIDTH, list_window::MIN_HEIGHT),
        fallback,
        move |window, cx| ListWindow::new(list_id, window, cx),
    )?;
    runtime(cx).list_views.insert(list_id, view.downgrade());
    Some(view)
}

/// 打开日历窗口（已开着时返回它）
fn open_calendar(cx: &mut App) -> Option<AnyWindowHandle> {
    if let Some(handle) = WindowManager::block_window(cx, BlockKey::Calendar) {
        return Some(handle);
    }
    let fallback = calendar_fallback(cx);
    open_block(
        cx,
        BlockKey::Calendar,
        WindowKind::Calendar,
        CALENDAR_TITLE.into(),
        (
            calendar_window::logic::MIN_WIDTH,
            calendar_window::logic::MIN_HEIGHT,
        ),
        fallback,
        CalendarWindow::new,
    )
    .map(|(handle, _)| handle)
}

/// 建一个桌面块窗口（以前 `main_window::open` 的做法，按 key 参数化）：按当前屏幕组合的布局恢复位置与尺寸（物理像素）与锁定，
/// 没有布局时放在 `fallback`。透明、自绘移动与缩放（建窗不可缩放）；全局偏好与锁定状态先就位（`open_window` 同步画第一帧）
fn open_block<V: Render + 'static>(
    cx: &mut App,
    key: BlockKey,
    kind: WindowKind,
    title: SharedString,
    min_size: (f32, f32),
    fallback: PhysRect,
    build: impl FnOnce(&mut Window, &mut Context<V>) -> V + 'static,
) -> Option<(AnyWindowHandle, Entity<V>)> {
    let geometry = window_mode::read_startup_geometry(cx, key);
    let monitors = platform::monitors(cx);
    // 有布局时客户区按布局的物理像素摆（中心不在任何显示器上 / 换算取整也以布局为准）；没有布局时按 `fallback`。
    // 两种情况都由 open_window 建窗后按客户区校正（GPUI 直接建出的客户区偏高半个底边框）
    let target = geometry.saved.unwrap_or(fallback);
    let (bounds, display) = match window_mode::logical_bounds_for(&monitors, target) {
        Some((bounds, display)) => (bounds, Some(display)),
        None => {
            let primary = platform::primary_monitor(cx);
            let size = logical_size(
                list_window::DEFAULT_WIDTH.max(min_size.0),
                list_window::DEFAULT_HEIGHT.max(min_size.1),
            );
            (
                crate::windows::centered_bounds(primary.as_ref(), None, size),
                primary.map(|m| m.display_id),
            )
        }
    };
    let mut spec = WindowSpec::new(title, bounds);
    spec.display_id = display;
    spec.client_rect = Some(target);
    spec.transparent = true;
    // 移动与缩放都是自绘的（标题栏 + 窗口内 8 个手柄，吸附对齐，`windows::block::BlockDrag`）：系统不再从边框缩放、
    // 不参与 Aero Snap（GPUI 的缩放热区按建窗时的 is_resizable 计算）。最小尺寸仍由 min_size 兜底
    spec.resizable = false;
    spec.min_size = Some(logical_size(min_size.0, min_size.1));

    window_mode::prepare(cx, key, geometry.is_fixed);
    let mut view_slot: Option<Entity<V>> = None;
    let handle: AnyWindowHandle = match crate::windows::open_window(cx, &spec, |window, cx| {
        let view = cx.new(|cx| build(window, cx));
        view_slot = Some(view.clone());
        view
    }) {
        Ok(handle) => handle.into(),
        Err(e) => {
            log::error!("[block] 打开{key}失败: {e}");
            window_mode::unregister(cx, key);
            return None;
        }
    };
    WindowManager::register(cx, kind, Some(key.registry_key()), handle);
    WindowManager::on_closed(cx, handle, move |cx| {
        window_mode::unregister(cx, key);
        if let BlockKey::List(id) = key {
            runtime(cx).list_views.remove(&id);
        }
    });
    let hwnd = cx
        .update_window(handle, |_, window, _| {
            platform::set_round_corners(window, true);
            platform::hwnd(window)
        })
        .ok()
        .flatten();
    let Some(view) = view_slot else {
        log::error!("[block] {key}没有根视图，窗口模式不会生效");
        return None;
    };
    window_mode::register(cx, key, handle, hwnd, view.entity_id());
    if geometry.is_fixed {
        let target = window_mode::TargetMode::for_state(true, geometry.embed_desktop);
        window_mode::apply_mode(cx, key, target).detach();
    }
    if geometry.saved.is_none() {
        // 没有布局：建初始记录（默认清单窗口顺带登记屏幕组合）
        window_mode::save_window_state(cx, key);
    }
    log::info!("[block] 已打开{key}");
    Some((handle, view))
}

/// 默认清单窗口没有布局时：主显示器居中 380×600（旧 `appStore.initSettings`）
fn centered_on_primary(cx: &App) -> PhysRect {
    let primary = platform::primary_monitor(cx);
    let bounds = crate::windows::centered_bounds(
        primary.as_ref(),
        None,
        logical_size(list_window::DEFAULT_WIDTH, list_window::DEFAULT_HEIGHT),
    );
    PhysRect::from_logical(bounds, primary.map_or(1., |m| m.scale))
}

/// 新清单窗口的位置：`monitor` 的工作区（取不到时 1920×1080@1）里找不与显示中的窗口（吸附组：清单 / 日历窗口与收纳箱）
/// 重叠的空位（[`new_list_rect`]）
fn empty_spot(cx: &App, monitor: Option<MonitorInfo>) -> PhysRect {
    let (work, scale) = match monitor {
        Some(m) => (
            if m.work.width > 0 && m.work.height > 0 {
                m.work
            } else {
                m.rect
            },
            m.scale,
        ),
        None => (PhysRect::new(0, 0, 1920, 1080), 1.),
    };
    new_list_rect(work, scale, &crate::services::snap::visible_rects(cx))
}

/// 新清单窗口在工作区 `work`（物理像素）里的位置：380×600 逻辑像素 × `scale`，从右上角起找不与 `existing` 重叠的空位，
/// 找不到就斜向错开（与新建收纳箱同一个算法 `fence::default_fence_rect`，边距 24、步长 32 逻辑像素）
pub fn new_list_rect(work: PhysRect, scale: f64, existing: &[PxRect]) -> PhysRect {
    let scale = if scale.is_finite() && scale > 0. {
        scale
    } else {
        1.
    };
    let px = |dip: f64| (dip * scale).round() as i32;
    let rect = fence::default_fence_rect(
        PxRect {
            x: work.x,
            y: work.y,
            width: work.width,
            height: work.height,
        },
        px(f64::from(list_window::DEFAULT_WIDTH)),
        px(f64::from(list_window::DEFAULT_HEIGHT)),
        existing,
        px(fence::SCREEN_MARGIN_DIP),
        px(fence::CASCADE_STEP_DIP),
    );
    PhysRect::new(rect.x, rect.y, rect.width, rect.height)
}

/// 日历窗口没有布局时：贴默认清单窗口右侧、与它等高（取不到默认清单窗口的位置时按它在主显示器居中的位置算）
fn calendar_fallback(cx: &mut App) -> PhysRect {
    let list = window_mode::client_rect(cx, BlockKey::DEFAULT_LIST)
        .unwrap_or_else(|| centered_on_primary(cx));
    let monitors = platform::monitors(cx);
    let (cx_, cy_) = (list.x + list.width / 2, list.y + list.height / 2);
    let monitor = monitors
        .iter()
        .find(|m| m.rect.contains(cx_, cy_))
        .cloned()
        .or_else(|| platform::primary_monitor(cx));
    let (work, scale) = monitor.map_or((PhysRect::new(0, 0, 1920, 1080), 1.), |m| {
        (
            if m.work.width > 0 && m.work.height > 0 {
                m.work
            } else {
                m.rect
            },
            m.scale,
        )
    });
    calendar_window::logic::first_rect(list, work, scale)
}

// ----------------------------------------------------------------------
// 显示 / 隐藏 / 关闭
// ----------------------------------------------------------------------

/// 窗口 `key` 显示中：属性里的隐藏清掉（已是显示时不写库）
fn mark_shown(cx: &mut App, key: BlockKey) {
    set_hidden(cx, key, false);
}

/// 写窗口 `key` 的隐藏属性（值没变时不写；失败只记日志：`AppState` 已回滚）
fn set_hidden(cx: &mut App, key: BlockKey, hidden: bool) {
    let state = AppState::global(cx);
    if state.read(cx).window_prefs(key).hidden == hidden {
        return;
    }
    state
        .update(cx, |state, cx| state.set_window_hidden(key, hidden, cx))
        .detach();
}

/// 关闭请求（Alt+F4 / 系统关闭、macOS ⌘W）：返回 GPUI 是否真的关窗。有模态子窗口时不关（把子窗口带到前台）；
/// 否则保存几何、记住隐藏（日历 = 关掉「显示日历」），Windows / macOS 原生隐藏窗口、不注销，Linux 真的关掉
pub fn request_close(window: &mut Window, cx: &mut App, key: BlockKey) -> bool {
    if WindowManager::focus_modal_child(cx, window.window_handle().window_id()) {
        return false;
    }
    window_mode::on_close_requested(window, cx, key);
    set_hidden(cx, key, true);
    if key == BlockKey::Calendar {
        crate::services::tray::set_calendar_checked(cx, false);
    }
    log::info!("[block] 关闭{key}（隐藏）");
    if CAN_HIDE {
        platform::hide_window(window, cx);
        false
    } else {
        true
    }
}

/// 托盘左键 / 单实例 / Dock 唤起之前（`window_mode::bring_all_to_front`）：所有清单窗口都要显示——已开过窗的清掉隐藏（随后由
/// 唤起显示出来），还没开过窗的（启动时隐藏着）现在开窗。返回这次新开的窗口（建窗时已显示并激活，不用再唤起）
pub fn prepare_wake_all(cx: &mut App) -> Vec<BlockKey> {
    let mut ids: Vec<i64> = AppState::global(cx)
        .read(cx)
        .lists
        .iter()
        .map(|l| l.id)
        .collect();
    if !ids.contains(&DEFAULT_LIST_ID) {
        ids.insert(0, DEFAULT_LIST_ID);
    }
    let mut opened = Vec::new();
    for id in ids {
        let key = BlockKey::List(id);
        if window_mode::is_registered(cx, key) {
            mark_shown(cx, key);
        } else if open_list(cx, id, Placement::Restore).is_some() {
            mark_shown(cx, key);
            opened.push(key);
        }
    }
    opened
}

/// 唤起所有窗口时要不要唤起 `key`：日历只在「显示日历」开着时（隐藏着的日历窗口不被托盘左键带出来，PRD R9）
pub fn should_wake(cx: &App, key: BlockKey) -> bool {
    key != BlockKey::Calendar || AppState::global(cx).read(cx).calendar_visible()
}

/// 默认清单窗口没开（Linux 上被关掉了 / 建窗失败过）时打开它；返回是否刚打开
pub fn ensure_default_list(cx: &mut App) -> bool {
    if window_mode::is_registered(cx, BlockKey::DEFAULT_LIST) {
        return false;
    }
    let opened = open_list(cx, DEFAULT_LIST_ID, Placement::Restore).is_some();
    mark_shown(cx, BlockKey::DEFAULT_LIST);
    opened
}

/// 显示并唤起清单 `list_id` 的窗口（没开过就开窗；调试 `MINI_TODO_DEBUG_OPEN=list:<id>` / `main`）
pub fn show_list(cx: &mut App, list_id: i64) {
    let key = BlockKey::List(list_id);
    if window_mode::is_registered(cx, key) {
        mark_shown(cx, key);
        window_mode::bring_to_front(cx, key);
    } else if open_list(cx, list_id, Placement::Restore).is_some() {
        mark_shown(cx, key);
    }
}

/// 「显示日历」（设置 → 常规、托盘勾选、调试 `calendar`）：写日历窗口的隐藏属性并显示 / 隐藏窗口，回写托盘勾选
pub fn set_calendar_visible(cx: &mut App, visible: bool) {
    set_hidden(cx, BlockKey::Calendar, !visible);
    crate::services::tray::set_calendar_checked(cx, visible);
    if visible {
        if window_mode::is_registered(cx, BlockKey::Calendar) {
            window_mode::bring_to_front(cx, BlockKey::Calendar);
        } else {
            open_calendar(cx);
        }
        return;
    }
    let Some(handle) = WindowManager::block_window(cx, BlockKey::Calendar) else {
        return;
    };
    // 延后：调用方可能正在别的窗口（设置窗口）的事件处理里
    cx.defer(move |cx| {
        let _ = cx.update_window(handle, |_, window, cx| {
            window_mode::on_close_requested(window, cx, BlockKey::Calendar);
            if CAN_HIDE {
                platform::hide_window(window, cx);
            } else {
                window.remove_window();
            }
        });
    });
}

/// 关掉清单 `list_id` 的窗口（清单已被删除）：先关它的模态子窗口，再注销、关窗
fn close_list_window(cx: &mut App, list_id: i64) {
    let key = BlockKey::List(list_id);
    let Some(handle) = WindowManager::block_window(cx, key) else {
        window_mode::unregister(cx, key);
        return;
    };
    if let Some(child) = WindowManager::modal_child(cx, handle.window_id()) {
        WindowManager::close(cx, child);
    }
    window_mode::unregister(cx, key);
    runtime(cx).list_views.remove(&list_id);
    WindowManager::close(cx, handle);
    log::info!("[block] 清单已删除，关闭{key}");
}

// ----------------------------------------------------------------------
// 清单列表变化：开 / 关清单窗口
// ----------------------------------------------------------------------

/// 对账的结果：要开窗 / 关窗的清单
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReconcilePlan {
    pub open: Vec<i64>,
    pub close: Vec<i64>,
}

/// 清单列表（`lists`）与已开的清单窗口（`open_windows`，含原生隐藏着的）对账：
/// - 清单已不存在的窗口关掉（默认清单永远不关）；
/// - 没开窗的清单：本机没有窗口记录（`hidden(id) == None`，同步 / cloud 新建的）或记录是显示中 → 开窗；记录是隐藏 → 不开。
///
/// 默认清单窗口不在这里开（启动时总是开，之后只有 Linux 上会被真的关掉）
pub fn plan_reconcile(
    lists: &[i64],
    open_windows: &[i64],
    hidden: impl Fn(i64) -> Option<bool>,
) -> ReconcilePlan {
    let close = open_windows
        .iter()
        .copied()
        .filter(|id| *id != DEFAULT_LIST_ID && !lists.contains(id))
        .collect();
    let open = lists
        .iter()
        .copied()
        .filter(|id| *id != DEFAULT_LIST_ID && !open_windows.contains(id))
        .filter(|id| hidden(*id) != Some(true))
        .collect();
    ReconcilePlan { open, close }
}

/// 按清单列表开 / 关清单窗口（启动时、`lists_revision` 变化后）。本机刚新建的清单（有窗口记录）在托盘新建进行中时
/// 放到新建找好的位置；同步来的（没有窗口记录）在主显示器上找空位
pub fn reconcile(cx: &mut App) {
    let open_windows: Vec<i64> = window_mode::registered(cx)
        .into_iter()
        .filter_map(BlockKey::list_id)
        .collect();
    let (plan, local) = {
        let state = AppState::global(cx).read(cx);
        let lists: Vec<i64> = state.lists.iter().map(|l| l.id).collect();
        let plan = plan_reconcile(&lists, &open_windows, |id| {
            let key = BlockKey::List(id);
            state
                .has_window_record(key)
                .then(|| state.window_prefs(key).hidden)
        });
        let local: Vec<i64> = plan
            .open
            .iter()
            .copied()
            .filter(|id| state.has_window_record(BlockKey::List(*id)))
            .collect();
        (plan, local)
    };
    for id in plan.close {
        close_list_window(cx, id);
    }
    for id in plan.open {
        let placement = match runtime(cx).creating.first() {
            Some(rect) if local.contains(&id) => Placement::At(*rect),
            _ => Placement::Restore,
        };
        open_list(cx, id, placement);
    }
}

// ----------------------------------------------------------------------
// 新建清单（托盘「新增待办清单」）
// ----------------------------------------------------------------------

/// 托盘「新增待办清单」（调试 `list:new`）：自动取名"清单 / 清单 2…"新建清单（`AppState::create_list`，同时建好显示中的
/// 窗口记录），在光标所在显示器上找空位开 380×600 的窗口，建好后直接进入改名（PRD R6 / R9）。失败在默认清单窗口里提示。
///
/// 清单建好时 `lists_revision` 变化，对账（[`reconcile`]）先于这里的回调开窗：位置事先放进 `creating`，两条路径开在
/// 同一个位置，谁先开都一样（`open_list` 不重复开窗）
pub fn create_list(cx: &mut App) {
    let monitor = platform::monitor_at_cursor(cx).or_else(|| platform::primary_monitor(cx));
    let rect = empty_spot(cx, monitor);
    runtime(cx).creating.push(rect);
    let task = AppState::global(cx).update(cx, |state, cx| state.create_list(None, cx));
    cx.spawn(async move |cx| {
        let result = task.await;
        cx.update(|cx| {
            let creating = &mut runtime(cx).creating;
            if let Some(index) = creating.iter().position(|r| *r == rect) {
                creating.remove(index);
            }
            match result {
                Ok(list) => {
                    log::info!("[block] 已新建清单 #{}「{}」", list.id, list.name);
                    if open_list(cx, list.id, Placement::At(rect)).is_some() {
                        begin_rename(cx, list.id);
                    }
                }
                Err(e) => {
                    log::error!("[block] 新建清单失败: {e}");
                    if let Some(handle) = WindowManager::default_list(cx) {
                        crate::ui::message::show_in(
                            handle,
                            cx,
                            crate::ui::message::MessageKind::Error,
                            crate::ui::message::error_text("新建清单失败", &e),
                        );
                    }
                }
            }
        });
    })
    .detach();
}

/// 让清单 `list_id` 的窗口进入改名（新建清单后）：先唤起它（新开的窗口已在前面），再在它的窗口里开始编辑
fn begin_rename(cx: &mut App, list_id: i64) {
    let key = BlockKey::List(list_id);
    let (Some(handle), Some(view)) = (
        WindowManager::block_window(cx, key),
        runtime(cx)
            .list_views
            .get(&list_id)
            .and_then(WeakEntity::upgrade),
    ) else {
        return;
    };
    // 延后：新窗口刚建好，等它画完第一帧（输入框挂上）再聚焦
    cx.defer(move |cx| {
        let _ = cx.update_window(handle, |_, window, cx| {
            platform::bring_to_front(window, cx);
            view.update(cx, |view, cx| view.begin_rename(window, cx));
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    const WORK: PhysRect = PhysRect {
        x: 0,
        y: 0,
        width: 1920,
        height: 1040,
    };

    #[test]
    fn new_list_windows_find_a_free_spot_from_the_top_right() {
        // 空桌面：右上角留 24 边距
        assert_eq!(
            new_list_rect(WORK, 1., &[]),
            PhysRect::new(1920 - 24 - 380, 24, 380, 600)
        );
        // 右上角被占了：往左挪到不重叠（留半个边距的空隙）
        let taken = PxRect {
            x: 1516,
            y: 24,
            width: 380,
            height: 600,
        };
        let rect = new_list_rect(WORK, 1., &[taken]);
        assert_eq!((rect.y, rect.width, rect.height), (24, 380, 600));
        assert!(rect.x + rect.width + 12 <= taken.x, "{rect:?}");
        // 150%：尺寸与边距都按物理像素
        let work = PhysRect::new(0, 0, 2880, 1560);
        assert_eq!(
            new_list_rect(work, 1.5, &[]),
            PhysRect::new(2880 - 36 - 570, 36, 570, 900)
        );
    }

    #[test]
    fn reconcile_opens_new_lists_and_closes_deleted_ones() {
        let records = |id: i64| match id {
            // 本机有记录、显示中
            10 => Some(false),
            // 隐藏着（关闭 = 隐藏后重启）
            11 => Some(true),
            // 12：同步 / cloud 新建，本机没有窗口记录
            _ => None,
        };
        let plan = plan_reconcile(&[1, 10, 11, 12], &[1], records);
        assert_eq!(
            plan,
            ReconcilePlan {
                open: vec![10, 12],
                close: vec![]
            }
        );
        // 已开着的不重复开；清单被删除（同步来的墓碑 / 本机删除）的窗口关掉；默认清单永远不关也不在这里开
        let plan = plan_reconcile(&[1, 12], &[10, 12], records);
        assert_eq!(
            plan,
            ReconcilePlan {
                open: vec![],
                close: vec![10]
            }
        );
        let plan = plan_reconcile(&[], &[1], records);
        assert_eq!(plan, ReconcilePlan::default());
    }
}
