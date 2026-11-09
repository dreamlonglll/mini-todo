//! Element Plus 浅色弹层（`el-popper is-light`）：白底、1px `#E4E7ED` 边、圆角 4、`box-shadow-light`，
//! 可选箭头（EP 是 10×10 的旋转方块，中心压在弹层边框上：外露约 12×6 的三角形）。下拉选择、下拉菜单、
//! 日期 / 时间面板、颜色面板共用。清单 / 日历窗口深色时，窗口里的下拉菜单用 EP 暗黑模式的配色
//! （[`frame_for`]、`PopperOptions::dark`，颜色见 `theme::ep_menu`）。
//!
//! 用法：触发元素（`relative()`）里放一个 [`Anchor::measure`]（记录触发元素的窗口坐标），打开时在 render 里追加
//! [`popper`]（`deferred` + `anchored`，画在窗口最上层）。弹层按触发元素位置定位：`placement` 优先，
//! 放不下且另一侧放得下时翻转（`flip`）；水平方向夹回窗口内（`snap_to_window`，贴边时不会整体翻到另一侧）；
//! 点弹层与触发元素之外的地方调用 `on_dismiss`（不拦截这次点击，与 EP 的 click-outside 一致）。
//! 右键菜单没有触发元素：用 [`Anchor::at`]（按下位置的零尺寸锚点）；级联子菜单用 [`Placement::RightStart`]
//! 贴在父菜单旁边（锚点量父菜单的外框）。
//!
//! 触发元素的位置取上一帧量到的值（GPUI 在 render 时还没有布局）；位置变化时量取元素会请求重绘，
//! 下一帧就跟上（滚动表单时弹层跟着走）。
//!
//! ```ignore
//! // 构造：anchor: Anchor::new()；触发元素：div().relative().child(self.anchor.measure())
//! ep_popper::popper(&self.anchor, window, PopperOptions { placement: Placement::BottomStart, flip: true,
//!     gap: ep_popper::OFFSET, arrow: true, width, height, dark: false }, ep_popper::frame().child(..),
//!     move |_, cx| close(cx))
//! ```

use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::{
    AnyElement, App, Bounds, InteractiveElement as _, IntoElement, ParentElement as _, Pixels,
    Styled as _, Window, canvas, deferred, div, point, prelude::FluentBuilder as _, px,
};

use crate::assets::AppIcon;
use crate::theme::{self, ep_menu};
use crate::ui::icon::icon;

/// 弹层与触发元素之间的距离（EP `offset` 默认 12，含箭头外露部分）
pub const OFFSET: f32 = 12.;

/// 触发元素的窗口坐标（上一帧）
#[derive(Clone, Default)]
pub struct Anchor(Rc<Cell<Option<Bounds<Pixels>>>>);

impl Anchor {
    pub fn new() -> Self {
        Self::default()
    }

    /// 固定在窗口坐标 `position` 的零尺寸锚点（右键菜单：弹层贴着按下的位置；不用 [`Self::measure`]）
    pub fn at(position: gpui_kit::Point<Pixels>) -> Self {
        Self(Rc::new(Cell::new(Some(Bounds::new(
            position,
            gpui_kit::size(px(0.), px(0.)),
        )))))
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
    /// 上方、左对齐（日期 / 时间面板：`top-start`）
    TopStart,
    /// 下方、左对齐（下拉选择 `bottom-start`）
    BottomStart,
    /// 下方、右对齐（下拉菜单 `bottom-end`）
    BottomEnd,
    /// 下方、水平居中（颜色面板、下拉菜单默认 `bottom`）
    Bottom,
    /// 右侧、上沿对齐（级联子菜单 `right-start`）：右边放不下且左边放得下时翻到左侧（`flip`）；垂直方向夹回窗口内。
    /// 侧边弹出不画箭头
    RightStart,
}

impl Placement {
    fn is_top(self) -> bool {
        matches!(self, Placement::TopStart)
    }

    fn is_side(self) -> bool {
        matches!(self, Placement::RightStart)
    }
}

/// 弹层参数
pub struct PopperOptions {
    pub placement: Placement,
    /// 放不下时翻到另一侧
    pub flip: bool,
    /// 与触发元素的间距（EP 默认 [`OFFSET`]：含箭头外露部分）
    pub gap: f32,
    pub arrow: bool,
    /// 弹层宽高（定位与翻转用；内容自己撑开时给估计值即可）
    pub width: f32,
    pub height: f32,
    /// 深色箭头（内容用 [`frame_for`]`(true)` 的深色外框时同时打开；只有清单 / 日历窗口的下拉菜单会用）
    pub dark: bool,
}

/// 箭头外露三角形的宽高（10×10 旋转方块，中心压在 1px 边框内侧：外露 ≈ 12×6）
const ARROW_W: f32 = 12.;
const ARROW_H: f32 = 6.;

/// 计算弹层左上角（窗口坐标）、是否在上方、箭头中心相对弹层左边的 x
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
    if options.placement == Placement::RightStart {
        let right_x = a_left + a_w + options.gap;
        let left_x = a_left - options.gap - options.width;
        let x = if options.flip && right_x + options.width > vw && left_x >= 0. {
            left_x
        } else {
            right_x
        };
        let x = x.min(vw - options.width).max(0.);
        let y = a_top.min(vh - options.height).max(0.);
        return (x, y, false, 0.);
    }
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
        Placement::TopStart | Placement::BottomStart | Placement::RightStart => a_left,
        Placement::BottomEnd => a_left + a_w - options.width,
        Placement::Bottom => a_left + (a_w - options.width) / 2.,
    };
    let x = x.min(vw - options.width).max(0.);
    let arrow_x = (a_left + a_w / 2. - x).clamp(14., (options.width - 14.).max(14.));
    (x, y, top, arrow_x)
}

/// 浅色弹层外框（不含定位；`relative`，内容可以绝对定位）
pub fn frame() -> gpui_kit::Div {
    frame_for(false)
}

/// 弹层外框：`dark` 时用 EP 暗黑模式的底色、边框与阴影（`theme::ep_menu`）
pub fn frame_for(dark: bool) -> gpui_kit::Div {
    let colors = ep_menu(dark);
    div()
        .relative()
        .bg(colors.bg)
        .border_1()
        .border_color(colors.border)
        .rounded(theme::radius::SM)
        .shadow(colors.shadow())
}

/// 箭头：弹层在下方时画在上沿（尖朝上），在上方时画在下沿（尖朝下）。外层边框色三角 12×6（底边压在弹层外沿）+
/// 内层底色三角 10×5（向弹层内移 2px，盖住箭头下面那 1px 边框，与 EP 箭头盖住边框一致）。
/// `center_x` 为箭头中心到弹层左边框外沿的距离。
fn arrow(top: bool, center_x: f32, dark: bool) -> impl IntoElement {
    let colors = ep_menu(dark);
    let tip = if top {
        AppIcon::ArrowDownTip
    } else {
        AppIcon::ArrowUpTip
    };
    div()
        .absolute()
        .left(px(center_x - ARROW_W / 2.))
        .w(px(ARROW_W))
        .h(px(ARROW_H + 2.))
        .when(!top, |this| this.top(px(-ARROW_H)))
        .when(top, |this| this.bottom(px(-ARROW_H)))
        .child(
            icon(tip, ARROW_W, colors.border)
                .absolute()
                .left_0()
                .when(!top, |this| this.top_0())
                .when(top, |this| this.bottom_0())
                .w(px(ARROW_W))
                .h(px(ARROW_H)),
        )
        .child(
            icon(tip, ARROW_W - 2., colors.bg)
                .absolute()
                .left(px(1.))
                .when(!top, |this| this.top(px(2.)))
                .when(top, |this| this.bottom(px(2.)))
                .w(px(ARROW_W - 2.))
                .h(px(ARROW_H - 1.)),
        )
}

/// 打开的弹层：`content` 已含外框（[`frame`] / [`frame_for`]）；点外面（且不在触发元素上）调用 `on_dismiss`
/// （`on_dismiss` 里可以用 `window.mouse_position()` 再排除别的区域，例如级联子菜单）
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
        .when(options.arrow && !options.placement.is_side(), |this| {
            this.child(arrow(top, arrow_x, options.dark))
        });
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
            dark: false,
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
            dark: false,
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

    #[test]
    fn bottom_end_aligns_right_and_clamps_into_the_window() {
        // 收纳箱「更多」菜单：按钮 (334, 4, 28, 24)，菜单宽 123，窗口宽 368
        let options = PopperOptions {
            placement: Placement::BottomEnd,
            flip: false,
            gap: 12.,
            arrow: true,
            width: 123.,
            height: 120.,
            dark: false,
        };
        let (x, y, top, arrow_x) = place(
            bounds(334., 4., 28., 24.),
            size(px(368.), px(336.)),
            &options,
        );
        assert_eq!((x, y, top), (362. - 123., 40., false));
        // 箭头对准按钮中心（距菜单右边 14）
        assert_eq!(arrow_x, 123. - 14.);
        // 贴右边：夹回窗口内，箭头仍指向按钮（夹在两端 14px 内）
        let (x, _, _, arrow_x) = place(
            bounds(350., 4., 28., 24.),
            size(px(368.), px(336.)),
            &options,
        );
        assert_eq!(x, 368. - 123.);
        assert_eq!(arrow_x, 123. - 14.);
    }

    #[test]
    fn context_menu_opens_at_the_cursor_and_flips_up_near_the_bottom() {
        // 待办行右键：零尺寸锚点在按下的位置，菜单左上角贴着它；下面放不下时菜单底边贴着它
        let options = PopperOptions {
            placement: Placement::BottomStart,
            flip: true,
            gap: 2.,
            arrow: false,
            width: 120.,
            height: 44.,
            dark: true,
        };
        let anchor = Anchor::at(point(px(100.), px(200.)));
        let at = anchor.bounds().unwrap();
        assert!(
            !anchor.contains(point(px(100.), px(200.))),
            "零尺寸：哪里都不算点在锚点上"
        );
        let viewport = size(px(380.), px(600.));
        assert_eq!(place(at, viewport, &options), (100., 202., false, 14.));
        let at = Anchor::at(point(px(300.), px(580.))).bounds().unwrap();
        let (x, y, top, _) = place(at, viewport, &options);
        // 贴右边夹回窗口内，翻到上方
        assert_eq!((x, y, top), (380. - 120., 580. - 2. - 44., true));
    }

    #[test]
    fn right_start_sits_beside_the_parent_and_flips_left_when_there_is_no_room() {
        // 级联子菜单：锚点是父菜单的外框
        let options = PopperOptions {
            placement: Placement::RightStart,
            flip: true,
            gap: 4.,
            arrow: false,
            width: 150.,
            height: 108.,
            dark: false,
        };
        let viewport = size(px(380.), px(600.));
        let (x, y, top, _) = place(bounds(40., 100., 120., 44.), viewport, &options);
        assert_eq!((x, y, top), (164., 100., false));
        // 右边放不下：翻到父菜单左边
        let (x, y, _, _) = place(bounds(250., 100., 120., 44.), viewport, &options);
        assert_eq!((x, y), (250. - 4. - 150., 100.));
        // 两边都放不下：夹回窗口内；底下放不下：上移到窗口里
        let (x, y, _, _) = place(bounds(100., 560., 200., 44.), viewport, &options);
        assert_eq!((x, y), (380. - 150., 600. - 108.));
    }
}
