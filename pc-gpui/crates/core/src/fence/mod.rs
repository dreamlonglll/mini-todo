//! 桌面收纳箱元数据与几何（原 `commands/fence.rs` 中与窗口无关的部分）。
//!
//! 每个收纳箱对应一个始终嵌入桌面层的无边框窗口（app 层负责建窗、嵌入、显示 / 隐藏、摆放），以及收纳根目录
//! 下的一个真实文件夹（`services::fence_fs`）。这里负责：
//!
//! - 纯几何（物理像素）：默认位置 [`default_fence_rect`]、折叠 / 展开形态 [`fence_view`]、
//!   重置排布 [`reset_layout`]、屏幕组合切换 [`layout_after_switch`]、落库几何 [`committed_rect`]
//! - 数据库：`fences` / `fence_layouts` 读写；按屏幕组合记忆位置（[`Placement`]）
//! - 组合操作（阻塞，放后台线程）：[`create`]、[`rename`]、[`update`]、[`commit_geometry`]、
//!   [`reset_positions`]、[`screen_switch`]、[`delete`]、[`prepare_restore`]
//!
//! 几何信息存物理像素：`x` / `y` 为虚拟屏幕坐标（折叠态即标题栏位置），`width` / `height` 为展开态
//! 尺寸。只有用户拖动 / 缩放结束（[`commit_geometry`]）与托盘重置（[`reset_positions`]）才落库；
//! 启动夹回、显示器变化导致的系统挪窗、折叠 / 展开、屏幕组合切换都不落库。
//!
//! 按屏幕组合记忆位置（PRD R5）：几何按屏幕组合（`settings::screen_config_id_of`，与主窗口
//! `screen_configs.config_id` 同一格式）分别存进 `fence_layouts`，`fences.x/y/width/height` 是
//! 「最近一次位置」。当前组合有记录就用记录，没有就用最近一次位置（[`Placement`]）。app 约每 2 秒调用
//! [`observe_screen_config`] 检测组合变化，变化后用 [`screen_switch`] 取各收纳箱的新几何；没有记录的
//! 沿用当前位置，只记在本次运行的内存里（不落库）。
//!
//! 收纳箱**内容**（列举对账、移入 / 移回 / 跨箱、打开、图标、排序、目录监听）在 [`items`]。
//! 契约见 `.trellis/spec/backend/desktop-fences.md`。

pub mod items;

use crate::db::{fence_from_row, Database, Fence, UpdateFenceRequest, FENCE_COLUMNS};
use crate::services::fence_fs::{self, ItemOpResult, ItemOpStatus};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap};
use std::sync::{Mutex, MutexGuard};

use self::items::{begin_deleting, delete_item, fence_originals, is_deleting, is_moving};

/// 收纳箱窗口 label 前缀（`fence-{id}`；app 层可用作窗口登记的键）
pub const FENCE_LABEL_PREFIX: &str = "fence-";

/// 标题栏高度（逻辑像素；收纳箱窗口标题栏 `crates/app/src/windows/fence/mod.rs` 直接用它，旧 Tauri 版前端对应
/// `FENCE_TITLE_BAR_HEIGHT`）。
/// 折叠态窗口高度 = 它 × 缩放比例
pub const FENCE_TITLE_BAR_DIP: f64 = 32.0;
/// 默认图标格（逻辑像素），参考 LucidDesk 的 88×96 DIP
pub const CELL_WIDTH_DIP: f64 = 88.0;
pub const CELL_HEIGHT_DIP: f64 = 96.0;
/// 新建收纳箱默认能放 4 列 × 3 行图标
pub const DEFAULT_COLUMNS: f64 = 4.0;
pub const DEFAULT_ROWS: f64 = 3.0;
/// 内容区内边距（逻辑像素），与 FenceView 的内容区 padding 一致
pub const CONTENT_PADDING_DIP: f64 = 8.0;
/// 最小宽度 / 展开态最小高度（逻辑像素）：展开态至少要放得下标题栏菜单
pub const MIN_WIDTH_DIP: f64 = 160.0;
pub const MIN_EXPANDED_HEIGHT_DIP: f64 = 160.0;
/// 新建 / 重置时离工作区边缘的距离、收纳箱之间的间距、叠放偏移（逻辑像素）
pub const SCREEN_MARGIN_DIP: f64 = 24.0;
pub const LAYOUT_GAP_DIP: f64 = 16.0;
pub const CASCADE_STEP_DIP: f64 = 32.0;

/// 默认标题；重名时加序号（"收纳箱 2"、"收纳箱 3"……）
pub const DEFAULT_FENCE_TITLE: &str = "收纳箱";
/// 标题最大长度（字符数），标题输入框的 maxlength 也用它
pub const MAX_TITLE_CHARS: usize = 40;
/// 图标大小上下限（逻辑像素），Ctrl+滚轮在此范围内调整
pub const MIN_ICON_SIZE: i32 = 24;
pub const MAX_ICON_SIZE: i32 = 128;

/// 本次运行中的临时几何（按收纳箱 id）：屏幕组合切换到一个没有记录的组合后"沿用当前位置"，不落库。
/// 用户拖动 / 缩放、重置位置后写入了当前组合的记录就移除；屏幕组合再次变化时整体重算
static SESSION_RECTS: Mutex<BTreeMap<i64, PxRect>> = Mutex::new(BTreeMap::new());

/// 上一次看到的屏幕组合 ID（`None` = 还没有收纳箱窗口 / 尚未初始化），见 [`observe_screen_config`]
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
pub struct PxRect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl PxRect {
    pub fn right(self) -> i32 {
        self.x + self.width
    }

    pub fn bottom(self) -> i32 {
        self.y + self.height
    }

    pub fn intersection_area(self, other: PxRect) -> i64 {
        let w = (self.right().min(other.right()) - self.x.max(other.x)).max(0) as i64;
        let h = (self.bottom().min(other.bottom()) - self.y.max(other.y)).max(0) as i64;
        w * h
    }

    pub fn inflate(self, by: i32) -> PxRect {
        PxRect {
            x: self.x - by,
            y: self.y - by,
            width: self.width + 2 * by,
            height: self.height + 2 * by,
        }
    }

    /// 点到矩形的距离平方（点在矩形内为 0）
    pub fn distance_sq_to(self, px: i32, py: i32) -> i64 {
        let dx = (self.x - px).max(px - self.right()).max(0) as i64;
        let dy = (self.y - py).max(py - self.bottom()).max(0) as i64;
        dx * dx + dy * dy
    }
}

/// 库里的「最近一次位置」
pub fn stored_rect(fence: &Fence) -> PxRect {
    PxRect {
        x: fence.x,
        y: fence.y,
        width: fence.width,
        height: fence.height,
    }
}

/// 收纳箱在当前屏幕组合下应使用的几何：该组合的记录 → 本次运行的临时几何 → 最近一次位置
pub fn effective_rect(
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
pub fn layout_after_switch(
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
pub fn committed_rect(window: PxRect, expanded_view: bool, current: PxRect) -> PxRect {
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
pub fn next_default_title(existing: &[String]) -> String {
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
pub fn default_fence_rect(
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
pub fn pick_work_area(probe: PxRect, works: &[PxRect]) -> Option<PxRect> {
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
pub fn clamp_into(rect: PxRect, work: PxRect) -> PxRect {
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
pub fn view_rect(stored: PxRect, expanded: bool, title_height: i32, works: &[PxRect]) -> PxRect {
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

/// 一块显示器：工作区（物理像素；取不到工作区时用整块显示器）与缩放比例。主显示器排第一
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MonitorArea {
    pub work: PxRect,
    pub scale: f64,
}

/// 标题栏所在显示器（选法同 [`pick_work_area`]）的缩放比例；没有显示器信息时按 1.0。
///
/// 折叠态高度、建窗时的逻辑坐标都要按**目标位置**所在显示器的缩放算，不能取窗口当前所在
/// 显示器的：混合 DPI 下从一块屏挪到另一块屏（重置位置、启动夹回）时两者不同。
pub fn title_scale(stored: PxRect, areas: &[MonitorArea]) -> f64 {
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
pub fn fence_view(stored: PxRect, expanded: bool, areas: &[MonitorArea]) -> (PxRect, f64) {
    let scale = title_scale(stored, areas);
    let works: Vec<PxRect> = areas.iter().map(|a| a.work).collect();
    (
        view_rect(stored, expanded, title_bar_px(scale), &works),
        scale,
    )
}

/// 托盘「重置收纳箱位置」：在工作区里从右上角起从右往左、逐行排开，放不下时回到右上角错开叠放。
/// 返回每个收纳箱的新左上角（尺寸由调用方先夹到工作区内）。
pub fn reset_layout(work: PxRect, sizes: &[(i32, i32)], margin: i32, gap: i32) -> Vec<(i32, i32)> {
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
pub fn normalize_title(raw: &str) -> Result<String, String> {
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
pub fn normalize_color(raw: &str) -> Result<String, String> {
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
/// 文件夹改名失败时标题照改、`folder_name` 保持旧值，失败原因放在 `folder_error` 里给界面提示（PRD R1）。
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

/// 只更新 patch 里给出的字段；背景色须为 `#RRGGBB`，透明度夹到 0~1，图标大小夹到上下限，
/// `show_hidden` 原样写入。任一字段校验失败时什么都不写
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
                    collapsed   = COALESCE(?1, collapsed),
                    locked      = COALESCE(?2, locked),
                    bg_color    = COALESCE(?3, bg_color),
                    bg_alpha    = COALESCE(?4, bg_alpha),
                    icon_size   = COALESCE(?5, icon_size),
                    show_hidden = COALESCE(?6, show_hidden),
                    updated_at  = datetime('now', 'localtime')
                 WHERE id = ?7",
                rusqlite::params![
                    patch.collapsed,
                    patch.locked,
                    bg_color,
                    bg_alpha,
                    icon_size,
                    patch.show_hidden,
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

// ============ 窗口标识与几何辅助 ============

/// 收纳箱窗口 label（`fence-{id}`）
pub fn fence_label(id: i64) -> String {
    format!("{FENCE_LABEL_PREFIX}{id}")
}

/// 是不是收纳箱窗口的 label
pub fn is_fence_label(label: &str) -> bool {
    label
        .strip_prefix(FENCE_LABEL_PREFIX)
        .is_some_and(|rest| !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit()))
}

/// 折叠态（只剩标题栏）的窗口高度（物理像素）
pub fn title_bar_px(scale: f64) -> i32 {
    (FENCE_TITLE_BAR_DIP * scale).round() as i32
}

/// 新建收纳箱的默认尺寸（物理像素）：4 列 × 3 行图标格 + 标题栏 + 内边距，按目标显示器缩放
pub fn default_fence_size(scale: f64) -> (i32, i32) {
    let px = |dip: f64| (dip * scale).round() as i32;
    (
        px(CELL_WIDTH_DIP * DEFAULT_COLUMNS + CONTENT_PADDING_DIP * 2.0),
        px(FENCE_TITLE_BAR_DIP + CELL_HEIGHT_DIP * DEFAULT_ROWS + CONTENT_PADDING_DIP * 2.0),
    )
}

/// 取不到任何显示器信息时新建收纳箱用的工作区（缩放按 1）
pub const FALLBACK_WORK_AREA: PxRect = PxRect {
    x: 0,
    y: 0,
    width: 1920,
    height: 1080,
};

/// 当前屏幕组合下各收纳箱应使用的几何（[`effective_rect`]：该组合的记录 → 本次运行的临时几何 →
/// 最近一次位置）
pub struct Placement {
    /// 屏幕组合 ID
    pub screen_id: String,
    layouts: HashMap<i64, PxRect>,
    session: BTreeMap<i64, PxRect>,
}

impl Placement {
    /// 读取 `screen_id` 组合下的位置记录（读库失败只记日志，按没有记录处理）
    pub fn for_screen(db: &Database, screen_id: &str) -> Placement {
        let layouts = db
            .with_connection(|conn| screen_layouts(conn, screen_id))
            .unwrap_or_else(|e| {
                log::warn!("[fence] 读取屏幕组合 {screen_id} 的收纳箱位置失败: {e}");
                HashMap::new()
            });
        Placement {
            screen_id: screen_id.to_string(),
            layouts,
            session: session_rects().clone(),
        }
    }

    /// 该收纳箱在这个屏幕组合下的存储几何（再经 [`fence_view`] 得到窗口应占的矩形）
    pub fn rect(&self, fence: &Fence) -> PxRect {
        effective_rect(fence, &self.layouts, &self.session)
    }

    /// 这个屏幕组合下有位置记录的收纳箱数
    pub fn layout_count(&self) -> usize {
        self.layouts.len()
    }
}

// ============ 组合操作（阻塞；app 在后台线程调用，然后自己处理窗口） ============

/// 全部收纳箱（按 sort_order、id）（`list_fences`）
pub fn list(db: &Database) -> Result<Vec<Fence>, String> {
    load_fences(db)
}

/// 读取单个收纳箱（`get_fence`）
pub fn get(db: &Database, id: i64) -> Result<Fence, String> {
    load_fence(db, id)
}

/// 更新折叠 / 锁定 / 外观 / 图标大小 /「显示隐藏的图标」（只改给出的字段，`update_fence`）。只写库：折叠态的
/// 窗口尺寸由 app 随后按 [`fence_view`] 呈现，锁定与隐藏项的显示由 app 控制
pub fn update(db: &Database, id: i64, patch: &UpdateFenceRequest) -> Result<Fence, String> {
    update_fence_row(db, id, patch)
}

/// 改标题并同步改收纳文件夹名（非法字符替换、重名加后缀，`rename_fence`）。文件夹改名失败时标题照改、
/// 保留旧文件夹名，原因在 `folder_error` 里；app 随后改窗口标题
pub fn rename(db: &Database, id: i64, title: &str) -> Result<RenameFenceResult, String> {
    rename_fence_and_folder(db, id, title, fence_fs::rename_fence_folder)
}

/// 新建收纳箱（`create_fence` / 托盘「新增收纳箱」的数据部分）：在 `work`（光标所在显示器的工作区，
/// 物理像素；取不到时用 [`FALLBACK_WORK_AREA`]、缩放 1）里从右上角起找不与已有收纳箱重叠的位置，取下一个
/// 默认标题，**先建收纳文件夹再插库**（插库失败回滚文件夹），并把位置记成 `screen_id` 组合的记录。
///
/// 返回的 `Fence` 的 `x/y/width/height` 就是窗口的存储几何。app 随后建窗；建窗失败调用
/// [`rollback_create`]，成功后确保目录监听已开（[`items::watch`]）。收纳箱全被隐藏时 app 先把它们显示回来。
pub fn create(db: &Database, work: PxRect, scale: f64, screen_id: &str) -> Result<Fence, String> {
    let existing = load_fences(db)?;
    let placement = Placement::for_screen(db, screen_id);
    let scale = scale.max(0.1);
    let px = |dip: f64| (dip * scale).round() as i32;
    let (width, height) = default_fence_size(scale);
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
    let folder = fence_fs::create_fence_folder(&title, &taken)?;
    let fence = match insert_fence(db, &title, rect, &folder) {
        Ok(fence) => fence,
        Err(e) => {
            rollback_fence_folder(&folder);
            return Err(e);
        }
    };
    // 新位置是按当前屏幕组合选的：一并记成该组合的位置，切到别的组合再切回来时回到这里
    if let Err(e) = db.with_connection(|conn| save_layout(conn, fence.id, screen_id, rect)) {
        log::warn!("[fence] 记录收纳箱 #{} 的屏幕组合位置失败: {e}", fence.id);
    }
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

/// 新建后窗口没建出来：删掉这条记录（否则下次启动会冒出一个用户从没见过的收纳箱）和刚建的空文件夹
pub fn rollback_create(db: &Database, fence: &Fence) {
    if let Err(e) = delete_fence_row(db, fence.id) {
        log::warn!("[fence] 回滚收纳箱 #{} 的记录失败: {e}", fence.id);
    }
    rollback_fence_folder(&fence.folder_name);
}

/// 删掉刚建的收纳文件夹（只删空文件夹，绝不递归删除）
fn rollback_fence_folder(folder: &str) {
    if let Err(e) = fence_fs::remove_fence_folder_if_empty(folder) {
        log::warn!("[fence] 回滚收纳文件夹失败: {e}");
    }
}

/// 应用启动恢复（原 `restore_fences` 的数据部分）：读出全部收纳箱，确保各自的收纳文件夹存在（用户在
/// 资源管理器里删掉了就按原名重建），返回每个收纳箱在 `screen_id` 组合下的存储几何。
///
/// 有收纳箱时把 `screen_id` 记为屏幕组合检测的起点（[`observe_screen_config`]）。app 随后逐个建窗、
/// 有收纳箱则开目录监听（[`items::watch`]），最后不论有没有收纳箱都调 [`items::spawn_startup_reconcile`]。
pub fn prepare_restore(db: &Database, screen_id: &str) -> Result<Vec<(Fence, PxRect)>, String> {
    let fences = load_fences(db)?;
    let placement = Placement::for_screen(db, screen_id);
    if !fences.is_empty() {
        log::info!(
            "[fence] 当前屏幕组合 {}（{} 个收纳箱有该组合的位置记录）",
            placement.screen_id,
            placement.layout_count()
        );
        *last_screen_id() = Some(placement.screen_id.clone());
    }
    Ok(fences
        .into_iter()
        .map(|fence| {
            if let Err(e) = fence_fs::ensure_fence_folder(&fence.folder_name) {
                log::warn!("[fence] 收纳箱 #{} 的文件夹不可用: {e}", fence.id);
            }
            let rect = placement.rect(&fence);
            (fence, rect)
        })
        .collect())
}

/// 用户拖动 / 缩放结束后落库（`commit_fence_geometry`）：`window` 为窗口当前外框（物理像素）。
///
/// `expanded_view`：窗口当前是否显示完整高度（展开，或折叠态下悬停临时展开）。是则位置与尺寸全部落库；
/// 否（只剩标题栏）则只落位置和宽度，展开高度保持不变。写 `screen_id` 组合的记录，同时更新最近一次位置，
/// 该收纳箱的临时几何作废。
pub fn commit_geometry(
    db: &Database,
    id: i64,
    screen_id: &str,
    window: PxRect,
    expanded_view: bool,
) -> Result<Fence, String> {
    let fence = load_fence(db, id)?;
    let placement = Placement::for_screen(db, screen_id);
    let rect = committed_rect(window, expanded_view, placement.rect(&fence));
    let saved = save_fence_geometry(db, id, screen_id, rect)?;
    // 当前组合已经有记录了，临时几何作废
    session_rects().remove(&id);
    Ok(saved)
}

/// 托盘「重置收纳箱位置」（`reset_fence_positions` 的数据部分）：把所有收纳箱在 `work`（主显示器工作区，
/// 物理像素）里从右上角起逐行排开（尺寸先夹到工作区内）并落库（写 `screen_id` 组合的记录 + 最近一次位置），
/// 清空临时几何。返回每个收纳箱与它的新存储几何；app 随后按 [`fence_view`] 摆窗，被隐藏的一并显示。
/// 没有收纳箱时返回空列表。
pub fn reset_positions(
    db: &Database,
    work: PxRect,
    scale: f64,
    screen_id: &str,
) -> Result<Vec<(Fence, PxRect)>, String> {
    let fences = load_fences(db)?;
    if fences.is_empty() {
        return Ok(Vec::new());
    }
    let placement = Placement::for_screen(db, screen_id);
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
            save_layout(tx, fence.id, screen_id, *rect)?;
        }
        Ok(())
    })
    .map_err(|e: rusqlite::Error| e.to_string())?;
    session_rects().clear();
    log::info!(
        "[fence] 已重置 {} 个收纳箱的位置（屏幕组合 {}）",
        positions.len(),
        screen_id
    );
    Ok(fences.into_iter().zip(rects).collect())
}

/// 屏幕组合检测（原 `tick_fence_screen_layouts`）：有收纳箱窗口时 app 约每 2 秒调用一次。
/// 记下 `screen_id`，与上一次不同时返回上一次的 ID（随后调 [`screen_switch`]）；第一次调用只记录。
pub fn observe_screen_config(screen_id: &str) -> Option<String> {
    let previous = last_screen_id().replace(screen_id.to_string());
    previous.filter(|previous| previous != screen_id)
}

/// 没有收纳箱窗口时调用：清掉检测起点（下次有窗口时重新开始，不读显示器信息）
pub fn forget_screen_config() {
    *last_screen_id() = None;
}

/// 屏幕组合从 `previous` 切到 `current` 之后各收纳箱窗口的新存储几何（原 `apply_screen_switch`）。
///
/// `windows`：有窗口的收纳箱 `(id, 窗口当前外框（取不到为 None）, 窗口当前缩放比例)`。新组合有记录就用记录；
/// 没有就沿用窗口当前位置（窗口只剩标题栏时展开高度沿用切换前的），记进本次运行的临时几何（不落库）。
/// 返回 `(收纳箱, 新存储几何)`，app 按 [`fence_view`]（展开态 = `!fence.collapsed`）摆窗。
pub fn screen_switch(
    db: &Database,
    previous: &str,
    current: &str,
    windows: &[(i64, Option<PxRect>, f64)],
) -> Vec<(Fence, PxRect)> {
    let fences = match load_fences(db) {
        Ok(fences) => fences,
        Err(e) => {
            log::warn!("[fence] 读取收纳箱失败: {e}");
            return Vec::new();
        }
    };
    // 切换前的几何：没有记录、窗口只剩标题栏时，用它的展开高度
    let before = Placement::for_screen(db, previous);
    let records = db
        .with_connection(|conn| screen_layouts(conn, current))
        .unwrap_or_else(|e| {
            log::warn!("[fence] 读取屏幕组合 {current} 的收纳箱位置失败: {e}");
            HashMap::new()
        });

    let mut session = BTreeMap::new();
    let mut targets = Vec::new();
    let mut restored = 0;
    for fence in fences {
        let Some(&(_, window, scale)) = windows.iter().find(|(id, _, _)| *id == fence.id) else {
            continue;
        };
        let previous_rect = before.rect(&fence);
        let current_rect = window.unwrap_or(previous_rect);
        let collapsed_view = current_rect.height < title_bar_px(scale) * 2;
        let record = records.get(&fence.id).copied();
        let target =
            layout_after_switch(record, current_rect, previous_rect.height, collapsed_view);
        if record.is_some() {
            restored += 1;
        } else {
            session.insert(fence.id, target);
        }
        targets.push((fence, target));
    }
    *session_rects() = session;
    log::info!(
        "[fence] 已按屏幕组合 {current} 摆放 {} 个收纳箱（{restored} 个恢复到该组合的记录）",
        targets.len()
    );
    targets
}

/// `delete` 的结果
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteFenceResult {
    /// 已删除：所有项目都已移回、收纳文件夹已删除、记录已删（app 随后销毁窗口）。
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

/// 删除收纳箱（`delete_fence`）：先把收纳文件夹里的项目全部移回原位置（原目录不存在则回桌面，重名自动改名），
/// 全部移走、空文件夹删除后才删记录（`fence_items` / `fence_layouts` 级联删除）。
///
/// - 还原整体无法进行（`Err`，例如桌面路径未知）：什么都不删，直接返回错误
/// - 有项目没能移回（文件被占用、用户取消管理员权限对话框）：删除中止，收纳箱整个保留（记录、窗口、文件夹）；
///   已移回的项删掉对应的 `fence_items` 行，`deleted = false`。app 随后用 [`items::reconcile`] 刷新该窗口
/// - `deleted = true`：app 销毁窗口（不经过关闭拦截）并注销嵌入登记
///
/// `owner_hwnd` 是收纳箱窗口的 HWND（系统对话框的父窗口）。**阻塞，且绝不能在 UI 主线程调用**：系统
/// "需要管理员权限" / "文件正在使用"对话框会同步给主线程发消息，主线程等待时会死锁。
pub fn delete(
    db: &Database,
    id: i64,
    owner_hwnd: Option<isize>,
) -> Result<DeleteFenceResult, String> {
    // 先登记再读文件夹名（见 items::FenceActivity）：还原期间（可能在等用户处理管理员权限对话框）
    // 对账跳过它、不能改名、不能再移入移出；有文件正在移动时直接拒绝删除
    let deleting = begin_deleting(id)?;
    let fence = load_fence(db, id)?;
    let originals = db
        .with_connection(|conn| fence_originals(conn, id))
        .map_err(|e| e.to_string())?;
    let restored = fence_fs::restore_all(&fence.folder_name, &originals, owner_hwnd)?;
    let plan = plan_delete(&restored);

    if plan.delete_fence {
        delete_fence_row(db, id)?;
        drop(deleting);
        session_rects().remove(&id);
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
        // 先注销"删除中"，之后的对账才不会跳过它
        drop(deleting);
        if let Err(e) = removed {
            log::warn!("[fence] 删除已移回项目的记录失败（下次对账会清理）: {e}");
        }
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
    fn show_hidden_defaults_off_and_is_saved_per_fence() {
        let db = Database::new_in_memory().unwrap();
        let a = insert_fence(&db, "收纳箱", rect(10, 20, 368, 334), "收纳箱").unwrap();
        let b = insert_fence(&db, "收纳箱 2", rect(40, 50, 368, 334), "收纳箱 2").unwrap();
        assert!(
            !a.show_hidden && !b.show_hidden,
            "新建收纳箱默认不显示隐藏的图标"
        );

        let shown = update_fence_row(
            &db,
            a.id,
            &UpdateFenceRequest {
                show_hidden: Some(true),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(shown.show_hidden);
        assert!(
            !shown.collapsed && !shown.locked && shown.icon_size == 48,
            "只改 show_hidden"
        );
        assert!(load_fence(&db, a.id).unwrap().show_hidden, "写进了库");
        assert!(
            !load_fence(&db, b.id).unwrap().show_hidden,
            "每个收纳箱独立"
        );

        // 不给出 show_hidden 的 patch 不动它
        let locked = update_fence_row(
            &db,
            a.id,
            &UpdateFenceRequest {
                locked: Some(true),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(locked.show_hidden && locked.locked);

        let hidden_again = update_fence_row(
            &db,
            a.id,
            &UpdateFenceRequest {
                show_hidden: Some(false),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(!hidden_again.show_hidden);
        let json = serde_json::to_string(&hidden_again).unwrap();
        assert!(json.contains("\"showHidden\":false"), "{json}");
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
        use super::items::begin_moving;
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

    /// 测试里的 Windows 路径字面量：非 Windows 上把 `\` 换成 `/`，`file_name()` 才取得到文件名
    fn native(path: &str) -> String {
        if cfg!(windows) {
            path.to_string()
        } else {
            path.replace('\\', "/")
        }
    }

    fn op(source: &str, status: ItemOpStatus, error: Option<&str>) -> ItemOpResult {
        ItemOpResult {
            source: native(source),
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

    // ---- 组合操作（原各命令里与窗口无关的部分）----

    /// 插一条指定 id 的收纳箱（这几条测试会动进程级的临时几何，id 与其它测试错开）
    fn insert_with_id(db: &Database, id: i64, r: PxRect, collapsed: bool) {
        db.with_connection(|conn| {
            conn.execute(
                "INSERT INTO fences (id, title, folder_name, x, y, width, height, collapsed, sort_order, created_at, updated_at)
                 VALUES (?1, ?2, ?2, ?3, ?4, ?5, ?6, ?7, ?1, '', '')",
                rusqlite::params![id, format!("箱{id}"), r.x, r.y, r.width, r.height, collapsed],
            )
        })
        .unwrap();
    }

    /// 临时几何（`SESSION_RECTS`）只在这一条测试里读写，避免并行测试互相干扰
    #[test]
    fn commit_reset_and_screen_switch_keep_layouts_and_session_geometry_consistent() {
        let db = Database::new_in_memory().unwrap();
        let (a, b) = (720_001, 720_002);
        insert_with_id(&db, a, rect(100, 100, 368, 334), false);
        insert_with_id(&db, b, rect(600, 100, 368, 334), true);

        // 屏幕组合 A → B：a 在 B 下没有记录，沿用窗口当前位置并只记在内存里；b 只剩标题栏，
        // 展开高度沿用切换前的
        let switched = screen_switch(
            &db,
            SCREEN_A,
            SCREEN_B,
            &[(a, Some(rect(150, 120, 368, 334)), 1.0), (b, None, 1.0)],
        );
        let targets: HashMap<i64, PxRect> = switched.iter().map(|(f, r)| (f.id, *r)).collect();
        assert_eq!(targets[&a], rect(150, 120, 368, 334));
        assert_eq!(
            targets[&b],
            rect(600, 100, 368, 334),
            "取不到窗口外框时用切换前的几何"
        );
        assert!(layouts_of(&db, SCREEN_B).is_empty(), "切换不落库");
        assert_eq!(
            Placement::for_screen(&db, SCREEN_B).rect(&load_fence(&db, a).unwrap()),
            rect(150, 120, 368, 334),
            "本次运行的临时几何"
        );

        // 拖动结束落库：折叠视图（只剩标题栏）保留展开高度；临时几何作废
        let saved = commit_geometry(&db, a, SCREEN_B, rect(160, 130, 400, 32), false).unwrap();
        assert_eq!(stored_rect(&saved), rect(160, 130, 400, 334));
        assert_eq!(layouts_of(&db, SCREEN_B)[&a], rect(160, 130, 400, 334));
        assert!(commit_geometry(&db, 999_999, SCREEN_B, rect(0, 0, 1, 1), true).is_err());

        // 托盘重置：两个都排进工作区并落库到当前组合
        let work = rect(0, 0, 1920, 1040);
        let reset = reset_positions(&db, work, 1.0, SCREEN_B).unwrap();
        assert_eq!(reset.len(), 2);
        for (fence, r) in &reset {
            assert_eq!(clamp_into(*r, work), *r, "#{} 在工作区内", fence.id);
            assert_eq!(layouts_of(&db, SCREEN_B)[&fence.id], *r);
            assert_eq!(stored_rect(&load_fence(&db, fence.id).unwrap()), *r);
        }
        assert_eq!(reset[0].1.right(), 1920 - 24, "从右上角起排");
        assert!(
            reset_positions(&Database::new_in_memory().unwrap(), work, 1.0, SCREEN_B)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn screen_config_changes_are_reported_once() {
        forget_screen_config();
        assert_eq!(observe_screen_config(SCREEN_A), None, "第一次只记录");
        assert_eq!(observe_screen_config(SCREEN_A), None);
        assert_eq!(
            observe_screen_config(SCREEN_B).as_deref(),
            Some(SCREEN_A),
            "变化时返回上一次的组合"
        );
        assert_eq!(observe_screen_config(SCREEN_B), None);
        forget_screen_config();
        assert_eq!(
            observe_screen_config(SCREEN_A),
            None,
            "没有窗口之后重新开始"
        );
    }

    #[test]
    fn default_size_scales_with_the_monitor() {
        assert_eq!(default_fence_size(1.0), (368, 336));
        assert_eq!(default_fence_size(1.5), (552, 504));
    }
}
