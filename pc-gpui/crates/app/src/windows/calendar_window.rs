//! 日历窗口（旧 `CalendarView.vue`，以前是主窗口开着日历时的右栏；规格 `research/ui-main-window.md` §9）：单例
//! （[`BlockKey::Calendar`]），独立的顶层窗口，与清单窗口一样自绘移动 / 缩放、按窗口锁定与日 / 夜、吸附到同一个吸附组。
//!
//! | 子模块 | 内容 |
//! |---|---|
//! | [`title_bar`] | 标题栏：月份导航、今天、清单筛选、日 / 夜、锁、设置 |
//! | [`filter`] | 清单筛选（下拉多选：全部 / 一组清单，跟随本窗口的日 / 夜） |
//! | [`grid`] | 6 周网格、农历 / 节气 / 节日、节假日、跨天条，节假日后台加载 |
//! | [`logic`] | 纯逻辑（日历网格、跨天条、第一次显示的位置，带单测） |
//!
//! - 显示**所有清单**的待办（含已完成），按日历的清单筛选（[`filter`]：`window_prefs.calendar_filter`，
//!   `AppState::calendar_todos`）；点击待办条打开只读详情；双击某天（待办条以外的地方，含灰色的上 / 下月格子；锁定与否都行）
//!   新建待办：开始时间预填那天 00:00，清单按筛选（恰好只选了一个清单时建到它，否则默认清单，[`logic::new_todo_list_id`]）
//! - 是否显示 = 日历窗口的属性 `!hidden`（设置 → 常规「显示日历」、托盘「显示日历」勾选）；关闭 = 隐藏 = 关掉「显示日历」。
//!   生命周期在 `services::block_windows`

pub mod filter;
pub mod grid;
pub mod logic;
pub mod title_bar;

use chrono::NaiveDate;
use gpui_kit::{
    AnyElement, Context, Entity, InteractiveElement as _, IntoElement, MouseDownEvent,
    ParentElement as _, Render, Styled as _, Subscription, Window, div,
    prelude::FluentBuilder as _, px, relative,
};

use super::block::{BlockDrag, BlockDragHost, BlockKey};
use super::drag;
use super::snap::SnapKey;
use super::{ChildOf, WindowManager, modal_overlay};
use crate::services::{block_windows, window_mode};
use crate::state::AppState;
use crate::theme::{self, MainColors, Palette};
use crate::ui::ep_popper::Anchor;
use crate::windows::editor;

/// 渲染一帧用到的颜色（按本窗口的日 / 夜）
#[derive(Clone, Copy)]
pub(crate) struct CalendarCtx {
    pub palette: Palette,
    pub colors: MainColors,
}

pub struct CalendarWindow {
    pub(crate) block: BlockKey,
    /// 标题栏自绘移动与 8 个缩放手柄
    window_drag: BlockDrag,
    pub(crate) state: Entity<AppState>,
    pub(crate) calendar: grid::CalendarState,
    /// 清单筛选的下拉开着
    filter_open: bool,
    /// 筛选按钮的位置（下拉挂在它下面）
    filter_anchor: Anchor,
    _subscriptions: Vec<Subscription>,
}

impl CalendarWindow {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let block = BlockKey::Calendar;
        let state = AppState::global(cx);
        let subscriptions = vec![
            cx.observe(&state, |_, _, cx| cx.notify()),
            cx.observe_global::<WindowManager>(|_, cx| cx.notify()),
            cx.observe_window_activation(window, |this, window, cx| {
                if !window.is_window_active() {
                    // 切到别的窗口：收起筛选下拉
                    this.close_filter_menu(cx);
                    return;
                }
                // macOS：贴边隐藏中被 Dock / ⌘Tab 激活时拉回（Windows 上是空操作）
                let block = this.block;
                cx.defer(move |cx| window_mode::on_activated(cx, block));
                this.calendar.refresh_today(cx);
                if !WindowManager::forward_activation(window, cx) {
                    AppState::global(cx).update(cx, |state, cx| state.reload_todos(cx));
                }
            }),
            // 系统 / 程序挪动、改尺寸后防抖保存；用户自绘拖动 / 缩放中不算，松手后由 BlockDrag 直接保存一次
            cx.observe_window_bounds(window, |this, _, cx| {
                if !this.window_drag.is_busy() {
                    window_mode::schedule_save(cx, this.block)
                }
            }),
        ];
        // 关闭（Alt+F4 / ⌘W）= 隐藏 = 关掉「显示日历」（Linux 上真的关窗）
        window.on_window_should_close(cx, move |window, cx| {
            block_windows::request_close(window, cx, block)
        });
        let mut calendar = grid::CalendarState::new();
        calendar.start(cx);
        Self {
            block,
            window_drag: BlockDrag::new(
                block,
                f64::from(logic::MIN_WIDTH),
                f64::from(logic::MIN_HEIGHT),
            ),
            state,
            calendar,
            filter_open: false,
            filter_anchor: Anchor::new(),
            _subscriptions: subscriptions,
        }
    }

    /// 点击待办条：打开只读详情（以日历窗口为打开方，模态）
    pub(crate) fn open_todo(&mut self, id: i64, window: &mut Window, cx: &mut Context<Self>) {
        if cx.has_active_drag() {
            return;
        }
        let parent = ChildOf::window(window, cx);
        editor::open(cx, Some(parent), Some(id), editor::EditorMode::View);
    }

    /// 双击日格（按下时 `click_count >= 2`；待办条上的双击被条拦下）：新建待办，开始时间 = 那天 00:00，
    /// 清单按筛选（[`logic::new_todo_list_id`]：恰好只选了一个清单时建到它，否则默认清单）。以日历窗口为打开方（模态；
    /// 已有模态子窗口时 `open_child` 只把它带到前台）
    pub(crate) fn on_day_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        day: NaiveDate,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.click_count < 2 || cx.has_active_drag() {
            return;
        }
        // 处理了：下面的缩放手柄（边上的日格）不再开始缩放
        cx.stop_propagation();
        let list_id = {
            let state = self.state.read(cx);
            let filter = state.calendar_filter();
            logic::new_todo_list_id(filter.as_deref(), |id| state.list(id).is_some())
        };
        let preset = editor::NewTodoPreset::in_list(list_id)
            .with_start_time(logic::new_todo_start_time(day));
        let parent = ChildOf::window(window, cx);
        editor::open_new(cx, Some(parent), preset);
    }
}

impl BlockDragHost for CalendarWindow {
    fn block_drag(&mut self) -> &mut BlockDrag {
        &mut self.window_drag
    }
}

impl Render for CalendarWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let fixed = window_mode::is_fixed(cx, self.block);
        let classic_fixed = window_mode::is_classic_fixed(cx, self.block);
        let snap_edges = crate::services::snap::edges(cx, SnapKey::Block(self.block));
        let resize_handles = self.window_drag.resize_handles(!fixed, cx);
        let palette = self.state.read(cx).palette_for(self.block);
        let rc = CalendarCtx {
            palette,
            colors: MainColors::new(&palette),
        };

        let title_bar = self.render_title_bar(&rc, window, cx).into_any_element();
        let body = div()
            .flex_1()
            .min_h_0()
            .overflow_hidden()
            .px(px(if palette.is_dark { 8. } else { 12. }))
            .pb(px(if palette.is_dark { 8. } else { 12. }))
            .child(self.render_calendar(&rc, window, cx))
            .into_any_element();
        let border = if classic_fixed { 0. } else { 1. };
        // 清单筛选的下拉（画在本帧最后的浮层；跟随本窗口的日 / 夜）
        let filter_menu = self.render_filter_menu(palette.is_dark, window, cx);
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
            .id("calendar-root")
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
            });
        self.window_drag
            .root(root, cx)
            .child(title_bar)
            // 缩放手柄在标题栏之上、内容之下（同清单窗口，见 `windows::block` 模块文档）
            .children(resize_handles)
            .child(body)
            .children(filter_menu)
            .children(overlay)
            .when(!snap_edges.is_empty(), |this| {
                let radius = if classic_fixed {
                    px(0.)
                } else {
                    theme::radius::MD
                };
                this.child(drag::render_snap_guides(snap_edges, radius))
            })
    }
}
