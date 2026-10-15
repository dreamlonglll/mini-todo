//! 桌面收纳箱生命周期：建窗与嵌入桌面、启动恢复、托盘三个入口（新增 / 显示隐藏 / 重置位置）、
//! 目录监听 → 前台刷新、200ms 嵌入守护（Explorer 重启后重挂）、约 2 秒一次的屏幕组合检测。
//!
//! 契约：`.trellis/spec/backend/desktop-fences.md` §3.1（建窗参数、可做的窗口操作、几何落库）、§3.4（文件层与线程）、
//! §3.5（屏幕组合布局）；旧实现 `pc/src-tauri/src/commands/fence.rs` 的 `open_fence_window` / `restore_fences` /
//! `create_fence_at_cursor` / `toggle_all_fences` / `reset_all_fence_positions` / `tick_fence_screen_layouts`。
//!
//! GPUI 下的做法（`research/backend-port.md` §5.3、§7.1）：
//! - 建窗 `is_resizable: false`、`is_minimizable: false`、Windows 上 `show: false` → 按物理像素 `SetWindowPos` 定位 →
//!   `WS_EX_TOOLWINDOW` + 嵌入桌面层（`platform::embed`，与主窗口嵌入共用）→ `SW_SHOWNOACTIVATE` 显示；拦截关闭（Alt+F4 无效）。
//! - 显示 / 隐藏用 `ShowWindow`，不改 ex style（GPUI 不会像 tao 那样重写样式），之后照样重挂一次（幂等）。
//! - 所有会给窗口同步发消息的 Win32 调用都在 `cx.spawn` 的 async 块里、`update` 闭包之外执行。
//! - 目录监听回调在 `fence-watch` 线程，经 channel 回前台交给对应收纳箱窗口。

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use gpui_kit::{AnyWindowHandle, App, AppContext as _, Entity, Global, Task, WeakEntity};
use mini_todo_core::db::Fence;
use mini_todo_core::fence::items::{self, FenceRefresh};
use mini_todo_core::fence::{self, MonitorArea, PxRect};
use mini_todo_core::services::fence_fs::FenceWatcher;

use crate::db::{Db, spawn_db};
use crate::platform::{self, Hwnd, PhysRect};
use crate::windows::fence::FenceWindow;
use crate::windows::{self, WindowKind, WindowManager, WindowSpec};

/// 200ms 守护轮询间隔；每 10 个 tick（约 2 秒）检测一次屏幕组合
const GUARD_TICK: Duration = Duration::from_millis(200);
const SCREEN_CHECK_TICKS: u32 = 10;

/// 一个收纳箱窗口
struct FenceWindowRecord {
    handle: AnyWindowHandle,
    view: WeakEntity<FenceWindow>,
    /// 只在 Windows 上用到（嵌入守护 / 显示隐藏）
    #[cfg_attr(not(windows), allow(dead_code))]
    hwnd: Option<Hwnd>,
}

/// 收纳箱运行时状态（Global）
#[derive(Default)]
pub struct FenceRuntime {
    /// 托盘「显示/隐藏所有收纳箱」：运行时状态，不持久化（重启后总是显示）
    hidden: bool,
    windows: HashMap<i64, FenceWindowRecord>,
    watcher: Option<FenceWatcher>,
    guard_started: bool,
}

impl Global for FenceRuntime {}

fn runtime(cx: &mut App) -> &mut FenceRuntime {
    if !cx.has_global::<FenceRuntime>() {
        cx.set_global(FenceRuntime::default());
    }
    cx.global_mut::<FenceRuntime>()
}

/// 当前是否处于"全部隐藏"
pub fn is_hidden(cx: &App) -> bool {
    cx.try_global::<FenceRuntime>().is_some_and(|r| r.hidden)
}

/// 某个收纳箱的窗口视图
pub fn view_of(cx: &App, fence_id: i64) -> Option<Entity<FenceWindow>> {
    cx.try_global::<FenceRuntime>()?
        .windows
        .get(&fence_id)?
        .view
        .upgrade()
}

/// 全部显示器的工作区与缩放（物理像素，主显示器排第一；取不到工作区时用整块显示器）
pub fn monitor_areas(cx: &App) -> Vec<MonitorArea> {
    let mut monitors = platform::monitors(cx);
    monitors.sort_by_key(|m| !m.primary);
    monitors
        .into_iter()
        .map(|m| MonitorArea {
            work: to_px(if m.work.width > 0 && m.work.height > 0 {
                m.work
            } else {
                m.rect
            }),
            scale: if m.scale > 0. { m.scale } else { 1. },
        })
        .collect()
}

pub fn to_px(r: PhysRect) -> PxRect {
    PxRect {
        x: r.x,
        y: r.y,
        width: r.width,
        height: r.height,
    }
}

pub fn to_phys(r: PxRect) -> PhysRect {
    PhysRect::new(r.x, r.y, r.width, r.height)
}

/// 把收纳箱窗口摆到 `rect`（物理像素）。Windows：一次 `SetWindowPos`（延后执行）；其它平台 GPUI 不能移动窗口，
/// 只改尺寸（截图 / 冒烟测试用）
pub fn apply_window_rect(window: &mut gpui_kit::Window, cx: &App, rect: PxRect, scale: f64) {
    #[cfg(windows)]
    {
        let _ = scale;
        platform::set_window_rect_physical(window, cx, to_phys(rect));
    }
    #[cfg(not(windows))]
    {
        let _ = cx;
        let s = if scale > 0. { scale } else { 1. } as f32;
        window.resize(gpui_kit::size(
            gpui_kit::px(rect.width as f32 / s),
            gpui_kit::px(rect.height as f32 / s),
        ));
    }
}

/// 建一个收纳箱窗口（已存在则直接返回它）。`rect` 是窗口应占的矩形（`fence_view` 的结果，物理像素），
/// `scale` 是目标显示器缩放
pub fn open_fence_window(
    cx: &mut App,
    fence: Fence,
    rect: PxRect,
    scale: f64,
) -> Option<AnyWindowHandle> {
    let id = fence.id;
    if let Some(existing) = runtime(cx).windows.get(&id) {
        return Some(existing.handle);
    }
    let s = if scale > 0. { scale } else { 1. };
    let bounds = gpui_kit::Bounds::new(
        gpui_kit::point(
            gpui_kit::px((rect.x as f64 / s) as f32),
            gpui_kit::px((rect.y as f64 / s) as f32),
        ),
        gpui_kit::size(
            gpui_kit::px((rect.width as f64 / s) as f32),
            gpui_kit::px((rect.height as f64 / s) as f32),
        ),
    );
    let mut spec = WindowSpec::new(fence.title.clone(), bounds);
    spec.transparent = true;
    spec.resizable = false;
    spec.minimizable = false;
    spec.focus = false;
    // Windows：先藏着建好，按物理像素定位、嵌入桌面层之后再显示（不闪任务栏）
    spec.show = !cfg!(windows);
    // 折叠态窗口只有标题栏高：最小尺寸取 160×32，展开态的最小高度由界面逻辑保证
    spec.min_size = Some(gpui_kit::size(
        gpui_kit::px(fence::MIN_WIDTH_DIP as f32),
        gpui_kit::px(fence::FENCE_TITLE_BAR_DIP as f32),
    ));
    spec.display_id = platform::monitors(cx)
        .into_iter()
        .find(|m| m.rect.contains(rect.x + rect.width / 2, rect.y + 4))
        .map(|m| m.display_id);
    let expanded = !fence.collapsed;
    let mut view_slot: Option<Entity<FenceWindow>> = None;
    let opened = windows::open_window(cx, &spec, |window, cx| {
        // 没有关闭按钮；Alt+F4 / WM_CLOSE 一律拦截（删除收纳箱走 remove_window）
        window.on_window_should_close(cx, |_, _| false);
        let view = cx.new(|cx| FenceWindow::new(fence, expanded, window, cx));
        view_slot = Some(view.clone());
        view
    });
    let handle: AnyWindowHandle = match opened {
        Ok(handle) => handle.into(),
        Err(e) => {
            log::error!("[fence] 创建收纳箱 #{id} 的窗口失败: {e}");
            return None;
        }
    };
    let view = view_slot?.downgrade();
    WindowManager::register(
        cx,
        WindowKind::Fence,
        Some(crate::windows::fence::fence_key(id)),
        handle,
    );
    let hwnd = cx
        .update_window(handle, |_, window, _| platform::hwnd(window))
        .ok()
        .flatten();
    runtime(cx)
        .windows
        .insert(id, FenceWindowRecord { handle, view, hwnd });
    WindowManager::on_closed(cx, handle, move |cx| {
        runtime(cx).windows.remove(&id);
        if runtime(cx).windows.is_empty() {
            fence::forget_screen_config();
        }
    });
    embed_after_open(cx, hwnd, rect);
    ensure_guard(cx);
    Some(handle)
}

/// Windows：建窗后（App 未借用时）按物理像素定位、改样式、嵌入桌面层、显示
fn embed_after_open(cx: &mut App, hwnd: Option<Hwnd>, rect: PxRect) {
    #[cfg(windows)]
    {
        let Some(h) = hwnd else {
            return;
        };
        let hidden = is_hidden(cx);
        cx.spawn(async move |_| {
            // 先位置后尺寸的效果：一次 SetWindowPos 同时给出（混合 DPI 下逻辑坐标建窗会差几个像素）
            h.set_rect(to_phys(rect));
            h.set_tool_window(true);
            h.set_minimizable(false);
            h.set_round_corners(true);
            if !platform::embed::attach_to_desktop(h) {
                log::debug!("[fence] 嵌入桌面层未完成（{:#x}），200ms 守护会重试", h.0);
            }
            platform::embed::set_always_on_bottom(h, true);
            if !hidden {
                h.show();
            }
        })
        .detach();
    }
    #[cfg(not(windows))]
    {
        let _ = (cx, hwnd, rect);
    }
}

/// 开始监听收纳根目录（已在监听则什么都不做）；回调经 channel 回前台交给对应窗口
pub fn ensure_watcher(cx: &mut App) {
    if runtime(cx).watcher.is_some() {
        return;
    }
    let (tx, rx) = async_channel::unbounded::<FenceRefresh>();
    match items::watch(Db::get(cx), move |refresh| {
        let _ = tx.send_blocking(refresh);
    }) {
        Ok(watcher) => runtime(cx).watcher = Some(watcher),
        Err(e) => {
            log::warn!("[fence] 目录监听启动失败: {e}");
            return;
        }
    }
    cx.spawn(async move |cx| {
        while let Ok(refresh) = rx.recv().await {
            cx.update(|cx| deliver_refresh(cx, refresh));
        }
    })
    .detach();
}

/// 把对账结果交给对应收纳箱窗口（旧 `fence-items-changed`）
pub fn deliver_refresh(cx: &mut App, refresh: FenceRefresh) {
    if let Some(view) = view_of(cx, refresh.fence_id) {
        view.update(cx, |view, cx| view.set_items(refresh.items, cx));
    }
}

/// 200ms 守护：Explorer 重启后重挂 owner / 样式被改掉时自愈（仅 Windows）；每 ~2 秒检测屏幕组合变化
fn ensure_guard(cx: &mut App) {
    if runtime(cx).guard_started {
        return;
    }
    runtime(cx).guard_started = true;
    cx.spawn(async move |cx| {
        let mut tick: u32 = 0;
        loop {
            cx.background_executor().timer(GUARD_TICK).await;
            tick = tick.wrapping_add(1);
            #[cfg(windows)]
            {
                let hwnds: Vec<Hwnd> = cx.update(|cx| {
                    runtime(cx)
                        .windows
                        .values()
                        .filter_map(|r| r.hwnd)
                        .collect()
                });
                // 只读调用判断；需要时重挂（这里不在任何 update 闭包里，可以安全地改样式 / owner）
                let shell_alive = platform::embed::desktop_host().is_some();
                for h in hwnds {
                    if !h.is_alive() {
                        continue;
                    }
                    let lacks_tool =
                        mini_todo_core::window_logic::lacks_tool_window_style(h.ex_style());
                    if platform::embed::needs_reattach(h) || (shell_alive && lacks_tool) {
                        log::info!("[fence] 重新嵌入桌面层（{:#x}）", h.0);
                        h.set_tool_window(true);
                        platform::embed::attach_to_desktop(h);
                        platform::embed::set_always_on_bottom(h, true);
                    }
                }
            }
            if tick.is_multiple_of(SCREEN_CHECK_TICKS) {
                cx.update(check_screen_layout);
            }
        }
    })
    .detach();
}

/// 屏幕组合变化检测（旧 `tick_fence_screen_layouts`）
fn check_screen_layout(cx: &mut App) {
    if runtime(cx).windows.is_empty() {
        fence::forget_screen_config();
        return;
    }
    let current = platform::screen_config_id(cx);
    let Some(previous) = fence::observe_screen_config(&current) else {
        return;
    };
    log::info!("[fence] 屏幕组合变化：{previous} → {current}");
    let monitors = platform::monitors(cx);
    let records: Vec<(i64, AnyWindowHandle)> = runtime(cx)
        .windows
        .iter()
        .map(|(id, r)| (*id, r.handle))
        .collect();
    let mut snapshot: Vec<(i64, Option<PxRect>, f64)> = Vec::new();
    for (id, handle) in records {
        let info = cx
            .update_window(handle, |_, window, _| {
                let rect = platform::window_rect_physical(window).map(to_px);
                let scale = f64::from(window.scale_factor());
                (rect, scale)
            })
            .ok();
        let (rect, scale) = info.unwrap_or((None, 1.));
        let scale = rect
            .and_then(|r| {
                monitors
                    .iter()
                    .find(|m| m.rect.contains(r.x + r.width / 2, r.y + 4))
                    .map(|m| m.scale)
            })
            .unwrap_or(scale);
        snapshot.push((id, rect, scale));
    }
    let areas = monitor_areas(cx);
    let task = spawn_db(cx, move |db| {
        Ok(fence::screen_switch(db, &previous, &current, &snapshot))
    });
    cx.spawn(async move |cx| {
        if let Ok(targets) = task.await {
            cx.update(|cx| apply_targets(cx, targets, &areas));
        }
    })
    .detach();
}

/// 按存储几何摆放一批收纳箱窗口（重置位置 / 屏幕组合切换）：持久化的折叠态决定形态，夹回可见区域
fn apply_targets(cx: &mut App, targets: Vec<(Fence, PxRect)>, areas: &[MonitorArea]) {
    for (fence, stored) in targets {
        let expanded = !fence.collapsed;
        let (rect, scale) = fence::fence_view(stored, expanded, areas);
        let Some(view) = view_of(cx, fence.id) else {
            continue;
        };
        let handle = runtime(cx).windows.get(&fence.id).map(|r| r.handle);
        if let Some(handle) = handle {
            let _ = cx.update_window(handle, |_, window, cx| {
                apply_window_rect(window, cx, rect, scale);
            });
        }
        view.update(cx, |view, cx| view.reset_view_state(expanded, cx));
    }
}

/// 显示 / 隐藏全部收纳箱窗口
fn set_all_visible(cx: &mut App, visible: bool) {
    runtime(cx).hidden = !visible;
    #[cfg(windows)]
    {
        let hwnds: Vec<Hwnd> = runtime(cx)
            .windows
            .values()
            .filter_map(|r| r.hwnd)
            .collect();
        cx.spawn(async move |_| {
            for h in hwnds {
                if visible {
                    h.show();
                    // ShowWindow 不改 ex style，重挂一次也无害（幂等）
                    platform::embed::attach_to_desktop(h);
                    platform::embed::set_always_on_bottom(h, true);
                } else {
                    h.hide();
                }
            }
        })
        .detach();
    }
}

/// 启动时恢复全部收纳箱窗口（旧 `restore_fences`，仅 Windows）：`prepare_restore` → 逐个建窗（夹回可见区域，
/// 不落库）→ 有收纳箱则开始监听 → 启动对账 + 20 秒后清理图标缓存
pub fn restore_all(cx: &mut App) {
    if !cfg!(windows) {
        return;
    }
    let screen_id = platform::screen_config_id(cx);
    let areas = monitor_areas(cx);
    let task = spawn_db(cx, move |db| fence::prepare_restore(db, &screen_id));
    let db = Db::get(cx);
    cx.spawn(async move |cx| {
        let fences = match task.await {
            Ok(fences) => fences,
            Err(e) => {
                log::error!("[fence] 恢复收纳箱失败: {e}");
                Vec::new()
            }
        };
        cx.update(|cx| {
            let any = !fences.is_empty();
            for (fence, stored) in fences {
                let (rect, scale) = fence::fence_view(stored, !fence.collapsed, &areas);
                open_fence_window(cx, fence, rect, scale);
            }
            if any {
                ensure_watcher(cx);
                log::info!("[fence] 已恢复 {} 个收纳箱", runtime(cx).windows.len());
            }
        });
        items::spawn_startup_reconcile(db);
    })
    .detach();
}

/// 打开（或带到前台）某个收纳箱窗口（调试 `MINI_TODO_DEBUG_OPEN=fence:<id>` 用）：按当前屏幕组合的存储几何建窗
pub fn open_by_id(cx: &mut App, fence_id: i64) -> Task<Option<AnyWindowHandle>> {
    if let Some(existing) = cx
        .try_global::<FenceRuntime>()
        .and_then(|r| r.windows.get(&fence_id))
        .map(|r| r.handle)
    {
        return Task::ready(Some(existing));
    }
    let screen_id = platform::screen_config_id(cx);
    let areas = monitor_areas(cx);
    let task = spawn_db(cx, move |db| {
        let fence = fence::get(db, fence_id)?;
        let stored = fence::Placement::for_screen(db, &screen_id).rect(&fence);
        Ok((fence, stored))
    });
    cx.spawn(async move |cx| match task.await {
        Ok((fence, stored)) => cx.update(|cx| {
            let (rect, scale) = fence::fence_view(stored, !fence.collapsed, &areas);
            let handle = open_fence_window(cx, fence, rect, scale);
            ensure_watcher(cx);
            handle
        }),
        Err(e) => {
            log::warn!("[fence] 打开收纳箱 #{fence_id} 失败: {e}");
            None
        }
    })
}

/// 托盘「新增收纳箱」（旧 `create_fence_at_cursor`）：光标所在显示器的工作区（取不到退回主显示器，再取不到按
/// 1920×1080@1）右上角起找空位；隐藏中先全部显示；建窗失败回滚记录与文件夹；成功后确保监听
pub fn create_fence(cx: &mut App) {
    let monitor = platform::monitor_at_cursor(cx);
    let (work, scale) = match &monitor {
        Some(m) => (
            to_px(if m.work.width > 0 { m.work } else { m.rect }),
            if m.scale > 0. { m.scale } else { 1. },
        ),
        None => (fence::FALLBACK_WORK_AREA, 1.),
    };
    let screen_id = platform::screen_config_id(cx);
    let areas = monitor_areas(cx);
    if is_hidden(cx) {
        set_all_visible(cx, true);
    }
    let task = spawn_db(cx, move |db| fence::create(db, work, scale, &screen_id));
    let db = Db::get(cx);
    cx.spawn(async move |cx| {
        let created = match task.await {
            Ok(fence) => fence,
            Err(e) => {
                log::error!("[fence] 新增收纳箱失败: {e}");
                return;
            }
        };
        let rollback = created.clone();
        let opened = cx.update(|cx| {
            let stored = fence::stored_rect(&created);
            let (rect, scale) = fence::fence_view(stored, true, &areas);
            let handle = open_fence_window(cx, created, rect, scale);
            if handle.is_some() {
                ensure_watcher(cx);
            }
            handle.is_some()
        });
        if !opened {
            let db: Arc<mini_todo_core::Database> = db;
            cx.background_executor()
                .spawn(async move { fence::rollback_create(&db, &rollback) })
                .await;
            log::error!("[fence] 创建收纳箱窗口失败，已回滚");
        } else {
            log::info!("[fence] 已新增收纳箱 #{}", rollback.id);
        }
    })
    .detach();
}

/// 托盘「显示/隐藏所有收纳箱」：返回切换后是否可见
pub fn toggle_visibility(cx: &mut App) -> bool {
    let visible = is_hidden(cx);
    set_all_visible(cx, visible);
    log::info!(
        "[fence] {}所有收纳箱",
        if visible { "显示" } else { "隐藏" }
    );
    visible
}

/// 托盘「重置收纳箱位置」：主显示器工作区从右上角起排开并落库，摆窗；隐藏中则全部显示
pub fn reset_positions(cx: &mut App) {
    let Some(primary) = platform::primary_monitor(cx) else {
        log::warn!("[fence] 重置收纳箱位置失败: 找不到可用的显示器");
        return;
    };
    let work = to_px(if primary.work.width > 0 {
        primary.work
    } else {
        primary.rect
    });
    let scale = if primary.scale > 0. {
        primary.scale
    } else {
        1.
    };
    let screen_id = platform::screen_config_id(cx);
    let areas = monitor_areas(cx);
    let task = spawn_db(cx, move |db| {
        fence::reset_positions(db, work, scale, &screen_id)
    });
    cx.spawn(async move |cx| match task.await {
        Ok(targets) => cx.update(|cx| {
            apply_targets(cx, targets, &areas);
            if is_hidden(cx) {
                set_all_visible(cx, true);
            }
        }),
        Err(e) => log::warn!("[fence] 重置收纳箱位置失败: {e}"),
    })
    .detach();
}

/// 收纳箱已删除：注销并关闭窗口
pub fn close_deleted(cx: &mut App, fence_id: i64) {
    let handle = runtime(cx).windows.remove(&fence_id).map(|r| r.handle);
    if let Some(handle) = handle {
        WindowManager::close(cx, handle);
    }
    if runtime(cx).windows.is_empty() {
        fence::forget_screen_config();
    }
}

/// 其它收纳箱（右键「移动到…」的目标；按 sort_order、id）
pub fn other_fences(cx: &App, current: i64) -> Task<Vec<Fence>> {
    let db = Db::get(cx);
    cx.background_executor().spawn(async move {
        fence::list(&db)
            .map(|list| list.into_iter().filter(|f| f.id != current).collect())
            .unwrap_or_else(|e| {
                log::warn!("[fence] 读取收纳箱列表失败: {e}");
                Vec::new()
            })
    })
}
