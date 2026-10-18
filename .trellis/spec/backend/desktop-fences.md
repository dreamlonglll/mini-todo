# 桌面收纳箱（Desktop Fences）

> 收纳箱的 code-spec。实现：
> - core（`pc-gpui/crates/core/src/`）：`fence/mod.rs`（元数据、几何、按屏幕组合记忆位置、删除）、`fence/items.rs`（内容对账、移入 /
>   移回 / 跨箱、打开、图标、目录监听、`FenceActivity` 登记）、`services/fence_fs/`（文件层，与数据库无关；Windows 走 `shell.rs` / `sta.rs`，
>   macOS 走叶子模块 `mac.rs`）；
> - app（`pc-gpui/crates/app/src/`）：`services/fences.rs`（建窗与嵌入 / mac 桌面层、启动恢复、托盘 / 菜单栏图标三个入口、监听结果回前台、
>   200ms 嵌入守护（Windows）、约 2 秒一次的屏幕组合检测、吸附高亮的跨窗口状态）、`windows/fence/`（`mod.rs` 标题栏 / 折叠 / 移动 / 缩放 / 改名 / 外部拖入，
>   `grid.rs` 项目网格，`panels.rs` 外观 / 删除确认 / 云同步（OneDrive / iCloud）面板，`logic.rs` 纯函数，`snap.rs` 吸附对齐纯函数）、`platform/embed.rs`
>   （Windows：与主窗口共用的嵌入核心）、`platform/macos/ns.rs`（macOS：`make_desktop_layer`、取色面板）、`platform/fence_win.rs`（「自定义颜色」）。
>
> 建立于任务 `10-15-desktop-fence`（2026-10-15，旧 Tauri 版，Win11 10.0.26300、双屏混合 DPI 150% + 125% 上 e2e 实测），随
> `10-15-gpui-rewrite` 移植到 GPUI；旧 Tauri 版已删除（见 git 历史）。主窗口的三种模式见 [window-modes.md](./window-modes.md)，本文件只讲收纳箱。
> 文件层逐函数说明见归档任务文档 `.trellis/tasks/archive/2026-10/10-15-desktop-fence/fence-fs-api.md`。
> macOS 移植于任务 `10-16-macos-fences`（2026-10-16；**只在 GitHub macOS runner 上验证，未经真机验收**），mac 的差异集中在后文
> 「场景：macOS 上的收纳箱」，调研资料在该任务的 `research/`（桌面层窗口、文件操作与 TCC）。
>
> **几何一律是客户区（= 可见区域）的物理像素**：GPUI 窗口在左 / 右 / 下另有不可见边框（96 DPI 下各 8px），摆窗用
> `platform::set_client_rect_physical` / `Hwnd::set_client_rect`，落库 / 缩放起点 / 屏幕组合切换读 `client_rect_physical`
> （旧版无边框窗口的外框就是客户区，库里的几何沿用它；e2e F07 断言可见区域 = 库里的几何）。macOS 的窗口 frame 就是客户区，
> 物理像素 = 全局点坐标 × 统一系数（[pc-gpui/gpui-conventions.md §2.15](../pc-gpui/gpui-conventions.md)；mac e2e MF01 / MF09 断言库里的几何 =
> 窗口 × 统一系数）。

---

## 场景：桌面上的真收纳区域（窗口 + 文件 + 屏幕组合布局）

### 1. Scope / Trigger

- 触发：一种特殊窗口（始终在桌面层的透明窗口：Windows 嵌入桌面，macOS 桌面图标层之上的不激活面板）、迁移 v29（3 张表 + 1 个 settings 键）
  与 v30（隐藏图标的两列）、Win32 嵌入与置底 / AppKit 窗口层级、会移动用户文件的 Shell / `renamex_np` 操作（硬约束：**不丢文件、绝不覆盖**）、
  一组 debug 专用环境变量（§3.6）。
- 改动以下任何东西前先读本文件：
  - core `fence/**`、`services/fence_fs/**`、`settings::delete_screen_config`、`db/paths.rs` 的隔离变量、`fences` / `fence_items` /
    `fence_layouts` 表
  - app `services/fences.rs`、`windows/fence/**`、`platform/embed.rs`（主窗口也用）、`platform/macos/ns.rs` 的桌面层 / 取色面板、
    `platform/fence_win.rs`、托盘 / 菜单栏图标的收纳箱菜单项（`services/tray.rs`）、`services/mac_menu.rs` 的 ⌘M / ⌘W、
    `scripts/macos/bundle.sh` 的隐私用途说明键
- 平台：托盘 / 菜单栏图标的收纳箱三项（`tray.rs`，Windows 与 macOS 都编译）与启动恢复（`services::fences::restore_all`）在 Windows 与 macOS 上；
  core 的移动 / 打开 / 定位 / 图标在 Windows（Shell）与 macOS（`fence_fs/mac.rs`）上可用，其他平台返回 `Err("桌面收纳箱仅支持 Windows 与 macOS")`。
  Linux 上收纳箱窗口只用于截图 / 冒烟（`MINI_TODO_DEBUG_OPEN=fence:<id>` / `fence:new`），启动不恢复、`apply_window_rect` 只改尺寸。
  §3 ~ §7 以 Windows 为准写成；macOS 与之不同的地方见「场景：macOS 上的收纳箱」，两处冲突时以 mac 场景为准（只对 mac）。

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
pub fn fence_root_dir() -> Result<PathBuf, String>;          // 不存在则创建；macOS 返回 canonicalize 后的路径
pub fn icon_cache_dir() -> PathBuf;                          // <app_data_dir>\fence-icons，不创建
pub fn user_desktop_dir() -> Option<PathBuf>;                // dirs::desktop_dir()；debug 下 MINI_TODO_DESKTOP_DIR 覆盖（公共桌面 public_desktop_dir 只在模块内用）
pub const DESKTOP_DIR_ENV: &str = "MINI_TODO_DESKTOP_DIR";   // location.rs；FENCES_ROOT_ENV 同理
pub fn is_ubiquitous_item(path: &Path) -> bool;             // macOS：移动前判定 iCloud 管理（NSURLIsUbiquitousItemKey）；其他平台恒 false
pub fn path_starts_with_ci(path: &Path, prefix: &Path) -> bool;  pub fn same_path_ci(a: &Path, b: &Path) -> bool;
pub fn is_valid_item_name(name: &str) -> bool;               // 单个路径段：拦 ..、绝对路径、ADS。Windows 规则拦 <>:"/\|?* 与控制字符；
                                                             // macOS / Linux 只拦空、. / ..、/ 与 NUL（其余字符在那里是合法文件名）
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
// 内部（naming.rs / ops.rs）：系统元数据文件表 SHELL_METADATA_FILES、包扩展名表 PACKAGE_EXTENSIONS + is_package_name(name)；
// pub(crate) struct Reserved { root, list_outside_root, taken }：Reserved::new(root)（macOS 不列举收纳根目录以外的目标目录）/ with_policy(..)

// ===== core services::fence_fs::mac（cfg(target_os = "macos")；叶子模块，不 use super:: / crate::，scripts/mac-check 原样编译） =====
pub struct MoveResult { pub final_path: Option<PathBuf>, pub errno: Option<i32>, pub message: Option<String>, pub source_left: bool }
pub fn move_item(source: &Path, dest_dir: &Path, first_name: &str, next_name: &dyn Fn(u32) -> String) -> MoveResult; // renamex_np(RENAME_EXCL)
pub const MAX_NAME_ATTEMPTS: u32 = 64;
pub fn describe_errno(source: &Path, dest_dir: &Path, errno: i32) -> String;
pub fn has_hidden_flag(meta: &Metadata) -> bool;  pub fn is_locked(path: &Path) -> bool;   // UF_HIDDEN / uchg、schg、仅追加
pub fn has_legacy_icloud_stub(path: &Path) -> bool;  pub fn is_ubiquitous_item(path: &Path) -> bool;
pub fn allow_materialize_on_this_thread() -> c_int;  pub fn restore_materialize_policy(old: c_int);  // dataless 文件按需下载
pub fn icon_png(path: &Path, px: u32) -> Result<Vec<u8>, String>;   // 恰好 px × px（夹到 16..=1024）的 PNG 字节
pub fn launch(path: &Path) -> Result<(), String>;  pub fn reveal(path: &Path) -> Result<(), String>;

// ===== app services::fences（crates/app/src/services/fences.rs） =====
pub struct FenceRuntime { hidden: bool, windows: HashMap<i64, FenceWindowRecord { handle, view, hwnd }>, watcher: Option<FenceWatcher>, guard_started: bool,
                          snap_highlights: HashMap<i64, SnapEdges>, snap_owner: Option<i64> } // Global
pub fn snap_scene(cx: &App, exclude: i64) -> SnapScene;          // 拖动 / 缩放开始时：其它显示中的收纳箱的可见客户区 + 各显示器（"全部隐藏"中为空）
pub fn set_snap_guides(cx: &mut App, owner: i64, guides: &[SnapGuide]); // 每帧整体替换高亮，只通知变了的目标窗口（App::notify(entity_id)）
pub fn clear_snap_guides(cx: &mut App, owner: i64);              // 拖动 / 缩放结束；窗口关闭 / 删除时 forget_snap 也会清
pub fn snap_edges(cx: &App, fence_id: i64) -> SnapEdges;         // 目标窗口渲染时读
pub fn open_fence_window(cx, fence: Fence, rect: PxRect, scale: f64) -> Option<AnyWindowHandle>; // 已存在则直接返回
pub fn restore_all(cx);            // 启动恢复（Windows 与 macOS；Linux 直接返回）
pub fn create_fence(cx);           // 托盘 / 菜单栏图标「新增收纳箱」/ MINI_TODO_DEBUG_OPEN=fence:new
pub fn toggle_visibility(cx) -> bool;  pub fn reset_positions(cx);   // 托盘「显示/隐藏所有收纳箱」「重置收纳箱位置」
pub fn ensure_watcher(cx);  pub fn deliver_refresh(cx, FenceRefresh);  pub fn close_deleted(cx, fence_id);
pub fn apply_window_rect(window, cx, rect: PxRect, scale: f64);   // Windows：按客户区一次 SetWindowPos；macOS setFrame:（都延后执行）；Linux 只改尺寸
fn embed_after_open(cx, hwnd: Option<Hwnd>, rect);               // 建窗后：Windows 嵌入桌面 / macOS make_desktop_layer → 显示（"全部隐藏"中不显示）

// ===== app platform（macOS 收纳箱专用；Windows / Linux 为 no-op） =====
impl Hwnd { pub fn make_desktop_layer(self) -> bool; pub fn describe_layer(self) -> Option<String> }
// platform/macos/ns.rs：desktop_layer_level() -> NSWindowLevel（CGWindowLevelForKey(kCGDesktopIconWindowLevelKey) + 1）、
// make_desktop_layer(number) -> bool、describe_window(number) -> Option<String>、
// show_color_panel(initial) -> bool、color_panel_state() -> Option<ColorPanelState { visible, color }>
pub fn fence_win::choose_color(cx, owner: Option<Hwnd>, initial: (u8, u8, u8)) -> Task<Option<(u8, u8, u8)>>; // Windows ChooseColorW / macOS NSColorPanel / Linux None
pub fn monitor_areas(cx) -> Vec<MonitorArea>;                    // 主显示器排第一；取不到工作区时用整块显示器
const GUARD_TICK: Duration = 200ms;  const SCREEN_CHECK_TICKS: u32 = 10;

// ===== app windows::fence（crates/app/src/windows/fence/） =====
pub fn fence_key(fence_id: i64) -> String;     // = core fence_label
// logic.rs（纯函数，单测向量来自旧前端 fence.test.ts）
pub fn normalize_fence_title(raw) -> Option<String>;  pub fn exceeds_drag_threshold(dx, dy, threshold) -> bool;
pub struct GeometryCommitGate { arm(now), accept(now) -> bool, is_armed(now) -> bool, disarm() } // "用户正在拖动"；accept 只剩单测用
pub fn resize_rect(..) -> PxRect;  impl ResizeDir { pub fn has_n / has_s / has_w / has_e }
pub fn icon_bucket(..);  pub fn icon_request_px(..);  pub fn clamp_icon_size(..);  pub fn next_icon_size(..);
pub fn summarize_op_results(..) -> OpSummary { moved, cancelled, failed, notes, auto_hidden };
pub fn describe_move_in(&OpSummary, show_hidden: bool);  pub fn describe_delete_kept(..);  pub fn delete_hint(..);
pub fn visible_items(&[FenceEntry], show_hidden) -> Vec<&FenceEntry>;    // 网格显示哪些项
pub fn merge_sorted<T>(full: &[T], sorted_visible: &[T], key) -> Vec<T>;  // 排序结束：隐藏项按原位置并回完整顺序
pub fn empty_hint(total_items: usize) -> String;                         // "把桌面图标拖到这里" / "N 个项目已隐藏"
pub const ICON_BUCKETS: [u32; 8] = [16, 24, 32, 48, 64, 96, 128, 256];  pub const ICON_SIZE_STEP: i32 = 8;
pub mod timing { HOVER_EXPAND_DELAY 150ms, HOVER_COLLAPSE_DELAY 400ms, DRAG_COLLAPSE_DELAY 300ms, GEOMETRY_ARM_MS 3000,
                 GEOMETRY_COMMIT_DELAY 16ms, MAX_ICON_REQUESTS 3, .. }
// snap.rs（吸附对齐纯函数，不依赖 GPUI；单测向量 = PRD 10-18-fence-snap 的验收用例）
pub const SNAP_THRESHOLD_DIP: f64 = 10.;  pub fn snap_threshold(scale: f64) -> i32;   // round(10 × 系数) 物理像素
pub struct SnapTarget { fence_id: i64, rect: PxRect }   pub struct SnapMonitor { rect: PxRect, work: PxRect }
pub struct SnapScene { targets: Vec<SnapTarget> /* 按 fence_id 排序 */, monitors: Vec<SnapMonitor> }
impl SnapScene { pub fn targets_at(&self, cursor: Option<(i32, i32)>) -> Vec<SnapTarget> }  // 光标所在显示器上的目标
pub enum SnapEdge { Left, Right, Top, Bottom }  pub struct SnapGuide { fence_id, edge }  pub struct SnapEdges(u8) { insert, contains, is_empty }
pub fn snap_move(rect, targets, threshold) -> (PxRect, Vec<SnapGuide>);
pub fn snap_resize(rect /* resize_rect 的结果 */, dir: ResizeDir, targets, threshold, min_w, min_h) -> (PxRect, Vec<SnapGuide>);
pub fn group_guides(&[SnapGuide]) -> HashMap<i64, SnapEdges>;  pub fn changed_fences(old, new) -> Vec<i64>;
// theme.rs：fence_tokens().snap_guide = #409EFF；FENCE_SNAP_GUIDE_WIDTH = 2px
```

core 与 app 必须成对的常量：`logic::ICON_BUCKETS` ↔ `fence_fs::ICON_SIZE_BUCKETS`；`normalize_fence_title` ↔ core `normalize_title`；
标题栏高度、最小尺寸、图标大小范围 app 直接用 core 的常量（`FENCE_TITLE_BAR_DIP`、`MIN_WIDTH_DIP`、`MIN_ICON_SIZE` / `MAX_ICON_SIZE`）。

### 3. Contracts

#### 3.1 收纳箱窗口

**标识**：`WindowKind::Fence`，`WindowManager` 登记 key = `fence-{id}`（`id = fences.id`，`is_fence_label` 只认 `fence-` 后接纯数字）；
运行时记录在 `services::fences::FenceRuntime.windows`。

**建窗**（`open_fence_window`，已有同 id 窗口时直接返回它）：

1. 建窗与所有会给窗口同步发消息的 Win32 调用都不在 App 借用期间执行（[pc-gpui/gpui-conventions.md §2.2](../pc-gpui/gpui-conventions.md)）：
   托盘回调只把事件投进 channel，由前台任务执行；嵌入（macOS：`setFrame:` 与改层级）在 `cx.spawn` 的 async 块里做。
2. `WindowSpec` 参数**一次给齐**：

   | 参数 | 值 | 原因 |
   |---|---|---|
   | `bounds` / `client_rect` | 逻辑 = 物理 ÷ 目标显示器缩放；`client_rect` 给精确的物理客户区 | `windows::open_window` 建好后按客户区校正一次（混合 DPI 下逻辑坐标差几个像素） |
   | `transparent` | true | 无系统标题栏的透明窗口，界面自己画圆角底色；DWM 圆角另外 `set_round_corners(true)` |
   | `resizable` | false | GPUI 的缩放热区按建窗时的 `is_resizable` 计算：不可缩放、不参与 Aero Snap；缩放走窗口内手柄（见下） |
   | `minimizable` | false | Win+M 不最小化它 |
   | `focus` | false | 建窗不抢焦点 |
   | `show` | Windows / macOS 上 false | 先藏着建好，定位、改样式、嵌入之后再 `SW_SHOWNOACTIVATE` 显示，不闪任务栏（macOS：改好层级再 `orderFrontRegardless`，不在 `PopUp` 默认的 101 层闪一下） |
   | `kind` | 默认 `Normal`；**macOS 上 `PopUp`** | macOS 是不激活应用的 NSPanel，见「场景：macOS 上的收纳箱」 |
   | `min_size` | 160 × 32（逻辑） | 折叠态只有标题栏高；展开态最小高度由界面逻辑保证 |
   | `display_id` | 矩形所在显示器 | |

3. 关闭：`window.on_window_should_close(cx, |_, _| false)`——没有关闭按钮，Alt+F4 / `WM_CLOSE` 一律拦截。删除收纳箱走
   `services::fences::close_deleted` → `WindowManager::close`；窗口关闭回调里从 `FenceRuntime.windows` 注销，最后一个关闭时 `forget_screen_config`。
4. 建好后（Windows，`embed_after_open`，async 块里；macOS 分支见 mac 场景 §M1）：`set_client_rect` → `set_tool_window(true)` → `set_minimizable(false)` →
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
| 摆窗（`services::fences::apply_window_rect` / `platform::set_client_rect_physical`） | 按客户区一次 `SetWindowPos(NOZORDER \| NOACTIVATE)`（延后到下一个前台任务）；跨 DPI 时按新边框再校正。macOS `setFrame:display:`（同样延后）；Linux 只改尺寸 |
| 改标题（改名后） | `window.set_window_title` |
| 显示 / 隐藏全部 | `Hwnd::show` / `hide`（`ShowWindow`，不改 ex style）；显示后再 `attach_to_desktop` + `set_always_on_bottom`（幂等）。macOS：显示前再 `make_desktop_layer`（幂等）→ `orderFrontRegardless`，隐藏 `orderOut:` |
| 改 resizable / minimizable / 加 `WS_EX_NOACTIVATE` / `WS_EX_LAYERED` / 加回 `WS_SYSMENU` | **禁止** |
| macOS：`set_topmost` / `ns::set_floating` / `setFloatingPanel:` | **禁止**（会把层级改回 0 / 3） |
| macOS：激活应用（`cx.activate(true)` / `bring_to_front`） | 只有双击标题改名时 `cx.activate(true)` + `window.activate_window()`（mac 场景 §M1「改名」）；单击、拖动、缩放、外部拖入、打开项目等其他操作**禁止**激活应用 |

**锁定（「固定」）完全由界面控制**：锁定时不发起拖动、不显示缩放手柄；仍可拖入、折叠、改名、打开项目。Win32 样式不随锁定变化。

**移动（自绘拖动，任务 `10-18-fence-snap` 起；两平台一份实现）**：不进系统的拖动循环——Windows `WM_NCLBUTTONDOWN(HTCAPTION)` /
macOS `performWindowDragWithEvent:` 到松手前都改不了位置（macOS 没有任何钩子），吸附做不了。收纳箱**不调** `platform::begin_window_drag`，
标题栏也**不用** `ui::title_bar::drag_region()`（`HTCAPTION` 区域双击会触发系统行为）。与缩放共用"屏幕物理坐标 + 每帧摆窗 + 松手落库"：

1. 标题栏 `on_mouse_down` 只记下按下点（窗口逻辑坐标）与光标（`cursor_position_physical`；Linux 取不到，用窗口逻辑坐标 + 窗口原点），
   **不立即拖**（双击改名要收得到两次单击）。改名中不记。客户区起点在进入移动会话时才读：按下到超过阈值之间窗口只可能被程序调整
   （折叠的收纳箱悬停展开、可能向上展开），用按下时的起点会把它摆回去。
2. 标题栏挂 GPUI `on_drag(TitleDrag)` + `on_drag_move::<TitleDrag>`（只在 `can_move()` = 未锁定、不在改名时挂；改名输入框里拖选文字不受影响）。
   `on_drag_move` 是捕获阶段、不看悬停的监听：Windows 上 GPUI 按下时已 `SetCapture`（gpui-pre-windows `events.rs`），macOS 的 mouseDragged
   一直发给按下的窗口，光标出了窗口也收得到。GPUI 拖动的两个副作用：
   - 移动超过 2px 就开始拖动、这次按下不再算单击——所以**双击改名不用 `on_click`**：按下时记 `click_count >= 2`（上一次按下拖动过收纳箱的不算：
     窗口跟着光标走，拖完紧接着的单击会被系统算成双击的第二下），标题栏松手（`on_mouse_up`，光标在标题栏上）时离按下点不超过 3 逻辑像素就改名，
     与"超过 3px 才拖"同一个阈值（`TITLE_DRAG_THRESHOLD`）。
   - 拖动期间所有元素都不算悬停：一开始就会报"离开"、松手后再报"悬停"。按住标题栏 / 缩放手柄的手势（`pointer_session()`）中的"离开"不处理
     （标题栏按钮不变暗、悬停展开的不收回；松手后的"悬停"也就不是一次新的进入，刚拖完的折叠收纳箱不会在 150ms 后展开）；松手后等最后一帧落地
     再按光标核对一次，光标真在窗口外才补上离开（`resync_hover_after_drag`）。
   - 另外 GPUI 的 tooltip 不看拖动（光标在标题上停够 500ms 就弹，窗口跟着光标走、光标一直在标题上）：手势进行中标题不挂 tooltip。
3. 移动超过 3 逻辑像素（`exceeds_drag_threshold`，按窗口坐标判断，这时窗口还没动）才进入移动会话：读客户区起点 → 先 `flush_geometry`（上一次刚松手、
   还没落库的先落库）→ 快照吸附目标（`snap_scene`）→ `gate.arm(now)`。之后每次移动：新位置 = 起点 + 光标相对按下时的位移（窗口一开始就追上光标，
   抓住的点不变）→ 吸附（见「吸附」）→ `gate.arm` 续期 →
   每帧最多摆一次窗（`on_next_frame`，只保留最新一帧）。**移动只改位置**（`platform::set_client_position_physical`），尺寸取窗口当前值：混合 DPI
   下跨屏时 `WM_DPICHANGED` 改了尺寸，不能用起点的尺寸摆回去。
4. 松手（标题栏的 `on_mouse_up` + `on_mouse_up_out`，在不在标题栏上都收）→ 摆完最后一帧、清高亮 → 等 `GEOMETRY_COMMIT_DELAY`（16ms，摆窗是延后执行的）
   后直接 `commit_geometry`，落库完成后 `disarm`。
5. 没收到松手（Windows 上鼠标捕获被别的窗口拿走、macOS 的菜单吞了 mouseUp）：之后不带左键的 `TitleDrag` 移动（再 `cx.stop_active_drag`）、
   GPUI 的拖动已被别的窗口里的松手清掉（`active_drag` 是全应用的）时光标回到窗口上的第一次不带左键的移动（根元素 `on_mouse_move`）、
   或冒泡到窗口根元素的新按下（根元素 `on_any_mouse_down`）都按松手收尾（落库 + 清高亮），不会卡在拖动中。缩放同理。

拖动 / 缩放中（含松手后落库完成前，`geometry_busy()` = 移动或缩放会话进行中、或闸门开着）悬停**不展开也不收回**；按住不动超过 3 秒也算（看会话，
不只看闸门）。

**缩放**：窗口是 `resizable: false`，系统不进缩放循环。8 个手柄（n/s/w/e/nw/ne/sw/se）只在 `can_resize()`（未锁定、未折叠、
不在改名）时显示：按下 → 先 `flush_geometry` → 记客户区起点与光标（同移动，拖左 / 上边时窗口在动，所以用屏幕物理坐标）→ 快照吸附目标；
移动 → `resize_rect`（拖哪条边改哪条边，对边不动，不小于 `round(160 × 系数)`，系数 = `platform::physical_scale(window)`：Windows / Linux 是
`scale_factor()`，macOS 是统一系数）→ 吸附（`snap_resize`）→ 每帧最多一次 `apply_window_rect`（`on_next_frame`，只保留最新一帧）；
松开 → 最后一帧落地后（16ms）`commit_geometry`。

**吸附（任务 `10-18-fence-snap`；纯逻辑 `windows/fence/snap.rs`，跨窗口高亮在 `services::fences`）**：

| 项 | 规则 |
|---|---|
| 阈值 | `snap_threshold(physical_scale)` = `round(10 × 系数)` 物理像素（100% 下 ±10px，150% 下 ±15px）；移动按窗口当前的系数（混合 DPI 下跨屏会变），缩放按开始时的（与最小尺寸同一个） |
| 目标 | 除自己以外、当前显示中的收纳箱（含锁定的），取它们**屏幕上的可见客户区**（`Hwnd::client_rect`，折叠的只有标题栏；Windows / macOS 都是物理像素口径；"全部隐藏"中没有目标）。拖动 / 缩放开始时快照一次（`snap_scene`，拖动期间其它收纳箱不会动），按 `fence_id` 排序；每次移动只取光标所在显示器上的（`SnapScene::targets_at`：目标属于交叠面积最大的工作区 `pick_work_area`，与光标所在显示器的工作区相同才算；取不到光标 / 光标不在任何显示器上时不过滤）。Linux 没有原生句柄，没有目标 |
| 方向 | 横向 / 纵向独立：各取距离最近且 ≤ 阈值的候选，可以同时吸 x 和 y；同距离取目标列表里靠前的（确定性） |
| 移动 | 同名边对齐 left↔left、right↔right、top↔top、bottom↔bottom；只平移、尺寸不变 |
| 缩放 | 在 `resize_rect` 之后；只调被拖的边、对边不动。候选 = 同名边对齐（E→right、W→left、N→top、S→bottom）+ 等宽（拖 E / W）/ 等高（拖 N / S），放在一起取最近；吸附后小于最小尺寸的候选跳过 |
| 不做 | 相邻贴边（我的 left 吸别人的 right）、屏幕工作区边缘、中心线、网格；不加设置项 |
| Alt | 按住 Alt（macOS Option，GPUI `modifiers.alt`）时完全不吸附、不出高亮；按每次移动事件当时的修饰键判断，松开后下一次移动恢复。Windows 上单按 Alt 松开不会进菜单模式（收纳箱没有 `WS_SYSMENU`，GPUI 处理 `WM_SYSKEYUP` 返回 0） |
| 高亮 | 画在**目标**收纳箱上：同名边对齐 → 目标的那条边；等宽 → 目标的上、下两边；等高 → 目标的左、右两边。吸附生效的方向上，所有与结果恰好成立的关系都亮（两个目标 left 相同，吸上后两个都亮）。被拖的收纳箱自己不画 |
| 高亮的样子 | 目标窗口根元素里一个 `absolute().inset_0().rounded(radius::MD)` 的叠层，只给被对齐的边 `border_*(FENCE_SNAP_GUIDE_WIDTH = 2px)`，颜色 `fence_tokens().snap_guide`（`#409EFF`，不随自定义背景色变）；跟着窗口圆角，折叠时就是标题栏的边；没有命中区，不挡鼠标。不新建覆盖窗口、不画跨窗口辅助线 |
| 高亮状态 | `FenceRuntime.snap_highlights`（目标 id → 边集合）+ `snap_owner`；拖动中的收纳箱每帧 `set_snap_guides` 整体替换，只对变了的目标 `App::notify(entity_id)`（不借用视图：调用方正在自己视图的 update 里）。拖动 / 缩放结束、按住 Alt、脱离吸附时清空；被拖或作目标的窗口关闭 / 删除时 `forget_snap` 清掉，不残留 |
| 落库 | 落库的就是吸附后的几何（移动 / 缩放结束照常 `commit_geometry` 读窗口客户区） |

**几何落库（只有用户发起的才落库）**：

| 时机 | 写什么 |
|---|---|
| 新建收纳箱 | `fence::create`：`INSERT fences`（x/y/width/height = 默认位置）+ 当前屏幕组合的 `fence_layouts` |
| 用户拖动结束 | 松手后等最后一帧落地（16ms）直接 `commit_geometry`（只落这一次；拖动中的每一帧、停顿都不落库）。不再用 `observe_window_bounds` + 闸门防抖识别"用户拖动"（自绘拖动之前的做法） |
| 用户缩放结束 | 同上 |
| 托盘「重置收纳箱位置」 | `fence::reset_positions` 事务内批量写 `fences` + 当前组合的 `fence_layouts` |

`commit_geometry` 读窗口当前客户区，按 `committed_rect` 计算：只剩标题栏（`expanded_view = false`）时只落位置和宽度，展开高度保持库里的值；
写 `fences.x/y/width/height`（"最近一次位置"）与当前组合的 `fence_layouts`（同一事务），并移除该收纳箱的 `SESSION_RECTS`；落库完成后
`disarm` 闸门（又开始拖了就不关）。收纳箱窗口不再订阅 `observe_window_bounds`：系统挪窗、程序调整自然不落库。
**不落库**：启动夹回、显示器变化导致的系统挪窗、折叠 / 展开、屏幕组合切换、拖动 / 缩放中的每一帧。界面做任何程序调整（折叠 / 展开、改外观、
开始拖动 / 缩放）前先 `flush_geometry`（松手后还在等 16ms 的那次立即落库，然后 `disarm` 闸门）。

**折叠 / 展开**：

- `fence::update({ collapsed })` 只写库；窗口形态由 `sync_view` 呈现（`Placement::for_screen(..).rect(..)` → `fence_view` → 摆窗，不写库）。
- 临时展开不改持久化的折叠态。原因集合（`ExpandReason`，任一存在即展开）：`Hover`（进入 150ms 后展开、离开 400ms 后收回，拖动中
  （`geometry_busy()`：移动 / 缩放会话进行中或闸门开着，含松手后落库完成前）不展开也不收回；GPUI 拖动期间报的"离开"不算，见「移动」第 2 步）、`Drag`（外部文件拖入经过；离开 / 放下后 300ms 收回）、`Menu`（标题栏「更多」菜单）、`Panel`（外观 / 删除确认 /
  云同步提示）、`ItemMenu`（项目右键菜单）。形态切换带代次，晚到的旧计算结果丢弃。
- `view_rect`：折叠 = 标题栏矩形（高度 = `round(32 × 目标显示器缩放)`）；展开 = 库里的展开尺寸，下方空间不够时整体上移
  （向上展开，标题栏原位置仍在窗口内）。两者都按标题栏所在工作区夹回（交叠面积最大的工作区；都不交叠取离中心最近的）。
- 缩放比例按**目标位置**所在显示器取（`title_scale`），不取窗口当前所在显示器：混合 DPI 下重置位置 / 启动夹回时两者不同。

**必须可激活**（Windows）：不加 `WS_EX_NOACTIVATE`。否则双击打开的程序可能拿不到前台；未激活的收纳箱上第一次点击即生效。
macOS 相反：收纳箱是**不激活应用**的面板（点它菜单栏不切换），鼠标事件不要求 key 窗口；只有改名要键盘，改名开始时先激活应用
（`cx.activate(true)`）再 `activate_window()`（mac 场景 §M1）。

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
- **右键菜单**：「打开所在位置」（macOS 文案「在访达中显示」，`grid.rs::reveal_label`；失败提示同名 +"失败"）之后是「隐藏」（`EpIcon::Hide`）；隐藏状态的项（显示隐藏的图标时才看得到）是「取消隐藏」（`EpIcon::View`）；
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
  打开云同步提示面板（只会有一次；字段名沿用 OneDrive）。文案按平台（`panels.rs::cloud_notice_text`）：Windows / Linux「桌面由 OneDrive 同步」
  （旧版文案），macOS「桌面由 iCloud 同步」（"也会从 iCloud 云盘中移除（你其他设备的桌面上也会少掉这些项目）"）。
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
  `write_app_settings` 读写——这是对 `CLAUDE.md`「维护检查清单」第 1 条的**有意偏离**。macOS 的 iCloud 提示共用这个键（键名沿用 OneDrive）。
  原因：桌面是否被 OneDrive 重定向 / 是否开了 iCloud「桌面与文稿」、这台设备是否已提示过，都是设备自身的事；同步或随备份导入会让另一台设备从未显示过的提示被跳过，或让已提示过的设备再提示一次。
  性质同 `webdav_*` 设备键。

#### 3.4 文件层规则

- **根目录**：`%USERPROFILE%\MiniTodo收纳箱\`（macOS `~/MiniTodo收纳箱/`；都是 `dirs::home_dir()` + `FENCE_ROOT_DIR_NAME`，家目录根不受
  macOS 隐私保护）。按需创建：只有出现第一个收纳箱（新建 / 启动恢复）后才开始监听和列举，没有收纳箱的用户不会多出这个文件夹。
  macOS 上 `fence_root_dir()` 返回 `canonicalize` 后的路径（访达拖放、FSEvents 给的都是解析过符号链接的路径，"已在该收纳箱中""来自别的收纳箱"
  按路径前缀比较，两边要同一种写法）；Windows 上不能 `canonicalize`（会变成 `\\?\` 路径）。
- **文件夹名**：`sanitize_folder_name(title)`（去首尾空白、控制字符与 `<>:"/\|?*` 换 `_`、去尾部 `.` 与空格、最多 48 字符、
  保留设备名加 `_`、空则 `收纳箱`），再与数据库里**全部** `folder_name`（即使文件夹已被用户删掉，防 UNIQUE 冲突）和磁盘上
  已有项不区分大小写去重（` (2)` 后缀）。新建：**先建文件夹再插库**；插库失败 → `remove_fence_folder_if_empty` 回滚；
  建窗失败 → `rollback_create`（删记录 + 回滚文件夹）。改名：整个过程持有数据库锁，文件夹改名失败则保留旧 `folder_name`。
- **系统元数据文件**（`naming::SHELL_METADATA_FILES`，不区分大小写，**所有平台同一张表**）：`desktop.ini`、`Thumbs.db`（资源管理器）与
  `.DS_Store`、`Icon\r`（文件夹自定义图标，名字末尾是回车）、`.localized`（访达）。不显示、不收纳（移入 `skipped`）、`restore_all` 不移动、
  删除收纳箱时不算剩余（文件夹只剩它们时连同它们一起删掉；只删这些名字的**文件**，同名文件夹照样算内容）。后三个是 macOS 移植时加的：
  访达打开过收纳文件夹就会写 `.DS_Store`，不加的话它会作为点文件出现、删除收纳箱时被搬到桌面（变成 `.DS_Store (2)`）、文件夹删不掉。
  **Windows 上的行为也随之变了**（Mac 拷来的 U 盘 / 共享上常见 `.DS_Store`）：以前当普通点文件收纳、移回，现在跳过。副作用：旧版本
  （3.0.2-pre 及更早）已经收进收纳文件夹的这类文件升级后不再列出（对账删掉它的行），删除收纳箱时不移回、随空文件夹一起删掉——它们是访达 /
  资源管理器的缓存，可再生，按"系统元数据"处理是有意的。
- **列举（`list_fence_items`）**：跳过系统元数据文件与带隐藏 / 系统属性的文件（`has_hidden_or_system_attribute`：Windows 的
  `FILE_ATTRIBUTE_HIDDEN | SYSTEM`，macOS 的 `UF_HIDDEN`（`chflags hidden`，访达不显示）；永不显示，"显示隐藏的图标"也不显示）；以 `.` 开头的
  项目所有平台都列出、`auto_hidden = true`（非 Windows 上不再按点开头排除，Linux 截图 / 冒烟里也能看到点文件）。移入校验 `plan_move_in` 用
  `is_hidden_or_system`（Windows 看属性、macOS 看 `UF_HIDDEN`——两者的点文件都照常移入、移入后按名字自动隐藏；Linux 看点开头）。
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
- **线程**：`fence_fs` 与 core `fence` 全部阻塞，app 一律放后台（`db::spawn_db`）。macOS 的文件操作在调用线程上执行（不需要主线程、
  不需要专用线程），隐私授权框、下载 iCloud 云端文件、跨卷复制、等 LaunchServices 回调都会阻塞调用线程，同样绝不在 UI 主线程调用。带 owner HWND 的移动（`fence::delete`、
  `items::{move_in, restore, move_to}`，内部是 `move_into_fence` / `restore_item` / `move_between_fences` / `restore_all`）**绝不在 UI 主线程**：
  系统"需要管理员权限"对话框以收纳箱窗口为父窗口，会给主线程同步发消息，主线程等结果就死锁。做法：HWND 在前台取好（`isize`），
  移动在后台线程里跑。需要 COM STA 的操作在 fence_fs 内部线程执行：移动 / 启动 / 定位每次新开 STA 线程（`fence-move` / `fence-launch` /
  `fence-reveal`），图标在常驻 STA 线程 `fence-icon` 上串行。
- **移入校验（`plan_move_in`，逐项）**：

  | 条件 | 结果 |
  |---|---|
  | 非绝对路径 / 磁盘根 / 无法访问 | `failed`（"需要完整路径" / "无法收纳磁盘根目录" / "无法访问: …"） |
  | 同一批重复源路径 | `skipped`（"重复的项目"） |
  | 系统元数据文件（`desktop.ini` / `Thumbs.db` / `.DS_Store` / `Icon\r` / `.localized`） | `skipped`（"desktop.ini 等系统文件不收纳"） |
  | 隐藏或系统文件（Windows 看属性；macOS 看 `UF_HIDDEN`；Linux 看点开头） | `skipped`（"隐藏或系统文件不收纳"；列举也跳过带这些属性的文件，避免"移走后哪里都看不到"）。没有隐藏属性的点文件（`.env`）在 Windows / macOS 上照常移入，移入后自动隐藏并提示 |
  | 收纳根目录本身或其上级 | `failed`（"不能收纳收纳目录本身或其上级目录"） |
  | **`protected`**：用户桌面、公共桌面本身或其上级 | `failed`（"不能收纳桌面文件夹本身或其上级目录"）——经 Shell 移动已知文件夹会让系统把"桌面"重定向到收纳文件夹里 |
  | 已在本收纳箱 | `skipped`（"已在该收纳箱中"） |
  | 别的收纳箱文件夹本身（父目录 = 根目录） | `failed`（"不能把收纳箱文件夹放进收纳箱"） |
  | 别的收纳箱文件夹里的项 | 允许（跨箱） |

  `protected` 由 `move_into_fence` 传入 `[user_desktop_dir(), public_desktop_dir()]`；桌面**上的**项照常可以收纳。
- **目标名预去重，绝不覆盖**：执行前按目标目录的实际列表（小写）加本批已预订的名字算好不冲突的名字（` (2)` 插在扩展名前，文件夹
  整体加后缀；**包**——扩展名在 `naming::PACKAGE_EXTENSIONS` 里的文件夹，如 `.app` / `.bundle` / `.framework` / `.rtfd` / `.pages`——按文件处理：
  `Safari.app` → `Safari (2).app`，否则访达不再当它是应用；纯函数、所有平台一样，Windows 上这类文件夹几乎只来自 Mac）。
  **macOS 不列举收纳根目录以外的目标目录**（`Reserved::new` 的 `list_outside_root = false`：列举桌面 / 文稿 / 下载会弹隐私授权框），
  这些目录只按本批已预订的名字去重，与磁盘上已有项的重名交给执行层的 `RENAME_EXCL` 撞名顺延（见 mac 场景 §M2）。
  Windows 执行：`IFileOperation` 标志 `FOF_RENAMEONCOLLISION | FOF_NOCONFIRMMKDIR | FOFX_NOMINIMIZEBOX`，`RENAMEONCOLLISION` 只作兜底。
  不带 `FOF_NOERRORUI`：权限不足弹系统对话框 → UAC，文件被占用弹"文件正在使用"（行为同资源管理器）。结果按 `PostMoveItem` 回调与文件
  系统实际状态归类：`COPYENGINE_E_USER_CANCELLED` / `ERROR_CANCELLED` → `cancelled`（"用户取消了操作"），`COPYENGINE_S_USER_IGNORED` →
  `cancelled`（"已跳过"）。
- **移回目标**：`original_path` 的上级目录是绝对路径、不在收纳根目录内且仍存在 → 原目录；否则（含 `NULL`）→ 用户桌面
  （`user_desktop_dir()` = `dirs::desktop_dir()`；debug 下 `MINI_TODO_DESKTOP_DIR`）。文件名：收纳时因重名被改成 `X (2).lnk` 的恢复成 `X.lnk`
  （包两种写法都认：`Safari (2).app` 与以前版本的 `Safari.app (2)`），用户在资源管理器 / 访达里改过的名字保留；目标处已有同名再加后缀。
  `restore_all` 处理文件夹里的**所有**项（含带隐藏属性的文件，系统元数据文件除外）；Windows 所有项在同一个 `IFileOperation` 里执行，macOS 逐项移动。删除确认框里的 N 用 `get_items` 的实际列举数（含收纳箱里隐藏的项目，不含带隐藏 / 系统属性的文件）。
- **监听**：`watch_fence_root`（`notify` 递归监听根目录，只关心第一、二层），最后一个事件后安静 300ms 回调一次、持续变化时最多 1.5s
  必回调；回调在线程 `fence-watch` 上 → 按文件夹名（不区分大小写）匹配收纳箱（`rescan` 时全部）→ 对账 → `FenceRefresh` 经 channel 回前台。
  `FenceWatcher` 由 app 的 `FenceRuntime` 一直持有（drop 即停止）；`services::fences::ensure_watcher` 在启动恢复（有收纳箱时）与新建后调用。
  本应用自己的移动同样触发回调，不区分来源。macOS 上 `notify` 走 FSEvents，报告解析过符号链接的路径：`watch_root_at` 先 `canonicalize(root)`
  （否则 `MINI_TODO_FENCES_ROOT` 在 `/var/folders` → `/private/var/folders` 这类路径下时算不出收纳文件夹名、不刷新）；Windows 不规范化。
- **启动**：`services::fences::restore_all` → `fence::prepare_restore`（逐个 `ensure_fence_folder`，用户可能删了文件夹；给出各自的存储几何；
  记屏幕组合检测起点）→ 逐个 `fence_view` + `open_fence_window` → 有收纳箱则 `ensure_watcher` → `items::spawn_startup_reconcile`
  （线程 `fence-startup`）：逐个对账、记录根目录下不属于任何收纳箱的文件夹（只记日志）、等 20 秒后 `prune_icon_cache(全部现存项目路径)`；
  任何一个收纳箱对账失败就跳过这次清理。
- **打开**：Windows `launch_item` 用 `ShellExecuteExW`（`SEE_MASK_FLAG_NO_UI | SEE_MASK_NOASYNC`），目标失效的 `.lnk` 约 3 秒后才返回错误，
  界面在此期间显示"正在打开"；macOS 用 `NSWorkspace openURL:configuration:completionHandler:`（先拦下失效的替身 / 符号链接，错误取自回调，
  最多等 30 秒、超时按成功），定位用 `activateFileViewerSelectingURLs:`（先 lstat，它没有错误返回）。`items::open` / `reveal` / `icon` 先 `is_valid_item_name` 校验文件名
  （防 `..`、绝对路径、ADS 逃逸出收纳文件夹；规则按平台，见 §4 错误矩阵）。
- **图标**：界面请求 `ceil(icon_size × 缩放)`（`icon_request_px`），向上取整到档位 `[16, 24, 32, 48, 64, 96, 128, 256]`；
  Windows `SIIGBF_ICONONLY`（`.lnk` 不带箭头，图片 / 视频显示类型图标），macOS `NSWorkspace iconForFile:` 画进自建的 N×N 位图再由 AppKit
  编码 PNG（边长恰好是档位；替身 / 符号链接带访达同款小箭头；在调用方的后台线程上画，`autoreleasepool` 包裹）；缓存在 `<app_data_dir>\fence-icons\`，键
  `{路径哈希 FNV-1a 64}-{mtime 毫秒}-{大小}-{档位}.png`，先写临时文件再改名。界面按可见项懒加载（只请求显示中的项目，没开
「显示隐藏的图标」时不请求隐藏项），同时最多 3 个请求
  （`timing::MAX_ICON_REQUESTS`；Windows 后端图标在单个 STA 线程排队）。图标一律先渲染进缓存再由 `img(PathBuf)` 加载，从不直接加载收纳文件夹
  或桌面上的文件。
- **云同步提示**（`onedrive_notice`）：`claim_onedrive_notice` 返回 true，且满足其一时为 true：
  - Windows：本次至少有一项从用户桌面移入（`path_starts_with_ci(source, desktop)`）且桌面被 OneDrive 重定向；
  - macOS：本次至少有一项（跨箱项不看）在**移动之前** `fence_fs::is_ubiquitous_item`（`NSURLIsUbiquitousItemKey`：由 iCloud 云盘或其它
    File Provider 管理）且移动成功——移走之后它就不是 iCloud 项目了，所以必须先判断；只读该项目自身的属性（拖放已授权它），不碰桌面目录。

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
| `MINI_TODO_DEBUG_FENCE_STATE` | GPUI 版新增：`onedrive` 打开即显示云同步提示面板（Windows / Linux OneDrive 文案，macOS iCloud 文案），`drag-hover` 显示外部拖入悬停框（Linux 上无法真实触发，截图用） |
| `MINI_TODO_DESKTOP_DIR` | macOS 移植时新增：`fence_fs::user_desktop_dir()` 返回该目录（去空白后非空才生效；macOS 上 `canonicalize`），即移回的兜底目标、`protected`（"不能收纳桌面本身"）、云同步提示的"来自桌面"判断都改用它。mac e2e 用它让"移回桌面"落在临时目录：碰真实 `~/Desktop` 可能弹隐私授权框，runner 上没人点，后台移动线程会一直卡住 |

release 保证：`dev_data_dir_override()` 的 `cfg(not(debug_assertions))` 分支直接 `None`、不读环境变量，纯函数
`data_dir_override(_, false) == None`（单测）；`location::fence_root_override` 与 `location::desktop_dir_override` 的 release 分支恒 `None`（单测
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
| 文件名不合法（`..`、绝对路径；Windows 上还有 `a:b`） | `Err("文件名不合法: …")`。`is_valid_item_name` 按平台：Windows 拦空、`.` / `..`、`<>:"/\|?*` 与控制字符、超过 255 个 UTF-16 单元；macOS / Linux 只拦空、`.` / `..`、`/` 与 NUL（`:` `?` `*` `"` `<` `>` `\|` `\` 在那里是合法文件名，见 mac 场景 §M4）。收纳文件夹名不分平台一律按 Windows 规则 |
| 项目不在文件夹里（移回 / 跨箱） | 逐项 `failed`（"收纳文件夹中找不到该项目"） |
| `items::set_hidden`：收纳箱正在删除 / 不存在 / 文件夹里已没有该项 | `Err("收纳箱正在删除")` / `Err("收纳箱 #{id} 不存在")` / `Err("收纳文件夹中找不到该项目")`；界面"隐藏失败" / "取消隐藏失败"提示后补一次列举 |
| 以 `.` 开头的项目 | 自动隐藏，右键「取消隐藏」禁用（"以 . 开头的项目自动隐藏"）；`set_hidden(.., false)` 只清标记，仍按名字隐藏 |
| 带隐藏 / 系统属性的文件（Windows 属性；macOS `UF_HIDDEN`） | 永不显示（不受「显示隐藏的图标」影响），移入时 `skipped`，删除收纳箱时一并移回 |
| 系统元数据文件（`desktop.ini` / `Thumbs.db` / `.DS_Store` / `Icon\r` / `.localized`） | 不列举；移入 `skipped`（"desktop.ini 等系统文件不收纳"）；删除收纳箱时不移动、不算剩余，随空文件夹一起删掉 |
| 拖入点文件且没开「显示隐藏的图标」 | 照常移入，提示"N 个以 . 开头的项目已自动隐藏，可在「更多」中显示隐藏的图标"（与其他拖入提示合成一条） |
| 用户在 UAC / 管理员权限对话框点取消；在错误对话框选"跳过" | 逐项 `cancelled`（"用户取消了操作" / "已跳过"），文件留在原处；界面"已取消 N 项" |
| 文件被占用 | Shell 弹"文件正在使用"，用户选择前调用一直阻塞（后台线程）；取消 / 跳过 → `cancelled`，其他失败 → `failed` + 中文原因 |
| 拖入用户桌面 / 公共桌面文件夹本身或其上级 | 逐项 `failed`（"不能收纳桌面文件夹本身或其上级目录"） |
| 收纳文件夹被用户删掉 | 对账 / 启动时按原名重建（`warn` 日志），显示为空 |
| 文件已移动但写库失败 | `warn` 日志，逐项结果照常返回成功；下次对账补行（无原始路径） |
| 双击目标失效的 `.lnk` | 约 3 秒后 `Err("快捷方式指向的目标不存在，可能已被移动或卸载")` |
| 打开 / 定位不存在的路径 | `Err("文件不存在")` |
| 图标提取失败 | `Err`，界面占位图标 |
| 既不是 Windows 也不是 macOS（Linux）：移动 / 打开 / 定位 / 图标 | `Err("桌面收纳箱仅支持 Windows 与 macOS")`；Linux 没有托盘，`restore_all` 直接返回。macOS 专有的错误见 mac 场景 §M4 |
| （Windows）`GetShellWindow()` 为空（Explorer 未运行） | `attach_to_desktop` 返回 false 不挂；守护等 Explorer 回来再挂 |
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
  覆盖了用户摆好的位置；拖动中每一帧 / 每次停顿都落库；松手后吸附高亮残留在目标收纳箱上；移动时按起点尺寸摆窗，把跨屏后 `WM_DPICHANGED`
  改好的尺寸覆盖回去；双击改名靠挂了 `on_drag` 的标题栏上的 `on_click`，第二下手抖 2~3px 就改不了名；把 GPUI 拖动期间的"离开"当真，
  拖动中标题栏按钮变暗、刚拖完的折叠收纳箱 150ms 后自己展开；带 owner 的移动在 UI 主线程执行导致死锁；拖入"桌面"文件夹本身导致桌面被重定向到收纳文件夹；移回时覆盖了同名文件；
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
| | `list_items_filters_and_describes` / `dot_names_are_auto_hidden` | 列举跳过系统元数据文件（`desktop.ini` / `.DS_Store` / `.localized` / 非 Windows 上的 `Icon\r`）与带隐藏属性的文件（Windows 属性、macOS `UF_HIDDEN`），点文件 / 点文件夹所有平台都列出且 `auto_hidden`，`hidden` 一律 false；点文件判定 |
| | `remove_only_when_empty` / `shell_metadata_files` | 只剩系统元数据文件的文件夹算空、连同它们一起删；名字叫 `.DS_Store` 的**文件夹**算内容；`Icon` / `.DS_Store (2)` 不是元数据 |
| | `package_folders_keep_their_extension_last` | 包扩展名判定（不区分大小写，`x.app.bak` / `.app` 不是包）；`Safari.app` → `Safari (2).app`、`a.b.framework` → `a.b (3).framework`，普通文件夹 `v1.2` → `v1.2 (2)`；移回时 `Safari (2).app` 与旧写法 `Safari.app (2)` 都恢复成 `Safari.app` |
| | `protected_targets_outside_the_root_are_not_listed` | `Reserved::with_policy(root, false)`（macOS 规则）不列举收纳根目录以外的目标目录、只按本批预订去重；根目录内照常列举；`Reserved::new` 只有 macOS 不列举 |
| | `default_desktop_is_the_system_desktop` | 不设 `MINI_TODO_DESKTOP_DIR` 时 `user_desktop_dir() == dirs::desktop_dir()` |
| | 命名（`sanitize_*`、`folder_name_validation_rejects_traversal`、`item_name_validation_{windows_rules,unix_rules,follows_the_platform}`、`names_legal_on_unix_can_be_restored_and_moved_between_fences`（非 Windows）、`unique_file_name_adds_suffix_before_extension`、`restore_name_undoes_collision_suffix`）、图标（`bucket_rounds_up`、`cache_key_is_stable_and_case_insensitive`、`strips_shell_frame_around_small_icons`、`prune_cache`）、监听（`watcher_reports_debounced_folder_changes`）、`release_build_ignores_override`（release） | |
| app `windows/fence/logic.rs` | `titles_follow_backend_rules` / `drag_threshold` / `geometry_gate_vectors` / `resize_vectors` / `icon_size_vectors` / `move_in_messages` / `delete_kept_messages` / `delete_hint_text` 等 | 测试向量来自旧前端 `fence.test.ts`：标题与后端同规则；闸门未 arm 的移动不算数、拖动中续期、超时 / `disarm` 后失效；缩放到最小尺寸不推动对边；图标档位与后端一致；提示文案 |
| app `windows/fence/snap.rs` | `threshold_is_ten_logical_pixels` | `round(10 × 系数)`：1 → 10、1.25 → 13、1.5 → 15、非法系数按 1 |
| | `moving_snaps_top_to_top_within_the_threshold` / `moving_snaps_both_axes_independently` / `moving_takes_the_nearest_candidate_and_breaks_ties_by_target_order` / `moving_highlights_every_target_that_ends_up_aligned` | PRD 用例：A(200,100,400,300)，B.top ∈ [90,110] 吸到 100、89 / 111 不吸；right 对齐的同时吸 top / bottom；只平移；取最近、同距离取靠前的目标；等宽时 left / right 都亮；多个目标同时对齐都亮 |
| | `collapsed_targets_snap_by_their_title_bar` / `no_targets_means_no_snapping` | 折叠的目标按标题栏矩形参与（不按展开高度）；没有目标原样返回 |
| | `resizing_the_south_edge_snaps_to_the_same_height_and_keeps_the_top` / `resizing_the_east_edge_snaps_to_the_same_width_and_keeps_the_left` / `resizing_the_south_edge_snaps_bottom_to_bottom` / `resizing_west_and_north_moves_only_the_dragged_edge` / `resizing_a_corner_snaps_both_axes` / `resizing_takes_the_nearest_of_edge_and_size_candidates` / `resizing_never_snaps_below_the_minimum_size` / `resize_snapping_runs_after_resize_rect` | 高度 ∈ [290,310] → 300、宽度 ∈ [390,410] → 400、对边不动；bottom↔bottom；W / N 只动被拖的边；角同时吸两个方向；对齐与等宽 / 等高取最近；小于最小尺寸的候选跳过；接在 `resize_rect` 之后右下角不被推走 |
| | `scene_keeps_only_targets_on_the_cursor_monitor` / `guides_are_grouped_per_target_and_diffed` | 只取光标所在显示器上的目标（跨屏的按交叠面积归属）、取不到光标不过滤；高亮按目标归并、只重绘变了的目标 |
| | `visible_items_follow_show_hidden` / `sorting_merges_hidden_items_back_in_place` / `empty_hint_text` / `move_in_mentions_auto_hidden_dot_files` | 按 `show_hidden` 过滤（用户隐藏 + 点文件）；排序后隐藏项留在原位、显示项按新顺序填回；空状态"N 个项目已隐藏"；点文件自动隐藏提示（按移入后的名字计数、开着显示隐藏时不提示、与其他提示合成一条） |
| app `windows/fence/panels.rs` | `title_menu_width_follows_the_longest_label` | 「更多」菜单切换项文案与估算宽度（123 → 151 / 165） |
| | `cloud_notice_names_the_platform_service` | 云同步提示：macOS「桌面由 iCloud 同步」，其他平台沿用 OneDrive 旧文案 |
| app `windows/fence/grid.rs` | `icon_zoom_accepts_ctrl_everywhere_and_cmd_on_mac` / `reveal_label_follows_the_platform` | 调图标大小的修饰键（Ctrl 全平台，⌘ 只在 macOS）；右键菜单「在访达中显示」/「打开所在位置」 |
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
| F12 拖动（另起实例，库里预置 A / B 两个收纳箱，主窗口最小化） | 拖 B 的标题栏：窗口跟手（位移 = 光标位移、尺寸不变）、松手后 `fences` 与当前组合的 `fence_layouts` = 客户区 |
| F13 移动吸附 | B.top 落在 A.top + 6（逻辑像素）→ 吸到 A.top（拖动中就吸）；拖动中截图 A 的上边有 `#409EFF`、左边没有；松手后高亮消失；落库的是吸附后的位置 |
| F14 Alt | 拖到一半按住 Alt → 落在阈值内也不吸、A 不高亮；松开后再拖照常吸附（单按 Alt 不进菜单模式） |
| F15 缩放吸附 | 拖 B 的下边让高度落在 A.height + 4（逻辑像素）→ 等高、上边与宽度不动；A 的左右两边高亮、上边不亮 |

仅能人工验证：公共桌面拖入点 UAC 取消；删除时占用一个文件选"跳过"；混合 DPI 下的折叠高度、跨屏拖动收纳箱时的尺寸与吸附；OneDrive 重定向桌面的提示。

macOS 的单测（只在 macOS 上编译，CI `pc-gpui-e2e-macos.yml` 的 `checks` 作业）与 mac e2e（MF01~MF10）见「场景：macOS 上的收纳箱」§M5。

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

#### Wrong — 窗口一变就落库 / 标题栏用拖动区 / 进系统拖动循环

```rust
cx.observe_window_bounds(window, |this, window, cx| this.commit_geometry(window, cx).detach()); // 显示器变化的系统挪窗也被落库
div().child(ui::title_bar::drag_region())  // 收纳箱标题栏：双击改名收不到
platform::begin_window_drag(window, cx);    // 系统拖动循环（HTCAPTION / performWindowDragWithEvent:）到松手前改不了位置：吸附做不了
```

#### Correct — 自绘拖动：超过阈值才动，每帧摆一次（吸附后），松手只落库一次

```rust
// 标题栏：on_mouse_down 记下按下点 / 光标 / 客户区起点；on_drag(TitleDrag) + on_drag_move::<TitleDrag>（捕获阶段、不看悬停）
fn on_title_drag_move(&mut self, event: &MouseMoveEvent, window: &mut Window, cx: &mut Context<Self>) {
    if event.pressed_button != Some(MouseButton::Left) { /* 没收到松手：end_move + stop_active_drag */ return; }
    if self.moving.is_none() && !self.begin_move_if_dragged(event, window, cx) { return; } // 超过 3px、未锁定、不在改名
    let raw = /* 起点 + 光标位移（屏幕物理坐标），尺寸取窗口当前值 */;
    let threshold = snap::snap_threshold(platform::physical_scale(window));   // 按窗口当前的系数
    let (rect, guides) = match session.snap.targets(event.modifiers.alt) {   // Alt：None，不吸附
        Some(targets) => snap::snap_move(raw, &targets, threshold),
        None => (raw, Vec::new()),
    };
    self.gate.arm(now_ms());                                                   // 拖动中悬停不展开 / 收回
    self.schedule_placement(PlacementTarget::Position(rect.x, rect.y), guides, window, cx); // on_next_frame：只改位置 + 更新高亮
}
fn end_move(&mut self, window: &mut Window, cx: &mut Context<Self>) {        // 标题栏 on_mouse_up + on_mouse_up_out
    if self.moving.take().is_none() { return; }
    self.flush_placement(window, cx);                                          // 最后一帧
    runtime::clear_snap_guides(cx, self.fence_id);                             // 高亮不残留
    self.commit_after_placement(window, cx);                                   // 16ms 后 commit_geometry，完成后 disarm
}
```

---

## 场景：macOS 上的收纳箱（预览，未经真机验收）

> 任务 `10-16-macos-fences`（2026-10-16）。数据、迁移、core 的元数据 / 几何 / 对账 / 登记逻辑与 Windows **完全相同**（同一套代码），
> 不同的只有窗口（没有 Progman 可挂）与文件层（没有 Shell）。维护者没有 Mac：下面的行为只在 GitHub macOS runner（`macos-15` arm64，
> 单屏 1x、一个桌面空间）上由单测与 e2e MF 组验证过，标「未验证」的只能靠用户反馈。背景调研：任务目录 `research/mac-desktop-window.md`、
> `research/mac-file-ops.md`。

### M1. 窗口：桌面图标层之上的不激活面板

**为什么不是普通窗口**：普通 NSWindow 被点击会激活应用（菜单栏切换、⌘H 隐藏中的应用被取消隐藏、主窗口一起弹回），还会计入 Dock 唤起的
`hasVisibleWindows`——只要有收纳箱可见，关掉主窗口后点 Dock 就唤不回来。所以 mac 上收纳箱建成 GPUI `WindowKind::PopUp`（`GPUIPanel`，
NSPanel + `NSWindowStyleMaskNonactivatingPanel`），再改成桌面层。Windows / Linux 仍是 `Normal`。收纳箱**不走** `platform::embed`。

**建窗**（`services::fences::open_fence_window` + `embed_after_open` 的 mac 分支）：

1. `WindowSpec`：`kind = PopUp`（`cfg!(target_os = "macos")`）、`show = false`（PopUp 默认在 101 层，所有窗口之上，不能先显示）、
   `resizable = false`、`minimizable = false`、`focus = false`，其余同 §3.1。`windows::open_window` 照常 `prepare_after_open`
   （隐藏红绿灯、`hidesOnDeactivate = NO`）并按客户区校正。
2. `cx.spawn` 的 async 块里（App 未借用；`setFrame:` / `orderFrontRegardless` 会同步触发 GPUI 回调）：`Hwnd::set_client_rect(rect)` →
   `Hwnd::make_desktop_layer()`（失败只记 warn）→ 不处于"全部隐藏"时 `Hwnd::show()`（= `orderFrontRegardless`：应用不在前台也显示，
   不改 key 窗口）→ 日志自报 `Hwnd::describe_layer()`。

**`make_desktop_layer` 之后的状态**（`platform/macos/ns.rs`；幂等，建窗后与每次重新显示前各调一次）：

| 属性 | 值 | 作用 |
|---|---|---|
| 类 / styleMask | NSPanel + `NonactivatingPanel`（建窗时由 GPUI `PopUp` 给，运行时加不上）；无 `Resizable` / `Miniaturizable` | 点击不激活应用；按 Apple 文档 NSPanel 不计入 `hasVisibleWindows` |
| `level` | `CGWindowLevelForKey(kCGDesktopIconWindowLevelKey) + 1`（= -2147483602；`ns.rs` 用 `extern "C"` 直接链接 CoreGraphics，key 18，不引入 objc2-core-graphics） | Finder 桌面图标窗口之上一层、所有普通窗口（0）之下。正好等于图标层时与 Finder 图标窗口前后不定，点击可能被它截走 |
| `collectionBehavior` | **整体替换**为 `CanJoinAllSpaces \| Stationary \| IgnoresCycle \| FullScreenNone` = `0x251`（GPUI `PopUp` 建窗时给的是 `CanJoinAllSpaces \| FullScreenAuxiliary`） | 所有桌面空间可见；调度中心 / 显示桌面时不动不藏（非 Normal 层级默认 `Transient`，会被藏起来）；不进 ⌘\` 循环；不进全屏空间 |
| `canHide` | NO | ⌘H（`[NSApp hide:]`）与别的应用「隐藏其他」都不隐藏它 |
| `hidesOnDeactivate` | NO | NSPanel 默认 YES（应用失活就藏） |
| `excludedFromWindowsMenu` | YES | 不进「窗口」菜单（GPUI 只给名为 `"Window"` 的菜单 `setWindowsMenu:`，本来也不会列出） |
| `animationBehavior` | None | 「显示/隐藏所有收纳箱」没有淡入淡出 |

日志自报（e2e MF01 / MF10 据此核对 CGWindowList 读不到的属性）：
`[fence] 收纳箱窗口 #<窗口号> 已放到桌面层：level=… behavior=0x… canHide=… excludedFromWindowsMenu=… hidesOnDeactivate=… nonactivating=… panel=… visible=…`。

**规则**：

- GPUI 建窗后不再改 level / collectionBehavior（gpui-pre-macos 只在 `MacWindow::open` 里设）；本 crate 里会改层级的只有 `ns::set_floating`
  （`set_topmost` / `Hwnd::set_topmost`），**收纳箱窗口绝不能调**；也不调 `setFloatingPanel:`（会改层级）。激活应用（`cx.activate(true)`）
  只在改名时做（见下文「改名」）；单击、拖动、缩放、外部拖入、打开项目都不激活应用，`bring_to_front` 不对收纳箱用。
- 摆窗：`apply_window_rect` → `platform::set_client_rect_physical` → `setFrame:display:`（延后执行）；frame = 客户区，物理像素 = 全局点 × 统一系数。
  缩放手柄的光标取 `cursor_position_physical()`（`NSEvent.mouseLocation` × 统一系数），最小尺寸与逻辑 → 物理换算用 `platform::physical_scale(window)`，
  **不用** `window.scale_factor()`（窗口所在屏自己的缩放，混合缩放时不同）；`check_screen_layout` 找不到显示器时的兜底系数同理。
- 显示 / 隐藏全部：显示前再 `make_desktop_layer`（幂等）→ `orderFrontRegardless`；隐藏 `orderOut:`。
- 守护：没有 Windows 那样的重挂（层级是全局排序、GPUI 不改它，Finder 重启后图标窗口仍在原层级——**未在真机验证**）；200ms 守护在 mac 上只做
  每 ~2 秒的屏幕组合检测。
- 拖动 / 缩放：与 Windows 同一份自绘拖动（§3.1「移动」「缩放」「吸附」）：mouseDragged 一直发给按下的面板（光标出了窗口也是），每帧
  `setFrameOrigin:` / `setFrame:`（`platform::set_client_position_physical` / `set_client_rect_physical`，延后执行），光标取 `NSEvent.mouseLocation` × 统一系数；
  不再用 `start_window_move`（`performWindowDragWithEvent:` 中途没有钩子，吸附做不了）。拖动不激活应用、不改层级；按住 Option 不吸附。
  GPUI 在左键按住时每 16ms 重放上一次 mouseDragged（`synthetic_drag`，修饰键是旧的）：位置按全局光标重算，重放无副作用。落库同 Windows
  （e2e MF09；只有一个收纳箱，不测吸附——吸附只在 Windows e2e F13–F15 与单测里验证，**mac 上未经真机 / runner 验证**）。
- 改名（`windows/fence/mod.rs` 的 `start_rename` → `begin_mac_rename_focus`）：非激活面板单击不一定成为 key 窗口，键盘进不来，所以改名是收纳箱
  **唯一激活应用**的操作：先 `cx.activate(true)` 再 `window.activate_window()`（`makeKeyAndOrderFront:`，GPUI 延后执行）。激活应用时 AppKit
  会把 key 还给上次的 key 窗口（主窗口），面板随即失去 key、改名被当成"点到别处"提交了原标题（2026-10-16 CI 日志）。所以：
  - 开始后 150 / 500 / 1000ms 复查（`MAC_RENAME_KEY_CHECKS`），面板不是 key 就重新激活应用并让它成为 key（焦点放回输入框，不重新全选）
  - 开始后 1.2 秒内（`MAC_RENAME_GRACE`）的失去 key 与输入框 Blur 不当作提交
  - 宽限期结束后再查一次（`MAC_RENAME_FINAL_CHECK_MS` = 1250ms）：面板仍不是 key（宽限期内失去 key、没能拿回来）就提交——之后不会再有
    失去 key 的通知，不提交就会卡在改名状态
  - 宽限期过后用户点到别的应用 / 桌面时面板失去 key，经 `observe_window_activation` 照常提交；回车提交、Esc 取消同其他平台
  - 过程写进日志（`[fence] 改名…`：GPUI 的窗口激活态、应用是否在前台、AppKit 的 key 窗口号），CI 失败时据此定位

  代价：改名期间菜单栏切到 mini-todo（主窗口可能一起到前面）；单击、拖动、缩放、外部拖入、双击打开项目仍不激活应用。e2e MF08：访达在前台时
  双击标题 → 系统输入流输入（逐字注入进不去就 pbcopy + ⌘A + ⌘V）→ 回车 → 落库，runner 上用粘贴 + 回车验证（runner 上 `CGEventKeyboardSetUnicodeString` 逐字注入的
  按键本来就进不了 GPUI 输入框，普通窗口的 M09 也一样，不是面板的问题）。**中文输入法在改名输入框里是否正常未验证**。
- ⌘M / ⌘W（`services/mac_menu.rs`）：活动窗口是收纳箱时两者都不做（`is_fence`；`miniaturize:` 不看 style mask，必须显式拦）。
- Dock 唤起：只有收纳箱可见时 `on_reopen` 照常回调（NSPanel 不算可见窗口，e2e MF07）。**退路**（若某个 macOS 版本上失效）：改用 `Normal` +
  自己注册 `kAEReopenApplication` Apple Event handler（调研 `mac-desktop-window.md` §5.3 有草图），其余设置不变。
- 外部拖入：GPUI 对所有窗口（含 NSPanel）注册 `NSFilenamesPboardType`，`ui::external_drop` 不分平台；但 GPUI 对外部拖入一律回报
  `NSDragOperationCopy`——访达光标带绿色"+"，实际执行的是移动（与 Windows 一样"真收纳"）。没有改。
- 挂在收纳箱上的 sheet / 系统对话框会和它同层（被所有应用窗口盖住）：收纳箱里的确认 / 提示一律用窗口内弹层（`ui::message_box` 一类）。
- 「自定义颜色」（`platform/fence_win.rs` 的 mac `imp`）：前台任务里 `ns::show_color_panel(initial)`（共享的 `NSColorPanel`：不显示透明度、
  `hidesOnDeactivate = NO`——否则用户点一下桌面面板就藏起来、被当成"关掉了"——设初始色、**先激活应用**再 `makeKeyAndOrderFront:`），之后每 150ms
  `ns::color_panel_state()`，面板不可见时返回最后读到的颜色，和初始色相同视为取消。读分量前先 `colorUsingColorSpace(sRGB)` 并确认是 RGB 模型
  （对非 RGB 颜色读 `redComponent` 会抛 ObjC 异常，穿过 Rust 帧直接 abort）。代价：打开取色面板会让菜单栏切到 mini-todo（与"不激活"相反，接受）。
- 其它界面差异：右键菜单「在访达中显示」、⌘+滚轮也能调图标大小（⌃+滚轮在开了「辅助功能 → 缩放」时被系统拿去放大屏幕）、云同步面板写 iCloud。

### M2. 文件层：`services/fence_fs/mac.rs`

`mac.rs` 是**叶子模块**：不 `use super::` / `crate::`，入参出参只有路径、字符串、bool 与普通结构，同一份源码按 edition 2021（core）与 2024
（`scripts/mac-check`）都能编译（extern 块写 `unsafe extern "C"`，unsafe 操作放进显式 `unsafe {}`）。适配（`MoveJob` ↔ `MoveResult`、
去重规则）留在 `ops.rs` 的 `execute_mac`。core 的 mac 依赖（libc / block2 / objc2 / objc2-foundation / objc2-app-kit）与 app 同版本，
`scripts/mac-check/Cargo.toml` 的特性 = app 与 core 的 mac 依赖的并集。

**移动（`move_item`，绝不覆盖）**：

1. 逐项 `renamex_np(src, dest_dir/name, RENAME_EXCL)`：内核持 rename 锁原子判重，目标已存在返回 `EEXIST`（不区分大小写的 APFS 上只差大小写 /
   Unicode 规范化形式也算重名）。
2. `EEXIST` → 换下一个候选名 `naming::suffixed_file_name(dest_name, n, is_dir)`（n = 2, 3, …，包按文件处理），最多 `MAX_NAME_ATTEMPTS = 64` 个名字，
   用完失败（"目标位置同名项目过多"），源原样保留。
3. `EXDEV`（跨卷）/ `ENOTSUP`（卷不支持 `RENAME_EXCL`，部分网络卷）→ `NSFileManager moveItemAtURL:toURL:error:`：目标已存在报 516 / `EEXIST`
   → 换下一个名字；跨卷是 copyfile（EXCL，逐项不覆盖）+ 删源，复制中途失败 Foundation 删掉目标里的半成品；**复制完成但删源失败不回滚**——
   返回 `final_path` + 提示"已复制到目标位置，但原位置的项目未能删除"（`source_left = true`，`classify` 归为 Moved + 附注），绝不删目标。
   `ENOTSUP` 卷上 NSFileManager 同卷是 lstat + rename，有极小的竞争窗口（只在这类卷上）。
4. 其他 errno → `describe_errno`（下表），`MoveOutcome { hr: Some(-errno), error: Some(中文) }`；成功 `hr: Some(0)`、`new_path` = 实际路径。
   mac 没有提权 / "文件正在使用"对话框，所以没有 `cancelled`；`owner_hwnd` 不用。
5. 整批在调用线程（后台）上执行；开始前 `allow_materialize_on_this_thread()`（`setiopolicy_np(IOPOL_TYPE_VFS_MATERIALIZE_DATALESS_FILES,
   IOPOL_SCOPE_THREAD, ON)`），结束后还原：rename 一个"只在云端"的 dataless 文件（macOS 14+ iCloud 走 File Provider）时内核先让它下载完再改名，
   不会留下坏文件，只是可能阻塞很久；策略禁止下载时返回 `EDEADLK`。

**禁止** `std::fs::rename` / `rename(2)` 做生产移动：目标已存在时它会**静默替换**（`fake_exec` 测试执行器先断言目标不存在，只适合测试）。

| errno（`describe_errno`） | 中文原因 |
|---|---|
| `EPERM`，且源 / 源所在文件夹 / 目标文件夹带 `uchg` / `schg` / 仅追加标志 | "项目或所在文件夹已锁定（访达「显示简介」里勾选了「已锁定」），请先解锁" |
| `EPERM`（其余：沙盒 / TCC 拒绝） | "macOS 拒绝访问该位置：请在「系统设置 → 隐私与安全性 → 文件与文件夹」中允许 mini-todo 访问相应文件夹" |
| `EACCES` | "没有权限" |
| `ENOENT`：源还在 / 同目录有 `.名字.icloud` 占位（macOS ≤ 13 的云端文件）/ 其余 | "目标文件夹不存在" / "该文件只在 iCloud 中，请先在访达中下载该文件" / "文件不存在" |
| `EDEADLK` / `ETIMEDOUT` | "该文件只在 iCloud 中、未能下载，请先在访达中下载该文件" |
| `ENOSPC` / `EROFS` / `ENAMETOOLONG` / `EBUSY` / `EEXIST`（名字用完） | 磁盘空间不足 / 目标位置是只读的 / 路径过长 / 正被系统占用 / 目标位置同名项目过多 |

规划阶段 `plan_move_in` lstat 源失败时，`EPERM` 与"`ENOENT` + iCloud 占位"同样给上表的说明（`describe_access_error`），其余仍是"无法访问: …"。
`describe_io_error` 的 Win32 错误码分支只在 Windows 编译（mac 上 32 是 `EPIPE`）；mac 的 `PermissionDenied` 是"没有权限"（不带"可能正被占用"）。

**隐私权限（TCC，「隐私与安全性 → 文件与文件夹」）**：

- 事实（Apple WWDC19-701 / DTS，见调研）：拖放即授权被拖的项目（user intent，授权记在项目上）；在桌面 / 文稿 / 下载等受保护目录里**新建**
  目录项不需要同意；**列举**它们（读已有内容）会弹「想访问桌面文件夹中的文件」授权框，拒绝后返回 `EPERM`。授权框**阻塞调用线程**直到用户点按钮。
  ad-hoc 签名每次升级 cdhash 都变，授权会失效、再弹一次。`cargo run` 的裸二进制套用终端的授权，测不出问题，要测就用 `bundle.sh` 的 `.app`。
- 设计：正常路径一次框都不弹。mac 上规划阶段**只列举收纳根目录内的目标目录**（`Reserved::new(root)` → `list_outside_root = false`），
  移回桌面 / 原目录时只按本批预订的名字去重，与磁盘上已有项的重名交给 `RENAME_EXCL` 顺延；`choose_restore_dir` 只 `stat` 原目录（是否存在），
  预计不弹框（**未验证**）；iCloud 判定只读被拖项目自身的属性。
- 拖放授权是否覆盖"把项目从桌面目录里移走"（rename 源端要改父目录）**未验证**；失败时得到 `EPERM`，按上表引导用户去系统设置授权。
- `bundle.sh` 的 Info.plist 带 `NSDesktopFolderUsageDescription` / `NSDocumentsFolderUsageDescription` / `NSDownloadsFolderUsageDescription`
  （万一弹框时的中文说明）；**不是身份键**，改了不影响升级。
- e2e 与本地测试**绝不碰**真实 `~/Desktop` / `~/Documents` / `~/Downloads`（runner 上授权框没人点，后台移动线程会永久阻塞）：
  `MINI_TODO_FENCES_ROOT` + `MINI_TODO_DESKTOP_DIR` 指向临时目录，且拖入的来源 / 移回的原目录都在临时目录里。

**其它与 Windows 对齐的细节**（规则在 §3.4，这里只列 mac 的实现）：

| 能力 | macOS 实现 |
|---|---|
| 隐藏属性 | `UF_HIDDEN`（`chflags hidden`）：列举永不显示、移入 `skipped`、删除时一并移回；点文件照常移入（按名字自动隐藏） |
| 系统元数据 | `.DS_Store` / `Icon\r` / `.localized`（全平台表，§3.4） |
| 包 | `PACKAGE_EXTENSIONS` 里的文件夹按文件去重：`Safari (2).app` |
| 打开 | `NSWorkspace openURL:configuration:completionHandler:`（`setActivates(true)`）；先拦失效的符号链接（"符号链接指向的目标不存在…"）与替身（`NSURLIsAliasFileKey` + `URLByResolvingAliasFileAtURL` 不弹界面、不挂载："替身指向的原身不存在…"）；回调错误 → "打开失败：…"；最多等 30 秒，超时按成功 |
| 定位 | `activateFileViewerSelectingURLs:`（没有错误返回，先 lstat） |
| 图标 | `NSWorkspace iconForFile:` 画进自建 N×N `NSBitmapImageRep`（1 point = 1 像素、sRGB、预乘 alpha）→ AppKit 编码 PNG → `write_atomically`；不改 `iconForFile:` 返回的共享图片；后台线程画要自己 `flushGraphics` |
| iCloud 提示 | 移动前 `is_ubiquitous_item`（§3.4「云同步提示」），共用 `fence_onedrive_notice_shown` |
| 监听 | `notify` = FSEvents；`watch_root_at` 先 `canonicalize(root)` |
| 根目录 | `~/MiniTodo收纳箱/`（`fence_root_dir` 返回 canonicalize 后的路径）；不是旧版沿用的名字（2.5.0 mac 版没有收纳箱） |

### M3. 线程

- core `mac.rs` 全是阻塞函数，**从不需要** `MainThreadMarker`：NSFileManager / NSWorkspace / NSImage / NSBitmapImageRep / NSGraphicsContext 都不是
  `MainThreadOnly`，Apple 文档写明 `iconForFile:`、`openURL:configuration:completionHandler:`、`activateFileViewerSelectingURLs:` 可在任意线程调用；
  用到 Foundation / AppKit 的函数都包在 `autoreleasepool` 里（Rust 线程没有自动释放池）。app 一律经 `spawn_db` 放后台。
- app `ns.rs`（桌面层、取色面板）只在主线程（GPUI 前台）、App 未借用时调用：建窗后的处理与显示隐藏在 `cx.spawn` 的 async 块里、`update` 之外；
  取色面板在 `choose_color` 返回的前台任务里显示与轮询（[pc-gpui/gpui-conventions.md §2.15](../pc-gpui/gpui-conventions.md)）。

### M4. mac 专有的错误与边界

| 条件 | 行为 |
|---|---|
| 拖入 / 移回撞名 | `RENAME_EXCL` 撞名顺延 ` (2)`、` (3)`…（包 `X (2).app`），绝不覆盖；64 个名字都占用 → 逐项 `failed`，源保留 |
| 项目被锁定（`uchg`） | 逐项 `failed`（"…已锁定…请先解锁"），原样不动 |
| 隐私权限被拒（`EPERM`） | 逐项 `failed`，引导到「系统设置 → 隐私与安全性 → 文件与文件夹」 |
| 只在 iCloud 的文件 | 先下载再移动（阻塞）；下载被禁止 / 超时 / macOS ≤ 13 的占位 → `failed` + "请先在访达中下载该文件" |
| 跨卷移动 | NSFileManager 复制 + 删源；删源失败 → Moved + 附注（两边各一份） |
| 名字含 `<>:"\|?*`、`\` 或控制字符（mac 上合法，`Icon\r` 是元数据除外；`:` 在访达里显示成 `/`） | 与普通项目一样：能收纳、列举、打开、在访达中显示、取图标、隐藏、单独移回、移动到…、随删除收纳箱整体移回（`is_valid_item_name` 在非 Windows 上只拦空、`.` / `..`、`/` 与 NUL） |
| 失效的替身 / 符号链接 | 打开报"替身指向的原身不存在…" / "符号链接指向的目标不存在…" |
| `make_desktop_layer` 失败（窗口号找不到） | warn 日志，窗口留在 `PopUp` 默认层级（不会发生在正常路径上） |
| 取色面板打不开 / 颜色不是 RGB | warn 日志 / 视为取消 |
| 当前活动窗口是收纳箱时按 ⌘M / ⌘W | 不做任何事 |

### M5. 测试

- **单测**（只在 macOS 上编译，CI `pc-gpui-e2e-macos.yml` 的 `checks` 作业跑）：`fence_fs::ops::real_moves_mac::{move_in_restore_and_cross_fence_round_trip,
  rename_excl_never_overwrites_and_tries_the_next_name, locked_items_fail_with_a_clear_reason_and_stay_put, icloud_placeholders_are_recognized}`
  （临时目录里真实 `renamex_np`：往返移入移回、跨箱、包 `Safari (2).app`、点文件移入 / `UF_HIDDEN` 跳过、桌面不列举时靠撞名顺延不覆盖、
  `restore_all` 不搬 `.DS_Store` 且删掉文件夹、只差大小写也不覆盖、名字用完失败、已锁定报"锁定"）；`icon::extracts_real_icons_into_cache_on_macos`
  （输出边长恰好等于档位的合法、非全透明 PNG，二次命中缓存）；`folder::list_items_filters_and_describes` 的 `UF_HIDDEN` 分支；
  `watch::watcher_reports_debounced_folder_changes`（根目录被规范化）。全平台跑的新增单测见 §6。
- **交叉检查**：`scripts/mac-check` 收录 `crates/core/src/services/fence_fs/mac.rs` 与 `crates/app/src/platform/macos/ns.rs`（Linux / Windows 上
  `--target aarch64-apple-darwin` 的 clippy）；`fences.rs` / `fence_win.rs` 的 mac 分支与 core 里 `mac.rs` 以外的 mac 分支只有 CI 能编译。
- **e2e**（`pc-gpui/scripts/e2e-macos/run_e2e.py` 的 MF 组，smoke 作业，debug `.app`，全部在临时目录）：MF01 新建（层级 = 桌面图标层 + 1、排在所有
  layer ≥ 0 的窗口之后、368×336pt 右上角、库与文件夹、日志自报 `0x251` / `canHide=false` / 非激活面板）、MF02 目录监听、MF03 真实拖入
  （`drag_source.py` 拖放源）与撞名不覆盖、MF04 移回原目录 / 桌面、MF05 删除收纳箱全部移回且 `.DS_Store` 不搬到桌面、MF06 ⌘H 与别的应用隐藏都不藏
  收纳箱 + 点收纳箱不激活应用、MF07 有收纳箱时 Dock 唤起主窗口、MF08 双击标题改名（激活应用，系统输入流粘贴 + 回车）、MF09 拖标题栏落库、MF10 菜单栏图标三项与重启恢复；M07 断言菜单栏图标
  菜单**有**收纳箱三项。窗口枚举不带 `kCGWindowListExcludeDesktopElements`（详见 `pc-gpui/scripts/e2e-macos/README.md`）。
- **runner 测不了**（写进交付说明）：Spaces / 台前调度 / 调度中心 /「显示桌面」/「点按墙纸显示桌面」、Retina / 多显示器 / 混合缩放、中文输入法、
  真实 `~/Desktop` 的 TCC 流程、iCloud「桌面与文稿」、跨卷移动、打开 / 在访达中显示（会拉起别的应用）、Finder 重启后的层级、光标形状。

### M6. Wrong vs Correct（mac）

#### Wrong — 用 `std::fs::rename` 移动

```rust
if !dest.exists() {
    std::fs::rename(&source, &dest)?;  // exists 与 rename 之间出现的同名文件会被静默替换；跨卷直接 EXDEV 失败
}
```

#### Correct — `renamex_np(RENAME_EXCL)`，撞名换下一个名字，跨卷交给 NSFileManager

```rust
let rc = unsafe { libc::renamex_np(src.as_ptr(), dst.as_ptr(), libc::RENAME_EXCL) }; // 内核原子判重
match last_errno() {
    libc::EEXIST => name = next_name(attempt + 1),                         // 绝不覆盖
    libc::EXDEV | libc::ENOTSUP => /* move_with_file_manager：516 / EEXIST 也换名 */,
    errno => return MoveResult::failed(Some(errno), describe_errno(source, dest_dir, errno)),
}
```

#### Wrong — 规划时列举桌面去重

```rust
let taken = existing_names_lower(&desktop);  // read_dir(~/Desktop)：弹「文件与文件夹」授权框、阻塞后台线程；拒绝后列表为空
```

#### Correct — 只列举收纳根目录内的目标目录

```rust
let mut reserved = Reserved::new(root);  // macOS：list_outside_root = false，桌面只按本批预订去重，与已有项的重名交给 RENAME_EXCL
```

#### Wrong — 对收纳箱窗口"置顶一下"

```rust
h.set_topmost(false);  // ns::set_floating(false) 把层级改回 NSNormalWindowLevel：收纳箱盖住其它应用的窗口
```

#### Correct — 层级只由 `make_desktop_layer` 设置

```rust
h.make_desktop_layer();  // 桌面图标层 + 1、0x251、canHide = NO …（幂等，重新显示前再调一次）
h.show();                // orderFrontRegardless：同层最前，不改 key 窗口、不激活应用
```

---

## Design Decisions

### Design Decision: 真收纳 + 可见文件夹为 source of truth

**Context**：Fences / 腾讯 / 360 让图标从桌面消失靠的是接管 Explorer 桌面视图（要注入 Explorer）；纯引用（NoFences）零风险但桌面
不会变干净。

**Decision**：物理移动文件（TuckPane 模式）到 `%USERPROFILE%\MiniTodo收纳箱\<名称>\`（macOS `~/MiniTodo收纳箱/<名称>/`），文件夹是内容的
唯一来源，数据库只存元数据、排序与原始路径。卸载 / 崩溃 / 数据库丢失都不丢文件，最坏情况是用户在该文件夹里手动找回。代价：依赖桌面路径的
脚本会失效；OneDrive / iCloud「桌面与文稿」桌面下的移动会同步到其他设备（首次提示一次）；公共桌面项需要 UAC（Windows）。

### Design Decision: 始终嵌入，「固定」只是界面锁定

**Decision**：收纳箱复用主窗口的嵌入方案（owner=Progman + `FLAG_BOTTOM` 永远置底 + `WS_EX_TOOLWINDOW`，`platform::embed`），
建窗参数一次给齐、之后不改；窗口建成 `resizable: false`，缩放用窗口内手柄 + `SetWindowPos`。锁定 / 解锁、折叠 / 展开都不碰窗口样式。
旧版实测多个收纳箱重叠时点击不会成组重排（被点的升到组内最上，其余相对顺序不变）。

### Design Decision: macOS 用不激活面板 + 桌面图标层，而不是普通窗口

**Context**：mac 没有 Progman 可挂。候选：普通 NSWindow 放到桌面层；`Floating`（激活型 NSPanel）；`PopUp`（不激活的 NSPanel）；
Electron `type: 'desktop'` 那样放到 `kCGDesktopWindowLevel - 1`（收不到鼠标键盘）。

**Decision**：GPUI `PopUp` + `CGWindowLevelForKey(kCGDesktopIconWindowLevelKey) + 1` + `CanJoinAllSpaces | Stationary | IgnoresCycle |
FullScreenNone` + `canHide = NO`（Plash 的可交互模式、Hammerspoon "能可靠收点击的最低层级"同一层）。不激活应用：点收纳箱不切换菜单栏、
不把隐藏中的应用（连同主窗口）取消隐藏、不计入 Dock 唤起的 `hasVisibleWindows`；`PopUp` 有 `ActiveAlways` 跟踪区，应用不在前台时悬停折叠 /
展开照常。代价：改名要键盘，改名开始时必须先激活应用（`cx.activate(true)`）再 `activate_window()`，并在宽限期内把 key 从主窗口拿回来
（§M1「改名」），改名期间菜单栏切到 mini-todo；改名时的中文输入法未验证。`Floating` 点击仍激活应用且没有跟踪区；普通 NSWindow 计入
`hasVisibleWindows`（退路：普通窗口 + 自己的 reopen Apple Event handler）。

### Design Decision: macOS 移动用 `renamex_np(RENAME_EXCL)`，不用 NSFileManager / `rename(2)` 打头

**Context**：硬约束"绝不覆盖"。`rename(2)` / `std::fs::rename` 目标存在时静默替换；`NSFileManager moveItemAtURL` 同卷是先 lstat 再普通 rename，
中间有竞争窗口。

**Decision**：同卷（桌面与 `~/MiniTodo收纳箱` 同在 Data 卷，几乎全部情况）用内核原子判重的 `RENAME_EXCL`，撞名换下一个后缀名重试（等价于
Windows 的 `FOF_RENAMEONCOLLISION`）；只有跨卷 / 卷不支持时交给 NSFileManager（复制阶段同样 EXCL）。规划阶段不列举受隐私保护的目标目录，
撞名完全交给内核判重——比按字符串比较还准（大小写、Unicode 规范化）。

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
- **macOS（预览，未经真机验收）**：只在 runner 上验证过（单屏 1x、一个桌面空间、临时目录；改名用粘贴 + 回车验证）。没验证过 Spaces / 台前调度 / 调度中心 /
  「显示桌面」/「点按墙纸显示桌面」下的表现、Retina 与多显示器、改名时的中文输入法、真实 `~/Desktop` 的隐私授权流程（含 ad-hoc 签名
  升级后重新授权）、iCloud「桌面与文稿」、跨卷移动、Finder 重启后的层级、打开 / 在访达中显示；访达拖入时光标带"+"（GPUI 回报 Copy）但实际是移动；
  改名与取色面板会激活应用（菜单栏切到 mini-todo）。混合缩放下统一系数随显示器组合变化时"最近一次位置"可能按新系数解释
  （与主窗口同性质，见 CLAUDE.md 已知限制）。
- **装回旧版本**（Windows 与 macOS 都是）：3.0.2-pre 及更早的 mac 版不恢复收纳箱（`restore_all` 在 mac 上直接返回）、菜单栏图标没有入口，
  2.5.0 mac 版完全不认这些表——收纳箱不显示，但文件都还在 `~/MiniTodo收纳箱/<名称>/` 里，可在访达里找回；再装回新版本后照常恢复。
