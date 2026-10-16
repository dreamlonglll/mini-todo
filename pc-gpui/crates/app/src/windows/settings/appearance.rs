//! 「外观」面板（§2.8）：待办字体（可搜索、可清空，每项用自身字体渲染）、字体大小（12~20，拖动时预览、松手保存）、预览框。
//! 清空字体 = 恢复默认字体（旧版缺陷 §2.13-3 按预期行为修好）。保存失败只记日志（与旧版相同）。

use gpui_kit::{
    Context, IntoElement, ParentElement as _, SharedString, Styled as _, Window, div, px,
};

use super::{SettingsWindow, form_label, panel_title};
use crate::theme::{self, settings_tokens};
use crate::ui::ep_select::EpSelectOption;
use crate::ui::ep_slider::EpSlider;

impl SettingsWindow {
    pub(super) fn load_system_fonts(&mut self, cx: &mut Context<Self>) {
        let task = if cfg!(target_os = "macos") {
            // macOS：core 的 DirectWrite 枚举只有 Windows 有；用 GPUI 的 CoreText 枚举（已排序去重）
            let text_system = cx.text_system().clone();
            cx.background_executor()
                .spawn(async move { Ok(visible_font_names(text_system.all_font_names())) })
        } else {
            cx.background_executor()
                .spawn(async { mini_todo_core::settings::system_fonts() })
        };
        let select = self.font_select.clone();
        cx.spawn(async move |_, cx| {
            let fonts = task.await.unwrap_or_else(|e| {
                log::warn!("[settings] 读取系统字体失败: {e}");
                Vec::new()
            });
            select.update(cx, |select, cx| {
                select.set_options(fonts.into_iter().map(EpSelectOption::font).collect(), cx)
            });
        })
        .detach();
    }

    pub(super) fn set_font_family(
        &mut self,
        value: Option<SharedString>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let family = value.map(|v| v.to_string()).unwrap_or_default();
        let size = self.state.read(cx).todo_font_size;
        let task = self
            .state
            .update(cx, |state, cx| state.set_todo_font(family, size, cx));
        cx.spawn(async move |_, _| {
            if let Err(e) = task.await {
                log::warn!("[settings] 保存待办字体失败: {e}");
            }
        })
        .detach();
    }

    fn set_font_size(&mut self, size: i32, cx: &mut Context<Self>) {
        let family = self.state.read(cx).todo_font_family.clone();
        self.font_size_preview = None;
        let task = self
            .state
            .update(cx, |state, cx| state.set_todo_font(family, size, cx));
        cx.spawn(async move |_, _| {
            if let Err(e) = task.await {
                log::warn!("[settings] 保存待办字号失败: {e}");
            }
        })
        .detach();
    }

    pub(super) fn render_appearance(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let t = settings_tokens();
        let state = self.state.read(cx);
        let size = self.font_size_preview.unwrap_or(state.todo_font_size);
        let family = state.todo_font_family.clone();

        div()
            .flex()
            .flex_col()
            .child(panel_title("外观"))
            .child(
                div()
                    .mb(px(20.))
                    .child(form_label().child("待办字体"))
                    .child(div().w_full().child(self.font_select.clone())),
            )
            .child(
                div()
                    .mb(px(20.))
                    .child(
                        form_label().child("字体大小").child(
                            div()
                                .text_size(px(12.))
                                .font_weight(theme::WEIGHT_SEMIBOLD)
                                .text_color(t.primary)
                                .child(format!("{size}px")),
                        ),
                    )
                    .child(
                        EpSlider::new("font-size", size as f32)
                            .range(12., 20.)
                            .step(1.)
                            .on_input(cx.listener(|this, v: &f32, _, cx| {
                                this.font_size_preview = Some(v.round() as i32);
                                cx.notify();
                            }))
                            .on_change(cx.listener(|this, v: &f32, _, cx| {
                                this.set_font_size(v.round() as i32, cx)
                            })),
                    ),
            )
            .child(
                div()
                    .p(px(16.))
                    .bg(t.surface)
                    .border_1()
                    .border_color(t.border)
                    .rounded(theme::radius::MD)
                    .text_color(t.label)
                    .font(theme::todo_font(&family))
                    .text_size(px(size as f32))
                    .child("待办事项预览 Todo Preview 1234"),
            )
    }
}

/// 去掉 `.` 开头的系统私有字体族（`.AppleSystemUIFont` 等：不能按名字选用，列出来也没意义）
fn visible_font_names(names: Vec<String>) -> Vec<String> {
    names
        .into_iter()
        .filter(|name| !name.is_empty() && !name.starts_with('.'))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hidden_system_fonts_are_not_listed() {
        let names = vec![
            ".AppleSystemUIFont".to_string(),
            "Menlo".to_string(),
            String::new(),
            "PingFang SC".to_string(),
        ];
        assert_eq!(visible_font_names(names), vec!["Menlo", "PingFang SC"]);
    }
}
