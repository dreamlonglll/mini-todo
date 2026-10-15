//! 「屏幕配置」面板（§2.10）：按 `updated_at DESC` 列出保存过的屏幕组合；当前组合高亮并带「当前」徽标、不可删除
//! （旧版缺陷 §2.13-1 按预期行为修好：设置窗口与主窗口同进程，直接算当前屏幕组合 ID）；删除连带删除该组合下的收纳箱布局。

use gpui_kit::{
    Context, InteractiveElement as _, IntoElement, ParentElement as _, SharedString, Styled as _,
    Window, div, linear_color_stop, linear_gradient, prelude::FluentBuilder as _, px,
};
use mini_todo_core::db::ScreenConfig;

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

/// `{固定模式|普通模式} | 位置: (x, y)`
pub(crate) fn config_meta(config: &ScreenConfig) -> String {
    format!(
        "{} | 位置: ({}, {})",
        if config.is_fixed {
            "固定模式"
        } else {
            "普通模式"
        },
        config.window_x,
        config.window_y
    )
}

impl SettingsWindow {
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
            ConfirmOptions::new("删除确认", format!("确定删除屏幕配置 \"{name}\" 吗？"))
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
                    let meta = config_meta(&config);
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
}
