//! 桌面收纳箱（10-15 desktop-fence）：元数据 CRUD + 窗口生命周期 + 按屏幕组合记忆位置。
//!
//! 每个收纳箱是一个独立的无边框透明 WebView 窗口（label `fence-{id}`，路由 `#/fence?id={id}`），
//! **始终嵌入桌面层**：建窗时一次性给齐 tao flag（always_on_bottom / minimizable(false) /
//! maximizable(false) / skip_taskbar / visible），之后由 window.rs「始终嵌入桌面的附属窗口」
//! 挂 owner=Progman + `WS_EX_TOOLWINDOW` + `HWND_BOTTOM`，200ms 轮询线程负责 Explorer 重启后重挂。
//! 建窗后不再改任何 tao flag，显示 / 隐藏只走 tao `show()` / `hide()`（并紧跟重申嵌入）。
//!
//! 「固定」= 锁定位置和大小（prd R2），完全由前端控制：不调 `startDragging`、不显示缩放手柄。
//! 窗口本身建成 `resizable(false)`（没有 `WS_THICKFRAME`，tauri 也不挂原生缩放边框），未锁定时由前端
//! 缩放手柄捕获指针、逐帧调 [`set_fence_bounds`]，所以锁定 / 解锁都不需要碰 tao flag。
//!
//! 几何信息存物理像素：`x` / `y` 为虚拟屏幕坐标（折叠态即标题栏位置），`width` / `height` 为展开态
//! 尺寸。只有用户拖动 / 缩放结束后前端调 [`commit_fence_geometry`] 才落库；启动时夹回可见工作区、
//! 显示器变化导致的系统挪窗都不落库。恢复 / 折叠 / 展开一律先 `set_position` 再 `set_size`。
//!
//! 按屏幕组合记忆位置（PRD R5）：几何按屏幕组合（与主窗口 `screen_configs.config_id` 同一格式，
//! [`screen_config_id_of`]）分别存进 `fence_layouts`，`fences.x/y/width/height` 是"最近一次位置"。
//! 当前组合有记录就用记录，没有就用最近一次位置（[`Placement`]）。轮询线程约每 2 秒检测一次屏幕组合
//! （[`tick_fence_screen_layouts`]），变化后把各收纳箱恢复到新组合下的记录；没有记录的沿用当前位置并
//! 夹回可见区域，只记在本次运行的内存里（不落库）。
//!
//! 文件层（`services::fence_fs`）：新建收纳箱先建收纳文件夹再插库（插库失败回滚文件夹）、改标题同步改
//! 文件夹名、删除收纳箱先把项目全部移回桌面、启动恢复时确保文件夹存在。收纳箱**内容**（列举对账、移入 /
//! 移回 / 跨箱、打开、图标、排序、目录监听）在 `fence_items.rs`。

use crate::db::{fence_from_row, Database, Fence, UpdateFenceRequest, FENCE_COLUMNS};
use crate::services::fence_fs::{self, ItemOpResult, ItemOpStatus};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard};
use tauri::{
    AppHandle, LogicalSize, Manager, PhysicalPosition, PhysicalSize, State, WebviewUrl,
    WebviewWindow, WebviewWindowBuilder,
};

use super::fence_items::{
    begin_deleting, blocking, delete_item, ensure_fence_watcher, fence_originals, is_deleting,
    is_moving, owner_hwnd, reconcile_and_notify, spawn_startup_reconcile,
};

use super::window::{
    embed_window_in_desktop, reassert_embedded_window, release_embedded_window,
    set_window_bounds_physical,
};

/// 收纳箱窗口 label 前缀（info.md 2.1：`fence-{id}`）
pub const FENCE_LABEL_PREFIX: &str = "fence-";

/// 标题栏高度（逻辑像素），与 `pc/src/types/fence.ts` 的 `FENCE_TITLE_BAR_HEIGHT` 一致。
/// 折叠态窗口高度 = 它 × 缩放比例
const FENCE_TITLE_BAR_DIP: f64 = 32.0;
/// 默认图标格（逻辑像素），参考 LucidDesk 的 88×96 DIP
const CELL_WIDTH_DIP: f64 = 88.0;
const CELL_HEIGHT_DIP: f64 = 96.0;
/// 新建收纳箱默认能放 4 列 × 3 行图标
const DEFAULT_COLUMNS: f64 = 4.0;
const DEFAULT_ROWS: f64 = 3.0;
/// 内容区内边距（逻辑像素），与 FenceView 的内容区 padding 一致
const CONTENT_PADDING_DIP: f64 = 8.0;
/// 最小宽度 / 展开态最小高度（逻辑像素）：展开态至少要放得下标题栏菜单
const MIN_WIDTH_DIP: f64 = 160.0;
const MIN_EXPANDED_HEIGHT_DIP: f64 = 160.0;
/// 新建 / 重置时离工作区边缘的距离、收纳箱之间的间距、叠放偏移（逻辑像素）
const SCREEN_MARGIN_DIP: f64 = 24.0;
const LAYOUT_GAP_DIP: f64 = 16.0;
const CASCADE_STEP_DIP: f64 = 32.0;

/// 默认标题；重名时加序号（"收纳箱 2"、"收纳箱 3"……）
const DEFAULT_FENCE_TITLE: &str = "收纳箱";
/// 标题最大长度（字符数），与前端输入框 maxlength 一致
const MAX_TITLE_CHARS: usize = 40;
/// 图标大小上下限（逻辑像素），Ctrl+滚轮在此范围内调整
const MIN_ICON_SIZE: i32 = 24;
const MAX_ICON_SIZE: i32 = 128;

/// 托盘「显示/隐藏所有收纳箱」的运行时状态（不持久化：重启后收纳箱总是显示）
static FENCES_HIDDEN: AtomicBool = AtomicBool::new(false);

/// 无显示器信息时的屏幕组合 ID（与前端 `generateScreenConfigId()` 一致）
const UNKNOWN_SCREEN_CONFIG: &str = "unknown";

/// 本次运行中的临时几何（按收纳箱 id）：屏幕组合切换到一个没有记录的组合后"沿用当前位置"，不落库。
/// 用户拖动 / 缩放、重置位置后写入了当前组合的记录就移除；屏幕组合再次变化时整体重算
static SESSION_RECTS: Mutex<BTreeMap<i64, PxRect>> = Mutex::new(BTreeMap::new());

/// 轮询线程上一次看到的屏幕组合 ID（`None` = 还没有收纳箱窗口 / 尚未初始化）
static LAST_SCREEN_ID: Mutex<Option<String>> = Mutex::new(None);

fn session_rects() -> MutexGuard<'static, BTreeMap<i64, PxRect>> {
    SESSION_RECTS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn last_screen_id() -> MutexGuard<'static, Option<String>> {
    LAST_SCREEN_ID
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

// ============ 纯函数（几何 / 命名），有单元测试 ============

/// 物理像素矩形（虚拟屏幕坐标）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PxRect {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
}

impl PxRect {
    fn right(self) -> i32 {
        self.x + self.width
    }

    fn bottom(self) -> i32 {
        self.y + self.height
    }

    fn intersection_area(self, other: PxRect) -> i64 {
        let w = (self.right().min(other.right()) - self.x.max(other.x)).max(0) as i64;
        let h = (self.bottom().min(other.bottom()) - self.y.max(other.y)).max(0) as i64;
        w * h
    }

    fn inflate(self, by: i32) -> PxRect {
        PxRect {
            x: self.x - by,
            y: self.y - by,
            width: self.width + 2 * by,
            height: self.height + 2 * by,
        }
    }

    /// 点到矩形的距离平方（点在矩形内为 0）
    fn distance_sq_to(self, px: i32, py: i32) -> i64 {
        let dx = (self.x - px).max(px - self.right()).max(0) as i64;
        let dy = (self.y - py).max(py - self.bottom()).max(0) as i64;
        dx * dx + dy * dy
    }
}

fn stored_rect(fence: &Fence) -> PxRect {
    PxRect {
        x: fence.x,
        y: fence.y,
        width: fence.width,
        height: fence.height,
    }
}

/// 屏幕组合 ID，与前端 `appStore.generateScreenConfigId()` 逐字一致（主窗口 `screen_configs.config_id`
/// 同一格式）：`{显示器数量}_{宽}x{高}@{缩放%}_…`。
///
/// - 输入为 `available_monitors()` 的枚举顺序：（物理像素宽, 高, 缩放比例）
/// - 缩放% = `round(scale × 100)`：缩放比例为正数，Rust `f64::round`（逢半远离零）与 JS `Math.round`
///   （逢半向 +∞）结果相同
/// - 按像素面积降序；稳定排序，同面积保持枚举顺序（与 JS `Array.prototype.sort` 一致）
/// - 没有显示器时为 `unknown`
fn screen_config_id_of(monitors: &[(u32, u32, f64)]) -> String {
    if monitors.is_empty() {
        return UNKNOWN_SCREEN_CONFIG.to_string();
    }
    let mut infos: Vec<(u32, u32, i64)> = monitors
        .iter()
        .map(|&(w, h, scale)| (w, h, (scale * 100.0).round() as i64))
        .collect();
    // sort_by_key 是稳定排序
    infos.sort_by_key(|&(w, h, _)| std::cmp::Reverse(u64::from(w) * u64::from(h)));
    let parts: Vec<String> = infos
        .iter()
        .map(|(w, h, scale)| format!("{w}x{h}@{scale}"))
        .collect();
    format!("{}_{}", monitors.len(), parts.join("_"))
}

/// 收纳箱在当前屏幕组合下应使用的几何：该组合的记录 → 本次运行的临时几何 → 最近一次位置
fn effective_rect(
    fence: &Fence,
    layouts: &HashMap<i64, PxRect>,
    session: &BTreeMap<i64, PxRect>,
) -> PxRect {
    layouts
        .get(&fence.id)
        .or_else(|| session.get(&fence.id))
        .copied()
        .unwrap_or_else(|| stored_rect(fence))
}

/// 屏幕组合切换后收纳箱的几何：新组合有记录就用记录；没有就沿用窗口当前位置（调用方随后夹回可见区域）。
/// 窗口当前只显示标题栏（`collapsed_view`）时，展开高度沿用切换前的 `expanded_height`
fn layout_after_switch(
    record: Option<PxRect>,
    window: PxRect,
    expanded_height: i32,
    collapsed_view: bool,
) -> PxRect {
    record.unwrap_or(PxRect {
        height: if collapsed_view {
            expanded_height
        } else {
            window.height
        },
        ..window
    })
}

/// 拖动 / 缩放结束时要落库的几何：窗口只剩标题栏（折叠态）时保留当前的展开高度
fn committed_rect(window: PxRect, expanded_view: bool, current: PxRect) -> PxRect {
    PxRect {
        height: if expanded_view {
            window.height
        } else {
            current.height
        },
        ..window
    }
}

/// 下一个默认标题："收纳箱" 没被占用就用它，否则从 2 开始找第一个空闲序号
fn next_default_title(existing: &[String]) -> String {
    let taken = |title: &str| existing.iter().any(|t| t.trim() == title);
    if !taken(DEFAULT_FENCE_TITLE) {
        return DEFAULT_FENCE_TITLE.to_string();
    }
    (2..)
        .map(|n| format!("{DEFAULT_FENCE_TITLE} {n}"))
        .find(|title| !taken(title))
        .unwrap_or_else(|| DEFAULT_FENCE_TITLE.to_string())
}

/// 新收纳箱的位置：工作区内、从右上角开始找一块不与已有收纳箱重叠的地方（桌面图标默认从左上角
/// 往下排，从右上角放可以少挡图标）；找不到就从右上角起斜向叠放，只避开与已有收纳箱左上角重合的
/// 位置（"相对已有收纳箱偏移、避免完全重叠"）。尺寸大于工作区时缩到工作区内。
fn default_fence_rect(
    work: PxRect,
    width: i32,
    height: i32,
    existing: &[PxRect],
    margin: i32,
    step: i32,
) -> PxRect {
    let step = step.max(1);
    let width = width.min(work.width - 2 * margin).max(1);
    let height = height.min(work.height - 2 * margin).max(1);
    let min_x = work.x + margin;
    let min_y = work.y + margin;
    let max_x = (work.right() - margin - width).max(min_x);
    let max_y = (work.bottom() - margin - height).max(min_y);
    let at = |x: i32, y: i32| PxRect {
        x,
        y,
        width,
        height,
    };

    // 第一轮：不重叠（留出半个间距的空隙）
    let gap = margin / 2;
    let mut y = min_y;
    while y <= max_y {
        let mut x = max_x;
        while x >= min_x {
            let candidate = at(x, y).inflate(gap);
            if existing.iter().all(|r| r.intersection_area(candidate) == 0) {
                return at(x, y);
            }
            x -= step;
        }
        y += step;
    }

    // 第二轮：斜向叠放。一条斜线放满后整体左移三个步长再起一条
    let per_diagonal = ((max_y - min_y) / step + 1).max(1);
    let diagonals = ((max_x - min_x) / (step * 3) + 1).max(1);
    for k in 0..(per_diagonal * diagonals).min(512) {
        let (series, d) = (k / per_diagonal, k % per_diagonal);
        let x = (max_x - series * step * 3 - d * step).max(min_x);
        let y = (min_y + d * step).min(max_y);
        let same_corner = existing
            .iter()
            .any(|r| (r.x - x).abs() < step / 2 + 1 && (r.y - y).abs() < step / 2 + 1);
        if !same_corner {
            return at(x, y);
        }
    }
    at(max_x, min_y)
}

/// 选出矩形所在的工作区：交叠面积最大的那块；一块都不交叠（显示器被拔掉、坐标跑到屏幕外）时
/// 取离矩形中心最近的一块。`works` 为空时返回 `None`。
fn pick_work_area(probe: PxRect, works: &[PxRect]) -> Option<PxRect> {
    let best = works
        .iter()
        .copied()
        .max_by_key(|w| w.intersection_area(probe))?;
    if best.intersection_area(probe) > 0 {
        return Some(best);
    }
    let (cx, cy) = (probe.x + probe.width / 2, probe.y + probe.height / 2);
    works
        .iter()
        .copied()
        .min_by_key(|w| w.distance_sq_to(cx, cy))
}

/// 把矩形平移进工作区（尺寸不变；比工作区还大时贴左上角）
fn clamp_into(rect: PxRect, work: PxRect) -> PxRect {
    let clamp_axis = |value: i32, size: i32, start: i32, end: i32| {
        let max = end - size;
        if max < start {
            start
        } else {
            value.clamp(start, max)
        }
    };
    PxRect {
        x: clamp_axis(rect.x, rect.width, work.x, work.right()),
        y: clamp_axis(rect.y, rect.height, work.y, work.bottom()),
        ..rect
    }
}

/// 收纳箱窗口当前应占的矩形。
///
/// - 折叠态：标题栏（`x`, `y`, `width`, `title_height`）
/// - 展开态：库里的展开尺寸；下方空间不够时整体上移（向上展开），标题栏原来的位置仍在窗口内，
///   悬停展开时鼠标不会因此离开窗口
///
/// 两种状态都按标题栏所在的工作区夹回可见范围（启动恢复、显示器变化后不会跑到屏幕外）。
fn view_rect(stored: PxRect, expanded: bool, title_height: i32, works: &[PxRect]) -> PxRect {
    let title_rect = PxRect {
        height: title_height,
        ..stored
    };
    let target = if expanded { stored } else { title_rect };
    match pick_work_area(title_rect, works) {
        Some(work) => clamp_into(target, work),
        None => target,
    }
}

/// 一块显示器：工作区（物理像素）与缩放比例
#[derive(Debug, Clone, Copy, PartialEq)]
struct MonitorArea {
    work: PxRect,
    scale: f64,
}

/// 标题栏所在显示器（选法同 [`pick_work_area`]）的缩放比例；没有显示器信息时按 1.0。
///
/// 折叠态高度、建窗时的逻辑坐标都要按**目标位置**所在显示器的缩放算，不能取窗口当前所在
/// 显示器的：混合 DPI 下从一块屏挪到另一块屏（重置位置、启动夹回）时两者不同。
fn title_scale(stored: PxRect, areas: &[MonitorArea]) -> f64 {
    let works: Vec<PxRect> = areas.iter().map(|a| a.work).collect();
    let probe = PxRect {
        height: FENCE_TITLE_BAR_DIP.round() as i32,
        ..stored
    };
    pick_work_area(probe, &works)
        .and_then(|work| areas.iter().find(|a| a.work == work))
        .map(|a| a.scale)
        .filter(|s| *s > 0.0)
        .unwrap_or(1.0)
}

/// 收纳箱窗口应占的矩形（见 [`view_rect`]）与目标显示器的缩放比例
fn fence_view(stored: PxRect, expanded: bool, areas: &[MonitorArea]) -> (PxRect, f64) {
    let scale = title_scale(stored, areas);
    let works: Vec<PxRect> = areas.iter().map(|a| a.work).collect();
    (
        view_rect(stored, expanded, title_bar_px(scale), &works),
        scale,
    )
}

/// 托盘「重置收纳箱位置」：在工作区里从右上角起从右往左、逐行排开，放不下时回到右上角错开叠放。
/// 返回每个收纳箱的新左上角（尺寸由调用方先夹到工作区内）。
fn reset_layout(work: PxRect, sizes: &[(i32, i32)], margin: i32, gap: i32) -> Vec<(i32, i32)> {
    let right = work.right() - margin;
    let left = work.x + margin;
    let top = work.y + margin;
    let bottom = work.bottom() - margin;
    let mut cursor_right = right;
    let mut y = top;
    let mut row_height = 0;
    let mut restarts = 0;
    let mut out = Vec::with_capacity(sizes.len());
    for &(w, h) in sizes {
        if cursor_right - w < left && cursor_right < right {
            cursor_right = right;
            y += row_height + gap;
            row_height = 0;
        }
        if y + h > bottom && y > top {
            restarts += 1;
            cursor_right = right - restarts * gap * 2;
            y = top + restarts * gap * 2;
            row_height = 0;
        }
        let rect = clamp_into(
            PxRect {
                x: cursor_right - w,
                y,
                width: w,
                height: h,
            },
            work,
        );
        out.push((rect.x, rect.y));
        cursor_right = rect.x - gap;
        row_height = row_height.max(h);
    }
    out
}

/// 规范化标题：去首尾空白、控制字符换成空格；不能为空、不超过 [`MAX_TITLE_CHARS`] 个字符
fn normalize_title(raw: &str) -> Result<String, String> {
    let cleaned: String = raw
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let title = cleaned.trim();
    if title.is_empty() {
        return Err("收纳箱标题不能为空".to_string());
    }
    if title.chars().count() > MAX_TITLE_CHARS {
        return Err(format!("收纳箱标题不能超过 {MAX_TITLE_CHARS} 个字符"));
    }
    Ok(title.to_string())
}

/// 背景色只接受 `#RRGGBB`，统一成大写
fn normalize_color(raw: &str) -> Result<String, String> {
    let value = raw.trim();
    let hex = value.strip_prefix('#').unwrap_or("");
    if hex.len() == 6 && hex.chars().all(|c| c.is_ascii_hexdigit()) {
        Ok(format!("#{}", hex.to_ascii_uppercase()))
    } else {
        Err(format!("背景色格式无效：{value}"))
    }
}

// ============ 数据库 ============

fn query_fence(conn: &rusqlite::Connection, id: i64) -> rusqlite::Result<Option<Fence>> {
    match conn.query_row(
        &format!("SELECT {FENCE_COLUMNS} FROM fences WHERE id = ?1"),
        [id],
        fence_from_row,
    ) {
        Ok(fence) => Ok(Some(fence)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e),
    }
}

pub(crate) fn load_fence(db: &Database, id: i64) -> Result<Fence, String> {
    db.with_connection(|conn| query_fence(conn, id))
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("收纳箱 #{id} 不存在"))
}

pub(crate) fn load_fences(db: &Database) -> Result<Vec<Fence>, String> {
    db.with_connection(|conn| {
        let mut stmt = conn.prepare(&format!(
            "SELECT {FENCE_COLUMNS} FROM fences ORDER BY sort_order, id"
        ))?;
        let rows = stmt.query_map([], fence_from_row)?;
        rows.collect()
    })
    .map_err(|e| e.to_string())
}

/// 插入收纳箱记录。`folder_name` 是已经建好的收纳文件夹名（`fence_fs::create_fence_folder` 的返回值）
fn insert_fence(
    db: &Database,
    title: &str,
    rect: PxRect,
    folder_name: &str,
) -> Result<Fence, String> {
    db.with_transaction(|tx| -> rusqlite::Result<Fence> {
        let sort_order: i32 = tx.query_row(
            "SELECT COALESCE(MAX(sort_order), -1) + 1 FROM fences",
            [],
            |r| r.get(0),
        )?;
        tx.execute(
            "INSERT INTO fences (title, folder_name, x, y, width, height, sort_order, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, datetime('now', 'localtime'), datetime('now', 'localtime'))",
            rusqlite::params![
                title,
                folder_name,
                rect.x,
                rect.y,
                rect.width,
                rect.height,
                sort_order
            ],
        )?;
        let id = tx.last_insert_rowid();
        query_fence(tx, id)?.ok_or(rusqlite::Error::QueryReturnedNoRows)
    })
    .map_err(|e| format!("保存收纳箱失败: {e}"))
}

/// 全部收纳箱的 `folder_name`：新建 / 改名时去重要避开它们（即使对应文件夹已被用户删掉，防 UNIQUE 冲突）
fn all_folder_names(conn: &rusqlite::Connection) -> rusqlite::Result<Vec<String>> {
    let mut stmt = conn.prepare("SELECT folder_name FROM fences")?;
    let rows = stmt.query_map([], |r| r.get(0))?;
    rows.collect()
}

fn delete_fence_row(db: &Database, id: i64) -> Result<(), String> {
    db.with_connection(|conn| conn.execute("DELETE FROM fences WHERE id = ?1", [id]))
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// `rename_fence` 的结果
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RenameFenceResult {
    pub fence: Fence,
    /// 收纳文件夹没能跟着改名的原因（标题照改、保留旧文件夹名）；`None` 表示文件夹已同步或无需改动
    pub folder_error: Option<String>,
}

/// 改标题并同步改收纳文件夹名（`rename_folder` 即 `fence_fs::rename_fence_folder`，测试时可替换）。
///
/// 整个过程持有数据库锁：收纳箱内容的对账也在锁内列举文件夹（`fence_items::reconcile_fence`），
/// 不会在"文件夹已改名、库里还是旧名"的间隙里按旧名把空文件夹重建出来。
/// 文件夹改名失败时标题照改、`folder_name` 保持旧值，失败原因放在 `folder_error` 里给前端提示（PRD R1）。
///
/// 删除 / 移动登记（`fence_items::FenceActivity`）也在锁内检查：正在删除时拒绝改名；有文件正在移进 / 移出时
/// 只改标题、文件夹暂不改名——移动 / 还原用的是开始时读到的文件夹路径，中途改名会让文件落进按旧名重建的
/// 文件夹（没有收纳箱管理的孤儿文件夹），删除还会因为"文件夹不存在"而误判为已清空
fn rename_fence_and_folder(
    db: &Database,
    id: i64,
    title: &str,
    rename_folder: impl FnOnce(&str, &str, &[String]) -> Result<String, String>,
) -> Result<RenameFenceResult, String> {
    let title = normalize_title(title)?;
    let folder_error = db
        .with_connection(|conn| -> rusqlite::Result<Result<Option<String>, String>> {
            let Some(fence) = query_fence(conn, id)? else {
                return Ok(Err(format!("收纳箱 #{id} 不存在")));
            };
            if is_deleting(id) {
                return Ok(Err("收纳箱正在删除".to_string()));
            }
            let (folder, folder_error) = if is_moving(id) {
                (
                    fence.folder_name.clone(),
                    Some("收纳箱里有文件正在移动，文件夹暂未改名（之后再改一次标题即可同步）".to_string()),
                )
            } else {
                let taken = all_folder_names(conn)?;
                match rename_folder(&fence.folder_name, &title, &taken) {
                    Ok(folder) => (folder, None),
                    Err(e) => {
                        log::warn!(
                            "[fence] 收纳箱 #{id} 的文件夹「{}」改名失败: {e}",
                            fence.folder_name
                        );
                        (fence.folder_name.clone(), Some(e))
                    }
                }
            };
            conn.execute(
                "UPDATE fences SET title = ?1, folder_name = ?2, updated_at = datetime('now', 'localtime')
                 WHERE id = ?3",
                rusqlite::params![title, folder, id],
            )?;
            Ok(Ok(folder_error))
        })
        .map_err(|e| e.to_string())??;
    Ok(RenameFenceResult {
        fence: load_fence(db, id)?,
        folder_error,
    })
}

/// 只更新 patch 里给出的字段；背景色须为 `#RRGGBB`，透明度夹到 0~1，图标大小夹到上下限。
/// 任一字段校验失败时什么都不写
fn update_fence_row(db: &Database, id: i64, patch: &UpdateFenceRequest) -> Result<Fence, String> {
    let bg_color = patch.bg_color.as_deref().map(normalize_color).transpose()?;
    let bg_alpha = patch.bg_alpha.map(|a| {
        if a.is_finite() {
            a.clamp(0.0, 1.0)
        } else {
            crate::db::DEFAULT_FENCE_BG_ALPHA
        }
    });
    let icon_size = patch
        .icon_size
        .map(|s| s.clamp(MIN_ICON_SIZE, MAX_ICON_SIZE));
    let changed = db
        .with_connection(|conn| {
            conn.execute(
                "UPDATE fences SET
                    collapsed = COALESCE(?1, collapsed),
                    locked    = COALESCE(?2, locked),
                    bg_color  = COALESCE(?3, bg_color),
                    bg_alpha  = COALESCE(?4, bg_alpha),
                    icon_size = COALESCE(?5, icon_size),
                    updated_at = datetime('now', 'localtime')
                 WHERE id = ?6",
                rusqlite::params![
                    patch.collapsed,
                    patch.locked,
                    bg_color,
                    bg_alpha,
                    icon_size,
                    id
                ],
            )
        })
        .map_err(|e| e.to_string())?;
    if changed == 0 {
        return Err(format!("收纳箱 #{id} 不存在"));
    }
    load_fence(db, id)
}

/// 写某个屏幕组合下的位置记录（`fence_layouts`，按 (fence_id, screen_config_id) upsert）
fn save_layout(
    conn: &rusqlite::Connection,
    fence_id: i64,
    screen_id: &str,
    rect: PxRect,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO fence_layouts (fence_id, screen_config_id, x, y, width, height, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, datetime('now', 'localtime'))
         ON CONFLICT(fence_id, screen_config_id) DO UPDATE SET
            x = excluded.x, y = excluded.y, width = excluded.width, height = excluded.height,
            updated_at = excluded.updated_at",
        rusqlite::params![fence_id, screen_id, rect.x, rect.y, rect.width, rect.height],
    )?;
    Ok(())
}

/// 某个屏幕组合下全部收纳箱的位置记录
fn screen_layouts(
    conn: &rusqlite::Connection,
    screen_id: &str,
) -> rusqlite::Result<HashMap<i64, PxRect>> {
    let mut stmt = conn.prepare(
        "SELECT fence_id, x, y, width, height FROM fence_layouts WHERE screen_config_id = ?1",
    )?;
    let rows = stmt.query_map([screen_id], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            PxRect {
                x: r.get(1)?,
                y: r.get(2)?,
                width: r.get(3)?,
                height: r.get(4)?,
            },
        ))
    })?;
    rows.collect()
}

/// 落库用户拖动 / 缩放（或重置位置）后的几何：写当前屏幕组合的记录，同时更新 `fences` 的"最近一次位置"
fn save_fence_geometry(
    db: &Database,
    id: i64,
    screen_id: &str,
    rect: PxRect,
) -> Result<Fence, String> {
    let changed = db
        .with_transaction(|tx| -> rusqlite::Result<usize> {
            let changed = tx.execute(
                "UPDATE fences SET x = ?1, y = ?2, width = ?3, height = ?4,
                 updated_at = datetime('now', 'localtime') WHERE id = ?5",
                rusqlite::params![rect.x, rect.y, rect.width, rect.height, id],
            )?;
            if changed > 0 {
                save_layout(tx, id, screen_id, rect)?;
            }
            Ok(changed)
        })
        .map_err(|e| e.to_string())?;
    if changed == 0 {
        return Err(format!("收纳箱 #{id} 不存在"));
    }
    load_fence(db, id)
}

// ============ 窗口 ============

/// 收纳箱窗口 label
pub fn fence_label(id: i64) -> String {
    format!("{FENCE_LABEL_PREFIX}{id}")
}

fn is_fence_label(label: &str) -> bool {
    label
        .strip_prefix(FENCE_LABEL_PREFIX)
        .is_some_and(|rest| !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit()))
}

pub(crate) fn fence_window(app: &AppHandle, id: i64) -> Option<WebviewWindow> {
    app.get_webview_window(&fence_label(id))
}

fn fence_windows(app: &AppHandle) -> Vec<WebviewWindow> {
    app.webview_windows()
        .into_iter()
        .filter(|(label, _)| is_fence_label(label))
        .map(|(_, window)| window)
        .collect()
}

fn title_bar_px(scale: f64) -> i32 {
    (FENCE_TITLE_BAR_DIP * scale).round() as i32
}

fn monitor_work_rect(monitor: &tauri::Monitor) -> PxRect {
    let area = monitor.work_area();
    // 个别平台拿不到工作区（返回 0 尺寸）时退回整块显示器
    if area.size.width > 0 && area.size.height > 0 {
        PxRect {
            x: area.position.x,
            y: area.position.y,
            width: area.size.width as i32,
            height: area.size.height as i32,
        }
    } else {
        PxRect {
            x: monitor.position().x,
            y: monitor.position().y,
            width: monitor.size().width as i32,
            height: monitor.size().height as i32,
        }
    }
}

/// 全部显示器的工作区（物理像素）与缩放比例，主显示器排第一
fn monitor_areas(app: &AppHandle) -> Vec<MonitorArea> {
    let area_of = |m: &tauri::Monitor| MonitorArea {
        work: monitor_work_rect(m),
        scale: m.scale_factor(),
    };
    let mut areas: Vec<MonitorArea> = app
        .available_monitors()
        .unwrap_or_default()
        .iter()
        .map(area_of)
        .collect();
    if let Some(primary) = app.primary_monitor().ok().flatten() {
        let primary = area_of(&primary);
        areas.retain(|a| a.work != primary.work);
        areas.insert(0, primary);
    }
    areas
}

/// 当前屏幕组合 ID（[`screen_config_id_of`]）；取不到显示器信息时为 `unknown`（与前端一致）
pub(crate) fn current_screen_config_id(app: &AppHandle) -> String {
    #[cfg(debug_assertions)]
    if let Some(id) = dev_screen_id_override() {
        return id;
    }
    match app.available_monitors() {
        Ok(monitors) => {
            let dims: Vec<(u32, u32, f64)> = monitors
                .iter()
                .map(|m| (m.size().width, m.size().height, m.scale_factor()))
                .collect();
            screen_config_id_of(&dims)
        }
        Err(e) => {
            log::warn!("[fence] 读取显示器信息失败: {e}");
            UNKNOWN_SCREEN_CONFIG.to_string()
        }
    }
}

/// 开发专用（**仅 debug 构建**）：设置了环境变量 `MINI_TODO_SCREEN_ID_FILE` 时，屏幕组合 ID 改为该文件的
/// 内容（去首尾空白后非空才生效）。e2e 无法真实插拔显示器，靠改这个文件模拟"屏幕组合变化"，走的是
/// 与真实切换相同的轮询 → 恢复路径。release 构建不编译这段代码
#[cfg(debug_assertions)]
fn dev_screen_id_override() -> Option<String> {
    const DEV_SCREEN_ID_FILE_ENV: &str = "MINI_TODO_SCREEN_ID_FILE";
    let path = std::env::var_os(DEV_SCREEN_ID_FILE_ENV).filter(|v| !v.is_empty())?;
    let text = std::fs::read_to_string(path).ok()?;
    let id = text.trim();
    (!id.is_empty()).then(|| id.to_string())
}

/// 当前屏幕组合下各收纳箱应使用的几何（[`effective_rect`]）
struct Placement {
    screen_id: String,
    layouts: HashMap<i64, PxRect>,
    session: BTreeMap<i64, PxRect>,
}

impl Placement {
    fn current(app: &AppHandle, db: &Database) -> Placement {
        Placement::for_screen(db, current_screen_config_id(app))
    }

    fn for_screen(db: &Database, screen_id: String) -> Placement {
        let layouts = db
            .with_connection(|conn| screen_layouts(conn, &screen_id))
            .unwrap_or_else(|e| {
                log::warn!("[fence] 读取屏幕组合 {screen_id} 的收纳箱位置失败: {e}");
                HashMap::new()
            });
        Placement {
            screen_id,
            layouts,
            session: session_rects().clone(),
        }
    }

    fn rect(&self, fence: &Fence) -> PxRect {
        effective_rect(fence, &self.layouts, &self.session)
    }
}

/// 按库里的状态打开（或返回已有的）收纳箱窗口。
///
/// 必须在 async 命令 / 独立线程里调用：Windows 上同步命令和事件回调里建窗会死锁
/// （tauri `WebviewWindowBuilder` 文档）。托盘入口经 `async_runtime::spawn` 进来。
/// `stored` 是当前屏幕组合下应使用的几何（[`Placement::rect`]）。
async fn open_fence_window(
    app: &AppHandle,
    fence: &Fence,
    stored: PxRect,
) -> Result<WebviewWindow, String> {
    let label = fence_label(fence.id);
    if let Some(window) = app.get_webview_window(&label) {
        return Ok(window);
    }

    let expanded = !fence.collapsed;
    let (view, scale) = fence_view(stored, expanded, &monitor_areas(app));
    let min_height = if expanded {
        MIN_EXPANDED_HEIGHT_DIP
    } else {
        FENCE_TITLE_BAR_DIP
    };
    let url = WebviewUrl::App(format!("index.html#/fence?id={}", fence.id).into());

    // tao flag 一次给齐，之后不再改：always_on_bottom 写进 flags 后任何 SetWindowPos 都会被压回
    // HWND_BOTTOM；minimizable(false) 让 Win+M 不最小化它；skip_taskbar 在 set_visible 之前生效，
    // 建窗不闪任务栏；resizable(false) 不挂原生缩放边框（锁定 / 折叠时就不能缩放，也不参与 Aero Snap），
    // 未锁定时由前端手柄调 set_fence_bounds；maximizable(false) 防止任何途径的最大化冲掉嵌入样式。
    // position / inner_size 是逻辑像素，按目标显示器的缩放换算，建好后再按物理像素校正一次
    let window = WebviewWindowBuilder::new(app, &label, url)
        .title(&fence.title)
        .position(view.x as f64 / scale, view.y as f64 / scale)
        .inner_size(view.width as f64 / scale, view.height as f64 / scale)
        .min_inner_size(MIN_WIDTH_DIP, min_height)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .skip_taskbar(true)
        .visible(true)
        .focused(false)
        .always_on_bottom(true)
        .minimizable(false)
        .maximizable(false)
        .resizable(false)
        .build()
        .map_err(|e| format!("创建收纳箱窗口失败: {e}"))?;

    // tao 按各显示器的缩放换算逻辑坐标，混合 DPI 时可能差几个像素：先位置、后尺寸再定一次
    let _ = window.set_position(PhysicalPosition::new(view.x, view.y));
    let _ = window.set_size(PhysicalSize::new(view.width as u32, view.height as u32));

    #[cfg(target_os = "windows")]
    crate::setup_window_rounded_corners(&window);

    // 闭包只持有 label，不持有窗口本身（窗口持有自己的事件闭包会成环）
    let event_label = label.clone();
    window.on_window_event(move |event| match event {
        // 收纳箱没有关闭按钮；聚焦时 Alt+F4 也不能把它关掉（关掉后直到重启都找不回来）。
        // 删除收纳箱走 destroy()，不经过这里
        tauri::WindowEvent::CloseRequested { api, .. } => api.prevent_close(),
        tauri::WindowEvent::Destroyed => release_embedded_window(&event_label),
        _ => {}
    });

    embed_window_in_desktop(&window);
    if FENCES_HIDDEN.load(Ordering::SeqCst) {
        let _ = window.hide();
        reassert_embedded_window(&window);
    }
    Ok(window)
}

/// 让窗口呈现折叠 / 展开态（按 `stored` 几何计算，见 [`view_rect`]；`stored` 一般是 [`Placement::rect`]）。
///
/// 只改位置、尺寸与最小尺寸，都不触发 tao 的 `apply_diff`，嵌入样式不受影响。
/// 展开：先定位再撑开，最后抬高最小高度；折叠：先放低最小高度，再定位、缩小。
fn apply_fence_view(app: &AppHandle, window: &WebviewWindow, stored: PxRect, expanded: bool) {
    // 标题栏高度按目标位置所在显示器的缩放算（见 title_scale），不取窗口当前的缩放
    let (rect, _) = fence_view(stored, expanded, &monitor_areas(app));
    let position = PhysicalPosition::new(rect.x, rect.y);
    let size = PhysicalSize::new(rect.width.max(1) as u32, rect.height.max(1) as u32);
    if expanded {
        let _ = window.set_position(position);
        let _ = window.set_size(size);
        let _ = window.set_min_size(Some(LogicalSize::new(
            MIN_WIDTH_DIP,
            MIN_EXPANDED_HEIGHT_DIP,
        )));
    } else {
        let _ = window.set_min_size(Some(LogicalSize::new(MIN_WIDTH_DIP, FENCE_TITLE_BAR_DIP)));
        let _ = window.set_position(position);
        let _ = window.set_size(size);
    }
}

/// 显示 / 隐藏全部收纳箱（托盘「显示/隐藏所有收纳箱」）。
///
/// 只走 tao 的 `show()` / `hide()`（tao 不同步 Win32 `ShowWindow` 的可见态，混用会让之后的
/// `apply_diff` 把窗口意外藏起或显示）；两者都会重写 ex style，紧跟着重申嵌入。
fn set_fences_visible(app: &AppHandle, visible: bool) {
    FENCES_HIDDEN.store(!visible, Ordering::SeqCst);
    for window in fence_windows(app) {
        let _ = if visible {
            window.show()
        } else {
            window.hide()
        };
        reassert_embedded_window(&window);
    }
}

/// 托盘「显示/隐藏所有收纳箱」：返回切换后是否可见
pub fn toggle_all_fences(app: &AppHandle) -> bool {
    let visible = FENCES_HIDDEN.load(Ordering::SeqCst);
    set_fences_visible(app, visible);
    visible
}

/// 在光标所在显示器的工作区内新建一个收纳箱并打开窗口（托盘「新增收纳箱」与 `create_fence` 命令）
async fn create_fence_at_cursor(app: &AppHandle) -> Result<Fence, String> {
    let db = app.state::<Database>();
    let existing = load_fences(&db)?;
    let placement = Placement::current(app, &db);
    // 收纳箱全被藏起时新建一个：先把它们都显示回来，否则用户看不到新建的结果
    if FENCES_HIDDEN.load(Ordering::SeqCst) {
        set_fences_visible(app, true);
    }

    let monitor = app
        .cursor_position()
        .ok()
        .and_then(|p| app.monitor_from_point(p.x, p.y).ok().flatten())
        .or_else(|| app.primary_monitor().ok().flatten());
    let (work, scale) = match &monitor {
        Some(m) => (monitor_work_rect(m), m.scale_factor().max(0.1)),
        None => (
            PxRect {
                x: 0,
                y: 0,
                width: 1920,
                height: 1080,
            },
            1.0,
        ),
    };
    let px = |dip: f64| (dip * scale).round() as i32;
    let width = px(CELL_WIDTH_DIP * DEFAULT_COLUMNS + CONTENT_PADDING_DIP * 2.0);
    let height =
        px(FENCE_TITLE_BAR_DIP + CELL_HEIGHT_DIP * DEFAULT_ROWS + CONTENT_PADDING_DIP * 2.0);
    let rects: Vec<PxRect> = existing.iter().map(|f| placement.rect(f)).collect();
    let rect = default_fence_rect(
        work,
        width,
        height,
        &rects,
        px(SCREEN_MARGIN_DIP),
        px(CASCADE_STEP_DIP),
    );
    let titles: Vec<String> = existing.iter().map(|f| f.title.clone()).collect();
    let title = next_default_title(&titles);

    // 先建收纳文件夹再插库：文件夹建不出来就不建收纳箱
    let taken: Vec<String> = existing.iter().map(|f| f.folder_name.clone()).collect();
    let folder_title = title.clone();
    let folder = blocking(move || fence_fs::create_fence_folder(&folder_title, &taken)).await?;
    let fence = match insert_fence(&db, &title, rect, &folder) {
        Ok(fence) => fence,
        Err(e) => {
            rollback_fence_folder(folder).await;
            return Err(e);
        }
    };
    // 新位置是按当前屏幕组合选的：一并记成该组合的位置，切到别的组合再切回来时回到这里
    if let Err(e) =
        db.with_connection(|conn| save_layout(conn, fence.id, &placement.screen_id, rect))
    {
        log::warn!("[fence] 记录收纳箱 #{} 的屏幕组合位置失败: {e}", fence.id);
    }

    if let Err(e) = open_fence_window(app, &fence, rect).await {
        // 窗口没建出来就不留这条记录，否则下次启动会冒出一个用户从没见过的收纳箱
        let _ = delete_fence_row(&db, fence.id);
        rollback_fence_folder(fence.folder_name.clone()).await;
        return Err(e);
    }
    ensure_fence_watcher(app);
    log::info!(
        "[fence] 新建收纳箱 #{} 「{}」（文件夹「{}」） at ({}, {}) {}x{}",
        fence.id,
        fence.title,
        fence.folder_name,
        fence.x,
        fence.y,
        fence.width,
        fence.height
    );
    Ok(fence)
}

/// 新建失败时删掉刚建的收纳文件夹（只删空文件夹，绝不递归删除）
async fn rollback_fence_folder(folder: String) {
    let removed = blocking(move || fence_fs::remove_fence_folder_if_empty(&folder)).await;
    if let Err(e) = removed {
        log::warn!("[fence] 回滚收纳文件夹失败: {e}");
    }
}

/// 托盘「新增收纳箱」：事件回调里不能直接建窗（会死锁），转到异步运行时
// 只有 Windows 的托盘 / setup 会调用（收纳箱入口仅 cfg(windows)）
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub fn spawn_create_fence(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(e) = create_fence_at_cursor(&app).await {
            log::error!("[fence] 新建收纳箱失败: {e}");
        }
    });
}

/// 应用启动时按库恢复全部收纳箱（异步，不阻塞 setup）
// 只有 Windows 的托盘 / setup 会调用（收纳箱入口仅 cfg(windows)）
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub fn restore_fences(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let db = app.state::<Database>();
        let fences = match load_fences(&db) {
            Ok(fences) => fences,
            Err(e) => {
                log::error!("[fence] 读取收纳箱失败: {e}");
                return;
            }
        };
        // 有当前屏幕组合的记录就用它，没有就用最近一次位置；建窗时都夹回可见工作区
        let placement = Placement::current(&app, &db);
        if !fences.is_empty() {
            log::info!(
                "[fence] 当前屏幕组合 {}（{} 个收纳箱有该组合的位置记录）",
                placement.screen_id,
                placement.layouts.len()
            );
            *last_screen_id() = Some(placement.screen_id.clone());
        }
        for fence in &fences {
            // 用户可能在资源管理器里删掉了收纳文件夹：按原名重建（里面没有可丢的东西）
            let folder = fence.folder_name.clone();
            if let Err(e) = blocking(move || fence_fs::ensure_fence_folder(&folder)).await {
                log::warn!("[fence] 收纳箱 #{} 的文件夹不可用: {e}", fence.id);
            }
            if let Err(e) = open_fence_window(&app, fence, placement.rect(fence)).await {
                log::error!("[fence] 恢复收纳箱 #{} 失败: {e}", fence.id);
            }
        }
        if !fences.is_empty() {
            log::info!("[fence] 已恢复 {} 个收纳箱", fences.len());
            ensure_fence_watcher(&app);
        }
        // 启动对账（文件夹为准）后清理图标缓存；没有收纳箱时也清理一次，删掉之前留下的缓存
        spawn_startup_reconcile(&app);
    });
}

/// 托盘「重置收纳箱位置」：把所有收纳箱排回主显示器工作区内的可见位置并落库（用户主动操作）
pub fn reset_all_fence_positions(app: &AppHandle) -> Result<(), String> {
    let db = app.state::<Database>();
    let fences = load_fences(&db)?;
    if fences.is_empty() {
        return Ok(());
    }
    let Some(MonitorArea { work, scale }) = monitor_areas(app).first().copied() else {
        return Err("找不到可用的显示器".to_string());
    };
    let placement = Placement::current(app, &db);
    let margin = (SCREEN_MARGIN_DIP * scale).round() as i32;
    let gap = (LAYOUT_GAP_DIP * scale).round() as i32;
    let sizes: Vec<(i32, i32)> = fences
        .iter()
        .map(|f| {
            let current = placement.rect(f);
            (
                current.width.min(work.width - 2 * margin).max(1),
                current.height.min(work.height - 2 * margin).max(1),
            )
        })
        .collect();
    let positions = reset_layout(work, &sizes, margin, gap);
    let rects: Vec<PxRect> = sizes
        .iter()
        .zip(&positions)
        .map(|(&(width, height), &(x, y))| PxRect {
            x,
            y,
            width,
            height,
        })
        .collect();

    // 用户主动操作：写当前屏幕组合的记录，同时更新最近一次位置
    db.with_transaction(|tx| -> rusqlite::Result<()> {
        for (fence, rect) in fences.iter().zip(&rects) {
            tx.execute(
                "UPDATE fences SET x = ?1, y = ?2, width = ?3, height = ?4,
                 updated_at = datetime('now', 'localtime') WHERE id = ?5",
                rusqlite::params![rect.x, rect.y, rect.width, rect.height, fence.id],
            )?;
            save_layout(tx, fence.id, &placement.screen_id, *rect)?;
        }
        Ok(())
    })
    .map_err(|e: rusqlite::Error| e.to_string())?;
    session_rects().clear();

    for (fence, rect) in fences.iter().zip(&rects) {
        if let Some(window) = fence_window(app, fence.id) {
            apply_fence_view(app, &window, *rect, !fence.collapsed);
        }
    }
    // 被藏起来的也一并显示，重置的目的就是"找回来"
    if FENCES_HIDDEN.load(Ordering::SeqCst) {
        set_fences_visible(app, true);
    }
    log::info!(
        "[fence] 已重置 {} 个收纳箱的位置（屏幕组合 {}）",
        positions.len(),
        placement.screen_id
    );
    Ok(())
}

/// 屏幕组合检测（PRD R5）：200ms 轮询线程约每 2 秒调用一次。
///
/// 屏幕组合变化后把各收纳箱恢复到新组合下的记录（先 `set_position` 再 `set_size`，折叠态按折叠高度）；
/// 没有记录的沿用当前位置并夹回可见区域，只记在本次运行的内存里，不落库。
/// 没有收纳箱窗口时不检测（也不读显示器信息）
pub fn tick_fence_screen_layouts(app: &AppHandle) {
    if fence_windows(app).is_empty() {
        *last_screen_id() = None;
        return;
    }
    let screen_id = current_screen_config_id(app);
    let previous = last_screen_id().replace(screen_id.clone());
    if let Some(previous) = previous {
        if previous != screen_id {
            log::info!("[fence] 屏幕组合变化：{previous} → {screen_id}");
            apply_screen_switch(app, &previous, &screen_id);
        }
    }
}

/// 屏幕组合从 `previous` 切到 `screen_id`：按新组合的记录（没有则沿用当前位置）摆放全部收纳箱窗口
fn apply_screen_switch(app: &AppHandle, previous: &str, screen_id: &str) {
    let db = app.state::<Database>();
    let fences = match load_fences(&db) {
        Ok(fences) => fences,
        Err(e) => {
            log::warn!("[fence] 读取收纳箱失败: {e}");
            return;
        }
    };
    // 切换前的几何：没有记录、窗口只剩标题栏时，用它的展开高度
    let before = Placement::for_screen(&db, previous.to_string());
    let records = db
        .with_connection(|conn| screen_layouts(conn, screen_id))
        .unwrap_or_else(|e| {
            log::warn!("[fence] 读取屏幕组合 {screen_id} 的收纳箱位置失败: {e}");
            HashMap::new()
        });

    let mut session = BTreeMap::new();
    let mut targets = Vec::new();
    let mut restored = 0;
    for fence in &fences {
        let Some(window) = fence_window(app, fence.id) else {
            continue;
        };
        let previous_rect = before.rect(fence);
        let current = match (window.outer_position(), window.outer_size()) {
            (Ok(position), Ok(size)) => PxRect {
                x: position.x,
                y: position.y,
                width: size.width as i32,
                height: size.height as i32,
            },
            _ => previous_rect,
        };
        let scale = window.scale_factor().unwrap_or(1.0);
        let collapsed_view = current.height < title_bar_px(scale) * 2;
        let record = records.get(&fence.id).copied();
        let target = layout_after_switch(record, current, previous_rect.height, collapsed_view);
        if record.is_some() {
            restored += 1;
        } else {
            session.insert(fence.id, target);
        }
        targets.push((window, target, !fence.collapsed));
    }
    *session_rects() = session;
    for (window, target, expanded) in &targets {
        apply_fence_view(app, window, *target, *expanded);
    }
    log::info!(
        "[fence] 已按屏幕组合 {screen_id} 摆放 {} 个收纳箱（{restored} 个恢复到该组合的记录）",
        targets.len()
    );
}

// ============ Tauri 命令 ============

/// 列出全部收纳箱（按 sort_order、id）
#[tauri::command]
pub fn list_fences(db: State<Database>) -> Result<Vec<Fence>, String> {
    load_fences(&db)
}

/// 读取单个收纳箱（FenceView 启动时按路由参数 id 读取自己的数据）
#[tauri::command]
pub fn get_fence(db: State<Database>, id: i64) -> Result<Fence, String> {
    load_fence(&db, id)
}

/// 在光标所在显示器新建收纳箱并打开窗口（与托盘「新增收纳箱」同一实现）。
/// 建窗必须 async：Windows 上同步命令里建窗会死锁
#[tauri::command]
pub async fn create_fence(app: AppHandle) -> Result<Fence, String> {
    create_fence_at_cursor(&app).await
}

/// 改标题（双击标题原地编辑 / 标题栏菜单「重命名」），收纳文件夹跟着改名（非法字符替换、重名加后缀）。
/// 文件夹改名失败时标题照改、保留旧文件夹名，原因在返回值的 `folderError` 里
#[tauri::command]
pub async fn rename_fence(
    app: AppHandle,
    id: i64,
    title: String,
) -> Result<RenameFenceResult, String> {
    let handle = app.clone();
    let result = blocking(move || {
        rename_fence_and_folder(
            &handle.state::<Database>(),
            id,
            &title,
            fence_fs::rename_fence_folder,
        )
    })
    .await?;
    if let Some(window) = fence_window(&app, id) {
        // SetWindowTextW，不触发 apply_diff
        let _ = window.set_title(&result.fence.title);
    }
    Ok(result)
}

/// 更新折叠 / 锁定 / 外观 / 图标大小（只改给出的字段）。
///
/// 只写库：折叠态的窗口尺寸由前端紧接着调 [`set_fence_view`] 呈现，锁定由前端控制交互。
#[tauri::command]
pub fn update_fence(
    db: State<Database>,
    id: i64,
    patch: UpdateFenceRequest,
) -> Result<Fence, String> {
    update_fence_row(&db, id, &patch)
}

/// 让收纳箱窗口呈现折叠 / 展开态（折叠按钮、悬停 / 拖文件经过时临时展开与收回）。
///
/// 不改库：持久化的折叠态由 [`update_fence`] 负责，悬停临时展开不改它。
#[tauri::command]
pub fn set_fence_view(
    app: AppHandle,
    db: State<Database>,
    id: i64,
    expanded: bool,
) -> Result<(), String> {
    let fence = load_fence(&db, id)?;
    let window = fence_window(&app, id).ok_or_else(|| format!("收纳箱 #{id} 的窗口不存在"))?;
    let stored = Placement::current(&app, &db).rect(&fence);
    apply_fence_view(&app, &window, stored, expanded);
    Ok(())
}

/// 缩放中由前端逐帧调用：一次性设置窗口位置与尺寸（物理像素），不落库。
///
/// 收纳箱窗口建成 `resizable(false)`（没有 `WS_THICKFRAME`，锁定 / 折叠时也就不能被系统缩放），
/// 系统的缩放循环（`startResizeDragging` → `WM_NCLBUTTONDOWN(HT*)`）对它不生效，所以缩放由前端
/// 缩放手柄捕获指针、按位移算出新矩形后调这里；松手后再调 [`commit_fence_geometry`] 落库。
#[tauri::command]
pub fn set_fence_bounds(
    app: AppHandle,
    id: i64,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
) -> Result<(), String> {
    let window = fence_window(&app, id).ok_or_else(|| format!("收纳箱 #{id} 的窗口不存在"))?;
    set_window_bounds_physical(&window, x, y, width.max(1), height.max(1))
}

/// 用户拖动 / 缩放结束后由前端调用：把窗口当前的位置（和展开态尺寸）落库——写当前屏幕组合的记录，
/// 同时更新最近一次位置（PRD R5）。
///
/// `expanded_view`：窗口当前是否显示完整高度（展开，或折叠态下悬停临时展开）。是则位置与尺寸
/// 全部落库；否（只剩标题栏）则只落位置和宽度，展开高度保持不变。
/// 显示器变化导致的系统挪窗不会走到这里（前端只在自己发起拖动 / 缩放之后调用）。
#[tauri::command]
pub fn commit_fence_geometry(
    app: AppHandle,
    db: State<Database>,
    id: i64,
    expanded_view: bool,
) -> Result<Fence, String> {
    let window = fence_window(&app, id).ok_or_else(|| format!("收纳箱 #{id} 的窗口不存在"))?;
    let position = window.outer_position().map_err(|e| e.to_string())?;
    let size = window.outer_size().map_err(|e| e.to_string())?;
    let fence = load_fence(&db, id)?;
    let placement = Placement::current(&app, &db);
    let window_rect = PxRect {
        x: position.x,
        y: position.y,
        width: size.width as i32,
        height: size.height as i32,
    };
    let rect = committed_rect(window_rect, expanded_view, placement.rect(&fence));
    let saved = save_fence_geometry(&db, id, &placement.screen_id, rect)?;
    // 当前组合已经有记录了，临时几何作废
    session_rects().remove(&id);
    Ok(saved)
}

/// `delete_fence` 的结果
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteFenceResult {
    /// 已删除：所有项目都已移回、收纳文件夹已删除、记录已删、窗口已关闭。
    /// 为 false 时删除中止，收纳箱整个保留（记录、窗口、文件夹），继续显示剩余项（PRD R1）
    pub deleted: bool,
    /// 已移回的项目数
    pub restored: usize,
    /// 没能移走、仍留在收纳文件夹里的文件名（含隐藏文件）
    pub remaining: Vec<String>,
    /// 移回失败 / 被取消的项（含原因）
    pub failures: Vec<ItemOpResult>,
}

/// 批量还原之后怎么处理收纳箱（纯函数，有单元测试）
#[derive(Debug, Clone, PartialEq, Eq)]
struct DeletePlan {
    /// 收纳文件夹已清空并删除：删记录、关窗口；否则整个收纳箱保留
    delete_fence: bool,
    /// 已移回的项在收纳文件夹里的文件名（保留收纳箱时删掉它们的 `fence_items` 行）
    restored: Vec<String>,
    /// 移回失败 / 被取消的项
    failures: Vec<ItemOpResult>,
}

/// 只有收纳文件夹已删除（里面的项目全部移走）才真正删除收纳箱；只要还有项目留在文件夹里
/// （失败、取消、或 `remaining` 里的隐藏文件），就保留整个收纳箱，不产生没人管的孤儿文件夹（PRD R1）
fn plan_delete(restored: &fence_fs::RestoreAllResult) -> DeletePlan {
    let moved_names = restored
        .items
        .iter()
        .filter(|r| r.status == ItemOpStatus::Moved)
        .filter_map(|r| {
            std::path::Path::new(&r.source)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
        })
        .collect();
    DeletePlan {
        delete_fence: restored.folder_removed,
        restored: moved_names,
        failures: restored
            .items
            .iter()
            .filter(|r| matches!(r.status, ItemOpStatus::Failed | ItemOpStatus::Cancelled))
            .cloned()
            .collect(),
    }
}

/// 删除收纳箱：先把收纳文件夹里的项目全部移回原位置（原目录不存在则回桌面，重名自动改名），
/// 全部移走、空文件夹删除后才删记录（`fence_items` / `fence_layouts` 级联删除）并关闭窗口。
///
/// - 还原整体无法进行（`Err`，例如桌面路径未知）：什么都不删，直接返回错误
/// - 有项目没能移回（文件被占用、用户取消管理员权限对话框）：删除中止，收纳箱整个保留（记录、窗口、文件夹）；
///   已移回的项删掉对应的 `fence_items` 行并重新对账，前端提示原因，用户处理后可以再删一次
#[tauri::command]
pub async fn delete_fence(app: AppHandle, id: i64) -> Result<DeleteFenceResult, String> {
    // 先登记再读文件夹名（见 fence_items::FenceActivity）：还原期间（可能在等用户处理管理员权限对话框）
    // 对账跳过它、不能改名、不能再移入移出；有文件正在移动时直接拒绝删除
    let deleting = begin_deleting(id)?;
    let db = app.state::<Database>();
    let fence = load_fence(&db, id)?;
    let originals = db
        .with_connection(|conn| fence_originals(conn, id))
        .map_err(|e| e.to_string())?;
    // 系统对话框以收纳箱窗口为父窗口；移动在阻塞线程池里执行，绝不在主线程
    let owner = fence_window(&app, id).and_then(|w| owner_hwnd(&w));
    let folder = fence.folder_name.clone();
    let restored = blocking(move || fence_fs::restore_all(&folder, &originals, owner)).await?;
    let plan = plan_delete(&restored);

    if plan.delete_fence {
        delete_fence_row(&db, id)?;
        drop(deleting);
        session_rects().remove(&id);
        if let Some(window) = fence_window(&app, id) {
            // destroy 不经过 CloseRequested（那里拦住了 Alt+F4）；Destroyed 事件里注销嵌入登记
            window.destroy().map_err(|e| e.to_string())?;
        }
        log::info!(
            "[fence] 已删除收纳箱 #{} 「{}」，移回 {} 项",
            fence.id,
            fence.title,
            plan.restored.len()
        );
    } else {
        let removed = db.with_transaction(|tx| -> rusqlite::Result<()> {
            for name in &plan.restored {
                delete_item(tx, id, name)?;
            }
            Ok(())
        });
        // 先注销"删除中"，对账才不会跳过它
        drop(deleting);
        if let Err(e) = removed {
            log::warn!("[fence] 删除已移回项目的记录失败（下次对账会清理）: {e}");
        }
        reconcile_and_notify(&app, &db, id);
        log::warn!(
            "[fence] 删除收纳箱 #{} 「{}」中止：移回 {} 项，{} 项未能移回，收纳箱保留",
            fence.id,
            fence.title,
            plan.restored.len(),
            restored.remaining.len()
        );
    }
    Ok(DeleteFenceResult {
        deleted: plan.delete_fence,
        restored: plan.restored.len(),
        remaining: restored.remaining,
        failures: plan.failures,
    })
}

/// 显示 / 隐藏全部收纳箱（与托盘同一实现），返回切换后是否可见
#[tauri::command]
pub fn toggle_fences_visibility(app: AppHandle) -> Result<bool, String> {
    Ok(toggle_all_fences(&app))
}

/// 把全部收纳箱排回主显示器工作区（与托盘「重置收纳箱位置」同一实现）
#[tauri::command]
pub fn reset_fence_positions(app: AppHandle) -> Result<(), String> {
    reset_all_fence_positions(&app)
}

#[cfg(test)]
mod tests {
    use super::*;

    const WORK: PxRect = PxRect {
        x: 0,
        y: 0,
        width: 1920,
        height: 1040,
    };

    fn rect(x: i32, y: i32, width: i32, height: i32) -> PxRect {
        PxRect {
            x,
            y,
            width,
            height,
        }
    }

    #[test]
    fn default_title_takes_the_first_free_number() {
        assert_eq!(next_default_title(&[]), "收纳箱");
        assert_eq!(next_default_title(&["工作".into()]), "收纳箱");
        assert_eq!(next_default_title(&["收纳箱".into()]), "收纳箱 2");
        assert_eq!(
            next_default_title(&["收纳箱".into(), "收纳箱 2".into(), "收纳箱 4".into()]),
            "收纳箱 3"
        );
        assert_eq!(
            next_default_title(&["收纳箱 2".into()]),
            "收纳箱",
            "基础名空着就用基础名"
        );
    }

    #[test]
    fn first_fence_goes_to_the_top_right_of_the_work_area() {
        let r = default_fence_rect(WORK, 368, 334, &[], 24, 32);
        assert_eq!(r, rect(1920 - 24 - 368, 24, 368, 334));
    }

    #[test]
    fn new_fences_avoid_overlapping_existing_ones() {
        let first = default_fence_rect(WORK, 368, 334, &[], 24, 32);
        let second = default_fence_rect(WORK, 368, 334, &[first], 24, 32);
        assert_eq!(second.intersection_area(first), 0, "{second:?}");
        assert_eq!(second.y, first.y, "同一行往左放");
        assert!(second.right() <= first.x);
        let third = default_fence_rect(WORK, 368, 334, &[first, second], 24, 32);
        assert_eq!(third.intersection_area(first), 0);
        assert_eq!(third.intersection_area(second), 0);
    }

    #[test]
    fn a_full_work_area_falls_back_to_cascading_without_identical_corners() {
        // 一个占满整个工作区的收纳箱：不可能不重叠，退化为斜向叠放
        let big = rect(0, 0, 1920, 1040);
        let mut existing = vec![big];
        for _ in 0..5 {
            let next = default_fence_rect(WORK, 368, 334, &existing, 24, 32);
            assert!(
                !existing[1..]
                    .iter()
                    .any(|r| (r.x - next.x).abs() <= 16 && (r.y - next.y).abs() <= 16),
                "{next:?} 与已有收纳箱完全重叠"
            );
            assert!(next.x >= WORK.x && next.right() <= WORK.right());
            assert!(next.y >= WORK.y && next.bottom() <= WORK.bottom());
            existing.push(next);
        }
    }

    #[test]
    fn default_rect_respects_offset_monitors_and_tiny_work_areas() {
        // 副屏在主屏左侧（负坐标）
        let left = rect(-1280, 100, 1280, 984);
        let r = default_fence_rect(left, 368, 334, &[], 24, 32);
        assert_eq!(r, rect(-24 - 368, 124, 368, 334));
        // 工作区比收纳箱还小：缩到工作区内
        let tiny = rect(0, 0, 300, 200);
        let r = default_fence_rect(tiny, 368, 334, &[], 24, 32);
        assert!(r.width <= 300 - 48 && r.height <= 200 - 48, "{r:?}");
        assert!(r.x >= 0 && r.y >= 0);
    }

    #[test]
    fn clamp_pulls_off_screen_rects_back_and_keeps_their_size() {
        assert_eq!(
            clamp_into(rect(1800, 1000, 368, 334), WORK),
            rect(1920 - 368, 1040 - 334, 368, 334)
        );
        assert_eq!(
            clamp_into(rect(-500, -40, 368, 334), WORK),
            rect(0, 0, 368, 334)
        );
        assert_eq!(
            clamp_into(rect(100, 100, 368, 334), WORK),
            rect(100, 100, 368, 334),
            "已在工作区内不动"
        );
        assert_eq!(
            clamp_into(rect(50, 50, 4000, 2000), WORK),
            rect(0, 0, 4000, 2000),
            "比工作区还大：贴左上角，尺寸不变"
        );
    }

    #[test]
    fn work_area_is_picked_by_overlap_then_by_distance() {
        let primary = rect(0, 0, 1920, 1040);
        let secondary = rect(1920, 0, 2560, 1400);
        let works = [primary, secondary];
        assert_eq!(
            pick_work_area(rect(2000, 100, 368, 32), &works),
            Some(secondary)
        );
        assert_eq!(
            pick_work_area(rect(1800, 100, 368, 32), &works),
            Some(secondary),
            "跨两块屏时取交叠面积大的"
        );
        // 显示器被拔掉后的坐标：离哪块近就回哪块
        assert_eq!(
            pick_work_area(rect(-3000, 200, 368, 32), &works),
            Some(primary)
        );
        assert_eq!(
            pick_work_area(rect(6000, 200, 368, 32), &works),
            Some(secondary)
        );
        assert_eq!(pick_work_area(rect(0, 0, 10, 10), &[]), None);
    }

    #[test]
    fn collapsed_view_is_just_the_title_bar() {
        let stored = rect(300, 200, 368, 334);
        assert_eq!(
            view_rect(stored, false, 32, &[WORK]),
            rect(300, 200, 368, 32)
        );
        assert_eq!(view_rect(stored, true, 32, &[WORK]), stored);
    }

    #[test]
    fn expanding_near_the_bottom_grows_upward_and_keeps_the_title_area_covered() {
        // 标题栏贴近底部：展开时整体上移到刚好放得下
        let stored = rect(300, 900, 368, 334);
        let expanded = view_rect(stored, true, 32, &[WORK]);
        assert_eq!(expanded, rect(300, 1040 - 334, 368, 334));
        // 原标题栏区域仍在展开后的窗口里（悬停展开时鼠标不会因此离开窗口）
        assert!(expanded.y <= stored.y && expanded.bottom() >= stored.y + 32);
        // 折叠回去仍是原位置
        assert_eq!(
            view_rect(stored, false, 32, &[WORK]),
            rect(300, 900, 368, 32)
        );
    }

    #[test]
    fn views_of_an_off_screen_fence_are_clamped_into_the_nearest_work_area() {
        let stored = rect(5000, 3000, 368, 334);
        assert_eq!(
            view_rect(stored, false, 32, &[WORK]),
            rect(1920 - 368, 1040 - 32, 368, 32)
        );
        assert_eq!(
            view_rect(stored, true, 32, &[WORK]),
            rect(1920 - 368, 1040 - 334, 368, 334)
        );
        assert_eq!(
            view_rect(stored, true, 32, &[]),
            stored,
            "没有显示器信息时不动"
        );
    }

    #[test]
    fn title_bar_height_follows_the_target_monitor_scale() {
        // 混合 DPI：左侧副屏 125%，主屏 150%
        let areas = [
            MonitorArea {
                work: rect(0, 0, 3840, 2088),
                scale: 1.5,
            },
            MonitorArea {
                work: rect(-2560, 0, 2560, 1540),
                scale: 1.25,
            },
        ];
        let on_secondary = rect(-2000, 100, 552, 504);
        assert_eq!(title_scale(on_secondary, &areas), 1.25);
        assert_eq!(
            fence_view(on_secondary, false, &areas),
            (rect(-2000, 100, 552, 40), 1.25)
        );
        let on_primary = rect(2676, 36, 552, 504);
        assert_eq!(
            fence_view(on_primary, false, &areas),
            (rect(2676, 36, 552, 48), 1.5)
        );
        // 跑到屏幕外：按就近的显示器（夹回后的位置）算
        assert_eq!(title_scale(rect(-9000, 100, 552, 504), &areas), 1.25);
        assert_eq!(title_scale(on_primary, &[]), 1.0);
    }

    #[test]
    fn reset_layout_rows_from_the_top_right_and_stays_on_screen() {
        let sizes = [(368, 334), (368, 334), (368, 334)];
        let positions = reset_layout(WORK, &sizes, 24, 16);
        assert_eq!(positions[0], (1920 - 24 - 368, 24));
        assert_eq!(positions[1], (1920 - 24 - 368 - 16 - 368, 24));
        assert_eq!(positions[2].1, 24, "一行放得下就不换行");

        // 很多个：换行、放不下时回到顶部错开叠放，始终在工作区内
        let many = vec![(600, 500); 12];
        for (i, &(x, y)) in reset_layout(WORK, &many, 24, 16).iter().enumerate() {
            let r = rect(x, y, 600, 500);
            assert!(
                r.x >= WORK.x
                    && r.right() <= WORK.right()
                    && r.y >= WORK.y
                    && r.bottom() <= WORK.bottom(),
                "#{i} {r:?} 跑出工作区"
            );
        }
    }

    #[test]
    fn titles_are_trimmed_and_validated() {
        assert_eq!(normalize_title("  工作  ").unwrap(), "工作");
        assert_eq!(normalize_title("a\nb\tc").unwrap(), "a b c");
        assert!(normalize_title("   ").is_err());
        assert!(normalize_title("\n").is_err());
        assert!(normalize_title(&"字".repeat(MAX_TITLE_CHARS)).is_ok());
        assert!(normalize_title(&"字".repeat(MAX_TITLE_CHARS + 1)).is_err());
    }

    #[test]
    fn colors_must_be_six_digit_hex() {
        assert_eq!(normalize_color("#1e293b").unwrap(), "#1E293B");
        assert_eq!(normalize_color(" #000000 ").unwrap(), "#000000");
        for bad in ["000000", "#000", "#00000G", "", "#0000000", "red"] {
            assert!(normalize_color(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn fence_labels_round_trip() {
        assert_eq!(fence_label(12), "fence-12");
        assert!(is_fence_label("fence-12"));
        assert!(!is_fence_label("fence-"));
        assert!(!is_fence_label("fence-x"));
        assert!(!is_fence_label("main"));
        assert!(!is_fence_label("settings-123"));
    }

    // ---- 数据库 ----

    #[test]
    fn insert_stores_the_folder_and_increasing_sort_order() {
        let db = Database::new_in_memory().unwrap();
        let a = insert_fence(&db, "收纳箱", rect(10, 20, 368, 334), "收纳箱").unwrap();
        let b = insert_fence(&db, "收纳箱 2", rect(40, 50, 368, 334), "收纳箱 2").unwrap();
        assert_eq!(a.folder_name, "收纳箱");
        assert_eq!(b.folder_name, "收纳箱 2");
        assert!(
            insert_fence(&db, "另一个", rect(0, 0, 368, 334), "收纳箱").is_err(),
            "folder_name 唯一：插库失败由调用方回滚文件夹"
        );
        assert_eq!((a.sort_order, b.sort_order), (0, 1));
        assert_eq!((a.x, a.y, a.width, a.height), (10, 20, 368, 334));
        assert!(!a.collapsed && !a.locked);
        assert_eq!(a.bg_color, "#000000");
        assert_eq!(a.bg_alpha, crate::db::DEFAULT_FENCE_BG_ALPHA);
        assert_eq!(a.icon_size, 48);
        assert!(!a.created_at.is_empty() && !a.updated_at.is_empty());

        let all = load_fences(&db).unwrap();
        assert_eq!(
            all.iter().map(|f| f.id).collect::<Vec<_>>(),
            vec![a.id, b.id]
        );
        delete_fence_row(&db, a.id).unwrap();
        assert!(load_fence(&db, a.id).is_err());
        assert_eq!(load_fences(&db).unwrap().len(), 1);
    }

    #[test]
    fn update_only_touches_given_fields_and_validates_them() {
        let db = Database::new_in_memory().unwrap();
        let f = insert_fence(&db, "收纳箱", rect(10, 20, 368, 334), "收纳箱").unwrap();

        let locked = update_fence_row(
            &db,
            f.id,
            &UpdateFenceRequest {
                locked: Some(true),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(locked.locked && !locked.collapsed, "只改 locked");
        assert_eq!(locked.bg_color, "#000000");

        let styled = update_fence_row(
            &db,
            f.id,
            &UpdateFenceRequest {
                collapsed: Some(true),
                bg_color: Some("#1e293b".into()),
                bg_alpha: Some(1.7),
                icon_size: Some(1000),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(styled.collapsed && styled.locked, "locked 保持不变");
        assert_eq!(styled.bg_color, "#1E293B");
        assert_eq!(styled.bg_alpha, 1.0, "透明度夹到 0~1");
        assert_eq!(styled.icon_size, MAX_ICON_SIZE);

        let bad = update_fence_row(
            &db,
            f.id,
            &UpdateFenceRequest {
                bg_color: Some("red".into()),
                collapsed: Some(false),
                ..Default::default()
            },
        );
        assert!(bad.is_err());
        assert!(
            load_fence(&db, f.id).unwrap().collapsed,
            "校验失败不写任何字段"
        );
        assert!(update_fence_row(&db, 999, &UpdateFenceRequest::default()).is_err());
    }

    #[test]
    fn rename_normalizes_the_title_and_moves_the_folder() {
        let db = Database::new_in_memory().unwrap();
        let f = insert_fence(&db, "收纳箱", rect(10, 20, 368, 334), "收纳箱").unwrap();
        insert_fence(&db, "工具", rect(0, 0, 368, 334), "工具").unwrap();
        let renamed =
            rename_fence_and_folder(&db, f.id, "  工作\n软件 ", |old, title, taken| {
                assert_eq!((old, title), ("收纳箱", "工作 软件"));
                assert!(
                    taken.contains(&"工具".to_string()),
                    "去重要避开其他收纳箱的文件夹"
                );
                Ok("工作 软件".to_string())
            })
            .unwrap();
        assert_eq!(renamed.fence.title, "工作 软件");
        assert_eq!(renamed.fence.folder_name, "工作 软件");
        assert_eq!(renamed.folder_error, None);

        let unreachable = |_: &str, _: &str, _: &[String]| -> Result<String, String> {
            panic!("标题不合法时不应碰文件夹")
        };
        assert!(rename_fence_and_folder(&db, f.id, "   ", unreachable).is_err());
        assert_eq!(load_fence(&db, f.id).unwrap().title, "工作 软件");
        assert!(rename_fence_and_folder(&db, 999, "x", |_, _, _| Ok("x".into())).is_err());
    }

    #[test]
    fn rename_keeps_the_old_folder_when_the_folder_cannot_be_renamed() {
        let db = Database::new_in_memory().unwrap();
        let f = insert_fence(&db, "收纳箱", rect(10, 20, 368, 334), "收纳箱").unwrap();
        let renamed = rename_fence_and_folder(&db, f.id, "常用", |_, _, _| {
            Err("正被其他程序占用".to_string())
        })
        .unwrap();
        assert_eq!(renamed.fence.title, "常用", "标题照改");
        assert_eq!(renamed.fence.folder_name, "收纳箱", "保留旧文件夹名");
        assert_eq!(renamed.folder_error.as_deref(), Some("正被其他程序占用"));
    }

    #[test]
    fn rename_keeps_the_folder_while_files_are_moving_and_is_refused_while_deleting() {
        use super::super::fence_items::begin_moving;
        // 删除 / 移动登记是全局的：用一个其他测试不会用到的 id
        let id = 987_660;
        let db = Database::new_in_memory().unwrap();
        db.with_connection(|conn| {
            conn.execute(
                "INSERT INTO fences (id, title, folder_name, x, y, width, height, created_at, updated_at)
                 VALUES (?1, '收纳箱', '收纳箱', 0, 0, 368, 334, '', '')",
                [id],
            )
        })
        .unwrap();
        let untouched = |_: &str, _: &str, _: &[String]| -> Result<String, String> {
            panic!("有文件正在移动 / 收纳箱正在删除时不应改文件夹名")
        };

        let moving = begin_moving(&[id]).unwrap();
        let renamed = rename_fence_and_folder(&db, id, "常用", untouched).unwrap();
        assert_eq!(renamed.fence.title, "常用", "标题照改");
        assert_eq!(renamed.fence.folder_name, "收纳箱", "文件夹暂不改名");
        assert!(renamed.folder_error.is_some());
        drop(moving);

        let deleting = begin_deleting(id).unwrap();
        assert!(rename_fence_and_folder(&db, id, "工具", untouched).is_err());
        assert_eq!(
            load_fence(&db, id).unwrap().title,
            "常用",
            "删除中连标题也不改"
        );
        drop(deleting);
    }

    #[test]
    fn geometry_from_a_collapsed_view_keeps_the_expanded_height() {
        let current = rect(10, 20, 368, 334);
        assert_eq!(
            committed_rect(rect(500, 600, 400, 48), false, current),
            rect(500, 600, 400, 334),
            "只剩标题栏：保留展开高度"
        );
        assert_eq!(
            committed_rect(rect(480, 560, 420, 500), true, current),
            rect(480, 560, 420, 500)
        );
    }

    const SCREEN_A: &str = "1_3840x2160@150";
    const SCREEN_B: &str = "2_3840x2160@150_2560x1600@125";

    fn layouts_of(db: &Database, screen: &str) -> HashMap<i64, PxRect> {
        db.with_connection(|conn| screen_layouts(conn, screen))
            .unwrap()
    }

    #[test]
    fn saving_geometry_writes_the_current_screen_layout_and_the_latest_position() {
        let db = Database::new_in_memory().unwrap();
        let f = insert_fence(&db, "收纳箱", rect(10, 20, 368, 334), "收纳箱").unwrap();
        let saved = save_fence_geometry(&db, f.id, SCREEN_A, rect(500, 600, 400, 334)).unwrap();
        assert_eq!(
            (saved.x, saved.y, saved.width, saved.height),
            (500, 600, 400, 334),
            "fences 存最近一次位置"
        );
        save_fence_geometry(&db, f.id, SCREEN_B, rect(-2000, 100, 300, 280)).unwrap();
        save_fence_geometry(&db, f.id, SCREEN_A, rect(480, 560, 420, 500)).unwrap();
        assert_eq!(layouts_of(&db, SCREEN_A)[&f.id], rect(480, 560, 420, 500));
        assert_eq!(layouts_of(&db, SCREEN_B)[&f.id], rect(-2000, 100, 300, 280));
        assert_eq!(
            stored_rect(&load_fence(&db, f.id).unwrap()),
            rect(480, 560, 420, 500)
        );
        assert!(layouts_of(&db, "unknown").is_empty());
        assert!(save_fence_geometry(&db, 999, SCREEN_A, rect(0, 0, 1, 1)).is_err());
        assert!(
            !layouts_of(&db, SCREEN_A).contains_key(&999),
            "收纳箱不存在时不写记录"
        );
    }

    #[test]
    fn deleting_a_fence_removes_its_layouts() {
        let db = Database::new_in_memory().unwrap();
        let a = insert_fence(&db, "a", rect(0, 0, 368, 334), "a").unwrap();
        let b = insert_fence(&db, "b", rect(0, 0, 368, 334), "b").unwrap();
        save_fence_geometry(&db, a.id, SCREEN_A, rect(1, 2, 368, 334)).unwrap();
        save_fence_geometry(&db, b.id, SCREEN_A, rect(3, 4, 368, 334)).unwrap();
        delete_fence_row(&db, a.id).unwrap();
        let left = layouts_of(&db, SCREEN_A);
        assert_eq!(left.keys().copied().collect::<Vec<_>>(), vec![b.id]);
    }

    #[test]
    fn effective_geometry_prefers_screen_record_then_session_then_latest() {
        let db = Database::new_in_memory().unwrap();
        let f = insert_fence(&db, "收纳箱", rect(10, 20, 368, 334), "收纳箱").unwrap();
        let mut layouts = HashMap::new();
        let mut session = BTreeMap::new();
        assert_eq!(
            effective_rect(&f, &layouts, &session),
            rect(10, 20, 368, 334)
        );
        session.insert(f.id, rect(700, 20, 368, 334));
        assert_eq!(
            effective_rect(&f, &layouts, &session),
            rect(700, 20, 368, 334)
        );
        layouts.insert(f.id, rect(-2000, 100, 300, 280));
        assert_eq!(
            effective_rect(&f, &layouts, &session),
            rect(-2000, 100, 300, 280)
        );
    }

    #[test]
    fn switching_screens_uses_the_record_or_keeps_the_current_window_position() {
        let window = rect(1200, 900, 552, 48);
        // 新组合有记录：用记录
        assert_eq!(
            layout_after_switch(Some(rect(-2000, 100, 300, 280)), window, 504, true),
            rect(-2000, 100, 300, 280)
        );
        // 没有记录、只剩标题栏：位置沿用窗口当前位置，展开高度沿用切换前的
        assert_eq!(
            layout_after_switch(None, window, 504, true),
            rect(1200, 900, 552, 504)
        );
        // 没有记录、展开着：整个窗口矩形
        assert_eq!(
            layout_after_switch(None, rect(1200, 600, 552, 504), 400, false),
            rect(1200, 600, 552, 504)
        );
        // 沿用的位置跑到了屏幕外：摆放时按工作区夹回（不落库）
        let target = layout_after_switch(None, rect(5000, 3000, 552, 48), 504, true);
        assert_eq!(
            view_rect(target, false, 48, &[WORK]),
            rect(1920 - 552, 1040 - 48, 552, 48)
        );
    }

    fn op(source: &str, status: ItemOpStatus, error: Option<&str>) -> ItemOpResult {
        ItemOpResult {
            source: source.to_string(),
            status,
            file_name: None,
            dest_path: None,
            error: error.map(str::to_string),
        }
    }

    #[test]
    fn delete_goes_ahead_only_when_the_folder_was_emptied_and_removed() {
        let all_back = fence_fs::RestoreAllResult {
            items: vec![
                op(r"C:\root\箱\a.lnk", ItemOpStatus::Moved, None),
                op(r"C:\root\箱\desktop.ini", ItemOpStatus::Skipped, None),
            ],
            folder_removed: true,
            remaining: vec![],
        };
        let plan = plan_delete(&all_back);
        assert!(plan.delete_fence);
        assert_eq!(plan.restored, vec!["a.lnk".to_string()]);
        assert!(plan.failures.is_empty());

        // 空收纳箱（文件夹本来就不存在 / 是空的）也直接删
        let empty = fence_fs::RestoreAllResult {
            items: vec![],
            folder_removed: true,
            remaining: vec![],
        };
        assert!(plan_delete(&empty).delete_fence);
    }

    #[test]
    fn delete_keeps_the_whole_fence_when_anything_is_left_behind() {
        // 一项移回、一项被占用、一项取消了管理员权限对话框
        let partial = fence_fs::RestoreAllResult {
            items: vec![
                op(r"C:\root\箱\a.lnk", ItemOpStatus::Moved, None),
                op(
                    r"C:\root\箱\b.txt",
                    ItemOpStatus::Failed,
                    Some("正被其他程序占用"),
                ),
                op(
                    r"C:\root\箱\c.lnk",
                    ItemOpStatus::Cancelled,
                    Some("用户取消了操作"),
                ),
            ],
            folder_removed: false,
            remaining: vec!["b.txt".into(), "c.lnk".into()],
        };
        let plan = plan_delete(&partial);
        assert!(!plan.delete_fence, "有项目留下：收纳箱整个保留");
        assert_eq!(plan.restored, vec!["a.lnk".to_string()], "只删已移回项的行");
        assert_eq!(plan.failures.len(), 2);
        assert_eq!(plan.failures[0].error.as_deref(), Some("正被其他程序占用"));

        // 所有逐项结果都成功，但文件夹里还有东西（例如列举不到的隐藏文件没能删）：同样保留
        let leftover = fence_fs::RestoreAllResult {
            items: vec![op(r"C:\root\箱\a.lnk", ItemOpStatus::Moved, None)],
            folder_removed: false,
            remaining: vec!["~$lock.tmp".into()],
        };
        let plan = plan_delete(&leftover);
        assert!(!plan.delete_fence);
        assert!(plan.failures.is_empty());
    }

    /// 与前端 `appStore.generateScreenConfigId()` 逐字一致
    #[test]
    fn screen_config_id_matches_the_frontend_format() {
        assert_eq!(screen_config_id_of(&[]), "unknown");
        assert_eq!(screen_config_id_of(&[(1920, 1080, 1.0)]), "1_1920x1080@100");
        // 双屏：按面积降序，与枚举顺序无关
        assert_eq!(
            screen_config_id_of(&[(2560, 1600, 1.25), (3840, 2160, 1.5)]),
            "2_3840x2160@150_2560x1600@125"
        );
        assert_eq!(
            screen_config_id_of(&[(3840, 2160, 1.5), (2560, 1600, 1.25)]),
            "2_3840x2160@150_2560x1600@125"
        );
        // 同面积：稳定排序，保持枚举顺序（竖屏与横屏）
        assert_eq!(
            screen_config_id_of(&[(1920, 1080, 1.0), (1080, 1920, 1.5)]),
            "2_1920x1080@100_1080x1920@150"
        );
        assert_eq!(
            screen_config_id_of(&[(1080, 1920, 1.5), (1920, 1080, 1.0)]),
            "2_1080x1920@150_1920x1080@100"
        );
        // 常见缩放：1.1 * 100 = 110.00000000000001，JS Math.round 与 Rust round 都得 110
        assert_eq!(
            screen_config_id_of(&[(2560, 1440, 1.1), (1920, 1080, 1.75), (1366, 768, 1.0)]),
            "3_2560x1440@110_1920x1080@175_1366x768@100"
        );
        assert_eq!(
            screen_config_id_of(&[(3000, 2000, 2.25)]),
            "1_3000x2000@225"
        );
    }
}
