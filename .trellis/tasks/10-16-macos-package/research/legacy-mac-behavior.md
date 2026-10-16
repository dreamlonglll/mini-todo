# Research: 2.5.0（Tauri）mac 版的能力、身份与 GPUI 版 mac 现状

- **Query**：2.5.0 mac 版能力清单；mac 上的数据 / 身份兼容（数据目录、日志、bundle id、单实例、开机自启）与 GPUI 版对比，
  给出 GPUI mac 版必须沿用的名字表；GPUI 代码里所有平台 cfg 在 macOS 上落到哪里、哪些会变成可见缺陷
- **Scope**：内部（git 历史 + 当前代码）+ 依赖源码（本机 cargo registry 中与两版 Cargo.lock 完全同版本的 crate）+ v2.5.0 Release 资产
- **Date**：2026-10-16

> 出处写法：`v2.5.0:<路径>:行` = `git show v2.5.0:<路径>` 的行号；`749405a:` 同理（删除 Tauri 版前的最后一个提交）；
> `reg:<crate>-<版本>/<路径>:行` = `~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/` 下的依赖源码；
> 当前代码路径相对 `pc-gpui/crates/`。所有"GPUI 在 mac 上会怎样"均为**读代码推断**（GPUI 版从未在 mac 上编译运行过），
> 标"需 CI 验证"的尤其要在 macOS runner 上确认。

---

## 0. 结论先行

1. **2.5.0 mac 版是"能跑的 Windows 版子集"**：有菜单栏图标（左键唤起 / 右键菜单 6 项）、Tauri 默认 App 菜单（⌘Q/⌘W/⌘H/编辑菜单）、
   单实例（Unix socket）、开机自启（LaunchAgent）、系统通知 + 应用内通知窗口、透明主窗口（CSS 圆角，深色主题有 mac 专属半透明样式）、
   简化的固定模式（不可缩放、不可拖、被最小化即还原、唤起临时置顶）、降级的贴边隐藏（没有全局光标，靠前端 mouseenter/leave）；
   **没有**嵌入桌面 / always-on-bottom（非 Windows 回落为普通固定）、没有收纳箱（2.5.0 尚无此功能）、字体下拉为空、Dock 图标不隐藏、
   关闭最后一个窗口即退出（Tauri 默认，2.5.0 未拦截）。未签名，README 让用户 `xattr -rd com.apple.quarantine`。
2. **身份**：bundle id `com.tauri-app.mini-todo`、`mini-todo.app`（CFBundleName / DisplayName / Executable 都是 `mini-todo`）、
   数据目录 `~/Library/Application Support/mini-todo/`（**GPUI 版 `dirs::data_local_dir()/mini-todo` 在 mac 上解析为同一路径，一致**）。
3. **GPUI 版当前与 2.5.0 mac 不一致、会影响升级 / 共存的有 5 处**：
   - 日志目录：旧 `~/Library/Logs/com.tauri-app.mini-todo/mini-todo.log`，GPUI 为 `~/Library/Application Support/com.tauri-app.mini-todo/logs/mini-todo.log`；
   - 单实例：旧 `/tmp/com_tauri_app_mini_todo_si.sock`，GPUI 非 Windows **没有单实例**（新旧两版、或两个新版可同时打开同一个库）；
   - 开机自启：旧 LaunchAgent `~/Library/LaunchAgents/mini-todo.plist`，GPUI 的 `auto-launch` 没设 `set_use_launch_agent(true)`，
     在 mac 上走 **AppleScript 登录项**（读不到旧 plist、每次启动 `osascript` 会触发"控制 System Events"授权弹窗、登录项指向裸可执行文件）；
   - 系统通知归属：旧 `notify_rust::set_application("com.tauri-app.mini-todo")`，GPUI 只设了 `appname`（mac 上无效）→ 通知显示为 **Finder**；
   - 屏幕配置 ID：旧按物理像素（如 `1_2880x1800@200`），GPUI fallback 按逻辑点、缩放恒 100% → 旧的窗口位置记录匹配不上（设备本地数据，只丢一次位置）。
4. **GPUI 的"Linux CI 专用降级"在 mac 上会变成可见缺陷**（详见 §3.3）：没有菜单栏图标；没有 App 菜单 / ⌘Q / ⌘W；
   窗口按 `TitlebarOptions{appears_transparent}` 建成带标题栏样式的 NSWindow，**左上角预计会露出红黄绿交通灯**（需 CI 截图确认）；
   运行时不能移动窗口（重置位置、贴边隐藏、多屏恢复都失效）；Retina 下"保存按 ×2、恢复按 ×1"导致**位置 / 尺寸记忆失效甚至窗口翻倍**；
   多显示器坐标在 mac 上是相对各显示器的（所有显示器原点都是 0,0）；界面字体落到 Linux 替身字体（mac 上不存在 → GPUI 回退到 Helvetica）；
   没有置顶；固定模式只剩"标题栏不画拖动区"。

---

## 1. 2.5.0 mac 版能力清单

> 2.5.0 = tag `v2.5.0`。`749405a` 比它多 32 个提交，`pc/` 下的差异只有**收纳箱**（`e1827e5`，托盘收纳箱项是 `cfg(target_os = "windows")`，
> `749405a:pc/src-tauri/src/lib.rs:294-316`）和开发隔离开关；mac 行为两者相同，以下以 v2.5.0 为准。

### 1.1 能力逐项

| 能力 | 2.5.0 mac 上 | 依据 |
|---|---|---|
| 菜单栏图标（托盘） | **有**。图标 = `app.default_window_icon()`（彩色应用图标，非 template）；左键单击抬起（300ms 去抖）→ `bring_main_window_to_front`；右键弹菜单（`show_menu_on_left_click(false)`，tray-icon mac 实现右键或"左键且 menu_on_left_click"才弹菜单） | `v2.5.0:pc/src-tauri/src/lib.rs:248-321`；`reg:tray-icon-0.21.3/src/platform_impl/macos/mod.rs:476-484` |
| 托盘菜单项 | `添加待办项` / 分隔 / `固定模式`（勾选）/ `重置位置` / `打开设置` / `开机自启动`（勾选）/ 分隔 / `退出`。**没有**收纳箱项 | `v2.5.0:pc/src-tauri/src/lib.rs:191-240` |
| 顶部 App 菜单 | **有**（Tauri 默认菜单，英文）：App 子菜单 About / Services / Hide ⌘H / Hide Others / Quit ⌘Q；File：Close Window ⌘W；Edit：Undo/Redo/Cut/Copy/Paste/Select All；View：Fullscreen；Window：Minimize/Maximize/Close；Help。2.5.0 没调 `.menu()` 也没关 `enable_macos_default_menu` | `reg:tauri-2.9.5/src/app.rs:1490,2097-2099`；`reg:tauri-2.9.5/src/menu/menu.rs:153-230` |
| 单实例 | **有**：`tauri-plugin-single-instance` 2.3.7 mac 实现 = Unix domain socket（见 §2.3）；重复启动带 `--autostart` 时静默，否则唤起主窗口 | `v2.5.0:pc/src-tauri/src/lib.rs:156-163`；`reg:tauri-plugin-single-instance-2.3.7/src/platform_impl/macos.rs` |
| 开机自启 | **有**：`tauri_plugin_autostart::init(MacosLauncher::LaunchAgent, Some(["--autostart"]))`；设置页开关 + 托盘勾选；启动时"已开启就重新 enable 一次"自愈（刷新 plist 里的路径） | `v2.5.0:pc/src-tauri/src/lib.rs:169-172,204-215,283-300`；`v2.5.0:pc/src/views/SettingsView.vue:9,125,282-296` |
| 系统通知 | **有**：`tauri-plugin-notification` → notify-rust → mac-notification-sys；release 版先 `set_application("com.tauri-app.mini-todo")`；在 async 任务里 `show()`，错误被忽略 | `v2.5.0:pc/src-tauri/src/services/notification.rs:300-315`；`reg:tauri-plugin-notification-2.3.3/src/desktop.rs:207-218` |
| 应用内通知窗口 | **有**：320×120、`always_on_top`、`skip_taskbar`、`focused(false)`、无边框；**mac 上不透明**（`transparent(true)` 只在非 macOS 加） | `v2.5.0:pc/src-tauri/src/services/notification.rs:340-360` |
| 固定模式 | **有，简化版**：前端 `setResizable(false)` + 后端 `set_window_fixed_mode(true)`；标题栏不可拖（`data-tauri-drag-region=false`）；200ms 轮询"被最小化立即还原"（tao `is_minimized`/`unminimize`，跨平台）；`WS_EX_TOOLWINDOW`（不进任务栏）是 Windows 专有，**mac 上 Dock 图标照常在** | `v2.5.0:pc/src/stores/appStore.ts:395-421`；`v2.5.0:pc/src-tauri/src/commands/window.rs:617-624,818-853`；`v2.5.0:pc/src/components/TitleBar.vue:129,268-271` |
| 置顶 | **有（临时）**：托盘唤起时置顶 600ms 后撤销；贴边唤起且"唤起时置顶"开启时置顶。非 Windows 走 tao `set_always_on_top`（NSWindow level） | `v2.5.0:pc/src-tauri/src/commands/window.rs:208,877-880,908-915,1493-1550` |
| always-on-bottom / 嵌入桌面 | **无**：`set_window_desktop_mode(true)` 在非 Windows 回落为普通固定；`set_fixed_embed_desktop` 非 Windows 只记偏好；前端 `IS_WINDOWS` 守卫；设置项只在 Windows 显示 | `v2.5.0:pc/src-tauri/src/commands/window.rs:1172-1188,1344-1347`；`v2.5.0:pc/src/stores/appStore.ts:11-13,62,399`；`v2.5.0:pc/src/views/SettingsView.vue:83,709-717` |
| 贴边自动隐藏 | **有，降级**：非 Windows `get_cursor_position()` 恒 `None`，状态机改用前端上报的"光标是否在窗口内"（`set_auto_hide_cursor_inside`，根元素 mouseenter/mouseleave）；隐藏 / 唤起靠 `set_position` 挪到屏幕边缘。设置项 mac 上照常显示。是否真能把无边框窗口挪出屏幕未经真机确认（作者 README 自述没有 Mac） | `v2.5.0:pc/src-tauri/src/commands/window.rs:418-433,521-612,658-725`；`v2.5.0:pc/src/views/MainView.vue:131-144,395,733-734`；`v2.5.0:README.md:63` |
| 收纳箱 | **无**（v2.5.0 之后的 `e1827e5` 才加，且托盘项仅 Windows） | `git log v2.5.0..749405a -- pc/` |
| 窗口透明 / 圆角 | 主窗口 **透明**：`macOSPrivateApi: true` + `transparent: true` + `decorations: false` + `shadow: false`，启动时 `set_background_color(0,0,0,0)` 让 WKWebView 底色透明；圆角 / 描边由 CSS 画；深色主题有 `body.platform-macos.dark-theme` 专属半透明样式（UA 检测）；固定模式去圆角描边。子窗口（编辑 / 设置 / 已完成 / 子任务）`transparent: false`、`decorations: false`、`parent: 主窗口` | `v2.5.0:pc/src-tauri/tauri.conf.json:12-28`；`v2.5.0:pc/src-tauri/src/lib.rs:74-85,187-188`；`v2.5.0:pc/src/stores/appStore.ts:27-31`；`v2.5.0:pc/src/styles/main.scss:232-275`；`v2.5.0:pc/src/views/MainView.vue:456-466` |
| Dock 图标 | **显示**：Info.plist 无 `LSUIElement`，代码无 `set_activation_policy` | §2.1 Info.plist |
| 关闭最后一个窗口 | **退出进程**：tauri-runtime-wry 在最后一个窗口 Destroyed 时发 `ExitRequested`，2.5.0 的 `.run(\|_, _\| {})` 没有 `prevent_exit`。主窗口标题栏**没有关闭按钮**，关闭途径是 ⌘W / ⌘Q / 托盘「退出」/ Dock 退出 | `reg:tauri-runtime-wry-2.9.3/src/lib.rs:4133-4142`；`v2.5.0:pc/src-tauri/src/lib.rs:429-431`；`v2.5.0:pc/src/components/TitleBar.vue:175-249` |
| 快捷键 | 编辑类窗口 Esc 关闭、Ctrl/**Cmd**+Enter 保存（`ctrlKey \|\| metaKey`）；系统菜单的 ⌘ 快捷键；生产环境前端拦截 F5/F12 与 Ctrl/Cmd+R/U/S/P/G/F/J/H/D/E、Ctrl/Cmd+Shift+I/J/C/R/N（⌘H 与菜单 Hide 谁先生效未验证） | `v2.5.0:pc/src/utils/editorShortcuts.ts:1-35`；`v2.5.0:pc/src/main.ts:8-62` |
| 文件对话框 | `tauri-plugin-dialog`（原生 NSSavePanel / NSOpenPanel，带扩展名过滤器） | `v2.5.0:pc/src-tauri/Cargo.toml` deps；`v2.5.0:pc/src-tauri/capabilities/default.json` |
| 更新检查 | 只有"检查 GitHub releases/latest → 标题栏红点"，没有自动更新器（CSP 放行 `https://api.github.com`） | `v2.5.0:pc/src-tauri/tauri.conf.json:31`；`v2.5.0:pc/src/components/TitleBar.vue:136-144` |
| 字体下拉 | **空**（只有占位"跟随系统"）：`get_system_fonts` 非 Windows 返回 `vec![]` | `v2.5.0:pc/src-tauri/src/commands/settings_cmd.rs:45-90`；`v2.5.0:pc/src/views/SettingsView.vue:817-833` |
| 界面字体 | CSS `-apple-system, BlinkMacSystemFont, 'Segoe UI', 'Microsoft YaHei', sans-serif` → mac 上 San Francisco + 苹方 | `v2.5.0:pc/src/styles/main.scss:150,439,450,502` |
| WebDAV 密码 | **明文**存库（DPAPI 仅 Windows）；与当前 core `secret.rs` 逐字相同（仅换行符不同） | `v2.5.0:pc/src-tauri/src/services/secret.rs:116-132` |
| 致命错误提示 | 非 Windows 只写 stderr（从 Finder 启动看不到） | `v2.5.0:pc/src-tauri/src/lib.rs:145-149` |
| 签名 / 架构 / 系统要求 | 未签名（`.app.tar.gz` 里没有 `_CodeSignature`）；universal（`--target universal-apple-darwin`）；`LSMinimumSystemVersion 10.13` | `v2.5.0:.github/workflows/build.yml:17-19,66-67,86-97`；§2.1 |

### 1.2 Release 资产与安装说明

- v2.5.0 mac 资产：`mini-todo_2.5.0_universal.dmg`、`mini-todo_universal.app.tar.gz`（`gh release view v2.5.0`）；由 `tauri-apps/tauri-action@v0`
  在 `macos-latest` 上 `--target universal-apple-darwin` 构建（`v2.5.0:.github/workflows/build.yml:15-19,86-97`）
- README mac 安装提示：Gatekeeper 报"已损坏"→ `sudo xattr -rd com.apple.quarantine /Applications/Mini\ Todo.app`（`v2.5.0:README.md:78-89`）。
  **注意 README 写错了 .app 名**：实际是 `mini-todo.app`（§2.1）
- 旧图标资源：`749405a:pc/src-tauri/icons/icon.icns`（及 32/128/128@2x png）仍在 git 历史里；当前 `crates/app/assets/` 只有 `icon.ico`

---

## 2. 数据与身份兼容

### 2.1 v2.5.0 mac 包的 Info.plist（Release 资产实测）

从 `mini-todo_universal.app.tar.gz` 解出（包内只有 `mini-todo.app/Contents/{Info.plist, MacOS/mini-todo, Resources/icon.icns}`）：

| 键 | 值 |
|---|---|
| `CFBundleIdentifier` | `com.tauri-app.mini-todo` |
| `CFBundleName` / `CFBundleDisplayName` | `mini-todo` / `mini-todo` |
| `CFBundleExecutable` | `mini-todo` |
| `CFBundleIconFile` | `icon.icns` |
| `CFBundleShortVersionString` / `CFBundleVersion` | `2.5.0` / `2.5.0` |
| `LSMinimumSystemVersion` | `10.13` |
| `NSHighResolutionCapable` | `true` |
| `LSUIElement` | **无**（Dock 图标显示） |

与配置一致：`productName: "mini-todo"`、`identifier: "com.tauri-app.mini-todo"`（`v2.5.0:pc/src-tauri/tauri.conf.json:3,5`），`bundle` 下没有 `macOS` 段
（`v2.5.0:pc/src-tauri/tauri.conf.json:39-55`）。`package_info().name` 取 productName（`reg:tauri-codegen-2.5.2/src/context.rs:270-283`）= `mini-todo`。

### 2.2 路径对比

| 项 | 2.5.0 mac | 推导依据 | GPUI 当前代码在 mac 上 | 一致？ |
|---|---|---|---|---|
| 数据目录 | `~/Library/Application Support/mini-todo/`（`data.db`、`images/`、`backups/`、`holidays/`） | `v2.5.0:pc/src-tauri/src/db/paths.rs:10-23`；dirs 5 mac：`data_local_dir()` = `~/Library/Application Support`（`reg:dirs-5.0.1/src/mac.rs:7,13`） | core `db/paths.rs:51-58` 同一表达式（`dirs = "5"`，Cargo.lock dirs 5.0.1）→ 同一路径；GPUI 新增的 `fence-icons/`、`ui-state.json` 也在这里 | **一致** |
| 数据库文件 | `data.db` | `paths.rs:16-18` | 同 | 一致 |
| 图片 asset 作用域 | `$LOCALDATA/mini-todo/images/**`；Markdown 旧链接形如 `asset://localhost/<编码绝对路径>` | `v2.5.0:pc/src-tauri/tauri.conf.json:33-36`；`v2.5.0:pc/src/utils/imageRef.ts:11` | `app/src/util/image_ref.rs:11,17,34-36` 识别 `asset://localhost/` 并按文件名映射 | 一致 |
| 日志 | `~/Library/Logs/com.tauri-app.mini-todo/mini-todo.log`（2MB 轮转，保留 1 份） | `TargetKind::LogDir{file_name: None}`（`v2.5.0:pc/src-tauri/src/lib.rs:91-104`）→ `app_log_dir()` mac 分支 `home/Library/Logs/<identifier>`（`reg:tauri-2.9.5/src/path/desktop.rs:271-289`），文件名 = `package_info().name`（`reg:tauri-plugin-log-2.8.0/src/lib.rs:575,627-635`） | `app/src/services/logging.rs:30-38` = `dirs::data_local_dir()/com.tauri-app.mini-todo/logs` → `~/Library/Application Support/com.tauri-app.mini-todo/logs/mini-todo.log`（模块注释只写了 Windows / Linux，`logging.rs:3`） | **不一致** |
| 单实例 | `/tmp/com_tauri_app_mini_todo_si.sock` | §2.3 | 无（§3.2） | **不一致** |
| 开机自启 | `~/Library/LaunchAgents/mini-todo.plist` | §2.4 | AppleScript 登录项（§2.4） | **不一致** |
| 通知归属 | bundle `com.tauri-app.mini-todo` | `reg:tauri-plugin-notification-2.3.3/src/desktop.rs:207-214` | `app/src/services/notify.rs:11-14` 非 Windows 只 `appname(APP_NAME)`；notify-rust 文档明说 mac 上无效（`reg:notify-rust-4.18.2/src/notification.rs:143-148`），mac-notification-sys 未设应用时用 `com.apple.Finder`（`reg:mac-notification-sys-0.6.15/src/lib.rs:120-137`）。`cx.set_app_identity` 在 mac 上是默认空实现（`reg:gpui-pre-0.3.9/src/platform.rs:517-519`，gpui-pre-macos 未覆写） | **不一致** |
| 屏幕配置 ID | `{n}_{W}x{H}@{scale%}`，Tauri `availableMonitors()` 物理尺寸，Retina 如 `1_2880x1800@200` | `v2.5.0:pc/src/stores/appStore.ts:106-129` | fallback `monitors()` 用 GPUI 显示器逻辑点、`scale: 1.0`（`app/src/platform/fallback.rs:36-58`）→ `1_1440x900@100` | **不一致**（`screen_configs` 是设备本地数据，后果是旧位置记录用不上、设置「屏幕配置」页多一条） |
| 窗口几何单位 | 物理像素、全局坐标（Tauri `outer_position`/`outer_size`） | `v2.5.0:pc/src-tauri/src/commands/window.rs:1567-1600` | 见 §3.3 Retina / 多屏问题 | 不一致 |
| 迁移版本 | 28 | `v2.5.0:pc/src-tauri/src/db/migrations.rs:129` | 29（v29 收纳箱表在 mac 上也会建，不用）；2.5.0 只跑 `> current` 的迁移（`migrations.rs:151-164`），降级回 2.5.0 不受影响 | 兼容 |
| WebDAV 密码 | 明文 | §1.1 | 同（`core/src/services/secret.rs:116-132`） | 一致 |
| WKWebView 本地存储 | 前端 `localStorage['subtask-expand-all']`（展开全部子任务） | `v2.5.0:pc/src/stores/appStore.ts:67-74` | GPUI 用数据目录下 `ui-state.json`；旧 localStorage 不迁移（WKWebView 数据目录位置未核实，影响仅一个界面开关） | 不迁移 |

### 2.3 单实例（tauri-plugin-single-instance 2.3.7，mac）

- socket 路径：`identifier.replace(['.', '-'], "_")` → `/tmp/com_tauri_app_mini_todo_si.sock`（`semver` feature 未开：2.5.0 Cargo.lock 中该 crate 无 semver 依赖）
  （`reg:tauri-plugin-single-instance-2.3.7/src/platform_impl/macos.rs:60-71`；`v2.5.0:pc/src-tauri/Cargo.lock`）
- 第二实例：`UnixStream::connect` 成功 → 写 `<cwd>` + `\0\0` + `args.join("\0")`，关闭连接 → `exit(0)`（`macos.rs:24-28,77-92`）
- 首个实例：connect 得 `NotFound` / `ConnectionRefused` → 删除残留 socket → `UnixListener::bind` → 每个连接 `read_to_string` 到 EOF，
  按 `\0\0` 切 cwd / args 回调（`macos.rs:29-35,94-133`）；其它错误"照常启动"；`RunEvent::Exit` 时删 socket（`macos.rs:47-58`）
- 回调：args 含 `--autostart` 静默，否则唤起主窗口（`v2.5.0:pc/src-tauri/src/lib.rs:156-163`）
- 备注：从 Finder / Dock 再次打开同一个 `.app` 时 LaunchServices 本来就只激活已运行的进程，socket 主要拦 LaunchAgent 自启、`open -n`、
  直接运行可执行文件这几种情况

### 2.4 开机自启（tauri-plugin-autostart 2.5.1 + auto-launch 0.5.0，mac）

- 旧版：`MacosLauncher::LaunchAgent` → `set_use_launch_agent(true)`，`app_name = package_info().name = "mini-todo"`，
  `app_path = current_exe().canonicalize()`（LaunchAgent 模式不截成 `.app`，即 `/Applications/mini-todo.app/Contents/MacOS/mini-todo`）
  （`reg:tauri-plugin-autostart-2.5.1/src/lib.rs:172-211`）
- auto-launch 写 `~/Library/LaunchAgents/mini-todo.plist`：`Label = mini-todo`、`ProgramArguments = [<exe 绝对路径>, "--autostart"]`、
  `RunAtLoad = true`；`is_enabled` = plist 文件存在；`disable` = 删文件（`reg:auto-launch-0.5.0/src/macos.rs:81-112,145-181,184-190`）
- GPUI 当前 `app/src/services/autostart.rs:14-22`：`AutoLaunchBuilder::new().set_app_name("mini-todo").set_app_path(<current_exe>).set_args(["--autostart"])`，
  **没有** `set_use_launch_agent(true)`；builder 默认 `use_launch_agent = false`（`reg:auto-launch-0.5.0/src/lib.rs:233-275`）→ mac 上走 AppleScript：
  - `is_enabled` = `osascript -e 'tell application "System Events" to get the name of every login item'`（`macos.rs:161-176,192-199`），
    首次调用触发"想要控制 System Events"的自动化授权弹窗；GPUI 每次启动都调（`app/src/lib.rs:84-89` → `refresh_if_enabled`），打开设置页也调（`windows/settings/general.rs:37-56`）
  - 读不到旧版 LaunchAgent → 升级用户看到"未开启"，而旧 plist 仍在（若新 .app 放在同一路径、可执行文件同名，旧 plist 照样会拉起新版）；再点开启会多一个登录项
  - 登录项路径是裸可执行文件（插件源码注释：这样在登录项里显示为 Unix 可执行文件，`reg:tauri-plugin-autostart-2.5.1/src/lib.rs:197-201`）
  - AppleScript 模式只认 `--hidden` / `--minimized`，`--autostart` 被丢掉（`macos.rs:25-26,113-124`）

### 2.5 GPUI mac 版必须沿用的名字表（参照 CLAUDE.md「兼容旧版沿用的名字」）

| 项 | 值 | 旧版出处 | GPUI 现状 |
|---|---|---|---|
| Bundle identifier | `com.tauri-app.mini-todo` | Info.plist；`v2.5.0:pc/src-tauri/tauri.conf.json:5` | 尚无 mac 打包；`app/src/lib.rs:31` 常量同值 |
| .app 名 / `CFBundleName` / `CFBundleDisplayName` | `mini-todo.app` / `mini-todo` / `mini-todo` | Info.plist | 无打包（GPUI `APP_NAME = "Mini Todo"`，`lib.rs:32`，只用于 Windows 标识与提示） |
| `CFBundleExecutable` | `mini-todo` | Info.plist | bin 名 `mini-todo`（`app/Cargo.toml:14-16`），一致 |
| 数据目录 | `~/Library/Application Support/mini-todo/` | `v2.5.0:pc/src-tauri/src/db/paths.rs:10-14` | 一致（core `db/paths.rs:51-58`） |
| 日志 | `~/Library/Logs/com.tauri-app.mini-todo/mini-todo.log`（2MB 轮转，保留 1 份，`mini-todo_<时间>.log`） | `reg:tauri-2.9.5/src/path/desktop.rs:277-282` | 不一致（`app/src/services/logging.rs:30-38`） |
| 单实例 | Unix socket `/tmp/com_tauri_app_mini_todo_si.sock`；消息 `"<cwd>\0\0<arg0>\0<arg1>…"`，写完关闭连接；首实例遇 `ConnectionRefused` / `NotFound` 删残留后 bind，退出时删除 | §2.3 | 缺失（`app/src/services/single_instance.rs:205-216`） |
| 开机自启 | LaunchAgent `~/Library/LaunchAgents/mini-todo.plist`，`Label = mini-todo`，`ProgramArguments = [<可执行文件绝对路径>, "--autostart"]`，`RunAtLoad = true`；已开启时启动刷新路径 | §2.4 | 不一致（AppleScript） |
| 系统通知 bundle | `com.tauri-app.mini-todo`（`notify_rust::set_application`） | `reg:tauri-plugin-notification-2.3.3/src/desktop.rs:207-214` | 不一致（Finder） |
| 屏幕配置 ID 口径 | 物理像素 + 缩放百分比（`{n}_{W}x{H}@{scale%}`，按像素数降序） | `v2.5.0:pc/src/stores/appStore.ts:106-129` | 不一致（逻辑点 @100） |
| Dock 图标 | 显示（无 `LSUIElement`，ActivationPolicy Regular） | Info.plist | GPUI 默认 `ActivationPolicy::Regular`（`reg:gpui-pre-0.3.9/src/platform.rs:165-171`），一致 |

> 为什么 bundle id 要沿用：通知中心的权限 / 显示设置、TCC 授权（自动化等）、LaunchServices 识别、上面的日志目录与 socket 名都挂在它上面；
> 换 id 等于换一个应用（通知需重新授权、拖进 /Applications 不覆盖旧版而是并存）。

---

## 3. GPUI 版代码中的平台 cfg 清单与 macOS 落点

> 只列非测试代码（`#[cfg(test)]` 块内的 `cfg!(windows)` / `cfg(unix)`：`core/src/fence/items.rs:979`、`core/src/fence/mod.rs:1630`、
> `core/src/todo.rs:1162`、`core/src/services/fence_fs/{watch.rs:196,icon.rs:492,ops.rs:1170,folder.rs:523-525}` 不影响运行）。
> `debug_assertions` 类开关（`MINI_TODO_*`）与平台无关，略。mac 上所有 `cfg(not(windows))` 都落到"为 Linux CI 写的实现"。

### 3.1 依赖与构建

| 位置 | 内容 | mac 上 |
|---|---|---|
| `app/Cargo.toml:43-63` | `windows` 0.62、`tray-icon`、`rfd` 只在 `cfg(windows)` | 无托盘、无 rfd 对话框 |
| `core/Cargo.toml:36-41` | `windows` 0.58 只在 Windows | — |
| `app/build.rs:13-15` | 非 Windows 目标直接 return（不嵌图标 / 版本信息） | mac 图标要靠 bundle 的 `.icns`（旧 `749405a:pc/src-tauri/icons/icon.icns`） |
| `app/src/main.rs:4` | `windows_subsystem` | 无影响 |

### 3.2 按模块

**platform/**

| 位置 | 分支 | mac 落点 | 分类 |
|---|---|---|---|
| `platform/mod.rs:53-61` | `windows.rs` / `fallback.rs` | **fallback**（整套窗口能力） | 见下行 |
| `platform/mod.rs:66-67` | `subclass`（几何锁定、永远置底）仅 Windows | 缺失 | Windows 专有 |
| `platform/fallback.rs:8-10` | `hwnd()` → `None` | 所有依赖 HWND 的守护 / 模式切换都不启动（见 window_mode） | **mac 可见缺陷**（连锁） |
| `fallback.rs:12-18` | 拖窗 `start_window_move`、最大化 `zoom_window` | GPUI mac 有实现，可用 | 可用 |
| `fallback.rs:20-29` | `window_rect_physical` = 逻辑 bounds × `scale_factor` | Retina 上是 ×2 的"物理"值 | 与下两行组合成缺陷 |
| `fallback.rs:31-34` | `set_client_rect_physical` / `set_client_position_physical` no-op（注释：GPUI 不能移动窗口） | 运行时**无法移动窗口**：托盘重置位置、贴边隐藏、通知按实际尺寸重新定位（`windows/notification.rs:101`）都失效 | **mac 可见缺陷** |
| `fallback.rs:36-58` | `monitors()`：逻辑点、`scale: 1.0`、`work = visible_bounds` | GPUI mac 的 `display.bounds()` **原点恒为 (0,0)**（`reg:gpui-pre-macos-0.3.9/src/display.rs:117-127`），窗口 bounds 也是相对所在屏幕（`reg:gpui-pre-macos-0.3.9/src/window.rs:911-933,1055-1062`）→ 多显示器全部重叠在原点；屏幕配置 ID 与旧版口径不同（§2.2） | **mac 可见缺陷**（多屏、位置记忆） |
| `fallback.rs:65-67` | 光标位置 `None` | 贴边隐藏 / 光标所在显示器（`monitor_at_cursor`）退化为主显示器 | 降级 |
| `fallback.rs:69-85` | 置顶、显示 / 隐藏、tool window、可最小化、可缩放边框、圆角、几何锁定、owner、最小化还原全部 no-op；`is_window_visible` 恒 true | 固定模式不锁缩放、无置顶、子窗口无 owner 关系、被 ⌘M 最小化后无守护 | **mac 可见缺陷**（固定 / 置顶）；圆角由界面自己画，无影响 |
| `fallback.rs:86-88` | `bring_to_front` = `activate_window` | 可用（激活 App + 前置窗口） | 可用 |
| `platform/embed.rs:49,206` | 嵌入桌面 stub（`desktop_host` None、`attach` false） | 不会被调到（`TargetMode::for_state` 只在 Windows 选嵌入） | Windows 专有，与 2.5.0 一致 |
| `platform/fence_win.rs:20,89` | 系统取色对话框；非 Windows 返回 None | 收纳箱专用，不可达 | Windows 专有 |
| `platform/file_dialog.rs:63,154` | Windows rfd；其它 GPUI `prompt_for_new_path` / `prompt_for_paths` | mac 上是原生 NSSavePanel / NSOpenPanel，可用；**无扩展名过滤器、无标题**，保存默认目录 = 家目录 | 小降级（2.5.0 有过滤器） |

**services/**

| 位置 | 分支 | mac 落点 | 分类 |
|---|---|---|---|
| `services/tray.rs:135,285` | 托盘仅 Windows；`start` / 勾选回写 no-op；另 `Icon::from_resource(1, None)`（`tray.rs:239`）是 Windows 专有 API | **没有菜单栏图标**（2.5.0 有），托盘菜单的"添加待办 / 固定 / 重置位置 / 设置 / 自启 / 退出"入口全无 | **mac 可见缺陷**（相对 2.5.0 回退） |
| `services/single_instance.rs:23-35,60,205-216` | 非 Windows `acquire` 恒成功、`listen` 空 | 无单实例；与 2.5.0 的 socket 互不识别 | **mac 可见缺陷**（可能双开同一库 → 重复提醒、同步互相 412） |
| `services/notify.rs:11-14` | 非 Windows 只 `appname` | 通知以 Finder 名义显示（§2.2） | **mac 可见缺陷** |
| `services/autostart.rs`（无 cfg） | auto-launch 默认 AppleScript | §2.4 的授权弹窗 / 名字不兼容 | **mac 可见缺陷** |
| `services/logging.rs:30-38`（无 cfg） | `data_local_dir()/<id>/logs` | 日志位置与 2.5.0 不同 | 兼容差异 |
| `services/window_mode.rs:87-97` | `on_main_opened`：`hwnd.is_some()` 才启动 200ms 轮询 | mac 上**不启动**：无贴边隐藏、无"被最小化即还原" | **mac 可见缺陷**（2.5.0 两者都有） |
| `window_mode.rs:138-145` | `TargetMode::for_state`：嵌入只在 Windows | mac 固定 = 经典固定 | 与 2.5.0 一致 |
| `window_mode.rs:263-280` | `apply_mode`：没有 hwnd 时不执行任何 Op | 经典固定只改 `AppState`（界面不画拖动区、去圆角描边），**窗口仍可缩放**、无置顶 | **mac 可见缺陷** |
| `window_mode.rs:309-320` | 嵌入偏好变化仅 Windows 生效 | — | 与 2.5.0 一致 |
| `window_mode.rs:506-555` | 托盘 / 单实例唤起：无 hwnd 时只 `activate_window`，不拉回贴边、不临时置顶 | 只剩激活 | 降级 |
| `window_mode.rs:564-600` | 重置位置：无 hwnd 时只 `resize(380×600)`，不移动 | 位置不重置 | **mac 可见缺陷**（且 mac 上本来就没有托盘入口） |
| `window_mode.rs:644-664,782-790` + `fallback.rs:20-58` | 保存用 `client_rect_physical`（×scale），恢复用 `logical_bounds_for`（按 monitors 的 scale=1 换算） | **Retina（scale 2）下保存 ×2、恢复 ×1**：中心点算出来常落在屏幕外 → 回退居中默认尺寸（位置记忆失效），落在屏内时窗口尺寸翻倍。Linux CI 设 `GPUI_X11_SCALE_FACTOR=1`（`fallback.rs:2`）所以测不出 | **mac 可见缺陷**（需 CI 验证） |
| `services/fences.rs:43,113-125,159,212-240,282,385-400,411` | 收纳箱定位 / 嵌入 / 显隐仅 Windows；`restore_all` 非 Windows 直接 return | 不可达（无托盘入口） | Windows 专有，PRD 已列 Out of Scope |

**lib.rs / windows / ui / state / theme**

| 位置 | 分支 | mac 落点 | 分类 |
|---|---|---|---|
| `app/src/lib.rs:66-74` | 非 Windows：最后一个窗口关闭即 `cx.quit()`（注释"非 Windows 没有托盘"） | 与 2.5.0 mac 行为**相同**（Tauri 也会退出）；但 GPUI 版没有 App 菜单、没有任何 `set_menus` / ⌘Q / ⌘W 绑定（全仓 grep 无 `set_menus` / `on_reopen`），主窗口标题栏也没有关闭按钮 → 退出只能靠 Dock 右键「退出」或下行的交通灯关闭钮 | **mac 可见缺陷**（缺菜单与快捷键） |
| `app/src/lib.rs:133-153` | 致命错误：Windows 弹框，其它 stderr | 从 Finder 启动看不到（与 2.5.0 相同） | 一致 |
| `windows/main_window.rs:769-776` | 主窗口关闭后仅 Windows 在这里退出 | mac 由 lib.rs 的通用回调退出 | — |
| `windows/mod.rs:186-212`（无 cfg） | 所有窗口 `titlebar: Some(TitlebarOptions{ appears_transparent: true, traffic_light_position: None })`、`window_decorations: Client`（Linux 概念） | GPUI mac 有 titlebar 时建 `Titled \| Closable \| FullSizeContentView`（可缩放再加 Resizable、可最小化再加 Miniaturizable），只把标题栏透明、标题隐藏，**没有隐藏标准窗口按钮**（`reg:gpui-pre-macos-0.3.9/src/window.rs:994-1013,1179-1182`；全文件无 `setHidden` 作用于按钮）→ **每个窗口左上角预计露出交通灯**，压在自绘标题栏上；点红钮会关窗（主窗口则退出） | **mac 可见缺陷**（需 CI 截图确认；2.5.0 是 `decorations: false` 无交通灯） |
| `ui/title_bar.rs:40-49` | Windows 走 `HTCAPTION`；其它 `on_mouse_down → start_window_move` | mac 可拖动；无标题栏双击最大化 | 小降级 |
| `windows/editor/subtasks.rs:155-191` | 子任务导入：Windows rfd（带 `.md/.txt` 过滤器）；其它 `prompt_for_paths` | mac 原生面板可用，无过滤器 | 小降级 |
| `windows/settings/general.rs:313,362-371` | "固定模式时，嵌入桌面中"仅 Windows 显示 | 隐藏 | 与 2.5.0 一致 |
| `windows/settings/general.rs:37-56,58-90`（无 cfg） | 开机自启行全平台显示 | 打开设置即调 `osascript`（§2.4） | 见 autostart |
| `windows/settings/appearance.rs:14-17` + `core/src/settings.rs:69-117` | 系统字体枚举仅 Windows（DirectWrite），其它返回空 | 字体下拉为空 | 与 2.5.0 相同（2.5.0 也空）；GPUI 有跨平台的 `TextSystem::all_font_names`（`reg:gpui-pre-0.3.9/src/text_system.rs:284`，mac 实现在 `reg:gpui-pre-apple-0.3.9/src/text_system.rs:108`） |
| `theme.rs:418-436` | 界面字体：Windows `Segoe UI` + 雅黑；**其它 `Liberation Sans` + `Noto Sans CJK SC`**（Linux 截图替身） | mac 默认没有这两款 → GPUI 回退栈 `.ZedMono, .ZedSans, Helvetica, …`（`reg:gpui-pre-0.3.9/src/text_system.rs:255-267,374-387`）→ 拉丁字落到 Helvetica，中文靠 CoreText 自动级联（未验证）；2.5.0 mac 是 San Francisco + 苹方 | **mac 可见缺陷** |
| `theme.rs:958-990` | Markdown 等宽 / 衬线：Windows Consolas / Georgia；其它 Liberation Mono / Liberation Serif + Noto CJK | 同上，回退到 Helvetica（等宽代码块会变成比例字体） | **mac 可见缺陷** |
| `state.rs:284-292` | `is_embedded_in_desktop = cfg!(windows) && …` | mac 恒经典固定外观 | 与 2.5.0 一致 |
| `util/file_link.rs:25` | `IS_WINDOWS` 决定 `file:///` 路径解析 | mac 按 POSIX 路径，`cx.reveal_path` / `cx.open_url` 走 GPUI mac 实现 | 与 2.5.0 一致（`v2.5.0:pc/src/utils/fileLink.ts:7`） |

**core/**

| 位置 | 分支 | mac 落点 | 分类 |
|---|---|---|---|
| `core/src/settings.rs:72-117` | 系统字体（见上） | 空列表 | 与 2.5.0 一致 |
| `core/src/services/secret.rs:41,116` | DPAPI / 明文 | 明文 | 与 2.5.0 一致 |
| `core/src/services/fence_fs/mod.rs:38-46` | `shell` / `sta` 仅 Windows；`UNSUPPORTED = "桌面收纳箱仅支持 Windows"` | — | Windows 专有 |
| `fence_fs/location.rs:73-84` | 公共桌面：非 Windows `None` | — | Windows 专有 |
| `fence_fs/open.rs:19-27,39-63` | 打开 / 打开所在位置：非 Windows `Err(UNSUPPORTED)` | — | Windows 专有 |
| `fence_fs/ops.rs:361-371` | 移入 / 移回（IFileOperation）：非 Windows `Err` | — | Windows 专有 |
| `fence_fs/icon.rs:67-229` | 图标提取：非 Windows `Err` | — | Windows 专有 |
| `fence_fs/folder.rs:54-68,238-252` | 隐藏文件判定（非 Windows 看 `.` 前缀）、清只读（非 Windows no-op） | — | Windows 专有功能的辅助 |

### 3.3 "Linux CI 专用降级"在 mac 上会变成可见缺陷的汇总

按影响从大到小（与 2.5.0 mac 对比）：

1. **左上角交通灯**（`windows/mod.rs:186-212` + gpui-pre-macos 样式掩码）：所有窗口，含应用内通知；2.5.0 无。需 CI 截图确认。
2. **没有菜单栏图标**（`tray.rs:285-292`）：2.5.0 有；托盘是固定模式下唯一的"唤起 / 退出 / 重置位置"入口。
3. **没有 App 菜单与 ⌘Q / ⌘W / ⌘H / ⌘M**（全仓无 `set_menus`）：2.5.0 有 Tauri 默认菜单。编辑框内复制粘贴由 gpui-component 自己的按键绑定处理，不依赖菜单（未单独核实）。
4. **窗口几何**：运行时不能移动（`fallback.rs:31-34`）；Retina 保存 / 恢复口径不一致（§3.2 window_mode 行）；多显示器原点全为 (0,0)。
   → 位置记忆、重置位置、贴边隐藏全部失效，严重时窗口每次启动翻倍。
5. **单实例缺失**：LaunchAgent / `open -n` / 新旧两版同时运行会双开同一个 `data.db`。
6. **开机自启走 AppleScript**：每次启动 `osascript` → 自动化授权弹窗；与旧 plist 不兼容。
7. **通知显示为 Finder**。
8. **固定模式只剩外观**：不锁缩放、无置顶、无最小化守护、无贴边隐藏（2.5.0 都有，哪怕是降级版）。
9. **字体**：Liberation / Noto 替身在 mac 上不存在 → Helvetica 回退，代码块非等宽；2.5.0 是系统字体。
10. 与 2.5.0 **相同、不算回退**的：关闭最后一个窗口即退出、字体下拉为空、嵌入桌面隐藏、收纳箱不可用、WebDAV 密码明文、致命错误只写 stderr、文件对话框（原生面板，但少了过滤器）。

---

## Files Found

| File Path | Description |
|---|---|
| `v2.5.0:pc/src-tauri/tauri.conf.json` | productName / identifier / macOSPrivateApi / 主窗口配置 / asset 作用域 |
| `v2.5.0:pc/src-tauri/Cargo.toml`、`Cargo.lock` | tauri 2.9.5、autostart 2.5.1、single-instance 2.3.7、log 2.8.0、notification 2.3.3、auto-launch 0.5.0、dirs 5.0.1 |
| `v2.5.0:pc/src-tauri/src/lib.rs` | 插件注册、托盘菜单、mac 透明 webview、窗口守护线程、退出行为 |
| `v2.5.0:pc/src-tauri/src/commands/window.rs` | 固定 / 嵌入 / 贴边隐藏 / 置顶的平台分支 |
| `v2.5.0:pc/src-tauri/src/services/notification.rs` | 系统通知、应用内通知窗口（mac 不透明） |
| `v2.5.0:pc/src/stores/appStore.ts`、`views/SettingsView.vue`、`styles/main.scss`、`views/MainView.vue`、`components/TitleBar.vue`、`main.ts`、`utils/editorShortcuts.ts` | 前端的 UA 平台判断、mac 深色样式、光标上报、快捷键 |
| `v2.5.0:.github/workflows/build.yml`、`README.md` | universal 构建、mac 安装提示 |
| Release `v2.5.0` 资产 `mini-todo_universal.app.tar.gz` | Info.plist 实测 |
| `crates/app/src/platform/{mod,fallback,embed,fence_win,file_dialog}.rs` | GPUI 平台层 |
| `crates/app/src/services/{tray,single_instance,autostart,notify,logging,window_mode,fences}.rs` | GPUI 平台服务 |
| `crates/app/src/{lib.rs,theme.rs,state.rs}`、`windows/{mod.rs,main_window.rs,editor/subtasks.rs,settings/general.rs,settings/appearance.rs}`、`ui/title_bar.rs`、`util/file_link.rs` | 其余平台分支 |
| `crates/core/src/{db/paths.rs,settings.rs,services/secret.rs,services/fence_fs/*}` | core 平台分支 |

## Related Specs

- `.trellis/spec/pc-gpui/release.md`、`data-compat.md`、`verification.md`、`gpui-conventions.md` —— 全部**没有** macOS 相关内容（`grep -ri macos .trellis/spec` 无结果）

## Caveats / Not Found

- 2.5.0 mac 版的贴边隐藏、置顶在真机上是否真的生效无从确认（作者 README 自述没有 Mac，`v2.5.0:README.md:63`），本文只陈述代码路径。
- GPUI 在 mac 上的所有结论都是读源码推断：交通灯是否可见、Retina 几何翻倍、Helvetica 回退、`osascript` 授权弹窗，都应在 macOS runner 上用截图 / 日志确认。
- 2.5.0 的 WKWebView 网站数据（localStorage）实际落在哪个目录没有核实；只影响"展开全部子任务"一个界面状态。
- 前端拦截 ⌘H（`main.ts`）与 Tauri 默认菜单 Hide（⌘H）谁先生效未验证。
- 下载的 Release 资产只解了 Info.plist（放在会话 scratchpad，未进仓库）；未检查二进制的 ad-hoc 签名细节。
