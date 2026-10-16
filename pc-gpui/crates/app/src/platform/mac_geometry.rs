//! macOS 的坐标换算（纯函数）。只有 `platform::macos` 用，但**全平台编译、在 Linux 上跑单测**（本仓库的开发机与大部分 CI
//! 都不是 mac，换算出错要在这里就能发现）。
//!
//! ## 三套坐标
//!
//! - **AppKit**：全局**点**坐标，原点在主屏（`NSScreen.screens[0]`，菜单栏所在屏）左下角，y 向上；[`CocoaRect`]。
//! - **GPUI（gpui-pre-macos 0.3.9）**：逻辑像素 = 点；窗口 `bounds()` 与建窗的 `window_bounds` 都是**所在显示器本地**的
//!   （以该显示器左上角为原点）。建窗参数由 [`display_local_bounds`] 从全局换算过去。
//! - **本 crate 的"物理像素"**（[`PhysRect`]：库里存的主窗口几何、显示器矩形、贴边隐藏的计算）：全局点坐标翻成左上原点后
//!   × 统一系数 [`ScreenLayout::unit`]（= 所有屏幕 `backingScaleFactor` 的最大值，至少 1）。逻辑像素（`PhysRect::to_logical`
//!   按 `MonitorInfo.scale` = 统一系数换算）因此就是全局点坐标（左上原点）。
//!
//! ## 与旧 Tauri 版（tao）的口径
//!
//! tao 按"点 × 各自所在屏的缩放"换算（窗口按它所在的屏、显示器按它自己）。**所有屏缩放相同**（单屏、多块 Retina、
//! 多块 1x）时两者逐像素一致：2.5.0 记下的主窗口位置照常恢复，屏幕组合 ID 也相同（`1_2880x1800@200`）。
//! 缩放混用（Retina 笔记本 + 1x 外接屏）时 tao 的换算不是一一映射——外接屏的"物理"区间与主屏的重叠（主屏 0..2880、
//! 右侧外接屏 1440..3360），同一个坐标能落在两块屏上，恢复位置会挑错屏（2.5.0 自己恢复时也按窗口当前所在屏的缩放除，
//! 同样会错）；统一系数是线性的，没有这个问题，代价是混用时外接屏上的"物理"尺寸不是真实像素。
//! 屏幕组合 ID 不受影响：[`ScreenLayout::screen_config_inputs`] 仍按各屏自己的缩放算（与 2.5.0 逐字一致）。

use gpui_kit::{Bounds, DisplayId, Pixels, point};

use super::{MonitorInfo, PhysRect};

/// AppKit 全局点坐标矩形（左下原点，y 向上）
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct CocoaRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// 一块屏幕（`NSScreen`）
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CocoaScreen {
    /// `deviceDescription["NSScreenNumber"]` = `CGDirectDisplayID` = GPUI 的 `DisplayId`
    pub display_id: u32,
    pub frame: CocoaRect,
    /// `visibleFrame`（不含菜单栏与程序坞）
    pub visible: CocoaRect,
    /// `backingScaleFactor`（Retina 2.0，普通屏 1.0）
    pub scale: f64,
}

fn valid_scale(scale: f64) -> f64 {
    if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    }
}

fn round(v: f64) -> i32 {
    v.round() as i32
}

/// 当前的屏幕布局（`NSScreen.screens` 顺序，第 0 个是主屏）与换算
#[derive(Clone, Debug, Default)]
pub struct ScreenLayout {
    screens: Vec<CocoaScreen>,
    primary_height: f64,
    unit: f64,
}

impl ScreenLayout {
    pub fn new(screens: Vec<CocoaScreen>) -> Self {
        let primary_height = screens.first().map_or(0.0, |s| s.frame.height);
        let unit = screens
            .iter()
            .map(|s| valid_scale(s.scale))
            .fold(1.0, f64::max);
        Self {
            screens,
            primary_height,
            unit,
        }
    }

    /// 点 → "物理像素"的统一系数（见模块文档）
    pub fn unit(&self) -> f64 {
        if self.unit > 0.0 { self.unit } else { 1.0 }
    }

    /// AppKit 矩形 → 物理像素（左上原点）
    pub fn to_physical(&self, r: CocoaRect) -> PhysRect {
        let u = self.unit();
        let top = self.primary_height - (r.y + r.height);
        PhysRect::new(
            round(r.x * u),
            round(top * u),
            round(r.width * u),
            round(r.height * u),
        )
    }

    /// 物理像素 → AppKit 矩形（`setFrame:display:` 用）
    pub fn to_cocoa(&self, r: PhysRect) -> CocoaRect {
        let u = self.unit();
        let width = f64::from(r.width) / u;
        let height = f64::from(r.height) / u;
        CocoaRect {
            x: f64::from(r.x) / u,
            y: self.primary_height - f64::from(r.y) / u - height,
            width,
            height,
        }
    }

    /// 窗口左上角要摆到物理 `(x, y)` 时 AppKit 的 frame 原点（左下角；`setFrameOrigin:` 用），`height` 是窗口当前的点高度
    pub fn cocoa_origin_for(&self, x: i32, y: i32, height: f64) -> (f64, f64) {
        let u = self.unit();
        (
            f64::from(x) / u,
            self.primary_height - f64::from(y) / u - height,
        )
    }

    /// AppKit 点（`NSEvent.mouseLocation`）→ 物理像素
    pub fn point_to_physical(&self, x: f64, y: f64) -> (i32, i32) {
        let u = self.unit();
        (round(x * u), round((self.primary_height - y) * u))
    }

    /// 显示器列表（物理像素；`scale` 一律是统一系数，`to_logical` 换算出来就是全局点坐标）
    pub fn monitors(&self) -> Vec<MonitorInfo> {
        self.screens
            .iter()
            .enumerate()
            .map(|(index, screen)| {
                let rect = self.to_physical(screen.frame);
                let work = if screen.visible.width > 0.0 && screen.visible.height > 0.0 {
                    self.to_physical(screen.visible)
                } else {
                    rect
                };
                MonitorInfo {
                    display_id: DisplayId::new(u64::from(screen.display_id)),
                    rect,
                    work,
                    scale: self.unit(),
                    primary: index == 0,
                }
            })
            .collect()
    }

    /// 屏幕组合 ID 的输入：（各屏点尺寸 × 各自缩放, 各自缩放），与旧 Tauri 版（tao `available_monitors()`）一致
    pub fn screen_config_inputs(&self) -> Vec<(u32, u32, f64)> {
        self.screens
            .iter()
            .map(|screen| {
                let scale = valid_scale(screen.scale);
                (
                    (screen.frame.width * scale).round().max(0.0) as u32,
                    (screen.frame.height * scale).round().max(0.0) as u32,
                    scale,
                )
            })
            .collect()
    }
}

/// 建窗参数：全局逻辑坐标（= 全局点坐标，左上原点）→ GPUI mac 要的显示器本地坐标与目标显示器。
///
/// 目标显示器：显式给的 `display_id`；没给时取包含矩形中心的显示器，再不行取主显示器。找不到任何显示器时原样返回
pub fn display_local_bounds(
    monitors: &[MonitorInfo],
    bounds: Bounds<Pixels>,
    display_id: Option<DisplayId>,
) -> (Bounds<Pixels>, Option<DisplayId>) {
    let center = point(
        bounds.origin.x + bounds.size.width / 2.,
        bounds.origin.y + bounds.size.height / 2.,
    );
    let target = display_id
        .and_then(|id| monitors.iter().find(|m| m.display_id == id))
        .or_else(|| monitors.iter().find(|m| m.logical_rect().contains(&center)))
        .or_else(|| monitors.iter().find(|m| m.primary))
        .or_else(|| monitors.first());
    match target {
        Some(monitor) => {
            let origin = monitor.logical_rect().origin;
            (
                Bounds::new(
                    point(bounds.origin.x - origin.x, bounds.origin.y - origin.y),
                    bounds.size,
                ),
                Some(monitor.display_id),
            )
        }
        None => (bounds, display_id),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::{px, size};

    fn rect(x: f64, y: f64, width: f64, height: f64) -> CocoaRect {
        CocoaRect {
            x,
            y,
            width,
            height,
        }
    }

    /// MacBook（1440×900 点，Retina）：菜单栏 25 点、程序坞 70 点
    fn macbook() -> CocoaScreen {
        CocoaScreen {
            display_id: 1,
            frame: rect(0., 0., 1440., 900.),
            visible: rect(0., 70., 1440., 805.),
            scale: 2.,
        }
    }

    /// 主屏右侧、顶边对齐的 1x 外接屏（1920×1080 点；AppKit 里它的 y = 900 − 1080 = −180）
    fn external_right() -> CocoaScreen {
        CocoaScreen {
            display_id: 7,
            frame: rect(1440., -180., 1920., 1080.),
            visible: rect(1440., -180., 1920., 1055.),
            scale: 1.,
        }
    }

    #[test]
    fn single_retina_screen_matches_the_old_tao_numbers() {
        let layout = ScreenLayout::new(vec![macbook()]);
        assert_eq!(layout.unit(), 2.);
        let monitors = layout.monitors();
        assert_eq!(monitors.len(), 1);
        assert_eq!(monitors[0].rect, PhysRect::new(0, 0, 2880, 1800));
        // 工作区：去掉顶部菜单栏（25 点）与底部程序坞（70 点）
        assert_eq!(monitors[0].work, PhysRect::new(0, 50, 2880, 1610));
        assert!(monitors[0].primary);
        assert_eq!(monitors[0].display_id, DisplayId::new(1));
        // 屏幕组合 ID 与 2.5.0 一致
        assert_eq!(
            mini_todo_core::settings::screen_config_id_of(&layout.screen_config_inputs()),
            "1_2880x1800@200"
        );
        // 逻辑矩形 = 点
        let logical = monitors[0].logical_rect();
        assert_eq!(logical.origin, point(px(0.), px(0.)));
        assert_eq!(logical.size, size(px(1440.), px(900.)));
    }

    #[test]
    fn window_frames_round_trip_through_physical_pixels() {
        let layout = ScreenLayout::new(vec![macbook()]);
        // 左上角在 (100, 50) 点、380×600 点的窗口：AppKit y = 900 − 50 − 600 = 250
        let frame = rect(100., 250., 380., 600.);
        let physical = layout.to_physical(frame);
        assert_eq!(physical, PhysRect::new(200, 100, 760, 1200));
        assert_eq!(layout.to_cocoa(physical), frame);
        // 只挪左上角：原点按窗口当前的点高度换算
        assert_eq!(layout.cocoa_origin_for(200, 100, 600.), (100., 250.));
        // 光标在主屏左上角 / 右下角
        assert_eq!(layout.point_to_physical(0., 900.), (0, 0));
        assert_eq!(layout.point_to_physical(1440., 0.), (2880, 1800));
    }

    #[test]
    fn mixed_scales_use_one_linear_unit() {
        let layout = ScreenLayout::new(vec![macbook(), external_right()]);
        assert_eq!(layout.unit(), 2.);
        let monitors = layout.monitors();
        // 外接屏在主屏右侧、顶边对齐：物理区间不与主屏重叠
        assert_eq!(monitors[1].rect, PhysRect::new(2880, 0, 3840, 2160));
        assert!(!monitors[1].primary);
        assert!(monitors.iter().all(|m| m.scale == 2.));
        // 外接屏上的窗口换算回来仍在外接屏上
        let on_external = rect(2000., 100., 380., 600.);
        let physical = layout.to_physical(on_external);
        let center = (
            physical.x + physical.width / 2,
            physical.y + physical.height / 2,
        );
        let hit: Vec<_> = monitors
            .iter()
            .filter(|m| m.rect.contains(center.0, center.1))
            .map(|m| m.display_id)
            .collect();
        assert_eq!(hit, vec![DisplayId::new(7)]);
        assert_eq!(layout.to_cocoa(physical), on_external);
        // 屏幕组合 ID 按各屏自己的缩放（与 2.5.0 一致）
        assert_eq!(
            mini_todo_core::settings::screen_config_id_of(&layout.screen_config_inputs()),
            "2_2880x1800@200_1920x1080@100"
        );
    }

    #[test]
    fn creation_bounds_are_local_to_the_target_display() {
        let layout = ScreenLayout::new(vec![macbook(), external_right()]);
        let monitors = layout.monitors();
        // 主屏：全局 = 本地
        let bounds = Bounds::new(point(px(530.), px(150.)), size(px(380.), px(600.)));
        assert_eq!(
            display_local_bounds(&monitors, bounds, Some(DisplayId::new(1))),
            (bounds, Some(DisplayId::new(1)))
        );
        // 外接屏（全局左上角 (1440, 0) 点）：减去它的原点；没给显示器时按中心找
        let bounds = Bounds::new(point(px(2000.), px(100.)), size(px(380.), px(600.)));
        let expected = Bounds::new(point(px(560.), px(100.)), size(px(380.), px(600.)));
        assert_eq!(
            display_local_bounds(&monitors, bounds, Some(DisplayId::new(7))),
            (expected, Some(DisplayId::new(7)))
        );
        assert_eq!(
            display_local_bounds(&monitors, bounds, None),
            (expected, Some(DisplayId::new(7)))
        );
        // 中心不在任何显示器上：主显示器
        let off = Bounds::new(point(px(-5000.), px(0.)), size(px(380.), px(600.)));
        assert_eq!(
            display_local_bounds(&monitors, off, None),
            (off, Some(DisplayId::new(1)))
        );
        // 没有显示器信息：原样
        assert_eq!(display_local_bounds(&[], off, None), (off, None));
    }

    #[test]
    fn screen_above_the_primary_has_negative_physical_y() {
        // 外接屏放在主屏正上方：AppKit y = 900
        let above = CocoaScreen {
            display_id: 9,
            frame: rect(0., 900., 1440., 900.),
            visible: rect(0., 900., 1440., 900.),
            scale: 2.,
        };
        let layout = ScreenLayout::new(vec![macbook(), above]);
        assert_eq!(
            layout.monitors()[1].rect,
            PhysRect::new(0, -1800, 2880, 1800)
        );
    }

    #[test]
    fn retina_screen_left_of_a_1x_primary() {
        // 主屏 1x（1920×1080 点）；Retina 外接屏（1440×900 点）放在主屏左侧、顶边对齐：AppKit x = −1440、y = 1080 − 900 = 180。
        // 统一系数取自非主屏的 2.0，主屏上的"物理"尺寸因此也 ×2
        let primary = CocoaScreen {
            display_id: 3,
            frame: rect(0., 0., 1920., 1080.),
            visible: rect(0., 0., 1920., 1055.),
            scale: 1.,
        };
        let left = CocoaScreen {
            display_id: 4,
            frame: rect(-1440., 180., 1440., 900.),
            visible: rect(-1440., 180., 1440., 900.),
            scale: 2.,
        };
        let layout = ScreenLayout::new(vec![primary, left]);
        assert_eq!(layout.unit(), 2.);
        let monitors = layout.monitors();
        assert_eq!(monitors[0].rect, PhysRect::new(0, 0, 3840, 2160));
        assert!(monitors[0].primary);
        assert_eq!(monitors[1].rect, PhysRect::new(-2880, 0, 2880, 1800));
        // 左侧屏上的窗口：负 x 往返不丢精度，中心只落在左侧屏上
        let on_left = rect(-1000., 300., 380., 600.);
        let physical = layout.to_physical(on_left);
        assert_eq!(physical, PhysRect::new(-2000, 360, 760, 1200));
        assert_eq!(layout.to_cocoa(physical), on_left);
        let center = (
            physical.x + physical.width / 2,
            physical.y + physical.height / 2,
        );
        let hit: Vec<_> = monitors
            .iter()
            .filter(|m| m.rect.contains(center.0, center.1))
            .map(|m| m.display_id)
            .collect();
        assert_eq!(hit, vec![DisplayId::new(4)]);
        // 光标在左侧屏左上角
        assert_eq!(layout.point_to_physical(-1440., 1080.), (-2880, 0));
        // 建窗：减去左侧屏的全局原点 (−1440, 0) 点
        let bounds = Bounds::new(point(px(-1000.), px(180.)), size(px(380.), px(600.)));
        assert_eq!(
            display_local_bounds(&monitors, bounds, None),
            (
                Bounds::new(point(px(440.), px(180.)), size(px(380.), px(600.))),
                Some(DisplayId::new(4))
            )
        );
        // 屏幕组合 ID 按各屏自己的缩放、像素数大的在前
        assert_eq!(
            mini_todo_core::settings::screen_config_id_of(&layout.screen_config_inputs()),
            "2_2880x1800@200_1920x1080@100"
        );
    }

    #[test]
    fn empty_layout_is_harmless() {
        let layout = ScreenLayout::new(Vec::new());
        assert_eq!(layout.unit(), 1.);
        assert!(layout.monitors().is_empty());
        assert!(layout.screen_config_inputs().is_empty());
        // 无效缩放按 1
        let odd = CocoaScreen {
            scale: 0.,
            ..macbook()
        };
        assert_eq!(ScreenLayout::new(vec![odd]).unit(), 1.);
    }
}
