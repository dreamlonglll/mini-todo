//! 收纳箱的面板（外观 / 删除确认 / OneDrive 提示，§3.7–§3.9）与标题栏「更多」下拉菜单（§3.4.4）。

use gpui_kit::{
    AnyElement, AppContext as _, Context, InteractiveElement as _, IntoElement, ParentElement as _,
    SharedString, StatefulInteractiveElement as _, Styled as _, Window, canvas, div, point, px,
};
use mini_todo_core::db::UpdateFenceRequest;

use super::logic::{self, timing};
use super::{ExpandReason, FenceWindow, Panel};
use crate::assets::EpIcon;
use crate::db::spawn_db;
use crate::platform;
use crate::services::fences as runtime;
use crate::theme::{self, PRESET_BG_COLORS, fence_tokens};
use crate::ui::button::EpButton;
use crate::ui::ep_dropdown::{EpDropdownItem, EpDropdownMenu};
use crate::ui::ep_popper::{self, Placement, PopperOptions};
use crate::ui::ep_slider::EpSlider;
use crate::ui::icon::icon;
use crate::ui::message::{self, MessageKind};

/// 「更多」菜单里「显示隐藏的图标」切换项的文案（EP 下拉菜单没有勾选态，用切换文案表达当前状态）
fn show_hidden_label(show_hidden: bool) -> &'static str {
    if show_hidden {
        "不显示隐藏的图标"
    } else {
        "显示隐藏的图标"
    }
}

/// 「更多」菜单的估算宽度：最长一项（左右内边距 16 + 图标 14 + 间距 5 + 每字 14）+ 2 边框。
/// 各项都是中文，最长的是切换项（7 / 8 个字），其次「删除收纳箱」（5 个字）
fn title_menu_width(labels: &[&str]) -> f32 {
    let longest = labels.iter().map(|l| l.chars().count()).max().unwrap_or(0);
    16. + 14. + 5. + 14. * longest as f32 + 16. + 2.
}

impl FenceWindow {
    pub(super) fn set_panel(&mut self, panel: Panel, window: &mut Window, cx: &mut Context<Self>) {
        if self.panel == Panel::Appearance && panel != Panel::Appearance {
            self.alpha_preview = None;
        }
        self.panel = panel;
        self.set_reason(ExpandReason::Panel, panel != Panel::None, window, cx);
        cx.notify();
    }

    fn open_delete_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.delete_count = None;
        self.set_panel(Panel::Delete, window, cx);
        // 以文件夹实际列举为准：含收纳箱里隐藏的项目（它们同样会移回），不含带 Windows 隐藏 / 系统属性的文件
        let id = self.fence_id;
        let task = spawn_db(cx, move |db| {
            mini_todo_core::fence::items::get_items(db, id)
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if let Ok(list) = result {
                    this.delete_count = Some(list.len());
                    this.set_items(list, cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// 旧 `confirmDelete`：后台删除（全部移回成功才删记录、关窗；否则整个保留并提示原因）
    fn confirm_delete(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.deleting {
            return;
        }
        self.deleting = true;
        cx.notify();
        let id = self.fence_id;
        // 系统 UAC / "文件正在使用"对话框以本窗口为父：移动在后台线程执行，绝不在主线程
        let owner = platform::hwnd(window).map(|h| h.0);
        let task = spawn_db(cx, move |db| mini_todo_core::fence::delete(db, id, owner));
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.deleting = false;
                match result {
                    Ok(result) if result.deleted => {
                        log::info!("[fence] 已删除收纳箱 #{id}");
                        let handle = window.window_handle();
                        cx.defer(move |cx| {
                            runtime::close_deleted(cx, id);
                            let _ = cx.update_window(handle, |_, window, _| window.remove_window());
                        });
                    }
                    Ok(result) => {
                        this.set_panel(Panel::None, window, cx);
                        message::show_for(
                            window,
                            cx,
                            MessageKind::Warning,
                            logic::describe_delete_kept(&result),
                            timing::DELETE_KEPT_MESSAGE_DURATION,
                        );
                        this.refresh_items(cx);
                    }
                    Err(e) => message::error_with(window, cx, "删除收纳箱失败", &e),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn set_bg_color(&mut self, color: String, window: &mut Window, cx: &mut Context<Self>) {
        let same = self
            .fence
            .as_ref()
            .is_some_and(|f| theme::is_same_color(&f.bg_color, &color));
        if same {
            return;
        }
        if let Some(fence) = &mut self.fence {
            fence.bg_color = color.to_uppercase();
        }
        self.patch_fence(
            UpdateFenceRequest {
                bg_color: Some(color),
                ..Default::default()
            },
            "设置收纳箱背景色失败",
            window,
            cx,
        );
    }

    fn pick_custom_color(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let initial = self
            .fence
            .as_ref()
            .map(|f| theme::hex_to_rgb_channels(&f.bg_color))
            .unwrap_or((0, 0, 0));
        let owner = platform::hwnd(window);
        let task = platform::fence_win::choose_color(cx, owner, initial);
        cx.spawn_in(window, async move |this, cx| {
            if let Some((r, g, b)) = task.await {
                let color = logic::color_hex(r, g, b);
                let _ = this.update_in(cx, |this, window, cx| this.set_bg_color(color, window, cx));
            }
        })
        .detach();
    }

    fn set_bg_alpha(&mut self, percent: f32, window: &mut Window, cx: &mut Context<Self>) {
        let alpha = f64::from(percent) / 100.;
        if let Some(fence) = &mut self.fence {
            fence.bg_alpha = alpha;
        }
        self.patch_fence(
            UpdateFenceRequest {
                bg_alpha: Some(alpha),
                ..Default::default()
            },
            "设置收纳箱透明度失败",
            window,
            cx,
        );
        self.alpha_preview = None;
    }

    // ------------------------------------------------------------------
    // 「更多」菜单
    // ------------------------------------------------------------------

    pub(super) fn toggle_title_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.title_menu_open {
            self.close_title_menu(window, cx);
        } else {
            self.title_menu_open = true;
            // 折叠态下窗口先展开，菜单才有地方弹出
            self.set_reason(ExpandReason::Menu, true, window, cx);
            cx.notify();
        }
    }

    pub(super) fn close_title_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.title_menu_open {
            return;
        }
        self.title_menu_open = false;
        cx.notify();
        // 下一帧再撤 menu 原因：让菜单项打开的面板先接管 panel 原因，避免收回又展开
        cx.on_next_frame(window, |this, window, cx| {
            this.set_reason(ExpandReason::Menu, false, window, cx);
        });
    }

    fn on_title_command(&mut self, command: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.close_title_menu(window, cx);
        match command {
            "rename" => self.start_rename(window, cx),
            "appearance" => self.set_panel(Panel::Appearance, window, cx),
            "show-hidden" => self.toggle_show_hidden(window, cx),
            "delete" => self.open_delete_panel(window, cx),
            _ => {}
        }
    }

    /// 标题栏「更多」下拉菜单（`bottom-end`，右边与按钮右边对齐、箭头对准按钮中心）；
    /// 关闭时不渲染任何元素（否则空元素会在标题栏里多占一个 gap）
    pub(super) fn render_title_menu(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if !self.title_menu_open {
            return None;
        }
        let show_hidden = self.show_hidden();
        let toggle_label = show_hidden_label(show_hidden);
        let width = title_menu_width(&["重命名", "外观", toggle_label, "删除收纳箱"]);
        let menu = EpDropdownMenu::new("fence-more-menu")
            .item(EpDropdownItem::new("rename", "重命名").icon(EpIcon::EditPen))
            .item(EpDropdownItem::new("appearance", "外观").icon(EpIcon::Brush))
            .item(
                EpDropdownItem::new("show-hidden", toggle_label).icon(if show_hidden {
                    EpIcon::Hide
                } else {
                    EpIcon::View
                }),
            )
            .item(
                EpDropdownItem::new("delete", "删除收纳箱")
                    .icon(EpIcon::Delete)
                    .divided(),
            )
            .max_height(px(200.))
            .on_select(cx.listener(|this, command: &SharedString, window, cx| {
                this.on_title_command(command, window, cx)
            }));
        let dismiss = cx.entity().downgrade();
        ep_popper::popper(
            &self.more_anchor,
            window,
            PopperOptions {
                placement: Placement::BottomEnd,
                // 菜单只能在标题栏下方（折叠态打开时窗口已先展开）
                flip: false,
                gap: ep_popper::OFFSET,
                arrow: true,
                width,
                height: EpDropdownMenu::estimated_height(4, 1),
            },
            div().w(px(width)).child(menu),
            move |window, cx| {
                // 点在「更多」按钮上不会走到这里（交给按钮自己切换）
                let _ = dismiss.update(cx, |this, cx| this.close_title_menu(window, cx));
            },
        )
    }

    // ------------------------------------------------------------------
    // 面板
    // ------------------------------------------------------------------

    pub(super) fn render_panel(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let fence = self.fence.clone()?;
        let t = fence_tokens();
        let body = match self.panel {
            Panel::None => return None,
            Panel::Appearance => self.render_appearance_panel(&fence, cx),
            Panel::Delete => {
                let deleting = self.deleting;
                let counting = self.delete_count.is_none();
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .gap(px(10.))
                    .child(
                        div()
                            .flex_1()
                            .child(
                                div()
                                    .mb(px(6.))
                                    .font_weight(theme::WEIGHT_SEMIBOLD)
                                    .child(format!("删除收纳箱「{}」？", fence.title)),
                            )
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(t.panel_label)
                                    .child(logic::delete_hint(self.delete_count)),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            // gap 8 + Element Plus 相邻按钮的 margin-left 12
                            .gap(px(20.))
                            .child(
                                EpButton::new("delete-cancel")
                                    .small()
                                    .label("取消")
                                    .disabled(deleting)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.set_panel(Panel::None, window, cx)
                                    })),
                            )
                            .child(
                                EpButton::new("delete-confirm")
                                    .small()
                                    .danger()
                                    .label("删除")
                                    .loading(deleting)
                                    .disabled(counting)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.confirm_delete(window, cx)
                                    })),
                            ),
                    )
                    .into_any_element()
            }
            Panel::OneDrive => div()
                .flex()
                .flex_col()
                .flex_1()
                .gap(px(10.))
                .child(
                    div()
                        .flex_1()
                        .child(
                            div()
                                .mb(px(6.))
                                .flex()
                                .items_center()
                                .gap(px(6.))
                                .font_weight(theme::WEIGHT_SEMIBOLD)
                                .child(icon(EpIcon::InfoFilled, 16., t.text))
                                .child("桌面由 OneDrive 同步"),
                        )
                        .child(
                            div()
                                .text_size(px(12.))
                                .text_color(t.panel_label)
                                .child(
                                    "收进收纳箱的项目会从桌面移走，移出桌面会同步到你的其他设备（它们的桌面上也会少掉这些图标）。 项目仍保存在收纳文件夹里，移回桌面即可恢复。",
                                ),
                        ),
                )
                .child(
                    div().flex().justify_end().child(
                        EpButton::new("onedrive-ok")
                            .small()
                            .primary()
                            .label("知道了")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.set_panel(Panel::None, window, cx)
                            })),
                    ),
                )
                .into_any_element(),
        };
        Some(
            div()
                .id("fence-panel")
                .absolute()
                .inset_0()
                .occlude()
                .flex()
                .flex_col()
                .gap(px(10.))
                .px(px(12.))
                .py(px(10.))
                .overflow_y_scroll()
                .bg(t.panel_bg)
                .child(body)
                .into_any_element(),
        )
    }

    fn render_appearance_panel(
        &mut self,
        fence: &mini_todo_core::db::Fence,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = fence_tokens();
        let alpha = (self.bg_alpha() * 100.).round() as f32;
        let current = fence.bg_color.clone();
        let label = |text: &'static str| {
            div()
                .text_size(px(12.))
                .text_color(t.panel_label)
                .child(text)
        };
        let swatches = div()
            .flex()
            .flex_wrap()
            .gap(px(6.))
            .children(PRESET_BG_COLORS.iter().map(|&(name, value)| {
                let active = theme::is_same_color(&current, value);
                let (r, g, b) = theme::hex_to_rgb_channels(value);
                div()
                    .id(SharedString::from(format!("swatch-{value}")))
                    .size(px(22.))
                    .rounded_full()
                    .border_2()
                    .border_color(if active { t.text } else { t.swatch_border })
                    .bg(theme::rgba_css(r, g, b, 1.))
                    .cursor_pointer()
                    .tooltip(crate::ui::tooltip::native(name))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.set_bg_color(value.to_string(), window, cx)
                    }))
            }))
            .child(self.render_custom_swatch(&current, cx));

        let slider = div()
            .flex()
            .items_center()
            .gap(px(10.))
            .px(px(4.))
            .child(
                div().flex_1().child(
                    EpSlider::new("fence-alpha", alpha)
                        .range(5., 100.)
                        .step(5.)
                        .small()
                        .on_input(cx.listener(|this, v: &f32, _, cx| {
                            this.alpha_preview = Some(f64::from(*v) / 100.);
                            cx.notify();
                        }))
                        .on_change(cx.listener(|this, v: &f32, window, cx| {
                            this.set_bg_alpha(*v, window, cx)
                        })),
                ),
            )
            .child(
                div()
                    .w(px(36.))
                    .text_right()
                    .text_size(px(12.))
                    .text_color(t.panel_value)
                    .child(format!("{}%", alpha as i32)),
            );

        div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .font_weight(theme::WEIGHT_SEMIBOLD)
                    .child("外观")
                    .child(
                        div()
                            .id("appearance-close")
                            .size(px(24.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(theme::radius::SM)
                            .cursor_pointer()
                            .hover(move |s| s.bg(t.button_hover_bg))
                            .tooltip(crate::ui::tooltip::native("关闭"))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.set_panel(Panel::None, window, cx)
                            }))
                            .child(icon(EpIcon::Close, 14., t.button)),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .child(label("背景色"))
                    .child(swatches),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .child(label("透明度"))
                    .child(slider),
            )
            .into_any_element()
    }

    /// 「自定义颜色」圆点：当前色 + 彩虹色环（CSS `conic-gradient` + `soft-light` 混合，这里按 12 段扇形逐段算混合色）
    fn render_custom_swatch(&self, current: &str, cx: &mut Context<Self>) -> impl IntoElement {
        let t = fence_tokens();
        let base = theme::hex_to_rgb_channels(current);
        div()
            .id("swatch-custom")
            .relative()
            .size(px(22.))
            .rounded_full()
            .border_2()
            .border_color(t.swatch_border)
            .overflow_hidden()
            .cursor_pointer()
            .tooltip(crate::ui::tooltip::native("自定义颜色"))
            .on_click(cx.listener(|this, _, window, cx| this.pick_custom_color(window, cx)))
            .child(
                canvas(
                    |_, _, _| {},
                    move |bounds, _, window, _| paint_conic_swatch(bounds, base, window),
                )
                .absolute()
                .inset_0(),
            )
    }
}

/// 色环的色标（`#ef4444, #f59e0b, #10b981, #3b82f6, #8b5cf6, #ef4444`）
const CONIC_STOPS: [(u8, u8, u8); 6] = [
    (0xEF, 0x44, 0x44),
    (0xF5, 0x9E, 0x0B),
    (0x10, 0xB9, 0x81),
    (0x3B, 0x82, 0xF6),
    (0x8B, 0x5C, 0xF6),
    (0xEF, 0x44, 0x44),
];

/// CSS `soft-light`（W3C 合成规范）：底色 `b`、混合色 `s`，0~1
fn soft_light(b: f32, s: f32) -> f32 {
    if s <= 0.5 {
        b - (1. - 2. * s) * b * (1. - b)
    } else {
        let d = if b <= 0.25 {
            ((16. * b - 12.) * b + 4.) * b
        } else {
            b.sqrt()
        };
        b + (2. * s - 1.) * (d - b)
    }
}

/// 色环在角度比例 `t`（0~1，从正上方顺时针）处的颜色
fn conic_color(t: f32) -> (f32, f32, f32) {
    let segments = (CONIC_STOPS.len() - 1) as f32;
    let pos = (t.clamp(0., 1.) * segments).min(segments - 0.0001);
    let i = pos.floor() as usize;
    let f = pos - i as f32;
    let (a, b) = (CONIC_STOPS[i], CONIC_STOPS[i + 1]);
    let lerp = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * f) / 255.;
    (lerp(a.0, b.0), lerp(a.1, b.1), lerp(a.2, b.2))
}

fn paint_conic_swatch(
    bounds: gpui_kit::Bounds<gpui_kit::Pixels>,
    base: (u8, u8, u8),
    window: &mut Window,
) {
    const SLICES: usize = 24;
    let center = bounds.center();
    let radius = f32::from(bounds.size.width.max(bounds.size.height));
    let b = (
        f32::from(base.0) / 255.,
        f32::from(base.1) / 255.,
        f32::from(base.2) / 255.,
    );
    for i in 0..SLICES {
        let t0 = i as f32 / SLICES as f32;
        let t1 = (i + 1) as f32 / SLICES as f32;
        let (r, g, bl) = conic_color((t0 + t1) / 2.);
        let color = theme::rgba_css(
            (soft_light(b.0, r) * 255.).round() as u8,
            (soft_light(b.1, g) * 255.).round() as u8,
            (soft_light(b.2, bl) * 255.).round() as u8,
            1.,
        );
        let at = |t: f32| {
            let angle = t * std::f32::consts::TAU - std::f32::consts::FRAC_PI_2;
            point(
                center.x + px(radius * angle.cos()),
                center.y + px(radius * angle.sin()),
            )
        };
        let mut builder = gpui_kit::PathBuilder::fill();
        builder.move_to(center);
        builder.line_to(at(t0));
        builder.line_to(at(t1));
        builder.close();
        if let Ok(path) = builder.build() {
            window.paint_path(path, color);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn soft_light_on_black_stays_black() {
        // 旧版默认黑色背景时，自定义色圆点仍是黑的（参考截图）
        for i in 0..=10 {
            let s = i as f32 / 10.;
            assert_eq!(soft_light(0., s), 0.);
        }
        assert!((soft_light(1., 0.3) - 1.).abs() < 1e-6);
        assert!((soft_light(0.5, 0.5) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn title_menu_width_follows_the_longest_label() {
        assert_eq!(
            title_menu_width(&["重命名", "外观", "删除收纳箱"]),
            123.,
            "只有三项时（最长「删除收纳箱」）的宽度不变"
        );
        assert_eq!(show_hidden_label(false), "显示隐藏的图标");
        assert_eq!(show_hidden_label(true), "不显示隐藏的图标");
        assert_eq!(
            title_menu_width(&["重命名", "外观", show_hidden_label(false), "删除收纳箱"]),
            151.
        );
        assert_eq!(
            title_menu_width(&["重命名", "外观", show_hidden_label(true), "删除收纳箱"]),
            165.
        );
    }

    #[test]
    fn conic_starts_and_ends_red() {
        let (r, g, b) = conic_color(0.);
        assert!((r - 0xEF as f32 / 255.).abs() < 1e-3 && g < 0.3 && b < 0.3);
        let (r, _, _) = conic_color(1.);
        assert!((r - 0xEF as f32 / 255.).abs() < 1e-2);
    }
}
