//! 桌面收纳箱窗口（旧 `FenceView.vue` + `FenceItemGrid.vue`，label `fence-{id}`）。
//!
//! 规格：`research/ui-settings-fence.md` §3–§5，契约 `.trellis/spec/backend/desktop-fences.md`。窗口生命周期（建窗、嵌入桌面、
//! 恢复、托盘入口、监听、守护、屏幕组合）在 [`crate::services::fences`]。
//!
//! | 文件 | 内容 |
//! |---|---|
//! | `mod.rs` | 视图状态、标题栏（超过 3px 才拖动、双击改名、折叠 / 固定 / 更多）、折叠与临时展开状态机、几何落库、缩放手柄、外部拖入 |
//! | `panels.rs` | 外观 / 删除确认 / OneDrive 提示面板，「更多」下拉菜单 |
//! | `grid.rs` | 项目网格：图标懒加载、选中 / 打开、自绘右键菜单（含「隐藏」/「取消隐藏」「移动到…」）、隐藏项过滤与半透明、箱内拖动排序、Ctrl+滚轮调图标大小 |
//! | `logic.rs` | 纯函数（标题规范化、拖动阈值、几何闸门、缩放矩形、图标档位、隐藏项过滤与排序合并、结果提示文案） |
//!
//! 几何一律物理像素的**客户区**（= 可见区域，与旧版 Tauri 无边框窗口的外框相同；GPUI 窗口另有不可见边框）：
//! `mini_todo_core::fence::fence_view` + `platform::set_client_rect_physical` 摆窗，`platform::client_rect_physical` 读几何。
//! 只有用户拖动 / 缩放结束才落库（`commit_geometry`），折叠 / 展开、启动夹回、系统挪窗都不落库。

mod grid;
pub mod logic;
mod panels;

use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};

use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::{
    AnyElement, AnyWindowHandle, App, AppContext as _, ClickEvent, Context, CursorStyle,
    DragMoveEvent, Empty, Entity, ExternalPaths, FileDropEvent, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, KeyDownEvent, MouseButton, MouseDownEvent,
    MouseMoveEvent, ParentElement as _, Pixels, Point, Render, ScrollHandle, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, Task, Window, div,
    prelude::FluentBuilder as _, px, relative,
};
use mini_todo_core::db::{Fence, UpdateFenceRequest};
use mini_todo_core::fence::items::{self, FenceEntry};
use mini_todo_core::fence::{self as core_fence, PxRect};

use self::logic::{GeometryCommitGate, ResizeDir, now_ms, timing};
use crate::assets::EpIcon;
use crate::db::spawn_db;
use crate::platform;
use crate::services::fences as runtime;
use crate::theme::{self, fence_tokens};
use crate::ui::ep_popper::Anchor;
use crate::ui::external_drop::{DropScope, ExternalDrop};
use crate::ui::icon::icon;
use crate::ui::input::EpInput;
use crate::ui::message;

/// 窗口登记用的 key（与旧版 label 相同：`fence-{id}`）
pub fn fence_key(fence_id: i64) -> String {
    core_fence::fence_label(fence_id)
}

/// 窗口临时展开的原因（任一存在即展开，不改持久化的折叠态）
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum ExpandReason {
    Hover,
    Drag,
    Menu,
    Panel,
    ItemMenu,
}

/// 盖住内容区的面板
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Panel {
    #[default]
    None,
    Appearance,
    Delete,
    OneDrive,
}

/// 进行中的缩放
#[derive(Debug, Clone, Copy)]
struct ResizeSession {
    dir: ResizeDir,
    /// 按下时的光标（Windows：屏幕物理像素；其它平台：窗口逻辑坐标）
    start_cursor: (f64, f64),
    /// 按下时的窗口客户区（物理像素）
    start: PxRect,
    dpr: f64,
}

/// 缩放手柄的拖动标记
#[derive(Clone, Copy)]
struct ResizeDrag(ResizeDir);

struct EmptyDragView;

impl Render for EmptyDragView {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        Empty
    }
}

/// 收纳箱窗口视图
pub struct FenceWindow {
    fence_id: i64,
    fence: Option<Fence>,
    load_error: Option<SharedString>,
    /// 全部项目（含隐藏项；网格按「显示隐藏的图标」过滤，见 `grid::display_items`）
    items: Vec<FenceEntry>,
    items_seq: u64,
    reasons: BTreeSet<ExpandReason>,
    panel: Panel,
    editing_title: bool,
    title_input: Entity<InputState>,
    drag_hover: bool,
    drag_epoch: u64,
    /// 外部拖入（窗口任何位置松手都收；不依赖 GPUI `on_drop`，见 `ui::external_drop`）
    external_drop: ExternalDrop,
    hovered: bool,
    hover_epoch: u64,
    alpha_preview: Option<f64>,
    delete_count: Option<usize>,
    deleting: bool,
    title_menu_open: bool,
    more_anchor: Anchor,
    press_point: Option<Point<Pixels>>,
    gate: GeometryCommitGate,
    commit_epoch: u64,
    commit_pending: bool,
    resizing: Option<ResizeSession>,
    resize_pending: Option<PxRect>,
    resize_frame_scheduled: bool,
    icon_size_pending: bool,
    icon_save_epoch: u64,
    /// 窗口当前呈现的形态（`set_fence_view` 最近一次的目标）
    applied_view: Option<bool>,
    view_epoch: u64,
    focus: FocusHandle,

    // ---- 网格（grid.rs） ----
    selected: Option<String>,
    launching: HashSet<String>,
    item_menu: Option<grid::ItemMenu>,
    icons: HashMap<String, grid::IconState>,
    icon_queue: VecDeque<grid::IconRequest>,
    icons_in_flight: usize,
    /// 拖动排序中的本地顺序（只含显示项；期间不跟随推送）
    sorting: Option<Vec<FenceEntry>>,
    dragging_item: Option<String>,
    grid_scroll: ScrollHandle,
    last_wheel_at: Option<std::time::Instant>,

    _subscriptions: Vec<Subscription>,
}

impl Focusable for FenceWindow {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl FenceWindow {
    /// `expanded`：建窗时呈现的形态（= 持久化的展开态）
    pub fn new(fence: Fence, expanded: bool, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let max_chars = core_fence::MAX_TITLE_CHARS;
        let title_input = cx.new(|cx| {
            InputState::new(window, cx).validate(move |s, _| s.chars().count() <= max_chars)
        });
        let subscriptions = vec![
            cx.subscribe_in(
                &title_input,
                window,
                |this, _, event: &InputEvent, window, cx| match event {
                    InputEvent::PressEnter { .. } => this.finish_rename(true, window, cx),
                    InputEvent::Blur => this.finish_rename(true, window, cx),
                    _ => {}
                },
            ),
            cx.observe_window_bounds(window, |this, window, cx| {
                this.on_geometry_event(window, cx)
            }),
            cx.observe_window_activation(window, |this, window, cx| {
                if !window.is_window_active() {
                    // 窗口失焦（例如点到桌面）：提交改名、关掉菜单
                    this.finish_rename(true, window, cx);
                    this.close_item_menu(window, cx);
                    this.close_title_menu(window, cx);
                }
            }),
        ];
        let fence_id = fence.id;
        let mut this = Self {
            fence_id,
            fence: Some(fence),
            load_error: None,
            items: Vec::new(),
            items_seq: 0,
            reasons: BTreeSet::new(),
            panel: Panel::None,
            editing_title: false,
            title_input,
            drag_hover: false,
            drag_epoch: 0,
            external_drop: ExternalDrop::new(),
            hovered: false,
            hover_epoch: 0,
            alpha_preview: None,
            delete_count: None,
            deleting: false,
            title_menu_open: false,
            more_anchor: Anchor::new(),
            press_point: None,
            gate: GeometryCommitGate::new(timing::GEOMETRY_ARM_MS),
            commit_epoch: 0,
            commit_pending: false,
            resizing: None,
            resize_pending: None,
            resize_frame_scheduled: false,
            icon_size_pending: false,
            icon_save_epoch: 0,
            applied_view: Some(expanded),
            view_epoch: 0,
            focus: cx.focus_handle(),
            selected: None,
            launching: HashSet::new(),
            item_menu: None,
            icons: HashMap::new(),
            icon_queue: VecDeque::new(),
            icons_in_flight: 0,
            sorting: None,
            dragging_item: None,
            grid_scroll: ScrollHandle::new(),
            last_wheel_at: None,
            _subscriptions: subscriptions,
        };
        this.reload_fence(window, cx);
        #[cfg(debug_assertions)]
        this.apply_debug_state(window, cx);
        this
    }

    /// debug 构建：`MINI_TODO_DEBUG_FENCE_STATE=onedrive|drag-hover` 直接呈现 Linux 上触发不了的状态
    /// （OneDrive 提示要 Windows 的 OneDrive 桌面；外部拖入悬停要真实的 XDND 拖放源），截图对照用
    #[cfg(debug_assertions)]
    fn apply_debug_state(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match std::env::var("MINI_TODO_DEBUG_FENCE_STATE").as_deref() {
            Ok("onedrive") => self.set_panel(Panel::OneDrive, window, cx),
            Ok("drag-hover") => {
                self.drag_hover = true;
                self.set_reason(ExpandReason::Drag, true, window, cx);
            }
            _ => {}
        }
    }

    pub fn fence_id(&self) -> i64 {
        self.fence_id
    }

    // ------------------------------------------------------------------
    // 数据
    // ------------------------------------------------------------------

    /// 重新读取收纳箱记录并列举内容（窗口启动）
    fn reload_fence(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let id = self.fence_id;
        let task = spawn_db(cx, move |db| core_fence::get(db, id));
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, window, cx| match result {
                Ok(fence) => {
                    this.apply_fence(fence, cx);
                    this.refresh_items(cx);
                    this.sync_view(window, cx);
                }
                Err(e) => {
                    log::warn!("[fence] 读取收纳箱 #{id} 失败: {e}");
                    this.fence = None;
                    this.load_error = Some(if e.trim().is_empty() {
                        "读取收纳箱失败".into()
                    } else {
                        e.into()
                    });
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// 旧 `applyFence`：图标大小还在防抖未落库时保留本地值。「显示隐藏的图标」可能随之变化：
    /// 不再显示的选中项 / 右键菜单随之取消
    fn apply_fence(&mut self, mut next: Fence, cx: &mut Context<Self>) {
        if self.icon_size_pending
            && let Some(current) = &self.fence
        {
            next.icon_size = current.icon_size;
        }
        self.fence = Some(next);
        self.on_items_changed(cx);
        cx.notify();
    }

    /// 列举 + 对账（只采用最后一次发起的结果；失败只记日志）
    pub(crate) fn refresh_items(&mut self, cx: &mut Context<Self>) {
        self.items_seq += 1;
        let seq = self.items_seq;
        let id = self.fence_id;
        let task = spawn_db(cx, move |db| items::get_items(db, id));
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| match result {
                Ok(list) if seq == this.items_seq => this.replace_items(list, cx),
                Ok(_) => {}
                Err(e) => log::debug!("[fence] 列举收纳箱 #{id} 失败: {e}"),
            });
        })
        .detach();
    }

    /// 后端推送 / 子操作后的完整列表（旧 `setItems`）；拖动排序期间不跟随
    pub fn set_items(&mut self, list: Vec<FenceEntry>, cx: &mut Context<Self>) {
        self.items_seq += 1;
        self.replace_items(list, cx);
    }

    fn replace_items(&mut self, list: Vec<FenceEntry>, cx: &mut Context<Self>) {
        if self.sorting.is_some() {
            return;
        }
        self.items = list;
        self.on_items_changed(cx);
        cx.notify();
    }

    /// 程序摆窗（重置位置 / 屏幕组合切换）之后：记下窗口当前形态
    pub fn reset_view_state(&mut self, expanded: bool, cx: &mut Context<Self>) {
        self.applied_view = Some(expanded);
        self.gate.disarm();
        cx.notify();
    }

    fn collapsed(&self) -> bool {
        self.fence.as_ref().is_some_and(|f| f.collapsed)
    }

    fn locked(&self) -> bool {
        self.fence.as_ref().is_some_and(|f| f.locked)
    }

    /// 「显示隐藏的图标」
    fn show_hidden(&self) -> bool {
        self.fence.as_ref().is_some_and(|f| f.show_hidden)
    }

    /// 窗口是否显示完整高度
    fn view_expanded(&self) -> bool {
        self.fence
            .as_ref()
            .is_some_and(|f| !f.collapsed || !self.reasons.is_empty())
    }

    /// 临时展开态也不能缩放
    fn can_resize(&self) -> bool {
        self.fence.is_some() && !self.locked() && !self.collapsed() && !self.editing_title
    }

    fn bg_alpha(&self) -> f64 {
        self.alpha_preview
            .or(self.fence.as_ref().map(|f| f.bg_alpha))
            .unwrap_or(mini_todo_core::db::DEFAULT_FENCE_BG_ALPHA)
    }

    // ------------------------------------------------------------------
    // 折叠 / 临时展开
    // ------------------------------------------------------------------

    fn set_reason(
        &mut self,
        reason: ExpandReason,
        on: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let changed = if on {
            self.reasons.insert(reason)
        } else {
            self.reasons.remove(&reason)
        };
        if changed {
            self.sync_view(window, cx);
            cx.notify();
        }
    }

    /// 形态变化时把窗口摆成折叠 / 展开（旧 `set_fence_view`，不写库）：先落库待落的拖动，再按库里的几何算形态
    fn sync_view(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.fence.is_none() {
            return;
        }
        let expanded = self.view_expanded();
        if self.applied_view == Some(expanded) {
            return;
        }
        self.applied_view = Some(expanded);
        self.view_epoch += 1;
        let epoch = self.view_epoch;
        let flush = self.flush_geometry(window, cx);
        let id = self.fence_id;
        let screen_id = platform::screen_config_id(cx);
        let areas = runtime::monitor_areas(cx);
        cx.spawn_in(window, async move |this, cx| {
            flush.await;
            let Ok(db) = cx.update(|_, cx| crate::db::Db::get(cx)) else {
                return;
            };
            let computed = cx
                .background_executor()
                .spawn(async move {
                    let fence = core_fence::get(&db, id)?;
                    let stored = core_fence::Placement::for_screen(&db, &screen_id).rect(&fence);
                    Ok::<_, String>(core_fence::fence_view(stored, expanded, &areas))
                })
                .await;
            let _ = this.update_in(cx, |this, window, cx| match computed {
                Ok((rect, scale)) if this.view_epoch == epoch => {
                    runtime::apply_window_rect(window, cx, rect, scale);
                }
                Ok(_) => {}
                Err(e) => log::warn!("[fence] 切换收纳箱 #{id} 的形态失败: {e}"),
            });
        })
        .detach();
    }

    fn toggle_collapsed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let collapsed = !self.collapsed();
        self.patch_fence(
            UpdateFenceRequest {
                collapsed: Some(collapsed),
                ..Default::default()
            },
            "折叠收纳箱失败",
            window,
            cx,
        );
        // 刚点折叠的收纳箱不因鼠标还在上面又弹开
        self.hover_epoch += 1;
        self.set_reason(ExpandReason::Hover, false, window, cx);
    }

    fn toggle_locked(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let locked = !self.locked();
        self.patch_fence(
            UpdateFenceRequest {
                locked: Some(locked),
                ..Default::default()
            },
            "固定收纳箱失败",
            window,
            cx,
        );
    }

    /// 「更多」→「显示隐藏的图标」/「不显示隐藏的图标」（每个收纳箱独立记住）
    fn toggle_show_hidden(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let show_hidden = !self.show_hidden();
        self.patch_fence(
            UpdateFenceRequest {
                show_hidden: Some(show_hidden),
                ..Default::default()
            },
            "切换显示隐藏的图标失败",
            window,
            cx,
        );
    }

    /// 旧 `patchFence`：先落库待落的拖动 → 写库 → 采用返回值；失败 `notifyError`
    pub(crate) fn patch_fence(
        &mut self,
        patch: UpdateFenceRequest,
        fail: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // 本地先改，窗口形态 / 网格立即跟上（失败时用库里的值覆盖回去）
        if let Some(fence) = &mut self.fence {
            if let Some(v) = patch.collapsed {
                fence.collapsed = v;
            }
            if let Some(v) = patch.locked {
                fence.locked = v;
            }
            if let Some(v) = patch.show_hidden {
                fence.show_hidden = v;
            }
        }
        if patch.show_hidden.is_some() {
            self.on_items_changed(cx);
        }
        self.sync_view(window, cx);
        cx.notify();
        let flush = self.flush_geometry(window, cx);
        let id = self.fence_id;
        cx.spawn_in(window, async move |this, cx| {
            flush.await;
            let Ok(task) =
                cx.update(|_, cx| spawn_db(cx, move |db| core_fence::update(db, id, &patch)))
            else {
                return;
            };
            let result = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(fence) => this.apply_fence(fence, cx),
                    Err(e) => {
                        message::error_with(window, cx, fail, &e);
                        this.reload_fence(window, cx);
                    }
                }
                this.sync_view(window, cx);
            });
        })
        .detach();
    }

    fn on_hover_changed(&mut self, hovered: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.hovered == hovered {
            return;
        }
        self.hovered = hovered;
        self.hover_epoch += 1;
        let epoch = self.hover_epoch;
        cx.notify();
        if hovered {
            if !self.collapsed() {
                return;
            }
            cx.spawn_in(window, async move |this, cx| {
                cx.background_executor()
                    .timer(timing::HOVER_EXPAND_DELAY)
                    .await;
                let _ = this.update_in(cx, |this, window, cx| {
                    if this.hover_epoch == epoch && !this.gate.is_armed(now_ms()) {
                        this.set_reason(ExpandReason::Hover, true, window, cx);
                    }
                });
            })
            .detach();
        } else if self.reasons.contains(&ExpandReason::Hover) {
            self.schedule_hover_release(epoch, window, cx);
        }
    }

    /// 离开 400ms 后收回；拖窗口期间（闸门开着）再等 400ms
    fn schedule_hover_release(&mut self, epoch: u64, window: &mut Window, cx: &mut Context<Self>) {
        cx.spawn_in(window, async move |this, cx| {
            cx.background_executor()
                .timer(timing::HOVER_COLLAPSE_DELAY)
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                if this.hover_epoch != epoch {
                    return;
                }
                if this.gate.is_armed(now_ms()) {
                    this.schedule_hover_release(epoch, window, cx);
                } else {
                    this.set_reason(ExpandReason::Hover, false, window, cx);
                }
            });
        })
        .detach();
    }

    // ------------------------------------------------------------------
    // 移动（超过 3px 才拖）与几何落库
    // ------------------------------------------------------------------

    fn on_title_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        if self.editing_title {
            return;
        }
        self.press_point = Some(event.position);
    }

    fn on_title_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(start) = self.press_point else {
            return;
        };
        if event.pressed_button != Some(MouseButton::Left) {
            self.press_point = None;
            return;
        }
        let dx = f64::from(f32::from(event.position.x - start.x));
        let dy = f64::from(f32::from(event.position.y - start.y));
        if !logic::exceeds_drag_threshold(dx, dy, 3.) {
            return;
        }
        self.press_point = None;
        if self.locked() || self.editing_title {
            return;
        }
        self.gate.arm(now_ms());
        platform::begin_window_drag(window, cx);
    }

    /// 窗口移动 / 尺寸变化：闸门开着（用户拖动中）才安排落库（停下 300ms 后）
    fn on_geometry_event(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.gate.accept(now_ms()) {
            return;
        }
        self.commit_pending = true;
        self.commit_epoch += 1;
        let epoch = self.commit_epoch;
        cx.spawn_in(window, async move |this, cx| {
            cx.background_executor()
                .timer(timing::GEOMETRY_COMMIT_DEBOUNCE)
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                if this.commit_epoch == epoch && this.commit_pending {
                    this.commit_geometry(window, cx).detach();
                }
            });
        })
        .detach();
    }

    /// 读窗口当前客户区（= 可见区域）落库（折叠视图只落位置和宽度）。落库后不关闸门：按住不放停顿后继续拖仍然算数
    fn commit_geometry(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Task<()> {
        self.commit_pending = false;
        self.commit_epoch += 1;
        let Some(rect) = platform::client_rect_physical(window) else {
            return Task::ready(());
        };
        let expanded = self.view_expanded();
        let screen_id = platform::screen_config_id(cx);
        let id = self.fence_id;
        let task = spawn_db(cx, move |db| {
            core_fence::commit_geometry(db, id, &screen_id, runtime::to_px(rect), expanded)
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, window, cx| match result {
                Ok(fence) => this.apply_fence(fence, cx),
                Err(e) => message::error_with(window, cx, "保存收纳箱位置失败", &e),
            });
        })
    }

    /// 程序调整之前：有待落库的拖动立即落库，然后关闭闸门
    fn flush_geometry(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Task<()> {
        let task = if self.commit_pending {
            self.commit_geometry(window, cx)
        } else {
            Task::ready(())
        };
        self.gate.disarm();
        task
    }

    // ------------------------------------------------------------------
    // 改名
    // ------------------------------------------------------------------

    fn start_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(fence) = &self.fence else {
            return;
        };
        let title = fence.title.clone();
        self.editing_title = true;
        self.press_point = None;
        self.title_input.update(cx, |s, cx| {
            s.set_value(title, window, cx);
            s.focus(window, cx);
            s.select_all(window, cx);
        });
        cx.notify();
    }

    fn finish_rename(&mut self, commit: bool, window: &mut Window, cx: &mut Context<Self>) {
        if !self.editing_title {
            return;
        }
        self.editing_title = false;
        self.focus.focus(window, cx);
        cx.notify();
        if !commit {
            return;
        }
        let draft = self.title_input.read(cx).value().to_string();
        let Some(title) = logic::normalize_fence_title(&draft) else {
            return;
        };
        if self.fence.as_ref().is_some_and(|f| f.title == title) {
            return;
        }
        let id = self.fence_id;
        let task = spawn_db(cx, move |db| core_fence::rename(db, id, &title));
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, window, cx| match result {
                Ok(renamed) => {
                    window.set_window_title(&renamed.fence.title);
                    this.apply_fence(renamed.fence, cx);
                    if let Some(error) = renamed.folder_error {
                        message::warning(
                            window,
                            cx,
                            format!("标题已修改，但收纳文件夹没能跟着改名：{error}"),
                        );
                    }
                }
                Err(e) => message::error_with(window, cx, "重命名收纳箱失败", &e),
            });
        })
        .detach();
    }

    // ------------------------------------------------------------------
    // 缩放（窗口内 8 个手柄）
    // ------------------------------------------------------------------

    /// 当前光标：Windows 用屏幕物理坐标（拖左 / 上边时窗口在动，窗口坐标不可靠）；其它平台用窗口坐标
    fn cursor_for_resize(&self, position: Point<Pixels>, window: &Window) -> (f64, f64) {
        match platform::cursor_position_physical() {
            Some((x, y)) => (f64::from(x), f64::from(y)),
            None => {
                let origin = window.bounds().origin;
                (
                    f64::from(f32::from(position.x + origin.x)),
                    f64::from(f32::from(position.y + origin.y)),
                )
            }
        }
    }

    fn begin_resize(
        &mut self,
        dir: ResizeDir,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.can_resize() || self.resizing.is_some() {
            return;
        }
        self.flush_geometry(window, cx).detach();
        let Some(start) = platform::client_rect_physical(window) else {
            return;
        };
        self.resizing = Some(ResizeSession {
            dir,
            start_cursor: self.cursor_for_resize(event.position, window),
            start: runtime::to_px(start),
            dpr: f64::from(window.scale_factor()),
        });
        cx.stop_propagation();
    }

    fn resize_move(
        &mut self,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.resizing else {
            return;
        };
        let (cx_now, cy_now) = self.cursor_for_resize(position, window);
        let physical = platform::cursor_position_physical().is_some();
        // 非 Windows 的窗口坐标是逻辑像素，乘缩放换成物理像素
        let scale = if physical { 1. } else { session.dpr };
        let dx = (cx_now - session.start_cursor.0) * scale;
        let dy = (cy_now - session.start_cursor.1) * scale;
        let min = logic::js_round(core_fence::MIN_WIDTH_DIP * session.dpr) as f64;
        let rect = logic::resize_rect(session.start, session.dir, dx, dy, min, min);
        self.resize_pending = Some(rect);
        if !self.resize_frame_scheduled {
            // 每帧最多一次 SetWindowPos
            self.resize_frame_scheduled = true;
            cx.on_next_frame(window, |this, window, cx| {
                this.resize_frame_scheduled = false;
                this.flush_resize(window, cx);
            });
        }
    }

    fn flush_resize(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(rect) = self.resize_pending.take() {
            let dpr = self.resizing.map(|s| s.dpr).unwrap_or(1.);
            runtime::apply_window_rect(window, cx, rect, dpr);
        }
    }

    fn end_resize(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.resizing.is_none() {
            return;
        }
        self.flush_resize(window, cx);
        self.resizing = None;
        // 摆窗是延后执行的：排在它之后再读外框落库（缩放结束直接落库，不经闸门）
        cx.spawn_in(window, async move |this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(16))
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.commit_geometry(window, cx).detach();
            });
        })
        .detach();
    }

    fn render_resize_handles(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        if !self.can_resize() {
            return Vec::new();
        }
        ResizeDir::ALL
            .iter()
            .map(|&dir| {
                let base = div()
                    .id(SharedString::from(format!("resize-{}", dir.key())))
                    .absolute()
                    .occlude();
                let base = match dir {
                    ResizeDir::N => base.top_0().left(px(10.)).right(px(10.)).h(px(5.)),
                    ResizeDir::S => base.bottom_0().left(px(10.)).right(px(10.)).h(px(5.)),
                    ResizeDir::W => base.left_0().top(px(10.)).bottom(px(10.)).w(px(5.)),
                    ResizeDir::E => base.right_0().top(px(10.)).bottom(px(10.)).w(px(5.)),
                    ResizeDir::NW => base.top_0().left_0().size(px(10.)),
                    ResizeDir::NE => base.top_0().right_0().size(px(10.)),
                    ResizeDir::SW => base.bottom_0().left_0().size(px(10.)),
                    ResizeDir::SE => base.bottom_0().right_0().size(px(10.)),
                };
                let cursor = match dir {
                    ResizeDir::N | ResizeDir::S => CursorStyle::ResizeUpDown,
                    ResizeDir::W | ResizeDir::E => CursorStyle::ResizeLeftRight,
                    ResizeDir::NW | ResizeDir::SE => CursorStyle::ResizeUpLeftDownRight,
                    ResizeDir::NE | ResizeDir::SW => CursorStyle::ResizeUpRightDownLeft,
                };
                base.cursor(cursor)
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                            this.begin_resize(dir, event, window, cx)
                        }),
                    )
                    .on_drag(ResizeDrag(dir), |_, _, _, cx| cx.new(|_| EmptyDragView))
                    .on_drag_move::<ResizeDrag>(cx.listener(
                        move |this, event: &DragMoveEvent<ResizeDrag>, window, cx| {
                            // 8 个手柄都会收到同一次拖动：只由按下的那个处理
                            if event.drag(cx).0 == dir {
                                this.resize_move(event.event.position, window, cx)
                            }
                        },
                    ))
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(|this, _, window, cx| this.end_resize(window, cx)),
                    )
                    .on_mouse_up_out(
                        MouseButton::Left,
                        cx.listener(|this, _, window, cx| this.end_resize(window, cx)),
                    )
                    .into_any_element()
            })
            .collect()
    }

    // ------------------------------------------------------------------
    // 外部拖入
    // ------------------------------------------------------------------

    fn on_external_drag_over(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.drag_epoch += 1;
        if !self.drag_hover {
            self.drag_hover = true;
            cx.notify();
        }
        self.set_reason(ExpandReason::Drag, true, window, cx);
    }

    fn on_external_drag_end(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.drag_hover = false;
        self.drag_epoch += 1;
        let epoch = self.drag_epoch;
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            cx.background_executor()
                .timer(timing::DRAG_COLLAPSE_DELAY)
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                if this.drag_epoch == epoch {
                    this.set_reason(ExpandReason::Drag, false, window, cx);
                }
            });
        })
        .detach();
    }

    fn on_external_drop(
        &mut self,
        paths: &ExternalPaths,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let paths: Vec<String> = paths
            .paths()
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect();
        self.on_external_drag_end(window, cx);
        self.move_in(paths, window, cx);
    }

    /// 旧 `moveIn`：后台移入（带 owner HWND，系统对话框以本窗口为父）→ 汇总提示（5 秒；没开「显示隐藏的图标」时
    /// 说明以 . 开头的项目已自动隐藏）→ OneDrive 一次性提示 → 刷新
    fn move_in(&mut self, paths: Vec<String>, window: &mut Window, cx: &mut Context<Self>) {
        if paths.is_empty() {
            return;
        }
        let id = self.fence_id;
        let owner = platform::hwnd(window).map(|h| h.0);
        let task = spawn_db(cx, move |db| items::move_in(db, id, &paths, owner));
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok((moved, refreshes)) => {
                        let summary = logic::summarize_op_results(&moved.results);
                        if let Some((kind, text)) =
                            logic::describe_move_in(&summary, this.show_hidden())
                        {
                            message::show_for(
                                window,
                                cx,
                                kind,
                                text,
                                timing::MOVE_IN_MESSAGE_DURATION,
                            );
                        }
                        if moved.onedrive_notice {
                            this.set_panel(Panel::OneDrive, window, cx);
                        }
                        for refresh in refreshes {
                            runtime::deliver_refresh(cx, refresh);
                        }
                    }
                    Err(e) => message::error_with(window, cx, "收纳失败", &e),
                }
                this.refresh_items(cx);
            });
        })
        .detach();
    }

    // ------------------------------------------------------------------
    // 键盘
    // ------------------------------------------------------------------

    fn on_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if event.keystroke.key != "escape" {
            return;
        }
        if self.editing_title {
            self.finish_rename(false, window, cx);
            cx.stop_propagation();
        } else if self.item_menu.is_some() {
            // 右键菜单先拦截 Esc（这次 Esc 不再关面板）
            self.close_item_menu(window, cx);
            cx.stop_propagation();
        } else if self.title_menu_open {
            self.close_title_menu(window, cx);
            cx.stop_propagation();
        } else if self.panel != Panel::None && !self.deleting {
            self.set_panel(Panel::None, window, cx);
            cx.stop_propagation();
        }
    }

    // ------------------------------------------------------------------
    // 渲染
    // ------------------------------------------------------------------

    fn title_button(
        &self,
        id: &'static str,
        source: EpIcon,
        tip: &'static str,
        active: bool,
        on_click: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let t = fence_tokens();
        let color = if active { t.button_active } else { t.button };
        div()
            .id(id)
            .size(px(24.))
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .rounded(theme::radius::SM)
            .cursor_pointer()
            .hover(move |s| s.bg(t.button_hover_bg))
            .tooltip(crate::ui::tooltip::native(tip))
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                cx.stop_propagation();
                on_click(this, window, cx)
            }))
            .child(icon(source, 14., color))
    }

    fn render_title_bar(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let t = fence_tokens();
        let expanded = self.view_expanded();
        let collapsed = self.collapsed();
        let locked = self.locked();
        let title: SharedString = self
            .fence
            .as_ref()
            .map(|f| f.title.clone().into())
            .or_else(|| self.load_error.clone())
            .unwrap_or_default();
        let has_fence = self.fence.is_some();

        let title_el = if self.editing_title {
            div()
                .flex_1()
                .min_w_0()
                .text_color(theme::ep().text_regular)
                .child(EpInput::new(&self.title_input).small().w_full())
                .into_any_element()
        } else {
            div()
                .id("fence-title")
                .flex_1()
                .min_w_0()
                .relative()
                .font_weight(theme::WEIGHT_SEMIBOLD)
                .when(has_fence, |this| {
                    this.tooltip(crate::ui::tooltip::native(title.clone()))
                })
                // text-shadow: 0 1px 2px rgba(0,0,0,.4)：先在下方 1px 画一遍阴影色，再画正文
                .child(
                    div()
                        .absolute()
                        .top(px(1.))
                        .left_0()
                        .right_0()
                        .truncate()
                        .text_color(t.title_shadow)
                        .child(title.clone()),
                )
                .child(div().relative().truncate().child(title))
                .into_any_element()
        };

        let actions = has_fence.then(|| {
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap(px(2.))
                .opacity(if self.hovered || self.title_menu_open {
                    1.
                } else {
                    0.55
                })
                // 按钮区的按下 / 双击不触发拖动、改名
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(self.title_button(
                    "fence-collapse",
                    if collapsed {
                        EpIcon::ArrowDown
                    } else {
                        EpIcon::ArrowUp
                    },
                    if collapsed { "展开" } else { "折叠" },
                    false,
                    |this, window, cx| this.toggle_collapsed(window, cx),
                    cx,
                ))
                .child(self.title_button(
                    "fence-lock",
                    if locked { EpIcon::Lock } else { EpIcon::Unlock },
                    if locked {
                        "取消固定"
                    } else {
                        "固定位置和大小"
                    },
                    locked,
                    |this, window, cx| this.toggle_locked(window, cx),
                    cx,
                ))
                .child(
                    div()
                        .relative()
                        .child(self.title_button(
                            "fence-more",
                            EpIcon::MoreFilled,
                            "更多",
                            false,
                            |this, window, cx| this.toggle_title_menu(window, cx),
                            cx,
                        ))
                        .child(self.more_anchor.measure()),
                )
        });

        div()
            .id("fence-titlebar")
            .flex()
            .flex_none()
            .items_center()
            .gap(px(6.))
            .h(px(core_fence::FENCE_TITLE_BAR_DIP as f32))
            .pl(px(10.))
            .pr(px(4.))
            .cursor_default()
            .when(expanded, |this| {
                this.border_b_1().border_color(t.title_divider)
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, window, cx| {
                    this.on_title_mouse_down(event, window, cx)
                }),
            )
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, window, cx| {
                this.on_title_mouse_move(event, window, cx)
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, _| this.press_point = None),
            )
            .when(has_fence, |this| {
                this.on_click(cx.listener(|this, event: &ClickEvent, window, cx| {
                    if event.click_count() >= 2 && !this.editing_title {
                        this.start_rename(window, cx);
                    }
                }))
            })
            .child(title_el)
            .children(actions)
            .children(self.render_title_menu(window, cx))
    }

    fn render_body(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = fence_tokens();
        let content = if self.fence.is_none() {
            div().into_any_element()
        } else if self.display_items().is_empty() {
            // 真的没有项目时引导拖入；有项目但全部隐藏（没开「显示隐藏的图标」）时说明隐藏了几个
            let all_hidden = !self.items.is_empty();
            div()
                .size_full()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(8.))
                .text_color(t.empty)
                .child(icon(
                    if all_hidden {
                        EpIcon::Hide
                    } else {
                        EpIcon::FolderOpened
                    },
                    28.,
                    t.empty,
                ))
                .child(logic::empty_hint(self.items.len()))
                .into_any_element()
        } else {
            self.render_grid(window, cx).into_any_element()
        };
        div()
            .id("fence-body")
            .relative()
            .flex_1()
            .min_h_0()
            .p(px(8.))
            .overflow_hidden()
            .rounded_b(theme::radius::MD)
            .child(content)
            .when(self.drag_hover, |this| {
                // outline: 2px dashed rgba(255,255,255,.45); outline-offset: -6px
                this.child(
                    div()
                        .absolute()
                        .inset(px(4.))
                        .border_2()
                        .border_dashed()
                        .border_color(t.drop_outline),
                )
            })
            .children(self.render_panel(window, cx))
    }
}

impl Render for FenceWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = fence_tokens();
        let (r, g, b) = self
            .fence
            .as_ref()
            .map(|f| theme::hex_to_rgb_channels(&f.bg_color))
            .unwrap_or((0, 0, 0));
        let bg = theme::rgba_css(r, g, b, self.bg_alpha() as f32);
        let expanded = self.view_expanded();
        self.pump_icons(window, cx);

        let root = div()
            .id("fence-window")
            .key_context("FenceWindow")
            .track_focus(&self.focus)
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .overflow_hidden()
            .font(theme::ui_font())
            .text_size(theme::FENCE_FONT_SIZE)
            .line_height(relative(theme::LINE_HEIGHT))
            .text_color(t.text)
            .bg(bg)
            .border_1()
            .border_color(t.window_border)
            .rounded(theme::radius::MD)
            .on_hover(cx.listener(|this, hovered: &bool, window, cx| {
                this.on_hover_changed(*hovered, window, cx)
            }))
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                this.on_key_down(event, window, cx)
            }))
            .on_any_mouse_down(cx.listener(|this, _: &MouseDownEvent, window, cx| {
                if !this.editing_title {
                    this.focus.focus(window, cx);
                }
            }))
            // 整个窗口（含标题栏）都是外部拖入的放置区；锁定状态下照样可以拖入
            .on_drag_move::<ExternalPaths>(cx.listener(
                |this, _: &DragMoveEvent<ExternalPaths>, window, cx| {
                    this.on_external_drag_over(window, cx)
                },
            ))
            .capture_any_mouse_up(cx.listener(|this, _, window, cx| {
                // 箱内拖动排序在网格外松手：收尾
                if this.sorting.is_some() {
                    this.finish_sort(window, cx);
                }
            }))
            .child(self.render_title_bar(window, cx))
            .when(expanded, |this| this.child(self.render_body(window, cx)))
            .children(self.render_resize_handles(cx))
            .children(self.render_item_menu(window, cx));
        // 松手 / 拖出不用 GPUI 的 on_drop / on_file_drop_exit：它们要求悬停命中，键盘模态下（例如刚改完名按了回车）
        // 一律不命中，第一次拖入会被丢掉、悬停框残留（e2e F10）。窗口任何位置松手都收进本收纳箱（旧版 onDragDropEvent 语义）
        self.external_drop.attach(
            root,
            DropScope::Window,
            cx.listener(|this, paths: &ExternalPaths, window, cx| {
                this.on_external_drop(paths, window, cx)
            }),
            cx.listener(|this, _: &FileDropEvent, window, cx| {
                this.on_external_drag_end(window, cx)
            }),
        )
    }
}

// ----------------------------------------------------------------------
// 入口（托盘 / 启动钩子 / 调试）
// ----------------------------------------------------------------------

/// 启动时恢复全部收纳箱窗口（`run()` 里调用；仅 Windows）
pub fn restore_all(cx: &mut App) {
    runtime::restore_all(cx);
}

/// 打开（或带到前台）某个收纳箱窗口（调试 `MINI_TODO_DEBUG_OPEN=fence:<id>`）。窗口异步建好，已存在时返回它
pub fn open(cx: &mut App, fence_id: i64) -> Option<AnyWindowHandle> {
    if let Some(existing) = crate::windows::WindowManager::find(
        cx,
        crate::windows::WindowKind::Fence,
        &fence_key(fence_id),
    ) {
        return Some(existing);
    }
    runtime::open_by_id(cx, fence_id).detach();
    None
}

/// 托盘「新增收纳箱」
pub fn tray_create_fence(cx: &mut App) {
    runtime::create_fence(cx);
}

/// 托盘「显示/隐藏所有收纳箱」
pub fn tray_toggle_fences(cx: &mut App) {
    runtime::toggle_visibility(cx);
}

/// 托盘「重置收纳箱位置」
pub fn tray_reset_fences(cx: &mut App) {
    runtime::reset_positions(cx);
}
