//! 图标：`svg().path(..)` 单色蒙版，颜色来自 `text_color`（Element Plus 图标都是 `fill="currentColor"`）。
//!
//! ```ignore
//! use crate::ui::icon::{icon, spinning};
//! use crate::assets::EpIcon;
//! div().child(icon(EpIcon::Bell, 12., palette.text_tertiary))
//! // 同步中转圈（1s 一圈，`@keyframes spin` 的等价物）
//! div().child(spinning("sync-spin", icon(EpIcon::Refresh, 16., color)))
//! ```

use std::time::Duration;

use gpui_kit::{
    Animation, AnimationExt as _, ElementId, Hsla, IntoElement, SharedString, Styled as _, Svg,
    Transformation, percentage, px, svg,
};

use crate::assets::{AppIcon, EpIcon};

/// 图标来源
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IconSource {
    Ep(EpIcon),
    App(AppIcon),
    /// 任意资源路径（例如 gpui-component 的 `icons/xxx.svg`）
    Path(SharedString),
}

impl IconSource {
    pub fn path(&self) -> SharedString {
        match self {
            IconSource::Ep(icon) => icon.path().into(),
            IconSource::App(icon) => icon.path().into(),
            IconSource::Path(path) => path.clone(),
        }
    }
}

impl From<EpIcon> for IconSource {
    fn from(icon: EpIcon) -> Self {
        IconSource::Ep(icon)
    }
}

impl From<AppIcon> for IconSource {
    fn from(icon: AppIcon) -> Self {
        IconSource::App(icon)
    }
}

/// 正方形图标（`size` 逻辑像素；el-icon 的 `:size`）
pub fn icon(source: impl Into<IconSource>, size: f32, color: Hsla) -> Svg {
    svg()
        .path(source.into().path())
        .size(px(size))
        .flex_none()
        .text_color(color)
}

/// 匀速转圈（同步按钮、`el-button` 的 loading）；`id` 在同一窗口内要稳定且唯一
pub fn spinning(id: impl Into<ElementId>, icon: Svg) -> impl IntoElement {
    icon.with_animation(
        id,
        Animation::new(Duration::from_secs(1)).repeat(),
        |svg, delta| svg.with_transformation(Transformation::rotate(percentage(delta))),
    )
}
