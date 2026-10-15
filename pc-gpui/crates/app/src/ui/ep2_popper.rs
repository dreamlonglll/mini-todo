//! Element Plus 浅色弹层（`el-popper is-light`）：白底、1px `#E4E7ED` 边、圆角 4、`box-shadow-light`，
//! 可选 10×10 旋转方块箭头（外露 5px）。下拉、日期 / 时间面板、颜色面板、下拉菜单共用。
//!
//! 用法：触发元素里放一个 [`Anchor::measure`]（记录触发元素的窗口坐标），打开时在 render 里追加
//! [`popper`]（`deferred` + `anchored`，画在窗口最上层）。弹层按触发元素位置定位：`placement` 优先，
//! 放不下且另一侧放得下时翻转（`flip`）；点弹层与触发元素之外的地方调用 `on_dismiss`（不拦截这次点击，
//! 与 EP 的 click-outside 一致）。
//!
//! 触发元素的位置取上一帧量到的值（GPUI 在 render 时还没有布局）；位置变化时量取元素会请求重绘，
//! 下一帧就跟上（滚动表单时弹层跟着走）。

use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::{
    AnyElement, App, Bounds, InteractiveElement as _, IntoElement, ParentElement as _, Pixels,
    Styled as _, Window, canvas, deferred, div, point, prelude::FluentBuilder as _, px,
};

use crate::assets::AppIcon;
use crate::theme::{self, ep};
use crate::ui::icon::icon;

/// 触发元素的窗口坐标（上一帧）
#[derive(Clone, Default)]
pub struct Anchor(Rc<Cell<Option<Bounds<Pixels>>>>);

impl Anchor {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn bounds(&self) -> Option<Bounds<Pixels>> {
        self.0.get()
    }

    /// 放进触发元素（父元素要 `relative()`）：铺满父元素的透明量取层
    pub fn measure(&self) -> impl IntoElement {
        let cell = self.0.clone();
        canvas(
            move |bounds, window, _| {
                if cell.get() != Some(bounds) {
                    let first = cell.get().is_none();
                    cell.set(Some(bounds));
                    if !first {
                        window.refresh();
                    }
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .inset_0()
    }

    /// 点在触发元素上
    pub fn contains(&self, position: gpui_kit::Point<Pixels>) -> bool {
        self.bounds().is_some_and(|b| b.contains(&position))
    }
}

/// 弹出方向
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placement {
    /// 上方、左对齐（日期 / 时间面板：`top-start`，不翻转）
    TopStart,
    /// 下方、左对齐（下拉选择 `bottom-start`）
    BottomStart,
    /// 下方、右对齐（下拉菜单 `bottom-end`）
    BottomEnd,
    /// 下方、水平居中（颜色面板 `bottom`）
    Bottom,
}

impl Placement {
    fn is_top(self) -> bool {
        matches!(self, Placement::TopStart)
    }
}

/// 弹层参数
pub struct PopperOptions {
    pub placement: Placement,
    /// 放不下时翻到另一侧
    pub flip: bool,
    /// 与触发元素的间距（EP 默认 12：含箭头外露的 5px）
    pub gap: f32,
    pub arrow: bool,
    /// 弹层宽高（定位与翻转用；内容自己撑开时给估计值即可）
    pub width: f32,
    pub height: f32,
}

/// 箭头外露的高度
const ARROW_OUT: f32 = 5.;

/// 计算弹层左上角（窗口坐标）、是否在上方、箭头相对弹层左边的中心 x
pub fn place(
    anchor: Bounds<Pixels>,
    viewport: gpui_kit::Size<Pixels>,
    options: &PopperOptions,
) -> (f32, f32, bool, f32) {
    let a_left = f32::from(anchor.origin.x);
    let a_top = f32::from(anchor.origin.y);
    let a_w = f32::from(anchor.size.width);
    let a_h = f32::from(anchor.size.height);
    let vw = f32::from(viewport.width);
    let vh = f32::from(viewport.height);
    let below_y = a_top + a_h + options.gap;
    let above_y = a_top - options.gap - options.height;
    let fits_below = below_y + options.height <= vh;
    let fits_above = above_y >= 0.;
    let top = if options.placement.is_top() {
        !(options.flip && !fits_above && fits_below)
    } else {
        options.flip && !fits_below && fits_above
    };
    let y = if top { above_y } else { below_y };
    let x = match options.placement {
        Placement::TopStart | Placement::BottomStart => a_left,
        Placement::BottomEnd => a_left + a_w - options.width,
        Placement::Bottom => a_left + (a_w - options.width) / 2.,
    };
    let x = x.min(vw - options.width).max(0.);
    let arrow_x = (a_left + a_w / 2. - x).clamp(14., (options.width - 14.).max(14.));
    (x, y, top, arrow_x)
}

/// 浅色弹层外框（不含定位）
pub fn frame() -> gpui_kit::Div {
    let ep = ep();
    div()
        .bg(ep.white)
        .border_1()
        .border_color(ep.border_light)
        .rounded(theme::radius::SM)
        .shadow(ep.box_shadow_light())
}

/// 箭头（10×10 旋转方块的外露部分：灰色描边三角 + 白色内三角）
fn arrow(top: bool, arrow_x: f32) -> impl IntoElement {
    let ep = ep();
    let (outer, inner) = if top {
        (AppIcon::ArrowDownTip, AppIcon::ArrowDownTip)
    } else {
        (AppIcon::ArrowUpTip, AppIcon::ArrowUpTip)
    };
    let left = px(arrow_x - 5.);
    div()
        .absolute()
        .left(left)
        .w(px(10.))
        .h(px(ARROW_OUT + 1.))
        .when(top, |this| this.bottom(px(-ARROW_OUT - 1.)))
        .when(!top, |this| this.top(px(-ARROW_OUT - 1.)))
        .child(
            div()
                .absolute()
                .left_0()
                .when(top, |this| this.top(px(1.)))
                .when(!top, |this| this.top_0())
                .child(
                    icon(outer, 10., ep.border_light)
                        .w(px(10.))
                        .h(px(ARROW_OUT)),
                ),
        )
        .child(
            div()
                .absolute()
                .left(px(1.))
                .when(top, |this| this.top_0())
                .when(!top, |this| this.top(px(1.5)))
                .child(icon(inner, 8., ep.white).w(px(8.)).h(px(ARROW_OUT - 1.))),
        )
}

/// 打开的弹层：`content` 已含外框（[`frame`]）；点外面（且不在触发元素上）调用 `on_dismiss`
pub fn popper(
    anchor: &Anchor,
    window: &Window,
    options: PopperOptions,
    content: impl IntoElement,
    on_dismiss: impl Fn(&mut Window, &mut App) + 'static,
) -> Option<AnyElement> {
    let bounds = anchor.bounds()?;
    let (x, y, top, arrow_x) = place(bounds, window.viewport_size(), &options);
    let anchor = anchor.clone();
    let panel = div()
        .relative()
        .occlude()
        .on_mouse_down_out(move |event, window, cx| {
            if !anchor.contains(event.position) {
                on_dismiss(window, cx);
            }
        })
        .child(content)
        .when(options.arrow, |this| this.child(arrow(top, arrow_x)));
    Some(
        deferred(
            // 位置已按视口夹好；贴边时不要让 GPUI 把锚点换到另一侧（默认的 SwitchAnchor 会整体翻到左边）
            gpui_kit::anchored()
                .position_mode(gpui_kit::AnchoredPositionMode::Window)
                .snap_to_window()
                .position(point(px(x), px(y)))
                .child(panel),
        )
        .with_priority(2)
        .into_any_element(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::size;

    fn bounds(x: f32, y: f32, w: f32, h: f32) -> Bounds<Pixels> {
        Bounds::new(point(px(x), px(y)), size(px(w), px(h)))
    }

    #[test]
    fn top_start_stays_above_and_aligns_left() {
        let options = PopperOptions {
            placement: Placement::TopStart,
            flip: false,
            gap: 12.,
            arrow: true,
            width: 324.,
            height: 342.,
        };
        let (x, y, top, arrow_x) = place(
            bounds(16., 421., 220., 32.),
            size(px(1080.), px(600.)),
            &options,
        );
        assert_eq!((x, y, top), (16., 421. - 12. - 342., true));
        assert_eq!(arrow_x, 110.);
    }

    #[test]
    fn bottom_flips_up_when_there_is_no_room() {
        let options = PopperOptions {
            placement: Placement::BottomStart,
            flip: true,
            gap: 12.,
            arrow: true,
            width: 668.,
            height: 216.,
        };
        let (_, y, top, _) = place(
            bounds(16., 409., 668., 32.),
            size(px(1080.), px(600.)),
            &options,
        );
        assert!(top);
        assert_eq!(y, 409. - 12. - 216.);
        let (_, y, top, _) = place(
            bounds(16., 100., 668., 32.),
            size(px(1080.), px(600.)),
            &options,
        );
        assert!(!top);
        assert_eq!(y, 144.);
    }
}
