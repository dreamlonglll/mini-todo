//! 「关于」面板（§2.11）：Logo、名称、版本（打开即显示，旧版缺陷 §2.13-2 按预期行为修好）、新版本徽标、检查更新。

use gpui_kit::{
    Context, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, Window, div, linear_color_stop, linear_gradient,
    prelude::FluentBuilder as _, px,
};

use super::SettingsWindow;
use super::update as update_check;
use crate::assets::EpIcon;
use crate::state::APP_VERSION;
use crate::theme::{self, settings_tokens};
use crate::ui::icon::{icon, spinning};
use crate::ui::message;
use crate::ui::message_box::{self, BoxKind, ConfirmOptions};

impl SettingsWindow {
    fn check_update(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.checking_update {
            return;
        }
        self.checking_update = true;
        cx.notify();
        let task = update_check::check_for_updates(cx);
        cx.spawn_in(window, async move |this, cx| {
            let has_update = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.checking_update = false;
                cx.notify();
                if !has_update {
                    message::success(window, cx, "当前已是最新版本");
                    return;
                }
                let (latest, url) = {
                    let state = this.state.read(cx);
                    (
                        state.latest_version.clone().unwrap_or_default(),
                        update_check::releases_url(state),
                    )
                };
                message_box::confirm(
                    window,
                    cx,
                    ConfirmOptions::new("版本更新", format!("发现新版本 {latest}，是否前往下载？"))
                        .confirm_text("前往下载")
                        .cancel_text("稍后再说")
                        .kind(BoxKind::Info),
                    move |confirmed, _, cx| {
                        if confirmed {
                            cx.open_url(&url);
                        }
                    },
                );
            });
        })
        .detach();
    }

    pub(super) fn render_about(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let t = settings_tokens();
        let state = self.state.read(cx);
        let update = state
            .has_update
            .then(|| state.latest_version.clone().unwrap_or_default());
        let checking = self.checking_update;

        let logo = div()
            .flex_none()
            .size(px(60.))
            .rounded(px(14.))
            .flex()
            .items_center()
            .justify_center()
            .bg(linear_gradient(
                135.,
                linear_color_stop(t.gradient_from, 0.),
                linear_color_stop(t.gradient_to, 1.),
            ))
            .child(icon(EpIcon::Promotion, 36., t.surface));

        let info = div()
            .flex()
            .flex_col()
            .child(
                div()
                    .mb(px(4.))
                    .text_size(px(18.))
                    .font_weight(theme::WEIGHT_SEMIBOLD)
                    .text_color(t.title)
                    .child("Mini Todo"),
            )
            .child(
                div()
                    .mb(px(4.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .text_size(px(13.))
                    .text_color(t.secondary)
                    .child(format!("版本 {APP_VERSION}"))
                    .when_some(update, |this, latest| {
                        this.child(
                            div()
                                .px(px(8.))
                                .py(px(2.))
                                .rounded(px(10.))
                                .bg(t.update_badge_bg)
                                .text_size(px(11.))
                                .line_height(px(16.5))
                                .font_weight(theme::WEIGHT_MEDIUM)
                                .text_color(t.update_badge_text)
                                .child(format!("新版本 {latest}")),
                        )
                    }),
            )
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(t.tertiary)
                    .child("一个简洁高效的桌面待办应用"),
            );

        let glyph = if checking {
            spinning("check-update-spin", icon(EpIcon::Loading, 16., t.label)).into_any_element()
        } else {
            icon(EpIcon::Refresh, 16., t.label).into_any_element()
        };
        let button = div()
            .id("check-update")
            .w_full()
            .flex()
            .items_center()
            .justify_center()
            .gap(px(8.))
            .px(px(16.))
            .py(px(12.))
            .border_1()
            .border_color(t.border)
            .rounded(theme::radius::MD)
            .bg(t.surface)
            .text_size(px(14.))
            .font_weight(theme::WEIGHT_MEDIUM)
            .text_color(t.label)
            .when(checking, |this| {
                this.opacity(0.6)
                    .cursor(gpui_kit::CursorStyle::OperationNotAllowed)
            })
            .when(!checking, |this| {
                this.cursor_pointer()
                    .hover(move |s| s.bg(t.window_bg).border_color(t.hover_border))
                    .on_click(cx.listener(|this, _, window, cx| this.check_update(window, cx)))
            })
            .child(glyph)
            .child(if checking {
                "检查中..."
            } else {
                "检查更新"
            });

        div()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(16.))
                    .mb(px(20.))
                    .child(logo)
                    .child(info),
            )
            .child(button)
    }
}
