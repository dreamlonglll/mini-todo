//! 「屏幕配置」面板（§2.10）：按 `updated_at DESC` 列出保存过的屏幕组合（每个组合一行）；当前组合高亮并带「当前」徽标、不可删除
//! （旧版缺陷 §2.13-1 按预期行为修好：设置窗口与清单窗口同进程，直接算当前屏幕组合 ID）。
//!
//! 一个屏幕组合下记着多个窗口（每个清单窗口、日历窗口、每个收纳箱各自的位置；清单 / 日历窗口还有锁定），所以每行第三行不再是
//! 以前单个主窗口的「固定模式 / 普通模式 | 位置」，而是这个组合保存了哪些窗口的位置（[`LayoutSummary`]，打开设置窗口时后台读）。
//! 删除连带删除该组合下全部清单 / 日历窗口与收纳箱的布局（core `settings::delete_screen_config`）。当前组合不能删：正在显示的
//! 窗口不会失去记录（窗口随后保存时会写回，删掉也没有意义）。

use std::collections::HashMap;

use gpui_kit::{
    Context, InteractiveElement as _, IntoElement, ParentElement as _, SharedString, Styled as _,
    Window, div, linear_color_stop, linear_gradient, prelude::FluentBuilder as _, px,
};
use mini_todo_core::Database;
use mini_todo_core::db::ScreenConfig;
use mini_todo_core::window_prefs::{self, WindowKey, WindowLayout};

use super::{SettingsWindow, card_hint, panel_title};
use crate::assets::EpIcon;
use crate::db::spawn_db;
use crate::theme::{self, settings_tokens};
use crate::ui::button::EpButton;
use crate::ui::icon::icon;
use crate::ui::message;
use crate::ui::message_box::{self, BoxKind, ConfirmOptions};

/// 旧 `formatConfigInfo`：`2_2560x1440@125_1920x1080@100` → `2 个显示器: 2560x1440 125%, 1920x1080 100%`
pub(crate) fn format_config_info(config_id: &str) -> String {
    match config_id {
        "legacy" => return "旧版本迁移的配置".into(),
        "unknown" => return "未知屏幕配置".into(),
        _ => {}
    }
    let parts: Vec<&str> = config_id.split('_').collect();
    if parts.len() < 2 {
        return config_id.to_string();
    }
    let monitors: Vec<String> = parts[1..]
        .iter()
        .map(|p| {
            let mut it = p.split('@');
            let res = it.next().unwrap_or_default();
            let scale = it.next().unwrap_or("undefined");
            format!("{res} {scale}%")
        })
        .collect();
    format!("{} 个显示器: {}", parts[0], monitors.join(", "))
}

/// 删除屏幕配置的确认文字（一个组合下记着多个窗口：清单、日历、收纳箱的位置一起删）
pub(crate) fn delete_message(name: &str) -> String {
    format!(
        "确定删除屏幕配置 \"{name}\" 吗？这个屏幕组合下保存的清单窗口、日历窗口与收纳箱的位置会一并删除。"
    )
}

/// 一个屏幕组合保存了哪些窗口的位置
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct LayoutSummary {
    /// 有位置记录的清单窗口数
    pub lists: usize,
    /// 日历窗口有没有位置记录
    pub calendar: bool,
    /// 其中锁定的清单 / 日历窗口数（收纳箱的锁定不按屏幕组合记）
    pub locked: usize,
    /// 有位置记录的收纳箱数
    pub fences: usize,
}

impl LayoutSummary {
    /// 从这个组合下的窗口布局（`window_layouts`）与收纳箱位置记录数算出；没有有效尺寸的记录（只记了锁定）不算
    pub(crate) fn new(windows: &[WindowLayout], fences: usize) -> Self {
        let mut summary = Self {
            fences,
            ..Self::default()
        };
        for layout in windows.iter().filter(|l| l.width > 0 && l.height > 0) {
            match layout.key {
                WindowKey::List(_) => summary.lists += 1,
                WindowKey::Calendar => summary.calendar = true,
            }
            if layout.is_fixed {
                summary.locked += 1;
            }
        }
        summary
    }

    /// 每行的第三行：`保存了 3 个清单窗口、日历窗口、2 个收纳箱的位置 | 1 个窗口锁定`
    pub(crate) fn text(&self) -> String {
        let mut parts = Vec::new();
        if self.lists > 0 {
            parts.push(format!("{} 个清单窗口", self.lists));
        }
        if self.calendar {
            parts.push("日历窗口".to_string());
        }
        if self.fences > 0 {
            parts.push(format!("{} 个收纳箱", self.fences));
        }
        if parts.is_empty() {
            return "没有保存窗口位置".into();
        }
        let mut text = format!("保存了 {}的位置", parts.join("、"));
        if self.locked > 0 {
            text.push_str(&format!(" | {} 个窗口锁定", self.locked));
        }
        text
    }
}

/// 读出每个保存过的屏幕组合的 [`LayoutSummary`]（阻塞，在后台线程调用）
pub(crate) fn load_layout_summaries(
    db: &Database,
) -> Result<HashMap<String, LayoutSummary>, String> {
    let mut out = HashMap::new();
    for config in mini_todo_core::settings::list_screen_configs(db)? {
        let windows = window_prefs::layouts_for_screen(db, &config.config_id)?;
        let fences =
            mini_todo_core::fence::Placement::for_screen(db, &config.config_id).layout_count();
        out.insert(config.config_id, LayoutSummary::new(&windows, fences));
    }
    Ok(out)
}

impl SettingsWindow {
    /// 后台读各屏幕组合保存了哪些窗口（打开设置窗口时）
    pub(super) fn load_screen_layouts(&mut self, cx: &mut Context<Self>) {
        let task = spawn_db(cx, load_layout_summaries);
        cx.spawn(async move |this, cx| match task.await {
            Ok(summaries) => {
                let _ = this.update(cx, |this, cx| {
                    this.screen_layouts = summaries;
                    cx.notify();
                });
            }
            Err(e) => log::warn!("[settings] 读取屏幕配置下的窗口布局失败: {e}"),
        })
        .detach();
    }

    fn delete_screen_config(
        &mut self,
        config: ScreenConfig,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if config.config_id == self.current_screen_id {
            message::warning(window, cx, "不能删除当前正在使用的屏幕配置");
            return;
        }
        let name = config
            .display_name
            .clone()
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| config.config_id.clone());
        let weak = cx.entity().downgrade();
        let config_id = config.config_id.clone();
        message_box::confirm(
            window,
            cx,
            ConfirmOptions::new("删除确认", delete_message(&name))
                .confirm_text("删除")
                .cancel_text("取消")
                .kind(BoxKind::Warning),
            move |confirmed, window, cx| {
                if !confirmed {
                    return;
                }
                let _ = weak.update(cx, |_, cx| {
                    let id = config_id.clone();
                    let task = spawn_db(cx, move |db| {
                        mini_todo_core::settings::delete_screen_config(db, &id)
                    });
                    cx.spawn_in(window, async move |this, cx| {
                        let result = task.await;
                        let _ = this.update_in(cx, |this, window, cx| match result {
                            Ok(()) => {
                                this.state.update(cx, |state, cx| {
                                    state.reload_screen_configs(cx).detach()
                                });
                                message::success(window, cx, "删除成功");
                            }
                            Err(e) => {
                                log::error!("[settings] 删除屏幕配置失败: {e}");
                                message::error(window, cx, "删除失败");
                            }
                        });
                    })
                    .detach();
                });
            },
        );
    }

    pub(super) fn render_screen(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let t = settings_tokens();
        let configs = self.state.read(cx).screen_configs.clone();
        let current = self.current_screen_id.clone();

        let body = if configs.is_empty() {
            div()
                .flex()
                .flex_col()
                .items_center()
                .p(px(24.))
                .text_color(t.tertiary)
                .child(
                    div()
                        .mb(px(8.))
                        .opacity(0.5)
                        .child(icon(EpIcon::Monitor, 28., t.tertiary)),
                )
                .child(div().text_size(px(13.)).child("暂无保存的屏幕配置"))
                .into_any_element()
        } else {
            div()
                .flex()
                .flex_col()
                .gap(px(8.))
                .children(configs.into_iter().enumerate().map(|(index, config)| {
                    let active = config.config_id == current;
                    let name = config
                        .display_name
                        .clone()
                        .filter(|n| !n.is_empty())
                        .unwrap_or_else(|| "未命名配置".into());
                    let detail = format_config_info(&config.config_id);
                    // 还没读出来时占住一行（不跳动）
                    let meta = self
                        .screen_layouts
                        .get(&config.config_id)
                        .map_or_else(|| "\u{a0}".to_string(), LayoutSummary::text);
                    let to_delete = config.clone();
                    div()
                        .id(SharedString::from(format!("screen-config-{index}")))
                        .flex()
                        .items_center()
                        .justify_between()
                        .px(px(14.))
                        .py(px(12.))
                        .rounded(theme::radius::MD)
                        .border_1()
                        .when(active, |this| this.border_color(t.primary).bg(t.active_bg))
                        .when(!active, |this| {
                            this.border_color(t.border)
                                .bg(t.surface)
                                .hover(move |s| s.bg(t.window_bg))
                        })
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .flex_1()
                                .min_w_0()
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap(px(8.))
                                        .text_size(px(13.))
                                        .font_weight(theme::WEIGHT_MEDIUM)
                                        .text_color(t.label)
                                        .child(name)
                                        .when(active, |this| {
                                            this.child(
                                                div()
                                                    .px(px(8.))
                                                    .py(px(2.))
                                                    .rounded(px(10.))
                                                    .bg(linear_gradient(
                                                        135.,
                                                        linear_color_stop(t.gradient_from, 0.),
                                                        linear_color_stop(t.gradient_to, 1.),
                                                    ))
                                                    .text_size(px(10.))
                                                    .line_height(px(15.))
                                                    .text_color(t.surface)
                                                    .child("当前"),
                                            )
                                        }),
                                )
                                .child(
                                    div()
                                        .mt(px(4.))
                                        .text_size(px(11.))
                                        .text_color(t.secondary)
                                        .truncate()
                                        .child(detail),
                                )
                                .child(
                                    div()
                                        .mt(px(2.))
                                        .text_size(px(11.))
                                        .text_color(t.tertiary)
                                        .child(meta),
                                ),
                        )
                        .child(
                            div().flex_none().ml(px(8.)).child(
                                EpButton::new(SharedString::from(format!("delete-config-{index}")))
                                    .danger()
                                    .text()
                                    .small()
                                    .icon(EpIcon::Delete)
                                    .disabled(active)
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.delete_screen_config(to_delete.clone(), window, cx)
                                    })),
                            ),
                        )
                }))
                .into_any_element()
        };

        div()
            .flex()
            .flex_col()
            .child(panel_title("屏幕配置"))
            .child(
                card_hint(
                    EpIcon::InfoFilled,
                    "应用会根据不同的屏幕组合自动保存和恢复窗口位置",
                    false,
                )
                .mb(px(12.)),
            )
            .child(body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_info_matches_frontend() {
        assert_eq!(
            format_config_info("2_2560x1440@125_1920x1080@100"),
            "2 个显示器: 2560x1440 125%, 1920x1080 100%"
        );
        assert_eq!(
            format_config_info("1_1920x1080@100"),
            "1 个显示器: 1920x1080 100%"
        );
        assert_eq!(format_config_info("legacy"), "旧版本迁移的配置");
        assert_eq!(format_config_info("unknown"), "未知屏幕配置");
        assert_eq!(format_config_info("odd"), "odd");
    }

    fn layout(key: WindowKey, width: i32, fixed: bool) -> WindowLayout {
        WindowLayout {
            key,
            screen_config_id: "1_1920x1080@100".into(),
            x: 0,
            y: 0,
            width,
            height: 600,
            is_fixed: fixed,
        }
    }

    #[test]
    fn summary_counts_every_window_of_the_combination() {
        let windows = [
            layout(WindowKey::List(1), 380, true),
            layout(WindowKey::List(1_762_000_000_000_001), 380, false),
            layout(WindowKey::List(1_762_000_000_000_002), 380, false),
            layout(WindowKey::Calendar, 596, true),
            // 只记了锁定、没有尺寸的记录不算位置
            layout(WindowKey::List(1_762_000_000_000_003), 0, true),
        ];
        let summary = LayoutSummary::new(&windows, 2);
        assert_eq!(
            summary,
            LayoutSummary {
                lists: 3,
                calendar: true,
                locked: 2,
                fences: 2
            }
        );
        assert_eq!(
            summary.text(),
            "保存了 3 个清单窗口、日历窗口、2 个收纳箱的位置 | 2 个窗口锁定"
        );
        // 升级后只有默认清单窗口（以前的主窗口）
        assert_eq!(
            LayoutSummary::new(&windows[..1], 0).text(),
            "保存了 1 个清单窗口的位置 | 1 个窗口锁定"
        );
        assert_eq!(LayoutSummary::new(&[], 1).text(), "保存了 1 个收纳箱的位置");
        assert_eq!(LayoutSummary::new(&[], 0).text(), "没有保存窗口位置");
    }

    #[test]
    fn summaries_follow_the_database_and_deletion() {
        use mini_todo_core::db::SaveScreenConfigRequest;
        let db = Database::new_in_memory().unwrap();
        let screen = "2_1920x1080@100_2560x1440@125";
        mini_todo_core::settings::save_screen_config(
            &db,
            &SaveScreenConfigRequest {
                config_id: screen.into(),
                display_name: None,
                window_x: 0,
                window_y: 0,
                window_width: 380,
                window_height: 600,
                is_fixed: false,
            },
        )
        .unwrap();
        let work = mini_todo_core::lists::create(&db, "工作").unwrap();
        let layouts = [
            layout(WindowKey::List(1), 380, false),
            layout(WindowKey::List(work.id), 380, true),
            layout(WindowKey::Calendar, 596, false),
        ]
        .map(|l| WindowLayout {
            screen_config_id: screen.into(),
            ..l
        });
        window_prefs::save_layouts(&db, &layouts).unwrap();
        let summaries = load_layout_summaries(&db).unwrap();
        assert_eq!(
            summaries.get(screen),
            Some(&LayoutSummary {
                lists: 2,
                calendar: true,
                locked: 1,
                fences: 0
            })
        );
        // 删除屏幕配置连带删除这些布局（确认框说的就是这个）
        mini_todo_core::settings::delete_screen_config(&db, screen).unwrap();
        assert!(load_layout_summaries(&db).unwrap().is_empty());
        assert!(
            window_prefs::layouts_for_screen(&db, screen)
                .unwrap()
                .is_empty()
        );
        assert!(delete_message("2屏").contains("清单窗口、日历窗口与收纳箱的位置会一并删除"));
    }
}
