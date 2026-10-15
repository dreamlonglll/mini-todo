# 桌面收纳箱（Desktop Fences）

> 收纳箱的 code-spec：`pc/src-tauri/src/commands/fence.rs`（元数据、窗口生命周期、按屏幕组合记忆位置）、
> `commands/fence_items.rs`（内容对账、移入 / 移回 / 跨箱、打开、图标、目录监听）、`services/fence_fs/`（文件层，
> 与数据库无关）、`commands/window.rs` 的「始终嵌入桌面的附属窗口」一节，以及前端 `views/FenceView.vue`、
> `components/FenceItemGrid.vue`、`types/fence.ts`、`utils/fence.ts`。
> 建立于任务 `10-15-desktop-fence`（2026-10-15），在 Win11 10.0.26300、双屏混合 DPI（150% + 125%）上 e2e 实测。
> 主窗口的三种模式（普通 / 固定 / 固定 + 嵌入桌面）见 [window-modes.md](./window-modes.md)，本文件只讲收纳箱。
> 文件层逐函数说明见任务文档 `10-15-desktop-fence/fence-fs-api.md`（完成后归档到 `.trellis/tasks/archive/2026-10/`）。
>
> GPUI 版（`pc-gpui/`）：v29 三张表、文件层（`services/fence_fs/` 原样复制）、对账与 `FenceActivity`、删除保留语义、物理像素几何与
> 落库时机、屏幕组合布局、debug 隔离变量**全部同样适用**，数据与旧版互通。与窗口无关的部分在 `pc-gpui/crates/core/src/fence/`
> （`mod.rs` = `fence.rs`，`items.rs` = `fence_items.rs`；`fence-items-changed` 事件改为 `FenceRefresh` 返回值 / 监听回调）；
> 窗口、嵌入与 200ms 守护在 `pc-gpui/crates/app/src/services/fences.rs` + `windows/fence/` + `platform/embed.rs`。建窗标志按 GPUI 实现
> （`resizable(false)` / 不可最小化 / `show: false` → 定位 → 嵌入 → `SW_SHOWNOACTIVATE`），`asset` scope 不再适用（GPUI 用 `img(PathBuf)`
> 直接读图标缓存）。带 `owner_hwnd` 的移动必须在后台线程调用（[pc-gpui/gpui-conventions.md §2.1](../pc-gpui/gpui-conventions.md)）。

---

## 场景：桌面上的真收纳区域（窗口 + 文件 + 屏幕组合布局）

### 1. Scope / Trigger

- 触发：新增 19 个 Tauri 命令与 1 个窗口定向事件、迁移 v29（3 张表 + 1 个 settings 键）、一种新窗口
  （始终嵌入桌面层的附属窗口）、Win32 排序约束、会移动用户文件的 Shell 操作（硬约束：**不丢文件、绝不覆盖**）、
  3 个 debug 专用环境变量、`assetProtocol` scope 变更。
- 改动以下任何东西前先读本文件：
  - `commands/fence.rs`、`commands/fence_items.rs`、`services/fence_fs/**`
  - `commands/window.rs` 里的 `EMBEDDED_WINDOWS` / `embed_window_in_desktop` / `reassert_embedded_window` /
    `release_embedded_window` / `tick_embedded_windows` / `set_window_bounds_physical` / `desktop_attach_hwnd` /
    `lacks_tool_window_style`，以及 `delete_screen_config`
  - `lib.rs` 的托盘收纳箱菜单项、`restore_fences`、200ms 轮询线程、`MINI_TODO_DATA_DIR` 相关的插件注册与自启自愈
  - `db/paths.rs` 的隔离变量、`fences` / `fence_items` / `fence_layouts` 表、`tauri.conf.json` 的 `assetProtocol.scope`
  - 前端 `FenceView.vue` / `FenceItemGrid.vue` / `types/fence.ts` / `utils/fence.ts`
- 平台：托盘入口与启动恢复只在 `cfg(target_os = "windows")`；命令在所有平台注册，但移动 / 打开 / 定位 / 图标在
  非 Windows 返回 `Err("桌面收纳箱仅支持 Windows")`。

### 2. Signatures

```rust
// ===== commands/fence.rs =====
pub const FENCE_LABEL_PREFIX: &str = "fence-";
pub fn fence_label(id: i64) -> String;                          // "fence-{id}"
pub(crate) fn fence_window(app: &AppHandle, id: i64) -> Option<WebviewWindow>;
pub(crate) fn current_screen_config_id(app: &AppHandle) -> String;
pub fn spawn_create_fence(app: &AppHandle);                      // 托盘「新增收纳箱」：转 async_runtime::spawn 建窗
pub fn restore_fences(app: &AppHandle);                          // setup（仅 Windows），异步建窗、不阻塞 setup
pub fn toggle_all_fences(app: &AppHandle) -> bool;               // 托盘「显示/隐藏所有收纳箱」，返回切换后是否可见
pub fn reset_all_fence_positions(app: &AppHandle) -> Result<(), String>; // 托盘「重置收纳箱位置」
pub fn tick_fence_screen_layouts(app: &AppHandle);               // 200ms 轮询线程每 10 个 tick 调一次
// 纯函数（有单测）
fn screen_config_id_of(monitors: &[(u32, u32, f64)]) -> String; // (物理宽, 物理高, 缩放) → 屏幕组合 ID
fn plan_delete(restored: &fence_fs::RestoreAllResult) -> DeletePlan;
fn view_rect(stored: PxRect, expanded: bool, title_height: i32, works: &[PxRect]) -> PxRect;
fn committed_rect(window: PxRect, expanded_view: bool, current: PxRect) -> PxRect;
fn layout_after_switch(record: Option<PxRect>, window: PxRect, expanded_height: i32, collapsed_view: bool) -> PxRect;
fn effective_rect(fence: &Fence, layouts: &HashMap<i64, PxRect>, session: &BTreeMap<i64, PxRect>) -> PxRect;
fn rename_fence_and_folder(db: &Database, id: i64, title: &str,
    rename_folder: impl FnOnce(&str, &str, &[String]) -> Result<String, String>) -> Result<RenameFenceResult, String>;

#[tauri::command] pub fn list_fences(db: State<Database>) -> Result<Vec<Fence>, String>;      // ORDER BY sort_order, id
#[tauri::command] pub fn get_fence(db: State<Database>, id: i64) -> Result<Fence, String>;
#[tauri::command] pub async fn create_fence(app: AppHandle) -> Result<Fence, String>;        // 与托盘同一实现
#[tauri::command] pub async fn rename_fence(app: AppHandle, id: i64, title: String) -> Result<RenameFenceResult, String>;
#[tauri::command] pub fn update_fence(db: State<Database>, id: i64, patch: UpdateFenceRequest) -> Result<Fence, String>;
#[tauri::command] pub fn set_fence_view(app: AppHandle, db: State<Database>, id: i64, expanded: bool) -> Result<(), String>;
#[tauri::command] pub fn set_fence_bounds(app: AppHandle, id: i64, x: i32, y: i32, width: u32, height: u32) -> Result<(), String>;
#[tauri::command] pub fn commit_fence_geometry(app: AppHandle, db: State<Database>, id: i64, expanded_view: bool) -> Result<Fence, String>;
#[tauri::command] pub async fn delete_fence(app: AppHandle, id: i64) -> Result<DeleteFenceResult, String>;
#[tauri::command] pub fn toggle_fences_visibility(app: AppHandle) -> Result<bool, String>;    // 与托盘同一实现
#[tauri::command] pub fn reset_fence_positions(app: AppHandle) -> Result<(), String>;         // 与托盘同一实现

// ===== commands/fence_items.rs =====
pub const FENCE_ITEMS_CHANGED_EVENT: &str = "fence-items-changed";
pub const ONEDRIVE_NOTICE_KEY: &str = "fence_onedrive_notice_shown";
#[derive(Default)] pub struct FenceWatcherState(Mutex<Option<FenceWatcher>>);   // lib.rs setup 里 app.manage
pub(crate) fn ensure_fence_watcher(app: &AppHandle);             // 已在监听则 no-op
pub(crate) fn reconcile_fence(db: &Database, fence_id: i64) -> Result<Option<Vec<FenceEntry>>, String>;
pub(crate) fn reconcile_and_notify(app: &AppHandle, db: &Database, fence_id: i64);
pub(crate) fn spawn_startup_reconcile(app: &AppHandle);          // 线程 fence-startup
pub(crate) fn begin_deleting(id: i64) -> Result<DeletingGuard, String>;   // drop 即注销
pub(crate) fn begin_moving(ids: &[i64]) -> Result<MovingGuard, String>;   // drop 即注销
pub(crate) fn is_deleting(id: i64) -> bool;
pub(crate) fn is_moving(id: i64) -> bool;
pub(crate) fn owner_hwnd(window: &WebviewWindow) -> Option<isize>;        // 非 Windows 恒 None
pub(crate) async fn blocking<R, F>(f: F) -> Result<R, String>;            // spawn_blocking 包装
pub(crate) fn fence_originals(conn: &Connection, fence_id: i64) -> rusqlite::Result<HashMap<String, String>>;
pub(crate) fn delete_item(conn: &Connection, fence_id: i64, file_name: &str) -> rusqlite::Result<()>;

#[tauri::command] pub async fn get_fence_items(app: AppHandle, id: i64) -> Result<Vec<FenceEntry>, String>;
#[tauri::command] pub async fn fence_move_in(app: AppHandle, id: i64, paths: Vec<String>) -> Result<FenceMoveInResult, String>;
#[tauri::command] pub async fn fence_restore_item(app: AppHandle, id: i64, file_name: String) -> Result<ItemOpResult, String>;
#[tauri::command] pub async fn fence_move_item(app: AppHandle, id: i64, file_name: String, target_id: i64) -> Result<ItemOpResult, String>;
#[tauri::command] pub async fn fence_open_item(app: AppHandle, id: i64, file_name: String) -> Result<(), String>;
#[tauri::command] pub async fn fence_reveal_item(app: AppHandle, id: i64, file_name: String) -> Result<(), String>;
#[tauri::command] pub async fn fence_item_icon(app: AppHandle, id: i64, file_name: String, size_px: u32) -> Result<String, String>;
#[tauri::command] pub fn reorder_fence_items(db: State<Database>, id: i64, file_names: Vec<String>) -> Result<(), String>;

// ===== commands/window.rs — 始终嵌入桌面的附属窗口 =====
struct EmbeddedWindowState { hwnd: isize, owner: isize }             // owner = 最近一次挂上的 Progman，0 = 未挂
static EMBEDDED_WINDOWS: Mutex<BTreeMap<String /* label */, EmbeddedWindowState>>;
pub fn embed_window_in_desktop(window: &WebviewWindow);     // 登记句柄 + 排队挂载（幂等）
pub fn reassert_embedded_window(window: &WebviewWindow);    // run_on_main_thread 排队重挂；闭包里再查登记与 IsWindow
pub fn release_embedded_window(label: &str);                // WindowEvent::Destroyed 时注销；未登记 no-op
pub fn tick_embedded_windows(app: &tauri::AppHandle);       // 200ms 轮询，每个 tick 都调
pub fn set_window_bounds_physical(window: &WebviewWindow, x: i32, y: i32, width: u32, height: u32) -> Result<(), String>;
fn desktop_attach_hwnd(hwnd: HWND) -> Option<isize>;        // 主窗口与收纳箱共用的 Win32 核心
fn lacks_tool_window_style(ex_style: u32) -> bool;          // 纯函数：无 TOOLWINDOW 或有 APPWINDOW
fn delete_screen_config_rows(conn: &Connection, config_id: &str) -> rusqlite::Result<()>; // 连带删 fence_layouts

// ===== db =====
pub const FENCE_COLUMNS: &str;  pub fn fence_from_row(row: &Row) -> rusqlite::Result<Fence>;   // db/models.rs
pub const DEFAULT_FENCE_BG_ALPHA: f64 = 0.45;
pub const DEV_DATA_DIR_ENV: &str = "MINI_TODO_DATA_DIR";   // db/paths.rs
pub fn dev_data_dir_override() -> Option<PathBuf>;          // release 恒 None（不读环境变量）
fn data_dir_override(raw: Option<OsString>, debug_build: bool) -> Option<PathBuf>; // 纯函数

// ===== services::fence_fs（mod.rs 重新导出的部分；全部阻塞，Result 错误为中文） =====
pub fn fence_root_dir() -> Result<PathBuf, String>;          // 不存在则创建
pub fn icon_cache_dir() -> PathBuf;                          // <app_data_dir>\fence-icons，不创建
pub fn user_desktop_dir() -> Option<PathBuf>;                // dirs::desktop_dir()
pub fn path_starts_with_ci(path: &Path, prefix: &Path) -> bool;
pub fn same_path_ci(a: &Path, b: &Path) -> bool;
pub fn is_valid_item_name(name: &str) -> bool;               // 单个路径段：拦 ..、绝对路径、ADS
pub fn create_fence_folder(title: &str, taken: &[String]) -> Result<String, String>;
pub fn ensure_fence_folder(folder_name: &str) -> Result<PathBuf, String>;
pub fn rename_fence_folder(old_folder: &str, new_title: &str, taken: &[String]) -> Result<String, String>;
pub fn remove_fence_folder_if_empty(folder_name: &str) -> Result<bool, String>;
pub fn fence_folder_path(folder_name: &str) -> Result<PathBuf, String>;
pub fn list_fence_items(folder_name: &str) -> Result<Vec<FenceEntry>, String>;
pub fn list_fence_folders() -> Result<Vec<String>, String>;
pub fn move_into_fence<P: AsRef<Path>>(sources: &[P], folder_name: &str, owner_hwnd: Option<isize>) -> Result<Vec<ItemOpResult>, String>;
pub fn restore_item(folder_name: &str, file_name: &str, original_path: Option<&str>, owner_hwnd: Option<isize>) -> Result<ItemOpResult, String>;
pub fn move_between_fences(from_folder: &str, file_name: &str, to_folder: &str, owner_hwnd: Option<isize>) -> Result<ItemOpResult, String>;
pub fn restore_all(folder_name: &str, originals: &HashMap<String, String>, owner_hwnd: Option<isize>) -> Result<RestoreAllResult, String>;
pub fn desktop_is_onedrive_redirected() -> bool;
pub fn launch_item(path: &str) -> Result<(), String>;
pub fn reveal_in_explorer(path: &str) -> Result<(), String>;
pub fn icon_png(path: &str, size_px: u32) -> Result<PathBuf, String>;
pub fn prune_icon_cache<P: AsRef<Path>>(live_paths: &[P]) -> Result<usize, String>;
pub fn watch_fence_root(on_change: impl Fn(FenceChange) + Send + 'static) -> Result<FenceWatcher, String>;
```

```ts
// pc/src/types/fence.ts
interface Fence { id; title; folderName; x; y; width; height; collapsed; locked; bgColor; bgAlpha; iconSize; sortOrder; createdAt; updatedAt }
interface UpdateFenceRequest { collapsed?; locked?; bgColor?; bgAlpha?; iconSize? }
interface FenceEntry { fileName; displayName; path; isDir; isShortcut; modifiedMs: number | null }
type FenceItemOpStatus = 'moved' | 'cancelled' | 'failed' | 'skipped'
interface FenceItemOpResult { source; status; fileName: string | null; destPath: string | null; error: string | null }
interface FenceMoveInResult { results: FenceItemOpResult[]; onedriveNotice: boolean }
interface RenameFenceResult { fence: Fence; folderError: string | null }
interface DeleteFenceResult { deleted: boolean; restored: number; remaining: string[]; failures: FenceItemOpResult[] }
FENCE_TITLE_BAR_HEIGHT = 32  FENCE_MIN_WIDTH = 160  FENCE_MIN_EXPANDED_HEIGHT = 160  FENCE_TITLE_MAX_LENGTH = 40
FENCE_ICON_SIZE_MIN = 24  FENCE_ICON_SIZE_MAX = 128  FENCE_ICON_SIZE_STEP = 8
FENCE_ICON_BUCKETS = [16, 24, 32, 48, 64, 96, 128, 256]  FENCE_ITEMS_CHANGED_EVENT = 'fence-items-changed'

// pc/src/utils/fence.ts（纯函数，vitest 覆盖）
parseFenceId(value) / normalizeFenceTitle(raw) / exceedsDragThreshold(dx, dy, threshold = 3)
class GeometryCommitGate { arm(now); accept(now): boolean; isArmed(now); disarm() }
resizeRect(start, direction, dx, dy, minWidth, minHeight) / iconBucket / iconRequestPx / clampIconSize / nextIconSize
summarizeOpResults / describeMoveIn / describeDeleteKept / baseName / parentDir
```

前后端常量必须成对修改：`FENCE_TITLE_BAR_DIP` ↔ `FENCE_TITLE_BAR_HEIGHT`、`MIN_WIDTH_DIP` / `MIN_EXPANDED_HEIGHT_DIP` ↔
`FENCE_MIN_*`、`MAX_TITLE_CHARS` ↔ `FENCE_TITLE_MAX_LENGTH`、`MIN_ICON_SIZE` / `MAX_ICON_SIZE` ↔ `FENCE_ICON_SIZE_MIN/MAX`、
`ICON_SIZE_BUCKETS` ↔ `FENCE_ICON_BUCKETS`、`normalize_title` ↔ `normalizeFenceTitle`。

### 3. Contracts

#### 3.1 收纳箱窗口

**标识**：label `fence-{id}`（`id = fences.id`，`is_fence_label` 只认 `fence-` 后接纯数字）；URL
`index.html#/fence?id={id}` → 路由 `/fence` → `FenceView.vue`（`parseFenceId` 只接受正整数）。capabilities 的
`windows: ["*"]` 已覆盖收纳箱窗口，不需要新增权限。

**建窗**（`open_fence_window`，已有同 label 窗口时直接返回它）：

1. 必须在 async 命令或 `tauri::async_runtime::spawn` 里建窗：Windows 上同步命令 / 托盘菜单回调里建窗会死锁。
   托盘「新增收纳箱」走 `spawn_create_fence`。
2. tao 标志**一次给齐，之后不再改**：

   | Builder 参数 | 值 | 原因 |
   |---|---|---|
   | `position` / `inner_size` | 物理像素 ÷ 目标显示器缩放 | 逻辑坐标；建好后立即按物理像素 `set_position` → `set_size` 校正一次（混合 DPI 下差几个像素） |
   | `min_inner_size` | `(160, 展开 ? 160 : 32)` 逻辑像素 | 折叠 / 展开时再调整 |
   | `decorations(false)` `transparent(true)` `shadow(false)` | | 无边框透明，圆角由 `setup_window_rounded_corners`（DWM）提供 |
   | `skip_taskbar(true)` `visible(true)` `focused(false)` | | `skip_taskbar` 在 `set_visible` 之前生效，建窗不闪任务栏 |
   | `always_on_bottom(true)` | | 之后任何 `SetWindowPos` 都被 tao 钩子压回 `HWND_BOTTOM` |
   | `minimizable(false)` | | Win+M 不最小化它 |
   | `maximizable(false)` | | 任何途径的最大化都会冲掉嵌入样式 |
   | `resizable(false)` | | 无 `WS_THICKFRAME`：锁定 / 折叠时系统不能缩放、不参与 Aero Snap；缩放走 `set_fence_bounds` |
   | **不设** `focusable(false)` | | 窗口必须可激活（见下） |

3. 事件：`CloseRequested` → `api.prevent_close()`（没有关闭按钮，聚焦时 Alt+F4 也关不掉）；
   `Destroyed` → `release_embedded_window(label)`。删除收纳箱走 `window.destroy()`，不经过 `CloseRequested`。
   事件闭包只持有 label，不持有窗口（否则成环）。
4. 然后 `embed_window_in_desktop(&window)`；如果此时处于"全部隐藏"（`FENCES_HIDDEN`），`hide()` 后紧跟 `reassert_embedded_window`。

**嵌入后的 Win32 状态**（`fence.ps1 -Step json` 必须一直是这样，任何操作之后都不变）：

| 字段 | 值 |
|---|---|
| owner（`GWLP_HWNDPARENT`） | Progman（`GetShellWindow()`） |
| `WS_EX_TOOLWINDOW` / `WS_EX_APPWINDOW` | 有 / 无（不进任务栏、不参与 Alt+Tab） |
| `WS_MINIMIZEBOX` / `WS_THICKFRAME` | 无 / 无 |
| `WS_EX_TOPMOST` / `WS_EX_NOACTIVATE` | 无 / 无 |
| Z 序 | Progman owned 组内；挂载时 `HWND_BOTTOM` 落到组内最底，点击激活升到组内最上，始终在所有普通应用窗口之下 |

**登记与轮询**：

- `embed_window_in_desktop` 把 `label → {hwnd, owner: 0}` 写进 `EMBEDDED_WINDOWS`（句柄登记时取一次），再排队挂载。
  挂载闭包（`attach_embedded_window`）执行时重新查登记表并 `IsWindow`：排队期间窗口可能已被删除。
  挂上后把 Progman 句柄写回 `owner`。
- `EMBEDDED_WINDOWS` 的锁内只做只读 Win32 调用，**绝不**在持锁时调 `run_on_main_thread`。
- 200ms 轮询线程（`lib.rs`）每个 tick 都调 `tick_embedded_windows`（与主窗口当前模式无关）：每 tick 一次
  `GetShellWindow()`，每个窗口读 `IsWindow` / owner / `GWL_EXSTYLE`；`窗口存活 && (needs_reattach(owner, owner_alive,
  shell_now, current_owner) || (shell_now != 0 && lacks_tool_window_style(ex)))` 的先在锁内收集 label，释放锁后
  `reassert_embedded_window`。`needs_reattach` 与主窗口同一个纯函数。
- 同一线程每 10 个 tick（约 2 秒，`FENCE_SCREEN_CHECK_TICKS`）调一次 `tick_fence_screen_layouts`（见 3.5）。

**之后允许的窗口操作**：

| 操作 | 是否触发 tao `apply_diff` | 规则 |
|---|---|---|
| `set_position` / `set_size` / `set_min_size` | 否 | 一律**先位置后尺寸**（跨 DPI 时位置触发 `WM_DPICHANGED` 缩放，随后的物理尺寸再定准） |
| `set_title`（改名后同步窗口标题） | 否 | `SetWindowTextW` |
| `set_window_bounds_physical`（`set_fence_bounds`） | 否 | 一次 `SetWindowPos(SWP_NOZORDER \| SWP_NOACTIVATE)`；拖左 / 上边时位置与尺寸同时变，分两步会抖 |
| `show()` / `hide()` | **是**（重写 ex style） | 只用 tao 的 `show()` / `hide()`（tao 不同步 Win32 `ShowWindow` 的可见态），**之后必须紧跟 `reassert_embedded_window`** |
| `set_resizable` / `set_minimizable` / `set_always_on_*` / `set_maximizable` | 是 | **禁止**。锁定 / 解锁、折叠 / 展开都不碰 tao flag |

**锁定（「固定」）完全由前端控制**：锁定时不调 `startDragging`、不显示缩放手柄；仍可拖入、折叠、改名、打开项目。
Win32 样式不随锁定变化。

**移动**：标题栏 `mousedown` 只记下按下点；`mousemove` 时左键仍按着且位移超过 3 CSS px（`exceedsDragThreshold`）
才 `geometryGate.arm(now)` + `getCurrentWindow().startDragging()`。不在 `mousedown` 里直接 `startDragging`（会吞掉
`mouseup`，双击改名收不到）；标题栏**不用 `data-tauri-drag-region`**（双击会最大化、冲掉嵌入样式）。

**缩放**：`resizable(false)` 的窗口上 `startResizeDragging`（`WM_NCLBUTTONDOWN(HT*)`）无效，系统只对带 `WS_THICKFRAME`
的窗口进入缩放循环。8 个缩放手柄（n/s/w/e/nw/ne/sw/se）只在 `!locked && !collapsed && !editingTitle` 时显示：
`pointerdown` → `setPointerCapture` → 取窗口外框矩形（物理像素）；`pointermove` → `resizeRect`（拖哪条边改哪条边，对边不动，
不小于 `160 × dpr`）→ 每帧最多一次 `set_fence_bounds`（串行，只保留最新一帧）；`pointerup` → 等最后一帧落地 →
`commit_fence_geometry`。

**几何落库（只有用户发起的才落库）**：

| 时机 | 写什么 |
|---|---|
| 新建收纳箱 | `INSERT fences`（x/y/width/height = 默认位置）+ `save_layout(当前屏幕组合)` |
| 用户拖动结束 | `GeometryCommitGate`：`startDragging` 时 `arm`，之后的 `onMoved` / `onResized` 在闸门打开期间才算数（每次续期 3000ms），停下 300ms 后 `commit_fence_geometry({ id, expandedView })` |
| 用户缩放结束 | `pointerup` 后 `commit_fence_geometry` |
| 托盘「重置收纳箱位置」 | 后端事务内批量写 `fences` + 当前组合的 `fence_layouts` |

`commit_fence_geometry` 读窗口当前外框，按 `committed_rect` 计算：`expandedView = false`（只剩标题栏）时只落位置和宽度，
展开高度保持库里的值；写 `fences.x/y/width/height`（"最近一次位置"）与当前组合的 `fence_layouts`（同一事务），
并移除该收纳箱的 `SESSION_RECTS`。**不落库**：启动夹回、显示器变化导致的系统挪窗、`set_fence_view`、屏幕组合切换、
`set_fence_bounds` 本身。前端做任何程序调整前先 `flushGeometry()`（有待落库的拖动立即落库，然后 `disarm` 闸门）。

**折叠 / 展开**：

- `update_fence({ collapsed })` 只写库；窗口形态由 `set_fence_view({ id, expanded })` 呈现（不写库）。
- 临时展开不改持久化的折叠态。原因集合（任一存在即展开）：`hover`（进入 150ms 后展开、离开 400ms 后收回，拖动闸门
  打开期间不展开也不收回）、`drag`（`onDragDropEvent` 的 enter/over；leave/drop 后 300ms 收回）、`menu`（标题栏菜单）、
  `panel`（外观 / 删除确认 / OneDrive 提示）、`itemMenu`（项目右键菜单）。前端把视图变化串行发给后端。
- `view_rect`：折叠 = 标题栏矩形（高度 = `round(32 × 目标显示器缩放)`）；展开 = 库里的展开尺寸，下方空间不够时整体上移
  （向上展开，标题栏原位置仍在窗口内）。两者都按标题栏所在工作区夹回（交叠面积最大的工作区；都不交叠取离中心最近的）。
- 缩放比例按**目标位置**所在显示器取（`title_scale`），不取窗口当前所在显示器：混合 DPI 下重置位置 / 启动夹回时两者不同。
- 顺序：展开 = 先定位 → 撑开 → 抬高最小高度；折叠 = 先放低最小高度 → 定位 → 缩小。

**必须可激活**：不设 `focusable(false)` / `WS_EX_NOACTIVATE`。否则双击打开的程序可能拿不到前台；实测在未激活的收纳箱上
第一次点击折叠按钮即生效。

**显示 / 隐藏全部**：`FENCES_HIDDEN` 只是运行时状态（重启后总是显示）。新建收纳箱、重置位置时如果处于隐藏状态，先全部显示。

**新建位置与标题**：光标所在显示器的工作区（取不到退回主显示器，再取不到按 1920×1080@1.0）；尺寸
`(88×4 + 8×2) × (32 + 96×3 + 8×2)` = 368×336 DIP（150% 下 552×504）；从工作区右上角起、从右往左逐行找不与已有收纳箱
（按当前组合下的有效几何）重叠的位置（边距 24 DIP、步长 32 DIP、间隙半个边距），找不到就从右上角斜向叠放、只避开左上角重合；
标题取 `收纳箱` / `收纳箱 2` / `收纳箱 3` … 中第一个空闲的。

**重置位置**：主显示器工作区，从右上角起从右往左逐行排开（间距 16 DIP），放不下回到右上角错开叠放；尺寸夹到工作区内；
写库后清空 `SESSION_RECTS`，逐个 `apply_fence_view`。

#### 3.2 命令返回结构与事件

- `Fence`（camelCase）：`id, title, folderName, x, y, width, height, collapsed, locked, bgColor, bgAlpha, iconSize,
  sortOrder, createdAt, updatedAt`。几何是物理像素：x/y 为虚拟屏幕坐标（折叠态即标题栏左上角），width/height 为展开态尺寸。
- `UpdateFenceRequest`：字段全可选，只更新给出的；`bgColor` 只接受 `#RRGGBB`（统一大写），`bgAlpha` 夹到 0~1（非有限数按
  0.45），`iconSize` 夹到 24~128。任一字段校验失败什么都不写。标题走 `rename_fence`，几何走 `commit_fence_geometry`。
- `RenameFenceResult { fence, folderError }`：`folderError` 非空 = 标题已改、收纳文件夹没跟着改名（保留旧 `folder_name`），
  前端提示"标题已修改，但收纳文件夹没能跟着改名：…"。
- `DeleteFenceResult { deleted, restored, remaining, failures }`（新语义）：
  - `deleted = true`：收纳文件夹里的项目全部移回、空文件夹已删除 → 删 `fences` 行（`fence_items` / `fence_layouts` 级联）、
    移除 `SESSION_RECTS`、`window.destroy()`（`Destroyed` 事件注销嵌入登记）。前端无需再做任何事
  - `deleted = false`：删除**中止**，收纳箱整个保留（记录、窗口、文件夹、布局）；已移回项的 `fence_items` 行删掉，注销"删除中"后
    `reconcile_and_notify`；前端关掉确认框，用 `describeDeleteKept` 提示"N 个项目未能移回桌面，收纳箱已保留：名字（原因）…"（8 秒）
  - `restored` 已移回项数；`remaining` 仍留在文件夹里的文件名（含隐藏文件）；`failures` 为 `failed` / `cancelled` 的逐项结果
  - 返回 `Err`：还原整体无法进行（桌面路径未知、根目录不可用等），什么都没删
- `FenceEntry { fileName, displayName, path, isDir, isShortcut, modifiedMs }`：`displayName` 去掉 `.lnk` / `.url`；
  文件夹（含指向文件夹的链接）`isDir = true`。
- `ItemOpResult { source, status, fileName, destPath, error }`，`status ∈ moved | cancelled | failed | skipped`。
  `moved` 偶尔带 `error` 提示（"已放入目标位置，但源文件未能删除…"、"已移动，但无法确定新的文件名…"）。
- `FenceMoveInResult { results, onedriveNotice }`：`results` 与传入 `paths` 一一对应、顺序相同；`onedriveNotice = true` 时前端
  打开 OneDrive 提示面板（只会有一次）。
- `fence_item_icon` 返回缓存 PNG 的绝对路径，前端 `convertFileSrc(path)` 加载；失败时前端显示占位图标。
- **事件 `fence-items-changed`**：`app.emit_to(fence_label(id), FENCE_ITEMS_CHANGED_EVENT, Vec<FenceEntry>)`，只投递给对应
  收纳箱窗口（窗口不存在时不投递、无副作用）；payload 是对账后按 `sort_order` 排好的完整列表。触发点：目录监听回调、
  跨箱移动后的来源 / 目标收纳箱、删除中止后。前端用 `getCurrentWindow().listen(...)` 接收，与 `get_fence_items` 共用一个序号，
  晚到的旧列表丢弃；箱内拖动排序期间不跟随推送，松手后补一次列举。

#### 3.3 数据库 v29

```sql
CREATE TABLE fences (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    title TEXT NOT NULL,
    folder_name TEXT NOT NULL UNIQUE,                 -- 收纳根目录下的子文件夹名
    x INTEGER NOT NULL, y INTEGER NOT NULL,           -- 物理像素，虚拟屏幕坐标（"最近一次位置"）
    width INTEGER NOT NULL, height INTEGER NOT NULL,  -- 物理像素，展开态尺寸
    collapsed INTEGER NOT NULL DEFAULT 0, locked INTEGER NOT NULL DEFAULT 0,
    bg_color TEXT NOT NULL DEFAULT '#000000', bg_alpha REAL NOT NULL DEFAULT 0.45,
    icon_size INTEGER NOT NULL DEFAULT 48,            -- 逻辑像素
    sort_order INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL, updated_at TEXT NOT NULL -- datetime('now', 'localtime')
);
CREATE TABLE fence_items (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    fence_id INTEGER NOT NULL REFERENCES fences(id) ON DELETE CASCADE,
    file_name TEXT NOT NULL,                          -- 收纳文件夹内的实际文件名
    original_path TEXT,                               -- 移入前的完整路径；Explorer 直接放入的为 NULL
    sort_order INTEGER NOT NULL DEFAULT 0,
    UNIQUE (fence_id, file_name)
);
CREATE TABLE fence_layouts (
    fence_id INTEGER NOT NULL REFERENCES fences(id) ON DELETE CASCADE,
    screen_config_id TEXT NOT NULL,                   -- 与 screen_configs.config_id 同一格式
    x INTEGER NOT NULL, y INTEGER NOT NULL, width INTEGER NOT NULL, height INTEGER NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (fence_id, screen_config_id)
);
INSERT OR IGNORE INTO settings (key, value, updated_at)
    VALUES ('fence_onedrive_notice_shown', 'false', datetime('now', 'localtime'));
```

- `LATEST_VERSION = 29`，迁移表驱动追加 `(29, migration_v29)`。
- 级联删除依赖连接上 `PRAGMA foreign_keys = 1`（`migrate` 完成后开启，有单测）。删收纳箱只删 `fences` 行。
- `delete_screen_config(config_id)` 在一个事务里删 `screen_configs` 行**和** `fence_layouts WHERE screen_config_id = ?`。
- `fence_items` 只是排序与原始路径的附加信息，内容以文件夹为准（3.4）。
- 三张表都是**设备本地数据**：不接 v28 的 `local_seq` 变更计数触发器与墓碑，不进 `ExportData` / `SyncData`
  （见 [sync-protocol.md §3.6](./sync-protocol.md)）。
- `fence_onedrive_notice_shown`：经 `settings_kv::get_bool_setting` / `set_bool_setting` 读写，`claim_onedrive_notice`
  只会返回一次 `true`。它**有意**不进 `AppSettings`、不进 `SYNCED_SETTING_KEYS`、不被 `read_app_settings` /
  `write_app_settings` 读写——这是对 `CLAUDE.md`「维护检查清单」第 1 条的**有意偏离**。原因：桌面是否被 OneDrive 重定向、
  这台设备是否已提示过，都是设备自身的事；同步或随备份导入会让另一台设备从未显示过的提示被跳过，或让已提示过的设备再提示一次。
  性质同 `webdav_*` 设备键。

#### 3.4 文件层规则

- **根目录**：`%USERPROFILE%\MiniTodo收纳箱\`（`dirs::home_dir()` + `FENCE_ROOT_DIR_NAME`）。按需创建：只有出现第一个收纳箱
  （新建 / 启动恢复）后才开始监听和列举，没有收纳箱的用户不会多出这个文件夹。
- **文件夹名**：`sanitize_folder_name(title)`（去首尾空白、控制字符与 `<>:"/\|?*` 换 `_`、去尾部 `.` 与空格、最多 48 字符、
  保留设备名加 `_`、空则 `收纳箱`），再与数据库里**全部** `folder_name`（即使文件夹已被用户删掉，防 UNIQUE 冲突）和磁盘上
  已有项不区分大小写去重（` (2)` 后缀）。新建：**先建文件夹再插库**；插库失败 → `remove_fence_folder_if_empty` 回滚；
  建窗失败 → 删记录 + 回滚文件夹。改名：整个过程持有数据库锁，文件夹改名失败则保留旧 `folder_name`。
- **文件夹是 source of truth**：每次列举都对账（`reconcile_rows`）：名字先精确匹配、再不区分大小写匹配（资源管理器里只改大小写
  时保留原始路径与排序，并把行里的名字改成新写法）；表里有、文件夹没有 → 删行；文件夹有、表里没有 → 补行
  （`original_path = NULL`，按文件名顺序排到末尾）；返回按 `sort_order` 排序。
- **对账在数据库锁内列举**：`reconcile_fence` 在一个 `with_transaction` 里依次检查"删除中"、读 `folder_name`、
  `list_fence_items`（一次本地 `read_dir`）、写行。文件夹不存在时 `ensure_fence_folder` 按原名重建后再列举（显示为空）。
- **移入 / 跨箱写库一律 upsert**：`INSERT … ON CONFLICT(fence_id, file_name) DO UPDATE SET original_path = excluded.original_path`
  （新行排末尾；已有行保留排序）。监听回调触发的对账可能已经先补了一行 `NULL`。
- **跨箱拖入**：源路径形如 `<根目录>\<另一个收纳箱的文件夹>\<项>` 时按跨箱处理：在移动**之前**读旧行的 `original_path`
  （移动一完成监听对账就可能删掉旧行），成功后删旧行、在本箱 upsert 并搬运原始路径，然后对来源收纳箱 `reconcile_and_notify`。
  其他来源：`original_path = 源路径`。
- **记账失败不翻案**：文件已经移动成功而写库失败时只记 `warn` 日志、照常返回逐项成功（下次对账补行，只是丢了原始路径）。
- **`FenceActivity`（进行中的文件操作登记）**：移动 / 删除可能要等用户处理管理员权限或"文件正在使用"对话框，不能一直占着数据库锁，
  靠登记互斥：

  | 操作 | 登记 | 冲突时 |
  |---|---|---|
  | `delete_fence` | `begin_deleting(id)` | 已在删除 → `Err("收纳箱正在删除")`；有移动 → `Err("收纳箱里还有文件正在移动，请稍后再删除")` |
  | `fence_move_in` | `begin_moving(&[id])`（不登记来源收纳箱） | 目标正在删除 → `Err("收纳箱正在删除")` |
  | `fence_restore_item` | `begin_moving(&[id])` | 同上 |
  | `fence_move_item` | `begin_moving(&[id, target_id])` | 任一正在删除 → 整次不开始、不留登记 |
  | `rename_fence`（数据库锁内检查） | — | 删除中 → `Err`；移动中 → 只改标题，`folderError` = "收纳箱里有文件正在移动，文件夹暂未改名…" |
  | `reconcile_fence`（数据库锁内检查） | — | 删除中 → `Ok(None)`，不补行、不重建文件夹 |

  规则：**先登记，再读 `folder_name`**；改名与对账在数据库锁内检查登记，与移动 / 删除读取文件夹名的数据库访问互斥，不会出现
  "已按旧文件夹名开始移动 / 还原，文件夹却被改名"的交错。**锁顺序：数据库锁 → 登记锁**，持有登记锁时绝不拿数据库锁。
- **删除 = `plan_delete`**：`restore_all` 之后 `delete_fence = restored.folder_removed`；`restored` = `moved` 项的文件名
  （取 `source` 的最后一段）；`failures` = `failed` + `cancelled`。**只有收纳文件夹已清空并删除才删收纳箱**；只要有项目留在文件夹里
  （失败、取消、或 `remaining` 里的隐藏文件），整体保留，不产生没有收纳箱管理的孤儿文件夹。
- **线程**：`fence_fs` 全部阻塞，命令里一律经 `blocking()`（`spawn_blocking`）调用。带 owner HWND 的移动（`move_into_fence` /
  `restore_item` / `move_between_fences` / `restore_all`）**绝不在主线程**：系统"需要管理员权限"对话框以收纳箱窗口为父窗口，
  会给主线程同步发消息，主线程等结果就死锁。做法：命令是 async，HWND 在 async 上下文里 `window.hwnd()` 取，移动在阻塞线程池里跑。
  需要 COM STA 的操作在内部线程执行：移动 / 启动 / 定位每次新开 STA 线程（`fence-move` / `fence-launch` / `fence-reveal`），
  图标在常驻 STA 线程 `fence-icon` 上串行。
- **移入校验（`plan_move_in`，逐项）**：

  | 条件 | 结果 |
  |---|---|
  | 非绝对路径 / 磁盘根 / 无法访问 | `failed`（"需要完整路径" / "无法收纳磁盘根目录" / "无法访问: …"） |
  | 同一批重复源路径 | `skipped`（"重复的项目"） |
  | `desktop.ini` / `Thumbs.db` | `skipped` |
  | 隐藏或系统文件 | `skipped`（列举也跳过它们，避免"移走后哪里都看不到"） |
  | 收纳根目录本身或其上级 | `failed`（"不能收纳收纳目录本身或其上级目录"） |
  | **`protected`**：用户桌面、公共桌面本身或其上级 | `failed`（"不能收纳桌面文件夹本身或其上级目录"）——经 Shell 移动已知文件夹会让系统把"桌面"重定向到收纳文件夹里 |
  | 已在本收纳箱 | `skipped`（"已在该收纳箱中"） |
  | 别的收纳箱文件夹本身（父目录 = 根目录） | `failed`（"不能把收纳箱文件夹放进收纳箱"） |
  | 别的收纳箱文件夹里的项 | 允许（跨箱） |

  `protected` 由 `move_into_fence` 传入 `[user_desktop_dir(), public_desktop_dir()]`；桌面**上的**项照常可以收纳。
- **目标名预去重，绝不覆盖**：执行前按目标目录的实际列表（小写）加本批已预订的名字算好不冲突的名字（` (2)` 插在扩展名前，文件夹
  整体加后缀）；`IFileOperation` 标志 `FOF_RENAMEONCOLLISION | FOF_NOCONFIRMMKDIR | FOFX_NOMINIMIZEBOX`，`RENAMEONCOLLISION` 只作兜底。
  不带 `FOF_NOERRORUI`：权限不足弹系统对话框 → UAC，文件被占用弹"文件正在使用"（行为同资源管理器）。结果按 `PostMoveItem` 回调与文件
  系统实际状态归类：`COPYENGINE_E_USER_CANCELLED` / `ERROR_CANCELLED` → `cancelled`（"用户取消了操作"），`COPYENGINE_S_USER_IGNORED` →
  `cancelled`（"已跳过"）。
- **移回目标**：`original_path` 的上级目录是绝对路径、不在收纳根目录内且仍存在 → 原目录；否则（含 `NULL`）→ 用户桌面
  （`dirs::desktop_dir()`）。文件名：收纳时因重名被改成 `X (2).lnk` 的恢复成 `X.lnk`，用户在资源管理器里改过的名字保留；
  目标处已有同名再加后缀。`restore_all` 处理文件夹里的**所有**项（含隐藏文件，`desktop.ini` / `Thumbs.db` 除外），所有项在同一个
  `IFileOperation` 里执行。删除确认框里的 N 用 `get_fence_items` 的实际列举数（不含隐藏文件）。
- **监听**：`watch_fence_root`（`notify` 递归监听根目录，只关心第一、二层），最后一个事件后安静 300ms 回调一次、持续变化时最多 1.5s
  必回调；回调在线程 `fence-watch` 上 → 按文件夹名（不区分大小写）匹配收纳箱（`rescan` 时全部）→ `reconcile_and_notify`。
  `FenceWatcher` 放在托管状态 `FenceWatcherState` 里一直持有；`ensure_fence_watcher` 在启动恢复（有收纳箱时）与新建后调用。
  本应用自己的移动同样触发回调，不区分来源。
- **启动**：`restore_fences` → 逐个 `ensure_fence_folder`（用户可能删了文件夹）→ `open_fence_window` → 有收纳箱则
  `ensure_fence_watcher` → `spawn_startup_reconcile`（线程 `fence-startup`）：逐个对账、记录根目录下不属于任何收纳箱的文件夹（只记日志）、
  等 20 秒后 `prune_icon_cache(全部现存项目路径)`；任何一个收纳箱对账失败就跳过这次清理。
- **打开**：`launch_item` 用 `ShellExecuteExW`（`SEE_MASK_FLAG_NO_UI | SEE_MASK_NOASYNC`），目标失效的 `.lnk` 约 3 秒后才返回错误，
  前端在此期间显示"正在打开"。`fence_open_item` / `fence_reveal_item` / `fence_item_icon` 先 `is_valid_item_name` 校验文件名
  （防 `..`、绝对路径、ADS 逃逸出收纳文件夹）。
- **图标**：前端请求 `ceil(iconSize × devicePixelRatio)`，后端向上取整到档位 `[16, 24, 32, 48, 64, 96, 128, 256]`；
  `SIIGBF_ICONONLY`（`.lnk` 不带箭头，图片 / 视频显示类型图标）；缓存在 `<app_data_dir>\fence-icons\`，键
  `{路径哈希 FNV-1a 64}-{mtime 毫秒}-{大小}-{档位}.png`，先写临时文件再改名。前端按可见项懒加载（`IntersectionObserver`），
  同时最多 3 个请求。
- **OneDrive 提示**：本次至少有一项从用户桌面移入（`path_starts_with_ci(source, desktop)`）、桌面被 OneDrive 重定向、且
  `claim_onedrive_notice` 返回 true 时，`onedriveNotice = true`。

#### 3.5 屏幕组合布局（PRD R5）

- **ID 格式**与主窗口 `screen_configs.config_id` 完全相同：`{显示器数}_{宽}x{高}@{缩放%}_…`。Rust `screen_config_id_of` 的输入是
  `available_monitors()` 的枚举顺序（物理像素宽高、缩放比例）：缩放% = `round(scale × 100)`；按像素面积**稳定**降序（同面积保持
  枚举顺序，与 JS `Array.prototype.sort` 一致）；无显示器为 `unknown`；读显示器失败也是 `unknown`。必须与前端
  `appStore.generateScreenConfigId()` **逐字一致**——改任何一边都要同步改另一边，并更新单测 `screen_config_id_matches_the_frontend_format`。
- **有效几何**（`effective_rect`）：当前组合的 `fence_layouts` 记录 → 本次运行的临时几何 `SESSION_RECTS` → `fences.x/y/width/height`。
  折叠 / 固定 / 外观 / 图标大小全局共用，不随组合变化。
- **写入时机**：只有 3.1「几何落库」表里的四种（新建、拖动结束、缩放结束、重置位置），每次都同时写当前组合的记录与 `fences` 的最近一次位置。
- **启动恢复**：`Placement::current` 取当前组合 → 每个收纳箱按有效几何建窗，`view_rect` 夹回可见工作区（**不写库**）；有收纳箱时把
  `LAST_SCREEN_ID` 设为当前组合。
- **运行时切换**：`tick_fence_screen_layouts` 约每 2 秒一次；没有收纳箱窗口时不读显示器、`LAST_SCREEN_ID = None`。ID 变化时
  `apply_screen_switch`：新组合有记录 → 用记录；没有 → 沿用窗口当前外框（窗口当前高度 < 2 × 标题栏高度视为只剩标题栏，展开高度
  沿用切换前的有效几何），记进 `SESSION_RECTS`（整体替换，**不写库**）；然后按持久化的折叠态 `apply_fence_view`（夹回可见区域）。
- **删除屏幕配置**：`delete_screen_config` 连带删除该组合的 `fence_layouts`。

#### 3.6 debug 专用开关（release 构建一律不读取）

| 环境变量 | 作用 |
|---|---|
| `MINI_TODO_DATA_DIR` | `db::paths::app_data_dir()` 返回该目录（空值视为未设置，相对路径转绝对）：库、图片、备份、节假日缓存、图标缓存 `fence-icons` 全部落到这里（新库没有 WebDAV 配置，不会同步）。另外：日志写到 `<dir>\logs`；进程启动时（任何 WebView 创建前）设 `WEBVIEW2_USER_DATA_FOLDER=<dir>\EBWebView`（外部已设置则尊重外部值；WebView2 实际落在 `<dir>\EBWebView\EBWebView`），不与安装版共用浏览器进程；**不注册 single-instance 插件**；**跳过开机自启自愈**（不刷新与安装版共用的注册表项）；托盘提示改为"Mini Todo（开发版）"；setup 首条日志警告隔离已启用；`asset_protocol_scope().allow_directory(images_dir, true)` 与 `allow_directory(icon_cache_dir, true)` 放行隔离目录 |
| `MINI_TODO_FENCES_ROOT` | 收纳根目录改为该目录（去空白后非空才生效，不存在由 `fence_root_dir` 创建） |
| `MINI_TODO_SCREEN_ID_FILE` | 文件内容（去首尾空白后非空）替代真实屏幕组合 ID，每次计算时重新读取；e2e 改写它模拟"屏幕组合变化"，走与真实切换相同的轮询 → 恢复路径 |

release 保证：`dev_data_dir_override()` 的 `cfg(not(debug_assertions))` 分支直接 `None`、不读环境变量，纯函数
`data_dir_override(_, false) == None`（单测）；`location::fence_root_override` 的 release 分支恒 `None`（单测
`release_build_ignores_override`，只在 `cargo test --release` 时编译）；`dev_screen_id_override` 整段 `#[cfg(debug_assertions)]`，
release 不编译。

#### 3.7 assetProtocol scope

`tauri.conf.json` → `app.security.assetProtocol.scope` = `["$LOCALDATA/mini-todo/images/**", "$LOCALDATA/mini-todo/fence-icons/**"]`，
本次**只新增** `fence-icons` 一项。图标一律先渲染进缓存再加载，从不直接加载收纳文件夹或桌面上的文件，所以不要把 scope 放宽到 `**`
或用户目录。`capabilities/default.json` 的 `fs:scope` 不变。

### 4. Validation & Error Matrix

| 条件 | 行为 |
|---|---|
| 收纳箱 id 不存在（get / update / rename / commit / delete / 移动命令） | `Err("收纳箱 #{id} 不存在")` |
| 收纳箱窗口不存在（`set_fence_view` / `set_fence_bounds` / `commit_fence_geometry`） | `Err("收纳箱 #{id} 的窗口不存在")` |
| `get_fence_items`：不存在或正在删除 | `Err("收纳箱 #{id} 不存在或正在删除")`（前端后台刷新不弹提示） |
| 标题规范化后为空 / 超过 40 字符 | `Err("收纳箱标题不能为空")` / `Err("收纳箱标题不能超过 40 个字符")`；前端 `normalizeFenceTitle` 同规则，不提交 |
| `bgColor` 不是 `#RRGGBB` | `Err("背景色格式无效：{value}")`，整个 patch 不写 |
| `bgAlpha` 非有限数 / 超范围；`iconSize` 超范围 | 0.45 / 夹到 0~1；夹到 24~128 |
| 新建：收纳文件夹建不出来 | `Err`，不插库、不建窗 |
| 新建：插库失败 / 建窗失败 | `Err("保存收纳箱失败: …")` / `Err("创建收纳箱窗口失败: …")`；回滚文件夹（建窗失败还删记录） |
| 改名：文件夹被占用等 | `Ok`，标题已改，`folderError` 为原因，`folder_name` 不变 |
| 改名：有文件正在移动 / 正在删除 | `Ok` + `folderError`（只改标题）/ `Err("收纳箱正在删除")` |
| 删除：有移动在进行 / 已在删除 | `Err("收纳箱里还有文件正在移动，请稍后再删除")` / `Err("收纳箱正在删除")` |
| 删除：`restore_all` 整体失败（如 `Err("无法确定桌面路径")`） | `Err`，什么都不删 |
| 删除：有项目 `failed` / `cancelled`，或文件夹里仍有剩余（隐藏文件） | `Ok { deleted: false, … }`，收纳箱整个保留 |
| 移动类命令：涉及的收纳箱正在删除 | `Err("收纳箱正在删除")` |
| `fence_move_item` 源 = 目标 | `Err("目标收纳箱与当前收纳箱相同")` |
| `fence_move_in` 的 `paths` 为空 | `Ok` 空结果 |
| 文件名不合法（`..`、绝对路径、`a:b`） | `Err("文件名不合法: …")` |
| 项目不在文件夹里（移回 / 跨箱） | 逐项 `failed`（"收纳文件夹中找不到该项目"） |
| 用户在 UAC / 管理员权限对话框点取消；在错误对话框选"跳过" | 逐项 `cancelled`（"用户取消了操作" / "已跳过"），文件留在原处；前端"已取消 N 项" |
| 文件被占用 | Shell 弹"文件正在使用"，用户选择前调用一直阻塞；取消 / 跳过 → `cancelled`，其他失败 → `failed` + 中文原因 |
| 拖入用户桌面 / 公共桌面文件夹本身或其上级 | 逐项 `failed`（"不能收纳桌面文件夹本身或其上级目录"） |
| 收纳文件夹被用户删掉 | 对账 / 启动时按原名重建（`warn` 日志），显示为空 |
| 文件已移动但写库失败 | `warn` 日志，逐项结果照常返回成功；下次对账补行（无原始路径） |
| 双击目标失效的 `.lnk` | 约 3 秒后 `Err("快捷方式指向的目标不存在，可能已被移动或卸载")` |
| 打开 / 定位不存在的路径 | `Err("文件不存在")` |
| 图标提取失败 | `Err`，前端占位图标 |
| 非 Windows：移动 / 打开 / 定位 / 图标 | `Err("桌面收纳箱仅支持 Windows")`；托盘不显示收纳箱菜单项，`restore_fences` 不调用 |
| `GetShellWindow()` 为空（Explorer 未运行） | `desktop_attach_hwnd` 返回 `None` 不挂；`shell_now == 0` 时轮询不排队，等 Explorer 回来 |
| Explorer 重启（新 Progman） | 下一个 tick 内全部重挂（实测 Explorer 被杀约 1.5s 内），窗口不随旧 owner 销毁 |
| 收纳箱 ex style 被 tao `apply_diff` 改成 APPWINDOW | 下一个 tick `lacks_tool_window_style` 命中，排队重挂 |
| 聚焦收纳箱按 Alt+F4 | `prevent_close`，不关闭 |
| 目录监听启动失败 | `warn` 日志；收纳箱不再随资源管理器里的改动自动刷新（操作后的刷新照常） |
| 读不到显示器信息 | 屏幕组合 ID = `unknown` |
| 切到没有记录的屏幕组合 | 沿用当前位置并夹回可见区域，只记 `SESSION_RECTS`，不写库 |
| 托盘「重置收纳箱位置」时没有显示器 | `Err("找不到可用的显示器")`（`warn` 日志） |

### 5. Good / Base / Bad Cases

- **Good**：托盘「新增收纳箱」→ 光标所在屏右上角出现收纳箱、建出 `收纳箱` 文件夹、写入当前组合的布局；从桌面拖一个 `.lnk`
  进来 → 桌面图标消失、文件在收纳文件夹、`fence_items` 有原始路径；Win+D 两次 → 收纳箱一直可见、第二次后普通应用窗口恢复；
  重启 Explorer → 第一次轮询内重挂；重启应用 → 位置、尺寸、标题、折叠 / 锁定、外观、图标大小、排序全部恢复。
- **Base**：锁定的收纳箱拖不动、没有缩放手柄，但仍可拖入、折叠、改名、打开；改名时文件夹被占用 → 标题照改并提示；
  删除时一个文件被占用、用户选"跳过" → 收纳箱保留、只剩该项、提示原因，释放后再删成功；切到没记录的屏幕组合 → 原位夹回、不写库，
  切回后回到原组合的记录。
- **Bad（不允许发生）**：有项目没移回却删掉了收纳箱记录，留下孤儿文件夹；对账在锁外列举，删掉了刚 upsert 的行；移入后用普通
  `INSERT` 撞 UNIQUE 丢了原始路径；标题栏用了 `data-tauri-drag-region`，双击最大化冲掉嵌入样式；`show()` 后没重挂，任务栏出现按钮；
  显示器变化引起的系统挪窗被落库，覆盖了用户摆好的位置；带 owner 的移动在主线程执行导致死锁；拖入"桌面"文件夹本身导致桌面被重定向
  到收纳文件夹；移回时覆盖了同名文件。

### 6. Tests Required

**单元测试**（`pc/src-tauri` 下 `cargo test`；release 专属断言用 `cargo test --release`）：

| 位置 | 测试 | 断言点 |
|---|---|---|
| `commands/fence.rs` | `screen_config_id_matches_the_frontend_format` | 无显示器 `unknown`；按面积降序且与枚举顺序无关；同面积保持枚举顺序；`1.1 → 110`、`2.25 → 225` |
| | `first_fence_goes_to_the_top_right_of_the_work_area` / `new_fences_avoid_overlapping_existing_ones` / `a_full_work_area_falls_back_to_cascading_without_identical_corners` / `default_rect_respects_offset_monitors_and_tiny_work_areas` | 默认位置在工作区内、不重叠、放满后叠放且左上角不重合 |
| | `clamp_pulls_off_screen_rects_back_and_keeps_their_size` / `work_area_is_picked_by_overlap_then_by_distance` / `views_of_an_off_screen_fence_are_clamped_into_the_nearest_work_area` | 夹回可见区域、尺寸不变、工作区选取规则 |
| | `collapsed_view_is_just_the_title_bar` / `expanding_near_the_bottom_grows_upward_and_keeps_the_title_area_covered` / `title_bar_height_follows_the_target_monitor_scale` | 折叠高度按目标显示器缩放；向上展开仍覆盖标题栏原位置 |
| | `reset_layout_rows_from_the_top_right_and_stays_on_screen` / `default_title_takes_the_first_free_number` / `titles_are_trimmed_and_validated` / `colors_must_be_six_digit_hex` / `fence_labels_round_trip` | 重置排布、默认标题、校验 |
| | `insert_stores_the_folder_and_increasing_sort_order` / `update_only_touches_given_fields_and_validates_them` | patch 只改给出字段；任一字段非法全不写 |
| | `rename_normalizes_the_title_and_moves_the_folder` / `rename_keeps_the_old_folder_when_the_folder_cannot_be_renamed` / `rename_keeps_the_folder_while_files_are_moving_and_is_refused_while_deleting` | `folderError` 语义与登记检查 |
| | `geometry_from_a_collapsed_view_keeps_the_expanded_height` / `saving_geometry_writes_the_current_screen_layout_and_the_latest_position` / `deleting_a_fence_removes_its_layouts` / `effective_geometry_prefers_screen_record_then_session_then_latest` / `switching_screens_uses_the_record_or_keeps_the_current_window_position` | 落库同时写两处；有效几何优先级；切换不写库 |
| | `delete_goes_ahead_only_when_the_folder_was_emptied_and_removed` / `delete_keeps_the_whole_fence_when_anything_is_left_behind` | `delete_fence == folder_removed`；只删已移回项的行；`failures` 含 failed + cancelled |
| `commands/fence_items.rs` | `first_reconcile_adds_rows_in_listing_order` / `reconcile_follows_saved_order_drops_missing_and_appends_new_items` / `case_only_rename_keeps_original_path_and_order` / `exact_match_wins_over_case_insensitive_match` | 对账规则 |
| | `upsert_fills_in_the_original_path_of_a_row_added_by_reconcile` / `originals_and_deletes_are_scoped_to_one_fence` / `reorder_ignores_unknown_names` | upsert 保留排序；按收纳箱隔离；不为未知名字建行 |
| | `onedrive_notice_is_claimed_once_and_stays_device_local` | v29 默认 `'false'`；只返回一次 true；不在 `SYNCED_SETTING_KEYS`、导出的 `AppSettings` JSON 里没有它 |
| | `deleting_guard_blocks_reentry_and_releases_on_drop` / `moving_blocks_deletion_until_every_move_is_done` / `reconcile_skips_a_fence_that_is_being_deleted` | 登记互斥、drop 注销、删除中对账返回 `None` 且不补行 |
| | `affected_fences_match_folder_names_case_insensitively` / `sources_inside_another_fence_folder_are_recognized` | 监听匹配；跨箱来源识别（只认第二层） |
| `commands/window.rs` | `tool_window_style_is_repaired_only_when_wiped` / `deleting_a_screen_config_also_drops_fence_layouts_of_that_combination` / `needs_reattach` 各例 | 自愈判定；删屏幕配置连带删布局、其他组合不受影响 |
| `db/migrations.rs` | `v29_creates_fence_tables_with_contract_defaults` / `v29_deleting_a_fence_cascades_to_its_items_and_layouts` / `v29_fence_writes_do_not_count_as_sync_changes`；`LATEST_VERSION == 29` | 列默认值、UNIQUE、外键；`PRAGMA foreign_keys = 1` 后级联；写收纳箱表 `local_seq` 不变、墓碑为 0 |
| `db/paths.rs` | `release_builds_ignore_the_dev_data_dir_variable` / `debug_builds_use_a_non_empty_dev_data_dir` | release 恒 None；空值不生效；相对路径转绝对 |
| `services/fence_fs` | `plan_move_in_validates_sources` | 预去重 `b (2).lnk` / `b (3).lnk`；`protected` 的桌面文件夹本身 `failed`；根目录及上级、别的收纳箱文件夹本身 `failed`；跨箱项允许 |
| | `restore_dir_selection` / `plan_restore_picks_dir_and_name` / `restore_all_keeps_folder_when_items_remain` / `move_in_results_keep_input_order` / `classify_*` | 移回目录与名字；剩余项时保留文件夹；结果顺序；取消 / 失败归类 |
| | 命名（`sanitize_*`、`folder_name_validation_rejects_traversal`、`item_name_validation`、`unique_file_name_adds_suffix_before_extension`、`restore_name_undoes_collision_suffix`）、图标（`bucket_rounds_up`、`cache_key_is_stable_and_case_insensitive`、`strips_shell_frame_around_small_icons`、`prune_cache`）、监听（`watcher_reports_debounced_folder_changes`）、`release_build_ignores_override`（release） | |

**前端**（`pc` 下 `npm run test`，vitest；`npm run build` 含 vue-tsc）：`pc/src/utils/fence.test.ts`——`parseFenceId` 只接受正整数；
`normalizeFenceTitle` 与后端同规则；`GeometryCommitGate` 未 arm 的移动不算数、拖动中续期、超时 / `disarm` 后失效；`resizeRect` 拖到最小尺寸
不推动对边；`iconBucket` 与后端档位一致；`describeDeleteKept` / `describeMoveIn` 文案。

**E2E**（手动，`pc/scripts/e2e/`，见该目录 README 与 [desktop-window-e2e-guide](../guides/desktop-window-e2e-guide.md)）：开发版必须带
`MINI_TODO_DATA_DIR` + `MINI_TODO_FENCES_ROOT`，只按 exe 路径 / PID 操作开发版。

| 步骤 | 方式 | 断言 |
|---|---|---|
| 新建 | CDP 调 `create_fence` 或托盘 | `fence.ps1 -Step json`：3.1 状态表全部吻合；UIA 轮询任务栏 4s 无按钮 |
| 显示 / 隐藏 ×3 | `toggle_fences_visibility` | 任务栏无按钮；显示后仍 `toolWindow=True` |
| Win+D ×2 / Win+M | `fence.ps1 -Step win-d-key` / `win-m` + 探针 `probe-start` | 收纳箱 `everIconic/everHidden=False`、rect 不变；第二次 Win+D 后探针恢复、原前台窗口拿回焦点 |
| 被普通窗口覆盖 | 探针与收纳箱重叠 | `WindowFromPoint` 命中探针 |
| Explorer 重启（需用户授权） | `fence.ps1 -Step restart-explorer` | 全部存活，第一次轮询内重挂到新 Progman |
| 拖动 / 缩放 / 锁定 | CDP 鼠标事件（用户离开时可用真实输入） | `fences` 与当前组合 `fence_layouts` 同时更新；锁定后位置不变 |
| 外部拖入 | `plugin:event\|emit_to` 给 `fence-{id}` 发 `tauri://drag-drop` | 测试文件离开桌面、进入收纳文件夹，`original_path` 为桌面原路径 |
| 移回 / 重名移回 / 跨箱 | 右键菜单 | 文件回原目录；同名时生成 `X (2)`，原文件内容不变；跨箱搬运原始路径、目标窗口自动刷新 |
| 删除（有项目 / 空） | 标题栏菜单 → 删除 | 确认框写明 N；全部回桌面、文件夹删除、窗口关闭、三张表清空 |
| 屏幕组合切换 | 改写 `MINI_TODO_SCREEN_ID_FILE` | 约 2s 内恢复到记录；没有记录的原位夹回；期间无写库 |
| 删除屏幕配置 | CDP 调 `delete_screen_config` | 该组合的 `fence_layouts` 删除，其他组合不变 |
| 进程重启 | 按 PID 结束 → 启动 exe（vite 单独运行） | 全部状态恢复；主窗口嵌入 + 收纳箱时主窗口约 200ms 内恢复 `toolWindow` |
| 仅能人工验证 | 用户手动 | 公共桌面拖入点 UAC 取消；删除时占用一个文件选"跳过"；从桌面真实 OLE 拖入 |

### 7. Wrong vs Correct

#### Wrong — `show()` / `hide()` 之后不重挂

```rust
for window in fence_windows(app) {
    let _ = window.show(); // apply_diff 整体重写 ex style：WS_EX_TOOLWINDOW 没了，任务栏冒出按钮
}
```

#### Correct — tao 操作之后紧跟排队重挂

```rust
for window in fence_windows(app) {
    let _ = if visible { window.show() } else { window.hide() };
    reassert_embedded_window(&window); // run_on_main_thread 排在 apply_diff 之后：owner + TOOLWINDOW + HWND_BOTTOM
}
```

#### Wrong — 不管还原结果如何都删收纳箱

```rust
let restored = blocking(move || fence_fs::restore_all(&folder, &originals, owner)).await?;
delete_fence_row(&db, id)?; // 有项目移不回去时留下没有收纳箱管理的孤儿文件夹
window.destroy()?;
```

#### Correct — 只有文件夹已清空并删除才删

```rust
let plan = plan_delete(&restored); // delete_fence = restored.folder_removed
if plan.delete_fence {
    delete_fence_row(&db, id)?;    // fence_items / fence_layouts 级联
    window.destroy()?;
} else {
    // 删掉已移回项的行 → drop(deleting) → reconcile_and_notify，返回 deleted = false，收纳箱整个保留
}
```

#### Wrong — 移入后普通 INSERT

```sql
INSERT INTO fence_items (fence_id, file_name, original_path, sort_order) VALUES (?1, ?2, ?3, ?4);
-- 监听对账可能已补了 (fence_id, file_name, NULL)：UNIQUE 冲突，原始路径丢失，移回时只能回桌面
```

#### Correct — upsert，保留对账排好的位置

```sql
INSERT INTO fence_items (fence_id, file_name, original_path, sort_order)
VALUES (?1, ?2, ?3, (SELECT COALESCE(MAX(sort_order), -1) + 1 FROM fence_items WHERE fence_id = ?1))
ON CONFLICT(fence_id, file_name) DO UPDATE SET original_path = excluded.original_path;
```

#### Wrong — 锁外列举、锁内对账

```rust
let entries = fence_fs::list_fence_items(&folder)?;          // 可能早于刚完成的移入
db.with_transaction(|tx| reconcile_rows(tx, id, entries))?;  // 用过期列表删掉刚 upsert 的行
```

#### Correct — 检查登记、读文件夹名、列举、写行都在同一个事务里

```rust
db.with_transaction(|tx| {
    if is_deleting(fence_id) { return Ok(Ok(None)); }   // 锁顺序：数据库锁 → 登记锁
    let folder = /* SELECT folder_name FROM fences WHERE id = ?1 */;
    let entries = fence_fs::list_fence_items(&folder) /* 失败则 ensure_fence_folder 后重试 */;
    Ok(Ok(Some(reconcile_rows(tx, fence_id, entries)?)))
})
```

#### Wrong — 带 owner HWND 的移动放在同步命令里

```rust
#[tauri::command]
pub fn fence_restore_item(window: WebviewWindow, /* … */) -> Result<ItemOpResult, String> {
    // 同步命令跑在主线程：管理员权限对话框以收纳箱窗口为父，给主线程同步发消息 → 死锁
    fence_fs::restore_item(&folder, &file_name, original, owner_hwnd(&window))
}
```

#### Correct — async 命令：先登记，在 async 上下文取 HWND，移动进阻塞线程池

```rust
#[tauri::command]
pub async fn fence_restore_item(app: AppHandle, id: i64, file_name: String) -> Result<ItemOpResult, String> {
    let moving = begin_moving(&[id])?;      // 先登记再读 folder_name
    let owner = fence_owner(&app, id);
    blocking(move || { let _moving = moving; /* load_fence → restore_item(…, owner) → delete_item */ }).await
}
```

#### Wrong — 每次 `onMoved` 都落库 / 标题栏用拖动区域

```vue
<div class="fence-titlebar" data-tauri-drag-region>  <!-- 双击最大化，冲掉嵌入样式 -->
```

```ts
appWindow.onMoved(() => invoke('commit_fence_geometry', { id, expandedView })) // 显示器变化的系统挪窗也被落库
```

#### Correct — 超过阈值才拖动，闸门之内的移动才落库

```ts
function onTitleMouseMove(e: MouseEvent) {
  if (!pressPoint || e.buttons !== 1) return
  if (!exceedsDragThreshold(e.screenX - pressPoint.x, e.screenY - pressPoint.y)) return
  pressPoint = null
  if (locked.value || editingTitle.value) return
  geometryGate.arm(Date.now())
  appWindow.startDragging()
}
function onGeometryEvent() {
  if (!geometryGate.accept(Date.now())) return // 用户没发起拖动：忽略
  clearTimeout(commitTimer)
  commitTimer = setTimeout(() => void commitGeometry(), GEOMETRY_COMMIT_DEBOUNCE)
}
```

---

## Design Decisions

### Design Decision: 真收纳 + 可见文件夹为 source of truth

**Context**：Fences / 腾讯 / 360 让图标从桌面消失靠的是接管 Explorer 桌面视图（要注入 Explorer）；纯引用（NoFences）零风险但桌面
不会变干净。

**Decision**：物理移动文件（TuckPane 模式）到 `%USERPROFILE%\MiniTodo收纳箱\<名称>\`，文件夹是内容的唯一来源，数据库只存元数据、
排序与原始路径。卸载 / 崩溃 / 数据库丢失都不丢文件，最坏情况是用户在该文件夹里手动找回。代价：依赖桌面路径的脚本会失效；OneDrive
桌面下的移动会同步到其他设备（首次提示一次）；公共桌面项需要 UAC。

### Design Decision: 始终嵌入，「固定」只是前端锁定

**Decision**：收纳箱复用主窗口的 owner=Progman + tao `always_on_bottom` 方案（`desktop_attach_hwnd`），建窗时 tao 标志一次给齐、
之后不改；窗口建成 `resizable(false)`，缩放用前端手柄 + `SetWindowPos`。锁定 / 解锁、折叠 / 展开都不碰 tao 标志，从根上避开
`apply_diff` 抹样式。实测多个收纳箱重叠时点击不会成组重排（被点的升到组内最上，其余相对顺序不变），因此**不**子类化
`WM_WINDOWPOSCHANGING`。

### Design Decision: 每个收纳箱一个 WebView2 渲染进程，先不加 `--process-per-site`

**Context**：同源窗口在 WebView2 里也各占一个 renderer。PR0 实测开发版私有内存：仅主窗口约 208MB，1 / 3 / 5 个收纳箱约
261 / 403 / 440–507MB（每个约 +45~60MB）。`--process-per-site` 能把 5 个收纳箱降到约 336MB，但所有窗口共用一个渲染进程（一个页面
卡死 / 崩溃连累全部），且是整个 WebView2 环境的启动参数（要在主窗口 `additionalBrowserArgs` 里设并保留 wry 默认的
`--disable-features=…`）。

**Decision**：2026-10-15 用户决定**先不加**，保持窗口间隔离。以后要做内存优化从这里入手。

### Design Decision: 收纳箱数据是设备本地数据

路径与屏幕坐标只对这台设备有意义，性质同 `screen_configs`：三张表与 `fence_onedrive_notice_shown` 都不进导出 / WebDAV 同步、不接
变更计数与墓碑（见 3.3 与 [sync-protocol.md §3.6](./sync-protocol.md)）。

---

## 已知限制

- **Win10 / Win11 ≤ 23H2**：显示桌面时抬起的是 `WorkerW` 而不是 Progman，Win+D 可能盖住收纳箱（与主窗口嵌入桌面相同的限制，
  见 [window-modes.md](./window-modes.md)）。实测环境为 Win11 24H2 之后的版本。
- **内存**：每个收纳箱一个 WebView2 渲染进程（数据见上方 Design Decision）。
- **显示 / 隐藏瞬间**：`show()` 后约 1.7ms 处于"可见 + `WS_EX_APPWINDOW`"，随即被排队的重挂改回；UIA 轮询未见任务栏按钮。
  若在别的机器上看到闪烁，可改为建窗时 `owner_raw(Progman)`（tao 走 `Parent::OwnedBy`，`apply_diff` 不再写 `WS_EX_APPWINDOW`）。
- **无法自动化的路径**：公共桌面项的 UAC 取消、删除时文件被占用（"文件正在使用"对话框）只能人工验证；判定逻辑有单测。
- 收纳的图片 / 视频显示类型图标而不是缩略图（`SIIGBF_ICONONLY`）；隐藏文件不显示、不计入删除确认的 N，但删除时会一并移回。
- Explorer 重启后某个收纳箱的 Z 序可能落在隐身的 Shell CoreWindow 之上（仍在所有普通应用窗口之下），不影响可见行为。
- dev 下首帧可能闪一下白底（`index.html` 内联背景先于样式表），release 预计不受影响。
