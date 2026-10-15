//! Element Plus 浅色弹出层（`el-popper is-light`）的外观：白底、1px `#E4E7ED` 边框、圆角 4、`--el-box-shadow-light`；
//! 可选的 10×10 箭头（旋转 45° 的方块，露出 5px 三角形，带边框）。下拉选择、下拉菜单共用。
//!
//! GPUI 的弹层必须画在窗口内部：调用方用 `deferred(anchored()…)` 定位，再把 [`panel`] 放进去；
//! 箭头用 [`arrow_up`] 作为面板的子元素（绝对定位在面板上沿之外，`left` 为箭头中心相对面板左边的距离）。

use gpui_kit::{AnyElement, Div, IntoElement, ParentElement as _, Pixels, Styled as _, div, px};

use crate::assets::AppIcon;
use crate::theme::{self, ep};
use crate::ui::icon::icon;

/// 弹层与触发元素之间的距离（EP `offset` 默认 12）
pub const OFFSET: f32 = 12.;

/// 浅色弹层面板（`relative`，箭头可以挂在它上面）
pub fn panel() -> Div {
    let ep = ep();
    div()
        .relative()
        .bg(ep.white)
        .border_1()
        .border_color(ep.border_light)
        .rounded(theme::radius::SM)
        .shadow(ep.box_shadow_light())
}

/// 面板上沿的箭头（尖朝上）：`center_x` 为箭头中心到面板左边框外沿的距离
pub fn arrow_up(center_x: Pixels) -> AnyElement {
    let ep = ep();
    // 外层（边框色）12×6，内层（白）10×5 下移 2px 盖住面板上边框
    div()
        .absolute()
        .top(px(-7.))
        .left(center_x - px(6.))
        .w(px(12.))
        .h(px(8.))
        .child(
            icon(AppIcon::ArrowUpTip, 12., ep.border_light)
                .absolute()
                .top_0()
                .left_0()
                .w(px(12.))
                .h(px(6.)),
        )
        .child(
            icon(AppIcon::ArrowUpTip, 10., ep.white)
                .absolute()
                .top(px(2.))
                .left(px(1.))
                .w(px(10.))
                .h(px(5.)),
        )
        .into_any_element()
}
