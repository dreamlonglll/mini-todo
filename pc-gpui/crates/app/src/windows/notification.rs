//! 应用内通知窗口（旧 `NotificationView.vue` + `services/notification.rs::send_app_notification`）。
//! **W2 实现**界面与调度；现在是带真实摆放逻辑的占位视图。
//!
//! 规格：`research/ui-editor-windows.md` §11。窗口 320×120（逻辑）、透明、置顶、不进任务栏、不抢焦点、
//! 不可缩放；位置用 core 的 `reminder::notification_position`（主显示器工作区右下角起自下而上叠放、
//! 满列向左换列），槽位 `NOTIFICATION_SLOTS` 在窗口关闭时归还。单击窗口任意位置关闭。

use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, Bounds, ClickEvent, Context, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, SharedString, StatefulInteractiveElement as _,
    Styled as _, Window, WindowKind as GpuiWindowKind, div, point, px,
};
use mini_todo_core::reminder::{
    self, FALLBACK_WORK_AREA, NOTIFICATION_SLOTS, REQUESTED_SIZE, Reminder, WorkArea,
};

use super::{WindowKind, WindowManager, WindowSpec, logical_size};
use crate::platform;
use crate::theme::{self, light};

/// 通知内容
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotificationContent {
    pub todo_id: Option<i64>,
    /// 标题（空时界面显示 `待办提醒`）
    pub title: String,
    /// 描述原文（Markdown 不渲染；空时不显示）
    pub description: String,
}

impl From<&Reminder> for NotificationContent {
    fn from(reminder: &Reminder) -> Self {
        Self {
            todo_id: Some(reminder.id),
            title: reminder.title.clone(),
            description: reminder.body().to_string(),
        }
    }
}

/// 通知窗口视图（W2 替换内容）
pub struct NotificationWindow {
    pub label: String,
    pub content: NotificationContent,
}

impl Render for NotificationWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = light();
        let title = if self.content.title.is_empty() {
            SharedString::from("待办提醒")
        } else {
            SharedString::from(self.content.title.clone())
        };
        div()
            .id("notification-window")
            .size_full()
            .font(theme::ui_font())
            .text_size(theme::FONT_SIZE)
            .cursor_pointer()
            .on_click(cx.listener(|_, _: &ClickEvent, window, _| window.remove_window()))
            .child(
                div()
                    .size_full()
                    .flex()
                    .flex_col()
                    .justify_between()
                    .p(px(14.))
                    .bg(theme::rgba_css(255, 255, 255, 0.98))
                    .rounded(theme::radius::LG)
                    .border_1()
                    .border_color(theme::rgba_css(0, 0, 0, 0.05))
                    .child(
                        div()
                            .font_weight(theme::WEIGHT_SEMIBOLD)
                            .text_color(palette.text_primary)
                            .truncate()
                            .child(title),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(palette.text_secondary)
                            .truncate()
                            .child(SharedString::from(self.content.description.clone())),
                    )
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .text_size(px(11.))
                            .text_color(palette.text_tertiary)
                            .child("Mini Todo"),
                    ),
            )
    }
}

/// 主显示器工作区（逻辑像素）
fn primary_work_area(cx: &App) -> WorkArea {
    platform::primary_monitor(cx)
        .map(|m| {
            let work = m.logical_work();
            WorkArea {
                x: f64::from(f32::from(work.origin.x)),
                y: f64::from(f32::from(work.origin.y)),
                width: f64::from(f32::from(work.size.width)),
                height: f64::from(f32::from(work.size.height)),
            }
        })
        .unwrap_or(FALLBACK_WORK_AREA)
}

/// 打开一个通知窗口；失败时归还槽位并返回 `None`
pub fn open(cx: &mut App, content: NotificationContent) -> Option<AnyWindowHandle> {
    let label = reminder::next_window_label();
    let slot = NOTIFICATION_SLOTS.acquire();
    let area = primary_work_area(cx);
    let size = reminder::last_actual_size().unwrap_or(REQUESTED_SIZE);
    let (x, y) = reminder::notification_position(area, size, slot);
    let mut spec = WindowSpec::new(
        "通知",
        Bounds::new(
            point(px(x as f32), px(y as f32)),
            logical_size(size.width as f32, size.height as f32),
        ),
    );
    spec.transparent = true;
    spec.resizable = false;
    spec.minimizable = false;
    spec.focus = false;
    spec.kind = GpuiWindowKind::PopUp;
    spec.display_id = platform::primary_monitor(cx).map(|m| m.display_id);

    let view_label = label.clone();
    let handle: AnyWindowHandle = match super::open_window(cx, &spec, move |_, cx| {
        cx.new(|_| NotificationWindow {
            label: view_label,
            content,
        })
    }) {
        Ok(handle) => handle.into(),
        Err(e) => {
            NOTIFICATION_SLOTS.release(slot);
            log::error!("[notification] 打开通知窗口失败: {e}");
            return None;
        }
    };
    WindowManager::register(cx, WindowKind::Notification, Some(label), handle);
    WindowManager::on_closed(cx, handle, move |_| NOTIFICATION_SLOTS.release(slot));
    Some(handle)
}

/// 关闭全部通知窗口（旧 `close_all_notification_windows`）
pub fn close_all(cx: &mut App) {
    for handle in WindowManager::all(cx, WindowKind::Notification) {
        WindowManager::close(cx, handle);
    }
}
