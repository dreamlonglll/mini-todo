//! 收纳箱拖动 / 缩放时的吸附对齐（纯函数，不依赖 GPUI，单测向量来自任务 `10-18-fence-snap` 的 PRD 验收用例）。
//!
//! 几何一律是物理像素的客户区（与库里的几何同一口径）。目标是其它显示中的收纳箱**屏幕上的可见矩形**（折叠的只有标题栏），
//! 拖动 / 缩放开始时快照一次（[`SnapScene`]），每次移动只取光标所在显示器上的（[`SnapScene::targets_at`]）。
//!
//! - 横向 / 纵向独立：两个方向各自找距离最近、且不超过阈值的候选，可以同时吸 x 和 y；同距离取目标列表里靠前的（确定性）
//! - 移动（[`snap_move`]）：同名边对齐 left↔left、right↔right、top↔top、bottom↔bottom，只平移、尺寸不变
//! - 缩放（[`snap_resize`]，在 `logic::resize_rect` 之后）：只调整被拖动的边（对边不动）；候选 = 同名边对齐 + 等宽（拖 E / W）/
//!   等高（拖 N / S），放在一起取最近；吸附后小于最小尺寸的候选跳过
//! - 不做：相邻贴边（我的 left 吸别人的 right）、屏幕工作区边缘、中心线
//!
//! 高亮（[`SnapGuide`]）画在**目标**收纳箱上：同名边对齐 → 目标的那条边；等宽 → 目标的上、下两边；等高 → 目标的左、右两边。
//! 吸附生效的方向上，所有与结果恰好成立的关系都高亮（例如两个目标的 left 相同，吸上之后两个都亮）。

use std::collections::HashMap;

use mini_todo_core::fence::{PxRect, pick_work_area};

use super::logic::{ResizeDir, js_round};

/// 吸附阈值（逻辑像素）：实际阈值 = `round(10 × 物理系数)` 物理像素（[`snap_threshold`]）
pub const SNAP_THRESHOLD_DIP: f64 = 10.;

/// 物理像素的吸附阈值。`scale` 是拖动中收纳箱的逻辑 → 物理系数（`platform::physical_scale`；非法值按 1）
pub fn snap_threshold(scale: f64) -> i32 {
    let scale = if scale > 0. { scale } else { 1. };
    js_round(SNAP_THRESHOLD_DIP * scale)
}

/// 一个吸附目标：另一个收纳箱窗口屏幕上的可见客户区（物理像素）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SnapTarget {
    pub fence_id: i64,
    pub rect: PxRect,
}

/// 一块显示器：整块（判断光标在哪块上）与工作区（判断目标属于哪块）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SnapMonitor {
    pub rect: PxRect,
    pub work: PxRect,
}

/// 拖动 / 缩放开始时的快照（拖动期间其它收纳箱不会动、显示器不会变）
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SnapScene {
    /// 其它显示中的收纳箱，按 `fence_id` 排序（同距离时按这个顺序取第一个）
    pub targets: Vec<SnapTarget>,
    pub monitors: Vec<SnapMonitor>,
}

impl SnapScene {
    /// 与光标在同一显示器上的目标：目标属于交叠面积最大的工作区（`pick_work_area`，与收纳箱夹回可见区域同一规则），
    /// 与光标所在显示器的工作区相同才算。取不到光标 / 光标不在任何显示器上 / 没有显示器信息时不按显示器过滤
    pub fn targets_at(&self, cursor: Option<(i32, i32)>) -> Vec<SnapTarget> {
        let monitor = cursor.and_then(|(x, y)| {
            self.monitors.iter().find(|m| {
                x >= m.rect.x && x < m.rect.right() && y >= m.rect.y && y < m.rect.bottom()
            })
        });
        let Some(monitor) = monitor else {
            return self.targets.clone();
        };
        let works: Vec<PxRect> = self.monitors.iter().map(|m| m.work).collect();
        self.targets
            .iter()
            .filter(|t| pick_work_area(t.rect, &works) == Some(monitor.work))
            .copied()
            .collect()
    }
}

/// 目标收纳箱上要高亮的一条边
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SnapEdge {
    Left,
    Right,
    Top,
    Bottom,
}

impl SnapEdge {
    fn bit(self) -> u8 {
        match self {
            SnapEdge::Left => 1,
            SnapEdge::Right => 2,
            SnapEdge::Top => 4,
            SnapEdge::Bottom => 8,
        }
    }
}

/// 高亮：目标收纳箱 `fence_id` 的 `edge` 边
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SnapGuide {
    pub fence_id: i64,
    pub edge: SnapEdge,
}

/// 一个收纳箱要高亮的边的集合
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SnapEdges(u8);

impl SnapEdges {
    pub fn insert(&mut self, edge: SnapEdge) {
        self.0 |= edge.bit();
    }

    pub fn contains(self, edge: SnapEdge) -> bool {
        self.0 & edge.bit() != 0
    }

    pub fn is_empty(self) -> bool {
        self.0 == 0
    }
}

/// 按目标收纳箱归并高亮（跨窗口高亮状态的形状）
pub fn group_guides(guides: &[SnapGuide]) -> HashMap<i64, SnapEdges> {
    let mut map: HashMap<i64, SnapEdges> = HashMap::new();
    for guide in guides {
        map.entry(guide.fence_id).or_default().insert(guide.edge);
    }
    map
}

/// 高亮从 `old` 变成 `new` 时需要重绘的收纳箱（升序、不重复）
pub fn changed_fences(old: &HashMap<i64, SnapEdges>, new: &HashMap<i64, SnapEdges>) -> Vec<i64> {
    let mut ids: Vec<i64> = old
        .keys()
        .chain(new.keys())
        .copied()
        .filter(|id| old.get(id) != new.get(id))
        .collect();
    ids.sort_unstable();
    ids.dedup();
    ids
}

/// 候选里绝对值最小、且不超过阈值的那个；同距离取先出现的
fn nearest(candidates: impl IntoIterator<Item = i32>, threshold: i32) -> Option<i32> {
    let mut best: Option<i32> = None;
    for delta in candidates {
        if delta.abs() <= threshold && best.is_none_or(|b| delta.abs() < b.abs()) {
            best = Some(delta);
        }
    }
    best
}

fn push_guide(guides: &mut Vec<SnapGuide>, fence_id: i64, edge: SnapEdge) {
    let guide = SnapGuide { fence_id, edge };
    if !guides.contains(&guide) {
        guides.push(guide);
    }
}

/// 移动吸附：`rect` 是跟手的位置（尺寸 = 窗口当前尺寸），返回吸附后的矩形（只平移）与要高亮的目标边
pub fn snap_move(rect: PxRect, targets: &[SnapTarget], threshold: i32) -> (PxRect, Vec<SnapGuide>) {
    let dx = nearest(
        targets
            .iter()
            .flat_map(|t| [t.rect.x - rect.x, t.rect.right() - rect.right()]),
        threshold,
    );
    let dy = nearest(
        targets
            .iter()
            .flat_map(|t| [t.rect.y - rect.y, t.rect.bottom() - rect.bottom()]),
        threshold,
    );
    let snapped = PxRect {
        x: rect.x + dx.unwrap_or(0),
        y: rect.y + dy.unwrap_or(0),
        ..rect
    };
    let mut guides = Vec::new();
    for t in targets {
        if dx.is_some() {
            if t.rect.x == snapped.x {
                push_guide(&mut guides, t.fence_id, SnapEdge::Left);
            }
            if t.rect.right() == snapped.right() {
                push_guide(&mut guides, t.fence_id, SnapEdge::Right);
            }
        }
        if dy.is_some() {
            if t.rect.y == snapped.y {
                push_guide(&mut guides, t.fence_id, SnapEdge::Top);
            }
            if t.rect.bottom() == snapped.bottom() {
                push_guide(&mut guides, t.fence_id, SnapEdge::Bottom);
            }
        }
    }
    (snapped, guides)
}

/// 一个方向上被拖动的边的候选尺寸里离 `current` 最近、不超过阈值且不小于 `min` 的那个
fn nearest_size(
    candidates: impl IntoIterator<Item = i32>,
    current: i32,
    threshold: i32,
    min: i32,
) -> Option<i32> {
    nearest(
        candidates
            .into_iter()
            .filter(|size| *size >= min)
            .map(|size| size - current),
        threshold,
    )
    .map(|delta| current + delta)
}

/// 缩放吸附：`rect` 是 `resize_rect` 的结果（拖哪条边改哪条边、已不小于最小尺寸），只调整 `dir` 拖动的边、对边不动；
/// 返回吸附后的矩形与要高亮的目标边
pub fn snap_resize(
    rect: PxRect,
    dir: ResizeDir,
    targets: &[SnapTarget],
    threshold: i32,
    min_w: i32,
    min_h: i32,
) -> (PxRect, Vec<SnapGuide>) {
    let mut out = rect;
    // 横向：E 边动（left 不动）→ right 对齐 / 等宽；W 边动（right 不动）→ left 对齐 / 等宽
    let (left, right) = (rect.x, rect.right());
    let width = if dir.has_e() {
        nearest_size(
            targets
                .iter()
                .flat_map(|t| [t.rect.right() - left, t.rect.width]),
            rect.width,
            threshold,
            min_w,
        )
    } else if dir.has_w() {
        nearest_size(
            targets
                .iter()
                .flat_map(|t| [right - t.rect.x, t.rect.width]),
            rect.width,
            threshold,
            min_w,
        )
    } else {
        None
    };
    if let Some(width) = width {
        if dir.has_w() {
            out.x = right - width;
        }
        out.width = width;
    }
    // 纵向：S 边动（top 不动）→ bottom 对齐 / 等高；N 边动（bottom 不动）→ top 对齐 / 等高
    let (top, bottom) = (rect.y, rect.bottom());
    let height = if dir.has_s() {
        nearest_size(
            targets
                .iter()
                .flat_map(|t| [t.rect.bottom() - top, t.rect.height]),
            rect.height,
            threshold,
            min_h,
        )
    } else if dir.has_n() {
        nearest_size(
            targets
                .iter()
                .flat_map(|t| [bottom - t.rect.y, t.rect.height]),
            rect.height,
            threshold,
            min_h,
        )
    } else {
        None
    };
    if let Some(height) = height {
        if dir.has_n() {
            out.y = bottom - height;
        }
        out.height = height;
    }
    let (snap_x, snap_y) = (width.is_some(), height.is_some());

    let mut guides = Vec::new();
    for t in targets {
        if snap_x {
            let edge_aligned = if dir.has_e() {
                (t.rect.right() == out.right()).then_some(SnapEdge::Right)
            } else {
                (t.rect.x == out.x).then_some(SnapEdge::Left)
            };
            if let Some(edge) = edge_aligned {
                push_guide(&mut guides, t.fence_id, edge);
            }
            if t.rect.width == out.width {
                push_guide(&mut guides, t.fence_id, SnapEdge::Top);
                push_guide(&mut guides, t.fence_id, SnapEdge::Bottom);
            }
        }
        if snap_y {
            let edge_aligned = if dir.has_s() {
                (t.rect.bottom() == out.bottom()).then_some(SnapEdge::Bottom)
            } else {
                (t.rect.y == out.y).then_some(SnapEdge::Top)
            };
            if let Some(edge) = edge_aligned {
                push_guide(&mut guides, t.fence_id, edge);
            }
            if t.rect.height == out.height {
                push_guide(&mut guides, t.fence_id, SnapEdge::Left);
                push_guide(&mut guides, t.fence_id, SnapEdge::Right);
            }
        }
    }
    (out, guides)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::windows::fence::logic::resize_rect;

    fn r(x: i32, y: i32, width: i32, height: i32) -> PxRect {
        PxRect {
            x,
            y,
            width,
            height,
        }
    }

    fn target(fence_id: i64, rect: PxRect) -> SnapTarget {
        SnapTarget { fence_id, rect }
    }

    fn g(fence_id: i64, edge: SnapEdge) -> SnapGuide {
        SnapGuide { fence_id, edge }
    }

    /// PRD 的收纳箱 A（left=200、top=100、宽 400、高 300）
    fn a() -> SnapTarget {
        target(1, r(200, 100, 400, 300))
    }

    #[test]
    fn threshold_is_ten_logical_pixels() {
        assert_eq!(snap_threshold(1.), 10);
        assert_eq!(snap_threshold(1.25), 13, "12.5 按 JS Math.round 进位");
        assert_eq!(snap_threshold(1.5), 15);
        assert_eq!(snap_threshold(2.), 20);
        assert_eq!(snap_threshold(0.), 10, "非法系数按 1");
    }

    #[test]
    fn moving_snaps_top_to_top_within_the_threshold() {
        let targets = [a()];
        // B 在 A 右边很远（x 方向没有候选），B.top 落在 [90, 110] 都吸到 100
        for top in [90, 95, 100, 104, 110] {
            let (rect, guides) = snap_move(r(900, top, 300, 200), &targets, 10);
            assert_eq!(rect, r(900, 100, 300, 200), "top = {top}");
            assert_eq!(guides, [g(1, SnapEdge::Top)], "top = {top}");
        }
        // 超出阈值：跟手、不高亮
        for top in [89, 111, 300] {
            let (rect, guides) = snap_move(r(900, top, 300, 200), &targets, 10);
            assert_eq!(rect, r(900, top, 300, 200), "top = {top}");
            assert!(guides.is_empty(), "top = {top}");
        }
    }

    #[test]
    fn moving_snaps_both_axes_independently() {
        let targets = [a()];
        // B 宽 300：B.right = 593（A.right 600 差 7）→ right 对齐；B.bottom = 395（A.bottom 400 差 5）→ bottom 对齐
        let (rect, guides) = snap_move(r(293, 195, 300, 200), &targets, 10);
        assert_eq!(rect, r(300, 200, 300, 200));
        assert_eq!(guides, [g(1, SnapEdge::Right), g(1, SnapEdge::Bottom)]);
        // 同时可吸 top：B.top = 92 → 100
        let (rect, guides) = snap_move(r(296, 92, 300, 150), &targets, 10);
        assert_eq!(rect, r(300, 100, 300, 150));
        assert_eq!(guides, [g(1, SnapEdge::Right), g(1, SnapEdge::Top)]);
        // 尺寸不变、只平移
        assert_eq!((rect.width, rect.height), (300, 150));
    }

    #[test]
    fn moving_takes_the_nearest_candidate_and_breaks_ties_by_target_order() {
        // left 差 -6、right 差 +4 → 取 right
        let (rect, guides) = snap_move(r(206, 500, 390, 100), &[a()], 10);
        assert_eq!(rect.x, 210);
        assert_eq!(guides, [g(1, SnapEdge::Right)]);
        // 两个目标同距离：取列表里靠前的
        let targets = [
            target(3, r(1000, 100, 200, 200)),
            target(5, r(1000, 120, 200, 200)),
        ];
        let (rect, guides) = snap_move(r(100, 110, 200, 50), &targets, 10);
        assert_eq!(rect.y, 100, "差 -10 与 +10 同距离，取先出现的目标 #3");
        assert_eq!(guides, [g(3, SnapEdge::Top)]);
        // 宽度相同：left 与 right 同时对齐，两条边都亮
        let (rect, guides) = snap_move(r(195, 600, 400, 100), &[a()], 10);
        assert_eq!(rect.x, 200);
        assert_eq!(guides, [g(1, SnapEdge::Left), g(1, SnapEdge::Right)]);
    }

    #[test]
    fn moving_highlights_every_target_that_ends_up_aligned() {
        // 两个目标 left 相同：吸上之后两个都亮
        let targets = [
            target(1, r(200, 100, 400, 300)),
            target(2, r(200, 600, 250, 100)),
        ];
        let (rect, guides) = snap_move(r(204, 900, 300, 100), &targets, 10);
        assert_eq!(rect.x, 200);
        assert_eq!(guides, [g(1, SnapEdge::Left), g(2, SnapEdge::Left)]);
    }

    #[test]
    fn collapsed_targets_snap_by_their_title_bar() {
        // 折叠的 A 只有标题栏（高 48）：B.top 吸 A.top、B.bottom 吸标题栏的下沿 148，而不是展开后的 400
        let collapsed = [target(1, r(200, 100, 400, 48))];
        let (rect, guides) = snap_move(r(900, 300, 300, 95), &collapsed, 10);
        assert_eq!(
            rect.y, 300,
            "bottom 395 离展开态的 400 只差 5，但折叠的目标下沿是 148：不吸"
        );
        assert!(guides.is_empty());
        let (rect, _) = snap_move(r(900, 300, 300, 95), &[a()], 10);
        assert_eq!(rect.y, 305, "对照：展开的 A 会吸 bottom");
        let (rect, guides) = snap_move(r(900, 95, 300, 60), &collapsed, 10);
        assert_eq!(rect.y, 100);
        assert_eq!(guides, [g(1, SnapEdge::Top)]);
        let (rect, guides) = snap_move(r(900, 142, 300, 10), &collapsed, 10);
        assert_eq!(rect.y, 138, "bottom 152 → 148");
        assert_eq!(guides, [g(1, SnapEdge::Bottom)]);
    }

    #[test]
    fn no_targets_means_no_snapping() {
        let (rect, guides) = snap_move(r(1, 2, 3, 4), &[], 10);
        assert_eq!(rect, r(1, 2, 3, 4));
        assert!(guides.is_empty());
        let (rect, guides) = snap_resize(r(1, 2, 300, 400), ResizeDir::SE, &[], 10, 160, 160);
        assert_eq!(rect, r(1, 2, 300, 400));
        assert!(guides.is_empty());
    }

    #[test]
    fn resizing_the_south_edge_snaps_to_the_same_height_and_keeps_the_top() {
        let targets = [a()];
        // B 在 A 右边，top = 600：高度落在 [290, 310] 吸成 300（bottom 对齐 A.bottom 400 在上方，距离远）
        for height in [290, 296, 300, 305, 310] {
            let (rect, guides) = snap_resize(
                r(900, 600, 300, height),
                ResizeDir::S,
                &targets,
                10,
                160,
                160,
            );
            assert_eq!(rect, r(900, 600, 300, 300), "height = {height}");
            assert_eq!(
                guides,
                [g(1, SnapEdge::Left), g(1, SnapEdge::Right)],
                "等高 → 目标的左、右两边"
            );
        }
        let (rect, guides) =
            snap_resize(r(900, 600, 300, 311), ResizeDir::S, &targets, 10, 160, 160);
        assert_eq!(rect.height, 311);
        assert!(guides.is_empty());
    }

    #[test]
    fn resizing_the_east_edge_snaps_to_the_same_width_and_keeps_the_left() {
        let targets = [a()];
        for width in [390, 400, 410] {
            let (rect, guides) = snap_resize(
                r(100, 700, width, 200),
                ResizeDir::E,
                &targets,
                10,
                160,
                160,
            );
            assert_eq!(rect, r(100, 700, 400, 200), "width = {width}");
            assert_eq!(
                guides,
                [g(1, SnapEdge::Top), g(1, SnapEdge::Bottom)],
                "等宽 → 上、下两边"
            );
        }
    }

    #[test]
    fn resizing_the_south_edge_snaps_bottom_to_bottom() {
        let targets = [a()];
        // B.top = 150：B.bottom = 395（A.bottom 400 差 5）→ 高度 250；等高候选 300 差 55，不在阈值内
        let (rect, guides) =
            snap_resize(r(900, 150, 300, 245), ResizeDir::S, &targets, 10, 160, 160);
        assert_eq!(rect, r(900, 150, 300, 250));
        assert_eq!(rect.bottom(), 400);
        assert_eq!(guides, [g(1, SnapEdge::Bottom)]);
    }

    #[test]
    fn resizing_west_and_north_moves_only_the_dragged_edge() {
        let targets = [a()];
        // 拖 W 边：left 205 → 200，right（800）不动
        let (rect, guides) =
            snap_resize(r(205, 700, 595, 200), ResizeDir::W, &targets, 10, 160, 160);
        assert_eq!(rect, r(200, 700, 600, 200));
        assert_eq!(rect.right(), 800);
        assert_eq!(guides, [g(1, SnapEdge::Left)]);
        // 拖 N 边：top 93 → 100，bottom（500）不动
        let (rect, guides) =
            snap_resize(r(900, 93, 300, 407), ResizeDir::N, &targets, 10, 160, 160);
        assert_eq!(rect, r(900, 100, 300, 400));
        assert_eq!(rect.bottom(), 500);
        assert_eq!(guides, [g(1, SnapEdge::Top)]);
        // 拖 W 边等宽：right 不动，width 395 → 400
        let (rect, _) = snap_resize(r(1405, 700, 395, 200), ResizeDir::W, &targets, 10, 160, 160);
        assert_eq!(rect, r(1400, 700, 400, 200));
    }

    #[test]
    fn resizing_a_corner_snaps_both_axes() {
        let targets = [a()];
        // 从 (300, 150) 拖 SE：right 595 → 600（right 对齐，同时宽度 300 ≠ 400）、bottom 403 → 400
        let (rect, guides) =
            snap_resize(r(300, 150, 295, 253), ResizeDir::SE, &targets, 10, 160, 160);
        assert_eq!(rect, r(300, 150, 300, 250));
        assert_eq!(guides, [g(1, SnapEdge::Right), g(1, SnapEdge::Bottom)]);
    }

    #[test]
    fn resizing_takes_the_nearest_of_edge_and_size_candidates() {
        let targets = [a()];
        // 拖 E 边、left = 205：right 对齐 A.right 600 要宽 395；等宽 400。当前 398：等宽差 2、对齐差 -3 → 等宽
        let (rect, guides) =
            snap_resize(r(205, 700, 398, 100), ResizeDir::E, &targets, 10, 160, 160);
        assert_eq!(rect.width, 400);
        assert_eq!(guides, [g(1, SnapEdge::Top), g(1, SnapEdge::Bottom)]);
        // 当前 396：对齐差 -1 → 对齐
        let (rect, guides) =
            snap_resize(r(205, 700, 396, 100), ResizeDir::E, &targets, 10, 160, 160);
        assert_eq!(rect.width, 395);
        assert_eq!(guides, [g(1, SnapEdge::Right)]);
        // left 与 A 相同：对齐 right 和等宽是同一个结果，三条边都亮
        let (rect, guides) =
            snap_resize(r(200, 700, 404, 100), ResizeDir::E, &targets, 10, 160, 160);
        assert_eq!(rect.width, 400);
        assert_eq!(
            guides,
            [
                g(1, SnapEdge::Right),
                g(1, SnapEdge::Top),
                g(1, SnapEdge::Bottom)
            ]
        );
    }

    #[test]
    fn resizing_never_snaps_below_the_minimum_size() {
        // 目标高 150 < 最小 160：等高候选跳过；它的 bottom 对齐要高 155，也跳过
        let small = [target(7, r(1000, 100, 400, 150))];
        let (rect, guides) = snap_resize(r(200, 95, 300, 160), ResizeDir::S, &small, 10, 160, 160);
        assert_eq!(
            rect,
            r(200, 95, 300, 160),
            "不满足最小尺寸的候选跳过，保持跟手（已是最小尺寸）"
        );
        assert!(guides.is_empty());
        // 满足最小尺寸的候选照常：top = 80 时对齐 bottom 250 要高 170
        let (rect, guides) = snap_resize(r(200, 80, 300, 165), ResizeDir::S, &small, 10, 160, 160);
        assert_eq!(rect, r(200, 80, 300, 170));
        assert_eq!(guides, [g(7, SnapEdge::Bottom)]);
    }

    #[test]
    fn resize_snapping_runs_after_resize_rect() {
        // 端到端：从 B(900, 600, 300, 200) 拖 S 边 dy = 95 → resize_rect 给 295 → 吸成 300；对边（top）不动
        let start = r(900, 600, 300, 200);
        let raw = resize_rect(start, ResizeDir::S, 0., 95., 160., 160.);
        let (rect, _) = snap_resize(raw, ResizeDir::S, &[a()], 10, 160, 160);
        assert_eq!(rect, r(900, 600, 300, 300));
        // 拖 NW 角到最小尺寸以下：resize_rect 停在最小尺寸，吸附不会再缩小它
        let raw = resize_rect(start, ResizeDir::NW, 1000., 1000., 160., 160.);
        let (rect, _) = snap_resize(raw, ResizeDir::NW, &[a()], 10, 160, 160);
        assert!(rect.width >= 160 && rect.height >= 160);
        assert_eq!((rect.right(), rect.bottom()), (1200, 800), "右下角不被推走");
    }

    #[test]
    fn scene_keeps_only_targets_on_the_cursor_monitor() {
        let left = SnapMonitor {
            rect: r(0, 0, 1920, 1080),
            work: r(0, 0, 1920, 1040),
        };
        let right = SnapMonitor {
            rect: r(1920, 0, 2560, 1440),
            work: r(1920, 0, 2560, 1392),
        };
        let scene = SnapScene {
            targets: vec![
                target(1, r(100, 100, 400, 300)),
                // 跨两块屏，交叠大部分在右屏
                target(2, r(1800, 100, 400, 300)),
                target(3, r(2200, 500, 400, 300)),
            ],
            monitors: vec![left, right],
        };
        let ids = |cursor| -> Vec<i64> {
            scene
                .targets_at(cursor)
                .iter()
                .map(|t| t.fence_id)
                .collect()
        };
        assert_eq!(ids(Some((500, 500))), [1]);
        assert_eq!(ids(Some((3000, 700))), [2, 3]);
        assert_eq!(ids(None), [1, 2, 3], "取不到光标：不按显示器过滤");
        assert_eq!(
            ids(Some((-50, 0))),
            [1, 2, 3],
            "光标不在任何显示器上：不过滤"
        );
        let no_monitors = SnapScene {
            targets: scene.targets.clone(),
            monitors: Vec::new(),
        };
        assert_eq!(no_monitors.targets_at(Some((500, 500))).len(), 3);
    }

    #[test]
    fn guides_are_grouped_per_target_and_diffed() {
        let grouped = group_guides(&[
            g(1, SnapEdge::Top),
            g(2, SnapEdge::Left),
            g(1, SnapEdge::Bottom),
            g(1, SnapEdge::Top),
        ]);
        assert_eq!(grouped.len(), 2);
        let one = grouped[&1];
        assert!(one.contains(SnapEdge::Top) && one.contains(SnapEdge::Bottom));
        assert!(!one.contains(SnapEdge::Left) && !one.contains(SnapEdge::Right));
        assert!(grouped[&2].contains(SnapEdge::Left));
        assert!(SnapEdges::default().is_empty());
        assert!(group_guides(&[]).is_empty());

        // 只有变了的收纳箱要重绘：#1 不变、#2 消失、#3 新增
        let next = group_guides(&[
            g(1, SnapEdge::Top),
            g(1, SnapEdge::Bottom),
            g(3, SnapEdge::Right),
        ]);
        assert_eq!(changed_fences(&grouped, &next), [2, 3]);
        assert_eq!(changed_fences(&next, &next), Vec::<i64>::new());
        assert_eq!(
            changed_fences(&next, &HashMap::new()),
            [1, 3],
            "清空：之前亮着的都要重绘"
        );
    }
}
