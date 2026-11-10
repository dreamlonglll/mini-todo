# 第 3 步（多窗口底座）实现说明与待同步的文档

> 范围：`pc-gpui/crates/app`。`window_mode` 按窗口、几何与锁定的持久化接缝、唤起 / 重置作用于所有窗口、吸附组服务通用化、
> 主窗口（= 默认清单窗口）自绘移动与缩放。界面上看不出拆分；拆窗是第 4 步。本文件只记录**需要同步进 spec / README 的内容**
> 与第 4 步要注意的事，本步没有改 `CLAUDE.md` 与 `.trellis/spec/**`。

## 0. 新的结构一览

| 位置 | 内容 |
|---|---|
| `windows/block.rs` | `BlockKey { List(i64), Calendar }`（`DEFAULT_LIST = List(1)` = 主窗口）、`DEFAULT_LIST_ID = 1`；`BlockDrag` + `BlockDragHost`（桌面块窗口的自绘移动 / 缩放）、`min_physical_size` |
| `windows/drag.rs` | 自绘拖动公共部分（原在 `windows/fence/logic.rs` / `fence/mod.rs`）：`exceeds_drag_threshold`、`ResizeDir`、`js_round`、`resize_rect`、`cursor_point` / `cursor_delta`、`place_client_rect`（原 `services::fences::apply_window_rect` 的实现）、`EmptyDragView`、`resize_handle` / `resize_cursor`、`render_snap_guides(edges, radius)` |
| `windows/snap.rs` | 吸附纯逻辑（由 `windows/fence/snap.rs` 挪来）：`SnapKey { Fence(i64), Block(BlockKey) }`、`SnapTarget { key, rect }`、`SnapGuide { key, edge }`、`group_guides`、`changed_targets`（原 `changed_fences`）、`snap_move` / `snap_resize` / `snap_threshold`、`SnapScene::targets_at` |
| `services/snap.rs` | 吸附组运行时（原 `services::fences` 的 `snap_scene` / `set_snap_guides` / `clear_snap_guides` / `snap_edges` / `forget_snap`）：`register` / `unregister` / `scene` / `targets` / `set_guides` / `clear_guides` / `edges`、`Member { hwnd, view, suppressed }` |
| `services/window_mode/mod.rs` | 按窗口的运行时（登记、模式切换、200ms 轮询、唤起、重置、保存） |
| `services/window_mode/machine.rs` | 每个窗口的纯状态机 `ModeState`、`TargetMode`、`Op` / `Ops`、`TickFacts` / `TickPlan` / `AutoHideMove`、`Wake`、`classic_fixed_look`、`reset_rect` |
| `services/window_mode/store.rs` | 几何与锁定的持久化接缝：`BlockPlacement { rect, fixed }`、`load` / `save` |
| `theme.rs` | `snap_guide_color()`（`#409EFF`）、`SNAP_GUIDE_WIDTH`（2px）——替代删掉的 `fence_tokens().snap_guide` / `FENCE_SNAP_GUIDE_WIDTH` |

删掉的接口：`AppState.is_fixed`、`AppState::{is_embedded_in_desktop, is_classic_fixed, toggle_fixed_mode, save_window_state}`；
`window_mode::{WindowMode.main_hwnd …, is_window_fixed, on_main_opened, on_main_closed, bring_main_to_front, on_main_activated,
reset_main_window, on_main_close_requested, main_window}`；`main_window::show_and_activate`；`windows::fence::snap` 模块；
`services::fences::{snap_scene, set_snap_guides, clear_snap_guides, snap_edges}`。

## 1. `spec/backend/window-modes.md` 要改的地方

### 标题与引言
- 标题改成「Window Modes（清单 / 日历窗口：普通 / 经典固定 / 固定 + 嵌入桌面，按窗口）」；引言补一句：任务 `11-09-multi-list-windows`
  第 3 步把单例 `WindowMode`（只认一个 `main_hwnd`）改成按 `BlockKey` 的状态表，现在只有默认清单窗口（原主窗口）一个使用者。
- 实现位置：`services/window_mode/{mod.rs, machine.rs, store.rs}`。

### §2 Signatures（整段替换 `services/window_mode.rs` 与 `state.rs` 两块）

```rust
// crates/app/src/windows/block.rs
pub enum BlockKey { List(i64), Calendar }          // Ord：清单在前（按 id），日历最后
impl BlockKey { pub const DEFAULT_LIST: BlockKey = BlockKey::List(1); } // = 原主窗口

// crates/app/src/services/window_mode/machine.rs —— 每个窗口一份的纯状态机（全平台编译，有单测）
pub struct ModeState {
    pub fixed: bool,          // 用户可见的锁定（两种固定都为 true；按屏幕组合持久化）
    pub classic_fixed: bool,  // 窗口侧：经典固定
    pub desktop: bool,        // 窗口侧：固定 + 嵌入桌面（与 classic_fixed 不会同时为 true）
    pub auto_hide: AutoHideState,
    pub desired_topmost: bool,
    /* top_on_wake（全局偏好的每窗口缓存）、pending_release */
}
pub enum TargetMode { Normal, ClassicFixed, Desktop }
impl TargetMode { pub fn for_state(is_fixed: bool, embed_desktop: bool) -> TargetMode; } // embed 只在 Windows 生效
pub fn classic_fixed_look(fixed: bool, embed_desktop: bool) -> bool;   // 无圆角无描边
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
pub fn reset_rect(monitor: MonitorBounds, scale: f64, index: usize) -> PhysRect; // 第 0 个 = 以前的主窗口位置

// crates/app/src/services/window_mode/mod.rs —— 按窗口的运行时（Global：BTreeMap<BlockKey, Block>）
pub fn prepare(cx, key, fixed: bool);                                  // 建窗前：第一帧按锁定外观画
pub fn register(cx, key, handle: AnyWindowHandle, hwnd: Option<Hwnd>, view: EntityId); // 建窗后：轮询 + 吸附组 + 集中观察偏好
pub fn unregister(cx, key);
pub fn is_registered(cx, key) -> bool;  pub fn registered(cx) -> Vec<BlockKey>;
pub fn is_fixed(cx, key) -> bool;  pub fn is_classic_fixed(cx, key) -> bool;  pub fn is_edge_hidden(cx, key) -> bool;
pub fn apply_mode(cx, key, target) -> Task<Result<(), String>>;       // 没登记 → Err("{key} 不存在")
pub fn set_fixed(cx, key, fixed: bool);  pub fn toggle_fixed(cx, key); // 锁按钮 / 托盘「固定模式」/ 重置：切完保存
pub fn on_embed_preference_changed(cx, enabled: bool);               // 所有锁定的窗口
pub fn apply_runtime_prefs(cx, prefs: RuntimePrefs);                  // 所有窗口
pub fn bring_all_to_front(cx);  pub fn bring_to_front(cx, key);       // 托盘左键 / 单实例 / Dock；打开设置
pub fn on_activated(cx, key);                                         // macOS：贴边隐藏中被激活 → bring_all_to_front
pub fn reset_all(cx);                                                 // 托盘「重置位置」
pub fn schedule_save(cx, key);  pub fn save_window_state(cx, key);  pub fn save_window_state_with(cx, key, rect);
pub fn read_startup_geometry(cx, key) -> StartupGeometry;  pub fn on_close_requested(window, cx, key);
pub const TICK_INTERVAL: Duration = Duration::from_millis(200);

// crates/app/src/services/window_mode/store.rs —— 几何与锁定的持久化接缝（第 4 步换实现）
pub struct BlockPlacement { pub rect: Option<PhysRect>, pub fixed: bool }
pub fn load(db, key, screen_id) -> Result<Option<BlockPlacement>, String>;
pub fn save(db, key, screen_id, rect: PhysRect, fixed: bool) -> Result<(), String>;

// crates/app/src/state.rs —— AppState（只剩全局偏好）
pub fixed_embed_desktop: bool;   // 偏好开关（settings fixed_embed_desktop）；锁定本身按窗口在 window_mode
```

### §3 Contracts
- **Persisted state** 表：`screen_configs.is_fixed` 一行改为"默认清单窗口在每个屏幕组合下的锁定（经 `window_mode::store`；其它窗口第 4 步
  进窗口属性表）"；`settings.is_fixed` 一行补"默认清单窗口保存时顺带写（旧全局键，参与同步；不再驱动界面）"。
- 新增一段**按窗口**：每个登记的窗口一份 `ModeState`（锁定、经典固定 / 嵌入、贴边隐藏状态机、期望置顶、收尾标记）与保存防抖代次；
  嵌入桌面 / 贴边隐藏 / 唤起置顶是**全局偏好**、作用于每个锁定的窗口。`top_on_wake` 在每个窗口里各缓存一份，
  `window_logic::apply_runtime_prefs` 才能对每个窗口各自发现"被关掉了"（共用一份缓存时只有第一个窗口会收尾）。
- **偏好开关的跨窗口契约** 第 2 条改为：`window_mode` 在第一个窗口登记时 `cx.observe(AppState)`（`ensure_prefs_observer`），
  发现 `fixed_embed_desktop` / `runtime_prefs` 变了就 `defer` 调 `on_embed_preference_changed` / `apply_runtime_prefs`，对所有窗口生效；
  窗口自己不再观察这两项（原来是主窗口 `on_state_changed` 里做的）。第 3 条（设置窗口不得保存窗口状态）不变。
- **Ordering rules** 第 1 条的 `plan_mode` / `plan_tick` 改为 `ModeState::plan_mode` / `plan_tick`（纯函数）；200ms 轮询每个 tick
  对所有存活的窗口各算一份 `TickPlan`（显示器只枚举一次、光标只读一次），再逐个执行；第 2 条"执行 `Op::Attach` 前重新检查"
  改为检查**该窗口**的 `desktop`；第 8 条托盘勾选：`apply_mode` 只对 `DEFAULT_LIST` 回写（第 5 步删掉这一项）。
- **唤起**：托盘左键 / 单实例 / Dock（`on_reopen`）/ 贴边隐藏中被激活 → `bring_all_to_front`：一次唤起只开一个托盘唤起代次
  （`begin_tray_wake`），所有非嵌入的窗口一起临时置顶，600ms 后各按自己的贴边状态决定撤不撤；默认清单窗口最后激活（在最上面、拿焦点）。
  托盘「打开设置」/ 应用菜单「设置…」只唤起默认清单窗口（`bring_to_front`）。默认清单窗口不在时重新打开它。
- **重置位置**：`reset_all` 对所有登记的窗口：清贴边状态、撤置顶、`machine::reset_rect(主显示器, 缩放, 序号)`（第 0 个 = 以前的主窗口：
  10% 边距、380×600 逻辑；之后每个向右下错开 32 逻辑像素，超出显示器从第 0 个位置重来）；锁定的 `set_fixed(false)`（切完保存），其余保存。
- **保存**：系统 / 程序挪窗（开关日历加宽、DPI、显示器变化）仍 500ms 防抖保存；**用户自绘拖动 / 缩放中不防抖保存**，松手 16ms 后
  `BlockDrag` 直接 `save_window_state` 一次。几何与锁定经 `store::save`；默认清单窗口另写旧全局键（`save_window_settings`）并刷新运行时偏好。

### §4 Validation & Error Matrix
- "主窗口不存在 → `Err("主窗口不存在")`" 改为"窗口没登记 → `Err("{key} 不存在")`（如 `清单窗口 #1 不存在`）；`set_fixed` / `toggle_fixed`
  对没登记的窗口只记 warn、什么都不改（以前托盘「固定模式」在主窗口关掉时会把 `AppState.is_fixed` 翻过去）"。

### §6 Tests Required（单测位置变了）
- `target_mode_follows_embed_preference_only_on_windows` → `services/window_mode/machine.rs`；
  原 `state.rs` 的 `embedded_only_on_windows_when_fixed` → `machine.rs::classic_look_only_when_fixed_without_embedding`。
- 新增 `machine.rs`：`mode_switches_keep_the_win32_ordering_rules`、`leaving_fixed_mode_pulls_an_edge_hidden_window_back_first`、
  `embed_preference_switches_only_locked_windows_on_windows`、`tick_guards_each_mode_independently`、`edge_hiding_runs_per_window`、
  `turning_a_preference_off_releases_every_window`、`tray_wake_pulls_hidden_windows_back_and_releases_the_topmost_later`、
  `reset_keeps_the_first_window_where_the_main_window_used_to_go`、`reset_wraps_around_instead_of_leaving_the_monitor`；
  `store.rs`：`default_list_round_trips_through_screen_configs`、`a_record_without_a_size_keeps_only_the_lock`、`other_windows_are_not_stored_yet`；
  `windows/block.rs`：`keys_order_the_default_list_first_and_the_calendar_last`、`minimum_size_follows_the_physical_scale`、
  `resizing_the_main_window_stops_at_its_minimum_and_keeps_the_far_edges`、`resize_snapping_respects_the_main_window_minimum`。
- **E2E 要改（第 6 步）**：S01 不再有 `HTCAPTION`（标题栏 `WM_NCHITTEST` = `HTCLIENT`）、不再有 `HTLEFT`（不可缩放：外框外沿是 `HTBORDER`）、
  没有 `WS_THICKFRAME`（`f["thickFrame"]` 为 false）、**双击标题栏不再最大化 / 还原**；拖标题栏移动 + 落库照常（自绘）。S07 的"标题栏又能拖动"
  不能再断言 `HTCAPTION`，改成真实拖动能移动。新增：拖主窗口 / 收纳箱互相吸附（参照 F13–F15）、主窗口 8 个手柄缩放 + 最小 320×400 + 落库。

### Scenario: macOS
- §2 表"普通"一行：拖动 / 缩放改为"自绘（标题栏 `BlockDrag`、窗口内 8 个手柄；`setFrameOrigin:` / `setFrame:`，不再 `start_window_move`）；
  建窗不带 `Resizable`"。"经典固定"一行：`LockGeometry(true)` 照旧 `setMovable:NO`（只拦 AppKit 自己的拖动，自绘拖动在锁定时本来就不挂）；
  去掉 `Resizable` 现在是空操作（建窗时就没有），解锁后也不恢复。
- §4 唤起与重置：`bring_main_to_front` → `bring_all_to_front`（菜单栏图标左键、单实例、Dock `on_reopen`、`on_activated`）/
  `bring_to_front(DEFAULT_LIST)`（应用菜单「设置…」）；托盘「重置位置」= `reset_all`。
- §5 Tests：M10 的"AX 改尺寸"探测在普通模式下多半也改不了（没有 `Resizable`），脚本会打印"普通模式下 AX 改尺寸就没生效"并跳过固定模式的
  尺寸断言——要么接受，要么改成断言"手柄缩放能改、固定后手柄消失"。M10 的拖动点 `w.y + 6` 在 N 手柄（顶部 5pt）之下，仍是移动。

### Design Decisions 补一条
- **按窗口的状态表 + 集中观察偏好**：Context = 清单 / 日历窗口各自锁定；Decision = `window_mode` 持有 `BTreeMap<BlockKey, Block>`，
  每窗口一份纯状态机 `ModeState`，Win32 / AppKit 操作照旧"update 里算计划、update 外执行"；全局偏好由 `window_mode` 订阅一次
  `AppState`，不让每个窗口各自观察（否则 N 个窗口对同一次偏好变化各切一次模式）。

## 2. `spec/backend/desktop-fences.md` 要改的地方

- 引言实现清单：`windows/fence/` 里去掉 `snap.rs`；`services/fences.rs` 的描述里"吸附高亮的跨窗口状态"改为"登记进吸附组（`services::snap`）"；
  加上 `windows/drag.rs`（拖动阈值、缩放矩形、光标换算、按客户区摆窗、手柄、高亮叠层，与清单 / 日历窗口共用）、`windows/snap.rs`、`services/snap.rs`。
- §2 Signatures：
  - `services::fences` 块删掉 `snap_highlights` / `snap_owner` 字段与 `snap_scene` / `set_snap_guides` / `clear_snap_guides` / `snap_edges`；
    `apply_window_rect` 注明委托 `windows::drag::place_client_rect`。
  - `windows::fence` 的 `logic.rs` 一行去掉 `exceeds_drag_threshold` / `resize_rect` / `ResizeDir`（挪到 `windows::drag`）。
  - `snap.rs` 一段改为 `windows/snap.rs`：`SnapKey { Fence(i64), Block(BlockKey) }`、`SnapTarget { key, rect }`、`SnapGuide { key, edge }`、
    `group_guides(..) -> HashMap<SnapKey, SnapEdges>`、`changed_targets(old, new) -> Vec<SnapKey>`；`SNAP_THRESHOLD_DIP` / `snap_threshold` /
    `snap_move` / `snap_resize` / `SnapScene::targets_at` 签名不变。
  - 新增 `services/snap.rs`：`pub struct Member { hwnd: Option<Hwnd>, view: EntityId, suppressed: fn(&App, SnapKey) -> bool }`、
    `register` / `unregister`（清掉它留下的、它身上的高亮）/ `scene(cx, exclude)` / `targets(scene, alt)` / `set_guides` / `clear_guides` / `edges`。
  - theme：`fence_tokens().snap_guide` / `FENCE_SNAP_GUIDE_WIDTH` → `theme::snap_guide_color()` / `theme::SNAP_GUIDE_WIDTH`（值不变）。
- §3.1「吸附」表：
  - 新增首行"**吸附组**：收纳箱、清单窗口、日历窗口（任务 `11-09-multi-list-windows` R10；现在是收纳箱 + 主窗口），双向吸附，规则相同"。
  - 「目标」：除自己以外、吸附组里当前显示中的窗口（含锁定的；**最小化的、收纳箱"全部隐藏"中、清单 / 日历窗口贴边藏起时不算**），
    快照在 `services::snap::scene`，按 `SnapKey` 排序（收纳箱在前按 id，清单 / 日历窗口在后按 `BlockKey`）。
  - 「高亮状态」：`services::snap` 的 `SnapRuntime.highlights` + `owner`；窗口注销（关闭 / 删除）时 `unregister` 清掉。
  - 「高亮的样子」：`windows::drag::render_snap_guides(edges, radius)`，收纳箱 `radius::MD`；清单窗口跟着自己的圆角（经典固定为 0）。
- §6 Tests：`windows/fence/snap.rs` 的测试整体改到 `windows/snap.rs`（名字不变，只是目标 / 高亮的 id 换成 `SnapKey`），另新增
  `keys_sort_fences_first_then_lists_then_the_calendar`、`a_list_window_snaps_to_fences_and_fences_snap_to_list_windows`、
  `mixed_targets_break_ties_by_key_order_and_diff_per_window`；`logic.rs` 的 `drag_threshold` / `resize_vectors` 改到 `windows/drag.rs`。
- §5 / 「移动」「缩放」正文里的 `logic::exceeds_drag_threshold` / `logic::resize_rect` / `cursor_point` 改成 `drag::…`；行为不变。

## 3. `spec/pc-gpui/gpui-conventions.md`

- §2.3 拖动窗口：
  - "普通标题栏：`ui::title_bar::drag_region()`"保留，但注明**主窗口（清单 / 日历窗口）不再用它**：它们与收纳箱一样自绘移动 + 窗口内 8 个缩放手柄
    （`windows::block::BlockDrag`，建窗 `resizable: false`），代价是没有 Aero Snap、没有标题栏双击最大化。
  - "不可拖动的状态（主窗口固定模式）不放拖动区即可"改为"锁定时 `BlockDrag::title_bar(.., can_move = false, ..)` 不挂拖动、
    `resize_handles(false, ..)` 不出手柄"。
  - 自绘拖动一段补：**手柄的层次**——收纳箱的手柄在内容之上且 `occlude()`；清单窗口的手柄放在**标题栏之后、内容之前**、不 `occlude()`：
    标题栏区域的边与角在最上面能缩放，其余边上内容在上面（列表滚动条滑块按下即 `stop_propagation`，照常能拖；轨道与留白处缩放），
    手柄按下时 `stop_propagation`（标题栏不会同时开始拖动、根元素的收尾监听收不到）。光标停在滚动条滑块上也显示缩放光标（滑块没有自己的光标样式）。
  - 实现：`windows/fence/mod.rs`（收纳箱）、`windows/block.rs`（清单 / 日历窗口），公共部分 `windows/drag.rs`。
- §2.15 macOS：「`app_owns_titlebar_drag`」一条里"拖窗走 `start_window_move`（`ui::title_bar::drag_region` 的非 Windows 分支…）"改为
  "主窗口与收纳箱都是自绘拖动，编辑等窗口的 `drag_region` 才走 `start_window_move`"；§3 升级核对清单里"`start_window_move` 是否仍是
  `performWindowDragWithEvent:`（主窗口的 `drag_region`）"改为"（编辑等窗口的 `drag_region`）"，并把"收纳箱的自绘拖动"扩成"收纳箱与清单窗口的自绘拖动"。

## 4. `crates/app/README.md`

- §2 模块地图：`windows/` 下加 `block.rs`（BlockKey、BlockDrag）、`drag.rs`、`snap.rs`；`fence/` 一行去掉 `snap`；`services/` 的
  `window_mode` 改成目录（`mod.rs` / `machine.rs` / `store.rs`），加 `snap`。
- §3 约定 6「拖动窗口」：主窗口改为自绘拖动（同收纳箱）；`drag_region` 只剩编辑 / 子任务 / 已完成 / 设置等窗口。
- §4.1 AppState：字段表去掉 `is_fixed`；方法表去掉 `is_embedded_in_desktop()` / `is_classic_fixed()` / `toggle_fixed_mode()` / `save_window_state()`
  （锁定按窗口：`window_mode::is_fixed / is_classic_fixed / toggle_fixed(cx, key)`）。
- §4.2 services：`window_mode` 一行改为"桌面块窗口（清单 / 日历；现在只有主窗口）按窗口的普通 / 经典固定 / 嵌入桌面、贴边隐藏与唤起、
  几何与锁定的持久化接缝（`store`）、唤起所有窗口（`bring_all_to_front`）/ 重置位置（`reset_all`，错开排列、解除锁定）、200ms 轮询遍历所有窗口；
  集中观察 AppState 的偏好"；新增 `snap` 一行；`fences` 一行的"吸附对齐的跨窗口部分 …"改为"登记进吸附组（`services::snap`）"；
  `single_instance` 一行"第二实例唤起 `main_window::show_and_activate`"改为"`window_mode::bring_all_to_front`"。
- §4.3：`main_window::show_and_activate(cx)` 删掉；主窗口一行"可缩放"改为"自绘移动 + 窗口内 8 个缩放手柄（建窗不可缩放、最小 320×400），
  与收纳箱互相吸附"。

## 5. `CLAUDE.md`（合并时一起改，本步没动）

- 「窗口一览」主窗口一行："380×600，最小 320×400，透明，自绘移动与缩放（不可系统缩放、没有 Aero Snap / 双击最大化），开日历时加宽"。
- 「窗口特殊功能 → 普通模式」："可拖动、可缩放（标题栏自绘拖动、窗口内 8 个手柄，吸附收纳箱）"。
- 「桌面收纳箱 → 吸附对齐」："靠近其它显示中的收纳箱**或主窗口**…"；跨窗口状态在 `services::snap`。
- 「项目结构」`services/` 加 `snap`，`window_mode` 是目录。
- 「核心架构 → 跨窗口状态与通信」：托盘左键 / 单实例 / Dock 唤起 = `window_mode::bring_all_to_front`。

## 6. 行为与以前不同的地方（除失去 Aero Snap 外）

1. 主窗口建窗改为 `resizable: false`：没有 `WS_THICKFRAME` / `WS_MAXIMIZEBOX`，**标题栏双击不再最大化 / 还原**，Win+↑ 不再最大化；
   缩放热区从"可见边缘外的不可见边框（左 / 右 / 下）+ 顶部一条"变成"可见边缘内侧 5px（角 10px）的手柄"。
2. 有模态子窗口（编辑 / 设置 / 已完成）打开时，主窗口不能缩放了（遮罩挡住手柄；以前可以从系统边框缩放）。移动以前就不行，不变。
3. 用户拖动 / 缩放不再走 500ms 防抖保存，松手 16ms 后保存一次；系统挪窗照旧防抖保存。
4. 托盘「固定模式」在主窗口关掉时不再把内存里的锁定状态翻过去（以前会翻、但切换失败），勾选按真实状态回写。
5. 主窗口加入吸附组：拖收纳箱会吸主窗口的边并在主窗口上画高亮，拖主窗口会吸收纳箱。
6. Linux（只用于 CI / 截图）：主窗口拖标题栏不再移动窗口（GPUI 在 Linux 上不能程序化移动窗口；以前走 `start_window_move`）。缩放仍按尺寸生效。
7. 切换失败的提示文字从"主窗口不存在"变成"清单窗口 #1 不存在"（只在窗口不存在时出现，实际看不到）。

## 7. 第 4 步要注意的事

- **持久化接缝**：只换 `services/window_mode/store.rs` 的 `load` / `save` 两个函数体（改读写 core 的窗口属性表，按 `(BlockKey, screen_id)`
  存客户区物理像素矩形 + 锁定）；调用方（`read_startup_geometry`、`persist_window_state`）不用改。同时决定 `persist_window_state` 里默认清单窗口
  "顺带写旧全局键"（`save_window_settings`：`is_fixed` / `window_position` / `window_size` / 主题…）去留——`is_fixed` 是同步键、旧设备还在读写，
  `window_position` / `window_size` 随导出备份；`text_theme` 等变成按窗口后不该再由窗口保存写回全局键。迁移 v31 的初始布局（默认清单继承
  `screen_configs`）要和 `load` 的口径一致（客户区、物理像素、`width/height > 0` 才算有几何）。
- **登记顺序**：新窗口照主窗口的写法——`prepare(key, fixed)`（建窗前；`open_window` 会同步画第一帧）→ `open_window` →
  `register(key, handle, hwnd, view_entity_id)` → 锁定的 `apply_mode(key, TargetMode::for_state(true, embed))`；关窗回调里 `unregister(key)`。
  关闭 = 隐藏时**不要** `unregister`（否则唤起、重置、轮询、吸附都找不到它），改成隐藏 / 显示即可；`Op::Restore` 会把隐藏的窗口重新显示，
  `bring_all_to_front` 本来就会唤起隐藏的窗口；吸附组按 `Hwnd::is_visible` 自动排除隐藏的窗口。
- **`ensure_default_list`**（`window_mode/mod.rs`）现在在默认清单窗口没登记时调 `main_window::open`；拆窗后改成打开 / 显示默认清单窗口。
  `reset_all` 在默认清单窗口不在时只重新打开它、不重置其它窗口——关闭 = 隐藏后这条分支不再出现，可以删掉。
- **窗口视图**：根视图实现 `BlockDragHost`，持有 `BlockDrag::new(key, min_w, min_h)`；标题栏 `drag.title_bar(bar, !is_fixed, cx)`；
  根元素 `drag.root(root, cx)`，子元素顺序 **标题栏 → `drag.resize_handles(!is_fixed, cx)` → 内容 → 遮罩 → 高亮叠层**；
  `observe_window_bounds` 里 `!drag.is_busy()` 才 `schedule_save`；激活回调 `defer(on_activated(key))`；关闭请求 `on_close_requested(key)`；
  渲染时读 `services::snap::edges(cx, SnapKey::Block(key))` 画高亮。双击清单名改名：`BlockDrag` 没有收纳箱那套"按下记 click_count、松手按距离判断"，
  要的话照 `windows/fence/mod.rs` 的 `TitlePress.double` 加，不要用挂了 `on_drag` 的元素上的 `on_click`（第二下抖 2~3px 就收不到）。
- **偏好观察**：嵌入 / 贴边 / 唤起置顶的收尾已经集中在 `window_mode`，新窗口**不要**再自己观察这两项；日 / 夜、视图、展开全部变成按窗口时，
  `is_classic_fixed` 的外观判断不受影响（只看锁定 + 嵌入偏好）。
- **托盘 / 菜单**：`apply_mode` 只对 `DEFAULT_LIST` 回写托盘「固定模式」勾选（第 5 步删掉该项时一起删）；`toggle_fixed(DEFAULT_LIST)` 是那一项的入口。
- **吸附目标的顺序**：`SnapKey` 的 `Ord` 让收纳箱排在清单 / 日历窗口前面（同距离时先吸收纳箱）；日历窗口排在所有清单之后。
- **已完成 / 编辑等子窗口的打开方**：`WindowManager::open_child` 的"打开方是主窗口时关闭后 `reload_todos`"、`main_has_modal_child`、
  `WindowKind::Main` 这些仍按"一个主窗口"（本步没动），拆窗时要按清单窗口改。
- **风险（需要真机看）**：Windows 上不可缩放窗口（无 `WS_THICKFRAME`）的 DWM 阴影 / 不可见边框厚度与以前可能略有不同（几何一律按客户区，
  记录不受影响）；跨不同 DPI 的显示器拖动时 GPUI 处理 `WM_DPICHANGED` 会按建议矩形摆一次，下一帧又按光标摆回——收纳箱一直是这样，主窗口第一次；
  光标停在列表滚动条滑块上显示的是左右缩放光标（滑块按下照常拖滚动条）。

## 8. 本地无法编译验证的 mac 代码

本步没有改 `platform/macos/*`、`ns.rs`、`mac_geometry.rs`、`scripts/mac-check` 覆盖的文件。改到的 `cfg(target_os = "macos")` 代码只有
`services/window_mode/mod.rs` 的 `bring_all_to_front` / `bring_to_front` 开头的 `cx.activate(true)`（与改之前的 `bring_main_to_front` 同一写法）；
`services/mac_menu.rs` 全平台编译（本地 clippy 覆盖）。只在 Linux 上编译的分支：`windows/drag.rs::place_client_rect` 的
`cfg(not(any(windows, target_os = "macos")))` 分支（从 `services::fences::apply_window_rect` 原样挪来）。建议推 `claude/**` 分支跑
`pc-gpui-ci.yml`（Linux / macOS clippy）与 `pc-gpui-e2e-macos.yml`。
