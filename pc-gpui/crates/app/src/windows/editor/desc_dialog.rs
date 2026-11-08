//! 描述放大编辑弹窗（§3.14，`el-dialog`）：左源码（等宽 13px / 行高 1.6、底 `#F8FAFC`）+ 1px 分隔 + 右预览
//! （300ms 防抖）。宽 88%、距顶 5vh、圆角 12、遮罩 50% 黑（点遮罩不关）；「最大化」联动窗口最大化（记住进入前
//! 窗口是否已最大化，关闭 / 还原时恢复）。确定写回描述，取消 / X / Esc 丢弃草稿。源码框与描述框一样支持粘贴图片
//! （`ui::markdown_editor::paste_images`：后台保存完插入光标处，预览随防抖刷新；旧版不支持）。
//! 预览里的图片与描述框的渲染结果一样可以拖动手柄调整大小（`ui::markdown_editor::ImageResizer`，改写左边的源码草稿，
//! 预览立刻按改写后的源码刷新、不等防抖）。

use std::rc::Rc;
use std::time::Duration;

use gpui_kit::component::Sizable as _;
use gpui_kit::component::input::{self, InputEvent, Textarea, TextareaState};
use gpui_kit::{
    AppContext as _, ClickEvent, Context, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _, Subscription,
    Task, Window, div, prelude::FluentBuilder as _, px, relative,
};

use super::EditorWindow;
use crate::assets::EpIcon;
use crate::platform;
use crate::theme::{self, editor_theme, ep, hex, light};
use crate::ui::button::EpButton;
use crate::ui::icon::icon;
use crate::ui::markdown::{ImageResizeEvent, MarkdownView};
use crate::ui::markdown_editor::{ImageResizer, ResizeOutcome, TEXTAREA_INSET_X, paste_images};
use crate::ui::tooltip::native;

/// 预览防抖
const PREVIEW_DEBOUNCE: Duration = Duration::from_millis(300);

pub(super) struct DescDialog {
    source: Entity<TextareaState>,
    /// 预览所用的源码（防抖后的草稿）；拖动图片手柄时图片的源码位置按它算
    preview: SharedString,
    /// 预览里拖动手柄调整图片大小
    resizer: ImageResizer,
    maximized: bool,
    /// 进入最大化前窗口是否已最大化
    was_window_maximized: bool,
    _debounce: Option<Task<()>>,
    _subscription: Subscription,
}

impl EditorWindow {
    /// 旧 `openDescDialog()`：草稿 = 当前描述（存储形式）
    pub(super) fn open_desc_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.desc_dialog.is_some() {
            return;
        }
        self.close_popups(window, cx);
        let draft = self.description_value(cx);
        let source = cx.new(|cx| {
            TextareaState::new(window, cx)
                .soft_wrap(true)
                .placeholder("输入 Markdown 源码...")
        });
        source.update(cx, |state, cx| {
            state.set_value(draft.clone(), window, cx);
            state.focus(window, cx);
        });
        let subscription = cx.subscribe_in(
            &source,
            window,
            |this, source, event: &InputEvent, _, cx| {
                if !matches!(event, InputEvent::Change) {
                    return;
                }
                let source = source.clone();
                let task = cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(PREVIEW_DEBOUNCE).await;
                    let _ = this.update(cx, |this, cx| {
                        let text = source.read(cx).value();
                        if let Some(dialog) = this.desc_dialog.as_mut() {
                            dialog.preview = text;
                            cx.notify();
                        }
                    });
                });
                if let Some(dialog) = this.desc_dialog.as_mut() {
                    dialog._debounce = Some(task);
                }
            },
        );
        self.desc_dialog = Some(DescDialog {
            source,
            preview: draft.into(),
            resizer: ImageResizer::default(),
            maximized: false,
            was_window_maximized: false,
            _debounce: None,
            _subscription: subscription,
        });
        cx.notify();
    }

    /// 关闭弹窗（`apply`：确定 = 写回描述）；仍处于最大化时还原窗口
    pub(super) fn close_desc_dialog(
        &mut self,
        apply: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(dialog) = self.desc_dialog.take() else {
            return;
        };
        if apply {
            let value = dialog.source.read(cx).value().to_string();
            self.description
                .update(cx, |editor, cx| editor.set_value(&value, window, cx));
        }
        if dialog.maximized && !dialog.was_window_maximized && window.is_maximized() {
            platform::toggle_maximize(window, cx);
        }
        window.focus(&self.focus, cx);
        cx.notify();
    }

    /// 预览里拖动图片手柄：改写左边的源码草稿；改写后预览立刻用新源码（等防抖的话松手后会闪回旧尺寸 300ms）
    fn on_desc_image_resize(
        &mut self,
        event: ImageResizeEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(dialog) = self.desc_dialog.as_mut() else {
            return;
        };
        let source = dialog.source.clone();
        let rendered = dialog.preview.clone();
        match dialog.resizer.handle(event, &source, &rendered, window, cx) {
            ResizeOutcome::Ignored => {}
            ResizeOutcome::Redraw => cx.notify(),
            ResizeOutcome::Gesture { rewritten } => {
                if rewritten {
                    dialog.preview = source.read(cx).value();
                }
                cx.notify();
            }
        }
    }

    fn toggle_desc_maximize(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(dialog) = self.desc_dialog.as_mut() else {
            return;
        };
        if dialog.maximized {
            dialog.maximized = false;
            if !dialog.was_window_maximized && window.is_maximized() {
                platform::toggle_maximize(window, cx);
            }
        } else {
            dialog.was_window_maximized = window.is_maximized();
            if !dialog.was_window_maximized {
                platform::toggle_maximize(window, cx);
            }
            dialog.maximized = true;
        }
        cx.notify();
    }

    pub(super) fn render_desc_dialog(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let weak = cx.entity().downgrade();
        let Some(resize) = self.desc_dialog.as_mut().map(|dialog| {
            dialog.resizer.prepare(Rc::new(move |event, window, cx| {
                let _ = weak.update(cx, |this, cx| this.on_desc_image_resize(event, window, cx));
            }))
        }) else {
            return div().into_any_element();
        };
        let Some(dialog) = self.desc_dialog.as_ref() else {
            return div().into_any_element();
        };
        let palette = light();
        let ep = ep();
        let viewport = window.viewport_size();
        let (vw, vh) = (f32::from(viewport.width), f32::from(viewport.height));
        let maximized = dialog.maximized;

        let header = div()
            .flex_none()
            .flex()
            .items_center()
            .justify_between()
            // EP 关闭按钮的 32 + 自己的 32
            .pr(px(64.))
            .pb(px(16.))
            .child(
                div()
                    .text_size(px(15.))
                    .font_weight(theme::WEIGHT_SEMIBOLD)
                    .text_color(palette.text_primary)
                    .child("编辑描述"),
            )
            .child(
                div()
                    .id("desc-dialog-max")
                    .group("desc-dialog-max")
                    .size(px(24.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(4.))
                    .cursor_pointer()
                    .hover(|s| s.bg(hex(editor_theme::BLUE_50)))
                    .tooltip(native(if maximized { "还原" } else { "最大化" }))
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                        this.toggle_desc_maximize(window, cx)
                    }))
                    .child(
                        icon(EpIcon::FullScreen, 14., palette.text_tertiary)
                            .group_hover("desc-dialog-max", |s| s.text_color(palette.primary)),
                    ),
            );
        let close = div()
            .id("desc-dialog-close")
            .group("desc-dialog-close")
            .absolute()
            .top_0()
            .right_0()
            .size(px(48.))
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                this.close_desc_dialog(false, window, cx)
            }))
            .child(
                icon(EpIcon::Close, 16., ep.text_secondary)
                    .group_hover("desc-dialog-close", |s| s.text_color(ep.primary)),
            );

        // id 不能与描述框渲染结果的 `editor-desc-preview` 相同（同一个窗口里，TextView 状态与图片手柄的拖动归属都按它区分）
        let preview = MarkdownView::new(
            "editor-desc-dialog-preview",
            dialog.preview.clone(),
            self.images_dir.clone(),
        )
        .readonly(true)
        .image_resize(resize);
        let body = div()
            .w_full()
            .when(maximized, |this| this.flex_1().min_h_0())
            .when(!maximized, |this| this.h(px(vh * 0.6)))
            .flex()
            .border_1()
            .border_color(palette.border)
            .rounded(px(8.))
            .overflow_hidden()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    // 外层保持旧版的 12 / 14 内边距（flex 等分时左右宽度与旧版一致），编辑框用负外边距抵掉
                    // 组件自带的左右 4px（xsmall 尺寸：左右 4、上下 0）
                    .px(px(14.))
                    .py(px(12.))
                    .bg(palette.bg_secondary)
                    .font(editor_theme::mono_font())
                    .text_size(px(13.))
                    .line_height(relative(1.6))
                    .text_color(hex(editor_theme::SLATE_700))
                    .child(
                        div()
                            .h_full()
                            .mx(px(-TEXTAREA_INSET_X))
                            // 粘贴图片：与描述框同一套（捕获阶段先于输入框，见 markdown_editor::paste_images）
                            .capture_action(cx.listener(|this, _: &input::Paste, window, cx| {
                                let Some(source) =
                                    this.desc_dialog.as_ref().map(|d| d.source.clone())
                                else {
                                    return;
                                };
                                if paste_images(&source, window, cx) {
                                    cx.stop_propagation();
                                }
                            }))
                            .child(
                                Textarea::new(&dialog.source)
                                    .appearance(false)
                                    .bordered(false)
                                    .xsmall()
                                    .text_size(px(13.))
                                    .line_height(relative(1.6))
                                    .h_full()
                                    .w_full(),
                            ),
                    ),
            )
            .child(div().flex_none().w(px(1.)).h_full().bg(palette.border))
            .child(
                div()
                    .id("desc-dialog-preview")
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .overflow_y_scroll()
                    .bg(hex(0xFFFFFF))
                    // 内边距放在内层：flex 等分时外层的内边距会算进左右宽度（源码框有 28 的内边距、预览外层没有）
                    .child(div().px(px(16.)).py(px(12.)).child(preview)),
            );
        let footer =
            div()
                .flex_none()
                .pt(px(16.))
                .flex()
                .justify_end()
                .gap(px(12.))
                .child(
                    EpButton::new("desc-dialog-cancel")
                        .small()
                        .label("取消")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.close_desc_dialog(false, window, cx)
                        })),
                )
                .child(
                    EpButton::new("desc-dialog-ok")
                        .primary()
                        .small()
                        .label("确定")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.close_desc_dialog(true, window, cx)
                        })),
                );

        let width = vw * 0.88;
        let panel = div()
            .id("desc-dialog")
            .absolute()
            .when(!maximized, |this| {
                this.left(px((vw - width) / 2.))
                    .top(px(vh * 0.05))
                    .w(px(width))
                    .rounded(px(12.))
            })
            .when(maximized, |this| this.inset_0())
            .flex()
            .flex_col()
            .p(px(16.))
            .bg(hex(0xFFFFFF))
            .shadow(ep.box_shadow())
            .occlude()
            .child(header)
            .child(body)
            .child(footer)
            .child(close);

        div()
            .id("desc-dialog-overlay")
            .absolute()
            .inset_0()
            .occlude()
            .bg(ep.overlay)
            .child(panel)
            .into_any_element()
    }
}
