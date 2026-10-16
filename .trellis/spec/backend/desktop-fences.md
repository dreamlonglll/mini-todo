# 桌面收纳箱（Desktop Fences）

> 收纳箱的 code-spec。实现：
> - core（`pc-gpui/crates/core/src/`）：`fence/mod.rs`（元数据、几何、按屏幕组合记忆位置、删除）、`fence/items.rs`（内容对账、移入 /
>   移回 / 跨箱、打开、图标、目录监听、`FenceActivity` 登记）、`services/fence_fs/`（文件层，与数据库无关）；
> - app（`pc-gpui/crates/app/src/`）：`services/fences.rs`（建窗与嵌入、启动恢复、托盘三个入口、监听结果回前台、200ms 嵌入守护、
>   约 2 秒一次的屏幕组合检测）、`windows/fence/`（`mod.rs` 标题栏 / 折叠 / 移动 / 缩放 / 改名 / 外部拖入，`grid.rs` 项目网格，
>   `panels.rs` 外观 / 删除确认 / OneDrive 面板，`logic.rs` 纯函数）、`platform/embed.rs`（与主窗口共用的嵌入核心）。
>
> 建立于任务 `10-15-desktop-fence`（2026-10-15，旧 Tauri 版，Win11 10.0.26300、双屏混合 DPI 150% + 125% 上 e2e 实测），随
> `10-15-gpui-rewrite` 移植到 GPUI；旧 Tauri 版已删除（见 git 历史）。主窗口的三种模式见 [window-modes.md](./window-modes.md)，本文件只讲收纳箱。
> 文件层逐函数说明见归档任务文档 `.trellis/tasks/archive/2026-10/10-15-desktop-fence/fence-fs-api.md`。
>
> **几何一律是客户区（= 可见区域）的物理像素**：GPUI 窗口在左 / 右 / 下另有不可见边框（96 DPI 下各 8px），摆窗用
> `platform::set_client_rect_physical` / `Hwnd::set_client_rect`，落库 / 缩放起点 / 屏幕组合切换读 `client_rect_physical`
> （旧版无边框窗口的外框就是客户区，库里的几何沿用它；e2e F07 断言可见区域 = 库里的几何）。

---

## 场景：桌面上的真收纳区域（窗口 + 文件 + 屏幕组合布局）

### 1. Scope / Trigger

- 触发：一种特殊窗口（始终嵌入桌面层的透明窗口）、迁移 v29（3 张表 + 1 个 settings 键）与 v30（隐藏图标的两列）、Win32 嵌入与置底、
  会移动用户文件的 Shell 操作（硬约束：**不丢文件、绝不覆盖**）、3 个 debug 专用环境变量。
- 改动以下任何东西前先读本文件：
  - core `fence/**`、`services/fence_fs/**`、`settings::delete_screen_config`、`db/paths.rs` 的隔离变量、`fences` / `fence_items` /
    `fence_layouts` 表
  - app `services/fences.rs`、`windows/fence/**`、`platform/embed.rs`（主窗口也用）、托盘收纳箱菜单项（`services/tray.rs`）
- 平台：托盘入口与启动恢复（`services::fences::restore_all`）只在 Windows；core 的移动 / 打开 / 定位 / 图标在非 Windows 返回
  `Err("桌面收纳箱仅支持 Windows")`。Linux 上收纳箱窗口只用于截图 / 冒烟（`MINI_TODO_DEBUG_OPEN=fence:<id>` / `fence:new`）。

### 2. Signatures

```rust
// ===== core fence（crates/core/src/fence/mod.rs）；几何一律物理像素 =====
pub const FENCE_LABEL_PREFIX: &str = "fence-";
pub fn fence_label(id: i64) -> String;                     // "fence-{id}"（app 用作窗口登记 key）
pub fn is_fence_label(label: &str) -> bool;
pub const FENCE_TITLE_BAR_DIP: f64 = 32.0;  pub const MIN_WIDTH_DIP: f64 = 160.0;  pub const MIN_EXPANDED_HEIGHT_DIP: f64 = 160.0;
pub const MAX_TITLE_CHARS: usize = 40;  pub const MIN_ICON_SIZE: i32 = 24;  pub const MAX_ICON_SIZE: i32 = 128;
pub struct PxRect { x, y, width, height }                  pub struct MonitorArea { work: PxRect, scale: f64 }
pub fn stored_rect(fence: &Fence) -> PxRect;
pub fn fence_view(stored: PxRect, expanded: bool, areas: &[MonitorArea]) -> (PxRect, f64); // 窗口应占矩形 + 目标显示器缩放
pub fn view_rect(stored: PxRect, expanded: bool, title_height: i32, works: &[PxRect]) -> PxRect;
pub fn title_scale(stored: PxRect, areas: &[MonitorArea]) -> f64;
pub fn committed_rect(window: PxRect, expanded_view: bool, current: PxRect) -> PxRect;
pub fn layout_after_switch(..) -> PxRect;  pub fn effective_rect(..) -> PxRect;  pub fn reset_layout(..);  pub fn default_fence_rect(..);
pub fn normalize_title(raw: &str) -> Result<String, String>;  pub fn normalize_color(raw: &str) -> Result<String, String>;
pub struct Placement { pub screen_id: String, .. }        // Placement::for_screen(db, screen_id).rect(&fence)
pub fn list(db) -> R<Vec<Fence>>;  pub fn get(db, id) -> R<Fence>;
pub fn update(db, id, &UpdateFenceRequest) -> R<Fence>;   // 折叠 / 锁定 / 外观 / 图标大小 /「显示隐藏的图标」，只写库
pub fn rename(db, id, title) -> R<RenameFenceResult { fence, folder_error }>;
pub fn create(db, work: PxRect, scale: f64, screen_id: &str) -> R<Fence>;   pub fn rollback_create(db, &Fence);
pub fn prepare_restore(db, screen_id) -> R<Vec<(Fence, PxRect)>>;
pub fn commit_geometry(db, id, screen_id, window_client: PxRect, expanded_view: bool) -> R<Fence>;
pub fn reset_positions(db, primary_work: PxRect, scale: f64, screen_id) -> R<Vec<(Fence, PxRect)>>;
pub fn observe_screen_config(screen_id: &str) -> Option<String>;  pub fn forget_screen_config();
pub fn screen_switch(db, previous, current, &[(i64, Option<PxRect>, f64)]) -> Vec<(Fence, PxRect)>;
pub fn delete(db, id, owner_hwnd: Option<isize>) -> R<DeleteFenceResult { deleted, restored, remaining, failures }>; // 后台线程
fn plan_delete(restored: &fence_fs::RestoreAllResult) -> DeletePlan;            // 纯函数，有单测
static SESSION_RECTS: Mutex<BTreeMap<i64, PxRect>>;  static LAST_SCREEN_ID: Mutex<Option<String>>;

// ===== core fence::items（crates/core/src/fence/items.rs） =====
pub use crate::services::fence_fs::{is_auto_hidden_name, FenceEntry, ItemOpResult, ItemOpStatus};
pub struct FenceRefresh { pub fence_id: i64, pub items: Vec<FenceEntry> }   // 旧 `fence-items-changed` 事件的替代；含隐藏项
pub const ONEDRIVE_NOTICE_KEY: &str = "fence_onedrive_notice_shown";
pub fn get_items(db, id) -> R<Vec<FenceEntry>>;
pub fn reconcile(db, id) -> R<Option<Vec<FenceEntry>>>;   pub fn refresh(db, id) -> Option<FenceRefresh>;
pub fn move_in(db, id, &[String], owner_hwnd) -> R<(FenceMoveInResult { results, onedrive_notice }, Vec<FenceRefresh>)>; // 后台线程
pub fn restore(db, id, file_name, owner_hwnd) -> R<ItemOpResult>;                                                     // 后台线程
pub fn move_to(db, id, file_name, target_id, owner_hwnd) -> R<(ItemOpResult, Option<FenceRefresh>)>;                 // 后台线程
pub fn open(db, id, file_name) -> R<()>;  pub fn reveal(db, id, file_name) -> R<()>;
pub fn icon(db, id, file_name, size_px: u32) -> R<PathBuf>;   // 图标 PNG 缓存路径，GPUI 用 img(PathBuf)
pub fn reorder(db, id, &[String]) -> R<()>;                  // 完整顺序（含隐藏项）
pub fn set_hidden(db, id, file_name, hidden: bool) -> R<FenceRefresh>; // 右键「隐藏」/「取消隐藏」：只写 fence_items.hidden（v30）
pub fn watch(db: Arc<Database>, on_refresh: impl Fn(FenceRefresh) + Send + 'static) -> R<FenceWatcher>; // 回调在 fence-watch 线程
pub fn spawn_startup_reconcile(db: Arc<Database>);            // 线程 fence-startup
pub(crate) fn begin_deleting(id) -> Result<DeletingGuard, String>;  pub(crate) fn begin_moving(ids: &[i64]) -> Result<MovingGuard, String>;
pub(crate) fn is_deleting(id) -> bool;  pub(crate) fn is_moving(id) -> bool;   // FenceActivity 登记，drop 即注销
fn reconcile_rows(..);  fn reconcile_locked(tx, id);  fn apply_hidden(conn, id, file_name, hidden, items);  // 隐藏标记写库（有单测）
fn record_cross_move(conn, from_id, from_name, to_id, to_name, original_path);  // 跨箱记账：搬运原始路径、不搬运隐藏标记
fn claim_onedrive_notice(conn) -> rusqlite::Result<bool>;

// ===== core db =====
pub const FENCE_COLUMNS: &str;  pub fn fence_from_row(row: &Row) -> rusqlite::Result<Fence>;   // db/models.rs
pub const DEFAULT_FENCE_BG_ALPHA: f64 = 0.45;
pub const DEV_DATA_DIR_ENV: &str = "MINI_TODO_DATA_DIR";   // db/paths.rs
pub fn dev_data_dir_override() -> Option<PathBuf>;          // release 恒 None（不读环境变量）
fn data_dir_override(raw: Option<OsString>, debug_build: bool) -> Option<PathBuf>; // 纯函数
pub fn settings::delete_screen_config(db, config_id) -> R<()>;   // 连带删 fence_layouts

// ===== core services::fence_fs（全部阻塞，Result 错误为中文） =====
pub fn fence_root_dir() -> Result<PathBuf, String>;          // 不存在则创建
pub fn icon_cache_dir() -> PathBuf;                          // <app_data_dir>\fence-icons，不创建
pub fn user_desktop_dir() -> Option<PathBuf>;                // dirs::desktop_dir()（公共桌面 public_desktop_dir 只在模块内用）
pub fn path_starts_with_ci(path: &Path, prefix: &Path) -> bool;  pub fn same_path_ci(a: &Path, b: &Path) -> bool;
pub fn is_valid_item_name(name: &str) -> bool;               // 单个路径段：拦 ..、绝对路径、ADS
pub fn is_auto_hidden_name(file_name: &str) -> bool;         // 以 . 开头 = 自动隐藏（纯函数，core / app 共用）
impl FenceEntry { pub fn is_hidden(&self) -> bool }          // hidden || auto_hidden
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
pub fn launch_item(path: &str) -> Result<(), String>;  pub fn reveal_in_explorer(path: &str) -> Result<(), String>;
pub fn icon_png(path: &str, size_px: u32) -> Result<PathBuf, String>;
pub fn prune_icon_cache<P: AsRef<Path>>(live_paths: &[P]) -> Result<usize, String>;
pub fn watch_fence_root(on_change: impl Fn(FenceChange) + Send + 'static) -> Result<FenceWatcher, String>;

// ===== app services::fences（crates/app/src/services/fences.rs） =====
pub struct FenceRuntime { hidden: bool, windows: HashMap<i64, FenceWindowRecord { handle, view, hwnd }>, watcher: Option<FenceWatcher>, guard_started: bool } // Global
pub fn open_fence_window(cx, fence: Fence, rect: PxRect, scale: f64) -> Option<AnyWindowHandle>; // 已存在则直接返回
pub fn restore_all(cx);            // 启动恢复（仅 Windows）
pub fn create_fence(cx);           // 托盘「新增收纳箱」/ MINI_TODO_DEBUG_OPEN=fence:new
pub fn toggle_visibility(cx) -> bool;  pub fn reset_positions(cx);   // 托盘「显示/隐藏所有收纳箱」「重置收纳箱位置」
pub fn ensure_watcher(cx);  pub fn deliver_refresh(cx, FenceRefresh);  pub fn close_deleted(cx, fence_id);
pub fn apply_window_rect(window, cx, rect: PxRect, scale: f64);   // Windows：按客户区一次 SetWindowPos（延后执行）
pub fn monitor_areas(cx) -> Vec<MonitorArea>;                    // 主显示器排第一；取不到工作区时用整块显示器
const GUARD_TICK: Duration = 200ms;  const SCREEN_CHECK_TICKS: u32 = 10;

// ===== app windows::fence（crates/app/src/windows/fence/） =====
pub fn fence_key(fence_id: i64) -> String;     // = core fence_label
// logic.rs（纯函数，单测向量来自旧前端 fence.test.ts）
pub fn normalize_fence_title(raw) -> Option<String>;  pub fn exceeds_drag_threshold(dx, dy, threshold) -> bool;
pub struct GeometryCommitGate { arm(now), accept(now) -> bool, is_armed(now) -> bool, disarm() }
pub fn resize_rect(..) -> PxRect;  pub fn icon_bucket(..);  pub fn icon_request_px(..);  pub fn clamp_icon_size(..);  pub fn next_icon_size(..);
pub fn summarize_op_results(..) -> OpSummary { moved, cancelled, failed, notes, auto_hidden };
pub fn describe_move_in(&OpSummary, show_hidden: bool);  pub fn describe_delete_kept(..);  pub fn delete_hint(..);
pub fn visible_items(&[FenceEntry], show_hidden) -> Vec<&FenceEntry>;    // 网格显示哪些项
pub fn merge_sorted<T>(full: &[T], sorted_visible: &[T], key) -> Vec<T>;  // 排序结束：隐藏项按原位置并回完整顺序
pub fn empty_hint(total_items: usize) -> String;                         // "把桌面图标拖到这里" / "N 个项目已隐藏"
pub const ICON_BUCKETS: [u32; 8] = [16, 24, 32, 48, 64, 96, 128, 256];  pub const ICON_SIZE_STEP: i32 = 8;
pub mod timing { HOVER_EXPAND_DELAY 150ms, HOVER_COLLAPSE_DELAY 400ms, DRAG_COLLAPSE_DELAY 300ms, GEOMETRY_ARM_MS 3000,
                 GEOMETRY_COMMIT_DEBOUNCE 300ms, MAX_ICON_REQUESTS 3, .. }
```

core 与 app 必须成对的常量：`logic::ICON_BUCKETS` ↔ `fence_fs::ICON_SIZE_BUCKETS`；`normalize_fence_title` ↔ core `normalize_title`；
标题栏高度、最小尺寸、图标大小范围 app 直接用 core 的常量（`FENCE_TITLE_BAR_DIP`、`MIN_WIDTH_DIP`、`MIN_ICON_SIZE` / `MAX_ICON_SIZE`）。

### 3. Contracts

#### 3.1 收纳箱窗口

**标识**：`WindowKind::Fence`，`WindowManager` 登记 key = `fence-{id}`（`id = fences.id`，`is_fence_label` 只认 `fence-` 后接纯数字）；
运行时记录在 `services::fences::FenceRuntime.windows`。

**建窗**（`open_fence_window`，已有同 id 窗口时直接返回它）：

1. 建窗与所有会给窗口同步发消息的 Win32 调用都不在 App 借用期间执行（[pc-gpui/gpui-conventions.md §2.2](../pc-gpui/gpui-conventions.md)）：
   托盘回调只把事件投进 channel，由前台任务执行；嵌入在 `cx.spawn` 的 async 块里做。
2. `WindowSpec` 参数**一次给齐**：

   | 参数 | 值 | 原因 |
   |---|---|---|
   | `bounds` / `client_rect` | 逻辑 = 物理 ÷ 目标显示器缩放；`client_rect` 给精确的物理客户区 | `windows::open_window` 建好后按客户区校正一次（混合 DPI 下逻辑坐标差几个像素） |
   | `transparent` | true | 无系统标题栏的透明窗口，界面自己画圆角底色；DWM 圆角另外 `set_round_corners(true)` |
   | `resizable` | false | GPUI 的缩放热区按建窗时的 `is_resizable` 计算：不可缩放、不参与 Aero Snap；缩放走窗口内手柄（见下） |
   | `minimizable` | false | Win+M 不最小化它 |
   | `focus` | false | 建窗不抢焦点 |
   | `show` | Windows 上 false | 先藏着建好，定位、改样式、嵌入之后再 `SW_SHOWNOACTIVATE` 显示，不闪任务栏 |
   | `min_size` | 160 × 32（逻辑） | 折叠态只有标题栏高；展开态最小高度由界面逻辑保证 |
   | `display_id` | 矩形所在显示器 | |

3. 关闭：`window.on_window_should_close(cx, |_, _| false)`——没有关闭按钮，Alt+F4 / `WM_CLOSE` 一律拦截。删除收纳箱走
   `services::fences::close_deleted` → `WindowManager::close`；窗口关闭回调里从 `FenceRuntime.windows` 注销，最后一个关闭时 `forget_screen_config`。
4. 建好后（Windows，`embed_after_open`，async 块里）：`set_client_rect` → `set_tool_window(true)` → `set_minimizable(false)` →
   `set_round_corners(true)` → `platform::embed::attach_to_desktop` → `set_always_on_bottom(true)` → 不处于"全部隐藏"时 `show()`。
   然后启动 200ms 守护（只启动一次）。在它之前，`windows::open_window` 排队的前台任务已经去掉 `WS_SYSMENU` 并按客户区校正过
   （同一个前台队列、先排先执行），所以显示时 DWM 不会在半透明底色下透出系统 ×（[pc-gpui/gpui-conventions.md §2.2](../pc-gpui/gpui-conventions.md)）。

**嵌入后的 Win32 状态**（Windows e2e F 组断言，任何操作之后都不变）：

| 字段 | 值 |
|---|---|
| owner（`GWLP_HWNDPARENT`） | Progman（`GetShellWindow()`） |
| `WS_EX_TOOLWINDOW` / `WS_EX_APPWINDOW` | 有 / 无（不进任务栏、不参与 Alt+Tab） |
| `WS_MINIMIZEBOX` | 无 |
| `WS_SYSMENU` | 无（`open_window` 建窗后去掉；DWM 标题栏按钮区 `DWMWA_CAPTION_BUTTON_BOUNDS` 宽度为 0） |
| `WS_EX_TOPMOST` / `WS_EX_NOACTIVATE` | 无 / 无 |
| Z 序 | `subclass::FLAG_BOTTOM`：每次改 Z 序都压到 `HWND_BOTTOM` = Progman owned 组内最底，始终在所有普通应用窗口之下 |

**登记与守护**：

- `platform::embed` 内部登记每个嵌入窗口挂上的宿主（与主窗口共用，见 [window-modes.md](./window-modes.md)）。
- 200ms 守护（`services::fences::ensure_guard`，Windows）：每个 tick 取全部收纳箱句柄，对存活的窗口
  `platform::embed::needs_reattach(h) || (Explorer 在运行 && lacks_tool_window_style(ex))` 时 `set_tool_window(true)` →
  `attach_to_desktop` → `set_always_on_bottom(true)`（在 async 块里、`update` 之外）。
- 每 10 个 tick（约 2 秒，`SCREEN_CHECK_TICKS`）调一次屏幕组合检测（见 3.5）。

**之后允许的窗口操作**：

| 操作 | 规则 |
|---|---|
| 摆窗（`services::fences::apply_window_rect` / `platform::set_client_rect_physical`） | 按客户区一次 `SetWindowPos(NOZORDER \| NOACTIVATE)`（延后到下一个前台任务）；跨 DPI 时按新边框再校正 |
| 改标题（改名后） | `window.set_window_title` |
| 显示 / 隐藏全部 | `Hwnd::show` / `hide`（`ShowWindow`，不改 ex style）；显示后再 `attach_to_desktop` + `set_always_on_bottom`（幂等） |
| 改 resizable / minimizable / 加 `WS_EX_NOACTIVATE` / `WS_EX_LAYERED` / 加回 `WS_SYSMENU` | **禁止** |

**锁定（「固定」）完全由界面控制**：锁定时不发起拖动、不显示缩放手柄；仍可拖入、折叠、改名、打开项目。Win32 样式不随锁定变化。

**移动**：标题栏 `on_mouse_down` 只记下按下点；`on_mouse_move` 时左键仍按着且位移超过 3 逻辑像素（`exceeds_drag_threshold`）
才 `gate.arm(now)` + `platform::begin_window_drag(window, cx)`（延后执行 `ReleaseCapture` + `WM_NCLBUTTONDOWN(HTCAPTION)`）。
不在 `mouse_down` 里直接发起拖动（模态拖窗循环会吞掉 `mouse_up`，双击改名收不到）；标题栏**不用** `ui::title_bar::drag_region()`
（`HTCAPTION` 区域双击会触发系统行为）。改名中不拖动。

**缩放**：窗口是 `resizable: false`，系统不进缩放循环。8 个手柄（n/s/w/e/nw/ne/sw/se）只在 `can_resize()`（未锁定、未折叠、
不在改名）时显示：按下 → 先 `flush_geometry` → 记客户区起点与光标（Windows 用屏幕物理坐标，拖左 / 上边时窗口在动）；移动 → `resize_rect`
（拖哪条边改哪条边，对边不动，不小于 `round(160 × dpr)`）→ 每帧最多一次 `apply_window_rect`（`on_next_frame`，只保留最新一帧）；
松开 → 最后一帧落地后（约 16ms）`commit_geometry`。

**几何落库（只有用户发起的才落库）**：

| 时机 | 写什么 |
|---|---|
| 新建收纳箱 | `fence::create`：`INSERT fences`（x/y/width/height = 默认位置）+ 当前屏幕组合的 `fence_layouts` |
| 用户拖动结束 | `GeometryCommitGate`：发起拖动时 `arm`，`observe_window_bounds` 的变化在闸门打开期间才算数（每次续期 3000ms），停下 300ms 后 `commit_geometry` |
| 用户缩放结束 | 松开后 `commit_geometry`（不经闸门） |
| 托盘「重置收纳箱位置」 | `fence::reset_positions` 事务内批量写 `fences` + 当前组合的 `fence_layouts` |

`commit_geometry` 读窗口当前客户区，按 `committed_rect` 计算：只剩标题栏（`expanded_view = false`）时只落位置和宽度，展开高度保持库里的值；
写 `fences.x/y/width/height`（"最近一次位置"）与当前组合的 `fence_layouts`（同一事务），并移除该收纳箱的 `SESSION_RECTS`。
**不落库**：启动夹回、显示器变化导致的系统挪窗、折叠 / 展开、屏幕组合切换、缩放中的每一帧。界面做任何程序调整（折叠 / 展开、改外观、
开始缩放）前先 `flush_geometry`（有待落库的拖动立即落库，然后 `disarm` 闸门）。

**折叠 / 展开**：

- `fence::update({ collapsed })` 只写库；窗口形态由 `sync_view` 呈现（`Placement::for_screen(..).rect(..)` → `fence_view` → 摆窗，不写库）。
- 临时展开不改持久化的折叠态。原因集合（`ExpandReason`，任一存在即展开）：`Hover`（进入 150ms 后展开、离开 400ms 后收回，拖动闸门
  打开期间不展开也不收回）、`Drag`（外部文件拖入经过；离开 / 放下后 300ms 收回）、`Menu`（标题栏「更多」菜单）、`Panel`（外观 / 删除确认 /
  OneDrive 提示）、`ItemMenu`（项目右键菜单）。形态切换带代次，晚到的旧计算结果丢弃。
- `view_rect`：折叠 = 标题栏矩形（高度 = `round(32 × 目标显示器缩放)`）；展开 = 库里的展开尺寸，下方空间不够时整体上移
  （向上展开，标题栏原位置仍在窗口内）。两者都按标题栏所在工作区夹回（交叠面积最大的工作区；都不交叠取离中心最近的）。
- 缩放比例按**目标位置**所在显示器取（`title_scale`），不取窗口当前所在显示器：混合 DPI 下重置位置 / 启动夹回时两者不同。

**必须可激活**：不加 `WS_EX_NOACTIVATE`。否则双击打开的程序可能拿不到前台；未激活的收纳箱上第一次点击即生效。

**外部拖入**：用 `ui::external_drop::ExternalDrop`（`DropScope::Window`），**不用** GPUI 的 `.on_drop::<ExternalPaths>()`：
gpui-pre 0.3.9 的文件拖放不改输入模态，`on_drop` 要求 `is_hovered`，窗口最后一次输入是键盘时（改名回车后）松手会被丢掉（e2e F10，
见 gpui-conventions §2.5.1）。

**显示 / 隐藏全部**：`FenceRuntime.hidden` 只是运行时状态（重启后总是显示）。新建收纳箱、重置位置时如果处于隐藏状态，先全部显示。

**新建位置与标题**：光标所在显示器的工作区（取不到退回主显示器，再取不到按 1920×1080@1.0 的 `FALLBACK_WORK_AREA`）；尺寸
`(88×4 + 8×2) × (32 + 96×3 + 8×2)` = 368×336 DIP（150% 下 552×504）；从工作区右上角起、从右往左逐行找不与已有收纳箱
（按当前组合下的有效几何）重叠的位置（边距 24 DIP、步长 32 DIP、间隙半个边距），找不到就从右上角斜向叠放、只避开左上角重合；
标题取 `收纳箱` / `收纳箱 2` / `收纳箱 3` … 中第一个空闲的。建窗失败 → `rollback_create`（删记录 + 删空文件夹）。

**重置位置**：主显示器工作区，从右上角起从右往左逐行排开（间距 16 DIP），放不下回到右上角错开叠放；尺寸夹到工作区内；
写库后清空 `SESSION_RECTS`，逐个按持久化的折叠态 `fence_view` 摆窗。

#### 3.2 返回结构与刷新

- `Fence`（serde camelCase）：`id, title, folderName, x, y, width, height, collapsed, locked, bgColor, bgAlpha, iconSize,
  sortOrder, createdAt, updatedAt, showHidden`。几何是物理像素：x/y 为虚拟屏幕坐标（折叠态即标题栏左上角），width/height 为展开态尺寸。
  `showHidden`（v30，`#[serde(default)]`）=「显示隐藏的图标」，每个收纳箱独立。`FENCE_COLUMNS` 末尾追加 `show_hidden`（下标 15）。
- `UpdateFenceRequest`：字段全可选，只更新给出的；`bgColor` 只接受 `#RRGGBB`（统一大写），`bgAlpha` 夹到 0~1（非有限数按
  0.45），`iconSize` 夹到 24~128，`showHidden` 原样写入。任一字段校验失败什么都不写。标题走 `fence::rename`，几何走 `commit_geometry`。
- `RenameFenceResult { fence, folder_error }`：`folder_error` 非空 = 标题已改、收纳文件夹没跟着改名（保留旧 `folder_name`），
  界面提示"标题已修改，但收纳文件夹没能跟着改名：…"，窗口标题随之更新。
- `DeleteFenceResult { deleted, restored, remaining, failures }`：
  - `deleted = true`：收纳文件夹里的项目全部移回、空文件夹已删除 → 删 `fences` 行（`fence_items` / `fence_layouts` 级联）、
    移除 `SESSION_RECTS`；界面随后 `close_deleted` 关窗
  - `deleted = false`：删除**中止**，收纳箱整个保留（记录、窗口、文件夹、布局）；已移回项的 `fence_items` 行删掉，注销"删除中"后
    `items::refresh` 刷新窗口；界面关掉确认框，用 `describe_delete_kept` 提示"N 个项目未能移回桌面，收纳箱已保留：名字（原因）…"（8 秒）
  - `restored` 已移回项数；`remaining` 仍留在文件夹里的文件名（含带隐藏属性的文件）；`failures` 为 `failed` / `cancelled` 的逐项结果
  - 返回 `Err`：还原整体无法进行（桌面路径未知、根目录不可用等），什么都没删
- `FenceEntry { file_name, display_name, path, is_dir, is_shortcut, modified_ms, hidden, auto_hidden }`（serde camelCase）：
  `display_name` 去掉 `.lnk` / `.url`；文件夹（含指向文件夹的链接）`is_dir = true`。隐藏信息（v30）：
  - `hidden`：用户右键「隐藏」的标记，对账时从 `fence_items.hidden` 带出（文件层 `list_fence_items` 一律 `false`）
  - `auto_hidden`：文件名以 `.` 开头（`is_auto_hidden_name`，所有平台一样），按名字自动隐藏，不存标记、不能单独取消隐藏
  - `is_hidden() = hidden || auto_hidden`。隐藏只影响收纳箱的显示，不改文件属性、不移动文件
- **隐藏项的显示（app）**：`get_items` / `FenceRefresh` / `set_hidden` 返回**全部**项目（含隐藏项）；窗口按 `Fence.show_hidden` 过滤
  （`logic::visible_items`）：关闭时网格只排可见项，全部被隐藏时空状态显示"N 个项目已隐藏"；开启时隐藏项照常排在网格里，图标与名字
  整体半透明（`theme::FENCE_HIDDEN_ITEM_OPACITY`）。图标懒加载只请求显示中的项目；选中项 / 右键菜单对应项不再显示时取消选中 / 关闭菜单。
  箱内拖动排序只排显示项，松手后 `logic::merge_sorted` 把隐藏项按原位置并回完整顺序再 `items::reorder`。
- **右键菜单**：「打开所在位置」之后是「隐藏」（`EpIcon::Hide`）；隐藏状态的项（显示隐藏的图标时才看得到）是「取消隐藏」（`EpIcon::View`）；
  点文件的「取消隐藏」禁用（半透明 + 右侧说明与 tooltip"以 . 开头的项目自动隐藏"，同「移动到…」无目标时的写法）。
  隐藏 / 取消隐藏经 `spawn_db` 后台 `items::set_hidden`，返回的列表交给 `set_items`，失败 `message::error_with` 后补一次列举。
- **标题栏「更多」菜单**：重命名 / 外观 /「显示隐藏的图标」（`EpIcon::View`，开启后文案换成「不显示隐藏的图标」、`EpIcon::Hide`——EP 下拉菜单
  没有勾选态）/ 分隔线 / 删除收纳箱；切换走 `patch_fence(UpdateFenceRequest { show_hidden })`，菜单宽度按最长一项估算（151 / 165）。
- **拖入提示**：`OpSummary.auto_hidden` = 移入后（按收纳文件夹里的新名字）以 `.` 开头的项数；收纳箱没开「显示隐藏的图标」时
  `describe_move_in` 补一句"N 个以 . 开头的项目已自动隐藏，可在「更多」中显示隐藏的图标"——全部成功时单独作为一条信息提示，
  有其他提示（失败 / 取消 / 附注）时接在后面合成一条，不连弹两条。
- `ItemOpResult { source, status, file_name, dest_path, error }`，`status ∈ moved | cancelled | failed | skipped`。
  `moved` 偶尔带 `error` 提示（"已放入目标位置，但源文件未能删除…"、"已移动，但无法确定新的文件名…"）。
- `FenceMoveInResult { results, onedrive_notice }`：`results` 与传入路径一一对应、顺序相同；`onedrive_notice = true` 时界面
  打开 OneDrive 提示面板（只会有一次）。
- `items::icon` 返回缓存 PNG 的绝对路径，界面 `img(PathBuf)` 加载；失败时显示占位图标。
- **`FenceRefresh`（旧 `fence-items-changed` 事件的替代）**：对账后按 `sort_order` 排好的完整列表，只交给对应收纳箱窗口
  （`services::fences::deliver_refresh` → `FenceWindow::set_items`；窗口不存在时丢弃）。来源：目录监听回调（`fence-watch` 线程，
  经 channel 回前台）、`move_in` / `move_to` 的返回值（来源 / 目标收纳箱）、删除中止后的 `items::refresh`。窗口给每次列表编号，
  箱内拖动排序期间不接收推送，松手后补一次列举。

#### 3.3 数据库 v29 / v30

```sql
-- v29
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

-- v30（10-16 fence-hidden-items）：隐藏图标
ALTER TABLE fence_items ADD COLUMN hidden INTEGER NOT NULL DEFAULT 0;      -- 用户右键「隐藏」
ALTER TABLE fences ADD COLUMN show_hidden INTEGER NOT NULL DEFAULT 0;      -- 「显示隐藏的图标」
```

- `LATEST_VERSION = 30`，迁移表驱动追加 `(29, migration_v29)`、`(30, migration_v30)`。
- v30 两列都有默认值 0：旧库升级后已有项目全部可见、开关关闭。以 `.` 开头的项目按名字自动隐藏，不存标记。
  旧版本（3.0.0 等）打开 v30 库只记警告、按现状继续运行（`db/connection.rs`，没有降级）；它们按显式列名读写这两张表，新列不影响，
  只是看不到隐藏效果（收纳箱数据不同步，只影响同一台机器上装回旧版本的情况，见 [pc-gpui/data-compat.md](../pc-gpui/data-compat.md)）。
- 级联删除依赖连接上 `PRAGMA foreign_keys = 1`（`migrate` 完成后开启，有单测）。删收纳箱只删 `fences` 行。
- `settings::delete_screen_config(config_id)` 在一个事务里删 `screen_configs` 行**和** `fence_layouts WHERE screen_config_id = ?`。
- `fence_items` 只是排序、原始路径与隐藏标记的附加信息，内容以文件夹为准（3.4）。
- 三张表（含 v30 的两列）都是**设备本地数据**：不接 v28 的 `local_seq` 变更计数触发器与墓碑，不进 `ExportData` / `SyncData`
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
  建窗失败 → `rollback_create`（删记录 + 回滚文件夹）。改名：整个过程持有数据库锁，文件夹改名失败则保留旧 `folder_name`。
- **列举（`list_fence_items`）**：跳过 `desktop.ini` / `Thumbs.db` 与带 Windows 隐藏 / 系统属性的文件（`has_hidden_or_system_attribute`，
  永不显示，"显示隐藏的图标"也不显示）；以 `.` 开头的项目所有平台都列出、`auto_hidden = true`（非 Windows 上不再按点开头排除，
  Linux 截图 / 冒烟里也能看到点文件）。移入校验 `plan_move_in` 仍用 `is_hidden_or_system`（Windows 看属性、其他平台看点开头），规则不变。
- **文件夹是 source of truth**：每次列举都对账（`reconcile_rows`）：名字先精确匹配、再不区分大小写匹配（资源管理器里只改大小写
  时保留原始路径、排序与隐藏标记，并把行里的名字改成新写法）；表里有、文件夹没有 → 删行（改成别的名字 = 新项目，隐藏标记随旧行丢掉）；
  文件夹有、表里没有 → 补行（`original_path = NULL`、可见，按文件名顺序排到末尾）；返回按 `sort_order` 排序，`hidden` 取自对应行。
- **对账在数据库锁内列举**：`items::reconcile` 在一个 `with_transaction` 里依次检查"删除中"、读 `folder_name`、
  `list_fence_items`（一次本地 `read_dir`）、写行（`reconcile_locked`）。文件夹不存在时 `ensure_fence_folder` 按原名重建后再列举（显示为空）。
- **隐藏 / 取消隐藏（`items::set_hidden`）**：先校验文件名（`is_valid_item_name`），然后在一个事务（数据库锁）里：检查"删除中"→
  `reconcile_locked`（行不存在——监听还没补行、只改了大小写——也先对上）→ `apply_hidden` 按文件名先精确、再不区分大小写匹配，
  `UPDATE fence_items SET hidden = ?` → 返回写入后的完整列表（`FenceRefresh`）。只写库，不改文件属性；锁顺序同上（数据库锁 → 登记锁）。
- **移入 / 跨箱写库一律 upsert**：`INSERT … ON CONFLICT(fence_id, file_name) DO UPDATE SET original_path = excluded.original_path, hidden = 0`
  （新行排末尾；已有行保留排序）。监听回调触发的对账可能已经先补了一行 `NULL`。刚移进来的项一律可见：已有同名行（监听先补的、或残留的
  旧行）的隐藏标记清零。
- **跨箱拖入 / 「移动到…」**：源路径形如 `<根目录>\<另一个收纳箱的文件夹>\<项>` 时按跨箱处理：在移动**之前**读旧行的 `original_path`
  （移动一完成监听对账就可能删掉旧行），成功后 `record_cross_move`：删旧行、在本箱 upsert 并搬运原始路径（**不**搬运隐藏标记，
  目标收纳箱里可见），然后返回来源收纳箱的 `FenceRefresh`。其他来源：`original_path = 源路径`。移回桌面后在桌面照常可见（文件属性没变过）。
- **记账失败不翻案**：文件已经移动成功而写库失败时只记 `warn` 日志、照常返回逐项成功（下次对账补行，只是丢了原始路径）。
- **`FenceActivity`（进行中的文件操作登记，`fence/items.rs`）**：移动 / 删除可能要等用户处理管理员权限或"文件正在使用"对话框，
  不能一直占着数据库锁，靠登记互斥：

  | 操作 | 登记 | 冲突时 |
  |---|---|---|
  | `fence::delete` | `begin_deleting(id)` | 已在删除 → `Err("收纳箱正在删除")`；有移动 → `Err("收纳箱里还有文件正在移动，请稍后再删除")` |
  | `items::move_in` | `begin_moving(&[id])`（不登记来源收纳箱） | 目标正在删除 → `Err("收纳箱正在删除")` |
  | `items::restore` | `begin_moving(&[id])` | 同上 |
  | `items::move_to` | `begin_moving(&[id, target_id])` | 任一正在删除 → 整次不开始、不留登记 |
  | `fence::rename`（数据库锁内检查） | — | 删除中 → `Err`；移动中 → 只改标题，`folder_error` = "收纳箱里有文件正在移动，文件夹暂未改名…" |
  | `items::reconcile`（数据库锁内检查） | — | 删除中 → `Ok(None)`，不补行、不重建文件夹 |

  规则：**先登记，再读 `folder_name`**；改名与对账在数据库锁内检查登记，与移动 / 删除读取文件夹名的数据库访问互斥，不会出现
  "已按旧文件夹名开始移动 / 还原，文件夹却被改名"的交错。**锁顺序：数据库锁 → 登记锁**，持有登记锁时绝不拿数据库锁。
- **删除 = `plan_delete`**：`restore_all` 之后 `delete_fence = restored.folder_removed`；`restored` = `moved` 项的文件名
  （取 `source` 的最后一段）；`failures` = `failed` + `cancelled`。**只有收纳文件夹已清空并删除才删收纳箱**；只要有项目留在文件夹里
  （失败、取消、或 `remaining` 里带隐藏属性的文件），整体保留，不产生没有收纳箱管理的孤儿文件夹。收纳箱里隐藏的项目（用户隐藏 + 点文件）
  同样移回，删除确认框的 N（`get_items` 的数量）也计入它们。
- **线程**：`fence_fs` 与 core `fence` 全部阻塞，app 一律放后台（`db::spawn_db`）。带 owner HWND 的移动（`fence::delete`、
  `items::{move_in, restore, move_to}`，内部是 `move_into_fence` / `restore_item` / `move_between_fences` / `restore_all`）**绝不在 UI 主线程**：
  系统"需要管理员权限"对话框以收纳箱窗口为父窗口，会给主线程同步发消息，主线程等结果就死锁。做法：HWND 在前台取好（`isize`），
  移动在后台线程里跑。需要 COM STA 的操作在 fence_fs 内部线程执行：移动 / 启动 / 定位每次新开 STA 线程（`fence-move` / `fence-launch` /
  `fence-reveal`），图标在常驻 STA 线程 `fence-icon` 上串行。
- **移入校验（`plan_move_in`，逐项）**：

  | 条件 | 结果 |
  |---|---|
  | 非绝对路径 / 磁盘根 / 无法访问 | `failed`（"需要完整路径" / "无法收纳磁盘根目录" / "无法访问: …"） |
  | 同一批重复源路径 | `skipped`（"重复的项目"） |
  | `desktop.ini` / `Thumbs.db` | `skipped` |
  | 隐藏或系统文件（Windows 看属性；其他平台看点开头） | `skipped`（列举也跳过带这两个属性的文件，避免"移走后哪里都看不到"）。没有隐藏属性的点文件（`.env`）在 Windows 上照常移入，移入后自动隐藏并提示 |
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
  目标处已有同名再加后缀。`restore_all` 处理文件夹里的**所有**项（含带隐藏属性的文件，`desktop.ini` / `Thumbs.db` 除外），所有项在同一个
  `IFileOperation` 里执行。删除确认框里的 N 用 `get_items` 的实际列举数（含收纳箱里隐藏的项目，不含带隐藏 / 系统属性的文件）。
- **监听**：`watch_fence_root`（`notify` 递归监听根目录，只关心第一、二层），最后一个事件后安静 300ms 回调一次、持续变化时最多 1.5s
  必回调；回调在线程 `fence-watch` 上 → 按文件夹名（不区分大小写）匹配收纳箱（`rescan` 时全部）→ 对账 → `FenceRefresh` 经 channel 回前台。
  `FenceWatcher` 由 app 的 `FenceRuntime` 一直持有（drop 即停止）；`services::fences::ensure_watcher` 在启动恢复（有收纳箱时）与新建后调用。
  本应用自己的移动同样触发回调，不区分来源。
- **启动**：`services::fences::restore_all` → `fence::prepare_restore`（逐个 `ensure_fence_folder`，用户可能删了文件夹；给出各自的存储几何；
  记屏幕组合检测起点）→ 逐个 `fence_view` + `open_fence_window` → 有收纳箱则 `ensure_watcher` → `items::spawn_startup_reconcile`
  （线程 `fence-startup`）：逐个对账、记录根目录下不属于任何收纳箱的文件夹（只记日志）、等 20 秒后 `prune_icon_cache(全部现存项目路径)`；
  任何一个收纳箱对账失败就跳过这次清理。
- **打开**：`launch_item` 用 `ShellExecuteExW`（`SEE_MASK_FLAG_NO_UI | SEE_MASK_NOASYNC`），目标失效的 `.lnk` 约 3 秒后才返回错误，
  界面在此期间显示"正在打开"。`items::open` / `reveal` / `icon` 先 `is_valid_item_name` 校验文件名
  （防 `..`、绝对路径、ADS 逃逸出收纳文件夹）。
- **图标**：界面请求 `ceil(icon_size × 缩放)`（`icon_request_px`），向上取整到档位 `[16, 24, 32, 48, 64, 96, 128, 256]`；
  `SIIGBF_ICONONLY`（`.lnk` 不带箭头，图片 / 视频显示类型图标）；缓存在 `<app_data_dir>\fence-icons\`，键
  `{路径哈希 FNV-1a 64}-{mtime 毫秒}-{大小}-{档位}.png`，先写临时文件再改名。界面按可见项懒加载（只请求显示中的项目，没开
「显示隐藏的图标」时不请求隐藏项），同时最多 3 个请求
  （`timing::MAX_ICON_REQUESTS`；后端图标在单个 STA 线程排队）。图标一律先渲染进缓存再由 `img(PathBuf)` 加载，从不直接加载收纳文件夹
  或桌面上的文件。
- **OneDrive 提示**：本次至少有一项从用户桌面移入（`path_starts_with_ci(source, desktop)`）、桌面被 OneDrive 重定向、且
  `claim_onedrive_notice` 返回 true 时，`onedrive_notice = true`。

#### 3.5 屏幕组合布局（PRD R5）

- **ID 格式**与主窗口 `screen_configs.config_id` 完全相同：`{显示器数}_{宽}x{高}@{缩放%}_…`，主窗口与收纳箱都经 core
  `settings::screen_config_id`（debug 下可被 `MINI_TODO_SCREEN_ID_FILE` 覆盖）→ `screen_config_id_of` 计算；app 侧入口
  `platform::screen_config_id(cx)`。输入是 `EnumDisplayMonitors` 枚举顺序的（物理像素宽高、缩放比例）：缩放% = `round(scale × 100)`；
  按像素面积**稳定**降序（同面积保持枚举顺序，与旧前端 JS `Array.prototype.sort` 一致）；无显示器为 `unknown`。单测
  `screen_config_id_matches_the_frontend_format` 固定这个格式——旧版本写下的 `screen_configs` / `fence_layouts` 记录靠它继续命中。
- **有效几何**（`effective_rect` / `Placement`）：当前组合的 `fence_layouts` 记录 → 本次运行的临时几何 `SESSION_RECTS` → `fences.x/y/width/height`。
  折叠 / 固定 / 外观 / 图标大小全局共用，不随组合变化。
- **写入时机**：只有 3.1「几何落库」表里的四种（新建、拖动结束、缩放结束、重置位置），每次都同时写当前组合的记录与 `fences` 的最近一次位置。
- **启动恢复**：`prepare_restore` 取当前组合 → 每个收纳箱按有效几何建窗，`fence_view` 夹回可见工作区（**不写库**）；有收纳箱时把
  `LAST_SCREEN_ID` 设为当前组合。
- **运行时切换**：守护每 10 个 tick（约 2 秒）`check_screen_layout`；没有收纳箱窗口时不读显示器、`forget_screen_config`。
  `observe_screen_config` 返回上一次的 ID 时，读各窗口当前客户区与缩放 → `screen_switch`：新组合有记录 → 用记录；没有 → 沿用窗口当前
  位置（当前高度 < 2 × 标题栏高度视为只剩标题栏，展开高度沿用切换前的有效几何），记进 `SESSION_RECTS`（整体替换，**不写库**）；
  然后按持久化的折叠态 `fence_view` 摆窗（夹回可见区域）。
- **删除屏幕配置**：`settings::delete_screen_config` 连带删除该组合的 `fence_layouts`。

#### 3.6 debug 专用开关（release 构建一律不读取）

| 环境变量 | 作用 |
|---|---|
| `MINI_TODO_DATA_DIR` | `db::paths::app_data_dir()` 返回该目录（空值视为未设置，相对路径转绝对）：库、图片、备份、节假日缓存、图标缓存 `fence-icons` 全部落到这里（新库没有 WebDAV 配置，不会同步）。另外：日志写到 `<dir>\logs`；**不启用单实例**；**跳过开机自启自愈**（不刷新与安装版共用的注册表项）；托盘提示改为"Mini Todo（开发版）" |
| `MINI_TODO_FENCES_ROOT` | 收纳根目录改为该目录（去空白后非空才生效，不存在由 `fence_root_dir` 创建） |
| `MINI_TODO_SCREEN_ID_FILE` | 文件内容（去首尾空白后非空）替代真实屏幕组合 ID，每次计算时重新读取；e2e 改写它模拟"屏幕组合变化"，走与真实切换相同的轮询 → 恢复路径 |
| `MINI_TODO_DEBUG_FENCE_STATE` | GPUI 版新增：`onedrive` 打开即显示 OneDrive 提示面板，`drag-hover` 显示外部拖入悬停框（Linux 上无法真实触发，截图用） |

release 保证：`dev_data_dir_override()` 的 `cfg(not(debug_assertions))` 分支直接 `None`、不读环境变量，纯函数
`data_dir_override(_, false) == None`（单测）；`location::fence_root_override` 的 release 分支恒 `None`（单测
`release_build_ignores_override`，只在 `cargo test --release` 时编译）；`dev_screen_id_override` 整段 `#[cfg(debug_assertions)]`，
release 不编译。Windows e2e 的 P01 在 release 构建上验证忽略 `MINI_TODO_DATA_DIR` / `MINI_TODO_DEBUG_OPEN`。

### 4. Validation & Error Matrix

| 条件 | 行为 |
|---|---|
| 收纳箱 id 不存在（get / update / rename / commit / delete / 移动） | `Err("收纳箱 #{id} 不存在")` |
| `items::get_items`：不存在或正在删除 | `Err("收纳箱 #{id} 不存在或正在删除")`（后台刷新不弹提示） |
| 标题规范化后为空 / 超过 40 字符 | `Err("收纳箱标题不能为空")` / `Err("收纳箱标题不能超过 40 个字符")`；界面 `normalize_fence_title` 同规则，不提交 |
| `bgColor` 不是 `#RRGGBB` | `Err("背景色格式无效：{value}")`，整个 patch 不写 |
| `bgAlpha` 非有限数 / 超范围；`iconSize` 超范围 | 0.45 / 夹到 0~1；夹到 24~128 |
| 新建：收纳文件夹建不出来 | `Err`，不插库、不建窗 |
| 新建：插库失败 / 建窗失败 | `Err("保存收纳箱失败: …")` / 建窗失败记日志并 `rollback_create`（删记录、回滚文件夹） |
| 改名：文件夹被占用等 | `Ok`，标题已改，`folder_error` 为原因，`folder_name` 不变 |
| 改名：有文件正在移动 / 正在删除 | `Ok` + `folder_error`（只改标题）/ `Err("收纳箱正在删除")` |
| 删除：有移动在进行 / 已在删除 | `Err("收纳箱里还有文件正在移动，请稍后再删除")` / `Err("收纳箱正在删除")` |
| 删除：`restore_all` 整体失败（如 `Err("无法确定桌面路径")`） | `Err`，什么都不删 |
| 删除：有项目 `failed` / `cancelled`，或文件夹里仍有剩余（隐藏文件） | `Ok { deleted: false, … }`，收纳箱整个保留 |
| 移动类操作：涉及的收纳箱正在删除 | `Err("收纳箱正在删除")` |
| `items::move_to` 源 = 目标 | `Err("目标收纳箱与当前收纳箱相同")` |
| `items::move_in` 的路径为空 | `Ok` 空结果 |
| 文件名不合法（`..`、绝对路径、`a:b`） | `Err("文件名不合法: …")` |
| 项目不在文件夹里（移回 / 跨箱） | 逐项 `failed`（"收纳文件夹中找不到该项目"） |
| `items::set_hidden`：收纳箱正在删除 / 不存在 / 文件夹里已没有该项 | `Err("收纳箱正在删除")` / `Err("收纳箱 #{id} 不存在")` / `Err("收纳文件夹中找不到该项目")`；界面"隐藏失败" / "取消隐藏失败"提示后补一次列举 |
| 以 `.` 开头的项目 | 自动隐藏，右键「取消隐藏」禁用（"以 . 开头的项目自动隐藏"）；`set_hidden(.., false)` 只清标记，仍按名字隐藏 |
| 带 Windows 隐藏 / 系统属性的文件 | 永不显示（不受「显示隐藏的图标」影响），移入时 `skipped`，删除收纳箱时一并移回 |
| 拖入点文件且没开「显示隐藏的图标」 | 照常移入，提示"N 个以 . 开头的项目已自动隐藏，可在「更多」中显示隐藏的图标"（与其他拖入提示合成一条） |
| 用户在 UAC / 管理员权限对话框点取消；在错误对话框选"跳过" | 逐项 `cancelled`（"用户取消了操作" / "已跳过"），文件留在原处；界面"已取消 N 项" |
| 文件被占用 | Shell 弹"文件正在使用"，用户选择前调用一直阻塞（后台线程）；取消 / 跳过 → `cancelled`，其他失败 → `failed` + 中文原因 |
| 拖入用户桌面 / 公共桌面文件夹本身或其上级 | 逐项 `failed`（"不能收纳桌面文件夹本身或其上级目录"） |
| 收纳文件夹被用户删掉 | 对账 / 启动时按原名重建（`warn` 日志），显示为空 |
| 文件已移动但写库失败 | `warn` 日志，逐项结果照常返回成功；下次对账补行（无原始路径） |
| 双击目标失效的 `.lnk` | 约 3 秒后 `Err("快捷方式指向的目标不存在，可能已被移动或卸载")` |
| 打开 / 定位不存在的路径 | `Err("文件不存在")` |
| 图标提取失败 | `Err`，界面占位图标 |
| 非 Windows：移动 / 打开 / 定位 / 图标 | `Err("桌面收纳箱仅支持 Windows")`；托盘不存在，`restore_all` 直接返回 |
| `GetShellWindow()` 为空（Explorer 未运行） | `attach_to_desktop` 返回 false 不挂；守护等 Explorer 回来再挂 |
| Explorer 重启（新 Progman） | 下一个 tick 内全部重挂，窗口不随旧 owner 销毁 |
| 收纳箱 ex style 丢了 TOOLWINDOW / 带上 APPWINDOW | 下一个 tick `lacks_tool_window_style` 命中，重挂（GPUI 下是防御） |
| 聚焦收纳箱按 Alt+F4 | `on_window_should_close` 返回 false，不关闭 |
| 目录监听启动失败 | `warn` 日志；收纳箱不再随资源管理器里的改动自动刷新（操作后的刷新照常） |
| 读不到显示器信息 | 屏幕组合 ID = `unknown` |
| 切到没有记录的屏幕组合 | 沿用当前位置并夹回可见区域，只记 `SESSION_RECTS`，不写库 |
| 托盘「重置收纳箱位置」时没有显示器 | `warn` 日志"重置收纳箱位置失败: 找不到可用的显示器"，什么都不做 |

### 5. Good / Base / Bad Cases

- **Good**：托盘「新增收纳箱」→ 光标所在屏右上角出现收纳箱、建出 `收纳箱` 文件夹、写入当前组合的布局；从桌面拖一个 `.lnk`
  进来 → 桌面图标消失、文件在收纳文件夹、`fence_items` 有原始路径；Win+D → 收纳箱一直可见；
  重启 Explorer → 第一次轮询内重挂；重启应用 → 位置、尺寸、标题、折叠 / 锁定、外观、图标大小、排序全部恢复。
- **Base**：锁定的收纳箱拖不动、没有缩放手柄，但仍可拖入、折叠、改名、打开；改名时文件夹被占用 → 标题照改并提示；
  删除时一个文件被占用、用户选"跳过" → 收纳箱保留、只剩该项、提示原因，释放后再删成功；切到没记录的屏幕组合 → 原位夹回、不写库，
  切回后回到原组合的记录。
- **Bad（不允许发生）**：有项目没移回却删掉了收纳箱记录，留下孤儿文件夹；对账在锁外列举，删掉了刚 upsert 的行；移入后用普通
  `INSERT` 撞 UNIQUE 丢了原始路径；标题栏用 `drag_region()` / 在 `mouse_down` 里直接发起拖动，双击改名失效；显示器变化引起的系统挪窗被落库，
  覆盖了用户摆好的位置；带 owner 的移动在 UI 主线程执行导致死锁；拖入"桌面"文件夹本身导致桌面被重定向到收纳文件夹；移回时覆盖了同名文件；
  用 `.on_drop::<ExternalPaths>()` 接拖入，改名回车后第一次拖入被丢掉。

### 6. Tests Required

**单元测试**（`pc-gpui/` 下 `cargo test --workspace`；release 专属断言用 `cargo test -p mini-todo-core --release`）：

| 位置 | 测试 | 断言点 |
|---|---|---|
| core `settings.rs` | `screen_config_id_matches_the_frontend_format` | 无显示器 `unknown`；按面积降序且与枚举顺序无关；同面积保持枚举顺序；`1.1 → 110`、`2.25 → 225` |
| | `deleting_a_screen_config_also_drops_fence_layouts_of_that_combination` | 删屏幕配置连带删布局、其他组合不受影响 |
| core `fence/mod.rs` | `first_fence_goes_to_the_top_right_of_the_work_area` / `new_fences_avoid_overlapping_existing_ones` / `a_full_work_area_falls_back_to_cascading_without_identical_corners` / `default_rect_respects_offset_monitors_and_tiny_work_areas` | 默认位置在工作区内、不重叠、放满后叠放且左上角不重合 |
| | `clamp_pulls_off_screen_rects_back_and_keeps_their_size` / `work_area_is_picked_by_overlap_then_by_distance` / `views_of_an_off_screen_fence_are_clamped_into_the_nearest_work_area` | 夹回可见区域、尺寸不变、工作区选取规则 |
| | `collapsed_view_is_just_the_title_bar` / `expanding_near_the_bottom_grows_upward_and_keeps_the_title_area_covered` / `title_bar_height_follows_the_target_monitor_scale` | 折叠高度按目标显示器缩放；向上展开仍覆盖标题栏原位置 |
| | `reset_layout_rows_from_the_top_right_and_stays_on_screen` / `default_title_takes_the_first_free_number` / `titles_are_trimmed_and_validated` / `colors_must_be_six_digit_hex` / `fence_labels_round_trip` | 重置排布、默认标题、校验 |
| | `insert_stores_the_folder_and_increasing_sort_order` / `update_only_touches_given_fields_and_validates_them` | patch 只改给出字段；任一字段非法全不写 |
| | `show_hidden_defaults_off_and_is_saved_per_fence` | 新建默认关闭；`UpdateFenceRequest.show_hidden` 只改它、按收纳箱独立、不给出时不动；JSON 为 `showHidden` |
| | `rename_normalizes_the_title_and_moves_the_folder` / `rename_keeps_the_old_folder_when_the_folder_cannot_be_renamed` / `rename_keeps_the_folder_while_files_are_moving_and_is_refused_while_deleting` | `folder_error` 语义与登记检查 |
| | `geometry_from_a_collapsed_view_keeps_the_expanded_height` / `saving_geometry_writes_the_current_screen_layout_and_the_latest_position` / `deleting_a_fence_removes_its_layouts` / `effective_geometry_prefers_screen_record_then_session_then_latest` / `switching_screens_uses_the_record_or_keeps_the_current_window_position` | 落库同时写两处；有效几何优先级；切换不写库 |
| | `delete_goes_ahead_only_when_the_folder_was_emptied_and_removed` / `delete_keeps_the_whole_fence_when_anything_is_left_behind` | `delete_fence == folder_removed`；只删已移回项的行；`failures` 含 failed + cancelled |
| core `fence/items.rs` | `first_reconcile_adds_rows_in_listing_order` / `reconcile_follows_saved_order_drops_missing_and_appends_new_items` / `case_only_rename_keeps_original_path_and_order` / `exact_match_wins_over_case_insensitive_match` | 对账规则 |
| | `upsert_fills_in_the_original_path_of_a_row_added_by_reconcile` / `originals_and_deletes_are_scoped_to_one_fence` / `reorder_ignores_unknown_names` | upsert 保留排序；按收纳箱隔离；不为未知名字建行 |
| | `hidden_flag_is_set_and_cleared_case_insensitively` / `reconcile_keeps_hidden_flags_through_case_only_renames` | 设置 / 清除隐藏标记（不区分大小写、按收纳箱隔离、返回列表同步）；重新对账保留标记，只改大小写保留、改成别的名字丢失；点文件 `auto_hidden` 但没有标记；找不到的项什么都不写 |
| | `moving_to_another_fence_does_not_carry_the_hidden_flag` / `set_hidden_checks_the_name_and_the_fence_before_touching_files` | 跨箱搬运原始路径、不搬运隐藏标记（目标已有同名隐藏行也清零）；`set_hidden` 文件名不合法 / 收纳箱不存在 / 正在删除时报错且不写行 |
| | `onedrive_notice_is_claimed_once_and_stays_device_local` | v29 默认 `'false'`；只返回一次 true；不在 `SYNCED_SETTING_KEYS`、导出的 `AppSettings` JSON 里没有它 |
| | `deleting_guard_blocks_reentry_and_releases_on_drop` / `moving_blocks_deletion_until_every_move_is_done` / `reconcile_skips_a_fence_that_is_being_deleted` | 登记互斥、drop 注销、删除中对账返回 `None` 且不补行 |
| | `affected_fences_match_folder_names_case_insensitively` / `sources_inside_another_fence_folder_are_recognized` | 监听匹配；跨箱来源识别（只认第二层） |
| core `window_logic.rs` | `tool_window_style_is_repaired_only_when_wiped` / `needs_reattach` 各例 | 自愈判定（与主窗口共用） |
| core `db/migrations.rs` | `v29_creates_fence_tables_with_contract_defaults` / `v29_deleting_a_fence_cascades_to_its_items_and_layouts` / `v29_fence_writes_do_not_count_as_sync_changes`；`LATEST_VERSION == 30` | 列默认值、UNIQUE、外键；`PRAGMA foreign_keys = 1` 后级联；写收纳箱表 `local_seq` 不变、墓碑为 0 |
| | `v30_adds_hidden_columns_defaulting_to_visible` / `v30_hidden_flags_do_not_count_as_sync_changes` | v29 旧库升级后已有项目可见、`show_hidden` 关闭，旧版本的显式列写法照常可用；写两列不计 `local_seq` |
| core `db/paths.rs` | `release_builds_ignore_the_dev_data_dir_variable` / `debug_builds_use_a_non_empty_dev_data_dir` | release 恒 None；空值不生效；相对路径转绝对 |
| core `services/fence_fs` | `plan_move_in_validates_sources` | 预去重 `b (2).lnk` / `b (3).lnk`；`protected` 的桌面文件夹本身 `failed`；根目录及上级、别的收纳箱文件夹本身 `failed`；跨箱项允许 |
| | `restore_dir_selection` / `plan_restore_picks_dir_and_name` / `restore_all_keeps_folder_when_items_remain` / `move_in_results_keep_input_order` / `classify_*` | 移回目录与名字；剩余项时保留文件夹；结果顺序；取消 / 失败归类 |
| | `list_items_filters_and_describes` / `dot_names_are_auto_hidden` | 列举跳过 `desktop.ini` 与带隐藏属性的文件（Windows），点文件 / 点文件夹所有平台都列出且 `auto_hidden`，`hidden` 一律 false；点文件判定 |
| | 命名（`sanitize_*`、`folder_name_validation_rejects_traversal`、`item_name_validation`、`unique_file_name_adds_suffix_before_extension`、`restore_name_undoes_collision_suffix`）、图标（`bucket_rounds_up`、`cache_key_is_stable_and_case_insensitive`、`strips_shell_frame_around_small_icons`、`prune_cache`）、监听（`watcher_reports_debounced_folder_changes`）、`release_build_ignores_override`（release） | |
| app `windows/fence/logic.rs` | `titles_follow_backend_rules` / `drag_threshold` / `geometry_gate_vectors` / `resize_vectors` / `icon_size_vectors` / `move_in_messages` / `delete_kept_messages` / `delete_hint_text` 等 | 测试向量来自旧前端 `fence.test.ts`：标题与后端同规则；闸门未 arm 的移动不算数、拖动中续期、超时 / `disarm` 后失效；缩放到最小尺寸不推动对边；图标档位与后端一致；提示文案 |
| | `visible_items_follow_show_hidden` / `sorting_merges_hidden_items_back_in_place` / `empty_hint_text` / `move_in_mentions_auto_hidden_dot_files` | 按 `show_hidden` 过滤（用户隐藏 + 点文件）；排序后隐藏项留在原位、显示项按新顺序填回；空状态"N 个项目已隐藏"；点文件自动隐藏提示（按移入后的名字计数、开着显示隐藏时不提示、与其他提示合成一条） |
| app `windows/fence/panels.rs` | `title_menu_width_follows_the_longest_label` | 「更多」菜单切换项文案与估算宽度（123 → 151 / 165） |
| app `platform/mod.rs` | `fence_geometry_is_the_visible_client_area` | 库里的几何 = 客户区 |

**E2E**（Windows，`pc-gpui/scripts/e2e-windows/run_e2e.py` 的 F 组；CI `.github/workflows/pc-gpui-e2e-windows.yml`，见其 README 与
[desktop-window-e2e-guide](../guides/desktop-window-e2e-guide.md)）：隔离运行（`MINI_TODO_DATA_DIR` + `MINI_TODO_FENCES_ROOT`）。

| 用例 | 断言 |
|---|---|
| F01 `fence:new` | 嵌入桌面（3.1 状态表，含无 `WS_SYSMENU`、DWM 标题栏按钮区宽度为 0）、收纳文件夹建在 `MINI_TODO_FENCES_ROOT` 下 |
| F02 往收纳文件夹放文件 | 监听刷新出图标 |
| F04 / F09 / F08 真实 OLE 拖入（.NET `DataObject`、Shell 数据对象、资源管理器窗口作拖放源） | 新实例第一次拖入即生效；文件离开来源、进入收纳文件夹 |
| F03 / F10 / F11 双击标题改名；改名后 / 单击标题后紧接着的拖入 | 文件夹同步改名；第一次拖入不丢 |
| F07 几何 | 可见区域 = 数据库记录的物理像素几何 |
| F05 / F06 | Win+D 后仍可见；Explorer 重启后自动重挂 |

仅能人工验证：公共桌面拖入点 UAC 取消；删除时占用一个文件选"跳过"；混合 DPI 下的折叠高度；OneDrive 重定向桌面的提示。

### 7. Wrong vs Correct

#### Wrong — 不管还原结果如何都删收纳箱

```rust
let restored = fence_fs::restore_all(&folder, &originals, owner)?;
delete_fence_row(db, id)?;   // 有项目移不回去时留下没有收纳箱管理的孤儿文件夹
```

#### Correct — 只有文件夹已清空并删除才删

```rust
let plan = plan_delete(&restored); // delete_fence = restored.folder_removed
if plan.delete_fence {
    delete_fence_row(db, id)?;     // fence_items / fence_layouts 级联；app 随后 close_deleted 关窗
} else {
    // 删掉已移回项的行 → drop(deleting) → items::refresh，返回 deleted = false，收纳箱整个保留
}
```

#### Wrong — 把过滤后的显示列表当完整顺序保存

```rust
// sorting 只含显示中的项（没开「显示隐藏的图标」时不含隐藏项）
let names: Vec<String> = sorted.iter().map(|i| i.file_name.clone()).collect();
items::reorder(db, id, &names);  // 隐藏项不在 names 里，保留旧 sort_order，与新编号撞号，下次对账顺序错乱
```

#### Correct — 先把隐藏项按原位置并回完整顺序

```rust
let list = logic::merge_sorted(&self.items, &sorted, |i: &FenceEntry| i.file_name.as_str());
let names: Vec<String> = list.iter().map(|i| i.file_name.clone()).collect(); // 完整顺序（含隐藏项）
// 顺序没变也要 refresh_items：拖动期间 replace_items 丢掉的推送（目录监听、隐藏结果）靠这次补列举找回
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

#### Wrong — 带 owner HWND 的移动在 UI 主线程执行

```rust
.on_click(cx.listener(move |this, _, window, cx| {
    let owner = platform::hwnd(window).map(|h| h.0);
    // 主线程阻塞在 IFileOperation 上；管理员权限对话框以收纳箱窗口为父，给主线程同步发消息 → 死锁
    let _ = items::restore(&Db::get(cx), id, &file_name, owner);
}))
```

#### Correct — 前台取 HWND，移动放后台，结果回前台

```rust
let owner = platform::hwnd(window).map(|h| h.0);          // isize，可以跨线程
let task = spawn_db(cx, move |db| items::restore(db, id, &file_name, owner)); // 后台线程；core 内部先登记再读 folder_name
cx.spawn_in(window, async move |this, cx| { let result = task.await; /* this.update_in(..) 提示 / 刷新 */ }).detach();
```

#### Wrong — 窗口一变就落库 / 标题栏用拖动区

```rust
cx.observe_window_bounds(window, |this, window, cx| this.commit_geometry(window, cx).detach()); // 显示器变化的系统挪窗也被落库
div().child(ui::title_bar::drag_region())  // 收纳箱标题栏：双击改名收不到
```

#### Correct — 超过阈值才拖动，闸门之内的移动才落库

```rust
fn on_title_mouse_move(&mut self, event: &MouseMoveEvent, window: &mut Window, cx: &mut Context<Self>) {
    let Some(start) = self.press_point else { return };
    if event.pressed_button != Some(MouseButton::Left) { self.press_point = None; return; }
    if !logic::exceeds_drag_threshold(dx, dy, 3.) { return; }
    self.press_point = None;
    if self.locked() || self.editing_title { return; }
    self.gate.arm(now_ms());
    platform::begin_window_drag(window, cx);
}
fn on_geometry_event(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    if !self.gate.accept(now_ms()) { return; }      // 用户没发起拖动：忽略
    /* 停下 GEOMETRY_COMMIT_DEBOUNCE（300ms）后 commit_geometry */
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

### Design Decision: 始终嵌入，「固定」只是界面锁定

**Decision**：收纳箱复用主窗口的嵌入方案（owner=Progman + `FLAG_BOTTOM` 永远置底 + `WS_EX_TOOLWINDOW`，`platform::embed`），
建窗参数一次给齐、之后不改；窗口建成 `resizable: false`，缩放用窗口内手柄 + `SetWindowPos`。锁定 / 解锁、折叠 / 展开都不碰窗口样式。
旧版实测多个收纳箱重叠时点击不会成组重排（被点的升到组内最上，其余相对顺序不变）。

### Design Decision: 收纳箱数据是设备本地数据

路径与屏幕坐标只对这台设备有意义，性质同 `screen_configs`：三张表与 `fence_onedrive_notice_shown` 都不进导出 / WebDAV 同步、不接
变更计数与墓碑（见 3.3 与 [sync-protocol.md §3.6](./sync-protocol.md)）。

---

## 已知限制

- **Win10 / Win11 ≤ 23H2**：显示桌面时抬起的是 `WorkerW` 而不是 Progman，Win+D 可能盖住收纳箱（与主窗口嵌入桌面相同的限制，
  见 [window-modes.md](./window-modes.md)）。实测环境为 Win11 24H2 之后的版本。
- **无法自动化的路径**：公共桌面项的 UAC 取消、删除时文件被占用（"文件正在使用"对话框）只能人工验证；判定逻辑有单测。
- 收纳的图片 / 视频显示类型图标而不是缩略图（`SIIGBF_ICONONLY`）；带 Windows 隐藏 / 系统属性的文件永不显示（「显示隐藏的图标」也不显示）、
  不计入删除确认的 N，但删除时会一并移回。
- 收纳箱里的「隐藏」只是本应用的标记（不改文件属性）：资源管理器里改名（只改大小写除外）后成了新项目，隐藏标记丢失；装回旧版本
  （3.0.0 等，没有 v30 迁移）时隐藏不生效（全部显示，点文件在 Windows 上也会显示）。以 `.` 开头的项目固定自动隐藏，没有开关。
- Explorer 重启后某个收纳箱的 Z 序可能落在隐身的 Shell CoreWindow 之上（仍在所有普通应用窗口之下），不影响可见行为（旧版实测）。
- GPUI 窗口外沿那圈不可见边框（96 DPI 下左 / 右 / 下各 8px）仍会接住鼠标（`HTBORDER`），见 gpui-conventions §2.2。
