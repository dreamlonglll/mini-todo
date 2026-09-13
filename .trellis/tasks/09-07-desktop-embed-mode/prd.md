# 桌面模式：主窗口嵌入桌面图标层之上，Win+D 不最小化

## Goal

新增第三种窗口模式「桌面模式」（普通 / 固定 / 桌面），把主窗口"嵌入"Windows 桌面：
位于桌面图标之上、所有应用窗口之下，按 Win+D / Win+M / 任务栏"显示桌面"时不被最小化、
不被盖住；保持现有的透明背景（深色主题下 rgba 半透明底透出壁纸）。

背景：现有「固定模式」只是给顶层窗口加 `WS_EX_TOOLWINDOW`，Win+D 仍会把它最小化，
`lib.rs` 里的 200ms 轮询线程再把它还原（`restore_if_minimized`），用户看到的是"消失再弹回"，
不是嵌入效果。

## What I already know

### 用户决定（2026-09-07）

* 嵌入位置：**桌面图标之上**（不是壁纸层 WorkerW 之下）
* 作为**独立的第三种模式**，保留现有固定模式及其贴边隐藏 / 唤起 / 唤起置顶功能不变
* **不接受实色底**：透明背景是硬性要求，做不出透明就不算完成

### 现有实现（repo 事实）

* 固定模式状态：`IS_FIXED_MODE` 原子量 + settings `is_fixed` + `screen_configs.is_fixed`
  （`pc/src-tauri/src/commands/window.rs`、`db/models.rs`、`commands/data.rs`）
* 固定模式入口：TitleBar 锁按钮（`components/TitleBar.vue`）、托盘 CheckMenuItem `toggle_fixed`
  （`lib.rs`，emit `tray-toggle-fixed` → `appStore.toggleFixedMode`）
* 前端 `WindowMode = 'normal' | 'fixed'`（`types/app.ts`），`appStore.windowMode` / `isFixed`
* 固定模式绕开 tao 的 flag 通路直接用 Win32 改样式 / Z 序，因为 tao `WindowFlags::apply_diff`
  会整体覆写 `GWL_EXSTYLE`（见 `.trellis/tasks/archive/2026-08/08-18-fix-taskbar-icon-flash/prd.md`）；
  `reassert_fixed_window_state` 通过 `run_on_main_thread` 排队兜底
* 200ms 轮询线程（`lib.rs` setup）：固定模式下 `restore_if_minimized` + `tick_auto_hide`
* 主窗口：`tauri.conf.json` `decorations:false, transparent:true, shadow:false`；`lib.rs`
  给它打 `DWMWA_WINDOW_CORNER_PREFERENCE = DWMWCP_ROUND`
* 主窗口打开编辑 / 设置 / 已完成窗口时传 `parent: appWindow`（`views/MainView.vue`
  414 / 484 / 548 行），Windows 上即 owner 关系
* `is_fixed` 走 settings 导出 / 导入 / WebDAV 同步（`data.rs` `read_app_settings` /
  `write_app_settings`；`sync_cmd.rs` 把 `settings` 作为整体 `serde_json::Value` 传输）
* `windows` crate 0.58，已启用 `Win32_Foundation` / `Win32_UI_WindowsAndMessaging` /
  `Win32_Graphics_Dwm`；`SetParent` / `FindWindowExW` / `SendMessageTimeoutW` / `EnumWindows`
  / `GetParent` / `IsWindow` 都在 `Win32_UI_WindowsAndMessaging` 内，无需新依赖
* 版本：tauri 2.9.5 / tao 0.34.5 / wry 0.53.5 / webview2-com 0.38.2

### 主窗口对键盘的依赖

主窗口本身基本是鼠标操作（勾选、拖拽排序、点开详情）；标题编辑 / 描述编辑 / 设置全部在
独立顶层 WebView 窗口中完成，不受嵌入影响。

## Assumptions (temporary)

* 嵌入后窗口不可拖拽、不可缩放（与固定模式一致："固定在用户指定位置"）；要挪位置先退回普通模式
* 桌面模式下不需要贴边隐藏 / 唤起 / 唤起置顶（窗口永远在所有应用窗口之下，这三者逻辑上不成立）
* 桌面模式下窗口天然不在任务栏、不参与 Alt+Tab（子窗口 / owner 关系都满足），不需要额外样式
* 桌面模式与固定模式互斥：进入其一即退出另一个

## Open Questions

* ~~透明背景在"桌面子窗口"形态下是否可行~~ → 原型已回答：方案 A 透明可行但 DPI / 圆角退化；方案 B 零退化，采用 B
* Explorer 重启后 owner=Progman 的主窗口是否被连带销毁 → e2e 阶段实测（见 Decision 第 7 条）

## Requirements (evolving)

1. 新增窗口模式 `desktop`，前端 `WindowMode = 'normal' | 'fixed' | 'desktop'`
2. 入口：
   * TitleBar 新增「桌面模式」图标按钮（Element Plus Icons，不用 emoji），与锁按钮并列；
     桌面模式下该按钮高亮，点击退出桌面模式回到普通模式
   * 托盘菜单新增 CheckMenuItem「桌面模式」，与「固定模式」互斥，勾选状态双向同步
3. 进入桌面模式：
   * 窗口停留在当前位置，位于桌面图标之上、所有应用窗口之下
   * Win+D / Win+M / 任务栏"显示桌面"后窗口仍可见、未最小化、未被桌面盖住；再次 Win+D 恢复其它窗口后仍在原位
   * 保持透明背景效果（深色主题 rgba 底 + 圆角），与普通模式观感一致
   * 鼠标操作（勾选完成、点开详情、拖拽排序、右键菜单、滚动）正常
   * 不可拖拽移动、不可缩放（同固定模式）
   * 从主窗口打开的编辑 / 设置 / 已完成窗口是普通顶层窗口，能正常显示在最前并获得键盘焦点
4. 退出桌面模式：窗口恢复为普通顶层窗口，位置不变，任务栏 / Alt+Tab 恢复正常
5. 持久化：settings 新键 `is_desktop`（v27 migration）+ `screen_configs.is_desktop` 列；
   启动时按当前屏幕配置恢复桌面模式（先定位、后嵌入）；纳入导出 / 导入 / WebDAV 同步
   （`AppSettings` 加 `#[serde(default)]` 字段，旧备份兼容）
6. 健壮性：Explorer 重启 / 崩溃后桌面宿主窗口失效，需在轮询中检测并自动重新挂载；
   轮询线程在桌面模式下跳过 `restore_if_minimized` / `tick_auto_hide`
7. 目标平台 Windows 11 24H2+（Show desktop 抬起 Progman 本身的层级）；Win10 / Win11 ≤23H2 上代码照常运行
   但不保证 Win+D 期间可见（见 Decision 第 6 条，留到下一轮）

## Acceptance Criteria (evolving)

* [ ] 桌面模式下按 Win+D，窗口全程可见、`IsIconic == false`，且未被桌面盖住；再按 Win+D 恢复后仍在原位
* [ ] 桌面模式下按 Win+M、点击任务栏最右侧"显示桌面"，表现同上
* [ ] 桌面模式下窗口在桌面图标之上（图标被窗口区域遮挡）、在任意普通窗口之下（把资源管理器窗口拖到其上方能盖住它）
* [ ] 桌面模式下透明背景与普通模式一致：壁纸透出、圆角保留，无黑底 / 无白底 / 无残影
* [ ] 桌面模式下鼠标勾选完成、点开待办详情、拖拽排序、滚动均正常
* [ ] 桌面模式下从主窗口打开编辑窗口，编辑窗口在最前且可以输入文字
* [ ] 桌面模式下任务栏无图标、Alt+Tab 不出现
* [ ] 退出桌面模式后窗口成为普通顶层窗口，位置不变，任务栏 / Alt+Tab 恢复
* [ ] 桌面模式与固定模式互斥切换正确，TitleBar 与托盘勾选状态一致
* [ ] 重启应用后自动恢复桌面模式且位置正确（多屏、主屏不在最左侧的负坐标场景）
* [ ] 任务管理器重启 explorer.exe 后 5 秒内窗口自动重新挂载到桌面
* [ ] 导出 → 导入、WebDAV 上传 → 下载应用后 `is_desktop` 保留；导入旧版备份（无该字段）不报错
* [ ] `cargo check` / `cargo test` / `npm run build`（vue-tsc）通过

## Definition of Done (team quality bar)

* Rust 侧 Win32 逻辑有单元测试覆盖纯函数部分（坐标换算、宿主查找结果判定）
* `cargo check` / `cargo test` / `npm run build` 通过
* CLAUDE.md 更新：窗口模式说明、迁移版本 v27、同步覆盖范围
* 版本号（package.json / tauri.conf.json / Cargo.toml）按发布流程递增

## Out of Scope (explicit)

* 桌面模式下拖拽移动 / 缩放窗口（先退普通模式再调整）
* 桌面模式下的贴边隐藏 / 唤起 / 唤起置顶
* 嵌入到壁纸层（图标之下）
* 实色底回退方案
* macOS / Linux 上的等价实现（命令在非 Windows 平台返回错误或 no-op）
* 重写现有固定模式
* Win10 / Win11 ≤23H2 的 WorkerW 哨兵状态机（本轮只做 24H2+，用户 2026-09-12 决定）

## Technical Approach

### 方案 A：SetParent 挂到桌面图标宿主窗口（用户描述的"嵌入"）

* 找到承载 `SHELLDLL_DefView` 的宿主（Win10 为 `Progman` 或经 `0x052C` 分裂出的 `WorkerW`；
  Win11 24H2 起层级有变化，需两套查找逻辑）
* `SetParent(hwnd, host)` 前把 `WS_POPUP` 换成 `WS_CHILD`；挂载后 `SetWindowPos` 把它排到
  `SHELLDLL_DefView` 之上（同级 Z 序）
* 坐标：子窗口 `SetWindowPos` 用父窗口客户区坐标，需按宿主的屏幕原点换算；`GetWindowRect`
  仍返回屏幕坐标，所以持久化的位置不用改
* 风险：DWM 透明是否对子窗口生效（**硬性要求**）；跨进程父子关系共享输入队列；tao 的
  `apply_diff` 会把 `WS_CHILD` 抹回 `WS_POPUP`，需要与现有 `reassert_fixed_window_state` 同类的兜底

### 方案 B：保持顶层 + owner = Progman + Rainmeter 式"显示桌面"检测

* 窗口仍是顶层窗口，透明路径不变
* `SetWindowLongPtr(GWLP_HWNDPARENT, progman)` 让它永远压在桌面正上方；去掉 `WS_MINIMIZEBOX`
* 轮询检测"显示桌面"把桌面宿主抬到顶层的情况，把自己重新插到宿主正上方
* 风险：Win+D 对无最小化框 / 有 owner 的工具窗口到底是"最小化"还是"只被盖住"，需实测

### 决策路径

1. 先做原型（spike）：在真实主窗口上分别验证 A 与 B 的 Win+D 免疫性、透明、鼠标输入
2. A 透明可行 → 采用 A；A 透明不可行而 B 免疫 Win+D → 采用 B；两者都不行 → 回到用户处报告
3. 原型结论写回本 PRD 的 Decision 节，再进入正式实现

## Research References

* `research/win32-desktop-embed.md` — 桌面窗口层级、Lively / Rainmeter 做法、Show desktop 行为（进行中）
* `research/tao-wry-transparency.md` — tao / wry 透明实现与子窗口透明可行性（进行中）

## Decision (ADR-lite)

**2026-09-12 定稿：采用方案 B（顶层窗口 + owner=Progman + tao `always_on_bottom`），本轮只保证 Win11 24H2+。**
原型结论见 `research/spike-results.md`。

### 决策要点

1. **不做 SetParent / 不做子窗口**：方案 A 在用户 125%+100% 双屏下 DPI 退化肉眼可见、圆角丢失，否决。
2. **Z 序压底不用 `SetWindowSubclass`，直接用 tao 自带的 `ALWAYS_ON_BOTTOM`**：
   `window.set_always_on_bottom(true)` 后，tao 在 `WM_WINDOWPOSCHANGING` 里把每次 `SetWindowPos` 的
   `hwndInsertAfter` 强制改成 `HWND_BOTTOM`（tao-0.34.5 `event_loop.rs:1229-1235`，见
   `research/tao-wry-transparency.md` §1.5）。有 owner=Progman 时 `HWND_BOTTOM` 的实际效果就是
   "紧贴 Progman 之上"（系统保证 owned 窗口永远在 owner 之上，原型实测 z[1]）。
   **不需要新增 Cargo feature**：`GetShellWindow` / `SetWindowLongPtrW` / `GetWindowLongPtrW` /
   `GWLP_HWNDPARENT` / `GetClassNameW` / `IsWindow` / `HWND_BOTTOM` / `HWND_TOP` 都在已启用的
   `Win32_UI_WindowsAndMessaging` 里。
3. **不被最小化用 tao `set_minimizable(false)`**（去 `WS_MINIMIZEBOX`），而不是手改 `GWL_STYLE`：
   tao 自己记录的 flag 在后续任何 `apply_diff` 里都会被保留，不需要兜底。
4. **owner=Progman 用 Win32 `SetWindowLongPtrW(hwnd, GWLP_HWNDPARENT, progman)`**：tao 不感知也不会覆写
   `GWLP_HWNDPARENT`（`set_window_flags` 只重写 `GWL_STYLE` / `GWL_EXSTYLE`）。
5. **任务栏 / Alt+Tab 复用固定模式的 `apply_fixed_ex_style(window, true)`**（加 `WS_EX_TOOLWINDOW`、去
   `WS_EX_APPWINDOW`）。tao 的 `apply_diff` 会整体覆写 ex style，所以同样需要 `reassert_*` 兜底
   （见下方"tao 覆写兜底"）。
6. **Win10 / Win11 ≤23H2 明确不在本轮范围**（用户 2026-09-12 决定）：那些系统上 Show desktop 抬起的是
   WorkerW 而非 Progman，owner 帮不上忙，需要 Rainmeter 式哨兵状态机，且当前机器（25H2）无法验证。
   本轮代码在旧系统上照常运行（不最小化、不进任务栏），只是 Win+D 期间可能被桌面盖住；
   `desktop_host()` 找不到合格宿主时记录日志并仍然挂 owner。
7. **Explorer 重启**：owner 被销毁时跨进程 owned 窗口的命运（被销毁 vs 仅被孤立）文档不明确，
   原型未测。设计上保留 seam：`desktop_attach` / `desktop_detach` / `tick_desktop_mode` 三个函数封装全部
   Win32 细节，e2e 阶段用 `taskkill /f /im explorer.exe` 实测（用户已同意）；若主窗口确实被销毁，
   回退方案是去掉 owner、只留 `always_on_bottom` + tick 检测 Progman 抬升后用 `SWP_NOSENDCHANGING`
   重排（绕过 tao 钩子）。

### 状态与持久化

| 层 | 内容 |
|---|---|
| Rust 原子量 | `pub static IS_DESKTOP_MODE: AtomicBool`（与 `IS_FIXED_MODE` 并列、互斥，`is_desktop_mode()`）；`static DESKTOP_OWNER: AtomicIsize` 缓存挂上的 Progman HWND，供 tick 检测宿主失效 |
| settings 键 | `is_desktop`（`'true'`/`'false'`），v27 migration `INSERT OR IGNORE ... 'false'` |
| screen_configs 列 | `is_desktop INTEGER NOT NULL DEFAULT 0`，同一 v27 migration `ALTER TABLE ADD COLUMN` |
| Rust 模型 | `AppSettings.is_desktop` / `ScreenConfig.is_desktop` / `SaveScreenConfigRequest.is_desktop`，全部 `#[serde(default)]`（旧备份、旧前端兼容） |
| 读写点 | `window.rs` `get_settings` / `save_settings` / `save_screen_config` / `get_screen_config` / `list_screen_configs`（所有 `SELECT ... is_fixed` 的地方同步加列）；`data.rs` `read_app_settings` / `write_app_settings`；`sync_cmd.rs` 把 `settings` 当整体 `Value` 传输，`AppSettings` 加字段即自动纳入 |
| 前端 | `WindowMode = 'normal' \| 'fixed' \| 'desktop'`；`appStore.isDesktop` ref + `windowMode`；`ScreenConfig.isDesktop` / `SaveScreenConfigRequest.isDesktop` / `AppSettings.isDesktop` |

### 后端命令与流程（`pc/src-tauri/src/commands/window.rs`）

```
#[tauri::command] set_window_desktop_mode(app_handle, db, enabled: bool)
  固定取 "main" 窗口（同 set_window_fixed_mode）
  enabled = true:
    1. 若 IS_FIXED_MODE：走退出固定模式的清理（撤销置顶、AutoHideState 复位、hidden 时挪回 anchor），
       IS_FIXED_MODE = false，sync_tray_fixed_checked(false)
    2. IS_DESKTOP_MODE = true
    3. tao 通路（会触发 apply_diff，必须排在 Win32 改样式之前）：
         window.set_minimizable(false)；window.set_always_on_bottom(true)
    4. run_on_main_thread(desktop_attach)   // 排在 3 之后，与 reassert_fixed_window_state 同一套队列语义
    5. sync_tray_desktop_checked(true)
  enabled = false:
    1. IS_DESKTOP_MODE = false
    2. window.set_always_on_bottom(false)；window.set_minimizable(true)
    3. run_on_main_thread(desktop_detach)
    4. sync_tray_desktop_checked(false)

set_window_fixed_mode(fixed = true) 里若 IS_DESKTOP_MODE：先按上面 enabled=false 的步骤退出桌面模式再进固定模式

#[cfg(windows)] fn desktop_host() -> Option<HWND>
  GetShellWindow()，校验 GetClassNameW == "Progman"，否则 None（记录 eprintln）

#[cfg(windows)] fn desktop_attach(window)        // 幂等，可反复调用
  hwnd = window_hwnd(window)?; host = desktop_host()?
  SetWindowLongPtrW(hwnd, GWLP_HWNDPARENT, host)
  apply_fixed_ex_style(window, true)
  SetWindowPos(hwnd, HWND_BOTTOM, 0,0,0,0, SWP_NOMOVE|SWP_NOSIZE|SWP_NOACTIVATE|SWP_FRAMECHANGED)
  DESKTOP_OWNER = host

#[cfg(windows)] fn desktop_detach(window)
  SetWindowLongPtrW(hwnd, GWLP_HWNDPARENT, 0)
  apply_fixed_ex_style(window, false)
  SetWindowPos(hwnd, HWND_TOP, ... SWP_NOMOVE|SWP_NOSIZE|SWP_NOACTIVATE|SWP_FRAMECHANGED)
  DESKTOP_OWNER = 0

pub fn tick_desktop_mode(window)                  // 200ms 轮询线程调用，仅 IS_DESKTOP_MODE 时
  owner = DESKTOP_OWNER
  若 owner == 0 || !IsWindow(owner) || GetShellWindow() != owner
     || GetWindowLongPtrW(hwnd, GWLP_HWNDPARENT) != owner
    → run_on_main_thread(desktop_attach)         // Explorer 重启 / owner 被清 → 重挂
  （不做 restore_if_minimized / tick_auto_hide）

reassert_fixed_window_state → 改名 reassert_window_mode_state（或保留名字但扩展语义）：
  fixed   → 现有逻辑
  desktop → desktop_attach（幂等地补 ex style + owner + HWND_BOTTOM）
  normal  → apply_fixed_ex_style(false)

bring_main_window_to_front（托盘单击/双击）在桌面模式下：不 set_always_on_top，只 show + set_focus + reassert
reset_window_impl（托盘"重置位置"）：前端收到 tray-reset-window 时若 isDesktop 也要退出桌面模式（同 isFixed 处理）
```

`SetWindowLongPtrW` 在 `windows` crate 里只对 64 位目标导出；项目只发 x64，直接用即可
（若需兼容 x86 再加 `cfg(target_pointer_width)` 分支）。

非 Windows 平台：`set_window_desktop_mode` 返回 `Err("桌面模式仅支持 Windows")`，`tick_desktop_mode` 为空函数。

### 轮询线程（`lib.rs` setup）

```
loop 200ms:
  if is_desktop_mode()  { tick_desktop_mode(&window) }
  else if is_fixed_mode() { restore_if_minimized; tick_auto_hide }
```

### 托盘（`lib.rs`）

* 新增 `CheckMenuItem::with_id(app, "toggle_desktop", "桌面模式", true, is_desktop_mode(), None)`，
  放在 `toggle_fixed` 之后；`commands::set_tray_toggle_desktop_item(item.clone())`
* 菜单事件 `"toggle_desktop"` → `window.emit("tray-toggle-desktop", ())`，由前端 `appStore.toggleDesktopMode()` 处理
  （与 `tray-toggle-fixed` 同一模式：托盘只发事件，状态由前端 store 单点维护后再回写后端）
* 勾选同步：`sync_tray_desktop_checked` / `sync_tray_fixed_checked` 在两个命令里互相清理

### 前端

* `appStore.ts`
  * `isDesktop = ref(false)`；`windowMode` 随 `isFixed` / `isDesktop` 推导（`'desktop' > 'fixed' > 'normal'`）
  * `toggleDesktopMode()`：翻转 `isDesktop`；进入时 `isFixed = false`；调 `applyDesktopMode()` /
    `applyNormalMode()`；最后 `saveWindowState()`
  * `applyDesktopMode()`：`appWindow.setResizable(false)` → `invoke('set_window_desktop_mode', { enabled: true })`
  * `applyNormalMode()`：`setResizable(true)` → 同时 `set_window_fixed_mode(false)` 与
    `set_window_desktop_mode(false)`（后端命令幂等，重复调用无副作用）
  * `toggleFixedMode()`：进入固定模式时 `isDesktop = false`（后端 `set_window_fixed_mode(true)` 自行退出桌面模式）
  * `initSettings()`：`savedConfig.isDesktop` → 先 setPosition / setSize，再 `applyDesktopMode()`
  * `saveWindowState()`：`configRequest.isDesktop` 与 `save_settings` 的 `isDesktop`
* `TitleBar.vue`：锁按钮旁新增桌面模式按钮，图标 `Monitor`（@element-plus/icons-vue），
  `:class="{ active: isDesktop }"`，title `'桌面模式' / '退出桌面模式'`；
  拖拽禁用条件从 `isFixed` 改为 `isFixed || isDesktop`（`no-drag` class、`data-tauri-drag-region`、`onTitleBarMouseDown`）
* `MainView.vue`：`listen('tray-toggle-desktop')` → `appStore.toggleDesktopMode()`；`tray-reset-window` 里
  `isDesktop` 也退出；**不要**给桌面模式加 `body.fixed-mode`（那个类会去掉圆角与描边，桌面模式要保留普通模式观感）；
  子窗口继续传 `parent: appWindow`（方案 B 主窗口仍是顶层窗口，owner 链 编辑器→主窗口→Progman 正常）
* `SettingsView.vue` 屏幕配置列表的模式文案：`isDesktop ? '桌面模式' : isFixed ? '固定模式' : '普通模式'`
* `types/app.ts`：`WindowMode` 加 `'desktop'`；`AppSettings` / `ScreenConfig` / `SaveScreenConfigRequest` 加 `isDesktop: boolean`

### 单元测试（Rust）

* `desktop_host` 的类名校验逻辑抽成纯函数 `is_desktop_host_class(name: &str) -> bool`，测 `"Progman"` / `"WorkerW"` / `""`
* `tick_desktop_mode` 的"是否需要重挂"判定抽成纯函数
  `needs_reattach(cached_owner: isize, owner_alive: bool, shell_now: isize, current_owner: isize) -> bool`，覆盖四种失效情形
* 现有 `evaluate_auto_hide_transition` 测试不受影响

### 2026-09-13 UX 调整（用户决定，e2e 全部通过之后）

**桌面模式不再作为第三种独立模式暴露给用户**，改为设置 → 常规里的开关
「固定模式时，嵌入桌面中」（仅 Windows 可见，说明文字注明需要 Windows 11 24H2 及以上）：

* 开启后，固定模式 = 原桌面模式（owner=Progman + always_on_bottom + 无最小化框）；关闭则是原固定模式
* 去掉 TitleBar 的 `Monitor` 按钮与托盘「桌面模式」项 / `tray-toggle-desktop` 事件；入口只剩锁按钮 + 托盘「固定模式」
* 当前已处于固定模式时切换开关立即生效：后端 `set_fixed_embed_desktop` 落库后按状态调
  `set_window_desktop_mode(true)` / `set_window_fixed_mode(true)`；不在固定模式时只记住偏好
* 持久化改为全局偏好：settings `fixed_embed_desktop`（v27 重写，未发布过所以直接改而不是加 v28）；
  `screen_configs.is_desktop` 列与 `is_desktop` 键删除，当前是否固定继续由 `is_fixed` 记录
* 前端 `WindowMode` 回到 `'normal' | 'fixed'`；`appStore.fixedEmbedDesktop` + `isEmbeddedInDesktop`
  （= isFixed && fixedEmbedDesktop）；`applyFixedMode` 按开关选命令；`body.fixed-mode` 只在
  "固定且未嵌入"时挂（嵌入态保留圆角与描边）
* 设置窗口 → 主窗口同步走既有 `app-settings-changed` 模式，key `fixedEmbedDesktop`
* 后端保留 `IS_DESKTOP_MODE` / `IS_FIXED_MODE` 两个原子量与全部 Win32 路径不变，
  托盘「固定模式」在两种状态下都勾选

## Technical Notes

* 相关文件：`pc/src-tauri/src/commands/window.rs`、`pc/src-tauri/src/lib.rs`、
  `pc/src-tauri/src/db/{models,migrations}.rs`、`pc/src-tauri/src/commands/data.rs`、
  `pc/src/stores/appStore.ts`、`pc/src/types/app.ts`、`pc/src/components/TitleBar.vue`、
  `pc/src/views/MainView.vue`
* 新增 settings 键必须走 CLAUDE.md「维护检查清单」12 项
* 方案 B 下主窗口仍是顶层窗口，MainView 打开子窗口**继续**传 `parent: appWindow`（原型已验证编辑窗口正常在主窗口之上）
* 轮询线程已存在（200ms），桌面模式的宿主存活检测 / 重挂载复用它，不新开线程
* 原型观察到的"首次点击丢失"是既有行为（普通模式同样存在），不属于本任务
* `Open Questions` 中的透明可行性已由原型回答：方案 B 顶层窗口透明 / 圆角 / DPI 全部零改动
* 自动化验证手段：`(New-Object -ComObject Shell.Application).ToggleDesktop()` 触发显示桌面；
  PowerShell + Win32 查询 `IsIconic` / `GetParent` / `GetWindowRect`；截屏比对透明效果
