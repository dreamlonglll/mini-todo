//! 主窗口（旧 `MainView.vue` + `TitleBar.vue` + `TodoList.vue` …）。**W1 实现完整版**。
//!
//! 现在是证明整条链路的**简版**：透明窗口 + 自绘圆角底与描边、标题栏（拖动、主题 / 视图 / 设置 / 已完成 / 同步
//! 按钮）、待办标题列表（颜色点 + 标题 + 子任务计数 / 重复 / 提醒时间）、空状态、FAB、模态遮罩、5s 变更轮询。
//! 数值照 `research/ui-main-window.md`，但没有 hover 操作条、拖拽排序、四象限、日历、子任务展开、窗口模式等。

use std::time::Duration;

use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, ClickEvent, Context, Entity, Hsla,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, Window, div,
    prelude::FluentBuilder as _, px, relative,
};
use mini_todo_core::db::time::{DefaultTime, parse_local_datetime_with};
use mini_todo_core::sync::SyncMode;

use super::{ChildOf, WindowKind, WindowManager, WindowSpec, logical_size, modal_overlay};
use crate::assets::{AppIcon, EpIcon};
use crate::state::{APP_VERSION, AppState, AppStateEvent, ViewMode};
use crate::theme::{self, Palette};
use crate::ui::icon::{IconSource, icon, spinning};
use crate::ui::title_bar::drag_region;
use crate::ui::{message, tooltip};
use crate::windows::{completed, editor, settings};

pub const DEFAULT_WIDTH: f32 = 380.;
pub const DEFAULT_HEIGHT: f32 = 600.;
pub const MIN_WIDTH: f32 = 320.;
pub const MIN_HEIGHT: f32 = 400.;
/// 变更计数轮询间隔
const POLL_INTERVAL: Duration = Duration::from_secs(5);

/// 一行待办需要的数据（render 时从 AppState 拷出）
struct Row {
    id: i64,
    title: SharedString,
    color: Hsla,
    notify: Option<SharedString>,
    subtasks: Option<(usize, usize)>,
    repeat: bool,
}

pub struct MainWindow {
    state: Entity<AppState>,
    _subscriptions: Vec<Subscription>,
}

impl MainWindow {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let state = AppState::global(cx);
        let subscriptions =
            vec![
                cx.observe(&state, |_, _, cx| cx.notify()),
                cx.subscribe_in(&state, window, |_, _, event: &AppStateEvent, window, cx| {
                    match event {
                        AppStateEvent::Message { kind, text } => {
                            message::show(window, cx, *kind, text.clone())
                        }
                    }
                }),
                cx.observe_global::<WindowManager>(|_, cx| cx.notify()),
                cx.observe_window_activation(window, |_, window, cx| {
                    if !WindowManager::forward_activation(window, cx) && window.is_window_active() {
                        AppState::global(cx).update(cx, |state, cx| state.reload_todos(cx));
                    }
                }),
            ];
        // 5s 变更轮询（模态子窗口打开期间跳过）
        cx.spawn(async move |_, cx| {
            loop {
                cx.background_executor().timer(POLL_INTERVAL).await;
                let alive = cx.update(|cx| {
                    if WindowManager::main(cx).is_none() {
                        return false;
                    }
                    if !WindowManager::main_has_modal_child(cx) {
                        AppState::global(cx).update(cx, |state, cx| state.refresh_if_changed(cx));
                    }
                    true
                });
                if !alive {
                    break;
                }
            }
        })
        .detach();
        Self {
            state,
            _subscriptions: subscriptions,
        }
    }

    fn rows(&self, cx: &App) -> Vec<Row> {
        let state = self.state.read(cx);
        let fallback = theme::light().text_tertiary;
        state
            .pending_todos()
            .into_iter()
            .map(|todo| Row {
                id: todo.id,
                title: todo.title.clone().into(),
                color: theme::todo_color(&todo.color, fallback),
                notify: todo
                    .notify_at
                    .as_deref()
                    .and_then(|s| parse_local_datetime_with(s, DefaultTime::Notify))
                    .map(|dt| dt.format("%m-%d %H:%M").to_string().into()),
                subtasks: (!todo.subtasks.is_empty()).then(|| {
                    (
                        todo.subtasks.iter().filter(|s| s.completed).count(),
                        todo.subtasks.len(),
                    )
                }),
                repeat: todo.repeat_enabled,
            })
            .collect()
    }

    fn title_button(
        id: &'static str,
        source: impl Into<IconSource>,
        tip: &'static str,
        active: bool,
        palette: &Palette,
        on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> impl IntoElement {
        let color = if active {
            palette.primary
        } else {
            palette.text_secondary
        };
        let hover_bg = palette.bg_tertiary;
        let hover_fg = if active {
            palette.primary
        } else {
            palette.text_primary
        };
        div()
            .id(id)
            .occlude()
            .size(px(28.))
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .rounded(theme::radius::BASE)
            .text_color(color)
            .cursor_pointer()
            .hover(move |s| s.bg(hover_bg).text_color(hover_fg))
            .tooltip(tooltip::native(tip))
            .on_click(on_click)
            .child(icon(source, 16., color))
    }

    fn render_title_bar(&self, palette: &Palette, cx: &mut Context<Self>) -> impl IntoElement {
        let state = self.state.read(cx);
        let completed = state.todo_count().completed;
        let is_dark = state.is_dark_theme;
        let is_fixed = state.is_fixed;
        let view_mode = state.view_mode;
        let syncing = state.syncing;
        let expand_all = state.subtask_expand_all;
        let palette = *palette;

        let sync_icon = icon(EpIcon::Refresh, 16., palette.text_secondary);
        let sync_button = div()
            .id("title-sync")
            .occlude()
            .size(px(28.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(theme::radius::BASE)
            .cursor_pointer()
            .hover(move |s| s.bg(palette.bg_tertiary))
            .tooltip(tooltip::native(if syncing {
                "同步中..."
            } else {
                "同步数据"
            }))
            .when(!syncing, |this| {
                this.on_click(|_, window, cx| {
                    let task = crate::services::sync::run_sync(cx, SyncMode::Smart { full: true });
                    let handle = window.window_handle();
                    cx.spawn(async move |cx| {
                        let result = task.await;
                        cx.update(|cx| match result {
                            Ok(report) => message::show_in(
                                handle,
                                cx,
                                crate::services::sync::report_message_kind(&report),
                                crate::services::sync::describe_sync_report(&report),
                            ),
                            Err(e) if crate::services::sync::is_sync_busy_error(&e) => {
                                message::show_in(
                                    handle,
                                    cx,
                                    message::MessageKind::Info,
                                    "已有同步正在进行，请稍后再试",
                                )
                            }
                            Err(e) => message::show_in(
                                handle,
                                cx,
                                message::MessageKind::Error,
                                message::error_text("同步失败", &e),
                            ),
                        });
                    })
                    .detach();
                })
            })
            .child(if syncing {
                spinning("title-sync-spin", sync_icon).into_any_element()
            } else {
                sync_icon.into_any_element()
            });

        div()
            .relative()
            .flex()
            .flex_none()
            .items_center()
            .justify_between()
            .px(px(16.))
            .py(px(12.))
            .when(!is_fixed, |this| this.child(drag_region()))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .child(
                        div()
                            .flex()
                            .items_baseline()
                            .child(
                                div()
                                    .text_size(px(16.))
                                    .font_weight(theme::WEIGHT_SEMIBOLD)
                                    .text_color(palette.text_primary)
                                    .child("待办清单"),
                            )
                            .child(
                                div()
                                    .ml(px(4.))
                                    .text_size(px(11.))
                                    .text_color(palette.text_tertiary)
                                    .whitespace_nowrap()
                                    .child(format!("v{APP_VERSION}")),
                            ),
                    )
                    .child(Self::title_button(
                        "title-expand",
                        if expand_all {
                            EpIcon::Fold
                        } else {
                            EpIcon::Expand
                        },
                        if expand_all {
                            "收起全部子任务"
                        } else {
                            "展开全部子任务"
                        },
                        expand_all,
                        &palette,
                        |_, _, cx| {
                            AppState::global(cx)
                                .update(cx, |state, cx| state.toggle_subtask_expand_all(cx))
                        },
                    )),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .occlude()
                    .when(completed > 0, |this| {
                        this.child(Self::title_button(
                            "title-completed",
                            EpIcon::Finished,
                            "已完成",
                            false,
                            &palette,
                            |_, window, cx| {
                                let parent = ChildOf::window(window, cx);
                                completed::open(cx, Some(parent));
                            },
                        ))
                    })
                    .child(match view_mode {
                        ViewMode::List => Self::title_button(
                            "title-view",
                            AppIcon::QuadGrid,
                            "切换到四象限视图",
                            false,
                            &palette,
                            |_, _, cx| {
                                AppState::global(cx).update(cx, |state, cx| {
                                    state.set_view_mode(ViewMode::Quadrant, cx)
                                })
                            },
                        )
                        .into_any_element(),
                        ViewMode::Quadrant => Self::title_button(
                            "title-view",
                            EpIcon::List,
                            "切换到列表视图",
                            true,
                            &palette,
                            |_, _, cx| {
                                AppState::global(cx)
                                    .update(cx, |state, cx| state.set_view_mode(ViewMode::List, cx))
                            },
                        )
                        .into_any_element(),
                    })
                    .child(Self::title_button(
                        "title-theme",
                        if is_dark { EpIcon::Sunny } else { EpIcon::Moon },
                        if is_dark {
                            "切换到浅色主题"
                        } else {
                            "切换到深色主题"
                        },
                        is_dark,
                        &palette,
                        move |_, window, cx| {
                            let task = AppState::global(cx)
                                .update(cx, |state, cx| state.set_dark_theme(!is_dark, cx));
                            let handle = window.window_handle();
                            cx.spawn(async move |cx| {
                                if let Err(text) = task.await {
                                    cx.update(|cx| {
                                        message::show_in(
                                            handle,
                                            cx,
                                            message::MessageKind::Error,
                                            text,
                                        )
                                    });
                                }
                            })
                            .detach();
                        },
                    ))
                    .child(Self::title_button(
                        "title-fixed",
                        if is_fixed {
                            EpIcon::Lock
                        } else {
                            EpIcon::Unlock
                        },
                        if is_fixed {
                            "取消固定"
                        } else {
                            "固定窗口"
                        },
                        is_fixed,
                        &palette,
                        |_, _, cx| {
                            AppState::global(cx).update(cx, |state, cx| state.toggle_fixed_mode(cx))
                        },
                    ))
                    .child(sync_button)
                    .child(Self::title_button(
                        "title-settings",
                        EpIcon::Setting,
                        "设置",
                        false,
                        &palette,
                        |_, window, cx| {
                            let parent = ChildOf::window(window, cx);
                            settings::open(cx, Some(parent), settings::SettingsTab::General);
                        },
                    )),
            )
    }

    fn render_row(row: Row, palette: &Palette, font_size: f32) -> impl IntoElement {
        let sub_size = font_size - 2.;
        let meta_color = palette.text_tertiary;
        let has_meta = row.subtasks.is_some() || row.repeat || row.notify.is_some();
        let hover_bg = if palette.is_dark {
            theme::hexa(0x00000000)
        } else {
            theme::rgba_css(0, 0, 0, 0.03)
        };
        let id = row.id;
        div()
            .id(("todo-row", row.id as u64))
            .flex()
            .items_start()
            .px(px(12.))
            .py(px(8.))
            .mb(px(4.))
            .rounded(theme::radius::BASE)
            .cursor_pointer()
            .hover(move |s| s.bg(hover_bg))
            .on_click(move |_, window, cx| {
                let parent = ChildOf::window(window, cx);
                editor::open(cx, Some(parent), Some(id), editor::EditorMode::View);
            })
            .child(
                div()
                    .size(px(8.))
                    .mt(px(6.))
                    .mr(px(12.))
                    .flex_none()
                    .rounded_full()
                    .bg(row.color),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .text_size(px(font_size))
                            .text_color(palette.text_primary)
                            .child(row.title),
                    )
                    .when(has_meta, |this| {
                        this.child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(8.))
                                .mt(px(4.))
                                .text_size(px(sub_size))
                                .text_color(meta_color)
                                .when_some(row.subtasks, |this, (done, total)| {
                                    this.child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap(px(4.))
                                            .child(icon(EpIcon::Finished, 12., meta_color))
                                            .child(format!("{done}/{total}"))
                                            .child(icon(EpIcon::ArrowRight, 10., meta_color)),
                                    )
                                })
                                .when(row.repeat, |this| {
                                    // 简版只显示通用文案；重复规则文案（repeatTooltip）由 W1 实现
                                    this.child(
                                        tooltip::EpTooltip::new(
                                            ("repeat-tip", id as u64),
                                            "重复提醒",
                                            icon(EpIcon::RefreshRight, 12., theme::ep().primary),
                                        )
                                        .show_after_ms(300),
                                    )
                                })
                                .when_some(row.notify, |this, notify| {
                                    this.child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap(px(4.))
                                            .child(icon(EpIcon::Bell, 12., meta_color))
                                            .child(notify),
                                    )
                                }),
                        )
                    }),
            )
    }

    fn render_empty(palette: &Palette) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .p(px(24.))
            .text_color(palette.text_tertiary)
            .when(palette.is_dark, |this| {
                this.bg(theme::rgba_css(0, 0, 0, 0.1))
            })
            .child(icon(EpIcon::Document, 48., palette.text_tertiary))
            .child(div().mt(px(8.)).text_size(px(14.)).child("暂无待办事项"))
            .child(
                div()
                    .mt(px(4.))
                    .text_size(px(12.))
                    .child("点击右下角悬浮按钮添加待办项"),
            )
    }
}

impl Render for MainWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = self.state.read(cx);
        let palette = state.main_palette();
        let classic_fixed = state.is_classic_fixed();
        let total = state.todo_count().total;
        let font_size = state.todo_font_size as f32;
        let todo_font = theme::todo_font(&state.todo_font_family);
        let rows = self.rows(cx);
        let empty = total == 0;

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
            .child(self.render_title_bar(&palette, cx))
            .child(
                div()
                    .id("main-content")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px(px(16.))
                    .pb(px(76.))
                    .mb(px(6.))
                    .font(todo_font)
                    .when(empty, |this| this.child(Self::render_empty(&palette)))
                    .children(
                        rows.into_iter()
                            .map(|row| Self::render_row(row, &palette, font_size)),
                    ),
            )
            .child(
                div()
                    .id("fab-add")
                    .absolute()
                    .right(px(20.))
                    .bottom(px(20.))
                    .size(px(48.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_full()
                    .bg(palette.primary)
                    .shadow(vec![theme::shadow(
                        0.,
                        4.,
                        12.,
                        0.,
                        theme::rgba_css(59, 130, 246, 0.4),
                    )])
                    .cursor_pointer()
                    .hover(move |s| s.bg(palette.primary_dark))
                    .tooltip(tooltip::native("新建待办"))
                    .on_click(|_, window, cx| {
                        let parent = ChildOf::window(window, cx);
                        editor::open(cx, Some(parent), None, editor::EditorMode::Edit);
                    })
                    .child(icon(EpIcon::Plus, 24., theme::hex(0xFFFFFF))),
            )
            .children(modal_overlay(window, cx, 0.3))
    }
}

/// 打开主窗口（380×600、最小 320×400、透明、无系统标题栏；先在主显示器居中，位置记忆由 W1 实现）
pub fn open(cx: &mut App) -> Option<AnyWindowHandle> {
    let monitor = crate::platform::primary_monitor(cx);
    let size = logical_size(DEFAULT_WIDTH, DEFAULT_HEIGHT);
    let mut spec = WindowSpec::new(
        "Mini Todo",
        super::centered_bounds(monitor.as_ref(), None, size),
    );
    spec.display_id = monitor.map(|m| m.display_id);
    spec.transparent = true;
    spec.min_size = Some(logical_size(MIN_WIDTH, MIN_HEIGHT));
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
    let _ = cx.update_window(handle, |_, window, _| {
        crate::platform::set_round_corners(window, true);
    });
    Some(handle)
}

/// 显示并激活主窗口（托盘左键、单实例唤起）；主窗口不存在时重新打开
pub fn show_and_activate(cx: &mut App) {
    match WindowManager::main(cx) {
        Some(handle) => {
            let _ = cx.update_window(handle, |_, window, cx| {
                crate::platform::bring_to_front(window, cx)
            });
        }
        None => {
            open(cx);
        }
    }
}
