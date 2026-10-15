//! `el-switch`（Element Plus 2.13 默认尺寸）。
//!
//! 外层高 32、`inline-flex` 垂直居中；轨道 `40 × 20`、圆角 10、1px 边框同底色（关 `#DCDFE6`、开 `#409EFF`）；
//! 拇指 16×16 白色圆，关时距左 2px、开时距右 2px（CSS `left: 1px` / `calc(100% - 17px)`，相对 1px 边框内侧），
//! 颜色 / 位置过渡 0.3s；`disabled` 或 `loading` → 整体 `opacity .6`、`cursor: not-allowed`，`loading` 时拇指里
//! 显示旋转的 Loading 图标。受控组件：点击只回调 `on_change(!checked)`，宿主写回状态。
//!
//! ```ignore
//! EpSwitch::new("auto-start")
//!     .checked(self.auto_start)
//!     .loading(self.auto_start_loading)
//!     .on_change(cx.listener(|this, checked: &bool, window, cx| this.set_auto_start(*checked, window, cx)))
//! ```

use std::rc::Rc;
use std::time::Duration;

use gpui_kit::{
    Animation, AnimationExt as _, App, CursorStyle, ElementId, Hsla, InteractiveElement as _,
    IntoElement, ParentElement as _, RenderOnce, StatefulInteractiveElement as _, Styled as _,
    Window, div, prelude::FluentBuilder as _, px,
};

use crate::assets::EpIcon;
use crate::theme::ep;
use crate::ui::icon::{icon, spinning};

/// 过渡时长（`--el-transition-duration`）
const TRANSITION: Duration = Duration::from_millis(300);
const TRACK_W: f32 = 40.;
const TRACK_H: f32 = 20.;
const THUMB: f32 = 16.;
/// 拇指在轨道边框内侧的偏移：关 1px，开 `100% − 17px`
const THUMB_OFF_X: f32 = 1.;
const THUMB_ON_X: f32 = TRACK_W - 2. - 17.;

type ChangeHandler = Rc<dyn Fn(&bool, &mut Window, &mut App)>;

/// 上一帧的开关状态（只在状态变化时播放过渡，首次渲染不动画）
#[derive(Default)]
struct SwitchAnim {
    last: Option<bool>,
    epoch: u64,
}

/// Element Plus 开关
#[derive(IntoElement)]
pub struct EpSwitch {
    id: ElementId,
    checked: bool,
    disabled: bool,
    loading: bool,
    on_change: Option<ChangeHandler>,
}

impl EpSwitch {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self {
            id: id.into(),
            checked: false,
            disabled: false,
            loading: false,
            on_change: None,
        }
    }

    pub fn checked(mut self, checked: bool) -> Self {
        self.checked = checked;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// `:loading`：半透明、拇指里转圈、不可点击
    pub fn loading(mut self, loading: bool) -> Self {
        self.loading = loading;
        self
    }

    /// 点击时回调新的值（`!checked`）
    pub fn on_change(mut self, f: impl Fn(&bool, &mut Window, &mut App) + 'static) -> Self {
        self.on_change = Some(Rc::new(f));
        self
    }
}

/// 两个颜色按 `t`（0~1）线性插值（RGB 空间）
pub(crate) fn mix(from: Hsla, to: Hsla, t: f32) -> Hsla {
    let a = from.to_rgb();
    let b = to.to_rgb();
    let lerp = |x: f32, y: f32| x + (y - x) * t.clamp(0., 1.);
    gpui_kit::Rgba {
        r: lerp(a.r, b.r),
        g: lerp(a.g, b.g),
        b: lerp(a.b, b.b),
        a: lerp(a.a, b.a),
    }
    .into()
}

impl RenderOnce for EpSwitch {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let ep = ep();
        let checked = self.checked;
        let inactive = self.disabled || self.loading;
        let anim =
            window.use_keyed_state((self.id.clone(), "anim"), cx, |_, _| SwitchAnim::default());
        let epoch = anim.update(cx, |s, _| {
            if s.last.is_some_and(|last| last != checked) {
                s.epoch += 1;
            }
            s.last = Some(checked);
            s.epoch
        });
        let (off_color, on_color) = (ep.border, ep.primary);
        let color_at = move |t: f32| {
            // t: 0 = 关，1 = 开
            mix(off_color, on_color, t)
        };
        let target = if checked { 1. } else { 0. };
        let loading = self.loading;
        let id = self.id.clone();

        let render_core = move |t: f32| {
            let color = color_at(t);
            let x = THUMB_OFF_X + (THUMB_ON_X - THUMB_OFF_X) * t;
            div()
                .relative()
                .flex_none()
                .w(px(TRACK_W))
                .h(px(TRACK_H))
                .rounded(px(10.))
                .border_1()
                .border_color(color)
                .bg(color)
                .child(
                    div()
                        .absolute()
                        .top(px(1.))
                        .left(px(x))
                        .size(px(THUMB))
                        .rounded_full()
                        .bg(ep.white)
                        .flex()
                        .items_center()
                        .justify_center()
                        .when(loading, |this| {
                            this.child(spinning(
                                (id.clone(), "spin"),
                                icon(EpIcon::Loading, 12., color),
                            ))
                        }),
                )
        };

        let core = if epoch == 0 {
            render_core(target).into_any_element()
        } else {
            div()
                .child(render_core(target))
                .with_animation(
                    ("ep-switch-anim", epoch as usize),
                    Animation::new(TRANSITION),
                    move |_, delta| {
                        let t = if checked { delta } else { 1. - delta };
                        render_core(t)
                    },
                )
                .into_any_element()
        };

        let on_change = self.on_change.clone();
        div()
            .id(self.id)
            .flex()
            .flex_none()
            .items_center()
            .h(px(32.))
            .when(inactive, |this| {
                this.opacity(0.6).cursor(CursorStyle::OperationNotAllowed)
            })
            .when(!inactive, |this| this.cursor_pointer())
            .when_some(on_change.filter(|_| !inactive), |this, handler| {
                this.on_click(move |_, window, cx| handler(&!checked, window, cx))
            })
            .child(core)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::hex;

    #[test]
    fn thumb_positions_match_element_plus() {
        // 1px 边框内侧：关 left 1px → 外框 2px；开 calc(100% - 17px) = 38 − 17 = 21 → 外框 22px（距右 2px）
        assert_eq!(THUMB_OFF_X + 1., 2.);
        assert_eq!(THUMB_ON_X + 1. + THUMB, TRACK_W - 2.);
    }

    #[test]
    fn mix_interpolates_endpoints() {
        let a = hex(0xDCDFE6);
        let b = hex(0x409EFF);
        assert_eq!(mix(a, b, 0.).to_rgb(), a.to_rgb());
        let end = mix(a, b, 1.).to_rgb();
        let want = b.to_rgb();
        assert!((end.r - want.r).abs() < 1e-4 && (end.b - want.b).abs() < 1e-4);
    }
}
