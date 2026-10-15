# Window Modes（主窗口：普通 / 经典固定 / 固定 + 嵌入桌面）

> 主窗口 Win32 样式、owner、Z 序与几何锁定的契约。实现：`pc-gpui/crates/app/src/services/window_mode.rs`（模式切换、200ms 轮询、
> 贴边隐藏、位置与屏幕配置记忆）+ `platform/embed.rs`（嵌入桌面层，主窗口与收纳箱共用）+ `platform/subclass.rs`（永远置底、
> 几何锁定）+ `platform/windows.rs`（`Hwnd` 原始调用）；纯逻辑在 `pc-gpui/crates/core/src/window_logic.rs`。
>
> 历史：建立于任务 `09-07-desktop-embed-mode`（2026-09，旧 Tauri 版，Win11 25H2 实测），`10-15-desktop-fence` 把嵌入核心泛化为
> 按窗口句柄执行、供收纳箱复用，`10-15-gpui-rewrite` 改为 GPUI 实现。旧 Tauri 版为对抗 tao `WindowFlags::apply_diff` 重写 ex style
> 而写的排队重申（`reassert_window_mode_state`、`run_on_main_thread` 排序规则）在 GPUI 下不再需要（GPUI 建窗后不改写 ex style），
> 已随旧版删除；保留的是 §3 的状态表、"嵌入不是第三种模式"、持久化键与 Explorer 重启 / 样式自愈的语义。

---

## Scenario: Main window mode switching

### 1. Scope / Trigger

- 改动以下任何东西前先读本文件：`services/window_mode.rs` 的 `plan_mode` / `apply_mode` / `plan_tick` / `run_tick`、
  `platform::embed::*`、`platform::subclass`（`FLAG_BOTTOM` / `FLAG_LOCK_GEOMETRY`）、`Hwnd::{set_tool_window, set_minimizable,
  set_topmost, set_geometry_locked, set_owner}`、core `window_logic` 的 `needs_reattach` / `lacks_tool_window_style` / 贴边状态机、
  settings 键 `is_fixed` / `fixed_embed_desktop` / `auto_hide_enabled` / `top_on_wake` 与 `screen_configs.is_fixed`。
- `platform::embed` 是主窗口与所有收纳箱窗口共用的嵌入核心（收纳箱的登记与守护见 [desktop-fences.md](./desktop-fences.md) §3.1）：
  改它要两边都回归。
- 只在 Windows 上生效；Linux 上 `platform::embed` / `subclass` 是安全的空实现，"嵌入"偏好按经典固定处理（§4）。

### 2. Signatures

```rust
// crates/app/src/services/window_mode.rs —— 主窗口运行时状态（Global）
pub struct WindowMode {
    pub main_hwnd: Option<Hwnd>,
    pub classic_fixed: bool,   // 经典固定
    pub desktop: bool,         // 固定 + 嵌入桌面（与 classic_fixed 不会同时为 true）
    pub auto_hide: AutoHideState,
    pub desired_topmost: bool, // 贴边唤起 / 托盘唤起时的期望置顶态
    /* top_on_wake、pending_release、save_generation、tick_running */
}
pub enum TargetMode { Normal, ClassicFixed, Desktop }
impl TargetMode { pub fn for_state(is_fixed: bool, embed_desktop: bool) -> TargetMode; } // embed 只在 Windows 生效
pub fn apply_mode(cx: &mut App, target: TargetMode) -> Task<Result<(), String>>;      // 主窗口不存在 → Err("主窗口不存在")
pub fn toggle_fixed(cx: &mut App, fixed: bool);                    // 锁按钮 / 托盘「固定模式」：切换后保存窗口状态
pub fn on_embed_preference_changed(cx: &mut App, enabled: bool);   // 已固定时偏好变化：两种固定之间直接切换
pub fn is_window_fixed(cx: &App) -> bool;
pub const TICK_INTERVAL: Duration = Duration::from_millis(200);    // 嵌入重挂 / 经典固定守护 / 贴边隐藏
pub fn read_startup_geometry(cx: &App) -> StartupGeometry;         // 当前屏幕组合的 screen_configs 记录 + fixed_embed_desktop

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

// crates/app/src/state.rs —— AppState
pub is_fixed: bool;                    // 两种固定都为 true
pub fixed_embed_desktop: bool;         // 偏好开关（settings fixed_embed_desktop）
pub fn is_embedded_in_desktop(&self) -> bool;  // IS_WINDOWS && is_fixed && fixed_embed_desktop
pub fn is_classic_fixed(&self) -> bool;        // is_fixed && !is_embedded_in_desktop()
pub fn toggle_fixed_mode(&mut self, cx);       // 翻转 is_fixed → defer window_mode::toggle_fixed
pub fn set_fixed_embed_desktop(&mut self, enabled, cx) -> Task<Result<(), String>>; // 只写键（失败回滚）
```

### 3. Contracts

**Persisted state**

| Store | Key / column | Meaning | Synced (export / WebDAV) |
|---|---|---|---|
| `settings` | `is_fixed` | main window currently fixed (classic or embedded) | yes |
| `settings` | `fixed_embed_desktop` (v27, default `'false'`) | preference: fixed mode embeds into desktop | yes (`AppSettings.fixed_embed_desktop`, `#[serde(default)]`) |
| `settings` | `auto_hide_enabled`, `top_on_wake` | classic-fixed-only behaviours; ignored while embedded | yes |
| `screen_configs` | `is_fixed` | per display-combination restore value | no (device-specific) |

There is **no** `screen_configs.is_desktop` column and no `is_desktop` key: "embedded" is not a third mode, it is
`is_fixed && fixed_embed_desktop`. 启动时 `main_window::open` 读 `read_startup_geometry`（当前屏幕组合的 `screen_configs.is_fixed`），
固定则 `apply_mode(TargetMode::for_state(true, embed_desktop))`。

**各模式的 Win32 状态**（Windows e2e S 组断言的就是这张表）

| 模式 | `classic_fixed` | `desktop` | owner（`GWLP_HWNDPARENT`） | ex style | `WS_MINIMIZEBOX` | 几何锁定（`FLAG_LOCK_GEOMETRY`） | Z 序 / 置顶 | 圆角与描边 |
|---|---|---|---|---|---|---|---|---|
| 普通 | false | false | 0 | `WS_EX_APPWINDOW`，无 TOOLWINDOW | 有 | 否 | 普通顶层窗口 | 有 |
| 经典固定 | true | false | 0 | `WS_EX_TOOLWINDOW`，无 APPWINDOW | 有 | 是 | 顶层；只在贴边唤起（`top_on_wake`）/ 托盘唤起时置顶 | 无 |
| 固定 + 嵌入桌面 | false | true | Progman（`GetShellWindow()`） | `WS_EX_TOOLWINDOW`，无 APPWINDOW，无 TOPMOST | **无** | 是 | `FLAG_BOTTOM`：每次改 Z 序都压到 `HWND_BOTTOM` = 紧贴 Progman 之上 | 有 |

**偏好开关的跨窗口契约**

1. 设置窗口 → `AppState::set_fixed_embed_desktop(v)`：先改状态再写键（`settings::set_fixed_embed_desktop`），失败回滚。
2. 主窗口 `cx.observe(&state)` 发现 `fixed_embed_desktop` 变了 → `defer` 调 `window_mode::on_embed_preference_changed`：
   只有已固定且在 Windows 上才在两种固定之间直接切换（不保存窗口状态）；未固定时只是键变了，下次固定时生效。
   同步应用了远端设置后重读设置，走同一条路径。
3. 设置窗口**不得**调用 `save_window_state`（会把设置窗口的状态当成主窗口的写进去；`AppState::save_window_state` 注释同样约定）。

**Ordering rules**

1. **App 借用期间不碰 Win32**：`apply_mode` 在 `cx.update` 里用 `plan_mode` 只算出一组 `Op` 并更新 `WindowMode`，再在 async 块里、
   `update` 闭包之外逐个执行（`Hwnd` 原始调用会同步给 GPUI 窗口过程发消息，App 被借用时 GPUI 处理不了，几何状态会错乱；
   见 [pc-gpui/gpui-conventions.md §2.2](../pc-gpui/gpui-conventions.md)）。200ms 轮询同理：`plan_tick` 算、`run_tick` 执行。
2. 排队期间模式可能又变了：`run_tick` 执行 `Op::Attach` 前重新检查 `mode.desktop`，已退出嵌入就跳过。
3. 嵌入 → 经典固定：`detach_from_desktop(hwnd, tool_window = true)`，全程保持 `WS_EX_TOOLWINDOW`，不闪任务栏。
4. 经典固定 → 嵌入 / 普通：先撤置顶（`Topmost(false)`）再离开经典固定，否则解锁后一直置顶；贴边隐藏中的窗口先拉回锚点
   （`AutoHideState::leave_fixed_mode`）。
5. `detach_from_desktop` 先关 `FLAG_BOTTOM`，否则它最后的 `HWND_TOP` 会被子类过程改回 `HWND_BOTTOM`。
6. owner 用 `SetWindowLongPtrW(GWLP_HWNDPARENT, Progman)`，**不用 `SetParent`**（子窗口化后 DPI 与圆角退化，见 Design Decisions）。
7. 几何锁定只能在 hit-test 层拦：GPUI 的缩放热区与拖动区按**建窗时**的 `is_resizable` 计算、没有运行时开关，所以固定模式用
   `FLAG_LOCK_GEOMETRY`（`WM_NCHITTEST` 回 `HTCLIENT` / `HTBORDER`，`WM_SYSCOMMAND` 拦移动 / 缩放 / 最大化）。
8. 托盘「固定模式」勾选由 `apply_mode` 显式回写（`tray::set_fixed_checked`，两种固定都勾选），不依赖 muda 的自动翻转。
9. **按实际状态自愈**（200ms 轮询，只在 Windows、主窗口存活时）：嵌入态 `platform::embed::needs_reattach` 为真 → `Attach`；
   经典固定态 `lacks_tool_window_style` → 补 `WS_EX_TOOLWINDOW`、被 Win+D / Win+M 最小化 → 立即还原（固定模式没有任务栏入口）。

### 4. Validation & Error Matrix

| Condition | Behaviour |
|---|---|
| 主窗口不存在 | `apply_mode` → `Err("主窗口不存在")`；`toggle_fixed` 记日志并在主窗口提示 `切换固定模式失败：…` |
| 非 Windows 且 `fixed_embed_desktop = true`（如同步来的） | `TargetMode::for_state` 只在 Windows 上选 `Desktop`，其它平台回退为经典固定；`on_embed_preference_changed` 不做事；固定不会因此失败 |
| `GetShellWindow()` 为空（Explorer 未运行） | `desktop_host()` → `None`，`attach_to_desktop` 返回 false、不挂；`shell_now == 0` 时 `needs_reattach` 为 false，轮询等 Explorer 回来再挂 |
| shell 窗口类名不是 `Progman`（Win10 / Win11 ≤ 23H2 / 第三方 shell） | 记一次 warn，**仍然挂上**——不最小化、不进任务栏、在所有应用窗口之下；Win+D 可能盖住它（已知限制） |
| Explorer 重启（新 Progman） | 下一个 tick `needs_reattach`（旧 owner 已死 / shell 句柄变了）→ 重挂到新 Progman；主窗口不随旧 owner 销毁 |
| 嵌入态 ex style 被改成 APPWINDOW / 丢了 TOOLWINDOW | `needs_reattach` 里的 `lacks_tool_window_style` 命中 → 重挂（GPUI 下是防御：GPUI 本身不改写 ex style） |
| 经典固定被 Win+D / Win+M 最小化 | 下一个 tick 还原并显示 |
| 未固定时切换偏好 | 只写键；下次固定时按新偏好进入 |
| 旧备份 / 远端文档里没有 `fixedEmbedDesktop` | `#[serde(default)]` → false；导入不会失败 |

### 5. Good/Base/Bad Cases

- **Good**: 偏好开着，点锁按钮 → owner = 桌面宿主、TOOLWINDOW、无 MINIMIZEBOX；Win+D 后仍可见；普通应用窗口能盖住它；
  重启 Explorer 后自动重挂；杀进程重启后按 `screen_configs.is_fixed` + 偏好恢复同样的模式与位置。
- **Base**: 偏好关着，点锁按钮 → 经典固定（owner = 0、TOOLWINDOW、有 MINIMIZEBOX、不可拖动 / 缩放）；已固定时切换偏好，两种固定之间直接切换、
  任务栏不闪。
- **Bad**: 在事件处理 / `update` 闭包里直接调 `Hwnd::set_rect` / `attach_to_desktop`（GPUI 丢掉几何变化）；`SetParent` 到桌面宿主
  （DPI 与圆角退化）；`detach_from_desktop` 里先 `HWND_TOP` 后关 `FLAG_BOTTOM`（窗口仍被压到底）；设置窗口切开关后再让主窗口 `save_window_state`。

### 6. Tests Required

Unit（`cargo test`，均已存在）：

- core `window_logic.rs`：`is_desktop_host_class`（`"Progman"` true；`"WorkerW"`、`""`、`"progman"` false）；`needs_reattach`（缓存 0 → true；
  owner 已死 → true；shell 句柄变了 → true；窗口 owner 被清 → true；`shell_now == 0` → 一律 false）；
  `tool_window_style_is_repaired_only_when_wiped`（只有 TOOLWINDOW / TOOLWINDOW|其它位为 false；只有 APPWINDOW、`0`、TOOLWINDOW|APPWINDOW 为 true）。
- core `db/migrations.rs`：`v27_adds_fixed_embed_desktop_key`（键默认 `'false'`；`screen_configs` 没有 `is_desktop` 列）。
- core `data.rs`：`import_without_fixed_embed_desktop_defaults_to_false`、`fixed_embed_desktop_survives_export_import_round_trip`。
- app `services/window_mode.rs`：`target_mode_follows_embed_preference_only_on_windows`；`platform/subclass.rs`：`geometry_hits_cover_resize_and_drag`。

E2E（Windows，`pc-gpui/scripts/e2e-windows/run_e2e.py` 的 S 组；CI `.github/workflows/pc-gpui-e2e-windows.yml`，见其 README 与
[desktop-window-e2e-guide](../guides/desktop-window-e2e-guide.md)）：

| 用例 | 断言 |
|---|---|
| S01 普通模式 | APPWINDOW、无 owner、`HTCAPTION` / `HTLEFT`，拖动标题栏移动且位置落库 |
| S02 点锁按钮 → 经典固定 | TOOLWINDOW、`WM_NCHITTEST` 不再是拖动 / 缩放区、真实拖动不动、`is_fixed` 落库 |
| S03 `fixed_embed_desktop=true` 重启恢复 | owner = 桌面宿主、TOOLWINDOW、无 MINIMIZEBOX |
| S04 Z 序 | 普通应用窗口（探针）在它之上 |
| S05 Win+D | 之后仍可见 |
| S06 Explorer 重启 | 自动重挂 owner |
| S07 点锁按钮回普通模式 | 状态表"普通"一行 |

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
pub fn apply_mode(cx: &mut App, target: TargetMode) -> Task<Result<(), String>> {
    cx.spawn(async move |cx| {
        let planned = cx.update(|cx| { /* WindowManager::main(cx)?; */ Some((mode(cx).main_hwnd, plan_mode(cx, target))) });
        let Some((hwnd, ops)) = planned else { return Err("主窗口不存在".to_string()) };
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

## Common Mistakes

### Common Mistake: 只在切换那一刻打一次 Win32 样式

**Symptom**（旧 Tauri 版实测）：开发版在"主窗口固定 + 嵌入桌面、桌面上有 5 个收纳箱"时重启，主窗口最终 owner=Progman 但
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
（`classic_fixed` / `desktop`），由 `TargetMode::for_state(is_fixed, embed)` 映射；`screen_configs` 只存 `is_fixed`。

### Design Decision: 嵌入核心按窗口句柄执行，主窗口与收纳箱共用

**Context**: 桌面收纳箱（多个窗口）需要与"固定 + 嵌入桌面"的主窗口完全相同的处理（owner=Progman + TOOLWINDOW + 置底、Explorer 重启重挂），
但只有"始终嵌入"一种形态。

**Decision**: `platform::embed` 的函数都只接收 `Hwnd`、不碰任何模式状态，挂上的宿主与嵌入前的 MINIMIZEBOX 记在 embed 内部的登记表里；
主窗口的模式状态在 `services::window_mode`，收纳箱的窗口登记在 `services::fences`（[desktop-fences.md](./desktop-fences.md) §3.1）。
Explorer 重启判定共用纯函数 `window_logic::needs_reattach`。
