//! 标题栏（§3.4）与只读详情排版（§3.5）。
//!
//! ```text
//! div.view-content        flex:1; padding:24px 28px; 滚动
//! ├─ h1.view-title        22px / 650(粗) / 行高 1.35 / 下边距 14；已完成灰色删除线
//! ├─ div.view-meta        徽章行：象限徽章（总是）+ 提醒徽章（有文案时）；下边距 18
//! └─ Markdown 渲染        或 "暂无描述"（13px #94A3B8）
//! ```

use gpui_kit::{
    App, Context, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, div, prelude::FluentBuilder as _, px, relative,
};

use super::{EditorWindow, QUADRANTS, RepeatType, WEEKDAYS};
use crate::assets::EpIcon;
use crate::theme::{self, hex, light, w2};
use crate::ui::button::EpButton;
use crate::ui::icon::icon;
use crate::ui::markdown::MarkdownView;
use crate::ui::title_bar::WindowTitleBar;
use crate::util::datetime::{self, DateTimeKind};

/// 重复设置（只读文案用）
pub(super) struct RepeatSummary<'a> {
    pub kind: RepeatType,
    pub interval: i32,
    pub weekdays: &'a [u8],
    pub month_day: i32,
}

/// 旧 `notifyStatusText`：只读详情里的提醒徽章文案（空串 = 不显示徽章）
pub(super) fn notify_status_text(
    repeat: Option<RepeatSummary<'_>>,
    notify_at: Option<&str>,
    notify_before: i32,
) -> String {
    let fmt = |v: &str| {
        datetime::format_date_time(Some(v), "YYYY-MM-DD HH:mm", DateTimeKind::Notify)
            .unwrap_or_else(|| v.to_string())
    };
    if let Some(repeat) = repeat {
        let mut text = format!("每 {} {}重复", repeat.interval, repeat.kind.unit());
        if repeat.kind == RepeatType::Weekly && !repeat.weekdays.is_empty() {
            let names: Vec<&str> = WEEKDAYS
                .iter()
                .filter(|(day, _)| repeat.weekdays.contains(day))
                .map(|(_, name)| *name)
                .collect();
            text.push_str(&format!("（{}）", names.join("、")));
        } else if repeat.kind == RepeatType::Monthly {
            text.push_str(&format!("（每月 {} 号）", repeat.month_day));
        }
        if let Some(at) = notify_at {
            text.push_str(&format!("，下次 {}", fmt(at)));
        }
        return text;
    }
    match notify_at {
        Some(at) => {
            let mut text = format!("提醒 {}", fmt(at));
            if notify_before > 0 {
                text.push_str(&format!("（提前 {notify_before} 分钟）"));
            }
            text
        }
        None => String::new(),
    }
}

impl EditorWindow {
    /// 标题栏：h2 + [编辑]（只读时）+ 关闭（直接关窗，不确认）
    pub(super) fn render_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        WindowTitleBar::new("editor-title", self.header_title())
            .when(self.is_view_mode, |bar| {
                bar.action(
                    EpButton::new("editor-edit")
                        .text()
                        .primary()
                        .icon(EpIcon::Edit)
                        .icon_size(14.)
                        .label("编辑")
                        .on_click(cx.listener(|this, _, _, cx| {
                            // 原地切换：不重新加载、不重拍快照
                            this.is_view_mode = false;
                            cx.notify();
                        })),
                )
            })
            .on_close(|_, window, _| window.remove_window())
    }

    fn notify_status(&self, cx: &App) -> String {
        let weekdays = self.repeat_weekdays.clone();
        let repeat = self.repeat_enabled.then(|| RepeatSummary {
            kind: self.repeat_type,
            interval: self.repeat_interval_value(cx),
            weekdays: &weekdays,
            month_day: self.repeat_month_day_value(cx),
        });
        notify_status_text(
            repeat,
            self.form_notify_at(cx).as_deref(),
            self.notify_before,
        )
    }

    /// 只读详情
    pub(super) fn render_view(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = light();
        let completed = self.todo.as_ref().is_some_and(|t| t.completed);
        let title = self.title.read(cx).value().to_string();
        let (_, quadrant_name, quadrant_color) = QUADRANTS
            .iter()
            .copied()
            .find(|(id, _, _)| *id == self.quadrant)
            .unwrap_or(QUADRANTS[3]);
        let notify = self.notify_status(cx);
        let description = self.description_value(cx);

        let badge = || {
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .px(px(10.))
                .py(px(3.))
                .rounded_full()
                .text_size(px(12.))
                .line_height(relative(1.6))
        };
        let meta = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(8.))
            .mb(px(18.))
            .child(
                badge()
                    .text_color(hex(quadrant_color))
                    .bg(hex(quadrant_color).opacity(0.1))
                    .child(
                        div()
                            .flex_none()
                            .size(px(8.))
                            .rounded_full()
                            .bg(hex(quadrant_color)),
                    )
                    .child(quadrant_name),
            )
            .when(!notify.is_empty(), |this| {
                this.child(
                    badge()
                        .text_color(palette.text_secondary)
                        .bg(palette.bg_tertiary)
                        .child(icon(EpIcon::Bell, 13., palette.text_secondary))
                        .child(notify),
                )
            });

        let body = if description.is_empty() {
            div()
                .text_size(px(13.))
                .text_color(palette.text_tertiary)
                .child("暂无描述")
                .into_any_element()
        } else {
            let weak = cx.entity().downgrade();
            MarkdownView::new(
                "editor-view-md",
                gpui_kit::SharedString::from(description),
                self.images_dir.clone(),
            )
            .readonly(true)
            .selectable(true)
            .on_image_click(move |paths, index, window, cx| {
                let _ = weak.update(cx, |this, cx| {
                    this.open_image_viewer(paths, index, window, cx)
                });
            })
            .into_any_element()
        };

        div()
            .id("editor-view")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&self.view_scroll)
            .child(
                div()
                    .px(px(28.))
                    .py(px(24.))
                    .child(
                        div()
                            .mb(px(14.))
                            .text_size(px(22.))
                            .font_weight(gpui_kit::FontWeight::BOLD)
                            .line_height(relative(1.35))
                            .text_color(if completed {
                                palette.text_tertiary
                            } else {
                                hex(w2::MD_TEXT)
                            })
                            .when(completed, |this| this.line_through())
                            .child(title),
                    )
                    .child(meta)
                    .child(body),
            )
            .font(theme::ui_font())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notify_status_matches_original_wording() {
        assert_eq!(
            notify_status_text(None, Some("2026-10-20 14:30:00"), 15),
            "提醒 2026-10-20 14:30（提前 15 分钟）"
        );
        assert_eq!(
            notify_status_text(None, Some("2026-10-20 14:30:00"), 0),
            "提醒 2026-10-20 14:30"
        );
        assert_eq!(notify_status_text(None, None, 15), "");
        let weekdays = [3, 1];
        assert_eq!(
            notify_status_text(
                Some(RepeatSummary {
                    kind: RepeatType::Weekly,
                    interval: 2,
                    weekdays: &weekdays,
                    month_day: 1,
                }),
                Some("2026-10-20 09:00:00"),
                15,
            ),
            "每 2 周重复（周一、周三），下次 2026-10-20 09:00"
        );
        assert_eq!(
            notify_status_text(
                Some(RepeatSummary {
                    kind: RepeatType::Monthly,
                    interval: 1,
                    weekdays: &[],
                    month_day: 31,
                }),
                None,
                0,
            ),
            "每 1 月重复（每月 31 号）"
        );
        assert_eq!(
            notify_status_text(
                Some(RepeatSummary {
                    kind: RepeatType::Daily,
                    interval: 1,
                    weekdays: &[],
                    month_day: 1,
                }),
                None,
                0,
            ),
            "每 1 天重复"
        );
    }
}
