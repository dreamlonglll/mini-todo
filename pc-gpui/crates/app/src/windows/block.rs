//! 桌面块：待办清单窗口（每个清单一个）与日历窗口（单例）。它们和收纳箱一样是独立的顶层窗口：各自锁定（固定模式）、
//! 各自按屏幕组合记几何，与收纳箱同一个吸附组。
//!
//! 任务 `11-09-multi-list-windows` 第 3 步只有现在的主窗口一个使用者（[`BlockKey::DEFAULT_LIST`]），界面上看不出拆分；
//! 拆成清单窗口 + 日历窗口是第 4 步。
//!
//! | 能力 | 位置 |
//! |---|---|
//! | 模式（普通 / 经典固定 / 嵌入桌面）、贴边隐藏、唤起置顶、几何保存 | `services::window_mode`（按 [`BlockKey`] 的状态表） |
//! | 几何与锁定的持久化（core 的窗口属性表 `window_layouts`，按屏幕组合） | `services::window_mode::store` |
//! | 吸附（与收纳箱同一组） | `services::snap` + `windows::snap` |
//! | 自绘移动与缩放 | 本模块的 [`BlockDrag`]（窗口根视图实现 [`BlockDragHost`]） |
//!
//! ## 自绘移动与缩放（[`BlockDrag`]）
//!
//! 与收纳箱同一套做法（`windows/fence/mod.rs`、spec `backend/desktop-fences.md`「移动」「缩放」「吸附」、gpui-conventions §2.3），
//! 去掉了收纳箱特有的折叠 / 悬停展开 / 双击改名，几何保存换成 `window_mode::save_window_state`（按 key 的持久化接缝）：
//!
//! - 不进系统的拖动循环（Windows `HTCAPTION` / macOS `performWindowDragWithEvent:` 到松手前改不了位置，吸附做不了）：窗口建窗时
//!   `resizable: false`（系统不再从边框缩放、不参与 Aero Snap），标题栏不放 `drag_region()`；
//! - 标题栏按下只记光标（屏幕物理坐标），移动超过 3 逻辑像素才进入移动会话（读客户区起点、快照吸附目标），之后 GPUI
//!   `on_drag_move`（捕获阶段、不看悬停；Windows 上按下时已 `SetCapture`，macOS 的 mouseDragged 一直发给按下的窗口）每次算
//!   "起点 + 光标位移"→ 吸附（按住 Alt / Option 不吸附）→ 每帧最多摆一次窗（移动只改位置），目标窗口上画高亮；
//! - 缩放用窗口内 8 个手柄（[`BlockDrag::resize_handles`]，不小于最小尺寸 × 系数），拖哪条边改哪条边、吸附只调被拖的边；
//! - 松手（标题栏 / 手柄的 `on_mouse_up` + `on_mouse_up_out`）→ 摆完最后一帧、清高亮 → 等 16ms（摆窗是延后执行的）直接保存一次；
//!   没收到松手（捕获被拿走、菜单吞了 mouseUp）时，不带左键的拖动移动 / 根元素上的新按下 / 光标回到窗口上的第一次移动都按松手收尾；
//! - 锁定（任一种固定）时不挂拖动、不显示手柄；标题栏上的按钮等可点击元素要 `.occlude()`（挡住标题栏，按它们不会开始拖动）。
//!
//! **手柄画在标题栏之后、内容之前**（与收纳箱不同）：标题栏区域的手柄在最上面（盖住标题栏），其余边上内容在手柄之上——
//! 列表滚动条的滑块（按下即 `stop_propagation`）仍能拖，滚动条轨道与内容留白处照常缩放；滑块自己声明了箭头光标
//! （`ui::scrollbar`），停在滑块上不再显示下面手柄的缩放光标。手柄不 `occlude()`：
//! 下面的根元素照常收到移动事件（FAB 变暗等）；按下手柄时 `stop_propagation`，标题栏不会同时开始拖动。

use std::fmt;
use std::time::Duration;

use gpui_kit::{
    AnyElement, AppContext as _, Context, Div, DragMoveEvent, InteractiveElement as _, IntoElement,
    MouseButton, MouseDownEvent, MouseMoveEvent, Pixels, Point, Stateful,
    StatefulInteractiveElement as _, Window,
};
use mini_todo_core::fence::PxRect;
use mini_todo_core::window_prefs::WindowKey;

use super::drag::{self, ResizeDir, js_round};
use super::snap::{self, SnapGuide, SnapKey, SnapScene};
use crate::platform;
use crate::services::snap as snap_service;
use crate::services::window_mode;

/// 默认清单的 id（core 迁移 v31 建的"待办清单"，id 固定为 1、不可删除；现在的主窗口就是它的窗口）
pub const DEFAULT_LIST_ID: i64 = 1;

/// 桌面块窗口的标识：`window_mode` 的状态表、几何与锁定的持久化接缝、吸附组共用。
///
/// 排序（`Ord`）：清单在前（按 id，默认清单 id 最小排第一），日历最后——轮询、重置位置按这个顺序处理，结果是确定的
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum BlockKey {
    /// 待办清单窗口（清单 id）
    List(i64),
    /// 日历窗口（单例）
    Calendar,
}

impl BlockKey {
    /// 默认清单窗口（= 现在的主窗口）
    pub const DEFAULT_LIST: BlockKey = BlockKey::List(DEFAULT_LIST_ID);

    /// core 窗口属性表（`window_prefs` / `window_layouts`）里的 key：`list:<清单 id>` / `calendar`
    pub fn window_key(self) -> WindowKey {
        match self {
            BlockKey::List(id) => WindowKey::List(id),
            BlockKey::Calendar => WindowKey::Calendar,
        }
    }

    pub fn from_window_key(key: WindowKey) -> BlockKey {
        match key {
            WindowKey::List(id) => BlockKey::List(id),
            WindowKey::Calendar => BlockKey::Calendar,
        }
    }

    /// 解析存储 / 登记用的 key 字符串（只认规范写法，见 `WindowKey::parse`）
    pub fn parse(key: &str) -> Option<BlockKey> {
        WindowKey::parse(key).map(BlockKey::from_window_key)
    }

    /// `WindowManager` 登记用的 key 字符串（与 core 存储的 key 相同）
    pub fn registry_key(self) -> String {
        self.window_key().as_key()
    }

    /// 清单窗口的清单 id
    pub fn list_id(self) -> Option<i64> {
        match self {
            BlockKey::List(id) => Some(id),
            BlockKey::Calendar => None,
        }
    }
}

impl fmt::Display for BlockKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BlockKey::List(id) => write!(f, "清单窗口 #{id}"),
            BlockKey::Calendar => f.write_str("日历窗口"),
        }
    }
}

// ----------------------------------------------------------------------
// 自绘移动与缩放
// ----------------------------------------------------------------------

/// 标题栏按下后移动超过这么多逻辑像素才算拖动（与收纳箱相同；不超过时按下 / 松手照常是单击）
const TITLE_DRAG_THRESHOLD: f64 = 3.;
/// 移动 / 缩放松手后等这么久再读客户区保存（摆窗是延后执行的，等最后一帧落地；与收纳箱的 `GEOMETRY_COMMIT_DELAY` 相同）
const COMMIT_DELAY: Duration = Duration::from_millis(16);

/// 最小客户区（物理像素）：逻辑最小尺寸 × 系数（`platform::physical_scale`；非法系数按 1）
pub fn min_physical_size(min_logical: (f64, f64), scale: f64) -> (i32, i32) {
    let scale = if scale.is_finite() && scale > 0. {
        scale
    } else {
        1.
    };
    (
        js_round(min_logical.0 * scale),
        js_round(min_logical.1 * scale),
    )
}

/// 桌面块窗口的根视图：拿到它的 [`BlockDrag`]（拖动的监听里要回到视图上改状态）
pub trait BlockDragHost: Sized + 'static {
    fn block_drag(&mut self) -> &mut BlockDrag;
}

/// 标题栏的拖动标记（移动）
#[derive(Clone, Copy)]
struct BlockTitleDrag;

/// 缩放手柄的拖动标记
#[derive(Clone, Copy)]
struct BlockResizeDrag(ResizeDir);

/// 标题栏按下（还没超过 3px，不一定是拖动：也可能是单击 / 双击改名）
#[derive(Debug, Clone, Copy)]
struct TitlePress {
    /// 按下点（窗口逻辑坐标；判断阈值时窗口还没动）
    position: Point<Pixels>,
    /// 按下时的光标（`drag::cursor_point`）
    cursor: (f64, f64),
    /// 是双击的第二下（系统的 `click_count >= 2`，且上一次按下没有拖动窗口）：松手时没移动超过 3px 就算双击
    /// （[`BlockDrag::is_double_click_release`]，同收纳箱的 `TitlePress.double`）
    double: bool,
}

impl TitlePress {
    /// `position`（窗口逻辑坐标）离按下点超过了 [`TITLE_DRAG_THRESHOLD`]
    fn moved_beyond_threshold(&self, position: Point<Pixels>) -> bool {
        let dx = f64::from(f32::from(position.x - self.position.x));
        let dy = f64::from(f32::from(position.y - self.position.y));
        drag::exceeds_drag_threshold(dx, dy, TITLE_DRAG_THRESHOLD)
    }
}

/// 进行中的移动
#[derive(Debug, Clone)]
struct MoveSession {
    start_cursor: (f64, f64),
    /// 开始移动时的客户区（物理像素）
    start: PxRect,
    /// 逻辑 → 物理像素的系数（`platform::physical_scale`）
    scale: f64,
    /// 吸附组里其它显示中的窗口与各显示器（开始时快照一次）
    scene: SnapScene,
}

/// 进行中的缩放
#[derive(Debug, Clone)]
struct ResizeSession {
    dir: ResizeDir,
    start_cursor: (f64, f64),
    /// 按下时的客户区（物理像素）
    start: PxRect,
    /// 逻辑 → 物理像素的系数（最小尺寸与吸附阈值都按它；缩放不跨屏）
    scale: f64,
    scene: SnapScene,
}

/// 下一帧要摆的窗
#[derive(Debug, Clone, Copy)]
enum PlacementTarget {
    /// 移动：只改位置（物理像素的客户区左上角），尺寸保持窗口当前值（混合 DPI 下跨屏时 `WM_DPICHANGED` 会改尺寸）
    Position(i32, i32),
    /// 缩放：整个客户区（物理像素）+ 系数（Linux 改尺寸用）
    Rect(PxRect, f64),
}

/// 拖动 / 缩放中等着下一帧执行的摆窗与高亮（每帧最多一次，只保留最新的）
#[derive(Debug, Clone)]
struct PendingPlacement {
    target: PlacementTarget,
    guides: Vec<SnapGuide>,
}

/// 一个桌面块窗口的自绘移动 / 缩放状态（窗口根视图持有一份，见模块文档）
pub struct BlockDrag {
    key: BlockKey,
    /// 最小客户区（逻辑像素）
    min_size: (f64, f64),
    press: Option<TitlePress>,
    /// 上一次标题栏按下拖动了窗口（这次按下即使 `click_count >= 2` 也不算双击）
    last_press_dragged: bool,
    moving: Option<MoveSession>,
    resizing: Option<ResizeSession>,
    pending: Option<PendingPlacement>,
    frame_scheduled: bool,
    /// 已松手、等最后一帧落地后保存
    commit_pending: bool,
    commit_epoch: u64,
}

impl BlockDrag {
    /// `min_width` / `min_height`：最小客户区（逻辑像素，与建窗时的 `min_size` 相同）
    pub fn new(key: BlockKey, min_width: f64, min_height: f64) -> Self {
        Self {
            key,
            min_size: (min_width, min_height),
            press: None,
            last_press_dragged: false,
            moving: None,
            resizing: None,
            pending: None,
            frame_scheduled: false,
            commit_pending: false,
            commit_epoch: 0,
        }
    }

    /// 用户正在移动 / 缩放窗口（含松手后等最后一帧落地、保存之前）：这期间窗口 bounds 的变化不另行防抖保存
    /// （松手后直接保存一次）
    pub fn is_busy(&self) -> bool {
        self.moving.is_some() || self.resizing.is_some() || self.commit_pending
    }

    /// 标题栏按住中（含还没超过 3px 的那一小段）或正在移动 / 缩放
    fn pointer_session(&self) -> bool {
        self.press.is_some() || self.moving.is_some() || self.resizing.is_some()
    }

    /// 标题栏上的元素（清单名）松手时调用：这次按下是双击的第二下、没有拖动窗口、松手点离按下点不超过 3px。
    /// 标题栏挂了 `on_drag`（移动超过 2px 就开始拖动、这次按下不再算单击），双击不能靠 GPUI 的 `on_click`
    /// （第二下手抖 2~3px 就收不到，gpui-conventions §2.3）；锁定时不挂拖动，照样能判断。
    /// 元素的 `on_mouse_up` 先于标题栏自己的（冒泡阶段子元素在前），这时按下还没被清掉
    pub fn is_double_click_release(&self, position: Point<Pixels>) -> bool {
        self.moving.is_none()
            && self
                .press
                .is_some_and(|p| p.double && !p.moved_beyond_threshold(position))
    }

    /// 标题栏挂上移动手势：按下记点（不立即拖）；`can_move`（未锁定）时挂 GPUI 拖动（超过 3px 才动）；
    /// 松手在不在标题栏上都收尾（窗口跟手有一帧延迟、吸附时窗口与光标有偏移）
    pub fn title_bar<V: BlockDragHost>(
        &self,
        bar: Stateful<Div>,
        can_move: bool,
        cx: &Context<V>,
    ) -> Stateful<Div> {
        let bar = bar.on_mouse_down(
            MouseButton::Left,
            cx.listener(|this: &mut V, event: &MouseDownEvent, window, _| {
                let block = this.block_drag();
                let after_drag = std::mem::take(&mut block.last_press_dragged);
                block.press = Some(TitlePress {
                    position: event.position,
                    cursor: drag::cursor_point(event.position, window).0,
                    double: event.click_count >= 2 && !after_drag,
                });
            }),
        );
        let bar = if can_move {
            bar.on_drag(BlockTitleDrag, |_, _, _, cx| {
                cx.new(|_| drag::EmptyDragView)
            })
            .on_drag_move::<BlockTitleDrag>(cx.listener(
                |this: &mut V, event: &DragMoveEvent<BlockTitleDrag>, window, cx| {
                    on_title_drag_move(this, &event.event, window, cx)
                },
            ))
        } else {
            bar
        };
        bar.on_mouse_up(
            MouseButton::Left,
            cx.listener(|this: &mut V, _, window, cx| {
                this.block_drag().press = None;
                end_move(this, window, cx);
            }),
        )
        .on_mouse_up_out(
            MouseButton::Left,
            cx.listener(|this: &mut V, _, window, cx| {
                this.block_drag().press = None;
                end_move(this, window, cx);
            }),
        )
    }

    /// 8 个缩放手柄（`can_resize`：未锁定）。放在根元素（`relative()`）里、标题栏之后、内容之前（见模块文档）
    pub fn resize_handles<V: BlockDragHost>(
        &self,
        can_resize: bool,
        cx: &Context<V>,
    ) -> Vec<AnyElement> {
        if !can_resize {
            return Vec::new();
        }
        ResizeDir::ALL
            .iter()
            .map(|&dir| {
                drag::resize_handle(dir)
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this: &mut V, event: &MouseDownEvent, window, cx| {
                            begin_resize(this, dir, event, window, cx)
                        }),
                    )
                    .on_drag(BlockResizeDrag(dir), |_, _, _, cx| {
                        cx.new(|_| drag::EmptyDragView)
                    })
                    .on_drag_move::<BlockResizeDrag>(cx.listener(
                        move |this: &mut V, event: &DragMoveEvent<BlockResizeDrag>, window, cx| {
                            // 8 个手柄都会收到同一次拖动：只由按下的那个处理
                            if event.drag(cx).0 == dir {
                                resize_move(this, &event.event, window, cx)
                            }
                        },
                    ))
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(|this: &mut V, _, window, cx| end_resize(this, window, cx)),
                    )
                    .on_mouse_up_out(
                        MouseButton::Left,
                        cx.listener(|this: &mut V, _, window, cx| end_resize(this, window, cx)),
                    )
                    .into_any_element()
            })
            .collect()
    }

    /// 根元素：没收到松手时收尾（Windows 上鼠标捕获被别的窗口拿走、macOS 的菜单吞了 mouseUp）——冒泡到根元素的新按下，
    /// 或 GPUI 的拖动已被别处的松手清掉（`active_drag` 是全应用的）后光标回到窗口上的第一次不带左键的移动
    pub fn root<V: BlockDragHost>(&self, root: Stateful<Div>, cx: &Context<V>) -> Stateful<Div> {
        root.on_any_mouse_down(cx.listener(|this: &mut V, _: &MouseDownEvent, window, cx| {
            // 标题栏的按下先于这里记下（子元素先收到），不受影响；缩放手柄的按下不冒泡到这里
            end_move(this, window, cx);
            end_resize(this, window, cx);
        }))
        .on_mouse_move(cx.listener(
            |this: &mut V, event: &MouseMoveEvent, window, cx| {
                if event.pressed_button != Some(MouseButton::Left)
                    && !cx.has_active_drag()
                    && this.block_drag().pointer_session()
                {
                    this.block_drag().press = None;
                    end_move(this, window, cx);
                    end_resize(this, window, cx);
                }
            },
        ))
    }
}

/// 标题栏拖动中的每次移动（GPUI `on_drag_move`：捕获阶段、不看悬停，光标出了窗口也收得到）
fn on_title_drag_move<V: BlockDragHost>(
    this: &mut V,
    event: &MouseMoveEvent,
    window: &mut Window,
    cx: &mut Context<V>,
) {
    if event.pressed_button != Some(MouseButton::Left) {
        // 没收到松手：按松手处理，并结束 GPUI 的拖动
        this.block_drag().press = None;
        end_move(this, window, cx);
        cx.stop_active_drag(window);
        return;
    }
    if this.block_drag().moving.is_none() && !begin_move_if_dragged(this, event, window, cx) {
        return;
    }
    let Some(session) = &this.block_drag().moving else {
        return;
    };
    let (dx, dy) = drag::cursor_delta(session.start_cursor, session.scale, event.position, window);
    // 移动只改位置，尺寸取窗口当前值：混合 DPI 下跨屏时 WM_DPICHANGED 改了尺寸，不能用起点的尺寸算吸附、摆回去
    let (width, height) = platform::client_rect_physical(window)
        .map_or((session.start.width, session.start.height), |r| {
            (r.width, r.height)
        });
    let raw = PxRect {
        x: session.start.x + js_round(dx),
        y: session.start.y + js_round(dy),
        width,
        height,
    };
    // 阈值按窗口当前的系数（混合 DPI 下拖到另一块屏上会变）
    let threshold = snap::snap_threshold(platform::physical_scale(window));
    let (rect, guides) = match snap_service::targets(&session.scene, event.modifiers.alt) {
        Some(targets) => snap::snap_move(raw, &targets, threshold),
        None => (raw, Vec::new()),
    };
    schedule_placement(
        this,
        PlacementTarget::Position(rect.x, rect.y),
        guides,
        window,
        cx,
    );
}

/// 按下后移动超过 3 逻辑像素（按窗口坐标判断，这时窗口还没动）才进入移动会话：读客户区起点、快照吸附目标。
/// 返回是否进入了移动
fn begin_move_if_dragged<V: BlockDragHost>(
    this: &mut V,
    event: &MouseMoveEvent,
    window: &mut Window,
    cx: &mut Context<V>,
) -> bool {
    let drag = this.block_drag();
    let Some(press) = drag.press else {
        return false;
    };
    let dx = f64::from(f32::from(event.position.x - press.position.x));
    let dy = f64::from(f32::from(event.position.y - press.position.y));
    if !drag::exceeds_drag_threshold(dx, dy, TITLE_DRAG_THRESHOLD) {
        return false;
    }
    drag.press = None;
    // 拖过的按下之后紧跟的一下不算双击（同收纳箱）
    drag.last_press_dragged = true;
    let key = drag.key;
    // 按住期间被锁定了（托盘「固定模式」）：不拖
    if window_mode::is_fixed(cx, key) {
        return false;
    }
    let Some(start) = platform::client_rect_physical(window) else {
        return false;
    };
    // 上一次拖动刚松手、还没保存：先保存
    flush_commit(this, cx);
    let scene = snap_service::scene(cx, SnapKey::Block(key));
    this.block_drag().moving = Some(MoveSession {
        start_cursor: press.cursor,
        start: to_px(start),
        scale: platform::physical_scale(window),
        scene,
    });
    true
}

/// 松手（或确认没收到松手）：摆完最后一帧、清掉高亮，等它落地后直接保存。没进入移动会话（没超过 3px）时什么都不做
fn end_move<V: BlockDragHost>(this: &mut V, window: &mut Window, cx: &mut Context<V>) {
    if this.block_drag().moving.take().is_none() {
        return;
    }
    flush_placement(this, window, cx);
    let key = this.block_drag().key;
    snap_service::clear_guides(cx, SnapKey::Block(key));
    commit_after_placement(this, cx);
}

fn begin_resize<V: BlockDragHost>(
    this: &mut V,
    dir: ResizeDir,
    event: &MouseDownEvent,
    window: &mut Window,
    cx: &mut Context<V>,
) {
    let key = this.block_drag().key;
    if this.block_drag().resizing.is_some() || window_mode::is_fixed(cx, key) {
        return;
    }
    // 上一次移动没收到松手（手柄的按下不冒泡到根元素的 on_any_mouse_down）：先收尾，再把待保存的保存掉
    this.block_drag().press = None;
    end_move(this, window, cx);
    flush_commit(this, cx);
    let Some(start) = platform::client_rect_physical(window) else {
        return;
    };
    let scene = snap_service::scene(cx, SnapKey::Block(key));
    this.block_drag().resizing = Some(ResizeSession {
        dir,
        start_cursor: drag::cursor_point(event.position, window).0,
        start: to_px(start),
        scale: platform::physical_scale(window),
        scene,
    });
    // 标题栏（手柄下面）与根元素不再收到这次按下：不会同时开始移动
    cx.stop_propagation();
}

fn resize_move<V: BlockDragHost>(
    this: &mut V,
    event: &MouseMoveEvent,
    window: &mut Window,
    cx: &mut Context<V>,
) {
    if event.pressed_button != Some(MouseButton::Left) {
        // 没收到松手（见 on_title_drag_move）
        end_resize(this, window, cx);
        cx.stop_active_drag(window);
        return;
    }
    let drag = this.block_drag();
    let Some(session) = &drag.resizing else {
        return;
    };
    let (dx, dy) = drag::cursor_delta(session.start_cursor, session.scale, event.position, window);
    let (min_w, min_h) = min_physical_size(drag.min_size, session.scale);
    let raw = drag::resize_rect(
        session.start,
        session.dir,
        dx,
        dy,
        f64::from(min_w),
        f64::from(min_h),
    );
    // 只调被拖的边、对边不动；吸附后不小于最小尺寸。阈值与最小尺寸同一个系数（缩放不跨屏）
    let threshold = snap::snap_threshold(session.scale);
    let (rect, guides) = match snap_service::targets(&session.scene, event.modifiers.alt) {
        Some(targets) => snap::snap_resize(raw, session.dir, &targets, threshold, min_w, min_h),
        None => (raw, Vec::new()),
    };
    let scale = session.scale;
    schedule_placement(this, PlacementTarget::Rect(rect, scale), guides, window, cx);
}

fn end_resize<V: BlockDragHost>(this: &mut V, window: &mut Window, cx: &mut Context<V>) {
    if this.block_drag().resizing.take().is_none() {
        return;
    }
    flush_placement(this, window, cx);
    let key = this.block_drag().key;
    snap_service::clear_guides(cx, SnapKey::Block(key));
    commit_after_placement(this, cx);
}

/// 拖动 / 缩放：每帧最多摆一次窗（`on_next_frame`，只保留最新的目标），目标窗口的高亮随同一帧更新
fn schedule_placement<V: BlockDragHost>(
    this: &mut V,
    target: PlacementTarget,
    guides: Vec<SnapGuide>,
    window: &mut Window,
    cx: &mut Context<V>,
) {
    let drag = this.block_drag();
    drag.pending = Some(PendingPlacement { target, guides });
    if !drag.frame_scheduled {
        drag.frame_scheduled = true;
        cx.on_next_frame(window, |this: &mut V, window, cx| {
            this.block_drag().frame_scheduled = false;
            flush_placement(this, window, cx);
        });
    }
}

/// 执行待摆的窗（`platform` 的写操作已延后到 App 借用之外）并更新高亮
fn flush_placement<V: BlockDragHost>(this: &mut V, window: &mut Window, cx: &mut Context<V>) {
    let drag = this.block_drag();
    let Some(pending) = drag.pending.take() else {
        return;
    };
    let key = drag.key;
    match pending.target {
        PlacementTarget::Position(x, y) => platform::set_client_position_physical(window, cx, x, y),
        PlacementTarget::Rect(rect, scale) => drag::place_client_rect(window, cx, rect, scale),
    }
    snap_service::set_guides(cx, SnapKey::Block(key), &pending.guides);
}

/// 移动 / 缩放结束：摆窗是延后执行的，等最后一帧落地（[`COMMIT_DELAY`]）再读客户区保存；等待期间又开始拖了，
/// 开始时先保存（[`flush_commit`]）
fn commit_after_placement<V: BlockDragHost>(this: &mut V, cx: &mut Context<V>) {
    let drag = this.block_drag();
    drag.commit_pending = true;
    drag.commit_epoch += 1;
    let epoch = drag.commit_epoch;
    cx.spawn(async move |this, cx| {
        cx.background_executor().timer(COMMIT_DELAY).await;
        let _ = this.update(cx, |this, cx| {
            let drag = this.block_drag();
            if drag.commit_pending && drag.commit_epoch == epoch {
                commit(this, cx);
            }
        });
    })
    .detach();
}

/// 保存窗口状态（当前屏幕组合下的几何与锁定，经 `window_mode::store`）
fn commit<V: BlockDragHost>(this: &mut V, cx: &mut Context<V>) {
    let drag = this.block_drag();
    drag.commit_pending = false;
    drag.commit_epoch += 1;
    let key = drag.key;
    window_mode::save_window_state(cx, key);
}

/// 有待保存的（松手后还在等最后一帧）立即保存
fn flush_commit<V: BlockDragHost>(this: &mut V, cx: &mut Context<V>) {
    if this.block_drag().commit_pending {
        commit(this, cx);
    }
}

fn to_px(r: platform::PhysRect) -> PxRect {
    PxRect {
        x: r.x,
        y: r.y,
        width: r.width,
        height: r.height,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::{point, px};

    #[test]
    fn only_an_unmoved_second_press_counts_as_a_double_click() {
        let mut drag = BlockDrag::new(BlockKey::List(7), 320., 400.);
        let press = |double: bool| TitlePress {
            position: point(px(10.), px(10.)),
            cursor: (10., 10.),
            double,
        };
        // 没有按下（拖动开始时已清掉）/ 单击：不算
        assert!(!drag.is_double_click_release(point(px(10.), px(10.))));
        drag.press = Some(press(false));
        assert!(!drag.is_double_click_release(point(px(10.), px(10.))));
        // 双击的第二下：手抖 2~3px 照样算（GPUI 的 on_click 在 2px 时就不算单击了），超过 3px 不算
        drag.press = Some(press(true));
        assert!(drag.is_double_click_release(point(px(12.), px(11.))));
        assert!(drag.is_double_click_release(point(px(13.), px(10.))));
        assert!(!drag.is_double_click_release(point(px(14.), px(10.))));
    }

    #[test]
    fn keys_order_the_default_list_first_and_the_calendar_last() {
        // 新清单 id 是 UNIX 毫秒 × 1000 + 随机数（与 cloud 同一算法），远大于默认清单的 1
        let mut keys = vec![
            BlockKey::Calendar,
            BlockKey::List(1_762_000_000_000_123),
            BlockKey::DEFAULT_LIST,
        ];
        keys.sort();
        assert_eq!(
            keys,
            vec![
                BlockKey::DEFAULT_LIST,
                BlockKey::List(1_762_000_000_000_123),
                BlockKey::Calendar
            ]
        );
        assert_eq!(BlockKey::DEFAULT_LIST, BlockKey::List(1));
        assert_eq!(BlockKey::DEFAULT_LIST.to_string(), "清单窗口 #1");
        assert_eq!(BlockKey::Calendar.to_string(), "日历窗口");
    }

    #[test]
    fn keys_map_to_the_core_window_keys() {
        for key in [
            BlockKey::DEFAULT_LIST,
            BlockKey::List(1_762_000_000_000_123),
            BlockKey::Calendar,
        ] {
            assert_eq!(BlockKey::from_window_key(key.window_key()), key);
            assert_eq!(BlockKey::parse(&key.registry_key()), Some(key));
        }
        assert_eq!(BlockKey::DEFAULT_LIST.registry_key(), "list:1");
        assert_eq!(BlockKey::Calendar.registry_key(), "calendar");
        assert_eq!(BlockKey::List(42).list_id(), Some(42));
        assert_eq!(BlockKey::Calendar.list_id(), None);
        assert_eq!(BlockKey::parse("fence-1"), None);
        assert_eq!(BlockKey::parse("list:0"), None);
    }

    /// 主窗口（默认清单窗口）的最小客户区：320×400 逻辑像素
    const MAIN_MIN: (f64, f64) = (320., 400.);

    fn r(x: i32, y: i32, width: i32, height: i32) -> PxRect {
        PxRect {
            x,
            y,
            width,
            height,
        }
    }

    #[test]
    fn minimum_size_follows_the_physical_scale() {
        assert_eq!(min_physical_size(MAIN_MIN, 1.), (320, 400));
        assert_eq!(min_physical_size(MAIN_MIN, 1.25), (400, 500));
        assert_eq!(min_physical_size(MAIN_MIN, 1.5), (480, 600));
        // 1.75：560 × 700
        assert_eq!(min_physical_size(MAIN_MIN, 1.75), (560, 700));
        assert_eq!(min_physical_size(MAIN_MIN, 0.), (320, 400), "非法系数按 1");
        assert_eq!(min_physical_size(MAIN_MIN, f64::NAN), (320, 400));
    }

    #[test]
    fn resizing_the_main_window_stops_at_its_minimum_and_keeps_the_far_edges() {
        let (min_w, min_h) = min_physical_size(MAIN_MIN, 1.);
        let (min_w, min_h) = (f64::from(min_w), f64::from(min_h));
        let start = r(100, 100, 380, 600);
        // 拖左边往右 200：宽到最小 320 就停，右边（480）不动
        let rect = drag::resize_rect(start, ResizeDir::W, 200., 0., min_w, min_h);
        assert_eq!(rect, r(160, 100, 320, 600));
        assert_eq!(rect.right(), start.right());
        // 拖上边往下 500：高到最小 400 就停，下边（700）不动
        let rect = drag::resize_rect(start, ResizeDir::N, 0., 500., min_w, min_h);
        assert_eq!(rect, r(100, 300, 380, 400));
        assert_eq!(rect.bottom(), start.bottom());
        // 拖右下角放大：左上角不动
        let rect = drag::resize_rect(start, ResizeDir::SE, 120., 80., min_w, min_h);
        assert_eq!(rect, r(100, 100, 500, 680));
        // 150%：最小 480 × 600 物理像素
        let (w, h) = min_physical_size(MAIN_MIN, 1.5);
        let rect = drag::resize_rect(
            r(0, 0, 570, 900),
            ResizeDir::SE,
            -1000.,
            -1000.,
            f64::from(w),
            f64::from(h),
        );
        assert_eq!((rect.width, rect.height), (480, 600));
    }

    #[test]
    fn resize_snapping_respects_the_main_window_minimum() {
        let (min_w, min_h) = min_physical_size(MAIN_MIN, 1.);
        // 收纳箱 A 高 300（< 主窗口最小高 400）、宽 400：拖主窗口 S 边时等高候选跳过；拖 E 边时等宽照常
        let fence = snap::SnapTarget {
            key: SnapKey::Fence(1),
            rect: r(200, 100, 400, 300),
        };
        let (rect, guides) = snap::snap_resize(
            r(900, 100, 380, 405),
            ResizeDir::S,
            &[fence],
            10,
            min_w,
            min_h,
        );
        assert_eq!(rect.height, 405);
        assert!(guides.is_empty());
        let (rect, guides) = snap::snap_resize(
            r(900, 700, 395, 600),
            ResizeDir::E,
            &[fence],
            10,
            min_w,
            min_h,
        );
        assert_eq!(rect.width, 400, "等宽");
        assert_eq!(guides.len(), 2, "目标的上、下两边");
    }
}
