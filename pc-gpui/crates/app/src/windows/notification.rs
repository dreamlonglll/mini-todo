//! 应用内通知窗口（旧 `NotificationView.vue` + `services/notification.rs::send_app_notification`）。
//!
//! 规格：`research/ui-editor-windows.md` §11。窗口 320×120（逻辑）、透明、置顶、不进任务栏、不抢焦点、
//! 不可缩放；位置用 core 的 `reminder::notification_position`（主显示器工作区右下角起自下而上叠放、
//! 满列向左换列），槽位 `NOTIFICATION_SLOTS` 在窗口关闭时归还。单击窗口任意位置关闭。

use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, Bounds, ClickEvent, Context, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, SharedString, StatefulInteractiveElement as _,
    Styled as _, Window, WindowKind as GpuiWindowKind, div, linear_color_stop, linear_gradient,
    point, prelude::FluentBuilder as _, px, relative,
};
use mini_todo_core::reminder::{
    self, FALLBACK_WORK_AREA, NOTIFICATION_SLOTS, NotificationSize, REQUESTED_SIZE, Reminder,
    WorkArea,
};

use super::{WindowKind, WindowManager, WindowSpec, logical_size};
use crate::assets::EpIcon;
use crate::platform;
use crate::theme::{self, editor_theme, hex, light};
use crate::ui::icon::icon;

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

/// 通知窗口视图（§11.4）
///
/// ```text
/// div.notification-card      满窗、白 98%、圆角 12、1px rgba(0,0,0,.05) 边、flex column、溢出裁掉
/// ├─ header                  padding 12 14 8：36×36 渐变铃铛 + 24×24 关闭按钮
/// ├─ content                 flex:1；标题 14px/600 单行省略 + 描述 12px/1.4 最多两行（320×120 时被裁掉，与旧版一致）
/// └─ footer                  padding 8 14 12，右对齐 "Mini Todo"
/// ```
/// 单击窗口任意位置关闭。建窗后 `open_window` 先按客户区摆到槽位，再按真实尺寸校正位置（`fit_to_actual_size`），
/// 窗口尺寸变化时再校正一次。
pub struct NotificationWindow {
    pub label: String,
    pub content: NotificationContent,
    area: WorkArea,
    slot: u32,
    placed_for: NotificationSize,
    _subscription: Option<gpui_kit::Subscription>,
}

impl NotificationWindow {
    fn new(
        label: String,
        content: NotificationContent,
        area: WorkArea,
        slot: u32,
        placed_for: NotificationSize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let subscription =
            cx.observe_window_bounds(window, |this, window, cx| this.fit(window, cx));
        Self {
            label,
            content,
            area,
            slot,
            placed_for,
            _subscription: Some(subscription),
        }
    }

    /// 旧 `fit_to_actual_size`：记下真实尺寸；与摆放时假设的不同就按真实尺寸重算本槽位并移过去。
    /// 尺寸与位置都按**客户区**（GPUI `window.bounds()` 就是客户区；外框另有不可见边框）
    fn fit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let bounds = window.bounds();
        let actual = NotificationSize {
            width: f64::from(f32::from(bounds.size.width)),
            height: f64::from(f32::from(bounds.size.height)),
        };
        if actual.width <= 0. || actual.height <= 0. {
            return;
        }
        if let Some((x, y)) =
            reminder::fit_to_actual_size(self.area, self.slot, self.placed_for, actual)
        {
            self.placed_for = actual;
            let scale = f64::from(window.scale_factor());
            platform::set_client_position_physical(
                window,
                cx,
                (x * scale).round() as i32,
                (y * scale).round() as i32,
            );
        }
    }
}

impl Render for NotificationWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = light();
        let title = if self.content.title.is_empty() {
            SharedString::from("待办提醒")
        } else {
            SharedString::from(self.content.title.clone())
        };
        let description = self.content.description.clone();
        let header = div()
            .flex_none()
            .flex()
            .items_center()
            .justify_between()
            .pt(px(12.))
            .px(px(14.))
            .pb(px(8.))
            .child(
                div()
                    .size(px(36.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(8.))
                    .bg(linear_gradient(
                        135.,
                        linear_color_stop(palette.primary, 0.),
                        linear_color_stop(hex(editor_theme::CYAN_500), 1.),
                    ))
                    .child(icon(EpIcon::Bell, 24., hex(0xFFFFFF))),
            )
            .child(
                div()
                    .id("notification-close")
                    .group("notification-close")
                    .size(px(24.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(6.))
                    .bg(palette.bg_tertiary)
                    .hover(|s| s.bg(palette.border))
                    .cursor_pointer()
                    .on_click(cx.listener(|_, _: &ClickEvent, window, cx| {
                        cx.stop_propagation();
                        window.remove_window();
                    }))
                    .child(
                        icon(EpIcon::Close, 14., palette.text_secondary)
                            .group_hover("notification-close", |s| {
                                s.text_color(hex(editor_theme::SLATE_700))
                            }),
                    ),
            );
        let content = div()
            .flex_1()
            .min_h_0()
            .px(px(14.))
            .overflow_hidden()
            .child(
                div()
                    .mb(px(4.))
                    .text_size(px(14.))
                    .font_weight(theme::WEIGHT_SEMIBOLD)
                    .text_color(palette.text_primary)
                    .truncate()
                    .child(title),
            )
            .when(!description.is_empty(), |this| {
                this.child(
                    div()
                        .text_size(px(12.))
                        .line_height(relative(1.4))
                        .text_color(palette.text_secondary)
                        .line_clamp(2)
                        .child(SharedString::from(description)),
                )
            });
        let footer = div()
            .flex_none()
            .flex()
            .justify_end()
            .pt(px(8.))
            .px(px(14.))
            .pb(px(12.))
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(palette.text_tertiary)
                    .child("Mini Todo"),
            );
        div()
            .id("notification-window")
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .font(theme::ui_font())
            .text_size(theme::FONT_SIZE)
            .line_height(relative(theme::LINE_HEIGHT))
            .cursor_pointer()
            .on_click(cx.listener(|_, _: &ClickEvent, window, _| window.remove_window()))
            .child(
                div()
                    .size_full()
                    .flex()
                    .flex_col()
                    .overflow_hidden()
                    .bg(theme::rgba_css(255, 255, 255, 0.98))
                    .rounded(theme::radius::LG)
                    .border_1()
                    .border_color(theme::rgba_css(0, 0, 0, 0.05))
                    .shadow(vec![
                        theme::shadow(0., 8., 32., 0., theme::rgba_css(0, 0, 0, 0.15)),
                        theme::shadow(0., 2., 8., 0., theme::rgba_css(0, 0, 0, 0.1)),
                    ])
                    .child(header)
                    .child(content)
                    .child(footer),
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
    let handle: AnyWindowHandle = match super::open_window(cx, &spec, move |window, cx| {
        cx.new(|cx| NotificationWindow::new(view_label, content, area, slot, size, window, cx))
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
    // 建窗返回时尺寸已经定下的平台之后不一定再发尺寸变化：立即校正一次
    let _ = cx.update_window(handle, |view, window, cx| {
        if let Ok(view) = view.downcast::<NotificationWindow>() {
            view.update(cx, |this, cx| this.fit(window, cx));
        }
    });
    Some(handle)
}

/// 关闭全部通知窗口（旧 `close_all_notification_windows`）
pub fn close_all(cx: &mut App) {
    for handle in WindowManager::all(cx, WindowKind::Notification) {
        WindowManager::close(cx, handle);
    }
}
