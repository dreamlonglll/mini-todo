//! 「常规」面板（§2.7）：开机自启、展示日历、贴边自动隐藏、唤起时置顶、固定模式时嵌入桌面（仅 Windows）、
//! 深色主题、界面底色、背景透明度、通知方式。全部即时生效（改动即写库并更新 AppState）。

use gpui_kit::{
    Context, InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, Styled as _, Task, Window, div, prelude::FluentBuilder as _,
    px,
};

use super::{SettingsWindow, panel_title, settings_row};
use crate::assets::EpIcon;
use crate::services::autostart;
use crate::state::{AppState, NotificationType};
use crate::theme::{self, PRESET_BG_COLORS, settings_tokens};
use crate::ui::ep_radio::EpRadioButtons;
use crate::ui::ep_slider::EpSlider;
use crate::ui::ep_switch::EpSwitch;
use crate::ui::message;

/// `AppState` 上的布尔设置写入方法（`set_show_calendar` 等）：先改内存、后台写库，失败回滚并返回提示文本
type SettingSetter = fn(&mut AppState, bool, &mut Context<AppState>) -> Task<Result<(), String>>;

/// 设置项写库失败的统一处理：AppState 已回滚，`Err` 是完整提示文本
fn report_setting_error(
    task: Task<Result<(), String>>,
    window: &mut Window,
    cx: &mut Context<SettingsWindow>,
) {
    cx.spawn_in(window, async move |_, cx| {
        if let Err(text) = task.await {
            let _ = cx.update(|window, cx| message::error(window, cx, text));
        }
    })
    .detach();
}

impl SettingsWindow {
    pub(super) fn load_autostart(&mut self, cx: &mut Context<Self>) {
        let task = cx
            .background_executor()
            .spawn(async { autostart::is_enabled() });
        cx.spawn(async move |this, cx| {
            let enabled = match task.await {
                Ok(enabled) => enabled,
                Err(e) => {
                    log::warn!("[settings] 读取开机自启状态失败: {e}");
                    false
                }
            };
            let _ = this.update(cx, |this, cx| {
                this.auto_start = enabled;
                cx.notify();
            });
        })
        .detach();
    }

    fn set_auto_start(&mut self, enabled: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.auto_start_loading {
            return;
        }
        self.auto_start = enabled;
        self.auto_start_loading = true;
        cx.notify();
        let task = cx
            .background_executor()
            .spawn(async move { autostart::set_enabled(enabled) });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.auto_start_loading = false;
                match result {
                    Ok(()) => {
                        message::success(
                            window,
                            cx,
                            if enabled {
                                "已开启开机自启"
                            } else {
                                "已关闭开机自启"
                            },
                        );
                        // 旧版此处调 `sync_auto_start_state` 回写托盘勾选
                        crate::services::tray::set_auto_start_checked(cx, enabled);
                    }
                    Err(e) => {
                        log::error!("[settings] 设置开机自启失败: {e}");
                        this.auto_start = !enabled;
                        message::error(window, cx, "设置开机自启失败");
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn set_bg_color(
        &mut self,
        color: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let alpha = self.state.read(cx).window_bg.alpha;
        let task = self.state.update(cx, |state, cx| {
            state.set_window_background(color, alpha, cx)
        });
        report_setting_error(task, window, cx);
    }

    fn set_bg_alpha(&mut self, percent: f32, window: &mut Window, cx: &mut Context<Self>) {
        let color = self.state.read(cx).window_bg.color.clone();
        let task = self.state.update(cx, |state, cx| {
            state.set_window_background(color, f64::from(percent) / 100., cx)
        });
        self.alpha_preview = None;
        report_setting_error(task, window, cx);
    }

    fn set_notification_type(
        &mut self,
        value: NotificationType,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let previous = self.state.read(cx).notification_type;
        if previous == value || self.notification_switching {
            return;
        }
        self.notification_switching = true;
        let task = self
            .state
            .update(cx, |state, cx| state.set_notification_type(value, cx));
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.notification_switching = false;
                match result {
                    Ok(()) => message::success(
                        window,
                        cx,
                        match value {
                            NotificationType::System => "已切换为系统通知",
                            NotificationType::App => "已切换为软件通知",
                        },
                    ),
                    Err(e) => {
                        log::error!("[settings] 设置通知类型失败: {e}");
                        this.state.update(cx, |state, cx| {
                            state.notification_type = previous;
                            cx.notify();
                        });
                        message::error(window, cx, "设置通知类型失败");
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn render_general(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let t = settings_tokens();
        let state = self.state.read(cx);
        let show_calendar = state.show_calendar;
        let auto_hide = state.auto_hide_enabled;
        let top_on_wake = state.top_on_wake;
        let embed = state.fixed_embed_desktop;
        let dark = state.is_dark_theme;
        let bg_color = state.window_bg.color.clone();
        let alpha = self
            .alpha_preview
            .unwrap_or((state.window_bg.alpha * 100.).round() as f32);
        let notification = state.notification_type;

        let state_entity = self.state.clone();
        let toggle = move |id: &'static str,
                           checked: bool,
                           disabled: bool,
                           apply: SettingSetter,
                           cx: &mut Context<Self>| {
            let state = state_entity.clone();
            EpSwitch::new(id)
                .checked(checked)
                .disabled(disabled)
                .on_change(cx.listener(move |_, value: &bool, window, cx| {
                    let value = *value;
                    let task = state.update(cx, |state, cx| apply(state, value, cx));
                    report_setting_error(task, window, cx);
                }))
        };

        // 第 7 行：6 个预设色圆点 + 颜色选择器
        let swatches = div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(6.))
            .children(PRESET_BG_COLORS.iter().map(|&(name, value)| {
                let active = theme::is_same_color(&bg_color, value);
                let (r, g, b) = theme::hex_to_rgb_channels(value);
                div()
                    .id(SharedString::from(format!("preset-{value}")))
                    .size(px(20.))
                    .rounded_full()
                    .border_2()
                    .border_color(if active {
                        t.primary
                    } else {
                        theme::rgba_css(0, 0, 0, 0.)
                    })
                    .bg(theme::rgba_css(r, g, b, 1.))
                    // `box-shadow: 0 0 0 1px #E2E8F0`（外圈细灰线）
                    .shadow(vec![theme::shadow(0., 0., 0., 1., t.border)])
                    .tooltip(crate::ui::tooltip::native(name))
                    .when(!dark, |this| {
                        this.opacity(0.4)
                            .cursor(gpui_kit::CursorStyle::OperationNotAllowed)
                    })
                    .when(dark, |this| {
                        this.cursor_pointer()
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.set_bg_color(value.to_string(), window, cx)
                            }))
                    })
            }))
            .child(self.bg_picker.clone());

        // 第 8 行：滑块 + 百分比
        let slider = div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(12.))
            .w(px(180.))
            .child(
                div().flex_1().child(
                    EpSlider::new("bg-alpha", alpha)
                        .range(5., 100.)
                        .step(5.)
                        .disabled(!dark)
                        .tooltip(|v| format!("{v}%").into())
                        .on_input(cx.listener(|this, v: &f32, _, cx| {
                            this.alpha_preview = Some(*v);
                            cx.notify();
                        }))
                        .on_change(cx.listener(|this, v: &f32, window, cx| {
                            this.set_bg_alpha(*v, window, cx)
                        })),
                ),
            )
            .child(
                div()
                    .min_w(px(34.))
                    .text_right()
                    .text_size(px(12.))
                    .text_color(t.secondary)
                    .child(format!("{}%", alpha.round() as i32)),
            );

        let radio = EpRadioButtons::new("notification-type")
            .small()
            .padding(6., 12.)
            .option("system", "系统通知")
            .option("app", "软件通知")
            .selected(Some(notification.as_str()))
            .disabled(self.notification_switching)
            .on_change(cx.listener(|this, value: &SharedString, window, cx| {
                this.set_notification_type(NotificationType::parse(value), window, cx)
            }));

        div()
            .flex()
            .flex_col()
            .child(panel_title("常规"))
            .child(settings_row(
                EpIcon::Monitor,
                "开机自启",
                None,
                EpSwitch::new("auto-start")
                    .checked(self.auto_start)
                    .loading(self.auto_start_loading)
                    .on_change(cx.listener(|this, value: &bool, window, cx| {
                        this.set_auto_start(*value, window, cx)
                    })),
                false,
            ))
            .child(settings_row(
                EpIcon::Calendar,
                "展示日历",
                Some("开启后主界面将显示日历视图"),
                toggle("show-calendar", show_calendar, false, AppState::set_show_calendar, cx),
                false,
            ))
            .child(settings_row(
                EpIcon::Monitor,
                "贴边自动隐藏",
                Some("固定模式下，贴边后自动隐藏并在边缘唤起"),
                toggle("auto-hide", auto_hide, false, AppState::set_auto_hide_enabled, cx),
                false,
            ))
            .child(settings_row(
                EpIcon::Top,
                "唤起时置顶",
                Some("贴边唤起时显示在其它窗口之上；关闭后可能被全屏窗口遮挡"),
                toggle("top-on-wake", top_on_wake, !auto_hide, AppState::set_top_on_wake, cx),
                false,
            ))
            .when(cfg!(windows) || show_embed_row_for_screenshots(), |this| {
                this.child(settings_row(
                    EpIcon::Monitor,
                    "固定模式时，嵌入桌面中",
                    Some(
                        "固定后窗口嵌在桌面图标之上、其它窗口之下，显示桌面（Win+D）时不会被隐藏；贴边隐藏与唤起置顶不再生效。 需要 Windows 11 24H2 及以上",
                    ),
                    toggle(
                        "embed-desktop",
                        embed,
                        false,
                        AppState::set_fixed_embed_desktop,
                        cx,
                    ),
                    false,
                ))
            })
            .child(settings_row(
                EpIcon::Moon,
                "深色主题",
                Some("启用半透明深色外观"),
                toggle("dark-theme", dark, false, AppState::set_dark_theme, cx),
                false,
            ))
            .child(settings_row(
                EpIcon::Brush,
                "界面底色",
                Some("仅深色主题下生效"),
                swatches,
                false,
            ))
            .child(settings_row(
                EpIcon::Sunny,
                "背景透明度",
                Some("数值越低越透明，仅深色主题下生效"),
                slider,
                false,
            ))
            .child(settings_row(
                EpIcon::Bell,
                "通知方式",
                Some("选择待办提醒的通知展示方式"),
                radio,
                true,
            ))
    }
}

/// 「固定模式时，嵌入桌面中」只在 Windows 显示；debug 构建可以用 `MINI_TODO_DEBUG_WINDOWS_UI=1` 在 Linux 截图里
/// 也显示它（参考截图按 Windows UA 渲染）
fn show_embed_row_for_screenshots() -> bool {
    #[cfg(debug_assertions)]
    {
        std::env::var("MINI_TODO_DEBUG_WINDOWS_UI").is_ok_and(|v| v == "1")
    }
    #[cfg(not(debug_assertions))]
    {
        false
    }
}
