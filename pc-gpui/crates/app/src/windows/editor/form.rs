//! 编辑表单（§3.6）与底部栏（§3.7）。
//!
//! `el-form label-position="top"`：每项 `margin-bottom:18px`；label 14px `#606266`、行高 22、下边距 8（必填项前
//! 红色 `*`）；内容区 `flex-wrap`、行高 32。时间选择器宽 220（实测参考截图：日期与时间框都是 EP 默认宽度），
//! 两个之间 8px。

use gpui_kit::{
    AnyElement, ClickEvent, Context, Div, InteractiveElement as _, IntoElement, ParentElement as _,
    SharedString, StatefulInteractiveElement as _, Styled as _, Window, div,
    prelude::FluentBuilder as _, px,
};

use super::{EditorWindow, PRESET_COLORS, QUADRANTS, RepeatType, WEEKDAYS};
use crate::assets::EpIcon;
use crate::theme::{self, editor_theme, ep, hex, light};
use crate::ui::button::EpButton;
use crate::ui::ep_checkbox::EpCheckbox;
use crate::ui::ep_switch::EpSwitch;
use crate::ui::icon::icon;
use crate::ui::input::EpInput;

/// 日期 / 时间框宽度
const PICKER_WIDTH: f32 = 220.;

/// 一个表单项（label 在上）
fn form_item(label: impl IntoElement, content: impl IntoElement) -> Div {
    div()
        .w_full()
        .mb(px(18.))
        .flex()
        .flex_col()
        .child(
            div()
                .mb(px(8.))
                .text_size(px(14.))
                .line_height(px(22.))
                .text_color(ep().text_regular)
                .child(label),
        )
        .child(
            div()
                .w_full()
                .flex()
                .flex_wrap()
                .items_center()
                .text_size(px(14.))
                .line_height(px(32.))
                .child(content),
        )
}

/// 必填项 label：红色 `*` + 文字
fn required_label(text: &'static str) -> impl IntoElement {
    div()
        .flex()
        .child(div().mr(px(4.)).text_color(ep().danger).child("*"))
        .child(text)
}

impl EditorWindow {
    /// 编辑表单（可滚动）
    pub(super) fn render_form(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let _ = window;
        let notify_at = self.form_notify_at(cx);
        let desc_label = self.render_desc_label(cx).into_any_element();
        let description = self.render_description(cx).into_any_element();
        let colors = self.render_colors(cx).into_any_element();
        let quadrants = self.render_quadrants(cx).into_any_element();
        let content = div()
            .p(px(16.))
            .flex()
            .flex_col()
            .child(form_item(
                required_label("标题"),
                EpInput::new(&self.title).w_full(),
            ))
            .child(form_item(desc_label, description))
            .child(form_item("颜色", colors))
            .child(form_item("四象限", quadrants))
            .child(form_item(
                "时间范围",
                div()
                    .w_full()
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .child(picker_row(
                        self.start_date.clone().into_any_element(),
                        self.start_time.clone().into_any_element(),
                    ))
                    .child(picker_row(
                        self.end_date.clone().into_any_element(),
                        self.end_time.clone().into_any_element(),
                    )),
            ))
            .child(form_item(
                if self.repeat_enabled {
                    "首次提醒"
                } else {
                    "提醒时间"
                },
                picker_row(
                    self.notify_date.clone().into_any_element(),
                    self.notify_time.clone().into_any_element(),
                ),
            ))
            .when(notify_at.is_some() && !self.repeat_enabled, |this| {
                this.child(form_item(
                    "提前提醒",
                    div()
                        .w_full()
                        .flex()
                        .flex_col()
                        .child(self.notify_before_select.clone())
                        .when(self.is_custom_notify_before, |this| {
                            this.child(div().mt(px(8.)).child(self.custom_notify_before.clone()))
                        }),
                ))
            })
            .child(form_item("重复", {
                let weak = cx.entity().downgrade();
                EpSwitch::new("editor-repeat")
                    .checked(self.repeat_enabled)
                    .on_change(move |checked: &bool, _, cx| {
                        let checked = *checked;
                        let _ = weak.update(cx, |this, cx| {
                            this.repeat_enabled = checked;
                            cx.notify();
                        });
                    })
            }))
            .when(self.repeat_enabled, |this| {
                this.child(form_item(
                    "重复方式",
                    div().w_full().child(self.repeat_type_select.clone()),
                ))
                .child(form_item(
                    SharedString::from(format!(
                        "每 {} {}",
                        self.repeat_interval_value(cx),
                        self.repeat_type.unit()
                    )),
                    div().w_full().child(self.repeat_interval.clone()),
                ))
                .when(self.repeat_type == RepeatType::Weekly, |this| {
                    this.child(form_item("星期", self.render_weekdays(cx)))
                })
                .when(self.repeat_type == RepeatType::Monthly, |this| {
                    this.child(form_item(
                        "每月几号",
                        div().w_full().child(self.repeat_month_day.clone()),
                    ))
                })
            });
        div()
            .id("editor-form")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&self.form_scroll)
            .child(content)
    }

    /// 描述 label：文字 + 放大编辑按钮
    fn render_desc_label(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = light();
        div().flex().items_center().gap(px(6.)).child("描述").child(
            div()
                .id("editor-desc-expand")
                .group("editor-desc-expand")
                .size(px(22.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(4.))
                .cursor_pointer()
                .hover(|s| s.bg(hex(editor_theme::BLUE_50)))
                .tooltip(crate::ui::tooltip::native("放大编辑"))
                .on_click(
                    cx.listener(|this, _: &ClickEvent, window, cx| {
                        this.open_desc_dialog(window, cx)
                    }),
                )
                .child(
                    icon(EpIcon::FullScreen, 14., palette.text_tertiary)
                        .group_hover("editor-desc-expand", |s| s.text_color(palette.primary)),
                ),
        )
    }

    /// 描述编辑框：1px 边（聚焦 `#3B82F6`）、圆角 8、高 120~280，超出在框内滚动
    fn render_description(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = light();
        let editing = self.description.read(cx).is_editing();
        div()
            .id("editor-desc-box")
            .w_full()
            // 参考截图实测空描述时外框 124 高（Milkdown 内部最小高度叠加的结果）
            .min_h(px(124.))
            .max_h(px(280.))
            .overflow_y_scroll()
            .bg(hex(0xFFFFFF))
            .border_1()
            .border_color(if editing {
                palette.primary
            } else {
                palette.border
            })
            .rounded(px(8.))
            .line_height(gpui_kit::relative(theme::LINE_HEIGHT))
            // flex 列：编辑器根元素撑满这 122 高（点框内任意位置都进入编辑），内容更长时随内容增高、在框内滚动
            .child(
                div()
                    .min_h(px(122.))
                    .flex()
                    .flex_col()
                    .child(self.description.clone()),
            )
    }

    /// 颜色：8 个预设色 + 颜色选择器
    fn render_colors(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = light();
        div()
            .flex()
            .items_center()
            .gap(px(8.))
            .children(PRESET_COLORS.iter().map(|(name, value)| {
                // 激活判断是区分大小写的字符串相等（与旧版一致）
                let active = self.color.as_deref() == Some(*value);
                let color = theme::todo_color(value, palette.success);
                div()
                    .id(SharedString::from(format!("editor-preset-{value}")))
                    .size(px(24.))
                    .rounded(px(4.))
                    .border_2()
                    .border_color(if active {
                        palette.primary
                    } else {
                        gpui_kit::transparent_black()
                    })
                    .when(active, |this| {
                        this.shadow(vec![theme::shadow(
                            0.,
                            0.,
                            0.,
                            2.,
                            theme::rgba_css(59, 130, 246, 0.3),
                        )])
                    })
                    .bg(color)
                    .cursor_pointer()
                    .tooltip(crate::ui::tooltip::native(*name))
                    .on_click(
                        cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.select_color(value, cx)
                        }),
                    )
            }))
            .child(self.color_picker.clone())
    }

    /// 四象限 2×2
    fn render_quadrants(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let button = |index: usize, cx: &mut Context<Self>| {
            let (id, name, color) = QUADRANTS[index];
            let color = hex(color);
            let active = self.quadrant == id;
            div()
                .id(SharedString::from(format!("editor-quadrant-{id}")))
                .flex_1()
                .min_w_0()
                .flex()
                .items_center()
                .gap(px(8.))
                .px(px(12.))
                .py(px(10.))
                // 10% 象限色叠在白底上的不透明等效色（GPUI 的阴影会透过半透明背景显示出来）
                .bg(editor_theme::tint(color, 0.1))
                .border_2()
                .border_color(if active {
                    color
                } else {
                    gpui_kit::transparent_black()
                })
                .rounded(px(8.))
                .cursor_pointer()
                .hover(move |s| s.border_color(color))
                .when(active, |this| {
                    this.shadow(vec![theme::shadow(0., 0., 0., 2., color.opacity(0.3))])
                })
                .on_click(
                    cx.listener(move |this, _: &ClickEvent, _, cx| this.select_quadrant(id, cx)),
                )
                .child(div().flex_none().size(px(10.)).rounded_full().bg(color))
                .child(
                    div()
                        .text_size(px(12.))
                        .line_height(px(32.))
                        .text_color(hex(editor_theme::SLATE_700))
                        .child(name),
                )
        };
        div()
            .w_full()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(
                div()
                    .flex()
                    .gap(px(8.))
                    .child(button(0, cx))
                    .child(button(1, cx)),
            )
            .child(
                div()
                    .flex()
                    .gap(px(8.))
                    .child(button(2, cx))
                    .child(button(3, cx)),
            )
    }

    /// 星期复选框组（`gap 4px 0`，每个右边距 30，窄时换行）
    fn render_weekdays(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .w_full()
            .flex()
            .flex_wrap()
            .gap_y(px(4.))
            .children(WEEKDAYS.iter().enumerate().map(|(index, (day, name))| {
                let day = *day;
                let weak = cx.entity().downgrade();
                EpCheckbox::new(SharedString::from(format!("editor-weekday-{day}")), *name)
                    .checked(self.repeat_weekdays.contains(&day))
                    .last(index + 1 == WEEKDAYS.len())
                    .on_change(move |checked: &bool, _, cx| {
                        let checked = *checked;
                        let _ = weak.update(cx, |this, cx| {
                            this.repeat_weekdays.retain(|d| *d != day);
                            if checked {
                                this.repeat_weekdays.push(day);
                            }
                            cx.notify();
                        });
                    })
            }))
    }

    /// 底部栏：按钮组靠左（`space-between` + 单个子元素），全部 small，间距 6 + 12
    pub(super) fn render_footer(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = light();
        let todo_state = self
            .todo
            .as_ref()
            .filter(|_| self.is_edit())
            .map(|t| t.completed);
        let complete = todo_state.map(|completed| {
            let button = if completed {
                EpButton::new("editor-reopen")
                    .warning()
                    .plain()
                    .icon(EpIcon::RefreshLeft)
                    .label("重新打开")
            } else {
                EpButton::new("editor-complete")
                    .success()
                    .plain()
                    .icon(EpIcon::CircleCheck)
                    .label("完成任务")
            };
            button
                .small()
                .icon_gap(6.)
                .loading(self.updating_complete)
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.update_todo_completed(!completed, window, cx)
                }))
        });
        let save = EpButton::new("editor-save")
            .primary()
            .small()
            .icon_gap(6.)
            .loading(self.saving)
            .disabled(self.saving)
            .label(if self.is_edit() { "保存" } else { "创建" })
            .when(!self.saving, |this| {
                this.icon(if self.is_edit() {
                    EpIcon::Check
                } else {
                    EpIcon::Plus
                })
            })
            .on_click(cx.listener(|this, _, window, cx| this.handle_save(window, cx)));
        div()
            .flex_none()
            .flex()
            .items_center()
            .gap(px(18.))
            .px(px(16.))
            .py(px(10.))
            .border_t_1()
            .border_color(palette.border)
            .children(complete)
            .child(
                EpButton::new("editor-cancel")
                    .small()
                    .icon_gap(6.)
                    .icon(EpIcon::Close)
                    .label("取消")
                    .on_click(|_, window, _| window.remove_window()),
            )
            .child(save)
    }
}

/// 日期框 + 时间框（各 220，间距 8）
fn picker_row(date: AnyElement, time: AnyElement) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .child(div().w(px(PICKER_WIDTH)).flex_none().child(date))
        .child(div().w(px(PICKER_WIDTH)).flex_none().child(time))
}
