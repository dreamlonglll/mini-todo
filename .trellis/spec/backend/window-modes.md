# Window Modes (normal / fixed / fixed + embedded in desktop)

> Code-spec for `pc/src-tauri/src/commands/window.rs` + the frontend `appStore` half.
> Source of truth for how the main window's Win32 style, owner and Z-order are managed.
> Established in task `09-07-desktop-embed-mode` (2026-09-12/13), e2e-verified on Win11 25H2.
>
> 2026-10-15（任务 `10-15-desktop-fence`）：嵌入桌面的 Win32 核心泛化为**按窗口句柄执行**
> （`desktop_attach_hwnd` / `apply_fixed_ex_style_hwnd`），供始终嵌入桌面的收纳箱窗口复用（登记、轮询、建窗规则见
> [desktop-fences.md](./desktop-fences.md)）。主窗口仍走原来的 `desktop_attach` / `desktop_detach` / `IS_DESKTOP_MODE` /
> `DESKTOP_OWNER` 路径，§3 的状态表不变。同时新增 `lacks_tool_window_style` 自愈（§3 Ordering rule 7、下方 Common Mistakes）。

---

## Scenario: Main window mode switching

### 1. Scope / Trigger

- Trigger: new Tauri command signatures, a `settings` migration (v27), cross-layer contract
  (settings window → backend → main window), Win32 ordering constraints that are invisible from
  the type system.
- Applies whenever code touches `IS_FIXED_MODE` / `IS_DESKTOP_MODE`, `apply_fixed_ex_style`,
  `SetWindowLongPtrW`, `SetWindowPos`, `set_always_on_top/bottom`, `set_minimizable`, or the
  200 ms polling thread in `lib.rs`.
- 同样适用于主窗口与收纳箱共用的按句柄核心 `desktop_attach_hwnd` / `apply_fixed_ex_style_hwnd`，以及
  `lacks_tool_window_style` / `tick_fixed_style`。改这几个函数会同时影响主窗口和所有收纳箱窗口，两边都要回归。
- 200ms 轮询线程现在每个 tick 先跑收纳箱的 `tick_embedded_windows`（与主窗口模式无关，每 10 个 tick 再跑一次
  `tick_fence_screen_layouts`），再按主窗口模式分支：`is_desktop_mode()` → `tick_desktop_mode`；`is_fixed_mode()` →
  `restore_if_minimized` → `tick_fixed_style` → `tick_auto_hide`。

### 2. Signatures

```rust
// commands/window.rs — global state (both atomics never true at once)
pub static IS_FIXED_MODE: AtomicBool;      // classic fixed mode
pub static IS_DESKTOP_MODE: AtomicBool;    // fixed mode with "embed in desktop" on
static DESKTOP_OWNER: AtomicIsize;         // Progman HWND we attached to, 0 = not attached
pub fn is_fixed_mode() -> bool;
pub fn is_desktop_mode() -> bool;

// Tauri commands (all resolve "main" via app_handle, never the calling window)
#[tauri::command] pub fn set_window_fixed_mode(app_handle: AppHandle, db: State<Database>, fixed: bool) -> Result<(), String>;
#[tauri::command] pub fn set_window_desktop_mode(app_handle: AppHandle, db: State<Database>, enabled: bool) -> Result<(), String>;
#[tauri::command] pub fn get_fixed_embed_desktop(db: State<Database>) -> Result<bool, String>;
#[tauri::command] pub fn set_fixed_embed_desktop(app_handle: AppHandle, db: State<Database>, enabled: bool) -> Result<(), String>;

// internal seams (cfg(windows) unless noted)
fn desktop_host() -> Option<HWND>;                       // GetShellWindow(); None only if no shell window
fn is_desktop_host_class(name: &str) -> bool;            // == "Progman" (pure, tested, all platforms)
fn desktop_attach(window: &WebviewWindow);               // idempotent: owner=Progman + TOOLWINDOW + HWND_BOTTOM（= desktop_attach_hwnd + 写 DESKTOP_OWNER）
fn desktop_attach_hwnd(hwnd: HWND) -> Option<isize>;     // 按句柄的 Win32 核心，主窗口与收纳箱共用；返回挂上的宿主，Explorer 未运行时 None；不碰任何全局状态
fn apply_fixed_ex_style(window: &WebviewWindow, fixed: bool);
fn apply_fixed_ex_style_hwnd(hwnd: HWND, fixed: bool);   // 上一行的按句柄版本：只翻 TOOLWINDOW / APPWINDOW 两位
fn desktop_detach(window: &WebviewWindow);               // owner=0 + ex style per is_fixed_mode() + HWND_TOP
fn leave_desktop_mode(window: &WebviewWindow);           // all platforms: atomics + tao + queued detach
fn needs_reattach(cached_owner: isize, owner_alive: bool, shell_now: isize, current_owner: isize) -> bool; // pure, tested
fn lacks_tool_window_style(ex_style: u32) -> bool;       // 纯函数（有单测，全平台编译）：没有 WS_EX_TOOLWINDOW 或带 WS_EX_APPWINDOW
pub fn tick_desktop_mode(window: &WebviewWindow);        // called every 200 ms while is_desktop_mode(); needs_reattach || (shell_now != 0 && lacks_tool_window_style) → 排队 desktop_attach
pub fn tick_fixed_style(window: &WebviewWindow);         // 200 ms，is_fixed_mode() 时：样式被抹掉才排队 reassert_window_mode_state
fn reassert_window_mode_state(window: &WebviewWindow);   // queued on main thread; re-applies style for current mode
```

```ts
// pc/src/stores/appStore.ts
isFixed: Ref<boolean>                 // true for classic fixed AND embedded
fixedEmbedDesktop: Ref<boolean>       // the preference switch (settings key fixed_embed_desktop)
isEmbeddedInDesktop: ComputedRef      // isFixed && fixedEmbedDesktop
windowMode: ComputedRef<'normal' | 'fixed'>
applyFixedMode()   // setResizable(false) → invoke(fixedEmbedDesktop ? 'set_window_desktop_mode' {enabled:true} : 'set_window_fixed_mode' {fixed:true})
applyNormalMode()  // setResizable(true) → invoke both commands with false (both idempotent no-ops when not in that mode)
loadFixedEmbedDesktop() / setFixedEmbedDesktop(enabled)  // narrow key, never saveWindowState()
```

### 3. Contracts

**Persisted state**

| Store | Key / column | Meaning | Synced (export / WebDAV) |
|---|---|---|---|
| `settings` | `is_fixed` | main window currently fixed (classic or embedded) | yes |
| `settings` | `fixed_embed_desktop` (v27, default `'false'`) | preference: fixed mode embeds into desktop | yes (`AppSettings.fixed_embed_desktop`, `#[serde(default)]`) |
| `settings` | `auto_hide_enabled`, `top_on_wake` | classic-fixed-only behaviours; ignored while embedded | yes |
| `screen_configs` | `is_fixed` | per display-combination restore value | no (device-specific) |

There is **no** `screen_configs.is_desktop` column and no `is_desktop` key: "embedded" is not a
third mode, it is `is_fixed && fixed_embed_desktop`. Startup restore (`appStore.initSettings`)
reads `screen_configs.is_fixed` and then `applyFixedMode()` picks the command by the preference.

**Resulting Win32 state per mode** (what `e2e.ps1 -Step state` must show)

| Mode | `IS_FIXED` | `IS_DESKTOP` | owner (`GWLP_HWNDPARENT`) | ex style | `WS_MINIMIZEBOX` | Z-order | `body.fixed-mode` |
|---|---|---|---|---|---|---|---|
| normal | false | false | 0 | `WS_EX_APPWINDOW`, no TOOLWINDOW | yes | ordinary top-level | no |
| fixed (classic) | true | false | 0 | `WS_EX_TOOLWINDOW`, no APPWINDOW | yes | top-level; `TOPMOST` only while woken with `top_on_wake` | yes |
| fixed + embedded | false | true | Progman (`GetShellWindow()`) | `WS_EX_TOOLWINDOW`, no APPWINDOW, no TOPMOST | **no** | z[1] directly above Progman; every later `SetWindowPos` is forced to `HWND_BOTTOM` by tao's `ALWAYS_ON_BOTTOM` hook | no (keeps rounded corners / border) |

**Cross-window contract for the preference switch**

1. SettingsView → `appStore.setFixedEmbedDesktop(v)` → `invoke('set_fixed_embed_desktop', { enabled })`
2. Backend writes the key, then — only if the main window is currently fixed — calls
   `set_window_desktop_mode(…, true)` (on) or `set_window_fixed_mode(…, true)` (off) itself.
3. SettingsView emits `app-settings-changed { key: 'fixedEmbedDesktop' }`; MainView only
   `loadFixedEmbedDesktop()` (updates `body.fixed-mode` and the value `saveWindowState()` will write).
   MainView must **not** re-apply the mode; the backend already did and a second apply would race
   the queued `desktop_attach` / `desktop_detach`.

**Ordering rules (the non-obvious part)**

1. tao's `WindowFlags::apply_diff` (triggered by `set_minimizable`, `set_always_on_bottom`,
   `set_resizable`, `set_always_on_top` …) rewrites the whole `GWL_EXSTYLE`. Any Win32 style
   change (`apply_fixed_ex_style`, owner, `HWND_BOTTOM`) must therefore be **queued after** those
   calls with `window.run_on_main_thread(...)` (`reassert_window_mode_state`), never executed
   inline before them.
2. Entering embedded mode: `set_minimizable(false)` → `set_always_on_bottom(true)` → **then**
   `IS_DESKTOP_MODE.store(true)` → queue `desktop_attach`. Storing the flag earlier lets the
   200 ms tick enqueue an attach *before* the two `apply_diff`s, which wipes `WS_EX_TOOLWINDOW`
   and flashes a taskbar icon.
3. Leaving embedded mode: `IS_DESKTOP_MODE.store(false)` **first** (queued tick/reassert
   closures re-check the flag and skip), then `set_always_on_bottom(false)`, `set_minimizable(true)`,
   then queue `desktop_detach` (its `HWND_TOP` would be rewritten to `HWND_BOTTOM` if the hook were
   still active).
4. `desktop_detach` restores the ex style from `is_fixed_mode()` at execution time, so a direct
   embedded → classic-fixed switch keeps `WS_EX_TOOLWINDOW` without a taskbar flash
   (`set_window_fixed_mode` sets `IS_FIXED_MODE = true` before the closure runs).
5. `GWLP_HWNDPARENT` is *not* touched by tao; only `GWL_STYLE` / `GWL_EXSTYLE` are. Owner survives
   every `apply_diff`; ex style does not.
6. The tray "固定模式" `CheckMenuItem` is set explicitly (`sync_tray_fixed_checked`) — true for both
   fixed flavours; never rely on muda's auto-toggle.
7. **`run_on_main_thread` 在主线程上调用时是就地执行，不是排队**。规则 1 的"排在 tao 操作之后"只有从非主线程排队时才成立：
   同步命令跑在主线程上，里面的 `run_on_main_thread(...)` 会立刻执行，可能跑到前端更早发出、尚在事件循环里排队的 tao 操作
   （`setResizable` 等）之前。兜底：200ms 轮询线程在桌面模式（`tick_desktop_mode`）、经典固定模式（`tick_fixed_style`）、
   收纳箱（`tick_embedded_windows`）三处都读 `GWL_EXSTYLE`，`lacks_tool_window_style` 命中就从轮询线程排队重申——
   从非主线程排队一定排在已入队的 tao 操作之后。正常情况下每个 tick 只多一次 `GetWindowLongW`。

### 4. Validation & Error Matrix

| Condition | Behaviour |
|---|---|
| `set_window_desktop_mode(enabled=true)` on non-Windows | 回退为经典固定模式：内部调 `set_window_fixed_mode(…, true)` 并返回其结果（D5：同步来的 `fixed_embed_desktop=true` 不能让固定失败；旧文档写的 `Err("桌面模式仅支持 Windows")` 已不成立） |
| `set_window_desktop_mode(enabled=false)` on non-Windows | `Ok(())` no-op (frontend `applyNormalMode` calls it unconditionally) |
| main window not found | `Err("主窗口不存在")` |
| `GetShellWindow()` returns null (Explorer not running) | `desktop_host()` → `None`, attach skipped with `eprintln!`; `needs_reattach` returns `false` while `shell_now == 0`, tick retries every 200 ms |
| shell window class ≠ `Progman` (Win10 / Win11 ≤ 23H2 / third-party shell) | logged, **still attached** — not minimized, not in taskbar, bottom of Z-order hold; Win+D may cover the window (out of scope, PRD Decision 6) |
| Explorer restarted (new Progman HWND) | `needs_reattach(cached, alive=false, …)` → queued `desktop_attach` re-owns to the new HWND within one tick (measured < 1.5 s); main window is **not** destroyed with its owner |
| preference toggled while not fixed | key written only; takes effect on next `applyFixedMode()` |
| `set_fixed_embed_desktop` from the settings window | must not call `saveWindowState()` (would persist the settings window's geometry as the main window's) |
| unknown / missing `fixedEmbedDesktop` in old backups or remote sync blobs | `#[serde(default)]` → false; import never fails |
| 启动竞态：主窗口固定（经典或嵌入）时 ex style 被晚到的 `apply_diff` 改成 APPWINDOW（owner 仍是 Progman / 0） | 下一个 tick `lacks_tool_window_style` 命中 → 排队重申（嵌入态 `desktop_attach`，经典固定 `reassert_window_mode_state`），约 200ms 内恢复；期间任务栏可能闪一下 |
| 普通模式下 ex style 是 APPWINDOW | 正常，不检查（`tick_fixed_style` 只在 `is_fixed_mode()` 时调用） |

### 5. Good/Base/Bad Cases

- **Good**: preference on, user clicks the lock → `owner=Progman, zIndex=1, minimizeBox=false,
  toolWindow=true`; Win+D: `everIconic=false, everHidden=false, centerHitIsMain=true`; a plain
  top-level window shown over it covers it; killing and relaunching the exe restores the same state
  and rect.
- **Base**: preference off, lock → classic fixed (`owner=0, toolWindow=true, minimizeBox=true`);
  toggling the preference while fixed switches live in both directions without a taskbar flash.
- **Bad**: calling `apply_fixed_ex_style` inline before `set_minimizable` — style lost, taskbar icon
  appears; calling `SetParent(hwnd, host)` — window becomes a child: DPI and rounded corners degrade
  (rejected in the spike); storing `IS_DESKTOP_MODE = true` before the tao calls — intermittent
  taskbar flash.

### 6. Tests Required

Unit (`cargo test`, all present):

- `is_desktop_host_class`: `"Progman"` true; `"WorkerW"`, `""`, `"progman"` false.
- `needs_reattach`: cached 0 → true; owner dead → true; shell handle changed → true; window's owner
  cleared → true; `shell_now == 0` → false regardless.
- `tool_window_style_is_repaired_only_when_wiped`：只有 `WS_EX_TOOLWINDOW` 为 false；`TOOLWINDOW | 其它位` 为 false；
  只有 `WS_EX_APPWINDOW`、`0`、`TOOLWINDOW | APPWINDOW` 都为 true。
- `migrations::v27_adds_fixed_embed_desktop_key`: key seeded `'false'`; `screen_configs` has no
  `is_desktop` column.
- `data::import_without_fixed_embed_desktop_defaults_to_false` and
  `fixed_embed_desktop_survives_export_import_round_trip`.

E2E (manual harness, `pc/scripts/e2e/`, see the README there; run against the dev build started with
`WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9222`):

| Step | Command | Assertion |
|---|---|---|
| enter mode | `node cdp.mjs "…click button[title='固定窗口']…"` | `e2e.ps1 -Step json` matches the mode row in §3 |
| show desktop | `e2e.ps1 -Step win-d` (×2) | `everIconic=False everHidden=False zIndexAfter=1 centerHitIsMain=True`, rect unchanged |
| minimise all | `e2e.ps1 -Step win-m` | same |
| covered by normal windows | WinForms `Form.Show()` over the rect | `WindowFromPoint` hits the form, main still z[1] |
| Explorer restart | `e2e.ps1 -Step restart-explorer` | `main window alive: True`, `reattached within … True`, then `win-d` again |
| app restart | `e2e.ps1 -Step kill-app` → start `target\debug\mini-todo.exe` (vite must be running separately) → `wait-window` → `json` | mode + rect restored |
| 主窗口嵌入 + 多个收纳箱时重启 | 同上（开发版须带 `MINI_TODO_DATA_DIR` 等隔离变量，按 PID 结束） | 最终 `ownerIsProgman=True、toolWindow=True、appWindow=False`；竞态命中时约 200ms 内自愈 |
| real input | `mouse.ps1 -Action click/drag/wheel` at `clientOrigin + css·dpr` | DOM state via CDP changes; editor window gets foreground on first click |

### 7. Wrong vs Correct

#### Wrong — Win32 style applied inline, then tao overwrites it

```rust
apply_fixed_ex_style(&window, true);          // WS_EX_TOOLWINDOW set …
let _ = window.set_minimizable(false);         // … and wiped here by apply_diff
IS_DESKTOP_MODE.store(true, Ordering::SeqCst);
```

#### Correct — tao first, flag second, Win32 queued behind them

```rust
let _ = window.set_minimizable(false);
let _ = window.set_always_on_bottom(true);
IS_DESKTOP_MODE.store(true, Ordering::SeqCst);
reassert_window_mode_state(&window);           // run_on_main_thread → desktop_attach
```

#### Wrong — main window re-applies the mode when the settings window flips the switch

```ts
case 'fixedEmbedDesktop':
  await appStore.loadFixedEmbedDesktop()
  if (appStore.isFixed) await appStore.applyFixedMode()   // races the backend's queued attach/detach
```

#### Correct — backend command owns the live switch, main window only reloads the value

```rust
// set_fixed_embed_desktop
if enabled && is_fixed_mode() { set_window_desktop_mode(app_handle, db, true) }
else if !enabled && is_desktop_mode() { set_window_fixed_mode(app_handle, db, true) }
else { Ok(()) }
```

```ts
case 'fixedEmbedDesktop':
  await appStore.loadFixedEmbedDesktop()
  break
```

#### Wrong — 以为同步命令里的 `run_on_main_thread` 一定排在前端的 tao 调用之后，样式只在切换时打一次

```rust
// set_window_desktop_mode（同步命令，主线程）
let _ = window.set_minimizable(false);
let _ = window.set_always_on_bottom(true);
IS_DESKTOP_MODE.store(true, Ordering::SeqCst);
reassert_window_mode_state(&window); // 在主线程上 run_on_main_thread 就地执行；前端早先排队的 SetResizable 稍后才执行，抹掉 TOOLWINDOW
```

#### Correct — 切换路径不变，轮询线程按实际样式自愈

```rust
// tick_desktop_mode（200ms 轮询线程，非主线程排队 → 一定排在已入队的 tao 操作之后）
if needs_reattach(cached_owner, owner_alive, shell_now, current_owner)
    || (shell_now != 0 && lacks_tool_window_style(ex_style))
{
    let w = window.clone();
    let _ = window.run_on_main_thread(move || if is_desktop_mode() { desktop_attach(&w) });
}
// tick_fixed_style：if is_fixed_mode() && lacks_tool_window_style(ex_style) { reassert_window_mode_state(window) }
```

---

## Common Mistakes

### Common Mistake: 以为 `run_on_main_thread` 总是排队执行

**Symptom**：开发版在"主窗口固定 + 嵌入桌面、桌面上有 5 个收纳箱"时重启，主窗口最终 owner=Progman 但 `toolWindow=False /
appWindow=True`——出现在任务栏、参与 Alt+Tab，之后再没人补回来。逐帧跟踪 ex style：475ms `set_minimizable(false)` →
482ms `desktop_attach` 打上 TOOLWINDOW → 519ms 前端早先发出的 `setResizable(false)` 才执行，`apply_diff` 把 TOOLWINDOW 抹掉。

**Cause**：前端 `applyFixedMode` 先 `await setResizable(false)`（异步窗口插件命令，只是把 `SetResizable` 排进事件循环），再调同步命令
`set_window_desktop_mode`。启动时主线程正处在 WebView2 的嵌套消息泵里（同时在创建收纳箱窗口），同步命令先于排队的
`SetResizable` 被处理；而同步命令里的 `run_on_main_thread` 在主线程上是**就地执行**、不排队，于是"Win32 样式排在 tao 操作之后"
（Ordering rule 1）的前提不成立。这是上游就有的潜在竞态，启动时并发恢复收纳箱让它稳定复现；经典固定模式同样可能命中。

**Fix**：200ms 轮询线程按**实际**样式自愈（Ordering rule 7）：`tick_desktop_mode` 与新增的 `tick_fixed_style` 发现
`lacks_tool_window_style` 就从轮询线程排队重申；收纳箱的 `tick_embedded_windows` 用同一判定。修复后同一场景 717ms 时样式恢复。

**Prevention**：任何"只在切换那一刻打一次 Win32 样式"的设计都要配一个按实际状态检查的轮询兜底；不要假设主线程上的
`run_on_main_thread` 会排在任何东西之后。

> **Warning**：残留约 200ms 的窗口期——竞态命中时主窗口最多带着 `WS_EX_APPWINDOW` 一个 tick，任务栏可能闪一下按钮。
> 正常情况下（未命中竞态）不会出现。

---

## Design Decisions

### Design Decision: 嵌入按窗口句柄泛化，主窗口路径与状态不变

**Context**：桌面收纳箱（多个窗口）需要和"固定 + 嵌入桌面"的主窗口完全相同的 Win32 处理（owner=Progman + TOOLWINDOW +
`HWND_BOTTOM`、Explorer 重启重挂），但收纳箱只有"始终嵌入"一种形态，没有模式切换。

**Decision**：只把 Win32 核心抽成按句柄、不碰全局状态的 `desktop_attach_hwnd` / `apply_fixed_ex_style_hwnd`，返回挂上的宿主，
由调用方各自记录 owner：主窗口写 `DESKTOP_OWNER`，收纳箱写 `EMBEDDED_WINDOWS`（label → `{hwnd, owner}`）。主窗口的两个原子量、
`desktop_attach` / `desktop_detach` / `leave_desktop_mode`、Ordering rules 1–6 与 §3 状态表都不变；Explorer 重启判定共用纯函数
`needs_reattach`。收纳箱的 tao 标志在建窗时一次给齐，之后只有 `show()` / `hide()` 需要紧跟重挂，见
[desktop-fences.md §3.1](./desktop-fences.md)。

### Design Decision: owner=Progman + tao `always_on_bottom`, not `SetParent`

**Context**: "Embed the window in the desktop" with a hard requirement of unchanged transparency,
rounded corners and per-monitor DPI, on Win11 24H2+ where Show-Desktop raises Progman itself.

**Options Considered**:
1. `SetParent(hwnd, SHELLDLL_DefView host)` child window (Lively/Wallpaper-Engine style)
2. Keep a top-level window, set `GWLP_HWNDPARENT = Progman`, and let tao's `ALWAYS_ON_BOTTOM`
   `WM_WINDOWPOSCHANGING` hook force `HWND_BOTTOM` on every `SetWindowPos`
3. `SetWindowSubclass` with a custom `WM_WINDOWPOSCHANGING`

**Decision**: Option 2. Owned windows are guaranteed to stay above their owner, so `HWND_BOTTOM`
resolves to "directly above Progman" — below every app window, above the icons — and the window
is raised together with Progman on Win+D. Option 1 degraded DPI and lost rounded corners in the
spike; option 3 duplicates what tao already ships (`event_loop.rs` `ALWAYS_ON_BOTTOM` handling).
No new `windows` crate features were needed.

**Extensibility**: Win10 / Win11 ≤ 23H2 raise a `WorkerW` instead of Progman on Show-Desktop; a
Rainmeter-style sentinel state machine would slot into `tick_desktop_mode` without touching the
attach/detach seams.

### Design Decision: "embed" is a preference on fixed mode, not a third mode

**Context**: The first implementation exposed a separate "桌面模式" button and tray item next to
"固定模式"; the user found two mutually-exclusive lock buttons confusing.

**Decision**: One user-facing mode (fixed) plus a Windows-only settings switch
`fixed_embed_desktop`. Backend keeps two atomics because the Win32 handling differs completely;
the frontend maps `isFixed × fixedEmbedDesktop` onto the two commands in `applyFixedMode()`.
`WindowMode` stays `'normal' | 'fixed'`; `screen_configs` keeps only `is_fixed`.
