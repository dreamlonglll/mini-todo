//! 吸附组的跨窗口运行时：清单窗口、日历窗口与收纳箱是同一个吸附组（任务 `11-09-multi-list-windows` R10，双向吸附）。
//! 原是 `services::fences` 里收纳箱专用的那部分（任务 `10-18-fence-snap`），规则不变；纯逻辑（吸附计算、按显示器筛目标、
//! 高亮归并）在 `windows::snap`。
//!
//! | 时机 | 调用 |
//! |---|---|
//! | 窗口建好 / 关闭（含删除收纳箱） | [`register`] / [`unregister`]（注销时它留下的、它身上的高亮一并清掉，不残留） |
//! | 拖动 / 缩放开始 | [`scene`]：快照其它参与者**屏幕上的可见客户区**（物理像素；隐藏 / 最小化 / 暂不算目标的跳过，锁定的照样算）+ 各显示器 |
//! | 每次移动 | [`targets`]：光标所在显示器上的目标；按住 Alt（macOS Option）时 `None`，完全不吸附 |
//! | 每帧摆窗 | [`set_guides`]：拖动中的窗口整体替换高亮，只通知高亮变了的目标窗口重绘（目标窗口渲染时 [`edges`] 读自己的那份） |
//! | 拖动 / 缩放结束 | [`clear_guides`] |
//!
//! 读 `Hwnd::is_visible` / `is_minimized` / `client_rect`（Windows `IsWindowVisible` / `IsIconic` / `GetClientRect` + `ClientToScreen`，
//! macOS 读 frame）：只读、不发窗口消息、不触发 GPUI 回调，可以在事件处理里直接调（gpui-conventions §2.2）。Linux 没有原生句柄，
//! 没有目标。

use std::collections::HashMap;

use gpui_kit::{App, EntityId, Global};
use mini_todo_core::fence::PxRect;

use crate::platform::{self, Hwnd, PhysRect};
use crate::windows::snap::{
    self, SnapEdges, SnapGuide, SnapKey, SnapMonitor, SnapScene, SnapTarget,
};

/// 吸附组的一个参与者
#[derive(Clone, Copy)]
pub struct Member {
    /// 原生窗口（读可见性与客户区；Linux 没有：不当目标）
    pub hwnd: Option<Hwnd>,
    /// 根视图（高亮变了只通知它重绘；按实体 id 通知、不借用视图：调用方正在自己视图的 update 里）
    pub view: EntityId,
    /// 暂时不算目标（收纳箱「全部隐藏」中、清单 / 日历窗口贴边藏起中）。窗口真被隐藏 / 最小化另由 `Hwnd` 判断
    pub suppressed: fn(&App, SnapKey) -> bool,
}

/// 吸附组运行时（Global）
#[derive(Default)]
struct SnapRuntime {
    members: HashMap<SnapKey, Member>,
    /// 目标窗口 → 要高亮的边。由正在拖动 / 缩放的窗口（`owner`）每帧整体替换；同一时刻只有一个在拖
    highlights: HashMap<SnapKey, SnapEdges>,
    owner: Option<SnapKey>,
}

impl Global for SnapRuntime {}

fn runtime(cx: &mut App) -> &mut SnapRuntime {
    cx.default_global::<SnapRuntime>()
}

/// 窗口建好后加入吸附组（同一个 key 再登记就替换）
pub fn register(cx: &mut App, key: SnapKey, member: Member) {
    runtime(cx).members.insert(key, member);
}

/// 窗口关闭（含删除收纳箱）：它在拖动中就清掉全部高亮；它是目标就只去掉它那一项（窗口已经没了，不用通知）
pub fn unregister(cx: &mut App, key: SnapKey) {
    clear_guides(cx, key);
    let rt = runtime(cx);
    rt.members.remove(&key);
    rt.highlights.remove(&key);
}

fn to_px(r: PhysRect) -> PxRect {
    PxRect {
        x: r.x,
        y: r.y,
        width: r.width,
        height: r.height,
    }
}

/// 当前显示中的参与者（除 `exclude`；含锁定的，不含隐藏 / 最小化 / 暂不算目标的）屏幕上的可见客户区，按 [`SnapKey`] 排序
fn visible_targets(cx: &App, exclude: Option<SnapKey>) -> Vec<SnapTarget> {
    let Some(rt) = cx.try_global::<SnapRuntime>() else {
        return Vec::new();
    };
    let mut targets: Vec<SnapTarget> = rt
        .members
        .iter()
        .filter(|(key, _)| Some(**key) != exclude)
        .filter(|(key, member)| !(member.suppressed)(cx, **key))
        .filter_map(|(key, member)| {
            let h = member.hwnd?;
            if !h.is_visible() || h.is_minimized() {
                return None;
            }
            Some(SnapTarget {
                key: *key,
                rect: to_px(h.client_rect()?),
            })
        })
        .collect();
    // HashMap 的顺序不确定：按 key 排，同距离时取哪个才是确定的
    targets.sort_by_key(|t| t.key);
    targets
}

/// 吸附组里所有显示中的窗口（清单 / 日历窗口、收纳箱）屏幕上的可见客户区（物理像素）：新清单窗口找空位时避开它们
pub fn visible_rects(cx: &App) -> Vec<PxRect> {
    visible_targets(cx, None)
        .into_iter()
        .map(|t| t.rect)
        .collect()
}

/// 拖动 / 缩放开始时的吸附快照：除 `exclude`（拖动中的窗口自己）外，当前显示中的参与者（含锁定的）屏幕上的可见客户区
/// （物理像素；折叠的收纳箱只有标题栏），按 [`SnapKey`] 排序，加上各显示器
pub fn scene(cx: &App, exclude: SnapKey) -> SnapScene {
    if cx.try_global::<SnapRuntime>().is_none() {
        return SnapScene::default();
    }
    let targets = visible_targets(cx, Some(exclude));
    let monitors = platform::monitors(cx)
        .into_iter()
        .map(|m| SnapMonitor {
            rect: to_px(m.rect),
            work: to_px(if m.work.width > 0 && m.work.height > 0 {
                m.work
            } else {
                m.rect
            }),
        })
        .collect();
    SnapScene { targets, monitors }
}

/// 这次移动用的目标：光标所在显示器上的（`SnapScene::targets_at`）；按住 Alt（macOS Option，GPUI `modifiers.alt`）时
/// `None`——完全不吸附、不出高亮。按每次移动事件当时的修饰键判断，松开后下一次移动恢复
pub fn targets(scene: &SnapScene, alt: bool) -> Option<Vec<SnapTarget>> {
    (!alt).then(|| scene.targets_at(platform::cursor_position_physical()))
}

/// 正在拖动 / 缩放的窗口 `owner` 这一帧的吸附高亮（空 = 没吸上 / 按着 Alt）。只通知高亮变了的目标窗口重绘
pub fn set_guides(cx: &mut App, owner: SnapKey, guides: &[SnapGuide]) {
    let next = snap::group_guides(guides);
    let rt = runtime(cx);
    let changed = snap::changed_targets(&rt.highlights, &next);
    rt.owner = (!next.is_empty()).then_some(owner);
    rt.highlights = next;
    let views: Vec<EntityId> = changed
        .iter()
        .filter_map(|key| rt.members.get(key).map(|m| m.view))
        .collect();
    for view in views {
        // 按实体 id 通知（不借用视图）：调用方正在自己视图的 update 里，残留的高亮万一在它自己身上也不会重入
        cx.notify(view);
    }
}

/// 拖动 / 缩放结束：清掉 `owner` 留下的高亮（别的窗口留下的不动）
pub fn clear_guides(cx: &mut App, owner: SnapKey) {
    let owned = cx
        .try_global::<SnapRuntime>()
        .is_some_and(|rt| rt.owner == Some(owner));
    if owned {
        set_guides(cx, owner, &[]);
    }
}

/// 窗口 `key` 现在要高亮的边（目标窗口渲染时读）
pub fn edges(cx: &App, key: SnapKey) -> SnapEdges {
    cx.try_global::<SnapRuntime>()
        .and_then(|rt| rt.highlights.get(&key).copied())
        .unwrap_or_default()
}
