//! 自绘标题栏（旧版窗口都是 `decorations: false`）。
//!
//! ## 拖动区
//!
//! [`drag_region`] 是铺满父元素的背景层（标题栏第一个子元素）：
//! - Windows：`window_control_area(Drag)` → GPUI 的 `WM_NCHITTEST` 返回 `HTCAPTION`，由系统拖动（贴边分屏、
//!   可缩放窗口双击最大化 / 还原，与旧版 Tauri `drag.js` 双击行为一致）；
//! - 其它平台：左键按下 `window.start_window_move()`。
//!
//! 拖动区之上的**可交互元素必须 `occlude()`**（[`EpButton`](crate::ui::button::EpButton) 已经是），否则 Windows
//! 上点它会变成拖窗口；纯文字（标题）不用处理，按在上面就是拖动。不可拖动的状态（主窗口固定模式）不放拖动区即可。
//! 收纳箱标题栏不用它：要"移动超过 3px 才拖"（双击改名）、拖动中还要吸附对齐，用的是自绘拖动（GPUI `on_drag_move` +
//! 每帧摆窗，见 `windows/fence/mod.rs` 模块文档）。
//!
//! ## 通用标题栏 [`WindowTitleBar`]
//!
//! 默认是编辑类窗口的样式（`ui-editor-windows.md` §3.4）：`padding 12px 16px`、最小高 44、底边 1px `#E2E8F0`、
//! 白底；标题 16px / 600 / 行高 1.2 / `#1E293B`；右侧按钮区 `gap 2px`，关闭按钮是 `el-button text` + Close 14px
//! （44×32）。设置窗口：`.title_size(17.).padding(12., 20.)`。
//!
//! ```ignore
//! WindowTitleBar::new("editor-title", "待办详情")
//!     .action(EpButton::new("edit").text().primary().icon(EpIcon::Edit).icon_size(14.).label("编辑"))
//!     .on_close(|_, window, _| window.remove_window())
//! ```

use std::rc::Rc;

use gpui_kit::component::StyledExt as _;
use gpui_kit::{
    AnyElement, App, ClickEvent, Div, ElementId, InteractiveElement as _, IntoElement,
    ParentElement as _, RenderOnce, SharedString, StyleRefinement, Styled, Window, div,
    prelude::FluentBuilder as _, px, relative,
};

use crate::assets::EpIcon;
use crate::theme::{self, light};
use crate::ui::button::EpButton;

/// 铺满父元素的拖动背景层（父元素要 `relative()`；放在第一个子元素，内容画在它上面）
pub fn drag_region() -> Div {
    let region = div().absolute().inset_0();
    #[cfg(windows)]
    let region = region.window_control_area(gpui_kit::WindowControlArea::Drag);
    #[cfg(not(windows))]
    let region = region.on_mouse_down(gpui_kit::MouseButton::Left, |_, window, _| {
        window.start_window_move()
    });
    region
}

type CloseHandler = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

/// 通用自绘标题栏：拖动区 + 标题 + 右侧按钮（默认带关闭按钮）
#[derive(IntoElement)]
pub struct WindowTitleBar {
    id: ElementId,
    title: SharedString,
    title_size: f32,
    padding_y: f32,
    padding_x: f32,
    draggable: bool,
    closable: bool,
    actions: Vec<AnyElement>,
    on_close: Option<CloseHandler>,
    style: StyleRefinement,
}

impl WindowTitleBar {
    pub fn new(id: impl Into<ElementId>, title: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            title_size: 16.,
            padding_y: 12.,
            padding_x: 16.,
            draggable: true,
            closable: true,
            actions: Vec::new(),
            on_close: None,
            style: StyleRefinement::default(),
        }
    }

    pub fn title_size(mut self, size: f32) -> Self {
        self.title_size = size;
        self
    }

    /// 上下 / 左右内边距（默认 12 / 16）
    pub fn padding(mut self, y: f32, x: f32) -> Self {
        self.padding_y = y;
        self.padding_x = x;
        self
    }

    pub fn draggable(mut self, draggable: bool) -> Self {
        self.draggable = draggable;
        self
    }

    pub fn closable(mut self, closable: bool) -> Self {
        self.closable = closable;
        self
    }

    /// 右侧按钮区追加一个元素（在关闭按钮之前）
    pub fn action(mut self, action: impl IntoElement) -> Self {
        self.actions.push(action.into_any_element());
        self
    }

    /// 关闭按钮点击（默认 `window.remove_window()`）
    pub fn on_close(mut self, f: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_close = Some(Rc::new(f));
        self
    }
}

impl Styled for WindowTitleBar {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl RenderOnce for WindowTitleBar {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let palette = light();
        let on_close = self.on_close.clone();
        let close = self.closable.then(|| {
            EpButton::new((self.id.clone(), "close"))
                .text()
                .icon(EpIcon::Close)
                .icon_size(14.)
                .on_click(move |event, window, cx| match &on_close {
                    Some(handler) => handler(event, window, cx),
                    None => window.remove_window(),
                })
        });
        div()
            .id(self.id)
            .relative()
            .flex()
            .flex_none()
            .items_center()
            .justify_between()
            .min_h(px(44.))
            .py(px(self.padding_y))
            .px(px(self.padding_x))
            .bg(theme::hex(0xFFFFFF))
            .border_b_1()
            .border_color(palette.border)
            .when(self.draggable, |this| this.child(drag_region()))
            .child(
                div()
                    .text_size(px(self.title_size))
                    .font_weight(theme::WEIGHT_SEMIBOLD)
                    .line_height(relative(1.2))
                    .text_color(palette.text_primary)
                    .truncate()
                    .child(self.title),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(2.))
                    .occlude()
                    .children(self.actions)
                    .children(close),
            )
            .refine_style(&self.style)
    }
}
