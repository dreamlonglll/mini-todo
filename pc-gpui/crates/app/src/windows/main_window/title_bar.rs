//! 主窗口标题栏（旧 `TitleBar.vue`，规格 §4）：padding 12 16、高 52；左区"待办清单" + 版本号（更新红点）+
//! 展开全部；日历开启时中区月份导航与"今天"；右区 已完成 / 视图 / 主题 / 固定 / 同步 / 设置。
//! 固定模式（含嵌入桌面）不可拖动：不放拖动区。

use std::time::Duration;

use gpui_kit::{
    Animation, AnimationExt as _, App, ClickEvent, Context, InteractiveElement as _, IntoElement,
    ParentElement as _, StatefulInteractiveElement as _, Styled as _, Window, div,
    prelude::FluentBuilder as _, px,
};
use mini_todo_core::sync::SyncMode;

use super::{MainWindow, RenderCtx};
use crate::assets::{AppIcon, EpIcon};
use crate::services::sync as sync_service;
use crate::state::{APP_VERSION, AppState, ViewMode};
use crate::theme::{self, Palette, main_window as dims};
use crate::ui::icon::{IconSource, icon, spinning};
use crate::ui::message::{self, MessageKind};
use crate::ui::message_box::{self, BoxKind, ConfirmOptions};
use crate::ui::title_bar::drag_region;
use crate::ui::tooltip;
use crate::windows::{ChildOf, completed, settings};

/// 更新红点的光环动画周期
const PULSE: Duration = Duration::from_secs(2);

/// 标题栏右区 / 展开按钮的公共样式：28×28、圆角 6、图标 16；hover 底 `--bg-tertiary`、字 `--text-primary`；
/// active 时颜色 `--primary`（active 与 hover 同时成立时颜色取 active）
fn title_button(
    id: &'static str,
    source: impl Into<IconSource>,
    tip: &'static str,
    active: bool,
    palette: &Palette,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> gpui_kit::Stateful<gpui_kit::Div> {
    let color = if active {
        palette.primary
    } else {
        palette.text_secondary
    };
    let hover_fg = if active {
        palette.primary
    } else {
        palette.text_primary
    };
    let hover_bg = palette.bg_tertiary;
    div()
        .id(id)
        .group(id)
        .occlude()
        .size(px(dims::TITLE_BUTTON))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .rounded(theme::radius::BASE)
        .cursor_pointer()
        .hover(move |s| s.bg(hover_bg))
        .tooltip(tooltip::native(tip))
        .on_click(on_click)
        .child(
            icon(source, dims::TITLE_ICON, color).group_hover(id, move |s| s.text_color(hover_fg)),
        )
}

impl MainWindow {
    pub(super) fn render_title_bar(
        &mut self,
        rc: &RenderCtx,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let state = self.state.read(cx);
        let palette = rc.palette;
        let show_calendar = state.show_calendar;
        let draggable = !state.is_fixed;

        div()
            .id("title-bar")
            .relative()
            .flex()
            .flex_none()
            .items_center()
            .justify_between()
            .px(px(16.))
            .py(px(12.))
            .when(draggable, |this| this.child(drag_region()))
            .child(self.render_title_left(&palette, show_calendar, cx))
            .when(show_calendar, |this| {
                this.child(self.render_title_center(rc, cx))
            })
            .child(self.render_title_right(&palette, cx))
    }

    fn render_title_left(
        &mut self,
        palette: &Palette,
        show_calendar: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let state = self.state.read(cx);
        let expand_all = state.subtask_expand_all;
        let has_update = state.has_update;
        let version_color = palette.text_tertiary;
        let version_hover = palette.text_secondary;

        // 标题栏左区放不下时（macOS 的系统字体比 Segoe UI 宽）先收缩版本号：容器可以缩到 0，版本文字按剩余宽度截断加省略号，
        // 更新红点不截。放得下时排版与以前一样（截断只在文字超出时才发生）
        let version = div()
            .id("version-tag")
            .occlude()
            .flex()
            .min_w_0()
            .items_center()
            .ml(px(4.))
            // align-items: baseline 的近似：11px 版本号的基线对齐 16px 标题的首行基线
            .mt(px(6.))
            .text_size(px(11.))
            .line_height(relative_line(11.))
            .text_color(version_color)
            .whitespace_nowrap()
            .when(has_update, |this| {
                this.cursor_pointer()
                    .hover(move |s| s.text_color(version_hover))
                    .on_click(cx.listener(|this, _, window, cx| this.confirm_update(window, cx)))
            })
            .child(
                div()
                    .min_w_0()
                    .text_ellipsis()
                    .child(format!("v{APP_VERSION}")),
            )
            .when(has_update, |this| this.child(update_dot()));

        div()
            .flex()
            .items_center()
            .gap(px(12.))
            .when(show_calendar, |this| {
                this.w(gpui_kit::relative(0.4)).min_w(px(280.)).flex_none()
            })
            .when(!show_calendar, |this| this.min_w_0())
            .child(
                div()
                    .flex()
                    .items_start()
                    .min_w_0()
                    .child(
                        // 旧版 CSS 标题不换行：任何平台都不收缩、不折行（收缩的是后面的版本号）
                        div()
                            .flex_none()
                            .whitespace_nowrap()
                            .text_size(px(16.))
                            .line_height(relative_line(16.))
                            .font_weight(theme::WEIGHT_SEMIBOLD)
                            .text_color(palette.text_primary)
                            .child("待办清单"),
                    )
                    .child(version),
            )
            .child(
                title_button(
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
                    palette,
                    |_, _, cx| {
                        AppState::global(cx)
                            .update(cx, |state, cx| state.toggle_subtask_expand_all(cx))
                    },
                )
                .ml_auto(),
            )
    }

    fn render_title_center(&mut self, rc: &RenderCtx, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = rc.colors;
        let palette = rc.palette;
        let nav = |id: &'static str, source: EpIcon| {
            let (fg, hover_fg, hover_bg) =
                (colors.nav_fg, colors.nav_hover_fg, colors.nav_hover_bg);
            div()
                .id(id)
                .group(id)
                .occlude()
                .h(theme::EP_HEIGHT_SMALL)
                .px(px(11.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(theme::radius::BASE)
                .cursor_pointer()
                .hover(move |s| s.bg(hover_bg))
                .active(move |s| s.bg(hover_bg))
                .child(icon(source, 12., fg).group_hover(id, move |s| s.text_color(hover_fg)))
        };
        let month = self.calendar.month_text();
        div()
            .flex()
            .flex_1()
            .min_w_0()
            .items_center()
            .gap(px(12.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .child(
                        nav("calendar-prev", EpIcon::ArrowLeft).on_click(cx.listener(
                            |this, _, _, cx| {
                                this.calendar.prev_month(cx);
                                cx.notify();
                            },
                        )),
                    )
                    .child(
                        div()
                            .min_w(px(90.))
                            .flex()
                            .justify_center()
                            .text_size(px(15.))
                            .line_height(relative_line(15.))
                            .font_weight(theme::WEIGHT_SEMIBOLD)
                            .text_color(palette.text_primary)
                            .child(month),
                    )
                    .child(
                        nav("calendar-next", EpIcon::ArrowRight).on_click(cx.listener(
                            |this, _, _, cx| {
                                this.calendar.next_month(cx);
                                cx.notify();
                            },
                        )),
                    ),
            )
            .child(
                div()
                    .id("calendar-today")
                    .occlude()
                    .h(theme::EP_HEIGHT_SMALL)
                    .px(px(11.))
                    .flex()
                    .items_center()
                    .rounded(theme::radius::BASE)
                    .border_1()
                    .border_color(colors.today_border)
                    .bg(colors.today_bg)
                    .text_size(px(12.))
                    .text_color(colors.today_fg)
                    .cursor_pointer()
                    .hover(move |s| {
                        s.text_color(colors.today_hover_fg)
                            .bg(colors.today_hover_bg)
                            .border_color(colors.today_hover_border)
                    })
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.calendar.go_to_today(cx);
                        cx.notify();
                    }))
                    .child("今天"),
            )
    }

    fn render_title_right(
        &mut self,
        palette: &Palette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let state = self.state.read(cx);
        let completed = state.todo_count().completed;
        let is_dark = state.is_dark_theme;
        let is_fixed = state.is_fixed;
        let view_mode = state.view_mode;
        let syncing = state.syncing;
        let palette = *palette;

        let sync_icon = icon(EpIcon::Refresh, dims::TITLE_ICON, palette.text_secondary)
            .group_hover("title-sync", move |s| s.text_color(palette.text_primary));
        let hover_bg = palette.bg_tertiary;
        let sync_button = div()
            .id("title-sync")
            .group("title-sync")
            .occlude()
            .size(px(dims::TITLE_BUTTON))
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .rounded(theme::radius::BASE)
            .cursor_pointer()
            .hover(move |s| s.bg(hover_bg))
            .tooltip(tooltip::native(if syncing {
                "同步中..."
            } else {
                "同步数据"
            }))
            .when(!syncing, |this| {
                this.on_click(|_, window, cx| manual_sync(window, cx))
            })
            .child(if syncing {
                spinning("title-sync-spin", sync_icon).into_any_element()
            } else {
                sync_icon.into_any_element()
            });

        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(8.))
            .when(completed > 0, |this| {
                this.child(title_button(
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
                ViewMode::List => title_button(
                    "title-view",
                    AppIcon::QuadGrid,
                    "切换到四象限视图",
                    false,
                    &palette,
                    |_, _, cx| {
                        AppState::global(cx)
                            .update(cx, |state, cx| state.set_view_mode(ViewMode::Quadrant, cx))
                    },
                ),
                ViewMode::Quadrant => title_button(
                    "title-view",
                    EpIcon::List,
                    "切换到列表视图",
                    true,
                    &palette,
                    |_, _, cx| {
                        AppState::global(cx)
                            .update(cx, |state, cx| state.set_view_mode(ViewMode::List, cx))
                    },
                ),
            })
            .child(title_button(
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
                            cx.update(|cx| message::show_in(handle, cx, MessageKind::Error, text));
                        }
                    })
                    .detach();
                },
            ))
            .child(title_button(
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
                |_, _, cx| AppState::global(cx).update(cx, |state, cx| state.toggle_fixed_mode(cx)),
            ))
            .child(sync_button)
            .child(title_button(
                "title-settings",
                EpIcon::Setting,
                "设置",
                false,
                &palette,
                |_, window, cx| {
                    let parent = ChildOf::window(window, cx);
                    settings::open(cx, Some(parent), settings::SettingsTab::General);
                },
            ))
    }

    /// 点版本号且有更新：确认后打开 GitHub Release 页面（旧 `handleVersionClick`）
    fn confirm_update(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let state = self.state.read(cx);
        if !state.has_update {
            return;
        }
        let latest = state.latest_version.clone().unwrap_or_default();
        message_box::confirm(
            window,
            cx,
            ConfirmOptions::new("版本更新", format!("发现新版本 {latest}，是否前往下载？"))
                .confirm_text("前往下载")
                .cancel_text("稍后再说")
                .kind(BoxKind::Info),
            |confirmed, _, cx| {
                if confirmed {
                    let url = crate::services::update_check::releases_url(cx);
                    cx.open_url(&url);
                }
            },
        );
    }
}

/// CSS `line-height: 1.5` 的像素值（文字行盒高度）
fn relative_line(font_size: f32) -> gpui_kit::Pixels {
    px(font_size * theme::LINE_HEIGHT)
}

/// 更新红点：6×6 `#EF4444`，`pulse 2s infinite` 向外扩散 4px 并淡出的光环
fn update_dot() -> impl IntoElement {
    div()
        .ml(px(2.))
        .size(px(6.))
        .flex_none()
        .rounded_full()
        .bg(theme::hex(0xEF4444))
        .with_animation(
            "update-dot-pulse",
            Animation::new(PULSE).repeat(),
            |dot, delta| {
                // 0% → 70%：0 → 4px、α 0.7 → 0；70% → 100%：保持透明
                let t = (delta / 0.7).min(1.);
                dot.shadow(vec![theme::shadow(
                    0.,
                    0.,
                    0.,
                    4. * t,
                    theme::rgba_css(239, 68, 68, 0.7 * (1. - t)),
                )])
            },
        )
}

/// 手动同步（旧 `handleSync`）：未配置 → 提示配置；`full: true`；结果 / 跳过记录 / 忙碌 / 失败分别提示
pub(crate) fn manual_sync(window: &mut Window, cx: &mut App) {
    let state = AppState::global(cx);
    if state.read(cx).syncing {
        return;
    }
    let handle = window.window_handle();
    // 检查配置期间按钮也转圈（旧版 isSyncing 在读设置前就置位）
    state.update(cx, |state, cx| state.begin_sync(cx));
    let settings_task = crate::db::spawn_db(cx, mini_todo_core::sync::get_settings);
    cx.spawn(async move |cx| {
        let configured = settings_task
            .await
            .map(|s| !s.webdav_url.trim().is_empty())
            .unwrap_or(false);
        let task = cx.update(|cx| {
            AppState::global(cx).update(cx, |state, cx| state.end_sync(cx));
            if !configured {
                message::show_in(
                    handle,
                    cx,
                    MessageKind::Warning,
                    "请先在设置中配置 WebDAV 服务器",
                );
                return None;
            }
            Some(sync_service::run_sync(cx, SyncMode::Smart { full: true }))
        });
        let Some(task) = task else {
            return;
        };
        let result = task.await;
        cx.update(|cx| match result {
            Ok(report) => {
                AppState::global(cx).update(cx, |state, _| state.auto_sync_failing = false);
                message::show_in(
                    handle,
                    cx,
                    sync_service::report_message_kind(&report),
                    sync_service::describe_sync_report(&report),
                );
                if let Some(skipped) = sync_service::describe_skipped_records(&report) {
                    message::show_in(handle, cx, MessageKind::Warning, skipped);
                }
            }
            Err(e) if sync_service::is_sync_busy_error(&e) => message::show_in(
                handle,
                cx,
                MessageKind::Info,
                "已有同步正在进行，请稍后再试",
            ),
            Err(e) => message::show_in(
                handle,
                cx,
                MessageKind::Error,
                message::error_text("同步失败", &e),
            ),
        });
    })
    .detach();
}
