//! 日历窗口标题栏（PRD R6）：padding 12 16、高 52；左区月份导航（上一月 / 年月 / 下一月）+「今天」；右区 日夜 / 锁 / 设置
//! （与清单窗口同一组按钮，`list_window::title_bar`）。清单筛选下拉留给任务第 5 步。
//!
//! 移动是自绘的（`windows::block::BlockDrag::title_bar`），锁定时不挂拖动；按钮都 `.occlude()`，按它们不会开始拖动。

use gpui_kit::{
    Context, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, Window, div, px,
};

use super::{CalendarCtx, CalendarWindow};
use crate::assets::EpIcon;
use crate::services::window_mode;
use crate::theme;
use crate::ui::icon::icon;
use crate::windows::list_window::title_bar::{
    lock_button, relative_line, settings_button, theme_button,
};

impl CalendarWindow {
    pub(super) fn render_title_bar(
        &mut self,
        rc: &CalendarCtx,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let block = self.block;
        let is_fixed = window_mode::is_fixed(cx, block);
        let is_dark = self.state.read(cx).is_dark(block);
        let palette = rc.palette;

        let bar = div()
            .id("title-bar")
            .relative()
            .flex()
            .flex_none()
            .items_center()
            .justify_between()
            .gap(px(12.))
            .px(px(16.))
            .py(px(12.));
        self.window_drag
            .title_bar(bar, !is_fixed, cx)
            .child(self.render_month_nav(rc, cx))
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(8.))
                    .child(theme_button(block, is_dark, &palette))
                    .child(lock_button(block, is_fixed, &palette))
                    .child(settings_button(&palette)),
            )
    }

    /// 月份导航 + 「今天」（以前主窗口开着日历时标题栏中区的那一组）
    fn render_month_nav(&mut self, rc: &CalendarCtx, cx: &mut Context<Self>) -> impl IntoElement {
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
                            .whitespace_nowrap()
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
                    .flex_none()
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
                    .whitespace_nowrap()
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
}
