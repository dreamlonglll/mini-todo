//! 待办清单窗口（旧 `MainView.vue` + `TitleBar.vue` + `TodoList.vue` + `TodoItem.vue` + `QuadrantView.vue` 的清单部分），
//! 每个清单一个（[`BlockKey::List`]；默认清单窗口 = 以前的主窗口）。规格：`research/ui-main-window.md`（日历部分除外）。
//!
//! | 子模块 | 内容 |
//! |---|---|
//! | [`title_bar`] | 标题栏（自绘移动、清单名、版本号 / 更新红点（只在默认清单窗口）、展开全部、右侧 已完成 / 视图 / 日夜 / 锁 / 同步 / 设置） |
//! | [`name`] | 清单名：双击就地改名、右键菜单「重命名 / 删除清单」、删除确认 |
//! | [`todo_item`] | 待办行（列表与四象限共用）、操作条、子任务展开、拖拽影子 |
//! | [`row_menu`] | 待办行的右键菜单「移动到 ▸」（级联列出其它清单，跟随本窗口的日 / 夜） |
//! | [`list`] | 列表视图、空状态、列表拖拽排序 |
//! | [`quadrant`] | 四象限视图与跨象限拖拽 |
//! | [`logic`] | 纯逻辑（重复文案、象限颜色、拖拽排序、版本比较……，带单测） |
//!
//! - 只显示**有效清单**等于本清单的待办（`AppState::pending_todos_in`：归属未知 / 清单不存在的待办按默认清单）；
//!   右键待办行「移动到」别的清单（[`row_menu`]）
//! - 新建待办（都建到本清单，`editor::open_new`）：右下角 FAB（**锁定时不显示**）、Ctrl+N / ⌘N（[`NewTodo`]，窗口有焦点时，
//!   锁定与否都行；改名中不响应）、双击空白处（列表：行以外的空白与空状态；四象限：双击某个象限里行以外的地方，预选该象限；
//!   四象限网格外的空白按列表处理）。双击判定在按下时（`click_count >= 2`）：待办行对第二下按下 `stop_propagation`，所以双击行
//!   只打开详情；拖拽排序会话还在（含松手后的落库中）时不新建
//! - 日 / 夜、列表 / 四象限、展开全部按窗口（`AppState::window_prefs`）；锁定按窗口（`services::window_mode`）
//! - 生命周期（启动开窗、关闭 = 隐藏、同步来的清单开 / 关窗、托盘唤起）在 `services::block_windows`；窗口模式（锁定 / 嵌入桌面）、
//!   几何保存 / 恢复、贴边隐藏、唤起见 `services::window_mode`；日历是另一个窗口（`windows::calendar_window`）

pub mod list;
pub mod logic;
pub mod name;
pub mod quadrant;
pub mod row_menu;
pub mod title_bar;
pub mod todo_item;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use gpui_kit::{
    AnyElement, App, Bounds, Context, Entity, FocusHandle, Font, Global, Hsla,
    InteractiveElement as _, IntoElement, KeyBinding, MouseButton, MouseDownEvent, MouseMoveEvent,
    ParentElement as _, Pixels, Point, Render, ScrollHandle, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, Task, Window, actions, deferred,
    div, prelude::FluentBuilder as _, px, relative,
};
use mini_todo_core::db::Todo;

use super::block::{BlockDrag, BlockDragHost, BlockKey, DEFAULT_LIST_ID};
use super::drag;
use super::snap::SnapKey;
use super::{ChildOf, WindowManager, modal_overlay};
use crate::assets::EpIcon;
use crate::services::{block_windows, window_mode};
use crate::state::{AppState, ViewMode};
use crate::theme::{self, MainColors, Palette, main_window as dims};
use crate::ui::icon::icon;
use crate::ui::message::{self, MessageKind};
use crate::ui::scrollbar::{self, ScrollbarState};
use crate::ui::tooltip;
use crate::windows::editor;

pub const DEFAULT_WIDTH: f32 = 380.;
pub const DEFAULT_HEIGHT: f32 = 600.;
pub const MIN_WIDTH: f32 = 320.;
pub const MIN_HEIGHT: f32 = 400.;
/// "操作模式"：悬停操作条 750ms 后 FAB 变暗，离开 1.5s 后恢复
const FAB_DIM_ENTER_DELAY: Duration = Duration::from_millis(750);
const FAB_DIM_LEAVE_DELAY: Duration = Duration::from_millis(1500);
/// 子任务展开 / 收起动画
const SUBTASK_ANIMATION: Duration = Duration::from_millis(200);

actions!(mini_todo_list, [NewTodo]);

/// 清单窗口根元素的 key context
const LIST_CONTEXT: &str = "MiniTodoList";

/// 新建待办快捷键的显示文案（绑定是 GPUI 的 `secondary-n`：macOS ⌘N，其它平台 Ctrl+N）
pub(crate) const NEW_TODO_SHORTCUT: &str = if cfg!(target_os = "macos") {
    "⌘N"
} else {
    "Ctrl+N"
};

/// 已注册过清单窗口快捷键的标记（`App::bind_keys` 不去重）
struct ListShortcutsBound;

impl Global for ListShortcutsBound {}

/// 注册清单窗口的快捷键（建第一个清单窗口时；之后的调用什么都不做）
fn init_shortcuts(cx: &mut App) {
    if cx.has_global::<ListShortcutsBound>() {
        return;
    }
    cx.set_global(ListShortcutsBound);
    cx.bind_keys([KeyBinding::new("secondary-n", NewTodo, Some(LIST_CONTEXT))]);
}

/// 一行子任务
#[derive(Clone, Debug)]
pub(crate) struct SubRow {
    pub id: i64,
    pub title: SharedString,
    pub completed: bool,
}

/// 一行待办需要的数据（render 时从 AppState 拷出）
#[derive(Clone, Debug)]
pub(crate) struct RowData {
    pub id: i64,
    pub title: SharedString,
    pub color: Hsla,
    pub completed: bool,
    pub subtasks: Vec<SubRow>,
    pub done: usize,
    /// 开启重复时的 tooltip 文案（可能为空串）
    pub repeat: Option<SharedString>,
    pub notify: Option<SharedString>,
}

impl RowData {
    pub fn from_todo(todo: &Todo) -> RowData {
        RowData {
            id: todo.id,
            title: todo.title.clone().into(),
            color: theme::todo_color(&todo.color, theme::light().text_tertiary),
            completed: todo.completed,
            done: todo.subtasks.iter().filter(|s| s.completed).count(),
            subtasks: todo
                .subtasks
                .iter()
                .map(|s| SubRow {
                    id: s.id,
                    title: s.title.clone().into(),
                    completed: s.completed,
                })
                .collect(),
            repeat: todo
                .repeat_enabled
                .then(|| logic::repeat_tooltip(todo).into()),
            notify: logic::notify_text(todo).map(Into::into),
        }
    }

    pub fn has_meta(&self) -> bool {
        !self.subtasks.is_empty() || self.repeat.is_some() || self.notify.is_some()
    }
}

/// 渲染一帧用到的公共参数
#[derive(Clone)]
pub(crate) struct RenderCtx {
    pub palette: Palette,
    pub colors: MainColors,
    pub todo_font: Font,
    pub font_size: f32,
    pub expand_all: bool,
}

impl RenderCtx {
    pub fn sub_size(&self) -> f32 {
        self.font_size - 2.
    }
}

/// 拖拽中的待办（GPUI 拖拽值）
#[derive(Clone)]
pub(crate) struct DraggedTodo {
    pub id: i64,
}

/// 一次拖拽排序会话（拖动中 + 松手后的落库中；期间按会话里的顺序渲染，不按 store 重建）
pub(crate) struct DragSession {
    pub id: i64,
    pub kind: DragKind,
    /// 已松手（落库中）
    pub dropped: bool,
    /// 最近一次指针位置（自动滚动用）
    pub pointer: Option<Point<Pixels>>,
}

pub(crate) enum DragKind {
    List {
        original: Vec<i64>,
        order: Vec<i64>,
    },
    Quadrant {
        original: [Vec<i64>; 4],
        lists: [Vec<i64>; 4],
        origin: usize,
        current: usize,
    },
}

/// 子任务展开 / 收起动画
#[derive(Clone, Copy, Debug)]
pub(crate) struct ExpandAnim {
    pub expanding: bool,
    pub generation: u64,
}

pub struct ListWindow {
    /// 这个窗口在 `window_mode` 状态表 / 几何持久化 / 吸附组里的 key（`BlockKey::List(清单 id)`）
    pub(crate) block: BlockKey,
    /// 标题栏自绘移动与 8 个缩放手柄（与收纳箱同一套做法，吸附到吸附组里的其它窗口）
    window_drag: BlockDrag,
    /// 根元素的焦点（[`NewTodo`] 快捷键要焦点在本窗口的元素树里才派发得到）：建窗、窗口激活时焦点不在树里、改名结束时拿回来
    focus: FocusHandle,
    pub(crate) state: Entity<AppState>,
    pub(crate) scroll: ScrollHandle,
    pub(crate) scrollbar: ScrollbarState,
    /// 单行展开覆盖（初值跟随本窗口的"展开全部"，它变化时清空）
    pub(crate) expanded: HashMap<i64, bool>,
    pub(crate) expand_anims: HashMap<i64, ExpandAnim>,
    anim_generation: u64,
    pub(crate) hovered_row: Option<i64>,
    /// 上一帧画出来的各行 / 各象限 / 操作条的窗口坐标（拖拽插入位置、FAB 变暗判断用）
    pub(crate) row_bounds: Rc<RefCell<HashMap<i64, Bounds<Pixels>>>>,
    pub(crate) quadrant_bounds: Rc<RefCell<[Option<Bounds<Pixels>>; 4]>>,
    pub(crate) actions_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    pub(crate) drag: Option<DragSession>,
    pub(crate) fab_dimmed: bool,
    pointer_in_actions: bool,
    fab_enter: Option<Task<()>>,
    fab_leave: Option<Task<()>>,
    last_expand_all: bool,
    last_view_mode: ViewMode,
    /// 系统窗口标题跟着清单名（默认清单窗口固定是 "Mini Todo"）
    last_name: String,
    /// 清单名的就地改名与右键菜单
    name: name::NameEditor,
    /// 待办行的右键菜单（「移动到 ▸」）
    row_menu: Option<row_menu::RowMenu>,
    _subscriptions: Vec<Subscription>,
}

impl ListWindow {
    pub fn new(list_id: i64, window: &mut Window, cx: &mut Context<Self>) -> Self {
        init_shortcuts(cx);
        let block = BlockKey::List(list_id);
        let state = AppState::global(cx);
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        let snapshot = state.read(cx);
        let last_expand_all = snapshot.expand_all(block);
        let last_view_mode = snapshot.view_mode(block);
        let last_name = snapshot.list_name(list_id);
        let mut subscriptions = vec![
            cx.observe_in(&state, window, Self::on_state_changed),
            cx.observe_global::<WindowManager>(|_, cx| cx.notify()),
            cx.observe_window_activation(window, |this, window, cx| {
                if !window.is_window_active() {
                    // 切到别的窗口：收起本窗口的右键菜单
                    this.close_row_menu(cx);
                    this.close_name_menu(cx);
                    return;
                }
                // macOS：贴边隐藏中被 Dock / ⌘Tab 激活时拉回（Windows 上是空操作）
                let block = this.block;
                cx.defer(move |cx| window_mode::on_activated(cx, block));
                if !WindowManager::forward_activation(window, cx) {
                    this.ensure_focus(window, cx);
                    AppState::global(cx).update(cx, |state, cx| state.reload_todos(cx));
                }
            }),
            // 系统 / 程序挪动、改尺寸（DPI 变化、显示器变化…）后防抖保存；用户自绘拖动 / 缩放中不算，
            // 松手后由 BlockDrag 直接保存一次
            cx.observe_window_bounds(window, |this, _, cx| {
                if !this.window_drag.is_busy() {
                    window_mode::schedule_save(cx, this.block)
                }
            }),
        ];
        // 整个应用的提示（加载待办失败）不在这里订阅：`services::notice` 挑一个看得见的清单 / 日历窗口显示
        // 关闭（Alt+F4 / ⌘W）= 隐藏并记住，不删清单、不退出（Linux 上真的关窗）
        window.on_window_should_close(cx, move |window, cx| {
            block_windows::request_close(window, cx, block)
        });
        let (name, name_subscription) = name::NameEditor::new(window, cx);
        subscriptions.push(name_subscription);

        Self {
            block,
            window_drag: BlockDrag::new(block, f64::from(MIN_WIDTH), f64::from(MIN_HEIGHT)),
            focus,
            state,
            scroll: ScrollHandle::new(),
            scrollbar: ScrollbarState::new(),
            expanded: HashMap::new(),
            expand_anims: HashMap::new(),
            anim_generation: 0,
            hovered_row: None,
            row_bounds: Rc::default(),
            quadrant_bounds: Rc::default(),
            actions_bounds: Rc::default(),
            drag: None,
            fab_dimmed: false,
            pointer_in_actions: false,
            fab_enter: None,
            fab_leave: None,
            last_expand_all,
            last_view_mode,
            last_name,
            name,
            row_menu: None,
            _subscriptions: subscriptions,
        }
    }

    /// 本窗口的清单 id
    pub(crate) fn list_id(&self) -> i64 {
        self.block.list_id().unwrap_or(DEFAULT_LIST_ID)
    }

    /// 观察全局状态：展开全部 / 视图切换 / 清单改名的变化各自收尾（嵌入偏好与运行时偏好由 `window_mode` 集中观察，
    /// 对所有窗口收尾）
    fn on_state_changed(
        &mut self,
        state: Entity<AppState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let s = state.read(cx);
        let list_id = self.list_id();
        let (expand_all, view_mode) = (s.expand_all(self.block), s.view_mode(self.block));
        let animate_ids: Vec<i64> = if expand_all != self.last_expand_all {
            crate::state::todos_in_list(&s.todos, &s.lists, list_id)
                .filter(|t| !t.subtasks.is_empty())
                .filter(|t| self.is_expanded(t.id, self.last_expand_all) != expand_all)
                .map(|t| t.id)
                .collect()
        } else {
            Vec::new()
        };
        let name = s.list(list_id).map(|l| l.name.clone());
        // 右键菜单对应的待办已不是本清单的未完成待办（被移走 / 完成 / 删除 / 同步改了归属）：收起菜单
        let row_menu_stale = self.row_menu.as_ref().is_some_and(|menu| {
            !crate::state::todos_in_list(&s.todos, &s.lists, list_id)
                .any(|t| t.id == menu.todo_id() && !t.completed)
        });
        if row_menu_stale {
            self.row_menu = None;
        }

        if expand_all != self.last_expand_all {
            self.last_expand_all = expand_all;
            self.expanded.clear();
            for id in animate_ids {
                self.start_expand_animation(id, expand_all, cx);
            }
        }
        if view_mode != self.last_view_mode {
            // 列表 ⇄ 四象限：行组件重建，展开状态回到本窗口的"展开全部"
            self.last_view_mode = view_mode;
            self.expanded.clear();
            self.expand_anims.clear();
            self.drag = None;
            self.hovered_row = None;
            self.row_menu = None;
        }
        if let Some(name) = name
            && name != self.last_name
        {
            if list_id != DEFAULT_LIST_ID {
                window.set_window_title(&name);
            }
            self.last_name = name;
        }
        cx.notify();
    }

    // ------------------------------------------------------------------
    // 子任务展开
    // ------------------------------------------------------------------

    pub(crate) fn is_expanded(&self, id: i64, expand_all: bool) -> bool {
        self.expanded.get(&id).copied().unwrap_or(expand_all)
    }

    pub(crate) fn toggle_expanded(&mut self, id: i64, cx: &mut Context<Self>) {
        let next = !self.is_expanded(id, self.last_expand_all);
        self.expanded.insert(id, next);
        self.start_expand_animation(id, next, cx);
        cx.notify();
    }

    fn start_expand_animation(&mut self, id: i64, expanding: bool, cx: &mut Context<Self>) {
        self.anim_generation += 1;
        let generation = self.anim_generation;
        self.expand_anims.insert(
            id,
            ExpandAnim {
                expanding,
                generation,
            },
        );
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(SUBTASK_ANIMATION + Duration::from_millis(30))
                .await;
            let _ = this.update(cx, |this, cx| {
                if this
                    .expand_anims
                    .get(&id)
                    .is_some_and(|a| a.generation == generation)
                {
                    this.expand_anims.remove(&id);
                    cx.notify();
                }
            });
        })
        .detach();
    }

    // ------------------------------------------------------------------
    // FAB "操作模式"
    // ------------------------------------------------------------------

    fn on_root_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.fab_visible(cx) {
            // 锁定时没有 FAB：不进"操作模式"（操作条也不用压到 FAB 之上）
            self.reset_fab_dim();
            return;
        }
        let inside = self
            .actions_bounds
            .get()
            .is_some_and(|b| b.contains(&event.position));
        if inside {
            if !self.pointer_in_actions {
                self.pointer_in_actions = true;
                self.fab_leave = None;
                if !self.fab_dimmed && self.fab_enter.is_none() {
                    self.fab_enter = Some(cx.spawn(async move |this, cx| {
                        cx.background_executor().timer(FAB_DIM_ENTER_DELAY).await;
                        let _ = this.update(cx, |this, cx| {
                            this.fab_enter = None;
                            this.fab_dimmed = true;
                            cx.notify();
                        });
                    }));
                }
            }
        } else {
            // 悬停到操作条以外（含 FAB 本身，以及操作条随行被删除 / 完成后消失的兜底）
            self.pointer_in_actions = false;
            self.fab_enter = None;
            self.start_fab_recovery(cx);
        }
    }

    fn start_fab_recovery(&mut self, cx: &mut Context<Self>) {
        if self.fab_dimmed && self.fab_leave.is_none() {
            self.fab_leave = Some(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(FAB_DIM_LEAVE_DELAY).await;
                let _ = this.update(cx, |this, cx| {
                    this.fab_leave = None;
                    this.fab_dimmed = false;
                    cx.notify();
                });
            }));
        }
    }

    /// FAB 只在没锁定时显示（锁定 = 固定模式，含经典固定与嵌入桌面：用 Ctrl+N / 双击空白新建）
    fn fab_visible(&self, cx: &App) -> bool {
        !window_mode::is_fixed(cx, self.block)
    }

    /// 退出"操作模式"（FAB 不显示时）：恢复不变暗、取消计时
    fn reset_fab_dim(&mut self) {
        self.fab_dimmed = false;
        self.pointer_in_actions = false;
        self.fab_enter = None;
        self.fab_leave = None;
    }

    /// `border`：根容器描边宽度（CSS 的 `position: fixed` 相对窗口而不是描边内侧）。新建的待办建到本清单
    fn render_fab(&self, palette: &Palette, border: f32) -> impl IntoElement {
        let dimmed = self.fab_dimmed;
        let size = dims::FAB_SIZE;
        let inset = dims::FAB_INSET - border;
        // transform: scale(1.05) / scale(0.95) 按中心缩放：改尺寸并挪位置
        let grow = size * 0.05 / 2.;
        let primary_dark = palette.primary_dark;
        let preset = editor::NewTodoPreset::in_list(self.list_id());
        let actions = self.actions_bounds.clone();
        div()
            .id("fab-add")
            .group("fab")
            .absolute()
            .right(px(inset))
            .bottom(px(inset))
            .size(px(size))
            .flex()
            .items_center()
            .justify_center()
            .rounded_full()
            .bg(palette.primary)
            .shadow(theme::fab_shadow(false))
            .cursor_pointer()
            // 平时盖住下面的行（旧版 z-index 100）：不 occlude 的话，最后几行的悬停操作条在 FAB 底下，点 FAB 会同时点到
            // 下面的「删除」等按钮。「操作模式」变暗时沉到操作条之下（旧版 z-index 4 < 操作条 5），点在操作条上的归操作条
            .when(!dimmed, |this| this.occlude())
            .when(dimmed, |this| this.opacity(0.35))
            .hover(move |s| {
                s.bg(primary_dark)
                    .size(px(size + grow * 2.))
                    .right(px(inset - grow))
                    .bottom(px(inset - grow))
                    .shadow(theme::fab_shadow(true))
                    .opacity(1.)
            })
            .active(move |s| {
                s.size(px(size - grow * 2.))
                    .right(px(inset + grow))
                    .bottom(px(inset + grow))
            })
            .tooltip(tooltip::native(format!("新建待办（{NEW_TODO_SHORTCUT}）")))
            // 双击 FAB：两下都是点 FAB（第二下打开方已有模态子窗口，只把它带到前台），不能落到下面内容区的"双击空白新建"
            // （变暗时 FAB 不 occlude，内容区也收得到这次按下）
            .on_mouse_down(MouseButton::Left, |event: &MouseDownEvent, _, cx| {
                if event.click_count >= 2 {
                    cx.stop_propagation();
                }
            })
            .on_click(move |_, window, cx| {
                if dimmed
                    && actions
                        .get()
                        .is_some_and(|b| b.contains(&window.mouse_position()))
                {
                    return;
                }
                let parent = ChildOf::window(window, cx);
                editor::open_new(cx, Some(parent), preset.clone());
            })
            .child(
                icon(EpIcon::Plus, 24., theme::hex(0xFFFFFF))
                    .group_hover("fab", |s| s.size(px(24. * 1.05))),
            )
    }

    // ------------------------------------------------------------------
    // 打开子窗口 / 行操作
    // ------------------------------------------------------------------

    /// 新建待办（建到本清单；`quadrant`：四象限里双击的象限）。打开方已有模态子窗口时 `open_child` 只把它带到前台
    fn open_new_todo(&self, quadrant: Option<i32>, window: &mut Window, cx: &mut Context<Self>) {
        let mut preset = editor::NewTodoPreset::in_list(self.list_id());
        if let Some(quadrant) = quadrant {
            preset = preset.with_quadrant(quadrant);
        }
        let parent = ChildOf::window(window, cx);
        editor::open_new(cx, Some(parent), preset);
    }

    /// Ctrl+N / ⌘N：新建待办（锁定与否都行）；清单名改名中不响应（输入框里按下不该把改名提交掉）
    fn on_new_todo_action(&mut self, _: &NewTodo, window: &mut Window, cx: &mut Context<Self>) {
        if self.name.is_renaming() {
            return;
        }
        self.open_new_todo(None, window, cx);
    }

    /// 双击空白处（列表的空白 / 空状态、四象限网格外：`quadrant = None`；某个象限里：该象限）新建待办。
    /// 在按下时判断（`click_count >= 2`）；待办行对双击的第二下 `stop_propagation`，到不了这里。处理了就 `stop_propagation`
    /// （外层的空白处理与下面的缩放手柄不再收到）
    pub(crate) fn on_blank_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        quadrant: Option<i32>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.click_count < 2 {
            return;
        }
        // 拖拽排序中 / 刚松手还在落库：不新建（松手后的下一次按下 click_count 本来就从 1 算起，这里兜底）
        if self.drag.is_some() || cx.has_active_drag() {
            return;
        }
        cx.stop_propagation();
        self.open_new_todo(quadrant, window, cx);
    }

    /// 焦点不在本窗口的元素树里（改名输入框已卸载、别处拿走过焦点……）时交给根元素：[`NewTodo`] 才派发得到
    fn ensure_focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.focus.contains_focused(window, cx) {
            window.focus(&self.focus, cx);
        }
    }

    /// 点击待办：打开只读详情（拖拽刚结束时的点击忽略）
    pub(crate) fn open_todo(&mut self, id: i64, window: &mut Window, cx: &mut Context<Self>) {
        if self.drag.as_ref().is_some_and(|d| !d.dropped) || cx.has_active_drag() {
            return;
        }
        let parent = ChildOf::window(window, cx);
        editor::open(cx, Some(parent), Some(id), editor::EditorMode::View);
    }

    /// 后台写操作失败时在本窗口里提示 `前缀：详情`
    pub(crate) fn notify_on_error<T: 'static>(
        &self,
        task: Task<Result<T, String>>,
        prefix: &'static str,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        let handle = window.window_handle();
        cx.spawn(async move |_, cx| {
            if let Err(e) = task.await {
                log::error!("[list] {prefix}: {e}");
                cx.update(|cx| {
                    message::show_in(
                        handle,
                        cx,
                        MessageKind::Error,
                        message::error_text(prefix, &e),
                    )
                });
            }
        })
        .detach();
    }

    // ------------------------------------------------------------------
    // 布局
    // ------------------------------------------------------------------

    fn render_main_body(
        &mut self,
        rc: &RenderCtx,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let content = match self.state.read(cx).view_mode(self.block) {
            ViewMode::List => self.render_list(rc, window, cx).into_any_element(),
            ViewMode::Quadrant => self.render_quadrant(rc, window, cx).into_any_element(),
        };
        let thumb = rc.palette.border;
        let thumb_hover = rc.palette.text_tertiary;

        // .main-content：padding 0 16px 76px；margin-bottom 6px；滚动条 6px（非覆盖式）。四象限的网格高度由内容决定，
        // 与列表一样整个内容区滚动
        div()
            .flex_1()
            .min_h_0()
            .mb(px(6.))
            .flex()
            .child(
                div()
                    .id("main-content")
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    // 双击空白处新建（列表：行以外 / 空状态；四象限：网格外。象限里的双击由象限自己先处理）
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, event: &MouseDownEvent, window, cx| {
                            this.on_blank_mouse_down(event, None, window, cx)
                        }),
                    )
                    .child(div().px(px(16.)).pb(px(76.)).child(content)),
            )
            .child(scrollbar::vertical(
                "main-scrollbar",
                &self.scroll,
                &self.scrollbar,
                dims::SCROLLBAR,
                thumb,
                thumb_hover,
            ))
    }
}

impl BlockDragHost for ListWindow {
    fn block_drag(&mut self) -> &mut BlockDrag {
        &mut self.window_drag
    }
}

impl Render for ListWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // 拖拽被取消（松手不在任何落点上 / 窗口失焦）：会话还在就收尾
        if self.drag.as_ref().is_some_and(|d| !d.dropped) && !cx.has_active_drag() {
            cx.defer_in(window, |this, window, cx| this.finish_drag(window, cx));
        }
        self.actions_bounds.set(None);

        let fixed = window_mode::is_fixed(cx, self.block);
        let classic_fixed = window_mode::is_classic_fixed(cx, self.block);
        // 锁定时不显示 FAB：也不留"操作模式"（变暗、操作条压到 FAB 之上）的状态
        let show_fab = !fixed;
        if !show_fab {
            self.reset_fab_dim();
        }
        // 吸附组里别的窗口（收纳箱、其它清单 / 日历窗口）拖动 / 缩放时吸附到本窗口的边
        let snap_edges = crate::services::snap::edges(cx, SnapKey::Block(self.block));
        // 锁定（任一种固定）时不能缩放：不放手柄
        let resize_handles = self.window_drag.resize_handles(!fixed, cx);
        let state = self.state.read(cx);
        let palette = state.palette_for(self.block);
        let font_size = state.todo_font_size as f32;
        let rc = RenderCtx {
            palette,
            colors: MainColors::new(&palette),
            todo_font: theme::todo_font(&state.todo_font_family),
            font_size,
            expand_all: state.expand_all(self.block),
        };

        let title_bar = self.render_title_bar(&rc, window, cx).into_any_element();
        let body = self.render_main_body(&rc, window, cx).into_any_element();
        let border = if classic_fixed { 0. } else { 1. };
        let fab = show_fab.then(|| self.render_fab(&palette, border).into_any_element());
        // 清单名 / 待办行的右键菜单（画在本帧最后的浮层；跟随本窗口的日 / 夜）
        let name_menu = self.render_name_menu(palette.is_dark, window, cx);
        let row_menu = self.render_row_menu(palette.is_dark, window, cx);
        // 遮罩是满窗口矩形（盖住描边与圆角外的透明角）
        let overlay: Option<AnyElement> = modal_overlay(window, cx, 0.3).map(|overlay| {
            div()
                .absolute()
                .top(px(-border))
                .left(px(-border))
                .right(px(-border))
                .bottom(px(-border))
                .child(overlay)
                .into_any_element()
        });

        let root = div()
            .id("main-root")
            .key_context(LIST_CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::on_new_todo_action))
            .size_full()
            .relative()
            .flex()
            .flex_col()
            .overflow_hidden()
            .font(theme::ui_font())
            .text_size(theme::FONT_SIZE)
            .line_height(relative(theme::LINE_HEIGHT))
            .text_color(palette.text_primary)
            .bg(palette.window_bg)
            .when(!classic_fixed, |this| {
                this.rounded(theme::radius::MD)
                    .border_1()
                    .border_color(palette.window_border)
            })
            .on_mouse_move(cx.listener(Self::on_root_mouse_move))
            .on_drop(cx.listener(|this, _: &DraggedTodo, window, cx| this.finish_drag(window, cx)));
        self.window_drag
            .root(root, cx)
            .child(title_bar)
            // 缩放手柄在标题栏之上、内容之下：标题栏区域的边与角能缩放；其余边上内容在上面（列表滚动条的滑块照常能拖，
            // 留白与滚动条轨道处缩放）。见 `windows::block` 模块文档
            .children(resize_handles)
            .child(body)
            .children(fab)
            .children(name_menu)
            .children(row_menu)
            .children(overlay)
            .when(!snap_edges.is_empty(), |this| {
                // 跟着窗口圆角（经典固定没有圆角）；在描边内侧，没有命中区
                let radius = if classic_fixed {
                    px(0.)
                } else {
                    theme::radius::MD
                };
                this.child(drag::render_snap_guides(snap_edges, radius))
            })
    }
}

/// 拖拽中操作条在 FAB 变暗时要压在 FAB 之上（CSS z-index 5 > 4）：延后绘制
pub(crate) fn above_fab(element: AnyElement, dimmed: bool) -> AnyElement {
    if dimmed {
        deferred(element).with_priority(1).into_any_element()
    } else {
        element
    }
}
