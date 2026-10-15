//! `el-select`（Element Plus 2.13，单选、无搜索）。
//!
//! 触发器：最小高 32、`padding 4px 12px`、14px、1px `#DCDFE6` 边（hover `#C0C4CC`、展开 `#409EFF`）、圆角 4
//! （main.scss 只把 `el-input__wrapper` 改成 6，`el-select__wrapper` 仍是 4）；右侧 ArrowDown 14px `#A8ABB2`，
//! 展开时旋转 180°、当前文字变成占位色。下拉：浅色弹层、宽同触发器、`padding 6px 0`；选项高 34、
//! `padding 0 32px 0 20px`、14px `#606266`，悬停底 `#F5F7FA`，选中字 `#409EFF` 加粗。默认在下方，放不下时翻到上方。

use gpui_kit::{
    ClickEvent, Context, EventEmitter, FontWeight, InteractiveElement as _, IntoElement,
    MouseButton, ParentElement as _, Render, SharedString, StatefulInteractiveElement as _,
    Styled as _, Transformation, Window, div, prelude::FluentBuilder as _, px,
};

use crate::assets::EpIcon;
use crate::theme::{self, ep};
use crate::ui::ep2_popper::{self, Anchor, Placement, PopperOptions};
use crate::ui::icon::icon;

/// 选中了某个值
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EpSelectEvent {
    Change(i32),
}

const ITEM_HEIGHT: f32 = 34.;
const LIST_PADDING: f32 = 6.;

pub struct EpSelect {
    id: SharedString,
    options: Vec<(i32, SharedString)>,
    value: Option<i32>,
    open: bool,
    hovered: Option<usize>,
    anchor: Anchor,
}

impl EventEmitter<EpSelectEvent> for EpSelect {}

impl EpSelect {
    pub fn new(id: impl Into<SharedString>, options: Vec<(i32, SharedString)>) -> Self {
        Self {
            id: id.into(),
            options,
            value: None,
            open: false,
            hovered: None,
            anchor: Anchor::new(),
        }
    }

    pub fn value(&self) -> Option<i32> {
        self.value
    }

    /// 外部赋值（不发事件）
    pub fn set_value(&mut self, value: Option<i32>, cx: &mut Context<Self>) {
        if self.value != value {
            self.value = value;
            cx.notify();
        }
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn close(&mut self, cx: &mut Context<Self>) {
        if self.open {
            self.open = false;
            cx.notify();
        }
    }

    fn toggle(&mut self, cx: &mut Context<Self>) {
        self.open = !self.open;
        self.hovered = self
            .value
            .and_then(|v| self.options.iter().position(|(o, _)| *o == v));
        cx.notify();
    }

    fn label(&self) -> Option<SharedString> {
        let value = self.value?;
        self.options
            .iter()
            .find(|(v, _)| *v == value)
            .map(|(_, l)| l.clone())
    }

    fn render_list(&self, width: f32, cx: &mut Context<Self>) -> impl IntoElement {
        let ep = ep();
        let items = self.options.iter().enumerate().map(|(ix, (value, label))| {
            let selected = self.value == Some(*value);
            let hovered = self.hovered == Some(ix);
            let value = *value;
            div()
                .id(SharedString::from(format!("{}-opt-{ix}", self.id)))
                .h(px(ITEM_HEIGHT))
                .pl(px(20.))
                .pr(px(32.))
                .flex()
                .items_center()
                .text_size(px(14.))
                .cursor_pointer()
                .text_color(if selected {
                    ep.primary
                } else {
                    ep.text_regular
                })
                .when(selected, |this| this.font_weight(FontWeight::BOLD))
                .when(hovered, |this| this.bg(ep.fill_light))
                .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                    if *hovered && this.hovered != Some(ix) {
                        this.hovered = Some(ix);
                        cx.notify();
                    }
                }))
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.open = false;
                    if this.value != Some(value) {
                        this.value = Some(value);
                        cx.emit(EpSelectEvent::Change(value));
                    }
                    cx.notify();
                }))
                .child(label.clone())
        });
        ep2_popper::frame()
            .w(px(width))
            .py(px(LIST_PADDING))
            .font(theme::ui_font())
            .children(items)
    }
}

impl Render for EpSelect {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let ep = ep();
        let open = self.open;
        let label = self.label();
        let caret = icon(EpIcon::ArrowDown, 14., ep.text_placeholder).when(open, |this| {
            this.with_transformation(Transformation::rotate(gpui_kit::radians(
                std::f32::consts::PI,
            )))
        });
        let popper = if open {
            let width = self
                .anchor
                .bounds()
                .map_or(200., |b| f32::from(b.size.width));
            let height = LIST_PADDING * 2. + ITEM_HEIGHT * self.options.len() as f32 + 2.;
            let entity = cx.entity().downgrade();
            ep2_popper::popper(
                &self.anchor,
                window,
                PopperOptions {
                    placement: Placement::BottomStart,
                    flip: true,
                    gap: 12.,
                    arrow: true,
                    width,
                    height,
                },
                self.render_list(width, cx),
                move |_, cx| {
                    let _ = entity.update(cx, |this, cx| this.close(cx));
                },
            )
        } else {
            None
        };
        div()
            .id(SharedString::from(format!("{}-trigger", self.id)))
            .relative()
            .w_full()
            .min_h(px(32.))
            .px(px(12.))
            .py(px(4.))
            .flex()
            .items_center()
            .gap(px(6.))
            .bg(ep.white)
            .border_1()
            .border_color(if open { ep.primary } else { ep.border })
            .rounded(theme::radius::SM)
            .cursor_pointer()
            .when(!open, |this| {
                this.hover(|s| s.border_color(ep.border_hover))
            })
            .child(self.anchor.measure())
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.toggle(cx);
                    cx.stop_propagation();
                }),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_size(px(14.))
                    .line_height(px(22.))
                    .truncate()
                    .text_color(if open {
                        ep.text_placeholder
                    } else {
                        ep.text_regular
                    })
                    .children(label),
            )
            .child(caret)
            .children(popper)
    }
}
