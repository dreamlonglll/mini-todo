# Window Modes（清单 / 日历窗口：普通 / 经典固定 / 固定 + 嵌入桌面，按窗口）

> 清单窗口与日历窗口（合称"桌面块窗口"，`windows::block::BlockKey::{List(id), Calendar}`）的 Win32 样式、owner、Z 序、几何锁定与生命周期的契约。
> 实现：`pc-gpui/crates/app/src/services/window_mode/`（`mod.rs` 按窗口的运行时：登记、模式切换、200ms 轮询、唤起、重置、保存；`machine.rs`
> 每个窗口一份的纯状态机；`store.rs` 几何与锁定的持久化）+ `services/block_windows.rs`（开 / 关 / 显示 / 隐藏）+ `platform/embed.rs`（嵌入桌面层，
> Windows 上与收纳箱共用）+ `platform/subclass.rs`（永远置底、几何锁定）+ `platform/windows.rs`（`Hwnd` 原始调用）；纯逻辑在
> `pc-gpui/crates/core/src/window_logic.rs`，窗口属性与布局持久化在 core `window_prefs.rs`。
> 任务 `11-09-multi-list-windows`（3.1）把原来的单个主窗口拆成每个清单一个的清单窗口 + 单例日历窗口：单例 `WindowMode`（只认一个 `main_hwnd`）
> 改成按 `BlockKey` 的状态表，默认清单窗口（`BlockKey::DEFAULT_LIST` = `List(1)`）就是以前的主窗口。
> macOS（预览，任务 `10-16-macos-package`）走同一套状态机与 200ms 轮询，Win32 部分换成 `platform/macos/`（AppKit），语义差异见「Scenario: macOS」。
>
> 历史：建立于任务 `09-07-desktop-embed-mode`（2026-09，旧 Tauri 版，Win11 25H2 实测），`10-15-desktop-fence` 把嵌入核心泛化为
> 按窗口句柄执行、供收纳箱复用，`10-15-gpui-rewrite` 改为 GPUI 实现。旧 Tauri 版为对抗 tao `WindowFlags::apply_diff` 重写 ex style
> 而写的排队重申（`reassert_window_mode_state`、`run_on_main_thread` 排序规则）在 GPUI 下不再需要（GPUI 建窗后不改写 ex style），
> 已随旧版删除；保留的是 §3 的状态表、"嵌入不是第三种模式"、持久化键与 Explorer 重启 / 样式自愈的语义。

---

## Scenario: List / calendar window mode switching

### 1. Scope / Trigger

- 改动以下任何东西前先读本文件：`services/window_mode/` 的 `ModeState::plan_mode` / `apply_mode` / `ModeState::plan_tick` / `run_tick`、
  `reset_all` / `bring_all_to_front`、`services/block_windows.rs`、`platform::embed::*`、`platform::subclass`（`FLAG_BOTTOM` / `FLAG_LOCK_GEOMETRY`）、
  `Hwnd::{set_tool_window, set_minimizable, set_topmost, set_geometry_locked, set_owner}`、core `window_logic` 的 `needs_reattach` /
  `lacks_tool_window_style` / 贴边状态机、settings 键 `fixed_embed_desktop` / `auto_hide_enabled` / `top_on_wake`、core `window_prefs`
  （`window_prefs` / `window_layouts` 表）。
- `platform::embed` 是 Windows 上清单 / 日历窗口与所有收纳箱窗口共用的嵌入核心（收纳箱的登记与守护见 [desktop-fences.md](./desktop-fences.md) §3.1）：
  改它要两边都回归。macOS 的收纳箱不经 `embed`（mac 上是空实现），而是桌面图标层之上的不激活面板（`Hwnd::make_desktop_layer`，
  见 desktop-fences.md「场景：macOS 上的收纳箱」）。
- 下面 §2 ~ §7 的 Win32 状态只在 Windows 上成立；macOS 的固定模式见「Scenario: macOS」；Linux 上 `platform::embed` / `subclass` 是安全的空实现、
  没有原生句柄（不跑 200ms 轮询），"嵌入"偏好在 mac / Linux 上都按经典固定处理（§4）。

### 2. Signatures

```rust
// crates/app/src/windows/block.rs
pub enum BlockKey { List(i64), Calendar }          // Ord：清单在前（按 id），日历最后；window_key() = "list:<id>" / "calendar"
impl BlockKey { pub const DEFAULT_LIST: BlockKey = BlockKey::List(1); } // = 以前的主窗口

// crates/app/src/services/window_mode/machine.rs —— 每个窗口一份的纯状态机（全平台编译，有单测）
pub struct ModeState {
    /* fixed：用户可见的锁定（两种固定都为 true，按屏幕组合持久化）；classic_fixed / desktop：窗口侧的两种固定（不会同时为 true）；
       auto_hide: AutoHideState；desired_topmost；top_on_wake（全局偏好的每窗口缓存）；pending_release */
}
pub enum TargetMode { Normal, ClassicFixed, Desktop }
impl TargetMode { pub fn for_state(is_fixed: bool, embed_desktop: bool) -> TargetMode; } // embed 只在 Windows 生效
pub fn classic_fixed_look(fixed: bool, embed_desktop: bool) -> bool;                   // 无圆角无描边
impl ModeState {
    pub fn plan_mode(&mut self, prefs: RuntimePrefs, target: TargetMode) -> Ops;
    pub fn apply_runtime_prefs(&mut self, prefs: RuntimePrefs);
    pub fn embed_switch(&self, enabled: bool) -> Option<TargetMode>;
    pub fn plan_tick(&mut self, facts: &TickFacts, now: Instant) -> TickPlan;
    pub fn wake(&mut self, now: Instant) -> Wake;
    pub fn release_wake_topmost(&mut self, latest: bool) -> bool;
    pub fn reset_runtime(&mut self);
    pub fn is_edge_hidden(&self) -> bool;
}
pub const RESET_CASCADE_STEP_DIP: f64 = 32.;
pub fn reset_size(key: BlockKey) -> (f64, f64);   // 清单 380×600；日历 calendar_window::logic::default_width(600) = 596 × 600
pub fn reset_rect(monitor: MonitorBounds, scale: f64, index: usize, size: (f64, f64)) -> PhysRect;
pub fn reset_plan(..);                            // 「重置位置」的排列（默认清单、其它清单按 id、日历最后）

// crates/app/src/services/window_mode/mod.rs —— 按窗口的运行时（Global：BTreeMap<BlockKey, Block>）
pub fn prepare(cx, key, fixed: bool);                                  // 建窗前：第一帧按锁定外观画
pub fn register(cx, key, handle: AnyWindowHandle, hwnd: Option<Hwnd>, view: EntityId); // 建窗后：轮询 + 吸附组 + 集中观察偏好
pub fn unregister(cx, key);
pub fn is_registered(cx, key) -> bool;  pub fn registered(cx) -> Vec<BlockKey>;
pub fn is_fixed(cx, key) -> bool;  pub fn is_classic_fixed(cx, key) -> bool;  pub fn is_edge_hidden(cx, key) -> bool;
pub fn is_shown(cx, key) -> bool;                                     // 已开窗、显示中、没最小化、没贴边藏起（services::notice 用）
pub fn apply_mode(cx, key, target) -> Task<Result<(), String>>;       // 没登记 → Err("{key} 不存在")，如 "清单窗口 #1 不存在"
pub fn set_fixed(cx, key, fixed: bool);  pub fn toggle_fixed(cx, key); // 标题栏锁按钮 / 重置：切完保存
pub fn on_embed_preference_changed(cx, enabled: bool);               // 所有锁定的窗口
pub fn apply_runtime_prefs(cx, prefs: RuntimePrefs);                  // 所有窗口
pub fn bring_all_to_front(cx);  pub fn bring_to_front(cx, key);       // 托盘左键 / 单实例 / Dock；打开设置
pub fn on_activated(cx, key);                                         // 记最近激活（notice）；macOS 贴边隐藏中被激活 → bring_all_to_front
pub fn reset_all(cx);                                                 // 托盘「重置位置」
pub fn schedule_save(cx, key);  pub fn save_window_state(cx, key);  pub fn save_window_state_with(cx, key, rect);
pub fn read_startup_geometry(cx, key) -> StartupGeometry;  pub fn on_close_requested(window, cx, key);
pub const TICK_INTERVAL: Duration = Duration::from_millis(200);       // 嵌入重挂 / 经典固定守护 / 贴边隐藏，每个 tick 遍历所有窗口

// crates/app/src/services/window_mode/store.rs —— 几何与锁定的持久化（core window_prefs 的 window_layouts）
pub struct BlockPlacement { pub rect: Option<PhysRect>, pub fixed: bool }
pub fn load(db, key, screen_id) -> Result<Option<BlockPlacement>, String>;
pub fn save(db, key, screen_id, rect: PhysRect, fixed: bool) -> Result<(), String>;   // 默认清单窗口另外登记 screen_configs
pub fn save_all(db, screen_id, &[(BlockKey, PhysRect, bool)]) -> Result<(), String>; // 一个事务（「重置位置」，含没开过窗的）

// crates/app/src/platform/embed.rs —— 立即执行的 Win32，只能在 App 未被借用时调用
pub fn desktop_host() -> Option<Hwnd>;                // GetShellWindow()；Explorer 未运行时 None
pub fn attach_to_desktop(hwnd: Hwnd) -> bool;         // 幂等：FLAG_BOTTOM + owner=Progman + TOOLWINDOW + 去 MINIMIZEBOX + HWND_BOTTOM
pub fn detach_from_desktop(hwnd: Hwnd, tool_window: bool); // 先关 FLAG_BOTTOM → owner=0 → 按 tool_window 恢复样式 → 恢复 MINIMIZEBOX → HWND_TOP
pub fn set_always_on_bottom(hwnd: Hwnd, on: bool);
pub fn needs_reattach(hwnd: Hwnd) -> bool;            // window_logic::needs_reattach(..) || (shell_now != 0 && lacks_tool_window_style(ex))

// crates/app/src/platform/subclass.rs —— SetWindowSubclass 子类过程
pub const FLAG_BOTTOM: u32;        // WM_WINDOWPOSCHANGING：Z 序一律改成 HWND_BOTTOM（替代旧版 tao always_on_bottom）
pub const FLAG_LOCK_GEOMETRY: u32; // WM_NCHITTEST / WM_SYSCOMMAND：固定模式锁定位置与尺寸
pub fn set_flag(h: Hwnd, flag: u32, on: bool);

// crates/core/src/window_logic.rs —— 纯函数（全平台编译，有单测）
pub fn is_desktop_host_class(name: &str) -> bool;                  // == "Progman"
pub fn needs_reattach(cached_owner: isize, owner_alive: bool, shell_now: isize, current_owner: isize) -> bool;
pub fn lacks_tool_window_style(ex_style: u32) -> bool;             // 没有 WS_EX_TOOLWINDOW 或带 WS_EX_APPWINDOW

// crates/app/src/state.rs —— AppState（只剩全局偏好；锁定按窗口在 window_mode，日 / 夜、视图等按窗口在 window_prefs 的内存副本）
pub fixed_embed_desktop: bool;         // 偏好开关（settings fixed_embed_desktop）
pub fn set_fixed_embed_desktop(&mut self, enabled, cx) -> Task<Result<(), String>>; // 只写键（失败回滚）
```

### 3. Contracts

**Persisted state**

| Store | Key / column | Meaning | Synced (export / WebDAV) |
|---|---|---|---|
| `window_layouts` (v31) | `is_fixed` + `x / y / width / height` per `(window_key, screen_config_id)` | each list / calendar window: locked (classic or embedded) and client-area physical rect for this display combination | no (device-specific) |
| `window_prefs` (v31) | `hidden` / `dark` / `view_mode` / `expand_all` / `calendar_filter` per `window_key` | per-window device-local properties | no |
| `settings` | `fixed_embed_desktop` (v27, default `'false'`) | preference: fixed mode embeds into desktop — global, applies to every locked window | yes (`AppSettings.fixed_embed_desktop`, `#[serde(default)]`) |
| `settings` | `auto_hide_enabled`, `top_on_wake` | classic-fixed-only behaviours (global, per-window state machines); ignored while embedded | yes |
| `settings` | `is_fixed` | legacy (≤ 3.0.x main window); only the v31 migration's seed for `list:1`, no longer read or written | yes (old devices still use it) |
| `screen_configs` | `is_fixed` + geometry | display-combination registry; written when the default list window saves (same geometry / lock), read by Settings → 屏幕配置 and by old versions | no (device-specific) |

There is **no** `is_desktop` column or key: "embedded" is not a third mode, it is `is_fixed && fixed_embed_desktop`.
建窗时 `block_windows::open_block` 读 `read_startup_geometry(key)`（当前屏幕组合的 `window_layouts` 记录，`width / height > 0` 才算有几何），
`prepare(key, fixed)` → `open_window` → `register` → 固定则 `apply_mode(key, TargetMode::for_state(true, embed_desktop))`。
迁移 v31 把原主窗口各组合的 `screen_configs` 记录复制给 `list:1`（原来开着日历的按左 40% / 右 60% 切给 `list:1` / `calendar`，锁定相同）。

**按窗口**：每个登记的窗口一份 `ModeState`（锁定、经典固定 / 嵌入、贴边隐藏状态机、期望置顶、收尾标记）与保存防抖代次；
嵌入桌面 / 贴边隐藏 / 唤起置顶是**全局偏好**、作用于每个锁定的窗口。`top_on_wake` 在每个窗口里各缓存一份，`apply_runtime_prefs` 才能对每个窗口
各自发现"被关掉了"（共用一份缓存时只有第一个窗口会收尾）。锁定的窗口不进任务栏 / Alt+Tab，未锁定的进（同一 AppUserModelID，Windows 合并成一个任务栏图标）。

**各模式的 Win32 状态**（Windows e2e S 组断言的就是这张表）

| 模式 | `classic_fixed` | `desktop` | owner（`GWLP_HWNDPARENT`） | ex style | `WS_MINIMIZEBOX` | 几何锁定（`FLAG_LOCK_GEOMETRY`） | Z 序 / 置顶 | 圆角与描边 |
|---|---|---|---|---|---|---|---|---|
| 普通 | false | false | 0 | `WS_EX_APPWINDOW`，无 TOOLWINDOW | 有 | 否 | 普通顶层窗口 | 有 |
| 经典固定 | true | false | 0 | `WS_EX_TOOLWINDOW`，无 APPWINDOW | 有 | 是 | 顶层；只在贴边唤起（`top_on_wake`）/ 托盘唤起时置顶 | 无 |
| 固定 + 嵌入桌面 | false | true | Progman（`GetShellWindow()`） | `WS_EX_TOOLWINDOW`，无 APPWINDOW，无 TOPMOST | **无** | 是 | `FLAG_BOTTOM`：每次改 Z 序都压到 `HWND_BOTTOM` = 紧贴 Progman 之上 | 有 |

三种模式都**没有 `WS_SYSMENU`**、DWM 标题栏按钮区（`DWMWA_CAPTION_BUTTON_BOUNDS`）宽度为 0：`windows::open_window` 建窗后统一去掉
（否则系统 × 会从深色半透明窗口的客户区透出来，见 [pc-gpui/gpui-conventions.md §2.2](../pc-gpui/gpui-conventions.md)），模式切换只改上表各列、
不碰它；只去这一位，`WS_CAPTION` 等其它样式位不动。去掉后普通模式的系统行为实测不受影响（Win11 26H2，2026-10-16）：单击任务栏按钮
最小化 / 还原（e2e S08）、Alt+F4；只是没有了系统菜单（Alt+Space）。
3.1 起清单 / 日历窗口建窗 `resizable: false`（与收纳箱一样自绘移动与缩放，`windows::block::BlockDrag`，见 gpui-conventions §2.3）：没有
`WS_THICKFRAME` / `WS_MAXIMIZEBOX`，标题栏 `WM_NCHITTEST` 是 `HTCLIENT`（不是 `HTCAPTION`），外框外沿是 `HTBORDER`；标题栏双击、Win+↑ 不再最大化，
拖到屏幕顶边不贴靠（没有 Aero Snap）；缩放热区是可见边缘内侧 5px（角 10px）的手柄。

**偏好开关的跨窗口契约**

1. 设置窗口 → `AppState::set_fixed_embed_desktop(v)`：先改状态再写键（`settings::set_fixed_embed_desktop`），失败回滚。
2. `window_mode` 在第一个窗口登记时 `cx.observe(AppState)`（`ensure_prefs_observer`，整个应用一次），发现 `fixed_embed_desktop` /
   `runtime_prefs` 变了就 `defer` 调 `on_embed_preference_changed` / `apply_runtime_prefs`，对所有窗口生效：只有已固定且在 Windows 上的窗口才在
   两种固定之间直接切换（不保存窗口状态）；未固定时只是键变了，下次固定时生效。同步应用了远端设置后重读设置，走同一条路径。
   清单 / 日历窗口自己**不要**再观察这两项（否则 N 个窗口对同一次偏好变化各切一次模式）。
3. 设置窗口**不得**调用 `save_window_state`（会把设置窗口的状态当成清单窗口的写进去）。

**Ordering rules**

1. **App 借用期间不碰 Win32**：`apply_mode` 在 `cx.update` 里用该窗口的 `ModeState::plan_mode` 只算出一组 `Op` 并更新状态，再在 async 块里、
   `update` 闭包之外逐个执行（`Hwnd` 原始调用会同步给 GPUI 窗口过程发消息，App 被借用时 GPUI 处理不了，几何状态会错乱；
   见 [pc-gpui/gpui-conventions.md §2.2](../pc-gpui/gpui-conventions.md)）。200ms 轮询同理：每个 tick 对所有存活的窗口各算一份 `TickPlan`
   （显示器只枚举一次、光标只读一次），再逐个执行。
2. 排队期间模式可能又变了：`run_tick` 执行 `Op::Attach` 前重新检查**该窗口**的 `desktop`，已退出嵌入就跳过。
3. 嵌入 → 经典固定：`detach_from_desktop(hwnd, tool_window = true)`，全程保持 `WS_EX_TOOLWINDOW`，不闪任务栏。
4. 经典固定 → 嵌入 / 普通：先撤置顶（`Topmost(false)`）再离开经典固定，否则解锁后一直置顶；贴边隐藏中的窗口先拉回锚点
   （`AutoHideState::leave_fixed_mode`）。
5. `detach_from_desktop` 先关 `FLAG_BOTTOM`，否则它最后的 `HWND_TOP` 会被子类过程改回 `HWND_BOTTOM`。
6. owner 用 `SetWindowLongPtrW(GWLP_HWNDPARENT, Progman)`，**不用 `SetParent`**（子窗口化后 DPI 与圆角退化，见 Design Decisions）。
7. 几何锁定：移动 / 缩放是自绘的，锁定时 `BlockDrag::title_bar(.., can_move = false, ..)` 不挂拖动、`resize_handles(false, ..)` 不出手柄；
   系统层面仍设 `FLAG_LOCK_GEOMETRY`（`WM_NCHITTEST` 回 `HTCLIENT` / `HTBORDER`，`WM_SYSCOMMAND` 拦移动 / 缩放 / 最大化），兜住键盘等系统入口。
8. 锁定只有每个窗口标题栏的锁按钮（`toggle_fixed(cx, key)`）与「重置位置」（解锁）两个入口；托盘没有「固定模式」（3.1 删除），`apply_mode`
   不回写任何托盘勾选。
9. **按实际状态自愈**（200ms 轮询，只在 Windows、对每个存活的窗口）：嵌入态 `platform::embed::needs_reattach` 为真 → `Attach`；
   经典固定态 `lacks_tool_window_style` → 补 `WS_EX_TOOLWINDOW`、被 Win+D / Win+M 最小化 → 立即还原（固定模式没有任务栏入口）。

**唤起**：托盘左键 / 单实例 / Dock（`on_reopen`）/ macOS 贴边隐藏中被激活 → `bring_all_to_front`：先 `block_windows::prepare_wake_all`
（所有清单都要显示：开过窗的清掉 `hidden`，没开过窗的现在开窗——新开的建窗时已显示并激活、不再唤起；日历只在「显示日历」开着时唤起，
`should_wake`）；一次唤起只开一个托盘唤起代次（`begin_tray_wake`），所有非嵌入的窗口一起临时置顶，600ms 后各按自己的贴边状态决定撤不撤
（嵌入桌面的只显示 + 聚焦）；默认清单窗口最后激活（在最上面、拿焦点）。托盘「打开设置」/ 应用菜单「设置…」只唤起默认清单窗口（`bring_to_front`）。

**重置位置**（托盘「重置位置」，`reset_all`；PRD R9：全部错开、解锁、取消隐藏）：
1. 重算屏幕组合 ID → `store::save_all` 一个事务把 `block_windows::all_keys` 的全部窗口（默认清单、其它清单按 id、日历最后，**不管开没开过窗**）
   重置后的几何（**未锁定**）写进当前组合（默认清单照旧登记 `screen_configs`；清单已被删的跳过）。第 i 个从 `window_logic::reset_window_position`
   （主显示器左上、`尺寸 × 10% ÷ 缩放` 边距 = 以前主窗口的重置位置）向右下错开 i × 32 逻辑像素；按这个窗口自己的尺寸（`reset_size`）判断，
   再错开就超出显示器右边或下边时从第一个位置重来。
2. `prepare_wake_all`：没开过窗的现在开——`open_block` 同步读布局，读到的就是第 1 步写的，直接开在重置后的位置、不带锁定（不会先在旧位置出现再跳）。
3. 已开的窗口：清贴边状态、撤置顶、（要显示的）还原最小化、`set_client_rect`、隐藏着的摆好后再显示；锁定的 `set_fixed(false)`（切完保存），
   其余 `save_window_state`（读回实际客户区覆盖第 1 步的记录）。
4. 按排列顺序唤起：后面的在上面（经典层叠，每个标题栏都露出来；获得焦点的是最后一个——日历开着时是日历）。
- 日历只在「显示日历」开着时显示；关着时已建的窗口在隐藏状态下摆好、解锁、保存，没建过的只写布局（下次显示时出现在层叠里的位置）。不改「显示日历」本身。
- 第 1 步写库失败只记日志，第 3 步照样摆好、各自保存。macOS 先 `cx.activate(true)`；Linux 不能移动窗口，只按重置尺寸 `window.resize`。

**保存**：系统 / 程序挪窗（DPI、显示器变化）500ms 防抖保存（`schedule_save`）；**用户自绘拖动 / 缩放中不防抖保存**，松手 16ms 后 `BlockDrag`
直接 `save_window_state` 一次。几何与锁定经 `store::save`（贴边隐藏中由调用方换成锚点）；不再写旧的全局键（`is_fixed` / `window_position` /
`window_size` / `text_theme` …）。

### 3.1 清单 / 日历窗口的生命周期（`services/block_windows.rs`）

| 时机 | 行为 |
|---|---|
| 启动 `block_windows::start`（`lib.rs`，`AppState` / `WindowManager` 之后、托盘之前） | 迁移 `ui-state.json` 的展开全部 → **同步**读清单、窗口属性、运行时偏好、屏幕组合（建窗前就要知道深色 / 视图）→ 默认清单窗口（总是显示，清掉它的 `hidden`）→「显示日历」开着时开日历 → `reconcile` 开其它没隐藏的清单 → 观察 `AppState::lists_revision` → 5 秒变更轮询 → 检查更新。**隐藏的窗口启动时不建**，第一次显示时才建 |
| 关闭（Alt+F4 / 系统关闭、macOS ⌘W）→ `request_close` | 有模态子窗口时不关、把子窗口带到前台；否则保存几何、写 `hidden = true`（日历 = 关掉「显示日历」并回写托盘勾选）、`platform::hide_window`，**不注销**（`window_mode` / 吸附组照常找得到；隐藏的窗口不算吸附目标）。Linux 不能程序化隐藏：真的关窗，最后一个窗口关掉就退出 |
| Windows 退出 | 没有"所有窗口都关了就退出"：只有托盘「退出」，同 macOS 的 ⌘Q |
| 清单列表变化（`lists_revision`：同步 / 5 秒轮询 / 本机新建改名删除 / 窗口属性变化都会加一）→ `reconcile` | 清单已不存在的窗口关掉（先关它的模态子窗口；默认清单永不关）；没开窗的清单里，本机没有窗口记录（同步 / cloud 新建）或记录是显示中 → 开窗，记录是隐藏 → 不开。同步来的新清单：主显示器工作区里从右上角找不与显示中的吸附组窗口重叠的空位（`new_list_rect`，与 `fence::default_fence_rect` 同一算法，边距 24、步长 32 逻辑像素），380×600、未锁定、浅色、列表视图（`WindowPrefs` 默认值） |
| 「显示日历」（设置 → 常规「展示日历」、托盘勾选、调试 `calendar`）→ `set_calendar_visible` | 写日历的 `hidden`、回写托盘勾选；显示时没建过就建（当前屏幕组合没有布局 → `calendar_window::logic::first_rect`：贴默认清单窗口右侧、等高，宽度按高度让日格大致是方的；右边放不下放左边，再放不下夹回工作区），建过就 `bring_to_front`；隐藏时保存几何 + 原生隐藏（Linux 关窗） |
| 托盘「新增待办清单」（调试 `list:new`）→ `create_list` | `AppState::create_list(None)`（后台自动取名"清单 / 清单 2…"，同一个后台任务建好显示中的窗口记录）→ 光标所在显示器上找空位开 380×600 的窗口 → 进入改名。清单建好那一刻 `lists_revision` 加一、`reconcile` 先于回调开窗，所以位置事先放进 `BlockWindows.creating`，`reconcile` 对"有窗口记录、没开窗"的清单用它，回调再 `begin_rename` |

- 窗口的系统标题：默认清单窗口固定 `Mini Todo`（e2e 按它找窗口），其它清单窗口 = 清单名（改名后跟着改），日历窗口 `日历`。
- 登记：`WindowManager` 的 `WindowKind::List` / `WindowKind::Calendar`，key = core 窗口属性表的 key（`list:<id>` / `calendar`）；
  `WindowManager::block_window(cx, key)` 按 key 找（含原生隐藏着的），`default_list(cx)` = 默认清单窗口。
- 新窗口的登记顺序：`prepare(key, fixed)`（建窗前；`open_window` 会同步画第一帧）→ `open_window` → `register(key, handle, hwnd, view)` →
  锁定的 `apply_mode(key, TargetMode::for_state(true, embed))`；关闭 = 隐藏时**不要** `unregister`，只有清单被删除、窗口真的关掉时才注销。

### 4. Validation & Error Matrix

| Condition | Behaviour |
|---|---|
| 窗口没登记 | `apply_mode` → `Err("{key} 不存在")`（如 `清单窗口 #1 不存在`）；`set_fixed` / `toggle_fixed` 对没登记的窗口只记 warn、什么都不改 |
| 关闭有模态子窗口的清单 / 日历窗口 | 不关，把子窗口带到前台 |
| 清单已被删除、窗口还没关（同步 / 本机删除后）时保存几何 | core `save_layout` 跳过（返回 false），不报错 |
| 非 Windows 且 `fixed_embed_desktop = true`（如同步来的） | `TargetMode::for_state` 只在 Windows 上选 `Desktop`，其它平台回退为经典固定；`on_embed_preference_changed` 不做事；固定不会因此失败 |
| `GetShellWindow()` 为空（Explorer 未运行） | `desktop_host()` → `None`，`attach_to_desktop` 返回 false、不挂；`shell_now == 0` 时 `needs_reattach` 为 false，轮询等 Explorer 回来再挂 |
| shell 窗口类名不是 `Progman`（Win10 / Win11 ≤ 23H2 / 第三方 shell） | 记一次 warn，**仍然挂上**——不最小化、不进任务栏、在所有应用窗口之下；Win+D 可能盖住它（已知限制） |
| Explorer 重启（新 Progman） | 下一个 tick `needs_reattach`（旧 owner 已死 / shell 句柄变了）→ 重挂到新 Progman；窗口不随旧 owner 销毁 |
| 嵌入态 ex style 被改成 APPWINDOW / 丢了 TOOLWINDOW | `needs_reattach` 里的 `lacks_tool_window_style` 命中 → 重挂（GPUI 下是防御：GPUI 本身不改写 ex style） |
| 经典固定被 Win+D / Win+M 最小化 | 下一个 tick 还原并显示 |
| 未固定时切换偏好 | 只写键；下次固定时按新偏好进入 |
| 旧备份 / 远端文档里没有 `fixedEmbedDesktop` | `#[serde(default)]` → false；导入不会失败 |

### 5. Good/Base/Bad Cases

- **Good**: 偏好开着，点锁按钮 → owner = 桌面宿主、TOOLWINDOW、无 MINIMIZEBOX；Win+D 后仍可见；普通应用窗口能盖住它；
  重启 Explorer 后自动重挂；杀进程重启后按 `window_layouts.is_fixed` + 偏好恢复同样的模式与位置。锁定一个清单窗口不影响其它清单 / 日历窗口。
- **Base**: 偏好关着，点锁按钮 → 经典固定（owner = 0、TOOLWINDOW、有 MINIMIZEBOX、不可拖动 / 缩放）；已固定时切换偏好，所有锁定的窗口在两种固定之间
  直接切换、任务栏不闪。
- **Bad**: 在事件处理 / `update` 闭包里直接调 `Hwnd::set_rect` / `attach_to_desktop`（GPUI 丢掉几何变化）；`SetParent` 到桌面宿主
  （DPI 与圆角退化）；`detach_from_desktop` 里先 `HWND_TOP` 后关 `FLAG_BOTTOM`（窗口仍被压到底）；设置窗口切开关后再让清单窗口 `save_window_state`；
  关闭 = 隐藏时 `unregister`（唤起、重置、轮询、吸附都找不到它）；每个窗口各自观察嵌入 / 贴边偏好。

### 6. Tests Required

Unit（`cargo test`，均已存在）：

- core `window_logic.rs`：`is_desktop_host_class`（`"Progman"` true；`"WorkerW"`、`""`、`"progman"` false）；`needs_reattach`（缓存 0 → true；
  owner 已死 → true；shell 句柄变了 → true；窗口 owner 被清 → true；`shell_now == 0` → 一律 false）；
  `tool_window_style_is_repaired_only_when_wiped`（只有 TOOLWINDOW / TOOLWINDOW|其它位为 false；只有 APPWINDOW、`0`、TOOLWINDOW|APPWINDOW 为 true）。
- core `db/migrations.rs`：`v27_adds_fixed_embed_desktop_key`（键默认 `'false'`；`screen_configs` 没有 `is_desktop` 列）。
- core `data.rs`：`import_without_fixed_embed_desktop_defaults_to_false`、`fixed_embed_desktop_survives_export_import_round_trip`。
- app `services/window_mode/machine.rs`：`target_mode_follows_embed_preference_only_on_windows`、`classic_look_only_when_fixed_without_embedding`、
  `mode_switches_keep_the_win32_ordering_rules`、`leaving_fixed_mode_pulls_an_edge_hidden_window_back_first`、
  `embed_preference_switches_only_locked_windows_on_windows`、`tick_guards_each_mode_independently`、`edge_hiding_runs_per_window`、
  `turning_a_preference_off_releases_every_window`、`tray_wake_pulls_hidden_windows_back_and_releases_the_topmost_later`、
  `reset_keeps_the_first_window_where_the_main_window_used_to_go`、`reset_wraps_around_instead_of_leaving_the_monitor`、
  `reset_plan_cascades_lists_then_the_calendar`；`store.rs`：`reset_writes_every_window_including_ones_never_opened` 等；
  `windows/block.rs`：`keys_order_the_default_list_first_and_the_calendar_last`、`minimum_size_follows_the_physical_scale`；
  `services/block_windows.rs` 的 `plan_reconcile` / `new_list_rect`；`platform/subclass.rs`：`geometry_hits_cover_resize_and_drag`。
- core `window_prefs.rs`、`db/migrations.rs` 的 v31 种子（`v31_seeds_the_default_list_window_from_the_main_window`、
  `v31_splits_the_widened_main_window_when_the_calendar_was_open`）。

E2E（Windows，`pc-gpui/scripts/e2e-windows/run_e2e.py` 的 S 组；CI `.github/workflows/pc-gpui-e2e-windows.yml`，见其 README 与
[desktop-window-e2e-guide](../guides/desktop-window-e2e-guide.md)）：

| 用例 | 断言 |
|---|---|
| S01 普通模式 | APPWINDOW、无 owner；3.1 起标题栏 `WM_NCHITTEST` = `HTCLIENT`、外框外沿 `HTBORDER`、没有 `WS_THICKFRAME`，双击标题栏**不**最大化；真实拖动标题栏移动且位置落库（自绘）（以前断言 `HTCAPTION` / `HTLEFT` 与双击最大化，脚本随第 6 步 e2e 一起改） |
| S02 点锁按钮 → 经典固定 | TOOLWINDOW、真实拖动不动、锁定落库（3.1 起在 `window_layouts.is_fixed`） |
| S03 `fixed_embed_desktop=true` 重启恢复 | owner = 桌面宿主、TOOLWINDOW、无 MINIMIZEBOX |
| S04 Z 序 | 普通应用窗口（探针）在它之上 |
| S05 Win+D | 之后仍可见 |
| S06 Explorer 重启 | 自动重挂 owner |
| S07 点锁按钮回普通模式 | 状态表"普通"一行；标题栏又能拖动（3.1 起用真实拖动判断，不再看 `HTCAPTION`） |
| S08 普通模式单击任务栏按钮 | 真实点击（UIA 找按钮）：前台时最小化、再单击还原到前台 |

S01 / S02 / S03 / S07 / S08 每次切换后都另外断言无 `WS_SYSMENU`、DWM 标题栏按钮区宽度为 0。

### 7. Wrong vs Correct

#### Wrong — 在事件处理里直接执行 Win32

```rust
.on_click(cx.listener(|_, _, window, _| {
    if let Some(h) = platform::hwnd(window) {
        platform::embed::attach_to_desktop(h); // 同步发 WM_WINDOWPOSCHANGING / WM_STYLECHANGED，App 正被借用
    }
}))
```

#### Correct — 先在 update 里算计划，再在 update 之外执行

```rust
pub fn apply_mode(cx: &mut App, key: BlockKey, target: TargetMode) -> Task<Result<(), String>> {
    cx.spawn(async move |cx| {
        let planned = cx.update(|cx| { let b = block_mut(cx, key)?; Some((b.hwnd, b.state.plan_mode(prefs, target))) });
        let Some((hwnd, ops)) = planned else { return Err(format!("{key} 不存在")) };
        if let Some(hwnd) = hwnd { ops.run(hwnd); } // Attach / Detach / ToolWindow / LockGeometry / Topmost …
        Ok(())
    })
}
```

#### Wrong — 退出嵌入时先抬到顶再关置底钩子

```rust
SetWindowPos(h, HWND_TOP, …);          // 子类过程仍开着 FLAG_BOTTOM：被改回 HWND_BOTTOM
set_flag(hwnd, FLAG_BOTTOM, false);
```

#### Correct — `detach_from_desktop` 的顺序

```rust
set_flag(hwnd, FLAG_BOTTOM, false);    // 先关钩子
SetWindowLongPtrW(h, GWLP_HWNDPARENT, 0);
hwnd.set_tool_window(tool_window);     // 嵌入 → 经典固定传 true：不闪任务栏
/* 恢复嵌入前的 MINIMIZEBOX */
SetWindowPos(h, HWND_TOP, …);
```

---

## Scenario: macOS（预览，未经真机验收）

### 1. Scope

- 2.5.0 mac 版的固定模式是"简化版"（不可缩放、不可拖、被最小化即还原、唤起临时置顶、贴边隐藏降级为按鼠标移入移出），GPUI 版对齐这套语义，
  并沿用上面的按窗口状态机：`TargetMode` / `plan_mode` / `apply_mode` / `plan_tick` / `run_tick` 一行没分叉，差异全在 `platform::*` 的 mac 实现
  （`platform/macos/mod.rs` → `ns.rs`）。`platform::Hwnd` 在 mac 上是 NSWindow 的窗口号（每次在 `NSApp.windows` 里重新找，见
  [pc-gpui/gpui-conventions.md §2.15](../pc-gpui/gpui-conventions.md)），所以 `register` 拿得到句柄、200ms 轮询照常启动。
- 改 `platform/macos/`、`window_mode/` 里的 `cfg!(windows)` / `cfg(target_os = "macos")`、`ns::set_geometry_locked` / `update_style` 前读本节。
- 3.1 的多清单 / 日历窗口没有新增 mac 专属代码（只有 `bring_all_to_front` / `bring_to_front` / `reset_all` 开头的 `cx.activate(true)`），
  共用路径**没在 Mac 上跑过**：⌘W → `request_close` → `platform::hide_window`（`orderOut:`）；全部隐藏后点 Dock 带出隐藏的窗口；新清单进入改名时
  普通窗口能否拿到 key（收纳箱是 NSPanel 才要那套宽限，清单窗口没加）；`orderOut:` 状态下 `setFrame:` 再显示（「重置位置」）。

### 2. 各模式在 mac 上的状态

| 模式 | 拖动 / 缩放 | 窗口层级 | 程序坞 / ⌘Tab | 圆角与描边 |
|---|---|---|---|---|
| 普通 | 自绘移动与缩放（标题栏 `BlockDrag`、窗口内 8 个手柄；`setFrameOrigin:` / `setFrame:`，不再 `start_window_move`）；建窗不带 `Resizable` | `NSNormalWindowLevel` | 应用级：Dock 图标照常显示、参与 ⌘Tab | 有（界面自己画） |
| 经典固定 | `Op::LockGeometry(true)` → `setMovable:NO`（只拦 AppKit 自己的拖动；自绘拖动与手柄在锁定时本来就不挂）；去掉 `Resizable` 是空操作（建窗时就没有），解锁后也不恢复 | 平时 `NSNormalWindowLevel`；托盘唤起临时 `NSFloatingWindowLevel` 600ms，贴边唤起且开了"唤起时置顶"时保持浮动层级 | **不变**：`Op::ToolWindow` 是 no-op（mac 没有逐窗口的"不进任务栏 / Alt+Tab"，程序坞与 ⌘Tab 是整个应用的激活策略） | 无（`is_classic_fixed()`，与 Windows 相同） |
| 固定 + 嵌入桌面 | **不存在**：`TargetMode::for_state` 只在 Windows 上选 `Desktop`；设置项「固定模式时，嵌入桌面中」不显示；同步来的 `fixed_embed_desktop = true` 只让固定走经典固定；`platform::embed::*` 是空实现 | — | — | — |

- 改 style mask（`Resizable` / `Miniaturizable`）一律走 `ns.rs` 的 `update_style`：AppKit 可能重建标题栏（红绿灯重新出现）并换掉第一响应者
  （键盘输入不再进 GPUI 视图），`update_style` 改完会重新隐藏红绿灯、把第一响应者还回去。
- `setMovable:NO` 只拦用户拖动，程序摆窗（`setFrame:` / `setFrameOrigin:`，贴边隐藏、重置位置、屏幕配置恢复）不受影响。
- 清单 / 日历窗口的"置顶"是 `ns::set_floating`（`NSFloatingWindowLevel` ⇄ `NSNormalWindowLevel`）。它只用于这些普通窗口：收纳箱窗口在桌面图标层之上一层，
  对它调 `set_topmost`（不管 true / false）都会把层级改成 3 / 0，收纳箱会盖住或混进普通窗口，**禁止**（desktop-fences.md §M1）。

### 3. 200ms 轮询在 mac 上做什么

| 项 | mac 行为 |
|---|---|
| 嵌入重挂 | 不会发生（`mode.desktop` 恒 false） |
| 经典固定被最小化（⌘M、应用菜单「最小化」） | 下一个 tick `isMiniaturized` → `deminiaturize:` 还原（红绿灯已隐藏，⌘M 是唯一入口；活动窗口是收纳箱时 ⌘M 什么都不做） |
| tool window 样式自愈 | 不做（`cfg!(windows) && lacks_tool_window_style(..)`，mac 的 `ex_style()` 恒 0，否则每个 tick 都会误判） |
| 贴边隐藏 | 照常：`cursor_position_physical()` = `NSEvent.mouseLocation`（全局光标），显示器来自 `NSScreen.screens`；比 2.5.0 的"前端上报 mouseenter / mouseleave"准确。隐藏 / 唤起靠 `Hwnd::set_client_position`（`setFrameOrigin:`）。**AppKit 会把带标题栏样式的窗口约束在屏幕内（顶边不能高过菜单栏）**：mac 的 `set_client_position` 摆完读回实际位置，差超过 1 个统一系数单位就返回 false，`run_tick` 据此把贴边状态回滚成显示态——有菜单栏的屏幕上往顶边藏不进去 |

- **贴边隐藏中被激活**：GPUI 只在"没有可见窗口"时回调 Dock 唤起（`on_reopen`），藏在屏幕边缘的窗口仍算可见（收纳箱是 NSPanel，按 Apple 文档
  不算，所以只有收纳箱可见时 `on_reopen` 照常回调，e2e MF07）。所以每个清单 / 日历窗口的激活回调里
  `cx.defer(window_mode::on_activated(key))`：mac 上若处于经典固定 + 贴边隐藏，就按托盘左键处理（`bring_all_to_front`：拉回锚点、临时置顶，
  也会带出隐藏的清单窗口）；Windows 上只记"最近激活"。点 Dock 图标、⌘Tab 切回来都走这条路。
- 同一原因：有清单 / 日历窗口显示着时点 Dock 只激活应用、**不会**带出隐藏的清单窗口（与托盘左键不完全一样，只能靠菜单栏图标左键；已知限制）。

### 4. 唤起与重置

- `bring_all_to_front`（菜单栏图标左键、单实例转交、Dock / 访达再次打开经 `mac_menu::on_reopen`、贴边隐藏中被激活经 `on_activated`）与
  `bring_to_front(DEFAULT_LIST)`（应用菜单「设置…」）开头先 `cx.activate(true)` 把应用切到前台，之后与 Windows 相同：贴边隐藏开着时给宽限并拉回 →
  还原 → 临时置顶 600ms → 激活（`makeKeyAndOrderFront:`）。
- 托盘「重置位置」（`reset_all`）：开头同样 `cx.activate(true)`；mac 能程序化摆窗（Linux 不能），与 Windows 相同——全部清单 / 日历窗口层叠排开、
  解锁、取消隐藏、保存（§3「重置位置」）。
- 启动恢复：与 Windows 相同，按当前屏幕组合的 `window_layouts.is_fixed` 进入经典固定；几何的物理像素口径见
  [pc-gpui/data-compat.md §2.1](../pc-gpui/data-compat.md)。

### 5. Tests

- 单测：`target_mode_follows_embed_preference_only_on_windows`（已有）；坐标换算 `platform/mac_geometry.rs`（Linux 上跑）。
- e2e：`pc-gpui/scripts/e2e-macos/` 的 M10（普通模式拖标题栏能移动 → 锁定 → 拖不动 → 解锁后又能拖），CI `pc-gpui-e2e-macos.yml`。
  3.1 删掉了菜单栏「固定模式」、锁定改记在 `window_layouts.is_fixed`、建窗不带 `Resizable`（AX 改尺寸在普通模式下多半也无效），
  M10 要改成点标题栏锁按钮，断言"手柄缩放能改、固定后手柄消失"（随第 6 步 e2e 一起改）。
- **没有覆盖**（待补用例 / 待真机反馈）：贴边隐藏与唤起置顶在 mac 上的实际效果（左 / 右 / 下边能否藏进去、往顶边藏被挡回时的回滚、
  贴边隐藏中经 Dock / ⌘Tab 激活后拉回）；被 ⌘M 最小化后的还原；固定模式在 Spaces / 全屏 / 调度中心下的表现（没有设置 `collectionBehavior`）；
  多显示器 / 混合缩放下的贴边判定。

---

## Common Mistakes

### Common Mistake: 只在切换那一刻打一次 Win32 样式

**Symptom**（旧 Tauri 版实测）：开发版在"主窗口（现在的默认清单窗口）固定 + 嵌入桌面、桌面上有 5 个收纳箱"时重启，主窗口最终 owner=Progman 但
`toolWindow=False / appWindow=True`——出现在任务栏、参与 Alt+Tab，之后再没人补回来。当时的原因是 tao 晚到的 `apply_diff` 重写了 ex style。

**Prevention**：GPUI 不再改写 ex style，但这条教训保留——任何"只在切换时设置一次"的窗口状态都要配一个按**实际状态**检查的轮询兜底：
嵌入态的 `needs_reattach` 同时看 owner 与 `lacks_tool_window_style`，经典固定态检查 TOOLWINDOW 与最小化，收纳箱的 200ms 守护用同一判定。
正常情况下每个 tick 只多一次 `GetWindowLongW`。

---

## Design Decisions

### Design Decision: owner=Progman + 永远置底，而不是 `SetParent`

**Context**: "嵌入桌面"要求透明、圆角、per-monitor DPI 与普通模式零差异，且 Win11 24H2+ 上"显示桌面"会抬起 Progman 本身。

**Options Considered**:
1. `SetParent(hwnd, SHELLDLL_DefView host)` 子窗口（Lively / Wallpaper Engine 式）
2. 保持顶层窗口，`GWLP_HWNDPARENT = Progman`，每次 `SetWindowPos` 都强制 `HWND_BOTTOM`

**Decision**: Option 2。owned 窗口总在 owner 之上，`HWND_BOTTOM` 实际落在"紧贴 Progman 之上"——所有应用窗口之下、桌面图标之上，
Win+D 时随 Progman 一起被抬起。Option 1 在旧版 spike 中 DPI 退化、丢圆角。强制置底在旧版用 tao 的 `always_on_bottom`，GPUI 不处理
`WM_WINDOWPOSCHANGING`，改为 `SetWindowSubclass` 子类过程（`platform/subclass.rs` 的 `FLAG_BOTTOM`；不用 `GWLP_USERDATA`，GPUI 占用它）。

**Extensibility**: Win10 / Win11 ≤ 23H2 显示桌面时抬起的是 `WorkerW` 而不是 Progman；Rainmeter 式的哨兵状态机可以加进 200ms 轮询，
不影响 attach / detach 接口。

### Design Decision: "嵌入"是固定模式的偏好，不是第三种模式

**Context**: 旧版最初做成了独立的"桌面模式"按钮和托盘项，与"固定模式"互斥，用户觉得两个锁按钮令人困惑。

**Decision**: 用户可见的只有一种模式（固定）加一个仅 Windows 的设置开关 `fixed_embed_desktop`。窗口侧因 Win32 处理完全不同保留两个标志
（`classic_fixed` / `desktop`），由 `TargetMode::for_state(is_fixed, embed)` 映射；`window_layouts` 只存 `is_fixed`。

### Design Decision: 按窗口的状态表 + 集中观察偏好（3.1）

**Context**: 主窗口拆成多个清单窗口 + 日历窗口后，每个窗口要各自锁定、各自贴边隐藏，嵌入 / 贴边 / 唤起置顶仍是全局偏好。

**Decision**: `window_mode` 持有 `BTreeMap<BlockKey, Block>`，每窗口一份纯状态机 `ModeState`，Win32 / AppKit 操作照旧"update 里算计划、update 外执行"；
全局偏好由 `window_mode` 订阅一次 `AppState`，不让每个窗口各自观察（否则 N 个窗口对同一次偏好变化各切一次模式）。关闭 = 隐藏、不注销，
唤起 / 重置 / 轮询 / 吸附照常找得到隐藏的窗口；隐藏的窗口启动时不建，省掉用不到的窗口资源。

**Consequences**: 清单 / 日历窗口改成自绘移动与缩放（同收纳箱，进吸附组），失去 Aero Snap 与标题栏双击最大化；托盘「固定模式」随之删除。

### Design Decision: 嵌入核心按窗口句柄执行，清单 / 日历窗口与收纳箱共用

**Context**: 桌面收纳箱（多个窗口）需要与"固定 + 嵌入桌面"的主窗口完全相同的处理（owner=Progman + TOOLWINDOW + 置底、Explorer 重启重挂），
但只有"始终嵌入"一种形态。

**Decision**: `platform::embed` 的函数都只接收 `Hwnd`、不碰任何模式状态，挂上的宿主与嵌入前的 MINIMIZEBOX 记在 embed 内部的登记表里；
清单 / 日历窗口的模式状态在 `services::window_mode`，收纳箱的窗口登记在 `services::fences`（[desktop-fences.md](./desktop-fences.md) §3.1）。
Explorer 重启判定共用纯函数 `window_logic::needs_reattach`。macOS 没有对应的宿主窗口：清单 / 日历窗口没有嵌入桌面，收纳箱另走
`Hwnd::make_desktop_layer`（层级 + collectionBehavior，不按句柄登记、不需要重挂）。
