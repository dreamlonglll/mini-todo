//! `el-select`（单选，Element Plus 2.13 默认样式）：有状态视图，在构造宿主视图时创建、作为子元素放进布局。
//!
//! 值类型是泛型（默认 `SharedString`；编辑窗口的「提前提醒」「重复方式」用 `i32`）。
//!
//! 触发框：默认 `min-height 32`、`padding 4px 12px`、14px；small `min-height 24`、`padding 2px 8px`、12px；白底、
//! 1px `#DCDFE6`（悬停 `#C0C4CC`、展开 `#409EFF`）、圆角 4（main.scss 只把 `el-input__wrapper` 改成 6）；右侧 ArrowDown
//! 14px `#A8ABB2`（展开时旋转 180°），展开时当前文字变成占位色；`clearable` 且有值时悬停换成 CircleClose。
//! `filterable`：展开时框内可输入（占位文字 = 当前选项），按不区分大小写的子串过滤。
//! 下拉层（[`ep_popper`]）：触发框下方 12px、与触发框同宽、浅色弹层 + 箭头，下方放不下时翻到上方；列表 `padding 6px 0`、
//! 最高 274（超出滚动，选项多时虚拟滚动 `uniform_list`）；选项高 34、`padding 0 32px 0 20px`、14px `#606266`，
//! 悬停 / 键盘高亮底 `#F5F7FA`（展开时高亮已选项），选中项 `#409EFF` 加粗；没有选项显示 `No data`、过滤后没有匹配显示
//! `No matching data`（项目没配置 Element Plus 语言，内置文案是英文）。选项可以用自己的字体渲染（字体下拉）。
//! 键盘：↑ / ↓ 移动高亮（未展开时先展开），Enter 选中 / 展开，Esc 收起。
//!
//! ```ignore
//! let fonts = cx.new(|cx| EpSelect::new("font", window, cx).filterable().clearable().placeholder("跟随系统"));
//! cx.subscribe_in(&fonts, window, |this, _, event: &EpSelectEvent, window, cx| match event {
//!     EpSelectEvent::Change(value) => this.set_font(value.clone(), window, cx),
//! }).detach();
//! fonts.update(cx, |s, cx| s.set_options(list, cx));
//! // 非字符串值
//! let kind = cx.new(|cx| EpSelect::<i32>::new("kind", window, cx)
//!     .with_options(vec![EpSelectOption::new(0, "每天"), EpSelectOption::new(1, "每周")]));
//! // render
//! div().w_full().child(self.fonts.clone())
//! ```

use std::ops::Range;

use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::{
    App, AppContext as _, Context, ElementId, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, KeyDownEvent, MouseButton, ParentElement as _, Render,
    SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, Transformation,
    UniformListScrollHandle, Window, div, prelude::FluentBuilder as _, px, uniform_list,
};

use crate::assets::EpIcon;
use crate::theme::{self, ep};
use crate::ui::ep_popper::{self, Anchor, Placement, PopperOptions};
use crate::ui::icon::icon;

/// 选项值的约束
pub trait SelectValue: Clone + PartialEq + 'static {
    /// 当前值不在选项里时触发框显示的文字（字符串值显示它自己，例如系统字体列表还没加载 / 没有该字体时；
    /// 其它类型显示占位文字）
    fn fallback_label(&self) -> Option<SharedString> {
        None
    }
}

impl SelectValue for SharedString {
    fn fallback_label(&self) -> Option<SharedString> {
        Some(self.clone())
    }
}

impl SelectValue for i32 {}
impl SelectValue for i64 {}

/// 一个选项
#[derive(Debug, Clone, PartialEq)]
pub struct EpSelectOption<V = SharedString> {
    pub value: V,
    pub label: SharedString,
    /// 用该字体渲染（字体下拉"每个选项用它自己的字体"）
    pub font: Option<SharedString>,
}

impl<V> EpSelectOption<V> {
    pub fn new(value: impl Into<V>, label: impl Into<SharedString>) -> Self {
        Self {
            value: value.into(),
            label: label.into(),
            font: None,
        }
    }
}

impl EpSelectOption<SharedString> {
    /// 值与文案相同、用自身字体渲染（字体列表）
    pub fn font(name: impl Into<SharedString>) -> Self {
        let name = name.into();
        Self {
            value: name.clone(),
            label: name.clone(),
            font: Some(name),
        }
    }
}

/// 用户改变了选择（`None` = 清空）；宿主用 `set_value` 赋值时不发
#[derive(Debug, Clone, PartialEq)]
pub enum EpSelectEvent<V = SharedString> {
    Change(Option<V>),
}

const ITEM_HEIGHT: f32 = 34.;
const LIST_PADDING: f32 = 6.;
const MAX_HEIGHT: f32 = 274.;

/// 下拉选择
pub struct EpSelect<V: SelectValue = SharedString> {
    id: ElementId,
    options: Vec<EpSelectOption<V>>,
    selected: Option<V>,
    placeholder: SharedString,
    small: bool,
    filterable: bool,
    clearable: bool,
    disabled: bool,
    open: bool,
    hovered: bool,
    /// 高亮行（`visible()` 的下标）：鼠标悬停与键盘共用
    highlighted: Option<usize>,
    anchor: Anchor,
    query: Entity<InputState>,
    focus: FocusHandle,
    scroll: UniformListScrollHandle,
    _subscriptions: Vec<Subscription>,
}

impl<V: SelectValue> EventEmitter<EpSelectEvent<V>> for EpSelect<V> {}

impl<V: SelectValue> Focusable for EpSelect<V> {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

/// 过滤：不区分大小写的子串匹配（`el-select filterable` 的默认行为）
pub(crate) fn filter_options<V>(options: &[EpSelectOption<V>], query: &str) -> Vec<usize> {
    let query = query.trim().to_lowercase();
    options
        .iter()
        .enumerate()
        .filter(|(_, o)| query.is_empty() || o.label.to_lowercase().contains(&query))
        .map(|(i, _)| i)
        .collect()
}

impl<V: SelectValue> EpSelect<V> {
    pub fn new(id: impl Into<ElementId>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let query = cx.new(|cx| InputState::new(window, cx));
        let subscriptions =
            vec![
                cx.subscribe_in(&query, window, |this, _, event: &InputEvent, window, cx| {
                    match event {
                        InputEvent::Change => {
                            this.highlighted = None;
                            this.scroll.scroll_to_item(0, gpui_kit::ScrollStrategy::Top);
                            cx.notify();
                        }
                        InputEvent::PressEnter { .. } => this.confirm_highlighted(window, cx),
                        _ => {}
                    }
                }),
            ];
        Self {
            id: id.into(),
            options: Vec::new(),
            selected: None,
            placeholder: SharedString::default(),
            small: false,
            filterable: false,
            clearable: false,
            disabled: false,
            open: false,
            hovered: false,
            highlighted: None,
            anchor: Anchor::new(),
            query,
            focus: cx.focus_handle(),
            scroll: UniformListScrollHandle::new(),
            _subscriptions: subscriptions,
        }
    }

    pub fn placeholder(mut self, text: impl Into<SharedString>) -> Self {
        self.placeholder = text.into();
        self
    }

    pub fn small(mut self) -> Self {
        self.small = true;
        self
    }

    pub fn filterable(mut self) -> Self {
        self.filterable = true;
        self
    }

    pub fn clearable(mut self) -> Self {
        self.clearable = true;
        self
    }

    pub fn with_options(mut self, options: Vec<EpSelectOption<V>>) -> Self {
        self.options = options;
        self
    }

    pub fn set_options(&mut self, options: Vec<EpSelectOption<V>>, cx: &mut Context<Self>) {
        self.options = options;
        self.highlighted = None;
        cx.notify();
    }

    /// 设置当前值（不发事件）
    pub fn set_value(&mut self, value: Option<V>, cx: &mut Context<Self>) {
        if self.selected != value {
            self.selected = value;
            cx.notify();
        }
    }

    pub fn value(&self) -> Option<&V> {
        self.selected.as_ref()
    }

    pub fn set_disabled(&mut self, disabled: bool, cx: &mut Context<Self>) {
        self.disabled = disabled;
        if disabled {
            self.open = false;
        }
        cx.notify();
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    /// 收起（宿主处理 Esc 时用）
    pub fn close(&mut self, cx: &mut Context<Self>) {
        if self.open {
            self.open = false;
            self.highlighted = None;
            cx.notify();
        }
    }

    fn selected_label(&self) -> Option<SharedString> {
        let value = self.selected.as_ref()?;
        self.options
            .iter()
            .find(|o| &o.value == value)
            .map(|o| o.label.clone())
            .or_else(|| value.fallback_label())
    }

    fn visible(&self, cx: &App) -> Vec<usize> {
        if self.filterable && self.open {
            filter_options(&self.options, &self.query.read(cx).value())
        } else {
            (0..self.options.len()).collect()
        }
    }

    fn set_open(&mut self, open: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.open == open {
            return;
        }
        self.open = open;
        self.highlighted = None;
        if open {
            if self.filterable {
                let placeholder = self
                    .selected_label()
                    .unwrap_or_else(|| self.placeholder.clone());
                self.query.update(cx, |s, cx| {
                    s.set_value("", window, cx);
                    s.set_placeholder(placeholder, window, cx);
                    s.focus(window, cx);
                });
            } else {
                self.focus.focus(window, cx);
            }
            // 展开时高亮并滚到已选项（EP：已选项 is-hovering）
            if let Some(index) = self
                .selected
                .as_ref()
                .and_then(|v| self.options.iter().position(|o| &o.value == v))
            {
                self.highlighted = Some(index);
                self.scroll
                    .scroll_to_item(index, gpui_kit::ScrollStrategy::Center);
            }
        }
        cx.notify();
    }

    fn choose(&mut self, value: Option<V>, window: &mut Window, cx: &mut Context<Self>) {
        self.set_open(false, window, cx);
        if self.selected != value {
            self.selected = value.clone();
            cx.emit(EpSelectEvent::Change(value));
        }
        cx.notify();
    }

    fn confirm_highlighted(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.open {
            return;
        }
        let visible = self.visible(cx);
        if let Some(&index) = self.highlighted.and_then(|h| visible.get(h)) {
            let value = self.options[index].value.clone();
            self.choose(Some(value), window, cx);
        }
    }

    fn on_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.disabled {
            return;
        }
        let key = event.keystroke.key.as_str();
        match key {
            "escape" if self.open => {
                self.set_open(false, window, cx);
                cx.stop_propagation();
            }
            "down" | "up" => {
                if !self.open {
                    self.set_open(true, window, cx);
                }
                let count = self.visible(cx).len();
                if count > 0 {
                    let next = match (self.highlighted, key) {
                        (None, "down") => 0,
                        (None, _) => count - 1,
                        (Some(h), "down") => (h + 1) % count,
                        (Some(h), _) => (h + count - 1) % count,
                    };
                    self.highlighted = Some(next);
                    self.scroll
                        .scroll_to_item(next, gpui_kit::ScrollStrategy::Nearest);
                }
                cx.stop_propagation();
                cx.notify();
            }
            "enter" if self.open && !self.filterable => {
                self.confirm_highlighted(window, cx);
                cx.stop_propagation();
            }
            "enter" | "space" if !self.open && !self.filterable => {
                self.set_open(true, window, cx);
                cx.stop_propagation();
            }
            _ => {}
        }
    }

    fn render_item(&self, index: usize, row: usize, cx: &mut Context<Self>) -> impl IntoElement {
        let ep = ep();
        let option = &self.options[index];
        let selected = self.selected.as_ref() == Some(&option.value);
        let highlighted = self.highlighted == Some(row);
        let value = option.value.clone();
        div()
            .id(("opt", index))
            .w_full()
            .h(px(ITEM_HEIGHT))
            .flex()
            .items_center()
            .pl(px(20.))
            .pr(px(32.))
            .text_size(px(14.))
            .whitespace_nowrap()
            .overflow_hidden()
            .text_ellipsis()
            .cursor_pointer()
            .text_color(if selected {
                ep.primary
            } else {
                ep.text_regular
            })
            .when(selected, |this| {
                this.font_weight(gpui_kit::FontWeight::BOLD)
            })
            .when(highlighted, |this| this.bg(ep.fill_light))
            .when_some(option.font.clone(), |this, font| this.font_family(font))
            .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                if *hovered && this.highlighted != Some(row) {
                    this.highlighted = Some(row);
                    cx.notify();
                }
            }))
            .on_click(cx.listener(move |this, _, window, cx| {
                this.choose(Some(value.clone()), window, cx);
            }))
            .child(option.label.clone())
    }

    fn render_dropdown(&self, window: &Window, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        if !self.open {
            return None;
        }
        let bounds = self.anchor.bounds()?;
        let ep = ep();
        let visible = self.visible(cx);
        let width = f32::from(bounds.size.width);
        let font_size = if self.small { 12. } else { 14. };
        let (body, body_height) = if visible.is_empty() {
            let text = if self.options.is_empty() {
                "No data"
            } else {
                "No matching data"
            };
            let body = div()
                .py(px(10.))
                .w_full()
                .text_center()
                .text_size(px(font_size))
                .text_color(ep.text_secondary)
                .child(text)
                .into_any_element();
            (body, 20. + font_size * 1.5)
        } else {
            let height = (visible.len() as f32 * ITEM_HEIGHT).min(MAX_HEIGHT - LIST_PADDING * 2.);
            let rows = visible.clone();
            let body = div()
                .py(px(LIST_PADDING))
                .child(
                    uniform_list(
                        (self.id.clone(), "list"),
                        rows.len(),
                        cx.processor(move |this, range: Range<usize>, _window, cx| {
                            range
                                .map(|row| this.render_item(rows[row], row, cx).into_any_element())
                                .collect::<Vec<_>>()
                        }),
                    )
                    .track_scroll(&self.scroll)
                    .h(px(height))
                    .w_full(),
                )
                .into_any_element();
            (body, height + LIST_PADDING * 2.)
        };
        let panel = ep_popper::frame()
            .id((self.id.clone(), "dropdown"))
            .w(px(width))
            .font(theme::ui_font())
            .child(body);
        let entity = cx.entity().downgrade();
        ep_popper::popper(
            &self.anchor,
            window,
            PopperOptions {
                placement: Placement::BottomStart,
                flip: true,
                gap: ep_popper::OFFSET,
                arrow: true,
                width,
                height: body_height + 2.,
            },
            panel,
            move |_, cx| {
                let _ = entity.update(cx, |this, cx| this.close(cx));
            },
        )
    }
}

impl<V: SelectValue> Render for EpSelect<V> {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let ep = ep();
        // EP 的边框是 `box-shadow: 0 0 0 1px inset`（不占布局），这里用真实 1px 边框，上下内边距各减 1
        let (min_h, pad_y, pad_x, font, gap, line_h) = if self.small {
            (24., 1., 8., 12., 4., 20.)
        } else {
            (32., 3., 12., 14., 6., 24.)
        };
        let open = self.open;
        let show_clear =
            self.clearable && self.hovered && self.selected.is_some() && !self.disabled;
        let border = if self.disabled {
            ep.disabled_border
        } else if open {
            ep.primary
        } else if self.hovered {
            ep.border_hover
        } else {
            ep.border
        };
        let label = self.selected_label();
        let content = if self.filterable && open {
            div()
                .flex_1()
                .min_w_0()
                .child(
                    Input::new(&self.query)
                        .appearance(false)
                        .px(px(0.))
                        .py(px(0.))
                        .h(px(line_h))
                        .text_size(px(font)),
                )
                .into_any_element()
        } else {
            // 展开时当前文字变成占位色（EP `is-transparent`）
            let (text, color) = match &label {
                Some(label) if !open => (label.clone(), ep.text_regular),
                Some(label) => (label.clone(), ep.text_placeholder),
                None => (self.placeholder.clone(), ep.text_placeholder),
            };
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_color(if self.disabled {
                    ep.disabled_text
                } else {
                    color
                })
                .child(text)
                .into_any_element()
        };
        let suffix = if show_clear {
            div()
                .id("clear")
                .flex()
                .items_center()
                .cursor_pointer()
                .text_color(ep.text_placeholder)
                .hover(move |s| s.text_color(ep.text_secondary))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(|this, _, window, cx| {
                    cx.stop_propagation();
                    this.choose(None, window, cx);
                }))
                .child(icon(EpIcon::CircleClose, font, ep.text_placeholder))
                .into_any_element()
        } else {
            icon(EpIcon::ArrowDown, 14., ep.text_placeholder)
                .with_transformation(Transformation::rotate(gpui_kit::radians(if open {
                    std::f32::consts::PI
                } else {
                    0.
                })))
                .into_any_element()
        };
        let disabled = self.disabled;

        div()
            .id(self.id.clone())
            .key_context("EpSelect")
            .track_focus(&self.focus)
            .capture_key_down(
                cx.listener(|this, event: &KeyDownEvent, window, cx| {
                    this.on_key(event, window, cx)
                }),
            )
            .relative()
            .flex()
            .items_center()
            .gap(px(gap))
            .w_full()
            .min_h(px(min_h))
            .px(px(pad_x))
            .py(px(pad_y))
            .text_size(px(font))
            .line_height(px(line_h))
            .bg(if disabled { ep.fill_light } else { ep.white })
            .border_1()
            .border_color(border)
            .rounded(theme::radius::SM)
            .when(disabled, |this| {
                this.cursor(gpui_kit::CursorStyle::OperationNotAllowed)
            })
            .when(!disabled, |this| {
                this.cursor(if self.filterable {
                    gpui_kit::CursorStyle::IBeam
                } else {
                    gpui_kit::CursorStyle::PointingHand
                })
                .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                    this.hovered = *hovered;
                    cx.notify();
                }))
                // 按下即切换（弹层的点外关闭也在按下时判断，触发框上的按下不算"外面"）
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, window, cx| {
                        let open = !this.open;
                        this.set_open(open, window, cx);
                        cx.stop_propagation();
                    }),
                )
            })
            .child(self.anchor.measure())
            .child(content)
            .child(suffix)
            .children(self.render_dropdown(window, cx))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_is_case_insensitive_substring() {
        let options = vec![
            EpSelectOption::font("Microsoft YaHei"),
            EpSelectOption::font("SimHei"),
            EpSelectOption::font("Segoe UI"),
        ];
        assert_eq!(filter_options(&options, ""), vec![0, 1, 2]);
        assert_eq!(filter_options(&options, "hei"), vec![0, 1]);
        assert_eq!(filter_options(&options, "SEGOE"), vec![2]);
        assert!(filter_options(&options, "宋体").is_empty());
    }

    #[test]
    fn options_accept_non_string_values() {
        let options: Vec<EpSelectOption<i32>> = vec![
            EpSelectOption::new(0, "每天"),
            EpSelectOption::new(1, "每周"),
        ];
        assert_eq!(options[1].value, 1);
        assert_eq!(filter_options(&options, "周"), vec![1]);
    }
}
