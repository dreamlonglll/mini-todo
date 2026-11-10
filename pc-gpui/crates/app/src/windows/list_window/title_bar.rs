//! 清单窗口标题栏（旧 `TitleBar.vue`，规格 §4）：padding 12 16、高 52；左区清单名 + 版本号（更新红点；只在默认清单窗口）+
//! 展开全部；右区 已完成（只看本清单）/ 视图 / 日夜 / 锁 / 同步 / 设置（PRD R6）。日历窗口的标题栏复用这里的按钮
//! （[`title_button`]、[`theme_button`]、[`lock_button`]、[`settings_button`]）。
//!
//! 移动是自绘的（`windows::block::BlockDrag::title_bar`：按下记光标、超过 3px 才跟手、吸附对齐、松手保存一次），不进系统拖动循环
//! （没有 `drag_region()`，也就没有 Aero Snap 与双击最大化）。固定模式（含嵌入桌面）不挂拖动。按钮、版本号等可点击元素
//! 都 `.occlude()`：挡住标题栏，按它们不会开始拖动。

use std::time::Duration;

use gpui_kit::{
    Animation, AnimationExt as _, App, ClickEvent, Context, InteractiveElement as _, IntoElement,
    ParentElement as _, StatefulInteractiveElement as _, Styled as _, Window, div,
    prelude::FluentBuilder as _, px,
};
use mini_todo_core::sync::SyncMode;

use super::{ListWindow, RenderCtx};
use crate::assets::{AppIcon, EpIcon};
use crate::services::sync as sync_service;
use crate::services::window_mode;
use crate::state::{APP_VERSION, AppState, ViewMode};
use crate::theme::{self, Palette, main_window as dims};
use crate::ui::icon::{IconSource, icon, spinning};
use crate::ui::message::{self, MessageKind};
use crate::ui::message_box::{self, BoxKind, ConfirmOptions};
use crate::ui::tooltip;
use crate::windows::block::{BlockKey, DEFAULT_LIST_ID};
use crate::windows::{ChildOf, completed, settings};

/// 更新红点的光环动画周期
const PULSE: Duration = Duration::from_secs(2);

/// 标题栏右区 / 展开按钮的公共样式：28×28、圆角 6、图标 16；hover 底 `--bg-tertiary`、字 `--text-primary`；
/// active 时颜色 `--primary`（active 与 hover 同时成立时颜色取 active）
pub(crate) fn title_button(
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

/// 日 / 夜按钮（按窗口）：深色时太阳、浅色时月亮；失败在本窗口提示
pub(crate) fn theme_button(
    block: BlockKey,
    is_dark: bool,
    palette: &Palette,
) -> gpui_kit::Stateful<gpui_kit::Div> {
    title_button(
        "title-theme",
        if is_dark { EpIcon::Sunny } else { EpIcon::Moon },
        if is_dark {
            "切换到浅色主题"
        } else {
            "切换到深色主题"
        },
        is_dark,
        palette,
        move |_, window, cx| {
            let task = AppState::global(cx)
                .update(cx, |state, cx| state.set_window_dark(block, !is_dark, cx));
            let handle = window.window_handle();
            cx.spawn(async move |cx| {
                if let Err(text) = task.await {
                    cx.update(|cx| message::show_in(handle, cx, MessageKind::Error, text));
                }
            })
            .detach();
        },
    )
}

/// 锁按钮（按窗口：经典固定 / 嵌入桌面都显示为锁定）
pub(crate) fn lock_button(
    block: BlockKey,
    is_fixed: bool,
    palette: &Palette,
) -> gpui_kit::Stateful<gpui_kit::Div> {
    title_button(
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
        palette,
        move |_, _, cx| window_mode::toggle_fixed(cx, block),
    )
}

/// 设置按钮：以本窗口为打开方（模态、遮罩；「深色主题」切的是本窗口）
pub(crate) fn settings_button(palette: &Palette) -> gpui_kit::Stateful<gpui_kit::Div> {
    title_button(
        "title-settings",
        EpIcon::Setting,
        "设置",
        false,
        palette,
        |_, window, cx| {
            let parent = ChildOf::window(window, cx);
            settings::open(cx, Some(parent), settings::SettingsTab::General);
        },
    )
}

impl ListWindow {
    pub(super) fn render_title_bar(
        &mut self,
        rc: &RenderCtx,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let can_move = !window_mode::is_fixed(cx, self.block);
        let palette = rc.palette;

        let bar = div()
            .id("title-bar")
            .relative()
            .flex()
            .flex_none()
            .items_center()
            .justify_between()
            .px(px(16.))
            .py(px(12.));
        self.window_drag
            .title_bar(bar, can_move, cx)
            .child(self.render_title_left(&palette, cx))
            .child(self.render_title_right(&palette, cx))
    }

    fn render_title_left(&mut self, palette: &Palette, cx: &mut Context<Self>) -> impl IntoElement {
        let block = self.block;
        let list_id = self.list_id();
        let state = self.state.read(cx);
        let expand_all = state.expand_all(block);
        let has_update = state.has_update;
        let name = state.list_name(list_id);
        // 版本号与更新红点只在默认清单窗口显示（PRD R6）；改名时让出位置给输入框
        let show_version = list_id == DEFAULT_LIST_ID && !self.name.is_renaming();
        let version_color = palette.text_tertiary;
        let version_hover = palette.text_secondary;

        // 标题栏左区放不下时（macOS 的系统字体比 Segoe UI 宽）先收缩版本号：容器可以缩到 0，版本文字按剩余宽度截断加省略号，
        // 更新红点不截。放得下时排版与以前一样（截断只在文字超出时才发生）
        let version = show_version.then(|| {
            div()
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
                        .on_click(
                            cx.listener(|this, _, window, cx| this.confirm_update(window, cx)),
                        )
                })
                .child(
                    div()
                        .min_w_0()
                        .text_ellipsis()
                        .child(format!("v{APP_VERSION}")),
                )
                .when(has_update, |this| this.child(update_dot()))
        });

        div()
            .flex()
            .items_center()
            .gap(px(12.))
            .min_w_0()
            .child(
                div()
                    .flex()
                    .items_start()
                    .min_w_0()
                    .child(self.render_list_name(name, palette, cx))
                    .children(version),
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
                    move |_, _, cx| {
                        AppState::global(cx)
                            .update(cx, |state, cx| state.toggle_window_expand_all(block, cx))
                    },
                )
                .ml_auto(),
            )
    }

    fn render_title_right(
        &mut self,
        palette: &Palette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let block = self.block;
        let list_id = self.list_id();
        let is_fixed = window_mode::is_fixed(cx, block);
        let state = self.state.read(cx);
        let completed = state.todo_count_in(list_id).completed;
        let is_dark = state.is_dark(block);
        let view_mode = state.view_mode(block);
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
                    move |_, window, cx| {
                        let parent = ChildOf::window(window, cx);
                        completed::open(cx, Some(parent), list_id);
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
                    move |_, _, cx| {
                        AppState::global(cx).update(cx, |state, cx| {
                            state.set_window_view_mode(block, ViewMode::Quadrant, cx)
                        })
                    },
                ),
                ViewMode::Quadrant => title_button(
                    "title-view",
                    EpIcon::List,
                    "切换到列表视图",
                    true,
                    &palette,
                    move |_, _, cx| {
                        AppState::global(cx).update(cx, |state, cx| {
                            state.set_window_view_mode(block, ViewMode::List, cx)
                        })
                    },
                ),
            })
            .child(theme_button(block, is_dark, &palette))
            .child(lock_button(block, is_fixed, &palette))
            .child(sync_button)
            .child(settings_button(&palette))
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
pub(crate) fn relative_line(font_size: f32) -> gpui_kit::Pixels {
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
