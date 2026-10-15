//! 桌面收纳箱（10-15 desktop-fence，集成阶段）：收纳箱**内容**——把文件层 `services::fence_fs`
//! 包装成 Tauri 命令，并维护 `fence_items`（排序与原始路径）。
//!
//! - **文件夹是内容的 source of truth**：每次列举都按收纳文件夹对账（[`reconcile_rows`]）：文件夹有、
//!   表里没有 → 补行（`original_path = NULL`，排在末尾）；表里有、文件夹没有 → 删行。结果按
//!   `fence_items.sort_order` 排序返回
//! - 对账在**数据库锁内**列举文件夹：与移入 / 移回 / 改名后的写库串行，不会用一份过期的列举结果
//!   删掉刚 upsert 的行（例如监听回调在移入完成前列举、移入写库后才对账）
//! - 移入 / 跨箱写库一律 upsert（`ON CONFLICT(fence_id, file_name) DO UPDATE SET original_path = …`）：
//!   监听回调触发的对账可能已经先补了一行 `original_path = NULL`
//!
//! 线程：`fence_fs` 的函数都是阻塞的，一律 `spawn_blocking`。带 owner HWND 的移动**绝不在主线程**调用
//! （系统"需要管理员权限"对话框以收纳箱窗口为父窗口，会同步给主线程发消息，主线程等结果时会死锁）：
//! 这里的移动命令都是 async，HWND 在 async 上下文里取，移动在阻塞线程池里执行。
//!
//! 监听：第一个收纳箱出现时（启动恢复 / 新建）开始监听收纳根目录（[`ensure_fence_watcher`]），
//! 变化经 300ms 去抖后对受影响的收纳箱对账，再把对账结果发给对应窗口（[`FENCE_ITEMS_CHANGED_EVENT`]）。
//! 没有收纳箱的用户不会因此在用户目录下多出一个 `MiniTodo收纳箱` 文件夹。

use crate::db::{settings_kv, Database, Fence, FenceItem};
use crate::services::fence_fs::{
    self, FenceChange, FenceEntry, FenceWatcher, ItemOpResult, ItemOpStatus,
};
use rusqlite::{Connection, OptionalExtension};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::Path;
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager, State, WebviewWindow};

use super::fence::{fence_label, fence_window, load_fence, load_fences};

/// 收纳箱内容变化事件：发给对应窗口 `fence-{id}`，payload 为对账后的项目列表（`Vec<FenceEntry>`）
pub const FENCE_ITEMS_CHANGED_EVENT: &str = "fence-items-changed";

/// 首次收纳时的 OneDrive 提示是否已显示过（`settings` 键，v29 迁移写入默认值 `false`）。
///
/// **设备本地**：不在 `SYNCED_SETTING_KEYS`、不在 `AppSettings`，导入导出与 WebDAV 同步都不碰它
/// （桌面是否被 OneDrive 重定向本来就是这台设备的事）
pub const ONEDRIVE_NOTICE_KEY: &str = "fence_onedrive_notice_shown";

/// 收纳根目录的监听句柄（托管状态，在 `lib.rs` setup 里 manage；一直持有，drop 即停止监听）
#[derive(Default)]
pub struct FenceWatcherState(Mutex<Option<FenceWatcher>>);

/// 收纳箱上进行中的文件操作。移动 / 删除可能要等用户处理管理员权限、"文件正在使用"对话框，
/// 不能一直占着数据库锁，靠这里的登记互相排斥：
///
/// - **删除中**（批量还原）：不能再移入 / 移出、不能改名；监听回调 / 窗口刷新触发的对账跳过它
///   （还原结束时文件夹已删除，对账会按原名把空文件夹重建出来，留下一个孤儿文件夹）
/// - **移动中**（移入 / 移回 / 跨箱）：不能删除；改名只改标题、文件夹暂不改名。移动用的是开始时读到的
///   文件夹路径，中途删除 / 改名会让文件落进按旧名重建的文件夹，成了没有收纳箱管理的孤儿文件夹
///
/// 登记都在读取 `folder_name` **之前**完成；改名与对账在数据库锁内检查登记，与移动 / 删除读取文件夹名的
/// 数据库访问互斥，因此不会出现"已按旧文件夹名开始移动 / 还原，文件夹却被改名"的交错。
/// 锁顺序：数据库锁 → 本锁，持有本锁时绝不去拿数据库锁。
struct FenceActivity {
    deleting: BTreeSet<i64>,
    /// 收纳箱 id → 进行中的移动数
    moving: BTreeMap<i64, usize>,
}

static FENCE_ACTIVITY: Mutex<FenceActivity> = Mutex::new(FenceActivity {
    deleting: BTreeSet::new(),
    moving: BTreeMap::new(),
});

fn activity() -> std::sync::MutexGuard<'static, FenceActivity> {
    FENCE_ACTIVITY
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// 删除中的登记，drop 时注销
pub(crate) struct DeletingGuard(i64);

impl Drop for DeletingGuard {
    fn drop(&mut self) {
        activity().deleting.remove(&self.0);
    }
}

/// 移动中的登记（涉及的收纳箱各计一次），drop 时注销
pub(crate) struct MovingGuard(Vec<i64>);

impl Drop for MovingGuard {
    fn drop(&mut self) {
        let mut activity = activity();
        for id in &self.0 {
            if let Some(count) = activity.moving.get_mut(id) {
                *count = count.saturating_sub(1);
                if *count == 0 {
                    activity.moving.remove(id);
                }
            }
        }
    }
}

/// 登记"正在删除"；已在删除中、或有文件正在移进 / 移出它时返回 `Err`
pub(crate) fn begin_deleting(id: i64) -> Result<DeletingGuard, String> {
    let mut activity = activity();
    if activity.deleting.contains(&id) {
        return Err("收纳箱正在删除".to_string());
    }
    if activity.moving.contains_key(&id) {
        return Err("收纳箱里还有文件正在移动，请稍后再删除".to_string());
    }
    activity.deleting.insert(id);
    Ok(DeletingGuard(id))
}

/// 登记"正在移动"（来源与目标收纳箱都要登记）；其中有收纳箱正在删除时返回 `Err`
pub(crate) fn begin_moving(ids: &[i64]) -> Result<MovingGuard, String> {
    let mut activity = activity();
    if ids.iter().any(|id| activity.deleting.contains(id)) {
        return Err("收纳箱正在删除".to_string());
    }
    let mut unique = ids.to_vec();
    unique.sort_unstable();
    unique.dedup();
    for id in &unique {
        *activity.moving.entry(*id).or_insert(0) += 1;
    }
    Ok(MovingGuard(unique))
}

pub(crate) fn is_deleting(id: i64) -> bool {
    activity().deleting.contains(&id)
}

pub(crate) fn is_moving(id: i64) -> bool {
    activity().moving.contains_key(&id)
}

/// 在阻塞线程池里执行（`fence_fs` 的文件操作、Shell 调用都是阻塞的）
pub(crate) async fn blocking<R, F>(f: F) -> Result<R, String>
where
    F: FnOnce() -> Result<R, String> + Send + 'static,
    R: Send + 'static,
{
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| format!("后台任务异常: {e}"))?
}

/// 收纳箱窗口的 HWND（系统对话框的父窗口）。
///
/// 只在 async 命令里调用：在非主线程取句柄要跨线程往返主线程，主线程本身取不会死锁，但带 owner 的
/// 移动绝不能在主线程执行（见模块注释）
#[cfg(target_os = "windows")]
pub(crate) fn owner_hwnd(window: &WebviewWindow) -> Option<isize> {
    window.hwnd().ok().map(|h| h.0 as isize)
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn owner_hwnd(_window: &WebviewWindow) -> Option<isize> {
    None
}

fn fence_owner(app: &AppHandle, id: i64) -> Option<isize> {
    fence_window(app, id).and_then(|w| owner_hwnd(&w))
}

// ============ fence_items 读写（纯数据库，有单元测试） ============

fn load_item_rows(conn: &Connection, fence_id: i64) -> rusqlite::Result<Vec<FenceItem>> {
    let mut stmt = conn.prepare(
        "SELECT id, fence_id, file_name, original_path, sort_order FROM fence_items
         WHERE fence_id = ?1 ORDER BY sort_order, id",
    )?;
    let rows = stmt.query_map([fence_id], |r| {
        Ok(FenceItem {
            id: r.get(0)?,
            fence_id: r.get(1)?,
            file_name: r.get(2)?,
            original_path: r.get(3)?,
            sort_order: r.get(4)?,
        })
    })?;
    rows.collect()
}

/// 某项的原始路径（没有这一行、或在资源管理器里直接放进来的项为 `None`）。
/// 名字先精确匹配，再不区分大小写匹配（NTFS 语义）
fn item_original_path(
    conn: &Connection,
    fence_id: i64,
    file_name: &str,
) -> rusqlite::Result<Option<String>> {
    let rows = load_item_rows(conn, fence_id)?;
    let lower = file_name.to_lowercase();
    Ok(rows
        .iter()
        .find(|r| r.file_name == file_name)
        .or_else(|| rows.iter().find(|r| r.file_name.to_lowercase() == lower))
        .and_then(|r| r.original_path.clone()))
}

/// 删除收纳箱时传给 `restore_all` 的 `file_name → original_path`（只含有原始路径的行）
pub(crate) fn fence_originals(
    conn: &Connection,
    fence_id: i64,
) -> rusqlite::Result<HashMap<String, String>> {
    Ok(load_item_rows(conn, fence_id)?
        .into_iter()
        .filter_map(|r| r.original_path.map(|p| (r.file_name, p)))
        .collect())
}

/// 移入 / 跨箱后写一行：新行排在末尾；已有同名行（监听对账先补的 NULL 行）只更新原始路径、保留排序
fn upsert_item(
    conn: &Connection,
    fence_id: i64,
    file_name: &str,
    original_path: Option<&str>,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO fence_items (fence_id, file_name, original_path, sort_order)
         VALUES (?1, ?2, ?3, (SELECT COALESCE(MAX(sort_order), -1) + 1 FROM fence_items WHERE fence_id = ?1))
         ON CONFLICT(fence_id, file_name) DO UPDATE SET original_path = excluded.original_path",
        rusqlite::params![fence_id, file_name, original_path],
    )?;
    Ok(())
}

/// 删除某项的行（名字不区分大小写）
pub(crate) fn delete_item(
    conn: &Connection,
    fence_id: i64,
    file_name: &str,
) -> rusqlite::Result<()> {
    let lower = file_name.to_lowercase();
    for row in load_item_rows(conn, fence_id)? {
        if row.file_name == file_name || row.file_name.to_lowercase() == lower {
            conn.execute("DELETE FROM fence_items WHERE id = ?1", [row.id])?;
        }
    }
    Ok(())
}

/// 保存箱内排序：`file_names` 依次编号；表里没有的名字忽略（下次对账会补行并排到末尾）
fn reorder_rows(conn: &Connection, fence_id: i64, file_names: &[String]) -> rusqlite::Result<()> {
    let mut stmt = conn
        .prepare("UPDATE fence_items SET sort_order = ?1 WHERE fence_id = ?2 AND file_name = ?3")?;
    for (index, name) in file_names.iter().enumerate() {
        stmt.execute(rusqlite::params![index as i64, fence_id, name])?;
    }
    Ok(())
}

/// 按文件夹实际内容对账 `fence_items`，返回按 `sort_order` 排好的项目。
///
/// - 名字先精确匹配，剩下的再不区分大小写匹配（资源管理器里只改大小写的重命名保留原始路径与排序，
///   并把行里的名字改成新写法）
/// - 文件夹有、表里没有 → 补行（`original_path = NULL`），按列举顺序（文件名）排在末尾
/// - 表里有、文件夹没有 → 删行
fn reconcile_rows(
    conn: &Connection,
    fence_id: i64,
    entries: Vec<FenceEntry>,
) -> rusqlite::Result<Vec<FenceEntry>> {
    let rows = load_item_rows(conn, fence_id)?;
    let mut row_of: Vec<Option<usize>> = vec![None; entries.len()];
    let mut used = vec![false; rows.len()];

    let exact: HashMap<&str, usize> = rows
        .iter()
        .enumerate()
        .map(|(i, r)| (r.file_name.as_str(), i))
        .collect();
    for (e, entry) in entries.iter().enumerate() {
        if let Some(&r) = exact.get(entry.file_name.as_str()) {
            row_of[e] = Some(r);
            used[r] = true;
        }
    }
    for (e, entry) in entries.iter().enumerate() {
        if row_of[e].is_some() {
            continue;
        }
        let lower = entry.file_name.to_lowercase();
        if let Some(r) =
            (0..rows.len()).find(|&r| !used[r] && rows[r].file_name.to_lowercase() == lower)
        {
            conn.execute(
                "UPDATE fence_items SET file_name = ?1 WHERE id = ?2",
                rusqlite::params![entry.file_name, rows[r].id],
            )?;
            row_of[e] = Some(r);
            used[r] = true;
        }
    }

    for (r, row) in rows.iter().enumerate() {
        if !used[r] {
            conn.execute("DELETE FROM fence_items WHERE id = ?1", [row.id])?;
        }
    }

    let mut next = rows.iter().map(|r| r.sort_order).max().map_or(0, |m| m + 1);
    let mut keyed: Vec<(i32, String, FenceEntry)> = Vec::with_capacity(entries.len());
    for (e, entry) in entries.into_iter().enumerate() {
        let order = match row_of[e] {
            Some(r) => rows[r].sort_order,
            None => {
                conn.execute(
                    "INSERT INTO fence_items (fence_id, file_name, original_path, sort_order)
                     VALUES (?1, ?2, NULL, ?3)",
                    rusqlite::params![fence_id, entry.file_name, next],
                )?;
                next += 1;
                next - 1
            }
        };
        keyed.push((order, entry.file_name.to_lowercase(), entry));
    }
    keyed.sort_by(|a, b| (a.0, &a.1).cmp(&(b.0, &b.1)));
    Ok(keyed.into_iter().map(|(_, _, entry)| entry).collect())
}

/// 首次收纳的 OneDrive 提示：还没提示过就记下"已提示"并返回 true（只会返回一次 true）
fn claim_onedrive_notice(conn: &Connection) -> rusqlite::Result<bool> {
    if settings_kv::get_bool_setting(conn, ONEDRIVE_NOTICE_KEY, false) {
        return Ok(false);
    }
    settings_kv::set_bool_setting(conn, ONEDRIVE_NOTICE_KEY, true)?;
    Ok(true)
}

// ============ 对账（数据库 + 文件夹） ============

/// 列举收纳文件夹并对账，返回排好序的项目。
///
/// 收纳箱不存在或正在删除时返回 `Ok(None)`。文件夹不存在（用户在资源管理器里删掉了）时按原名重建
/// （`ensure_fence_folder`）后显示为空。整个过程持有数据库锁（见模块注释），列举只是一次本地 `read_dir`。
///
/// "正在删除"在数据库锁内检查：`delete_fence` 先登记再读库，登记之前已拿到锁的对账会在它读库前做完，
/// 之后的对账一定看得到登记，不会在还原途中按原名把刚删掉的文件夹重建出来。
pub(crate) fn reconcile_fence(
    db: &Database,
    fence_id: i64,
) -> Result<Option<Vec<FenceEntry>>, String> {
    db.with_transaction(
        |tx| -> rusqlite::Result<Result<Option<Vec<FenceEntry>>, String>> {
            if is_deleting(fence_id) {
                return Ok(Ok(None));
            }
            let folder: Option<String> = tx
                .query_row(
                    "SELECT folder_name FROM fences WHERE id = ?1",
                    [fence_id],
                    |r| r.get(0),
                )
                .optional()?;
            let Some(folder) = folder else {
                return Ok(Ok(None));
            };
            let entries = match fence_fs::list_fence_items(&folder) {
                Ok(entries) => entries,
                Err(first) => {
                    let retried = fence_fs::ensure_fence_folder(&folder)
                        .and_then(|_| fence_fs::list_fence_items(&folder));
                    match retried {
                        Ok(entries) => {
                            log::warn!(
                                "[fence] 收纳文件夹「{folder}」不存在，已按原名重建（{first}）"
                            );
                            entries
                        }
                        Err(e) => return Ok(Err(e)),
                    }
                }
            };
            Ok(Ok(Some(reconcile_rows(tx, fence_id, entries)?)))
        },
    )
    .map_err(|e| format!("读取收纳箱内容失败: {e}"))?
}

/// 对账后把结果发给收纳箱窗口（窗口不存在时 emit_to 不投递，无副作用）
pub(crate) fn reconcile_and_notify(app: &AppHandle, db: &Database, fence_id: i64) {
    match reconcile_fence(db, fence_id) {
        Ok(Some(items)) => {
            if let Err(e) = app.emit_to(fence_label(fence_id), FENCE_ITEMS_CHANGED_EVENT, items) {
                log::warn!("[fence] 通知收纳箱 #{fence_id} 刷新失败: {e}");
            }
        }
        Ok(None) => {}
        Err(e) => log::warn!("[fence] 收纳箱 #{fence_id} 对账失败: {e}"),
    }
}

/// 监听回调里受影响的收纳箱：`rescan` 时全部，否则按文件夹名（不区分大小写）匹配；
/// 根目录里不对应任何收纳箱的文件夹忽略
fn affected_fence_ids(fences: &[(i64, String)], change: &FenceChange) -> Vec<i64> {
    let changed: HashSet<String> = change.folders.iter().map(|n| n.to_lowercase()).collect();
    fences
        .iter()
        .filter(|(_, folder)| change.rescan || changed.contains(&folder.to_lowercase()))
        .map(|(id, _)| *id)
        .collect()
}

fn on_fence_change(app: &AppHandle, change: &FenceChange) {
    let db = app.state::<Database>();
    let fences = match load_fences(&db) {
        Ok(fences) => fences,
        Err(e) => {
            log::warn!("[fence] 读取收纳箱失败: {e}");
            return;
        }
    };
    let pairs: Vec<(i64, String)> = fences.into_iter().map(|f| (f.id, f.folder_name)).collect();
    for id in affected_fence_ids(&pairs, change) {
        reconcile_and_notify(app, &db, id);
    }
}

/// 开始监听收纳根目录（已在监听则什么都不做）。在有收纳箱之后调用：启动恢复、新建收纳箱
pub(crate) fn ensure_fence_watcher(app: &AppHandle) {
    let Some(state) = app.try_state::<FenceWatcherState>() else {
        return;
    };
    let mut slot = state
        .0
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if slot.is_some() {
        return;
    }
    let handle = app.clone();
    match fence_fs::watch_fence_root(move |change| on_fence_change(&handle, &change)) {
        Ok(watcher) => {
            log::info!("[fence] 开始监听收纳根目录 {}", watcher.root().display());
            *slot = Some(watcher);
        }
        Err(e) => {
            log::warn!("[fence] 无法监听收纳根目录，收纳箱不会随资源管理器里的改动自动刷新: {e}")
        }
    }
}

/// 启动对账后多久清理图标缓存：避开窗口刚打开时成批请求图标的时段（清理会删掉别人写到一半的临时文件）
const ICON_PRUNE_DELAY: std::time::Duration = std::time::Duration::from_secs(20);

/// 应用启动：所有收纳箱按文件夹对账，然后在后台清理图标缓存（只保留现存项目当前版本的缓存）
// 只有 Windows 的启动恢复会调用（收纳箱入口仅 cfg(windows)）
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub(crate) fn spawn_startup_reconcile(app: &AppHandle) {
    let app = app.clone();
    let spawned = std::thread::Builder::new()
        .name("fence-startup".to_string())
        .spawn(move || {
            let db = app.state::<Database>();
            let fences = load_fences(&db).unwrap_or_default();
            let mut live: Vec<String> = Vec::new();
            for fence in &fences {
                match reconcile_fence(&db, fence.id) {
                    Ok(Some(items)) => live.extend(items.into_iter().map(|item| item.path)),
                    Ok(None) => {}
                    Err(e) => {
                        log::warn!("[fence] 收纳箱 #{} 启动对账失败: {e}", fence.id);
                        // 对账失败的收纳箱不知道有哪些项目，这次不清理，免得删掉仍在用的缓存
                        return;
                    }
                }
            }
            if !fences.is_empty() {
                log_orphan_folders(&fences);
            }
            std::thread::sleep(ICON_PRUNE_DELAY);
            match fence_fs::prune_icon_cache(&live) {
                Ok(0) => {}
                Ok(n) => log::info!("[fence] 已清理 {n} 个过期的图标缓存"),
                Err(e) => log::warn!("[fence] 清理图标缓存失败: {e}"),
            }
        });
    if let Err(e) = spawned {
        log::warn!("[fence] 启动对账线程失败: {e}");
    }
}

/// 收纳根目录下不属于任何收纳箱的文件夹（例如删除收纳箱时有项目没能移回而保留下来的）：只记日志，便于排查
// 只有 Windows 的启动恢复会调用（见 spawn_startup_reconcile）
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn log_orphan_folders(fences: &[Fence]) {
    let known: HashSet<String> = fences
        .iter()
        .map(|f| f.folder_name.to_lowercase())
        .collect();
    match fence_fs::list_fence_folders() {
        Ok(folders) => {
            let orphans: Vec<String> = folders
                .into_iter()
                .filter(|folder| !known.contains(&folder.to_lowercase()))
                .collect();
            if !orphans.is_empty() {
                log::info!("[fence] 收纳根目录下有不属于任何收纳箱的文件夹: {orphans:?}");
            }
        }
        Err(e) => log::warn!("[fence] 读取收纳根目录失败: {e}"),
    }
}

/// 源路径是不是某个收纳文件夹里的一项（`<收纳根目录>\<文件夹>\<项>`）：是则返回文件夹名
fn source_fence_folder(source: &Path, root: &Path) -> Option<String> {
    let parent = source.parent()?;
    let grand = parent.parent()?;
    if !fence_fs::same_path_ci(grand, root) {
        return None;
    }
    parent.file_name()?.to_str().map(str::to_string)
}

/// 跨箱搬运的信息：来源收纳箱、原文件名、原始路径
struct Carried {
    from_id: i64,
    file_name: String,
    original_path: Option<String>,
}

// ============ Tauri 命令 ============

/// 收纳箱内容（按文件夹对账后，按排序返回）
#[tauri::command]
pub async fn get_fence_items(app: AppHandle, id: i64) -> Result<Vec<FenceEntry>, String> {
    blocking(move || {
        reconcile_fence(&app.state::<Database>(), id)?
            .ok_or_else(|| format!("收纳箱 #{id} 不存在或正在删除"))
    })
    .await
}

/// `fence_move_in` 的结果
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FenceMoveInResult {
    /// 与传入的 `paths` 一一对应
    pub results: Vec<ItemOpResult>,
    /// 需要显示一次性提示"桌面被 OneDrive 重定向，移出桌面会同步到其他设备"
    pub onedrive_notice: bool,
}

/// 外部拖入（前端 `onDragDropEvent` 的 drop）：把文件 / 快捷方式 / 文件夹移入收纳文件夹。
///
/// 来自另一个收纳箱文件夹的项按跨箱处理：搬运旧行的原始路径，而不是记成那个收纳文件夹里的路径
#[tauri::command]
pub async fn fence_move_in(
    app: AppHandle,
    id: i64,
    paths: Vec<String>,
) -> Result<FenceMoveInResult, String> {
    if paths.is_empty() {
        return Ok(FenceMoveInResult::default());
    }
    // 先登记再读文件夹名（见 FenceActivity）。从别的收纳箱拖来的项不登记来源：来源正被删除 / 改名时，
    // 该项要么移动失败留在原处，要么已经进了本收纳箱，都不会落进没人管的文件夹
    let moving = begin_moving(&[id])?;
    let owner = fence_owner(&app, id);
    blocking(move || {
        let _moving = moving;
        let db = app.state::<Database>();
        let fences = load_fences(&db)?;
        let fence = fences
            .iter()
            .find(|f| f.id == id)
            .ok_or_else(|| format!("收纳箱 #{id} 不存在"))?;
        let root = fence_fs::fence_root_dir()?;

        // 移动之前取旧行：移动一完成，监听回调的对账就可能先把旧行删掉
        let folder_ids: HashMap<String, i64> = fences
            .iter()
            .map(|f| (f.folder_name.to_lowercase(), f.id))
            .collect();
        let mut carried: HashMap<usize, Carried> = HashMap::new();
        for (index, source) in paths.iter().enumerate() {
            let source = Path::new(source);
            let Some(from_id) = source_fence_folder(source, &root)
                .and_then(|folder| folder_ids.get(&folder.to_lowercase()).copied())
                .filter(|from| *from != id)
            else {
                continue;
            };
            let Some(file_name) = source.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            let original_path = db
                .with_connection(|conn| item_original_path(conn, from_id, file_name))
                .map_err(|e| e.to_string())?;
            carried.insert(
                index,
                Carried {
                    from_id,
                    file_name: file_name.to_string(),
                    original_path,
                },
            );
        }

        let results = fence_fs::move_into_fence(&paths, &fence.folder_name, owner)?;

        let mut moved_from_desktop = false;
        let desktop = fence_fs::user_desktop_dir();
        // 文件已经移完：记账失败只记日志、照常返回逐项结果（下一次对账会补行，只是丢了原始路径），
        // 不能把已经成功的移动报成失败
        let saved = db.with_transaction(|tx| -> rusqlite::Result<()> {
            for (index, result) in results.iter().enumerate() {
                if result.status != ItemOpStatus::Moved {
                    continue;
                }
                let Some(name) = result.file_name.as_deref() else {
                    continue; // 罕见：已移动但取不到新文件名，交给下一次对账补行
                };
                match carried.get(&index) {
                    Some(c) => {
                        delete_item(tx, c.from_id, &c.file_name)?;
                        upsert_item(tx, id, name, c.original_path.as_deref())?;
                    }
                    None => {
                        upsert_item(tx, id, name, Some(&result.source))?;
                        moved_from_desktop |= desktop.as_deref().is_some_and(|d| {
                            fence_fs::path_starts_with_ci(Path::new(&result.source), d)
                        });
                    }
                }
            }
            Ok(())
        });
        if let Err(e) = saved {
            log::warn!(
                "[fence] 保存收纳箱 #{id} 的项目记录失败（文件已移入，下次对账会补行）: {e}"
            );
        }

        let onedrive_notice = moved_from_desktop
            && fence_fs::desktop_is_onedrive_redirected()
            && db
                .with_connection(claim_onedrive_notice)
                .unwrap_or_else(|e| {
                    log::warn!("[fence] 记录 OneDrive 提示状态失败: {e}");
                    false
                });

        let moved = results
            .iter()
            .filter(|r| r.status == ItemOpStatus::Moved)
            .count();
        log::info!(
            "[fence] 收纳箱 #{id} 移入 {moved}/{} 项（跨箱 {}）",
            results.len(),
            carried.len()
        );
        // 跨箱来源的收纳箱立即刷新（监听回调稍后也会刷新）
        let sources: BTreeSet<i64> = carried.values().map(|c| c.from_id).collect();
        for from_id in sources {
            reconcile_and_notify(&app, &db, from_id);
        }
        Ok(FenceMoveInResult {
            results,
            onedrive_notice,
        })
    })
    .await
}

/// 右键「移回桌面」：移回原目录（原目录不存在则回用户桌面），重名自动改名不覆盖
#[tauri::command]
pub async fn fence_restore_item(
    app: AppHandle,
    id: i64,
    file_name: String,
) -> Result<ItemOpResult, String> {
    let moving = begin_moving(&[id])?;
    let owner = fence_owner(&app, id);
    blocking(move || {
        let _moving = moving;
        let db = app.state::<Database>();
        let fence = load_fence(&db, id)?;
        let original = db
            .with_connection(|conn| item_original_path(conn, id, &file_name))
            .map_err(|e| e.to_string())?;
        let result =
            fence_fs::restore_item(&fence.folder_name, &file_name, original.as_deref(), owner)?;
        if result.status == ItemOpStatus::Moved {
            // 文件已经移回：记账失败只记日志（下一次对账会删掉这一行），不把成功的移回报成失败
            if let Err(e) = db.with_connection(|conn| delete_item(conn, id, &file_name)) {
                log::warn!(
                    "[fence] 删除收纳箱 #{id} 中 {file_name:?} 的记录失败（下次对账会清理）: {e}"
                );
            }
            log::info!(
                "[fence] 收纳箱 #{id} 移回 {:?} → {:?}",
                file_name,
                result.dest_path
            );
        }
        Ok(result)
    })
    .await
}

/// 右键「移动到…」：移到另一个收纳箱，搬运原始路径；重名加后缀
#[tauri::command]
pub async fn fence_move_item(
    app: AppHandle,
    id: i64,
    file_name: String,
    target_id: i64,
) -> Result<ItemOpResult, String> {
    if id == target_id {
        return Err("目标收纳箱与当前收纳箱相同".to_string());
    }
    let moving = begin_moving(&[id, target_id])?;
    let owner = fence_owner(&app, id);
    blocking(move || {
        let _moving = moving;
        let db = app.state::<Database>();
        let from = load_fence(&db, id)?;
        let to = load_fence(&db, target_id)?;
        let original = db
            .with_connection(|conn| item_original_path(conn, id, &file_name))
            .map_err(|e| e.to_string())?;
        let result =
            fence_fs::move_between_fences(&from.folder_name, &file_name, &to.folder_name, owner)?;
        if result.status == ItemOpStatus::Moved {
            // 文件已经移过去：记账失败只记日志（下一次对账会补行，只是丢了原始路径），不把成功的移动报成失败
            let saved = db.with_transaction(|tx| -> rusqlite::Result<()> {
                delete_item(tx, id, &file_name)?;
                if let Some(new_name) = result.file_name.as_deref() {
                    upsert_item(tx, target_id, new_name, original.as_deref())?;
                }
                Ok(())
            });
            if let Err(e) = saved {
                log::warn!("[fence] 保存跨箱移动的项目记录失败（下次对账会补行）: {e}");
            }
            log::info!(
                "[fence] {:?} 从收纳箱 #{id} 移到 #{target_id}（{:?}）",
                file_name,
                result.file_name
            );
            reconcile_and_notify(&app, &db, target_id);
        }
        Ok(result)
    })
    .await
}

/// 收纳箱里某项的完整路径（校验文件名，防止 `..` / 绝对路径逃逸出收纳文件夹）
fn item_path(db: &Database, id: i64, file_name: &str) -> Result<String, String> {
    if !fence_fs::is_valid_item_name(file_name) {
        return Err(format!("文件名不合法: {file_name:?}"));
    }
    let fence = load_fence(db, id)?;
    Ok(fence_fs::fence_folder_path(&fence.folder_name)?
        .join(file_name)
        .to_string_lossy()
        .into_owned())
}

/// 双击打开。目标失效的快捷方式约 3 秒后才返回错误（Shell 先静默尝试解析），前端显示"正在打开"
#[tauri::command]
pub async fn fence_open_item(app: AppHandle, id: i64, file_name: String) -> Result<(), String> {
    blocking(move || {
        let path = item_path(&app.state::<Database>(), id, &file_name)?;
        fence_fs::launch_item(&path)
    })
    .await
}

/// 右键「打开所在位置」：在资源管理器里打开收纳文件夹并选中该项
#[tauri::command]
pub async fn fence_reveal_item(app: AppHandle, id: i64, file_name: String) -> Result<(), String> {
    blocking(move || {
        let path = item_path(&app.state::<Database>(), id, &file_name)?;
        fence_fs::reveal_in_explorer(&path)
    })
    .await
}

/// 图标 PNG 的缓存文件路径（前端 `convertFileSrc` 加载）。`size_px` 为物理像素（逻辑尺寸 × DPR）
#[tauri::command]
pub async fn fence_item_icon(
    app: AppHandle,
    id: i64,
    file_name: String,
    size_px: u32,
) -> Result<String, String> {
    blocking(move || {
        let path = item_path(&app.state::<Database>(), id, &file_name)?;
        let png = fence_fs::icon_png(&path, size_px)?;
        Ok(png.to_string_lossy().into_owned())
    })
    .await
}

/// 保存箱内拖动排序（`file_names` 为新的完整顺序）
#[tauri::command]
pub fn reorder_fence_items(
    db: State<Database>,
    id: i64,
    file_names: Vec<String>,
) -> Result<(), String> {
    db.with_transaction(|tx| reorder_rows(tx, id, &file_names))
        .map_err(|e: rusqlite::Error| format!("保存排序失败: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str) -> FenceEntry {
        FenceEntry {
            file_name: name.to_string(),
            display_name: name.to_string(),
            path: format!(r"C:\root\箱\{name}"),
            is_dir: false,
            is_shortcut: name.ends_with(".lnk"),
            modified_ms: None,
        }
    }

    fn names(items: &[FenceEntry]) -> Vec<&str> {
        items.iter().map(|i| i.file_name.as_str()).collect()
    }

    fn db_with_fences(ids: &[i64]) -> Database {
        let db = Database::new_in_memory().unwrap();
        db.with_connection(|conn| {
            for id in ids {
                conn.execute(
                    "INSERT INTO fences (id, title, folder_name, x, y, width, height, created_at, updated_at)
                     VALUES (?1, '收纳箱', ?2, 0, 0, 368, 334, '', '')",
                    rusqlite::params![id, format!("箱{id}")],
                )?;
            }
            Ok(())
        })
        .unwrap();
        db
    }

    fn rows(db: &Database, fence_id: i64) -> Vec<(String, Option<String>, i32)> {
        db.with_connection(|conn| load_item_rows(conn, fence_id))
            .unwrap()
            .into_iter()
            .map(|r| (r.file_name, r.original_path, r.sort_order))
            .collect()
    }

    fn reconcile(db: &Database, fence_id: i64, entries: &[&str]) -> Vec<String> {
        let entries = entries.iter().map(|n| entry(n)).collect();
        db.with_transaction(|tx| reconcile_rows(tx, fence_id, entries))
            .map(|items: Vec<FenceEntry>| names(&items).iter().map(|s| s.to_string()).collect())
            .unwrap()
    }

    #[test]
    fn first_reconcile_adds_rows_in_listing_order() {
        let db = db_with_fences(&[1]);
        assert_eq!(
            reconcile(&db, 1, &["a.lnk", "b.txt", "c"]),
            ["a.lnk", "b.txt", "c"]
        );
        assert_eq!(
            rows(&db, 1),
            vec![
                ("a.lnk".into(), None, 0),
                ("b.txt".into(), None, 1),
                ("c".into(), None, 2)
            ]
        );
    }

    #[test]
    fn reconcile_follows_saved_order_drops_missing_and_appends_new_items() {
        let db = db_with_fences(&[1]);
        reconcile(&db, 1, &["a.lnk", "b.lnk", "c.lnk"]);
        db.with_connection(|conn| {
            reorder_rows(conn, 1, &["c.lnk".into(), "a.lnk".into(), "b.lnk".into()])
        })
        .unwrap();
        // b 在资源管理器里被删、新放进来 d 和 0（文件名排在最前，但新项一律排在末尾）
        assert_eq!(
            reconcile(&db, 1, &["0.lnk", "a.lnk", "c.lnk", "d.lnk"]),
            ["c.lnk", "a.lnk", "0.lnk", "d.lnk"]
        );
        let saved = rows(&db, 1);
        assert!(
            saved.iter().all(|(name, _, _)| name != "b.lnk"),
            "{saved:?}"
        );
        assert_eq!(saved.len(), 4);
    }

    #[test]
    fn case_only_rename_keeps_original_path_and_order() {
        let db = db_with_fences(&[1]);
        db.with_connection(|conn| {
            upsert_item(conn, 1, "a.lnk", None)?;
            upsert_item(
                conn,
                1,
                "chrome.lnk",
                Some(r"C:\Users\me\Desktop\chrome.lnk"),
            )
        })
        .unwrap();
        assert_eq!(
            reconcile(&db, 1, &["a.lnk", "Chrome.lnk"]),
            ["a.lnk", "Chrome.lnk"]
        );
        assert_eq!(
            rows(&db, 1)[1],
            (
                "Chrome.lnk".into(),
                Some(r"C:\Users\me\Desktop\chrome.lnk".into()),
                1
            )
        );
    }

    #[test]
    fn exact_match_wins_over_case_insensitive_match() {
        let db = db_with_fences(&[1]);
        db.with_connection(|conn| {
            upsert_item(conn, 1, "a.lnk", Some(r"C:\old\a.lnk"))?;
            upsert_item(conn, 1, "A.lnk", Some(r"C:\new\A.lnk"))
        })
        .unwrap();
        assert_eq!(reconcile(&db, 1, &["A.lnk"]), ["A.lnk"]);
        assert_eq!(
            rows(&db, 1),
            vec![("A.lnk".into(), Some(r"C:\new\A.lnk".into()), 1)]
        );
    }

    #[test]
    fn upsert_fills_in_the_original_path_of_a_row_added_by_reconcile() {
        let db = db_with_fences(&[1]);
        reconcile(&db, 1, &["a.lnk", "b.lnk"]);
        db.with_connection(|conn| {
            upsert_item(conn, 1, "a.lnk", Some(r"C:\Users\me\Desktop\a.lnk"))
        })
        .unwrap();
        assert_eq!(
            rows(&db, 1)[0],
            ("a.lnk".into(), Some(r"C:\Users\me\Desktop\a.lnk".into()), 0),
            "保留对账时排好的位置"
        );
        db.with_connection(|conn| upsert_item(conn, 1, "new.lnk", Some(r"C:\x\new.lnk")))
            .unwrap();
        assert_eq!(
            rows(&db, 1)[2],
            ("new.lnk".into(), Some(r"C:\x\new.lnk".into()), 2)
        );
        // 跨箱搬来的项没有原始路径：照样写 NULL
        db.with_connection(|conn| upsert_item(conn, 1, "a.lnk", None))
            .unwrap();
        assert_eq!(rows(&db, 1)[0].1, None);
    }

    #[test]
    fn originals_and_deletes_are_scoped_to_one_fence() {
        let db = db_with_fences(&[1, 2]);
        db.with_connection(|conn| {
            upsert_item(conn, 1, "a.lnk", Some(r"C:\d\a.lnk"))?;
            upsert_item(conn, 1, "b.lnk", None)?;
            upsert_item(conn, 2, "a.lnk", Some(r"C:\other\a.lnk"))
        })
        .unwrap();
        let originals = db.with_connection(|conn| fence_originals(conn, 1)).unwrap();
        assert_eq!(originals.len(), 1);
        assert_eq!(originals["a.lnk"], r"C:\d\a.lnk");
        assert_eq!(
            db.with_connection(|conn| item_original_path(conn, 1, "A.LNK"))
                .unwrap()
                .as_deref(),
            Some(r"C:\d\a.lnk"),
            "不区分大小写"
        );
        assert_eq!(
            db.with_connection(|conn| item_original_path(conn, 1, "missing.lnk"))
                .unwrap(),
            None
        );
        db.with_connection(|conn| delete_item(conn, 1, "A.lnk"))
            .unwrap();
        assert_eq!(rows(&db, 1), vec![("b.lnk".into(), None, 1)]);
        assert_eq!(rows(&db, 2).len(), 1, "另一个收纳箱的同名项不受影响");
    }

    #[test]
    fn reorder_ignores_unknown_names() {
        let db = db_with_fences(&[1]);
        reconcile(&db, 1, &["a", "b"]);
        db.with_connection(|conn| reorder_rows(conn, 1, &["b".into(), "ghost".into(), "a".into()]))
            .unwrap();
        assert_eq!(reconcile(&db, 1, &["a", "b"]), ["b", "a"]);
        assert_eq!(rows(&db, 1).len(), 2, "不为不存在的名字建行");
    }

    #[test]
    fn onedrive_notice_is_claimed_once_and_stays_device_local() {
        let db = Database::new_in_memory().unwrap();
        // v29 迁移写入了默认值
        assert_eq!(
            db.with_connection(|conn| settings_kv::get_setting(conn, ONEDRIVE_NOTICE_KEY))
                .unwrap()
                .as_deref(),
            Some("false")
        );
        assert!(db.with_connection(claim_onedrive_notice).unwrap());
        assert!(!db.with_connection(claim_onedrive_notice).unwrap());
        // 不参与同步：不在同步键里，也不在导出 / 同步的 AppSettings 里
        assert!(!settings_kv::is_synced_setting(ONEDRIVE_NOTICE_KEY));
        let settings = db
            .with_connection(|conn| Ok(super::super::data::read_app_settings(conn)))
            .unwrap();
        let json = serde_json::to_string(&settings).unwrap();
        assert!(!json.contains("onedrive"), "{json}");
    }

    #[test]
    fn affected_fences_match_folder_names_case_insensitively() {
        let fences = vec![
            (1, "常用".to_string()),
            (2, "Tools".to_string()),
            (3, "游戏".to_string()),
        ];
        let change = FenceChange {
            folders: vec!["tools".into(), "常用".into(), "不是收纳箱".into()],
            rescan: false,
        };
        assert_eq!(affected_fence_ids(&fences, &change), vec![1, 2]);
        let rescan = FenceChange {
            folders: vec![],
            rescan: true,
        };
        assert_eq!(affected_fence_ids(&fences, &rescan), vec![1, 2, 3]);
    }

    #[test]
    fn sources_inside_another_fence_folder_are_recognized() {
        let root = Path::new(r"C:\Users\me\MiniTodo收纳箱");
        assert_eq!(
            source_fence_folder(Path::new(r"C:\Users\me\MiniTodo收纳箱\常用\a.lnk"), root)
                .as_deref(),
            Some("常用")
        );
        assert_eq!(
            source_fence_folder(Path::new(r"c:\users\ME\minitodo收纳箱\Tools\b.txt"), root)
                .as_deref(),
            Some("Tools"),
            "不区分大小写"
        );
        // 收纳的文件夹内部、桌面上的项、根目录下的文件夹本身：都不是"另一个收纳箱里的一项"
        assert_eq!(
            source_fence_folder(
                Path::new(r"C:\Users\me\MiniTodo收纳箱\常用\sub\a.lnk"),
                root
            ),
            None
        );
        assert_eq!(
            source_fence_folder(Path::new(r"C:\Users\me\Desktop\a.lnk"), root),
            None
        );
        assert_eq!(
            source_fence_folder(Path::new(r"C:\Users\me\MiniTodo收纳箱\常用"), root),
            None
        );
    }

    #[test]
    fn deleting_guard_blocks_reentry_and_releases_on_drop() {
        let id = 987_654;
        let guard = begin_deleting(id).unwrap();
        assert!(is_deleting(id));
        assert!(begin_deleting(id).is_err());
        assert!(begin_moving(&[id]).is_err(), "删除中不能再移入 / 移出");
        drop(guard);
        assert!(!is_deleting(id));
        drop(begin_moving(&[id]).unwrap());
    }

    #[test]
    fn moving_blocks_deletion_until_every_move_is_done() {
        let (a, b) = (987_655, 987_656);
        let first = begin_moving(&[a, b, a]).unwrap();
        let second = begin_moving(&[a]).unwrap();
        assert!(is_moving(a) && is_moving(b));
        assert!(
            begin_deleting(a).is_err(),
            "有文件正在移进 / 移出时不能删除"
        );
        assert!(begin_deleting(b).is_err());
        drop(first);
        assert!(!is_moving(b), "同一次移动里重复的 id 只计一次");
        assert!(is_moving(a), "另一个移动还没结束");
        assert!(begin_deleting(a).is_err());
        drop(second);
        assert!(!is_moving(a));
        let deleting = begin_deleting(a).unwrap();
        assert!(
            begin_moving(&[b, a]).is_err(),
            "目标或来源正在删除时整次移动都不开始"
        );
        assert!(!is_moving(b), "没开始的移动不留登记");
        drop(deleting);
    }

    #[test]
    fn reconcile_skips_a_fence_that_is_being_deleted() {
        let id = 987_657;
        let db = db_with_fences(&[id]);
        let guard = begin_deleting(id).unwrap();
        assert_eq!(reconcile_fence(&db, id), Ok(None));
        assert!(rows(&db, id).is_empty(), "删除中不补行、不重建文件夹");
        drop(guard);
    }
}
