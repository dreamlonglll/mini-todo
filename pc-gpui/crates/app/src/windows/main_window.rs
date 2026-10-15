//! 主窗口（旧 `MainView.vue` + `TitleBar.vue` + `TodoList.vue` + `TodoItem.vue` + `QuadrantView.vue` +
//! `CalendarView.vue`）。**W1**。规格：`research/ui-main-window.md`（全文）。
//!
//! | 子模块 | 内容 |
//! |---|---|
//! | [`title_bar`] | 标题栏（拖动、版本号 / 更新红点、展开全部、日历月份导航、右侧 6 个按钮） |
//! | [`todo_item`] | 待办行（列表与四象限共用）、操作条、子任务展开、拖拽影子 |
//! | [`list`] | 列表视图、空状态、列表拖拽排序 |
//! | [`quadrant`] | 四象限视图与跨象限拖拽 |
//! | [`calendar`] | 日历（6 周网格、农历 / 节气 / 节日、节假日、跨天条） |
//! | [`logic`] | 纯逻辑（重复文案、象限颜色、日历网格、版本比较……，带单测） |
//!
//! 窗口模式、几何保存 / 恢复、贴边隐藏、托盘唤起见 `services::window_mode`。

pub mod calendar;
pub mod list;
pub mod logic;
pub mod quadrant;
pub mod title_bar;
pub mod todo_item;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use gpui_kit::{
    AnyElement, AnyWindowHandle, App, AppContext as _, Bounds, Context, Entity, Font, Hsla,
    InteractiveElement as _, IntoElement, MouseMoveEvent, ParentElement as _, Pixels, Point,
    Render, ScrollHandle, SharedString, StatefulInteractiveElement as _, Styled as _, Subscription,
    Task, Window, deferred, div, prelude::FluentBuilder as _, px, relative,
};
use mini_todo_core::db::Todo;
use mini_todo_core::settings::RuntimePrefs;

use super::{ChildOf, WindowKind, WindowManager, WindowSpec, logical_size, modal_overlay};
use crate::assets::EpIcon;
use crate::services::window_mode;
use crate::state::{AppState, AppStateEvent, ViewMode};
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
/// 变更计数轮询间隔
const POLL_INTERVAL: Duration = Duration::from_secs(5);
/// "操作模式"：悬停操作条 750ms 后 FAB 变暗，离开 1.5s 后恢复
const FAB_DIM_ENTER_DELAY: Duration = Duration::from_millis(750);
const FAB_DIM_LEAVE_DELAY: Duration = Duration::from_millis(1500);
/// 子任务展开 / 收起动画
const SUBTASK_ANIMATION: Duration = Duration::from_millis(200);

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

pub struct MainWindow {
    pub(crate) state: Entity<AppState>,
    pub(crate) scroll: ScrollHandle,
    pub(crate) scrollbar: ScrollbarState,
    /// 单行展开覆盖（初值跟随全局"展开全部"，全局变化时清空）
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
    pub(crate) calendar: calendar::CalendarState,
    /// 日历打开前的窗口宽度（关闭日历时缩回）
    pre_calendar_width: Option<f32>,
    /// 左栏宽度（日历开启后 = 打开前的窗口宽度；启动时就开着日历则为空 → 40%）
    pub(crate) left_panel_width: Option<f32>,
    /// 首次加载完成后，日历开关变化才调整窗口宽度
    calendar_resize_ready: bool,
    last_expand_all: bool,
    last_view_mode: ViewMode,
    last_show_calendar: bool,
    last_embed_desktop: bool,
    last_prefs: RuntimePrefs,
    _subscriptions: Vec<Subscription>,
}

impl MainWindow {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let state = AppState::global(cx);
        let snapshot = state.read(cx);
        let last_expand_all = snapshot.subtask_expand_all;
        let last_view_mode = snapshot.view_mode;
        let last_show_calendar = snapshot.show_calendar;
        let last_embed_desktop = snapshot.fixed_embed_desktop;
        let last_prefs = snapshot.runtime_prefs;
        let subscriptions =
            vec![
                cx.observe_in(&state, window, Self::on_state_changed),
                cx.subscribe_in(&state, window, |_, _, event: &AppStateEvent, window, cx| {
                    match event {
                        AppStateEvent::Message { kind, text } => {
                            message::show(window, cx, *kind, text.clone())
                        }
                    }
                }),
                cx.observe_global::<WindowManager>(|_, cx| cx.notify()),
                cx.observe_window_activation(window, |this, window, cx| {
                    if !window.is_window_active() {
                        return;
                    }
                    this.calendar.refresh_today(cx);
                    if !WindowManager::forward_activation(window, cx) {
                        AppState::global(cx).update(cx, |state, cx| state.reload_todos(cx));
                    }
                }),
                cx.observe_window_bounds(window, |_, _, cx| window_mode::schedule_save(cx)),
            ];
        window.on_window_should_close(cx, |window, cx| {
            window_mode::on_main_close_requested(window, cx);
            true
        });
        // 5s 变更轮询（模态子窗口打开期间跳过）
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(POLL_INTERVAL).await;
                let alive = this.update(cx, |_, cx| {
                    if !WindowManager::main_has_modal_child(cx) {
                        AppState::global(cx).update(cx, |state, cx| state.refresh_if_changed(cx));
                    }
                });
                if alive.is_err() {
                    break;
                }
            }
        })
        .detach();

        let mut calendar = calendar::CalendarState::new();
        calendar.start(cx);
        Self {
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
            calendar,
            pre_calendar_width: None,
            left_panel_width: None,
            calendar_resize_ready: false,
            last_expand_all,
            last_view_mode,
            last_show_calendar,
            last_embed_desktop,
            last_prefs,
            _subscriptions: subscriptions,
        }
    }

    /// 观察全局状态：展开全部 / 视图切换 / 日历开关 / 嵌入偏好 / 运行时偏好的变化各自收尾
    fn on_state_changed(
        &mut self,
        state: Entity<AppState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let s = state.read(cx);
        let (expand_all, view_mode, show_calendar, embed, prefs, initialized) = (
            s.subtask_expand_all,
            s.view_mode,
            s.show_calendar,
            s.fixed_embed_desktop,
            s.runtime_prefs,
            s.initialized,
        );
        let animate_ids: Vec<i64> = if expand_all != self.last_expand_all {
            s.todos
                .iter()
                .filter(|t| !t.subtasks.is_empty())
                .filter(|t| self.is_expanded(t.id, self.last_expand_all) != expand_all)
                .map(|t| t.id)
                .collect()
        } else {
            Vec::new()
        };

        if expand_all != self.last_expand_all {
            self.last_expand_all = expand_all;
            self.expanded.clear();
            for id in animate_ids {
                self.start_expand_animation(id, expand_all, cx);
            }
        }
        if view_mode != self.last_view_mode {
            // 列表 ⇄ 四象限：行组件重建，展开状态回到全局值
            self.last_view_mode = view_mode;
            self.expanded.clear();
            self.expand_anims.clear();
            self.drag = None;
            self.hovered_row = None;
        }
        if show_calendar != self.last_show_calendar {
            self.last_show_calendar = show_calendar;
            if self.calendar_resize_ready {
                self.resize_for_calendar(show_calendar, window);
            }
        }
        if initialized && !self.calendar_resize_ready {
            self.calendar_resize_ready = true;
        }
        if embed != self.last_embed_desktop {
            self.last_embed_desktop = embed;
            cx.defer(move |cx| window_mode::on_embed_preference_changed(cx, embed));
        }
        if prefs != self.last_prefs {
            self.last_prefs = prefs;
            cx.defer(move |cx| {
                window_mode::apply_runtime_prefs(cx, prefs.top_on_wake, prefs.auto_hide_enabled)
            });
        }
        cx.notify();
    }

    /// 打开 / 关闭日历时调整窗口宽度（旧 MainView `watch(showCalendar)`）
    fn resize_for_calendar(&mut self, show: bool, window: &mut Window) {
        let size = window.bounds().size;
        let (w, h) = (f32::from(size.width), f32::from(size.height));
        if show {
            self.pre_calendar_width = Some(w);
            self.left_panel_width = Some(w);
            let new_width = logic::calendar_window_width(w, h);
            window.resize(logical_size(new_width, h));
        } else {
            self.left_panel_width = None;
            if let Some(previous) = self.pre_calendar_width.take() {
                window.resize(logical_size(previous, h));
            }
        }
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

    /// `border`：根容器描边宽度（CSS 的 `position: fixed` 相对窗口而不是描边内侧）
    fn render_fab(&self, palette: &Palette, border: f32) -> impl IntoElement {
        let dimmed = self.fab_dimmed;
        let size = dims::FAB_SIZE;
        let inset = dims::FAB_INSET - border;
        // transform: scale(1.05) / scale(0.95) 按中心缩放：改尺寸并挪位置
        let grow = size * 0.05 / 2.;
        let primary_dark = palette.primary_dark;
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
            .tooltip(tooltip::native("新建待办"))
            .on_click(|_, window, cx| {
                let parent = ChildOf::window(window, cx);
                editor::open(cx, Some(parent), None, editor::EditorMode::Edit);
            })
            .child(
                icon(EpIcon::Plus, 24., theme::hex(0xFFFFFF))
                    .group_hover("fab", |s| s.size(px(24. * 1.05))),
            )
    }

    // ------------------------------------------------------------------
    // 打开子窗口 / 行操作
    // ------------------------------------------------------------------

    /// 点击待办 / 日历条：打开只读详情（拖拽刚结束时的点击忽略）
    pub(crate) fn open_todo(&mut self, id: i64, window: &mut Window, cx: &mut Context<Self>) {
        if self.drag.as_ref().is_some_and(|d| !d.dropped) || cx.has_active_drag() {
            return;
        }
        let parent = ChildOf::window(window, cx);
        editor::open(cx, Some(parent), Some(id), editor::EditorMode::View);
    }

    /// 后台写操作失败时在主窗口里提示 `前缀：详情`
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
                log::error!("[main] {prefix}: {e}");
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
        let state = self.state.read(cx);
        let show_calendar = state.show_calendar;
        let view_mode = state.view_mode;
        let quadrant_fill = show_calendar && view_mode == ViewMode::Quadrant;

        let content = match view_mode {
            ViewMode::List => self.render_list(rc, window, cx).into_any_element(),
            ViewMode::Quadrant => self
                .render_quadrant(rc, quadrant_fill, window, cx)
                .into_any_element(),
        };
        let thumb = rc.palette.border;
        let thumb_hover = rc.palette.text_tertiary;

        // .main-content：padding 0 16px 76px；margin-bottom 6px；滚动条 6px（非覆盖式）
        let main_content = if quadrant_fill {
            // 日历分栏 + 四象限：左栏被拉伸，四象限撑满，各象限自己滚动
            div()
                .flex_1()
                .min_h_0()
                .mb(px(6.))
                .px(px(16.))
                .pb(px(76.))
                .child(content)
                .into_any_element()
        } else {
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
                .into_any_element()
        };

        let left_panel = div()
            .flex()
            .flex_col()
            .overflow_hidden()
            .map(|this| {
                if show_calendar {
                    let this = this.flex_none().h_full().min_w(px(280.));
                    match self.left_panel_width {
                        Some(w) => this.w(px(w)),
                        None => this.w(relative(0.4)),
                    }
                } else {
                    this.flex_1().min_h_0()
                }
            })
            .child(main_content);

        div()
            .flex_1()
            .min_h_0()
            .flex()
            .overflow_hidden()
            .map(|this| {
                if show_calendar {
                    this.flex_row()
                } else {
                    this.flex_col()
                }
            })
            .child(left_panel)
            .when(show_calendar, |this| {
                this.child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .h_full()
                        .overflow_hidden()
                        .p(px(if rc.palette.is_dark { 8. } else { 12. }))
                        .child(self.render_calendar(rc, window, cx)),
                )
            })
    }
}

impl Render for MainWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // 拖拽被取消（松手不在任何落点上 / 窗口失焦）：会话还在就收尾
        if self.drag.as_ref().is_some_and(|d| !d.dropped) && !cx.has_active_drag() {
            cx.defer_in(window, |this, window, cx| this.finish_drag(window, cx));
        }
        self.actions_bounds.set(None);

        let state = self.state.read(cx);
        let palette = state.main_palette();
        let classic_fixed = state.is_classic_fixed();
        let font_size = state.todo_font_size as f32;
        let rc = RenderCtx {
            palette,
            colors: MainColors::new(&palette),
            todo_font: theme::todo_font(&state.todo_font_family),
            font_size,
            expand_all: state.subtask_expand_all,
        };

        let title_bar = self.render_title_bar(&rc, window, cx).into_any_element();
        let body = self.render_main_body(&rc, window, cx).into_any_element();
        let border = if classic_fixed { 0. } else { 1. };
        let fab = self.render_fab(&palette, border).into_any_element();
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

        div()
            .id("main-root")
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
            .on_drop(cx.listener(|this, _: &DraggedTodo, window, cx| this.finish_drag(window, cx)))
            .child(title_bar)
            .child(body)
            .child(fab)
            .children(overlay)
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

/// 打开主窗口：按当前屏幕组合恢复位置与尺寸（物理像素）与固定模式，没有记录则主显示器居中
/// 380×600（旧 `appStore.initSettings`）
pub fn open(cx: &mut App) -> Option<AnyWindowHandle> {
    let geometry = window_mode::read_startup_geometry(cx);
    let monitors = crate::platform::monitors(cx);
    let primary = crate::platform::primary_monitor(cx);
    let size = logical_size(DEFAULT_WIDTH, DEFAULT_HEIGHT);
    let restored = geometry
        .saved
        .and_then(|rect| window_mode::logical_bounds_for(&monitors, rect));
    let mut spec = match restored {
        Some((bounds, display)) => {
            let mut spec = WindowSpec::new("Mini Todo", bounds);
            spec.display_id = Some(display);
            spec
        }
        None => {
            let mut spec = WindowSpec::new(
                "Mini Todo",
                super::centered_bounds(primary.as_ref(), None, size),
            );
            spec.display_id = primary.as_ref().map(|m| m.display_id);
            spec
        }
    };
    spec.transparent = true;
    spec.min_size = Some(logical_size(MIN_WIDTH, MIN_HEIGHT));

    // 状态先就位（窗口第一帧就按固定模式外观画）
    let runtime_prefs = {
        let db = crate::db::Db::get(cx);
        mini_todo_core::settings::runtime_prefs(&db)
    };
    let state = AppState::global(cx);
    state.update(cx, |state, cx| {
        state.current_screen_config_id = geometry.screen_config_id.clone();
        state.is_fixed = geometry.is_fixed;
        state.fixed_embed_desktop = geometry.embed_desktop;
        state.runtime_prefs = runtime_prefs;
        state.auto_hide_enabled = runtime_prefs.auto_hide_enabled;
        state.top_on_wake = runtime_prefs.top_on_wake;
        cx.notify();
    });

    let handle: AnyWindowHandle = match super::open_window(cx, &spec, |window, cx| {
        cx.new(|cx| MainWindow::new(window, cx))
    }) {
        Ok(handle) => handle.into(),
        Err(e) => {
            log::error!("[window] 打开主窗口失败: {e}");
            return None;
        }
    };
    WindowManager::register(cx, WindowKind::Main, None, handle);
    WindowManager::on_closed(cx, handle, |cx| {
        window_mode::on_main_closed(cx);
        // 与旧版一致：所有窗口都关了就退出（收纳箱等窗口还在时进程继续，托盘可再唤起主窗口）
        if cfg!(windows) && cx.windows().is_empty() {
            cx.quit();
        }
    });
    let hwnd = cx
        .update_window(handle, |_, window, _| {
            crate::platform::set_round_corners(window, true);
            crate::platform::hwnd(window)
        })
        .ok()
        .flatten();
    window_mode::on_main_opened(cx, hwnd);

    // 记录与实际位置有偏差（中心不在任何显示器上 / 换算取整）时按物理像素校正
    if let (Some(saved), Some(hwnd)) = (geometry.saved, hwnd) {
        cx.spawn(async move |_| {
            if hwnd.client_rect() != Some(saved) {
                hwnd.set_client_rect(saved);
            }
        })
        .detach();
    }
    if geometry.is_fixed {
        let target = window_mode::TargetMode::for_state(true, geometry.embed_desktop);
        window_mode::apply_mode(cx, target).detach();
    }
    if geometry.saved.is_none() {
        // 没有记录：建初始记录
        window_mode::save_window_state(cx);
    }
    crate::services::update_check::check_for_updates(cx);
    Some(handle)
}

/// 显示并激活主窗口（托盘左键、单实例唤起）；主窗口不存在时重新打开
pub fn show_and_activate(cx: &mut App) {
    window_mode::bring_main_to_front(cx);
}
