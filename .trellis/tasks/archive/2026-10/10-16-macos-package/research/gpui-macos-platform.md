# Research: GPUI 版（pc-gpui）在 macOS 上的平台能力

- **Query**：让 `pc-gpui`（`gpui-kit =0.7.3` = gpui-pre 0.3.9 + gpui-component 0.7.3，不升级）在 macOS 上成为可用的应用：编译、托盘、Dock/激活策略、单实例、窗口能力、通知/对话框/自启/字体/密码存储，以及没有 Mac、只能靠 CI runner 验证这一约束下的 MVP 划分
- **Scope**：mixed（本机 cargo registry 源码 + 仓库代码 + git `749405a` 旧 Tauri 版 + GitHub 上的上游源码 / issue）
- **Date**：2026-10-16
- 源码根目录简写：`REG` = `C:\Users\12197\.cargo\registry\src\index.crates.io-1949cf8c6b5b557f\`；`APP` = `pc-gpui/crates/app/src/`；`CORE` = `pc-gpui/crates/core/src/`

---

## 结论先行

1. **能做，而且不用升级 GPUI。** 0.3.9 的 mac 平台层（`gpui-pre-macos` / `gpui-pre-apple`）是 Zed 的主力平台，源码都在本机，`Cargo.lock` 已经锁定 0.3.9。macOS 需要的 API 基本齐全：`set_activation_policy`、`on_reopen`、`on_open_urls`、`set_menus`、`set_dock_menu`、`show_system_notification`（UNUserNotificationCenter）、钥匙串 `write/read_credentials`、`prompt_for_paths` / `prompt_for_new_path`、`text_system().all_font_names()`、透明窗口、`WindowKind::PopUp`/`Floating`。
   **缺的只有运行时摆窗**：GPUI 没有 set position / set level / show-hide / 运行时改 style mask。这些要用 `objc2-app-kit` 直接调 NSWindow / NSScreen，工作量相当于 `platform/windows.rs` 的一个子集。
2. **编译**：gpui-kit 对 `gpui_platform` 强制打开了 `runtime_shaders` 特性，Metal 着色器在**运行时**用 `new_library_with_source` 编译，构建期**不调用** `xcrun metal`，因此不需要 Metal 工具链。构建期只需要 cbindgen（纯 Rust）生成头文件并拼接源码。C/ObjC 依赖（rusqlite bundled、mac-notification-sys 等）需要 Xcode Command Line Tools，GitHub macOS runner 自带。
   `build.rs` 里的 embed-resource 已经按 `CARGO_CFG_TARGET_OS` 隔离。`Cargo.lock` 与平台无关，已包含全部 mac 依赖：本机 `cargo tree/fetch --locked --offline --target {aarch64,x86_64}-apple-darwin` 均成功，所以在 mac 上用 `--locked` 不会因为缺条目失败。
3. **当前 Windows 机器上做不了任何 mac 编译检查**（连 `cargo check` 都不行）。原因有两个：gpui-pre-apple 的 build.rs 按**宿主机** cfg（`#[cfg(target_os="macos")]`）决定是否生成 `stitched_shaders.metal`，Windows 宿主上它什么也不做，随后 `include_str!` 就会失败；另外多个 C/ObjC 构建脚本需要 Apple SDK。能在 Windows 上做的只有**依赖解析层面**的检查（`cargo tree --target aarch64-apple-darwin --locked`）和改依赖后更新 `Cargo.lock`。
4. **托盘**：`tray-icon 0.21` 与 GPUI 可以共存。GPUI 在 `applicationDidFinishLaunching` 里回调 `Application::run`，满足 tray-icon “run loop 已在跑且在主线程”的要求。GPUI 没有拦截 `sendEvent:`，tray-icon 的回调也在主线程。现有 `services/tray.rs` 的 channel 模式可以原样复用，只需把依赖与 `imp` 的 cfg 扩到 macOS，并把 `Icon::from_resource`（仅 Windows）换成 PNG→`from_rgba` 的模板图标。GPUI 自身**没有** status item API，只有 Dock 菜单（`set_dock_menu`）。
5. **Dock / 退出**：`QuitMode::Explicit` 在 mac 上关掉最后一个窗口**不会**退出（与 `QuitMode::Default` 在 mac 上的行为相同）。但 `lib.rs` 里 `cfg(not(windows))` 写了“窗口全关就 quit”，要改成只对 Linux 生效。
   GPUI **不会**自动建应用菜单，不调 `set_menus` 就没有 Cmd+Q，系统存储面板里的 Cmd+V 也失效。
   **注意**：GPUI 在 `didFinishLaunching` 里会按自身状态（默认 Regular）重新 `setActivationPolicy`，Info.plist 里的 `LSUIElement` 会被覆盖。要隐藏 Dock 图标只能用 `Application::with_windowing(Headless)` 或 `cx.set_activation_policy(Accessory)`。
6. **单实例**：推荐沿用旧版 `tauri-plugin-single-instance 2.3.7` mac 实现的**同名 Unix socket**（`/tmp/com_tauri_app_mini_todo_si.sock`，内容 `cwd\0\0arg0\0arg1…`），与 2.5.0 mac 版互认，新旧两版不会同开一个库。正常从 Finder/Dock 再次打开时由 LaunchServices 激活已有实例，GPUI `on_reopen` 负责“没有可见窗口时唤起主窗口”。`on_open_urls` 不需要。
7. **窗口**：透明窗口在 mac 上走 `setOpaque:NO` + 几乎透明的背景色 + CAMetalLayer 非不透明，属正式支持路径。
   现在所有窗口都用 `titlebar: Some(appears_transparent: true)`，在 mac 上会得到“透明标题栏 + 左上角红绿灯按钮”。红绿灯压在自绘标题栏上，需要隐藏（objc `standardWindowButton:` → `setHidden:`）或给它让位。
   固定模式可以用 `setMovable:NO` + 去掉 `NSWindowStyleMaskResizable` + `setLevel:` 实现。与 Windows 不同，mac 的缩放完全由 AppKit 按 style mask 处理，**运行时可以改**。
8. **通知**：有两条路。GPUI 的 `show_system_notification`（UNUserNotificationCenter）要求 `.app` 带 bundle id，且**至少 ad-hoc 签名**，否则系统拒绝授权（UNErrorDomain code 1）。notify-rust 走已弃用的 NSUserNotification，默认冒用 `com.apple.Finder` 的身份，要像旧版那样 `set_application(identifier)`。两条路都拿不到“投递失败”的结果，CI 上也点不了授权弹窗。
9. **开机自启**：`auto-launch 0.5` 在 mac 上默认走 AppleScript（System Events 登录项，会触发“自动化”授权，且丢弃 `--autostart`）。必须 `set_use_launch_agent(true)` + `app_path = current_exe()`，这样与 2.5.0 写出的 `~/Library/LaunchAgents/mini-todo.plist` 完全同名同格式。
10. **字体 / 密码**：字体下拉直接用 `cx.text_system().all_font_names()`（CoreText），零新依赖；`theme.rs` 的非 Windows 字体（Liberation Sans / Noto Sans CJK SC）mac 上没有，要加 mac 分支（`.SystemUIFont` + `PingFang SC`）。
    WebDAV 密码：2.5.0 mac 是**明文**（`secret.rs` 非 Windows 分支，现版本相同）。MVP 建议维持明文，与旧版互通。钥匙串（GPUI 内置 / `security-framework`）在未签名、ad-hoc 签名下每次升级都会弹“允许访问钥匙串”，延后。
11. **CI 可断言的面相当大**：GitHub macOS runner 的 TCC 数据库给 `/bin/bash`、`/usr/bin/osascript` 预授权了屏幕录制、辅助功能、AppleEvents、PostEvent，GPU 是 “Apple Paravirtual device” Metal。因此可以做截图、System Events 读写窗口与菜单栏图标、合成键鼠、读库断言。
    做不了：真机视觉验收（Retina、多显示器、Spaces、全屏、阴影 / 圆角细节）、通知授权与送达、钥匙串弹窗、Gatekeeper 首次打开体验。
12. **工作量粗估**：新增与修改代码约 1500–2500 行（`platform/macos.rs` ~400–600 行，托盘 / 单实例 / 菜单 / 自启 / 通知 / 字体合计 ~400–600 行，mac e2e 脚本 ~400–800 行，打包 / 签名 / dmg 的 CI ~200–300 行）。
    一个人在没有 Mac、只能经 CI 迭代的情况下，粗估 2–4 周。主要风险是迭代慢、视觉细节不可见，以及未签名分发的 Gatekeeper / 通知 / 钥匙串体验。

---

## 0. 基线事实

| 项 | 事实 | 出处 |
|---|---|---|
| gpui-kit 对平台层的特性 | `gpui_platform`（= gpui-pre-platform）固定开 `font-kit`、`x11`、`wayland`、`runtime_shaders` | `REG/gpui-kit-0.7.3/Cargo.toml:460-468` |
| runtime_shaders 传递链 | gpui-pre-platform → `gpui_macos/runtime_shaders` → `gpui_apple/runtime_shaders` | `REG/gpui-pre-platform-0.3.9/Cargo.toml:44`；`REG/gpui-pre-macos-0.3.9/Cargo.toml:40-41` |
| mac 平台 crate 版本 | `gpui-pre-macos 0.3.9`、`gpui-pre-apple 0.3.9` 已在 `pc-gpui/Cargo.lock` 中锁定，源码在本机 | `pc-gpui/Cargo.lock`（`name = "gpui-pre-macos"` / `version = "0.3.9"`）；`REG/gpui-pre-macos-0.3.9/` |
| 已确认的特性组合 | 本机 `cargo tree --locked --offline --target aarch64-apple-darwin -e features -i gpui-pre-apple`：`runtime_shaders` 与 `font-kit` 都开启 | 本机命令输出 |
| lock 对 mac 目标的完整性 | `cargo fetch --locked --offline --target aarch64-apple-darwin` 与 `x86_64-apple-darwin` 均 exit 0（依赖源码齐全、lock 无需改） | 本机命令输出 |
| tray-icon / rfd / muda 的 mac 依赖 | lock 里 `tray-icon 0.21.3`、`muda 0.17.2`、`rfd 0.16.0` 的依赖列表已含 `objc2 0.6.5`、`objc2-app-kit 0.3.2`、`objc2-foundation 0.3.2` 等 | `pc-gpui/Cargo.lock` 对应条目 |
| 本机 rustup 目标 | 只有 `x86_64-pc-windows-msvc` | `rustup target list --installed` |
| 2.5.0 mac 数据目录 | 旧版同样是 `dirs::data_local_dir()/mini-todo` = `~/Library/Application Support/mini-todo`，与现版本一致 | `git show 749405a:pc/src-tauri/src/db/paths.rs:54-56`；`CORE/db/paths.rs`（`app_data_dir`） |

---

## 1. 编译（aarch64 / x86_64-apple-darwin）

**可行性**：高。非 Windows 分支的代码已在 Linux CI 上编译过（`clippy -D warnings`），mac 走的是同一组 `cfg(not(windows))` 代码。app 里没有 `cfg(target_os="linux")` 专用路径，预计首次编译的问题集中在依赖层，应用层很少。

**Metal 着色器：运行时编译，不需要 `xcrun metal`**

- gpui-pre-apple 的 `build.rs` 在开了 `runtime_shaders` 时只做两件事：用 cbindgen 生成 `scene.h`，再与 `src/shaders.metal` 拼成 `OUT_DIR/stitched_shaders.metal`。只有没开该特性时才调用 `xcrun -sdk macosx metal … && xcrun metallib`（`REG/gpui-pre-apple-0.3.9/build.rs:18-25`、`107-124`、`126-207`）。
- 运行时：`include_str!(concat!(env!("OUT_DIR"), "/stitched_shaders.metal"))` + `device.new_library_with_source(..)`（`REG/gpui-pre-apple-0.3.9/src/metal_renderer.rs:31-34`、`251-255`）。
- Cargo 特性只增不减，`runtime_shaders` 由 gpui-kit 打开，应用侧关不掉。代价是每次启动编译一次着色器（通常几十到几百毫秒，系统有缓存）。好处是**不需要 Metal 工具链**。另外，据称 Xcode 26 起 Metal 工具链需要单独下载（未在本机核实），runtime_shaders 正好绕开这个问题。
- cbindgen 是 gpui-pre-apple 的 mac 目标 build-dependency（`REG/gpui-pre-apple-0.3.9/Cargo.toml:265`），纯 Rust，lock 里有 `cbindgen 0.28.0`。

**需要的工具链**

- Rust：edition 2024，需要 ≥1.85。`rustup target add aarch64-apple-darwin x86_64-apple-darwin`。
- Xcode 或 Command Line Tools（clang + macOS SDK），用于：`rusqlite` bundled 编译 sqlite3.c、`mac-notification-sys` 编译 `objc/notify.m`（`REG/mac-notification-sys-0.6.15/build.rs:11-33`，仅 macOS 目标），以及链接 AppKit / Metal / UserNotifications 等 framework。GitHub macOS runner 自带。
- 建议设 `MACOSX_DEPLOYMENT_TARGET=11.0`。arm64 最低就是 11.0；GPUI 自身的离线着色器路径用的是 `-mmacosx-version-min=10.15.7`（`build.rs:138`）可作参考。
- universal：在 arm64 runner 上分别 `--target aarch64-apple-darwin` 与 `--target x86_64-apple-darwin` 构建，再 `lipo -create`。lipo 之后签名失效，**必须重新 `codesign --force -s -`**（Apple Silicon 上没有签名的 arm64 可执行文件会被直接杀掉）。build.rs 在宿主机运行，arm64 宿主构建 x86_64 目标时 gpui-pre-apple 的 build.rs 同样有效。

**本仓库自身**

- `APP/../build.rs:13-15`：`CARGO_CFG_TARGET_OS != "windows"` 直接 return，embed-resource 不会执行。`embed-resource` 作为 build-dependency 在 mac 宿主上可以编译（它对非 Windows 返回 `NotWindows`，见 `build.rs:74-77`）。**已隔离**。
- `APP/../Cargo.toml:43-63`：`windows`、`tray-icon`、`rfd` 都是 `cfg(windows)` 依赖。mac 上要用托盘或 rfd，需要把后两者移到 `cfg(any(windows, target_os = "macos"))`。lock 里已有它们及其 mac 依赖，**不改 lock 也能 `--locked`**。
- 新增直接依赖（如 `objc2` / `objc2-app-kit` / `objc2-foundation` / `block2`，lock 里都已有对应版本）会改变 lock 中 `mini-todo` 的依赖列表，需要重新生成 lock。这一步**可以在 Windows 上完成**（Cargo 解析全平台），提交后 mac CI 再 `--locked`。

**能否在当前 Windows 机器上做 mac 交叉检查：基本不能**

1. `REG/gpui-pre-apple-0.3.9/build.rs:4-7` 用的是 `#[cfg(any(target_os = "macos", target_os = "ios"))]`，这是 build 脚本的**宿主机** cfg。Windows/Linux 宿主上 build.rs 什么也不生成，`metal_renderer.rs:34` 的 `include_str!` 必然编译失败。即使在 Linux 上配了 osxcross 也一样。
2. 多个 crate 的构建脚本要用 clang + Apple SDK 编译 C/ObjC（`mac-notification-sys`、`rusqlite` bundled 等）。`cargo check` 也会执行构建脚本。
3. 能做的：`cargo tree --locked --offline --target aarch64-apple-darwin …` 验证依赖解析和特性（已验证可行）；改依赖后在 Windows 上更新 lock。

**风险**：首次在 mac 上编译可能暴露 gpui-pre 快照包本身的打包问题（本机抽查：vendor 的 5 个 gpui 源文件齐全，`REG/gpui-pre-apple-0.3.9/vendor/gpui/src/`）。clippy 在 mac 上的结果与 Linux 应一致（cfg 相同）。

**工作量**：Cargo.toml 调整约 10 行；mac CI job（fmt/clippy/test/build）约 60–100 行 YAML。

---

## 2. 托盘 / 菜单栏图标

**可行性**：高。**推荐**：继续用 `tray-icon 0.21`（2.5.0 mac 版的 Tauri 托盘底层也是这个 crate），代码基本复用 `APP/services/tray.rs` 的 Windows `imp`。

**与 GPUI 共存的依据**

- tray-icon 要求：“On macOS, an event loop must be running on the main thread … create the tray icon on the main thread … make sure that the event loop is already running”（`REG/tray-icon-0.21.3/src/lib.rs:17-18`）。不在主线程时直接返回 `Error::NotMainThread`（`REG/tray-icon-0.21.3/src/platform_impl/macos/mod.rs:34-35`）。
- GPUI mac 的 `Platform::run` 先建好 `GPUIApplication`（NSApplication 子类）并 `[app run]`，`Application::run` 的回调在 `applicationDidFinishLaunching:` 里执行（`REG/gpui-pre-macos-0.3.9/src/platform.rs:551-591`、`1305-1345`）。此时 run loop 已在跑、处于主线程，所以 `lib.rs` 里 `services::tray::start(cx)`（`APP/lib.rs:79`）的调用时机正合适。
- GPUI 没有重写 `sendEvent:`、没有本地事件监视器（在 `gpui-pre-macos-0.3.9/src` 中 grep `sendEvent|addLocalMonitorForEvents|nextEventMatchingMask` 无结果），状态栏按钮的鼠标事件不会被吞。
- 事件线程：tray-icon 的 `mouseDown:` / `mouseUp:` / `rightMouseDown:` 都是 NSResponder 方法，在主线程调用 `TrayIconEvent::send`（`macos/mod.rs:316-348`、`509-560`）；muda 菜单项也通过自己的 target-action 在主线程回调。现有做法是“回调里只往 async_channel 投递，由 GPUI 前台任务处理”（`APP/services/tray.rs:218-268`），mac 上同样成立。GPUI 前台执行器跑在主 dispatch 队列上，channel 唤醒即可执行。
- 菜单分发不冲突：GPUI 的 `validateMenuItem:` / `handleGPUIMenuItem:` 只作用于 target 为 GPUI app delegate 的菜单项（`platform.rs:107-147`、`1500-1538`），muda 的菜单项有自己的 target 和 delegate。
- 左键：`with_menu_on_left_click(false)` 在 mac 上有效（`macos/mod.rs:471-492`），左键抬起会发出 `Click{Left, Up}`（`macos/mod.rs:333-348`），所以 `handle_left_click` 的去抖逻辑可以直接用。mac 习惯是左键弹菜单，这是产品选择。

**需要改的地方**

- `Icon::from_resource` 只在 Windows 上有（`REG/tray-icon-0.21.3/src/icon.rs:148-167` 带 `#[cfg(windows)]`）。mac 上要嵌一张菜单栏用的 PNG（建议 18pt @2x = 36px 的单色模板），用 `image` 解码（app 已依赖 `image`）后 `Icon::from_rgba`，并 `with_icon_as_template(true)`，深浅色菜单栏会自动反色（`lib.rs:178`、`294`、`421`）。
- 菜单项：mac 上隐藏收纳箱的三项（旧版同样只在 Windows 显示，`git show 749405a:pc/src-tauri/src/lib.rs`，`fence_menu_items` 的 cfg）。
- `APP/services/tray.rs:285-292` 的 `cfg(not(windows))` 空实现改为 `cfg(not(any(windows, target_os="macos")))`。

**GPUI 自带能力**：没有 NSStatusItem API。有 `App::set_dock_menu(Vec<MenuItem>)`（Dock 图标右键菜单，`REG/gpui-pre-0.3.9/src/app.rs:2799-2801`；mac 实现在 `platform.rs:1166-1176`、`1552-1562`）。它分发的是 GPUI Action，需要用 `actions!` 定义并 `cx.on_action` 注册全局处理。可以作为托盘的补充，但当 activation policy 为 Accessory 时没有 Dock 图标。

**风险**：低到中。仅凭源码推断，tray-icon 与 GPUI 的组合在 mac 上没有找到前例。Tauri/tao 也是“外部 NSApp + tray-icon”的结构，可以类比。要靠 CI 用 System Events 读 `menu bar 2` 的项来断言（见 §7）。
**工作量**：约 60–100 行，外加一张 PNG 资源。

---

## 3. Dock 图标、激活策略、退出

**GPUI 已有的 API**

- `App::set_activation_policy(ActivationPolicy::{Regular, Accessory})`（`REG/gpui-pre-0.3.9/src/app.rs:1967`；枚举见 `src/platform.rs:165-171`）。文档写明 Accessory 用于“an accessory app that shows windows, such as a menu bar utility”。从 Accessory 切回 Regular 后需要自己 `cx.activate(true)`。
- `Application::with_windowing(WindowingRequest::Headless)`：在 mac 上等同于启动前设为 Accessory（`app.rs:233-242`；`platform.rs:213-219`、`681-683`）。mac 实现会在 run loop 开始前设置 Accessory，避免 Dock 图标闪一下（`platform.rs:562-575`）。
- **坑**：`did_finish_launching` 无条件执行 `app.setActivationPolicy_(state.activation_policy)`，默认值是 Regular（`platform.rs:249-251`、`1305-1309`）。因此 Info.plist 里写 `LSUIElement=1` 也会在启动后被改回 Regular，出现 Dock 图标。要隐藏 Dock 图标只能用上面两个 API。如果在 `run` 回调里才调 `set_activation_policy(Accessory)`，策略会立即生效，但 Dock 图标可能闪一下。
- `QuitMode`：`Default` 在 mac 上本来就等于 Explicit（`app.rs:457-466`、`3180-3188`）。现在用的 `QuitMode::Explicit`（`APP/lib.rs:52-55`）在 mac 上关掉最后一个窗口**不退出**。GPUI app delegate 也没有实现 `applicationShouldTerminateAfterLastWindowClosed:`（`platform.rs:79-176`），系统默认同样不退出。
  - 需要改：`APP/lib.rs:67-74` 的 `#[cfg(not(target_os = "windows"))]`“窗口全关就 quit”是给 Linux CI 用的，mac 有托盘或 Dock 后应改成 `cfg(target_os = "linux")`。`APP/windows/main_window.rs:770-776` 的“主窗口关闭且无窗口则 quit”只在 Windows 上生效，mac 上不受影响。
- 退出入口：`cx.quit()` 在 mac 上是异步 `terminate:`（`platform.rs:593-611`）。Dock 右键“退出”与系统注销会直接走 `terminate:` → `applicationWillTerminate:` → `on_quit` 回调（`platform.rs:1381-1389`），不依赖菜单。
- **应用菜单**：GPUI 不会自动建主菜单。`App::set_menus`（`app.rs:2788`）→ `NSApp setMainMenu:`（`platform.rs:1150-1160`），快捷键从 keymap 反查并写进 key equivalent（`platform.rs:346-466`）。
  - MVP 至少需要：应用菜单「关于 / 设置…(cmd-,) / 隐藏(cmd-h) / 退出(cmd-q)」，以及「编辑」菜单（`MenuItem::os_action` 映射到 `cut:` / `copy:` / `paste:` / `selectAll:` 选择器，`platform.rs:387-397`）。没有编辑菜单时，`NSSavePanel` 文件名框里的 Cmd+C/V 不工作（GPUI 注释，`platform.rs:111-135`）。GPUI 窗口内的快捷键不依赖菜单（app 已用 `secondary-enter` 等，`APP/util/editor_shortcuts.rs:30-36`）。
- `on_reopen`：Dock 图标被点击、或 Finder 再次打开已运行的 app 时，只有在**没有可见窗口**的情况下才回调（`app.rs:302-312`；`platform.rs:1369-1379`）。主窗口被 `orderOut` 隐藏或关闭后，靠它唤起。

**推荐**

- MVP 用 **Regular**（有 Dock 图标与菜单栏）+ 托盘。2.5.0 mac 版也是 Regular（Tauri 默认，tauri.conf 里没有 LSUIElement），可发现性好，Cmd+Q 和 Dock 唤起都是标准行为。
- “不显示 Dock 图标”放到后续，做成设置开关（符合“平台专属能力做设置开关”的约定），用 `set_activation_policy` 在运行时切换。
- 主窗口点红色关闭按钮：mac 惯例是关窗不退出（Explicit 已满足），Dock 或托盘再唤起（`on_reopen` → `main_window::open/show`）。

**风险**：Accessory 模式下窗口不会自动拿到焦点，从托盘唤起时要 `cx.activate(true)` + `window.activate_window()`。**工作量**：菜单 + actions + 退出逻辑约 80–150 行。

---

## 4. 单实例

**现状**：`APP/services/single_instance.rs:205-216` 在非 Windows 上什么也不做。

**旧版 mac 的做法**（`tauri-plugin-single-instance 2.3.7`，旧 lock 锁定该版本，`semver` 特性非默认）：

- socket 路径 `/tmp/{identifier 中 '.' '-' 替换为 '_'}_si.sock`，即 `/tmp/com_tauri_app_mini_todo_si.sock`。启动时先 `UnixStream::connect`：成功就发送 `cwd + "\0\0" + args.join("\0")` 后 `exit(0)`；`NotFound` / `ConnectionRefused` 时删掉残留文件、`bind` 并开始监听。
- 来源：<https://github.com/tauri-apps/plugins-workspace/blob/single-instance-v2.3.7/plugins/single-instance/src/platform_impl/macos.rs>；features：<https://github.com/tauri-apps/plugins-workspace/blob/single-instance-v2.3.7/plugins/single-instance/Cargo.toml>。

**推荐做法（与 Windows 侧“同名同格式、新旧互认”的设计一致）**

1. `acquire()`：connect 同一个 socket。连上就转交参数并退出；参数含 `--autostart` 时静默退出，复用 `should_activate`。连不上就 `remove_file` + `bind`，持有 `UnixListener` 直到进程退出。
2. `listen()`：后台线程 `accept` → 解析 `cwd\0\0args`（新增一个与 `parse_payload` 并列的纯函数，可单测）→ 用 async_channel 通知前台 → `main_window::show_and_activate` + `cx.activate(true)`。
3. 退出时删除 socket 文件（`on_app_quit`）。崩溃残留由下次启动的 `ConnectionRefused` 分支清理。
4. 可选加固：在数据目录放一个 `flock` 锁文件，防止两个进程同时启动时的竞态（Tauri 实现也有同样的竞态窗口）。

**与 LaunchServices / GPUI API 的关系**

- 从 Finder / Dock / `open` 打开已在运行的同一 bundle 时，LaunchServices 不会启动第二个进程，只激活已有实例并发 reopen 事件，由 `on_reopen` 处理（见 §3）。所以 socket 主要兜住这几种情况：直接执行二进制（终端、LaunchAgent 用 exe 路径启动）、`open -n`、旧版与新版同时存在。
- `NSRunningApplication runningApplicationsWithBundleIdentifier:` 能检测，但不能转交参数，且依赖 bundle。不推荐作为主方案。
- `on_open_urls`（`app.rs:292-300`；`platform.rs:894-896`、`1476-1498`）只在注册了 URL scheme 时有用，本需求用不到。
- 开发隔离（`MINI_TODO_DATA_DIR`）时仍然跳过，与现有逻辑一致。

**风险**：低。`/tmp` 被清理或权限异常时退化为“不做单实例”（与 Tauri 相同）。**工作量**：约 100–150 行（只用 std 的 `std::os::unix::net`），外加 payload 解析单测。

---

## 5. 窗口能力

### 5.1 GPUI mac 已有 / 没有的能力

| 能力 | GPUI 0.3.9 mac | 出处 |
|---|---|---|
| 建窗位置 / 显示器 | 有：`window_bounds` + `display_id`，坐标是**显示器本地**的逻辑点（左上为原点） | `REG/gpui-pre-macos-0.3.9/src/window.rs:1030-1067`、`1297-1301` |
| 运行时移动窗口 / 改位置 | **无**（PlatformWindow 只有 `resize`） | `REG/gpui-pre-0.3.9/src/platform.rs:1194-1251`；`window.rs:1443-1457` |
| 置顶 | 只能建窗时指定：`WindowKind::Floating` → NSPanel + `NSFloatingWindowLevel`；`PopUp` → 非激活 NSPanel + `NSPopUpWindowLevel` + 所有桌面空间可见。运行时改 level **无** API | `window.rs:1015-1028`、`1210-1243` |
| 不可拖动 | 建窗时 `is_movable` → `setMovable:`；运行时**无** API | `window.rs:1171`；`REG/gpui-pre-0.3.9/src/platform.rs:2509-2513` |
| 自绘标题栏拖动 | `start_window_move()` → `performWindowDragWithEvent:`（app 的非 Windows `drag_region` 已经在用） | `window.rs:2220-2232`；`APP/ui/title_bar.rs:40-49` |
| 不可缩放 | 建窗时 `is_resizable` 决定 `NSResizableWindowMask`（**只在 `titlebar: Some` 时生效**；`titlebar: None` 的 style mask 固定为 Titled + FullSizeContentView）。缩放由 AppKit 按 style mask 处理，运行时改 mask 可行，但 GPUI 没有封装 | `window.rs:994-1013` |
| 透明背景 | `WindowBackgroundAppearance::Transparent` → `setOpaque:NO` + 背景色 alpha 0.0001（为保留阴影）+ 渲染层透明；`Blurred` 会插入 NSVisualEffectView | `window.rs:1864-1902` |
| 最大化 / 最小化 | `zoom_window` / `minimize_window`（fallback 已用 `zoom_window`） | `window.rs:1948-1966`；`APP/platform/fallback.rs:16-18` |
| 激活 | `activate_window()` → `makeKeyAndOrderFront:`；`cx.activate(bool)` → `activateIgnoringOtherApps:` | `window.rs:1791-1809`；`platform.rs:661-666` |
| 隐藏 / 显示单个窗口 | **无**（只有整个 app 的 `cx.hide()`） | `platform.rs:710-715` |
| 显示器信息 | `cx.displays()` 的 `bounds()` **原点恒为 (0,0)**（只有尺寸）；`visible_bounds()` 也是显示器本地坐标；不暴露每块屏的缩放 | `REG/gpui-pre-macos-0.3.9/src/display.rs:117-151` |
| 窗口 bounds | 相对所在显示器（左上原点）的逻辑点 | `window.rs:911-933` |
| 原生句柄 | `HasWindowHandle for Window` → `RawWindowHandle::AppKit(ns_view)`，再取 `[view window]` 得到 NSWindow | `window.rs:2422-2430`；`REG/gpui-pre-0.3.9/src/window.rs:7572` |
| 标题栏点击延迟 / AppKit 抢拖动 | `WindowOptions.app_owns_titlebar_drag = true`：整个内容视图报告为应用自管的标题栏，AppKit 不再从标题栏拖窗、不延迟点击（文档提到 macOS 27 上出现过点击延迟）。自绘标题栏的窗口**应该打开** | `REG/gpui-pre-0.3.9/src/platform.rs:2515-2526`；`window.rs:3566-3575` |

### 5.2 `fallback.rs` 各函数在 mac 上的实现路径

现状：`APP/platform/mod.rs:53-61` 中，非 Windows 一律用 `fallback.rs`，大部分是空操作（`APP/platform/fallback.rs:31-34`、`65-84`、`90-152`）。

**推荐**：新增 `platform/macos.rs`（`cfg(target_os="macos")`），fallback 只留给 Linux。用 `objc2-app-kit 0.3`（features：`NSWindow`、`NSScreen`、`NSApplication`、`NSButton`、`NSView`、`NSEvent`、`NSGraphics`、`NSRunningApplication` 等）+ `objc2-foundation 0.3`。这些在 lock 里都已有（0.3.2 / 0.6.5）。

| `platform::` 函数 | mac 实现 | 备注 |
|---|---|---|
| `hwnd()` | 可返回 `Hwnd(NSWindow 指针 as isize)` | `window_mode.rs` 的 200ms 轮询、贴边隐藏、模式切换全靠 `Hwnd` 的原始调用（`APP/services/window_mode.rs:42`、`180-200`、`356-387`、`448-463`、`514-583`）。让 `raw::*` 在 mac 上用 NSWindow 实现可以最大化复用；否则要在 window_mode 里另写一套 |
| `raw::is_alive` | 遍历 `NSApp.windows` 比对指针 | GPUI 建窗时 `setReleasedWhenClosed:NO`（`window.rs:1082-1085`），关窗后指针可能悬空，**不能直接解引用** |
| `monitors()` / `monitor_of_window` | `NSScreen.screens`：`frame` / `visibleFrame`（全局点，左下原点）按主屏高度翻转成左上原点；`backingScaleFactor`；`NSScreenNumber` = CGDirectDisplayID = GPUI `DisplayId` | fallback 的实现在 mac 上会让所有屏都落在 (0,0)，`monitor_at_cursor` 和屏幕组合 ID 都会错 |
| `cursor_position_physical()` | `NSEvent.mouseLocation`（全局点，左下原点）→ 翻转 × 缩放 | 贴边隐藏需要 |
| `window_rect_physical` / `client_rect_physical` / `raw::rect` / `raw::client_rect` | `[window frame]` / `contentRectForFrameRect:` → 翻转 → × `backingScaleFactor` | 窗口用 FullSizeContentView，外框 = 客户区（没有 Windows 那样的不可见边框问题） |
| `set_client_rect_physical` / `set_client_position_physical` / `raw::set_*` | `setFrame:display:` / `setFrameOrigin:`（逆向换算） | 同样会**同步**触发 `windowDidMove:` / `windowDidResize:` → GPUI 回调，需要沿用“写操作延后到 App 借用之外”的规则（`APP/platform/mod.rs:6-17`） |
| `set_topmost` | `setLevel:`（`NSFloatingWindowLevel` = 3 / `NSNormalWindowLevel` = 0） | 可选加 `collectionBehavior` 的 `canJoinAllSpaces` |
| `show_window` / `hide_window` / `is_window_visible` | `orderFrontRegardless`（或 `orderFront:`）/ `orderOut:` / `isVisible` | |
| `restore_if_minimized` / `raw::is_minimized` / `raw::restore` | `isMiniaturized` / `deminiaturize:` | |
| `bring_to_front` / `raw::set_foreground` | `cx.activate(true)` + `makeKeyAndOrderFront:` | |
| `set_resizable_frame` | 运行时增删 `NSWindowStyleMaskResizable` | 与 Windows 不同，GPUI mac 没有自绘缩放热区，改 mask 即可生效 |
| `set_geometry_locked` | `setMovable:NO` + 去掉 Resizable | 自绘拖动区在固定模式下本来就不渲染（`APP/ui/title_bar.rs:9-11` 的约定） |
| `set_minimizable` | 增删 `NSWindowStyleMaskMiniaturizable` | |
| `set_tool_window` | **没有逐窗口的等价物**。Dock / Cmd+Tab 是 app 级的（activation policy）；逐窗口只能用 `collectionBehavior` 的 `IgnoresCycle`（不参与 Cmd+`）/ `Transient` | 建议空操作，或只设 `IgnoresCycle` |
| `set_owner` | 可选 `addChildWindow:ordered:` | **语义不同**：mac 的子窗口会随父窗口移动。建议空操作，或只用 level 保证在上层 |
| `set_round_corners` | 空操作 | Titled 窗口的圆角由系统处理；透明窗口的圆角由界面自己画 |
| `raw::set_system_menu` / `ex_style` / `class_name` | 空操作 / 0 / None | Windows 专有 |
| `begin_window_drag` / `toggle_maximize` | 保持 GPUI（`start_window_move` / `zoom_window`） | |

### 5.3 透明窗口与标题栏

- 透明窗口走正式支持的路径（见 §5.1）。阴影、边缘高光、深色模式下窗口的 1px 描边**只能看 CI 截图确认**。
- **红绿灯**：`WindowSpec::options()` 给所有窗口 `titlebar: Some(TitlebarOptions{ appears_transparent: true, traffic_light_position: None })`（`APP/windows/mod.rs:187-212`）。在 mac 上会得到 Closable + Titled + FullSizeContentView 的窗口（`window.rs:995-1009`）、隐藏标题文字的透明标题栏（`window.rs:1180-1183`），左上角显示**系统红绿灯按钮**，压在自绘标题栏左侧。通知用的 `PopUp` 面板也一样。可选方案：
  - (a) objc：`[[window standardWindowButton:NSWindowCloseButton/MiniaturizeButton/ZoomButton] setHidden:YES]`，完全保留现有 Windows 风格的自绘按钮。
  - (b) 保留红绿灯，用 `set_traffic_light_position` 调整位置（`REG/gpui-pre-0.3.9/src/window.rs:3049`），mac 上隐藏自绘的关闭 / 最小化按钮，标题栏左侧留出约 70–80pt。这是 mac 惯例，但界面差异更大。
  - MVP 倾向 (a)，改动最小。
- 想要真正无边框（`NSWindowStyleMaskBorderless`），需要运行时改 style mask。GPUI 的窗口类重写了 `canBecomeKeyWindow` / `canBecomeMainWindow`（`window.rs:429-445` 一带），技术上可行，但偏离 GPUI 的建窗假设，不建议。
- `WindowKind::PopUp` / `Floating` / `Dialog` 都是 NSPanel（`window.rs:1021-1027`）。GPUI 没有设置 `hidesOnDeactivate`，NSPanel 的系统默认值需要在 CI 上验证。如果通知窗口在 app 不活跃时被隐藏，需要 objc `setHidesOnDeactivate:NO`。

### 5.4 固定模式在 mac 上的可行实现

| 行为 | Windows | mac 可行实现 |
|---|---|---|
| 不可拖动 / 缩放 | 子类化 `WM_NCHITTEST` | `setMovable:NO` + 去掉 Resizable mask（运行时生效） |
| 不进任务栏 / Alt+Tab | `WS_EX_TOOLWINDOW` | 只能 app 级：Accessory 策略。可作为设置项，延后 |
| Win+D 最小化后立即还原 | 200ms 轮询 | mac 没有 Win+D 语义，“显示桌面”是把窗口移开。可选 `collectionBehavior` 加 `Stationary`（Mission Control / 显示桌面时不移动） |
| 贴边隐藏 + 唤起置顶 | Win32 摆窗 + 光标轮询 | `setFrameOrigin:` + `NSEvent.mouseLocation` 轮询，可行（core `window_logic` 是纯逻辑，可复用） |
| 嵌入桌面 | Progman owner | PRD 已列为不做。技术上可用 `kCGDesktopIconWindowLevel+1` 近似，设置项在 mac 上隐藏（`state.rs:286` 与 `window_mode.rs:140` 已按 `cfg!(windows)` 处理） |

### 5.5 坐标约定（与 2.5.0 mac 的 `screen_configs` 兼容）

- 旧版用 tao：`outer_position` / `available_monitors()` 在 mac 上是“点 × backingScaleFactor”的物理像素，原点在主屏左上；显示器顺序取 CGGetActiveDisplayList。屏幕组合 ID 按“物理宽高@缩放%”生成（`CORE/settings.rs:332-356`）。
- mac 实现若采用“物理 = 点 × 所在屏 backingScaleFactor、原点为主屏左上”，屏幕顺序与 GPUI 一样用 `CGGetActiveDisplayList`（`display.rs:40-56`），就能沿用旧版记下的位置。tao 的具体换算细节**未在源码中核实**，只是推断。

**风险**：中。视觉与交互细节（红绿灯、阴影、Retina、多屏、Spaces、全屏）只能靠截图和用户反馈。
**工作量**：`platform/macos.rs` 约 400–600 行；标题栏 / 红绿灯处理约 30–80 行。

---

## 6. 其它

### 6.1 系统通知

| 方案 | 机制 | 限制 |
|---|---|---|
| A. 现状 `notify-rust 4.18.2`（mac 上走 `mac-notification-sys 0.6.15`） | 已弃用的 `NSUserNotificationCenter`；`show()` 返回的 handle 在 drop 时**异步**发送，错误被 `.ok()` 吞掉（`REG/notify-rust-4.18.2/src/macos/nsusernotifications.rs:142-161`、`252-257`） | 未调用 `set_application` 时默认冒用 `com.apple.Finder`（图标和点击行为都是 Finder 的；`REG/mac-notification-sys-0.6.15/src/lib.rs:119-138`）。旧版 Tauri 插件在 release 下调用 `notify_rust::set_application(identifier)`（<https://github.com/tauri-apps/plugins-workspace/blob/v2/plugins/notification/src/desktop.rs> 第 518-524 行附近）。现在的 `APP/services/notify.rs:11-14` 在非 Windows 上只设了 `appname`，mac 上不起作用。`Result` 恒为 Ok，“发送失败→下一分钟重试”的约定在 mac 上失效 |
| B. GPUI `cx.show_system_notification(SystemNotification{tag,title,body,actions})` | `UNUserNotificationCenter`，首次显示时自动请求授权（`REG/gpui-pre-0.3.9/src/app.rs:1839`；`REG/gpui-pre-macos-0.3.9/src/platform.rs:1113-1132`；`src/system_notifications.rs`） | 没有 bundle id 时直接禁用，只记一条日志，以避免系统 abort（`system_notifications.rs:118-128`），所以 `cargo run` 不可用。**未签名的 bundle 会被拒绝授权（UNErrorDomain code 1），ad-hoc 签名可以**（<https://github.com/jellydn/oak/pull/159>、<https://github.com/Section9Labs/rupu/pull/615>）。没有返回值；需要主线程，调度器的 System 分支要像 App 分支那样经 channel 交给前台（`APP/services/scheduler.rs:24-63`） |

**推荐**：B（现代 API，GPUI 内置，零新依赖）+ 发布包 ad-hoc 签名（universal 产物本来就必须重签）。如果想最快复刻旧版，A 只需在 mac 上加一行 `set_application(APP_IDENTIFIER)`，但它依赖已弃用的 API。两者在 CI 上都无法自动点“允许”，送达无法断言，只能断言“不崩溃、日志有记录”。

### 6.2 文件对话框

- GPUI：`prompt_for_paths` / `prompt_for_new_path` 在 mac 上是 `NSOpenPanel` / `NSSavePanel` 的 `beginWithCompletionHandler`（非阻塞、app 级、不是 sheet），支持默认目录与默认文件名，**不支持扩展名过滤和标题**（`platform.rs:898-1017`）。`APP/platform/file_dialog.rs:154-206` 的非 Windows 分支在 mac 上可以直接用。
- rfd 0.16 mac 后端支持过滤、标题和父窗口 sheet。**但现有的 Windows 用法（独立线程调用同步 `pick_file`/`save_file`）在 mac 上会经 `dispatch2::run_on_main` 同步跑到主队列再 `runModal`**（`REG/rfd-0.16.0/src/backend/macos/utils.rs:20-32`；`file_dialog.rs:18-31`），对话框期间 GPUI 的主队列任务全部停摆，所有窗口不刷新。要用 rfd，应在前台用 `AsyncFileDialog` + `set_parent`（sheet，`panel_ffi.rs:52-72`）。rfd 在 mac 上不需要额外 feature，lock 里已有其 mac 依赖。
- **推荐**：MVP 直接用 GPUI 的对话框（导入本来就按 `PK` 魔数 / JSON 区分格式，没有过滤器影响不大）；需要过滤器时再上 rfd async。

### 6.3 开机自启（auto-launch 0.5）

- mac 有两种实现（`REG/auto-launch-0.5.0/src/macos.rs:27-52`、`70-130`）：
  - LaunchAgent：写 `~/Library/LaunchAgents/{app_name}.plist`，`ProgramArguments=[app_path, args…]`，`RunAtLoad`。`app_path` 必须是存在的绝对路径。
  - AppleScript：System Events 登录项。`app_name` 被强制改成路径的 basename，除 `--hidden` / `--minimized` 外的参数会被**丢弃**，还会触发“自动化”授权。
- `AutoLaunchBuilder` 的 `use_launch_agent` 默认为 false，即 AppleScript（`REG/auto-launch-0.5.0/src/lib.rs:262-325`）。现在的 `APP/services/autostart.rs:14-22` 没有设置这一项，所以 mac 上会走 AppleScript，`--autostart` 丢失、第二实例无法静默。
- 2.5.0：`MacosLauncher::LaunchAgent` + `--autostart`（`git show 749405a:pc/src-tauri/src/lib.rs:213-216`）。插件在 LaunchAgent 模式下 `app_path = current_exe().canonicalize()`，即 `.app/Contents/MacOS/<exe>`，只有 AppleScript 模式才截到 `.app`（<https://github.com/tauri-apps/plugins-workspace/blob/autostart-v2.5.0/plugins/autostart/src/lib.rs>，`set_use_launch_agent` 附近）。`app_name` = 包名 `mini-todo`。
- **推荐**：mac 上 `set_use_launch_agent(true)`，`app_path = current_exe()`（canonicalize），名字保持 `mini-todo`，即可与旧版同名同格式（`~/Library/LaunchAgents/mini-todo.plist`）。
  `.app` 名称和可执行文件名应保持 `mini-todo.app/Contents/MacOS/mini-todo`，这样覆盖安装后旧 plist 仍然有效；`refresh_if_enabled` 的“自愈”逻辑在 mac 上同样有用。macOS 13+ 会在“登录项 / 允许在后台”里显示这一项，并弹“已添加后台项目”的通知。
- **CI 可断言**：plist 是否存在、`plutil -lint` 是否通过、`ProgramArguments` 是否正确。

### 6.4 字体

- `CORE/settings.rs:70-72` 的 `system_fonts()` 在非 Windows 上返回空列表。GPUI 的 `cx.text_system().all_font_names()` 是跨平台的：已排序去重（`REG/gpui-pre-0.3.9/src/text_system.rs:284-289`），mac 实现用 CoreText 的 `create_for_all_families`（`REG/gpui-pre-apple-0.3.9/src/text_system.rs:108-135`），Windows 实现用 DirectWrite。mac 上建议过滤掉以 `.` 开头的隐藏系统字体名。**零新依赖**，可以在 app 层替换 core 的调用（`APP/windows/settings/appearance.rs:14-17`）。
- `APP/theme.rs:422-437`、`961-985`：非 Windows 用 `Liberation Sans` + `Noto Sans CJK SC`，代码字体与衬线字体同理，这些在 mac 上没有。建议加 mac 分支：界面字体 `.SystemUIFont`（GPUI 在 mac 上映射到 `.AppleSystemUIFont`，`REG/gpui-pre-apple-0.3.9/src/text_system.rs:290`；`REG/gpui-pre-0.3.9/src/text_system.rs:1295-1313`）或 `Helvetica Neue`，中文回退 `PingFang SC`；等宽字体 `Menlo` / `SF Mono`。

### 6.5 WebDAV 密码（DPAPI 的替代）

- 2.5.0 与现版本在非 Windows 上都是**明文**存储（`git show 749405a:pc/src-tauri/src/services/secret.rs` 的 `#[cfg(not(windows))] mod platform`；现版本 `CORE/services/secret.rs:116-` 相同）。
- 可选方案：
  - (1) 维持明文：与 2.5.0 mac 共用同一个库时完全兼容。
  - (2) core 里 `cfg(target_os="macos")` 用 `security-framework`（lock 里已有 2.11.1 / 3.7.0，阻塞 API，适合 core）存通用密码，数据库只存 `keychain:` 标记。
  - (3) GPUI 的 `cx.write_credentials/read_credentials`：钥匙串 Internet Password，以 URL 为键（`REG/gpui-pre-0.3.9/src/app.rs:1789-1810`；`REG/gpui-pre-apple-0.3.9/src/keychain.rs`）。是异步的 app 层 API，与 core 的阻塞同步调用不匹配。
  - (4) `keyring` crate：新依赖，lock 里没有。
- 风险：钥匙串项的访问控制绑定 app 的代码签名。ad-hoc 签名每次构建都不同，**每次升级后首次读取都会弹出“mini-todo 想要访问钥匙串”**；未签名时更糟。数据保护钥匙串需要 Team ID 和 entitlements。另外，标记值被 2.5.0 读到时会被当作明文密码（影响降级场景）。
- **推荐**：MVP 维持 (1) 明文，与旧版行为一致，设置页提示“macOS 上密码以明文保存在本地数据库”。等有 Developer ID 签名后再考虑 (2)。

### 6.6 日志目录

`APP/services/logging.rs:30-38` 在 mac 上写到 `~/Library/Application Support/com.tauri-app.mini-todo/logs/`。2.5.0 用 tauri-plugin-log 的 `LogDir`，按 Tauri 的约定 mac 上是 `~/Library/Logs/com.tauri-app.mini-todo/`（未核实）。两者不一致，但无害，可在文档里写明。

---

## 7. 汇总

### 7.1 建议的模块划分

```
crates/app/src/platform/
  mod.rs        imp 选择改成三路：windows.rs / macos.rs / fallback.rs(linux)
  macos.rs      新增：NSWindow / NSScreen / NSEvent 实现 §5.2 的全部函数与 raw::*（NSWindow 指针放进 Hwnd(isize)）
  file_dialog.rs  mac 先用 GPUI 分支（现有 cfg(not(windows)) 即可）
crates/app/src/services/
  tray.rs       imp 改成 cfg(any(windows, macos))；图标来源按平台区分（ICO 资源 / PNG 模板）
  single_instance.rs  新增 cfg(macos) imp：Unix socket（旧 Tauri 同名同格式）
  autostart.rs  mac 上 set_use_launch_agent(true)
  notify.rs     mac：经前台调用 GPUI show_system_notification（或 notify-rust set_application）
  mac_menu.rs（新）  set_menus + actions（关于 / 设置 / 隐藏 / 退出 / 编辑）+ cmd-q 等 keybinding；on_reopen 接主窗口
crates/app/src/theme.rs   mac 字体分支
crates/app/src/lib.rs     Linux 才“窗口全关就退出”；mac 注册 on_reopen / 菜单
crates/app/src/windows/mod.rs  mac 上 app_owns_titlebar_drag: true；建窗后隐藏红绿灯（macos.rs 提供）
crates/app/assets/        菜单栏模板 PNG、mac .icns（打包用）
Cargo.toml                tray-icon、rfd 的 target 扩到 macos；macos 目标加 objc2 / objc2-app-kit / objc2-foundation（lock 已有对应版本，需要重新生成 lock）
```

### 7.2 MVP 必做与可延后

| MVP 必做 | 可延后 |
|---|---|
| mac 编译、clippy、test 进 CI；release 产出 universal `.app`（ad-hoc 签名）+ dmg | Developer ID 签名与公证（消除 Gatekeeper 拦截、钥匙串弹窗） |
| `platform/macos.rs`：显示器、摆窗、显示 / 隐藏、置顶、几何锁定、光标（主窗口位置记忆、子窗口居中、通知叠放都依赖这些） | 贴边隐藏、唤起置顶的完整移植与调优 |
| 隐藏红绿灯（或改用 mac 风格的标题栏），`app_owns_titlebar_drag` | “不显示 Dock 图标”设置项（Accessory） |
| 应用菜单（Cmd+Q / Cmd+, / 编辑菜单）+ `on_reopen` + 退出语义 | Dock 右键菜单（`set_dock_menu`） |
| 托盘（菜单栏图标与菜单） | rfd async 带过滤器的对话框 |
| 单实例（旧 Tauri socket 兼容） | 钥匙串存密码 |
| 开机自启（LaunchAgent，与 2.5.0 兼容） | 固定模式在 Spaces、全屏下的行为（Stationary、CanJoinAllSpaces） |
| 系统通知（GPUI UN + ad-hoc 签名），应用内通知窗口 | 多显示器、混合缩放的精细校正 |
| mac 字体与字体列表 | 收纳箱、嵌入桌面（PRD 明确不做） |
| 数据目录与 2.5.0 兼容（已天然满足） | |

### 7.3 GitHub macOS runner 上能自动断言的行为

依据：runner 镜像的 TCC 配置给 `/bin/bash`、`/usr/bin/osascript` 授予了 `kTCCServiceScreenCapture`、`kTCCServiceAccessibility`、`kTCCServiceAppleEvents`，并给 `/bin/bash` 授予了 `kTCCServicePostEvent`（<https://github.com/actions/runner-images/blob/main/images/macos/scripts/build/configure-tccdb-macos.sh>）。arm64 runner 的 GPU 是 `Apple Paravirtual device`（Metal 可用，但有性能和正确性问题的报告：<https://github.com/actions/runner-images/issues/14840>；Intel runner 的 Metal 问题：<https://github.com/actions/runner-images/issues/14836>）。仓库是 PUBLIC，标准 macOS runner 不计费。

| 可断言 | 手段 |
|---|---|
| 编译 / clippy / `cargo test --workspace`（core 的同步、迁移、农历；app 的纯逻辑） | 直接跑 |
| universal 产物结构、签名有效 | `lipo -info`、`codesign --verify --deep --strict`、`plutil -lint Info.plist` |
| 启动不崩溃、建库、日志 | debug 构建 + `MINI_TODO_DATA_DIR`（debug 开关只在 debug 构建生效）+ seed；检查进程存活、`data.db` 与日志内容 |
| 窗口存在、尺寸、位置、层级 | `osascript` System Events（`position` / `size of windows of process "mini-todo"`）；或 Swift 脚本调 `CGWindowListCopyWindowInfo`（`kCGWindowLayer` 可断言置顶 = 3、`kCGWindowAlpha`、`isOnscreen`） |
| 截图（透明、圆角、红绿灯是否隐藏、字体） | `screencapture -x -l <windowID>`，作为 artifact 上传。mac 字体与 Linux 不同，需要单独的基线 |
| 托盘图标与菜单 | System Events：`menu bar items of menu bar 2 of process "mini-todo"` → 点击 → 读 `menu items` → 点「打开设置」→ 断言设置窗口出现 |
| 应用菜单 | `menu bar 1` 的菜单项（退出 / 设置 / 编辑） |
| 单实例 | 再次执行二进制：立即以 exit 0 退出；第一个实例窗口数不变；socket 文件存在 |
| 开机自启 | 切换后检查 `~/Library/LaunchAgents/mini-todo.plist` 的内容 |
| 固定模式 | 切换后 CGWindow 层级、尝试用 AX 设置位置看是否被拒或保持（部分可断言） |
| 键鼠交互 | `osascript` 的 keystroke / click，或 `cliclick`（brew），配合 `sqlite3`（系统自带）读库断言（对应 Windows e2e 的“真实输入 + 按库断言”） |
| x86_64 切片 | `macos-15-intel` runner 原生跑冒烟；arm64 runner 上的 Rosetta 是否可用**未核实** |

**不能可靠断言**：通知授权与送达（需要人工点允许）、钥匙串弹窗、Gatekeeper 首次打开（隔离属性下的“仍要打开”流程）、Retina（runner 大概率是 1x 缩放）、多显示器、Spaces 与全屏、真实 GPU 上的渲染细节、阴影与边缘观感。这些只能在发布说明里写明“未经真机验收的预览版”，依赖用户反馈。

### 7.4 总体工作量与主要风险

- **工作量**（粗估）：代码 1500–2500 行；以“一人、无 Mac、每轮 CI 15–30 分钟”计，2–4 周。最大的不确定性在视觉与交互的反复调整。
- **风险排序**：
  1. 无真机：所有问题都要经过 CI 截图和日志定位，迭代慢；用户体验细节（红绿灯、阴影、焦点、Spaces）可能长期有偏差。
  2. 分发：未签名或 ad-hoc 签名的 app 首次打开会被 Gatekeeper 拦截（macOS 15 起取消了“右键 → 打开”的绕过方式，需要到“隐私与安全性”里点“仍要打开”，或执行 `xattr -dr com.apple.quarantine`）。通知需要 ad-hoc 签名；钥匙串每次升级都会弹窗。
  3. 运行时 Metal 着色器编译与 runner 的虚拟 GPU：CI 能证明“能画出来”，不能证明真机的性能与正确性。
  4. 坐标与缩放换算：GPUI 的坐标是显示器本地的，NSScreen 是全局左下原点，旧 `screen_configs` 用的是 tao 的物理像素。换算出错会导致位置记忆、通知叠放、贴边隐藏出错。
  5. GPUI 快照包在 mac 上首次编译、运行可能遇到快照包自身的问题。版本钉死不能升级，只能在应用侧绕过。
  6. 单实例与 LaunchServices、LaunchAgent 的交互（二次启动、`--autostart`）只能部分在 CI 上覆盖。

---

## Findings 附录

### Files Found

| File Path | Description |
|---|---|
| `pc-gpui/crates/app/Cargo.toml` | `cfg(windows)` 依赖：windows / tray-icon / rfd（43-63）；build-dep embed-resource（65-66） |
| `pc-gpui/crates/app/build.rs` | 非 Windows 目标直接 return（13-15） |
| `pc-gpui/crates/app/src/lib.rs` | `QuitMode::Explicit`（52-55）；非 Windows 窗口全关就 quit（67-74）；托盘、调度启动（79-82） |
| `pc-gpui/crates/app/src/platform/mod.rs` | `imp` 选择（53-61）、`Hwnd`（69-71）、全部平台函数签名 |
| `pc-gpui/crates/app/src/platform/fallback.rs` | 非 Windows 的近似 / 空实现 |
| `pc-gpui/crates/app/src/platform/file_dialog.rs` | Windows 用 rfd 同步（63-152），其它平台用 GPUI 对话框（154-206） |
| `pc-gpui/crates/app/src/services/tray.rs` | 托盘（Windows imp 135-283，非 Windows 空实现 285-292，图标 `from_resource` 239） |
| `pc-gpui/crates/app/src/services/single_instance.rs` | Windows 互斥量 + WM_COPYDATA；非 Windows 空实现（205-216） |
| `pc-gpui/crates/app/src/services/autostart.rs` | auto-launch builder，未设置 `use_launch_agent`（14-22） |
| `pc-gpui/crates/app/src/services/notify.rs` | notify-rust；非 Windows 只设 appname（11-14） |
| `pc-gpui/crates/app/src/services/scheduler.rs` | System 通知在后台线程投递，App 通知经 channel 交给前台（24-63） |
| `pc-gpui/crates/app/src/services/window_mode.rs` | 依赖 `Hwnd` 原始调用的模式切换、轮询、贴边隐藏 |
| `pc-gpui/crates/app/src/windows/mod.rs` | `WindowSpec::options()`：`titlebar: Some(appears_transparent)`、透明背景等（187-212） |
| `pc-gpui/crates/app/src/windows/main_window.rs` | 主窗口关闭后的退出逻辑（770-776） |
| `pc-gpui/crates/app/src/windows/notification.rs` | 通知窗口按主屏工作区建窗（234-289） |
| `pc-gpui/crates/app/src/ui/title_bar.rs` | `drag_region`：非 Windows 用 `start_window_move`（40-49） |
| `pc-gpui/crates/app/src/theme.rs` | 平台字体（422-437、961-985） |
| `pc-gpui/crates/app/src/services/logging.rs` | 日志目录（30-38） |
| `pc-gpui/crates/core/src/settings.rs` | `system_fonts()` 非 Windows 为空（70-72）；`screen_config_id_of`（332-356） |
| `pc-gpui/crates/core/src/services/secret.rs` | 非 Windows 明文（116-） |
| `pc-gpui/crates/core/src/db/paths.rs` | `app_data_dir()` = `data_local_dir()/mini-todo` |
| `REG/gpui-kit-0.7.3/Cargo.toml` | gpui_platform 特性含 runtime_shaders（460-468） |
| `REG/gpui-pre-apple-0.3.9/build.rs` | 宿主机 cfg 门控；runtime_shaders 时只做 cbindgen + 拼接 |
| `REG/gpui-pre-apple-0.3.9/src/metal_renderer.rs` | 运行时编译着色器（31-34、251-255） |
| `REG/gpui-pre-apple-0.3.9/src/text_system.rs` | `all_font_names`（108-135），系统 UI 字体（290） |
| `REG/gpui-pre-apple-0.3.9/src/keychain.rs` | 钥匙串 Internet Password |
| `REG/gpui-pre-macos-0.3.9/src/platform.rs` | NSApplication 子类与 delegate、激活策略、run / quit、菜单、Dock 菜单、对话框、通知、凭据、reopen / open_urls |
| `REG/gpui-pre-macos-0.3.9/src/window.rs` | 建窗 style mask / NSPanel / level、透明、拖动、bounds、rwh |
| `REG/gpui-pre-macos-0.3.9/src/display.rs` | 显示器 bounds 原点恒为 0、visible_bounds |
| `REG/gpui-pre-macos-0.3.9/src/system_notifications.rs` | UN 通知，bundle 守卫（118-128） |
| `REG/gpui-pre-0.3.9/src/app.rs` | `QuitMode`（457-466、3180-3188）、`with_windowing`（239）、`on_open_urls` / `on_reopen`（292-312）、`set_activation_policy`（1967）、`set_menus` / `set_dock_menu`（2788-2801）、凭据（1789-1810）、通知（1839）、`app_path`（1870） |
| `REG/gpui-pre-0.3.9/src/platform.rs` | `ActivationPolicy`（165-171）、`WindowOptions`（`is_movable`、`app_owns_titlebar_drag`，2490-2570） |
| `REG/tray-icon-0.21.3/src/lib.rs`、`src/platform_impl/macos/mod.rs`、`src/icon.rs` | mac 线程要求、事件、`from_resource` 仅 Windows |
| `REG/rfd-0.16.0/src/backend/macos/*` | `run_on_main` 同步模态、async sheet |
| `REG/notify-rust-4.18.2/src/macos/*`、`REG/mac-notification-sys-0.6.15/src/lib.rs`、`build.rs` | NSUserNotification、默认冒用 Finder、ObjC 构建 |
| `REG/auto-launch-0.5.0/src/{lib,macos}.rs` | LaunchAgent / AppleScript 两种模式与默认值 |
| `git 749405a: pc/src-tauri/src/{lib.rs, services/secret.rs, db/paths.rs, commands/window.rs}`、`tauri.conf.json` | 2.5.0 mac 的做法：LaunchAgent 自启、明文密码、同一数据目录、identifier `com.tauri-app.mini-todo`、tao 负责窗口能力 |

### External References

- tray-icon 0.21 文档（mac 线程与 run loop 要求）：<https://docs.rs/tray-icon/0.21.3/tray_icon/>
- tauri-plugin-single-instance 2.3.7 mac 实现（socket 路径与格式）：<https://github.com/tauri-apps/plugins-workspace/blob/single-instance-v2.3.7/plugins/single-instance/src/platform_impl/macos.rs>
- tauri-plugin-autostart 2.5.0（LaunchAgent 与 app_path 规则）：<https://github.com/tauri-apps/plugins-workspace/blob/autostart-v2.5.0/plugins/autostart/src/lib.rs>
- tauri-plugin-notification desktop（mac 上 `set_application(identifier)`）：<https://github.com/tauri-apps/plugins-workspace/blob/v2/plugins/notification/src/desktop.rs>
- UNUserNotificationCenter 拒绝未签名 app、ad-hoc 签名可用：<https://github.com/jellydn/oak/pull/159>、<https://github.com/Section9Labs/rupu/pull/615>
- GitHub macOS runner 的 TCC 预授权：<https://github.com/actions/runner-images/blob/main/images/macos/scripts/build/configure-tccdb-macos.sh>
- runner 上的 Metal 是 `Apple Paravirtual device`：<https://github.com/actions/runner-images/issues/14840>；Intel runner 的 Metal 问题：<https://github.com/actions/runner-images/issues/14836>
- Apple：NSApplication.ActivationPolicy <https://developer.apple.com/documentation/appkit/nsapplication/activationpolicy>；LSUIElement <https://developer.apple.com/documentation/bundleresources/information-property-list/lsuielement>；NSWindow.hidesOnDeactivate <https://developer.apple.com/documentation/appkit/nswindow/hidesondeactivate>

### Related Specs

- `.trellis/spec/pc-gpui/gpui-conventions.md`：后台执行、App 借用期间不同步调平台 API（mac 的 `setFrame:` 同样会同步触发 GPUI 回调，规则直接适用）
- `.trellis/spec/pc-gpui/verification.md`：验证门槛（需要补 mac 一节）
- `.trellis/spec/pc-gpui/release.md`：发布流水线与产物命名（需要补 mac 产物、签名、dmg）
- `.trellis/spec/pc-gpui/data-compat.md`：与旧版本、cloud 的数据兼容（密码存储方式变化会影响降级）
- `.trellis/spec/backend/window-modes.md`：窗口模式契约（mac 的固定模式需要补充语义）
- `.trellis/spec/guides/desktop-window-e2e-guide.md`：Windows e2e 思路，可迁移到 mac 的 osascript / CGWindow 方案

## Caveats / Not Found

- **以下均未在 mac 上实测**，结论来自源码阅读与上游资料：tray-icon 与 GPUI 共存、NSPanel 的 `hidesOnDeactivate` 默认行为对通知窗口的影响、`with_windowing(Headless)` 在 mac 上除激活策略外是否还有副作用、runtime_shaders 在 paravirtual GPU 上的编译耗时。
- 旧版 tao 在 mac 上的物理坐标、显示器顺序细节（影响 `screen_configs` 兼容）为推断，未核对 tao 源码（本机 registry 中没有 tao）。
- 2.5.0 的 tauri-plugin-log 在 mac 上的日志路径（`~/Library/Logs/<identifier>`）按 Tauri 惯例推断，未核实。
- Xcode 26 的 Metal 工具链需单独下载一说来自记忆，未在本机核实（runtime_shaders 下无影响）。
- arm64 runner 上是否装有 Rosetta 2（用于跑 x86_64 切片）未核实；`macos-15-intel` runner 可以兜底。
- `.app` 打包（Info.plist、icns、dmg、签名命令）只在与平台能力相关处提及，详细流程应另写一篇调研。
