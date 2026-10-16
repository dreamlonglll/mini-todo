# Research: macOS 收纳箱窗口（桌面层窗口）的 AppKit 行为

- 日期：2026-10-16；调研对象：gpui-pre-macos 0.3.9、objc2 0.6.5 / objc2-foundation 0.3.2 / objc2-app-kit 0.3.2 / objc2-core-graphics 0.3.2，
  `pc-gpui/crates/app/src/platform/macos/{mod,ns}.rs`
- 方法：读本机 registry 源码（行号均指 `~/.cargo/registry/src/index.crates.io-*/` 下的文件）；Apple 文档（developer.apple.com 当前页面与
  归档指南，原文见文末「来源」）；开源实现：Plash（仓库已闭源，源码取自 `refs/pull/185/head`，2025-09）、Übersicht（main，2023-12）、
  Hammerspoon `hs.canvas`、Electron `native_window_mac.mm`、Chromium `app_controller_mac.mm`。
- 第 7 节的 objc2 代码草图已在本机用 `cargo clippy --offline --target aarch64-apple-darwin -- -D warnings` 做过类型检查（scratch crate，
  不在仓库里）——**只证明 API 名、签名、cargo 特性对得上，不证明运行时行为**。
- 标 **未验证** 的结论只来自推理 / 旧资料 / 间接证据，维护者没有 Mac，只能在 CI 的 macOS runner 上确认（见第 8 节）。

## 结论先行

1. **层级：`CGWindowLevelForKey(kCGDesktopIconWindowLevelKey) + 1`（= `-2147483602`）**。在 Finder 桌面图标窗口之上一层、所有普通窗口
   （0）之下。Plash 的「浏览模式」（可交互）正是这个值，注释写明 `+1` 是为了修"有时点不到"（macOS 11.2.1）；Hammerspoon 文档写明
   "鼠标回调只有在 `desktopIcon + 1` 或更高层级才能可靠收到点击"。这一层能收点击、悬停、滚轮、Finder 拖放，`canBecomeKeyWindow = YES`
   时也能成为 key 窗口收键盘。备选：Übersicht 的可交互窗口用 `kCGNormalWindowLevel - 1`（= -1）。
2. **collectionBehavior 整体覆盖为 `CanJoinAllSpaces | Stationary | IgnoresCycle | FullScreenNone`**（GPUI PopUp 建窗时设的是
   `CanJoinAllSpaces | FullScreenAuxiliary`，要整个替换）。`Stationary` 让 Mission Control / 显示桌面不挪动、不隐藏它——非 Normal 层级的
   窗口默认是 `Transient`（在 Mission Control 里隐藏），所以必须显式加。Übersicht、Electron `type: 'desktop'` 用的就是前三项，Plash
   用 `Stationary | IgnoresCycle | FullScreenNone`（+ 可选 `CanJoinAllSpaces`）。
3. **其它属性**：`setCanHide(false)`（⌘H / 别的应用「隐藏其他」时不隐藏，Plash 同款）、`setHidesOnDeactivate(false)`（NSPanel 默认 YES；
   现有 `ns::prepare_new_window` 已经对所有窗口做了）、`setExcludedFromWindowsMenu(true)`、`setAnimationBehavior(None)`。
4. **窗口类型推荐 GPUI `WindowKind::PopUp`（仅 macOS；Windows / Linux 保持 `Normal`）**：它是带 `NSWindowStyleMaskNonactivatingPanel`
   的 NSPanel。好处：点收纳箱**不激活应用**（菜单栏不切换、隐藏中的应用不会被取消隐藏从而弹出主窗口、Stage Manager 不切舞台）；
   NSPanel **不计入** Dock 唤起的 `hasVisibleWindows`（Apple 文档原文），现有 `on_reopen` 不用改；GPUI 给 PopUp 加了 `ActiveAlways`
   跟踪区，应用不在前台时悬停也有效。代价：建窗后要改掉 GPUI 设的 PopUp 层级（101）/ 行为 / 动画；单击不会让它成为 key 窗口，
   **改名开始时要显式 `window.activate_window()`**（`makeKeyAndOrderFront:`；非激活面板成为 key 不会激活应用），中文输入法 **未验证**。
   建窗用 `show: false`，改完层级再 `orderFrontRegardless`，避免先在 101 层闪一下。
5. **Dock 唤起**：Apple 文档：AppKit 默认"检查有没有可见的 `NSWindow`（不含 `NSPanel`）"，最小化的窗口算可见。所以：收纳箱用 PopUp
   （NSPanel）→ 不影响 `on_reopen`；若改用 `Normal`（NSWindow）→ 只要有收纳箱可见，关掉主窗口后点 Dock 永远唤不回来，必须自己注册
   `kCoreEventClass/kAEReopenApplication` Apple Event handler（Apple 文档：在 AppKit 装好默认 handler 之后注册即"替换 AppKit 版本"；
   代码草图见 §5.3，已类型检查）。建议无论选哪种都加一个 e2e：有收纳箱可见时 ⌘W 主窗口 → `open <app>` → 主窗口回来。
6. **GPUI 建窗后从不再改 level / collectionBehavior**（`window.rs` 里 `setLevel_` 只在 1214/1217/1234、`setCollectionBehavior_` 只在 1239，
   都在 `MacWindow::open` 里）。但**本 crate 自己的** `ns::set_floating(false)`（`set_topmost`）会把层级改回 Normal（0），收纳箱窗口绝不能走
   这条路，要有专门的 `ns::make_desktop_layer`。
7. **CI 能断言的**：层级（`kCGWindowLayer`）、前后顺序、⌘H / 「隐藏其他」后仍在屏幕上、点击 / 改名后前台应用不变（验证非激活）、
   键盘改名落库、拖标题 / 缩放手柄改几何、滚轮改图标大小、Dock 唤起（`open <app>`）。**不能断言的**：Spaces、Mission Control、
   Stage Manager、「点按墙纸显示桌面」、Retina / 多屏、collectionBehavior 本身（CGWindowList 不暴露，只能让应用 debug 日志自报）。

---

## 1. 窗口层级

### 1.1 数值

objc2-core-graphics 0.3.2 `src/lib.rs:43-60` 与 `src/generated/CGWindowLevel.rs:9-43` 给出编译期常量（`kCGBaseWindowLevel = i32::MIN`、
`kCGNumReservedBaseWindowLevels = 5`）；按它算出的值与运行时 `CGWindowLevelForKey` 一致（Hammerspoon、James Fisher 等实测列表里
`desktopIcon = -2147483603`、`desktop = -2147483623`）：

| 名称 | 值 | 说明 |
|---|---|---|
| `kCGMinimumWindowLevel` | -2147483643 | `i32::MIN + 5` |
| `kCGDesktopWindowLevel` | -2147483623 | 壁纸层附近（Sonoma 起壁纸由 Dock / WallpaperAgent 持有，具体层号 **未验证**） |
| `kCGDesktopWindowLevel - 1` | -2147483624 | Electron `type: 'desktop'`，**收不到鼠标 / 键盘**（被上面 Finder 的图标窗口挡住） |
| `kCGDesktopIconWindowLevel` | -2147483603 | Finder 画桌面图标的窗口 |
| **`kCGDesktopIconWindowLevel + 1`** | **-2147483602** | **推荐**：Plash 浏览模式、Hammerspoon "可靠收点击"的下限 |
| `kCGBackstopMenuLevel` | -20 | |
| `kCGNormalWindowLevel - 1` | -1 | Übersicht 可交互窗口 |
| `kCGNormalWindowLevel` / `NSNormalWindowLevel` | 0 | 普通窗口 |
| `kCGFloatingWindowLevel` / `NSFloatingWindowLevel` | 3 | 本 crate 的"置顶"（`ns::set_floating`） |
| `kCGPopUpMenuWindowLevel` / GPUI `NSPopUpWindowLevel` | 101 | GPUI `WindowKind::PopUp` 建窗时的层级（`window.rs:104`、`1234`） |

objc2-app-kit 只导出 `NSNormalWindowLevel` … `NSScreenSaverWindowLevel`（`NSWindow.rs:265-292`），**没有桌面层的常量**，要么用
objc2-core-graphics（`features = ["CGWindowLevel"]`，`CGWindowLevelForKey(CGWindowLevelKey::DesktopIconWindowLevelKey)` 或常量
`kCGDesktopIconWindowLevel`），要么自己写 `i32::MIN + 45`。objc2-core-graphics 0.3.2 已在 `Cargo.lock` 里（gpui-pre-macos 依赖它），
加成 app 的直接依赖只多编一个特性。

### 1.2 开源实现怎么做

| 项目 | 层级 | collectionBehavior | 其它 | 出处 |
|---|---|---|---|---|
| **Plash**（可交互的"浏览模式"） | `.desktopIcon + 1`（或用户选「置顶」时 `.floating`）；非交互时 `.desktop` + `ignoresMouseEvents = true` | `[.stationary, .ignoresCycle, .fullScreenNone]`，设置「所有桌面空间」时加 `.canJoinAllSpaces` | `canHide = false`、`isRestorable = false`、`disableSnapshotRestoration()`；交互时 `makeKeyAndOrderFront`；`canBecomeKey/Main` 只在交互时为 true；普通 `NSWindow`（激活型） | `Plash/DesktopWindow.swift:16-30, 43-65`；`AppState.swift:271, 307-308` |
| **Übersicht** | 可交互窗口 `kCGNormalWindowLevel - 1`；背景窗口 `kCGDesktopWindowLevel` + `setIgnoresMouseEvents:YES` | `Stationary \| CanJoinAllSpaces \| IgnoresCycle` | 无边框透明全屏窗口、`NSTrackingActiveAlways` 跟踪区；`NSApplication` 子类在 `MouseEntered` 时 `makeKeyWindow`；`LSUIElement`（没有 Dock 图标） | `Uebersicht/UBWindow.m:38-49, 130-149, 160-164`；`UBApplication.m:13-19` |
| **Hammerspoon `hs.canvas`** | 任意（文档列出各层） | 任意 | 窗口类是 `NSPanel`；`clickActivating(false)` 给 styleMask 加 `NSWindowStyleMaskNonactivatingPanel`，文档："点击带鼠标回调的画布是否把 Hammerspoon 的所有窗口带到前面"；文档："A canvas object with a mouseCallback function can only reliably receive mouse click events when its window level is at `desktopIcon + 1` or higher"；拖放回调要求层级 ≤ `dragging` | `extensions/canvas/libcanvas.m:2742-2772, 3819-3847`；`Canvas.h:4` |
| **Electron** `type: 'desktop'` | `kCGDesktopWindowLevel - 1` | `CanJoinAllSpaces \| Stationary \| IgnoresCycle` | `setDisableKeyOrMainWindow:YES`；文档："desktop window will not receive focus, keyboard or mouse events" | `shell/browser/native_window_mac.mm:273-279`；`docs/api/structures/base-window-options.md:154-159` |

### 1.3 这一层能不能收到各类输入

| 输入 | 结论 | 依据 |
|---|---|---|
| 鼠标点击（左 / 右 / 双击） | **能**。窗口服务器把鼠标事件交给光标下最上面的、接收事件的窗口，层级只决定前后。`+1` 后严格在 Finder 图标窗口之上，点击不会被 Finder 截走 | Plash `+1` 注释、Hammerspoon 文档；GPUI `acceptsFirstMouse:` 恒 YES（`window.rs:284-286`、`3551-3556`），第一下点击就会送进 GPUI（`MouseDownEvent.first_mouse = true`，`2843-2856`），gpui core 不会因此丢弃点击（`interactive.rs:162, 414` 只是透出这个标记） |
| 悬停 / mouseMoved / 进出 | **能**，应用不在前台时也能：GPUI 给 `Normal` 和 `PopUp` 都加了 `MouseMoved \| MouseEnteredAndExited \| ActiveAlways \| InVisibleRect` 跟踪区（`window.rs:2499-2517`，调用点 `1219`、`1232`）。注意 `Floating` 只开 `setAcceptsMouseMovedEvents:YES`、不加跟踪区（`1212-1215`），非 key 时收不到 mouseMoved——**不要用 `Floating`** | GPUI 源码注释 "Track even when another application is active" |
| 滚轮 | **能**（macOS 把滚轮发给光标下的窗口，不要求 key）。修饰键取自滚轮事件自身的 `modifierFlags`。坑：⌃+滚轮在开了「辅助功能 → 缩放 → 用修饰键滚动缩放」时被系统吃掉，收纳箱的"Ctrl+滚轮改图标大小"在 mac 上建议同时认 ⌘+滚轮 | 一般 AppKit 行为；⌃ 缩放冲突 **未验证** 是否默认开启（默认关） |
| 从 Finder（含桌面图标）拖文件进来 | **能**。GPUI 对所有窗口类型都 `registerForDraggedTypes:@[NSFilenamesPboardType]`（`window.rs:1076-1081`），拖放方法加在窗口类上，NSWindow / NSPanel 两个类都有（`build_window_class` `429`、`491-509`）。拖放目标不要求应用在前台。坑：GPUI 对外部拖入一律返回 `NSDragOperationCopy`（`3600-3613`），光标会带绿色"+"；Finder 作为源不会因此删 / 移文件，移动由我们自己做（与 Windows 一样"真收纳"），但"+"号视觉上像复制 | GPUI 源码；Finder 作为源对 Move/Copy 都不动原文件 **未验证**（只影响光标，不影响功能） |
| 成为 key 窗口 / 键盘输入 | **能**：key 窗口的判定与层级无关，只看 `canBecomeKeyWindow`（GPUI 恒 YES，`window.rs:436-443`）。Plash 浏览模式就是对 `desktopIcon + 1` 的窗口 `makeKeyAndOrderFront` 后在网页里打字。PopUp（非激活面板）见 §4.2 | Plash `DesktopWindow.swift:19-21` |
| 光标形状（缩放手柄的 ↔ ↕） | 应用不在前台 / 窗口不是 key 时，GPUI 的 cursor rect（`resetCursorRects`）可能不生效，光标保持箭头 | **未验证**；只影响观感 |

### 1.4 已知 / 可能的坑

- **Finder 桌面窗口截点击**：层级等于 `kCGDesktopIconWindowLevel` 时与 Finder 图标窗口同层，前后顺序不定 → 偶尔点不到（Plash 注释、
  2008 年 cocoa-dev 帖子"放在 `kCGDesktopIconWindowLevel - 1` 的窗口……"）。`+1` 解决。Finder 重启（`killall Finder`）后图标窗口重建，
  仍在同一层级，我们的窗口仍在其上——**不需要** Windows 那种"Explorer 重启后重挂"的守护（**未验证**，但层级是全局排序，没有理由失效）。
- **Mission Control / 显示桌面**：非 Normal 层级的窗口默认 `Transient`（"floats in Spaces and is hidden in Mission Control"），不加
  `Stationary` 会在 Mission Control 里消失。加了 `Stationary` 后"stays visible and does not move, like the desktop window"。见 §2。
- **Stage Manager**：激活型窗口（`Normal`）被点击 → 应用激活 → Stage Manager 很可能把 Mini Todo 切成当前舞台（把正在用的应用挪到侧栏）。
  非激活面板不激活应用，不会触发。另外 Stage Manager 默认"在桌面上显示项目"关闭时会藏起桌面图标，但我们的窗口不是 Finder 的，
  不受这个开关影响。以上均 **未验证**；macOS 13 起有 `NSWindowCollectionBehaviorCanJoinAllApplications`（"don't participate in Stage
  Manager layout"），可作为可选项在 CI（macos-15）上试，默认不加。
- **Sonoma「点按墙纸以显示桌面」**（默认"始终"）：点击落在收纳箱窗口上就不会到达墙纸，不会触发显示桌面；点空白桌面照常触发，
  触发后收纳箱因 `Stationary` 留在原地。若透明像素被窗口服务器按 alpha 穿透（圆角外的四个角），点角落会触发显示桌面。**未验证**。
- **Sonoma 起的桌面小组件**：与收纳箱谁在上面取决于小组件窗口的层级，**未验证**（不影响功能）。
- **Sequoia 窗口平铺**：收纳箱窗口 `resizable: false`、层级非 0，平铺 / 拖到边缘吸附应该不作用于它，**未验证**。
- **第三方窗口管理器**（Rectangle、yabai、AeroSpace、Amethyst…）走 AX API，一般只管 `AXStandardWindow` / 层级 0 的窗口；非激活面板的
  AX subrole 会变（Hammerspoon 文档："Setting this to false changes a canvas object's AXsubrole value"），更不容易被它们平铺。**未验证**。
- **sheet / 系统对话框**：挂在收纳箱窗口上的 sheet（GPUI `window.prompt` 有父窗口时走 `beginSheetModalForWindow`）会和父窗口同层级，
  被所有应用窗口盖住。收纳箱里的确认 / 提示必须用应用自己的窗口内弹层（`ui::message_box` 一类），或者不带父窗口的应用级对话框。
  取色（Windows 的 `choose_color`）若用 `NSColorPanel`，是独立面板，注意它的层级与激活。
- **透明区域点击穿透**：GPUI 透明窗口 `opaque = NO`；AppKit 对非不透明窗口默认按 alpha 做命中测试、`setIgnoresMouseEvents:NO`
  显式设置后整块矩形都接收。CAMetalLayer 内容是否参与 alpha 命中 **未验证**。收纳箱只在圆角外透明，影响很小，先不处理。
- **TCC「桌面」文件夹权限**：非沙盒应用读写 `~/Desktop` 需要用户授权（Catalina 起），拖放进来的文件算用户意图，但把它从 `~/Desktop`
  移走要改父目录。不属于本文件范围（PRD 的另一个开放问题），CI 上用 `MINI_TODO_FENCES_ROOT` + 临时目录避开。

## 2. collectionBehavior

objc2-app-kit 0.3.2 `NSWindow.rs:146-181`（`NSWindowCollectionBehavior` bitflags）；Apple 现行文档与归档指南「Setting Window Collection
Behavior」原文：

| 组 | 常量 | Apple 原文（节选） |
|---|---|---|
| Spaces | `CanJoinAllSpaces` (1<<0) | "This option causes the window to appear on all spaces, like the menu bar." |
| Spaces | `MoveToActiveSpace` (1<<1) | 激活时把窗口移到当前空间 |
| Exposé / Mission Control | `Managed` (1<<2) | "participate in both Spaces and Exposé"；**Normal 层级窗口的默认值** |
| Exposé / Mission Control | `Transient` (1<<3) | "float in Spaces and be hidden in Exposé. This is the default behavior if the window level is not NSNormalWindowLevel." |
| Exposé / Mission Control | `Stationary` (1<<4) | "causes the window to be unaffected by Exposé; i.e. it stays visible and does not move, like the desktop window."；现行文档："Mission Control doesn't affect the window, so it stays visible and stationary, like the desktop window." |
| 循环 | `ParticipatesInCycle` (1<<5) / `IgnoresCycle` (1<<6) | 是否参与「Cycle Through Windows」（⌘`） |
| 全屏 | `FullScreenPrimary` (1<<7) / `FullScreenAuxiliary` (1<<8) / `FullScreenNone` (1<<9) | Auxiliary："displays on the same space as the full screen window"；None："doesn't support full-screen mode" |
| Stage Manager + 全屏（macOS 13+） | `Primary` (1<<16) / `Auxiliary` (1<<17) / `CanJoinAllApplications` (1<<18) | 三者互斥；CanJoinAllApplications："don't participate in Stage Manager layout but can join the windows of other apps in full screen spaces"，"Use this collection behavior for floating windows and system overlays" |

每组最多一个。**推荐值**：`CanJoinAllSpaces | Stationary | IgnoresCycle | FullScreenNone`。

| 场景 | 效果 | 把握 |
|---|---|---|
| 切换桌面空间（Spaces） | `CanJoinAllSpaces`：每个空间都显示，切换时不跟着滑动 | Apple 文档；Übersicht / Electron 同款 |
| 全屏应用的空间 | 不加 `FullScreenAuxiliary`：不进别的应用的全屏空间（即便进了，也在全屏窗口后面看不见）。`FullScreenNone`：收纳箱自己不能进全屏（GPUI PopUp 默认带的 `FullScreenAuxiliary` 要去掉）。Plash 注释：`.fullScreenNone` 保证"在全屏应用时启动也不会出现在全屏应用后面，而是在主空间" | Plash 注释；**未验证** |
| Mission Control | `Stationary`：原地、可见，像桌面；**不加就默认 Transient → 在 Mission Control 里被藏起来** | Apple 文档 |
| 显示桌面（F11 / 四指张开 / 触发角） | 显示桌面是 Mission Control（Exposé）的一个模式：`Managed` 窗口被推到屏幕外，`Stationary` 窗口不动 → **是 `Stationary` 让它留在原地**（加上它本来就在普通窗口之下，推开普通窗口后露出来）。归档指南没有单独写"Show Desktop"，这是由"unaffected by Exposé"推出的 | 推理 + 三个开源实现都用 Stationary；**未验证** |
| App Exposé（Dock 右键「显示所有窗口」/ 三指下滑） | `Stationary`：不作为缩略图出现 | **未验证** |
| ⌘` | `IgnoresCycle`：不参与 | Apple 文档 |
| Stage Manager | 见 §1.4；可选加 `CanJoinAllApplications`（macOS 13+，与 Primary/Auxiliary 互斥） | **未验证** |

GPUI 的 PopUp 建窗时设 `CanJoinAllSpaces | FullScreenAuxiliary`（`window.rs:1239-1242`），Normal 不设（默认 0 → Managed）；**之后不再改**
（全文只有这一处 `setCollectionBehavior_`）。所以建窗后整体 `setCollectionBehavior(推荐值)` 一次即可。

## 3. ⌘H、「隐藏其他」、「窗口」菜单、hidesOnDeactivate

- **`setCanHide(false)`**：Apple 文档："whether the window can hide when its application becomes hidden (during execution of the
  `NSApplication.hide(_:)` method). By default ... true"。⌘H（本应用菜单「隐藏 Mini Todo」→ GPUI `cx.hide()` → `[NSApp hide:]`，
  `platform.rs:710-715`）时收纳箱留在桌面上。别的应用的「隐藏其他」（⌥⌘H）最终也是让我们的进程执行 `hide:`，同样受 `canHide` 约束
  ——Plash 设 `canHide = false` 就是为此。"别的应用触发"这条路径 **未验证**，可在 CI 上用 `NSRunningApplication.hide()` 模拟（§8）。
- **应用被隐藏期间点收纳箱**：激活型窗口（`Normal`）被点 → 应用激活 → 激活会取消隐藏 → **主窗口等被藏起的窗口一起弹回来**（**未验证**，
  但激活隐藏中的应用会取消隐藏是常规行为）。非激活面板不激活应用，不会发生。这是选 PopUp 的又一个理由。
- **`setExcludedFromWindowsMenu(true)`**（`NSWindow.rs:751-756`）：本应用的「窗口」菜单叫"窗口"，GPUI 只在菜单名等于英文 `"Window"`
  时才 `setWindowsMenu:`（`platform.rs:313-316`），所以 AppKit 本来就不会往里列窗口；仍建议设上，Dock 右键菜单的窗口列表是否也按它
  过滤 **未验证**（NSPanel 本来也不进这些列表）。
- **`setHidesOnDeactivate(false)`**：NSPanel 默认 YES（应用失活就隐藏）。现有 `ns::prepare_new_window`（`ns.rs:234-240`）已对所有新窗口
  设 NO，收纳箱建窗也走 `windows::open_window` → `adjust_native_window` → `prepare_after_open`（`windows/mod.rs:273-281`），不要绕开。
- 「窗口」菜单的 ⌘M（`mac_menu.rs:56`）作用于 `cx.active_window()`：收纳箱成为 key（改名中）时按 ⌘M 会对没有 Miniaturizable 位的面板
  调 `miniaturize:`（AppKit 忽略 / 响铃）。`close_window`（`mac_menu.rs:126-137`）已经跳过收纳箱，建议 Minimize 也同样跳过。

## 4. 激活：点击会不会把主窗口带到前面？用 PopUp 行不行？

### 4.1 激活型窗口（`Normal` = NSWindow）被点击

- Apple 文档（`NSApplication.ActivationOptions.activateAllWindows`）："By default, activation brings only the main and key windows forward."
  ——默认只把 key / main 窗口提前，不是全部窗口（"全部窗口提前"只在点 Dock 图标时发生；且该选项自 10.15 起实际上已失效，见开发者论坛
  724143）。
- GPUI 窗口 `canBecomeKeyWindow` / `canBecomeMainWindow` 恒 YES（`window.rs:436-443`），点哪个收纳箱，它就成为 key + main，
  被"提前"的只有它自己（而且只在它自己的层级内提前，仍在所有普通窗口之下）。**主窗口不会被带到别的应用前面**（**未验证**；反例是
  Hammerspoon：画布 `canBecomeKeyWindow` 返回 NO，点它激活应用后 AppKit 把之前的 key 窗口——控制台——带到前面，所以它才有
  `clickActivating(false)` 选项）。
- 但激活本身的副作用仍在：菜单栏换成 Mini Todo、前台应用失焦、隐藏中的应用被取消隐藏（§3）、Stage Manager 可能切舞台（§1.4）、
  Dock 唤起判定把它算作可见窗口（§5）。对比：Windows 上点嵌入桌面的收纳箱也会让它拿到前台，所以"激活"本身算行为一致，
  问题出在 mac 特有的这几个副作用。

### 4.2 非激活面板（NSPanel + NonactivatingPanel）能不能收键盘

- Apple 文档 `NSNonactivatingPanelMask`："The panel can receive keyboard input without activating the owning app. Valid only for an
  instance of NSPanel or its subclasses; not valid for a window."（所以不能给 `Normal` 的 NSWindow 运行时加这个位；GPUI PopUp 在
  `initWithContentRect` 时就带上（`window.rs:1021-1023`），窗口服务器的"不激活"标记在建窗时生效。）
- `NSPanel.becomesKeyOnlyIfNeeded` 文档："If the panel is a non-activating panel, then it becomes key only if the hit view returns true from
  `needsPanelToBecomeKey`." GPUIView 没有覆盖 `needsPanelToBecomeKey`（NSView 默认 NO），所以**单击收纳箱大概率不会让它成为 key**
  ——这对点击 / 双击打开 / 拖动 / 右键菜单都无所谓（鼠标事件不要求 key）。**键盘只在改名时需要**：`start_rename`
  （`windows/fence/mod.rs:669-682`）里先 `window.activate_window()`（GPUI → `makeKeyAndOrderFront:`，`window.rs:1791-1809`），面板成为
  key、应用不激活，键盘事件进 GPUI。失焦（用户点别的应用）时面板 resign key → 输入框 Blur → `finish_rename(true)` 提交，与现有逻辑一致。
- 开源佐证：Spotlight 式浮动面板（Cindori 的 floating panel 教程、各类启动器）都是 `.nonactivatingPanel` + `canBecomeKey = true` +
  显式 `makeKeyAndOrderFront`。**中文输入法在非激活 key 面板里是否正常 未验证**（Raycast / Alfred 这类非激活面板支持中文输入，
  间接说明可行）；e2e 用 `CGEventKeyboardSetUnicodeString` 只能验证"能打字"，验证不了输入法候选窗。
- 非激活 key 面板期间菜单栏仍是别的应用的；⌘A / ⌘C / ⌘V 由 GPUI 视图的 `performKeyEquivalent:` 按 GPUI 键绑定处理
  （`window.rs` 视图类注册了 `performKeyEquivalent:`），不依赖菜单栏 **未验证**。

### 4.3 GPUI `WindowKind::PopUp` 在 mac 上意味着什么（gpui-pre-macos 0.3.9 `window.rs`）

| 项 | PopUp | Normal | 对收纳箱的影响 |
|---|---|---|---|
| 类 | `GPUIPanel`（`NSPanel` 子类，`130`） | `GPUIWindow`（`NSWindow` 子类） | 两个类由同一个 `build_window_class`（`429`）生成，方法完全相同（key/main、拖放、resize 回调…） |
| styleMask | `Titled \| Closable \| FullSizeContentView`（+ `Resizable` / `Miniaturizable` 按 spec）`\| NonactivatingPanel`（`995-1023`） | 同左，不带 NonactivatingPanel | 收纳箱 spec `resizable: false`、`minimizable: false`（`services/fences.rs:155-156`） |
| 层级 | `NSPopUpWindowLevel` = 101（`1234`） | `NSNormalWindowLevel`（`1217`） | **建窗后改成 desktopIcon+1** |
| collectionBehavior | `CanJoinAllSpaces \| FullScreenAuxiliary`（`1239-1242`） | 不设 | **建窗后整体替换** |
| 动画 | `setAnimationBehavior: NSWindowAnimationBehaviorUtilityWindow`（`1235-1238`） | 默认 | 改成 `None`，否则「显示 / 隐藏所有收纳箱」时有淡入淡出 |
| 跟踪区 | 有（`1232`，`ActiveAlways`） | 有（`1219`）+ `setAcceptsMouseMovedEvents:NO` | 悬停两者都行 |
| 拖放注册 | 有（`1076-1081`，所有类型） | 有 | 一样 |
| `acceptsFirstMouse` | YES（视图类，`284-286`） | YES | 一样 |
| `canBecomeKey/Main` | YES | YES | 一样 |
| 显示 | `focus && show` → `makeKeyAndOrderFront:`；`show` → `orderFront:`（`1291-1295`）；`show: false` 不显示 | 同 | **mac 上也用 `show: false`**（现在是 `spec.show = !cfg!(windows)`，`services/fences.rs:159`），设好层级后 `orderFrontRegardless` |
| `start_window_move` | `performWindowDragWithEvent:` + `NSApp.currentEvent`（`2220-2231`） | 同 | NSPanel 继承 NSWindow 的这个方法，标题栏拖动照常（`app_owns_titlebar_drag` 的 `_opaqueRectForWindowMoveWhenInTitlebar` 在视图类上，两者相同） |
| `becomesKeyOnlyIfNeeded` / `setFloatingPanel` | GPUI 都不调（NSPanel 默认 NO / NO） | — | **不要调 `setFloatingPanel:`**：它会把层级改成 floating（YES）或 normal（NO） |
| `hidesOnDeactivate` | GPUI 不设（NSPanel 默认 YES） | 默认 NO | `prepare_after_open` 已设 NO |
| tabbing | 只有 `tabbing_identifier.is_some()` 才自动并入标签页（`987-993`、`1261-1290`）；本应用不设 | 同 | 不会被并成主窗口的标签页 |
| 假 `windowDidBecomeKey` | GPUI 对"应用不在前台时打开 pop-up，Cocoa 给上一个 key 窗口发假的 didBecomeKey"做了 `resignKeyWindow` 补偿（`3157-3172`） | — | 无需处理 |
| gpui core | `WindowKind::PopUp` 在 gpui-pre core 里没有任何特殊分支（只有 `platform.rs:2711-2712` 的定义与文档） | — | 焦点、键盘分发与 Normal 相同 |

### 4.4 GPUI 会不会在建窗后重设 level / collectionBehavior

不会。`window.rs` 全文：`setLevel_` 只出现在 `1214`、`1217`、`1234`，`setCollectionBehavior_` 只在 `1239`，全部在 `MacWindow::open` 里；
`activate`（`1791-1809`）只 `makeKeyAndOrderFront:`；`toggle_fullscreen` 走 `toggleFullScreen:`（要求 FullScreenPrimary/Auxiliary，
我们设了 FullScreenNone，调用无效果）；`platform.rs` 没有任何改窗口层级的代码。AppKit 本身：`makeKeyAndOrderFront:` / `orderFront:` /
`setStyleMask:` 都不改层级；只有 `setFloatingPanel:` 会改（见上）。**本 crate 里会改层级的只有 `ns::set_floating`**（`set_topmost` /
`raw::set_topmost`，`ns.rs:171-179`），当前收纳箱代码没有调用它（`services/fences.rs`、`windows/fence/` 里没有 `set_topmost`），
以后也不能调（会把收纳箱拉到 0 或 3 层）。

### 4.5 PopUp vs Normal：推荐 PopUp

| | PopUp（非激活 NSPanel） | Normal（NSWindow） |
|---|---|---|
| 点击激活应用 | 否 | 是（菜单栏切换、前台应用失焦） |
| 应用被 ⌘H 隐藏时点收纳箱 | 不取消隐藏 | 取消隐藏，主窗口弹回（**未验证**） |
| Stage Manager | 不切舞台（**未验证**） | 可能切舞台（**未验证**） |
| Dock 唤起 `hasVisibleWindows` | 不计入（Apple 文档） | 计入 → 必须自己处理 reopen（§5） |
| 键盘（改名） | 改名开始时显式 `activate_window()`；输入法 **未验证** | 点击即 key，标准路径 |
| 建窗后要改的 | 层级、行为、动画 | 层级、行为 |
| 悬停 / 拖放 / 点击 / 拖动 | 相同 | 相同 |

PopUp 只在 macOS 上用：Windows 的 PopUp 是另一套（置顶弹窗），收纳箱在 Windows 上继续 `Normal` + 嵌入桌面；Linux 截图回归也保持 `Normal`。
**退路**：若 CI 证明非激活面板收不到键盘（改名用例失败），改用 `Normal` + §5.3 的 reopen handler，其余设置不变。
`Floating`（激活型 NSPanel）不考虑：点击仍激活应用，且没有 `ActiveAlways` 跟踪区（悬停只在 key 时有效）。

## 5. Dock 唤起（`hasVisibleWindows`）

### 5.1 AppKit 怎么算

Apple 文档 `applicationShouldHandleReopen(_:hasVisibleWindows:)` 原文：

> By default the Application Kit will handle this event by checking whether there are any visible `NSWindow` (not `NSPanel`) objects, and,
> if there are none, it goes through the standard untitled document creation …
> Miniaturized windows, windows in the Dock, are considered visible by this method, and cause `flag` to return true, despite the fact that
> miniaturized windows return false when sent an `isVisible` message.

- **NSPanel 不计入**：现行文档原文如此；2004 年 cocoa-dev 帖子「Clicking Dock Icon creates a new window」的结论相同（把文档窗口的类从
  NSPanel 改回 NSWindow 后问题消失）。相关佐证：`NSApplication.orderedWindows` 文档"panels are not included"。
  反向的弱证据：Chromium `app_controller_mac.mm:1853-1874` 的注释说 `hasVisibleWindows` 为真时"windows are panels or notifications"
  也有可能——这段注释年代久远（Chromium 旧 Panels 功能），与 Apple 文档冲突，以文档为准，但**用 e2e 实测**。
- **层级、canHide 都不影响计入**：只看"可见的 NSWindow（含最小化）"。所以 `Normal` 的收纳箱一旦可见，flag 恒为 YES。
- GPUI `should_handle_reopen`（`platform.rs:1369-1379`）只在 `!has_open_windows` 时回调 `on_reopen`（本应用 `mac_menu::on_reopen` →
  `window_mode::bring_main_to_front`）。附带：这个函数按 `extern "C" fn(...) -> ()` 注册（`platform.rs:99-102`），AppKit 期待返回 `BOOL`，
  实际读到的是寄存器残值（大概率非零 = "照常处理"），对本应用无害，不用管。

### 5.2 方案对比

| 方案 | 做法 | 评价 |
|---|---|---|
| (c) 收纳箱用 PopUp（NSPanel） | 什么都不用加 | **推荐**。依赖 Apple 文档的"not NSPanel"；e2e 验证（§8 的 F-reopen）。与已有的 PopUp 应用内通知同理（它们也从未影响 `on_reopen`） |
| (a) 自己注册 `kCoreEventClass`('aevt') / `kAEReopenApplication`('rapp') handler | `NSAppleEventManager.setEventHandler:andSelector:forEventClass:andEventID:`，handler 是 `define_class!` 定义的 ObjC 对象；每次 reopen 都唤起主窗口 | **退路 / 加固**。Apple 归档指南原文："A good place to install event handlers is in the `applicationWillFinishLaunching:` method … At that point, the Application Kit has installed its default event handlers, so if you install a handler for one of the same events, it will replace the Application Kit version." "If an event handler is already installed for the specified event class and event ID, it is replaced." → 装上后 AppKit 的 reopen 处理（含调用 delegate 的 `applicationShouldHandleReopen:`、即 GPUI 的 `on_reopen`）**不再执行**，完全由我们决定 |
| (b) 监听 `NSApplicationDidBecomeActiveNotification` | 激活时判断主窗口是否可见 | **不推荐**：应用已在前台时点 Dock 不会再发激活通知（收不到）；⌘Tab、点通知、点激活型收纳箱都会触发（误唤起） |
| (d) 替换 GPUI delegate 的 `applicationShouldHandleReopen:hasVisibleWindows:` 实现（`class_replaceMethod`） | 运行时换掉 `GPUIApplicationDelegate` 的方法 | 不推荐：改第三方类、依赖其类名，(a) 是 Apple 文档化的等价做法 |

(a) 的风险与注意：
- **时机**：必须在 AppKit 装好默认 handler 之后。GPUI 的 `app.run(|cx| …)` 闭包在 `applicationDidFinishLaunching:` 里执行
  （`platform.rs:1305-1345` 的 `finish_launching` 回调），晚于 AppKit 安装默认 handler（`applicationWillFinishLaunching:` 之前），可以在闭包里装。
  GPUI / tray-icon / muda / auto-launch 都没有注册 Apple Event handler（registry 源码 grep `AppleEvent` / `setEventHandler` 无结果），不会互相覆盖。
- **失去 AppKit 默认行为**：AppKit 的 reopen 处理在"没有可见窗口"时会还原一个最小化窗口（Chromium 注释"This callback runs before AppKit picks its
  own window to deminiaturize"）。我们的 handler 调 `bring_main_to_front`（`restore_if_minimized` 会 `deminiaturize`），主窗口场景已覆盖。
  隐藏中的应用被点 Dock 时的"取消隐藏"由激活完成，不在 reopen handler 里（**未验证**，CI 可测：§8 F-hidden-reopen）。
- **handler 生命周期**：`NSAppleEventManager` 是否 retain handler 文档没写，草图里 `mem::forget` 一份常驻。
- **线程 / 借用**：Apple Event 在主线程 run loop 里分发，不在 GPUI 的 App 借用期间；但 handler 拿不到 `&mut App`，用现有托盘的模式：
  handler 往 `async_channel` 发消息，前台任务收到后 `cx.update(window_mode::bring_main_to_front)`（`services/tray.rs:258-306` 同款）。
- **语义变化**：现在只有"没有可见窗口"才唤起；(a) 之后每次点 Dock / 在访达里再次打开都 `bring_main_to_front`（主窗口可见时就是激活 +
  成为 key，和普通 mac 应用一致；贴边隐藏中的主窗口也会被拉回，与 `on_main_activated` 的处理一致）。M08 用例语义不变。

### 5.3 (a) 的 objc2 0.6 草图（已类型检查）

```rust
// platform/macos/ns.rs（或单独的 reopen.rs；不依赖 GPUI，可进 mac-check）
use objc2::rc::Retained;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_foundation::{NSAppleEventDescriptor, NSAppleEventManager, NSObject, NSObjectProtocol};

const K_CORE_EVENT_CLASS: u32 = u32::from_be_bytes(*b"aevt"); // kCoreEventClass
const K_AE_REOPEN_APPLICATION: u32 = u32::from_be_bytes(*b"rapp"); // kAEReopenApplication

pub struct ReopenIvars {
    callback: Box<dyn Fn()>,
}

define_class!(
    // SAFETY: NSObject 没有子类化要求；没有实现 Drop
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "MiniTodoReopenHandler"]
    #[ivars = ReopenIvars]
    pub struct ReopenHandler;

    impl ReopenHandler {
        #[unsafe(method(handleReopen:withReplyEvent:))]
        fn handle_reopen(&self, _event: &NSAppleEventDescriptor, _reply: &NSAppleEventDescriptor) {
            (self.ivars().callback)();
        }
    }

    unsafe impl NSObjectProtocol for ReopenHandler {}
);

impl ReopenHandler {
    fn new(mtm: MainThreadMarker, callback: Box<dyn Fn()>) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(ReopenIvars { callback });
        unsafe { msg_send![super(this), init] }
    }
}

/// 替换 AppKit 的 reopen（'aevt'/'rapp'）处理：之后 GPUI 的 on_reopen 不再被调用。
/// 必须在 applicationWillFinishLaunching 之后调用（GPUI 的 run 闭包里即可），只能在主线程。
pub fn install_reopen_handler(callback: Box<dyn Fn()>) -> bool {
    let Some(mtm) = MainThreadMarker::new() else {
        return false;
    };
    let handler = ReopenHandler::new(mtm, callback);
    let manager = NSAppleEventManager::sharedAppleEventManager();
    // 不用生成的 setEventHandler_andSelector_forEventClass_andEventID：它被 objc2-foundation 的
    // `objc2-core-services` 特性门控，会引入新依赖 objc2-core-services（不在 Cargo.lock 里）。
    // AEEventClass / AEEventID = FourCharCode = UInt32，编码 'I'，与 u32 一致（debug 构建下 objc2 会校验编码）。
    unsafe {
        let _: () = msg_send![
            &*manager,
            setEventHandler: &*handler,
            andSelector: sel!(handleReopen:withReplyEvent:),
            forEventClass: K_CORE_EVENT_CLASS,
            andEventID: K_AE_REOPEN_APPLICATION
        ];
    }
    std::mem::forget(handler); // 是否被 retain 未写明：常驻
    true
}
```

接到 GPUI（`lib.rs` 的 `app.run` 闭包里，`#[cfg(target_os = "macos")]`）：

```rust
let (tx, rx) = async_channel::unbounded::<()>();
platform::macos::ns::install_reopen_handler(Box::new(move || {
    let _ = tx.try_send(());
}));
cx.spawn(async move |cx| {
    while rx.recv().await.is_ok() {
        cx.update(|cx| services::mac_menu::on_reopen(cx));
    }
})
.detach();
```

cargo 特性（app 与 `scripts/mac-check/Cargo.toml` 都要加）：objc2-foundation `NSAppleEventManager`、`NSAppleEventDescriptor`
（`NSObject` 已被 objc2-app-kit 的特性间接打开）。`define_class!` / `MainThreadOnly` / `msg_send!` / `sel!` 在 objc2 0.6.5 默认可用
（`src/macros/define_class.rs:1-446` 的规范：`#[unsafe(super(..))]`、`#[thread_kind = MainThreadOnly]`、`#[name]`、`#[ivars]`、
`#[unsafe(method(..))]`；`MainThreadOnly::alloc(mtm)` 在 `top_level_traits.rs:547`，`set_ivars` 在 `rc/allocated_partial_init.rs:200`）。
`#[name]` 全进程唯一，ivar 里的 `Box<dyn Fn()>` 不是 Send，配 `MainThreadOnly` 没问题。

## 6. ⌘`、Mission Control 缩略图、App Exposé 及其它

| 目标 | 手段 | 把握 |
|---|---|---|
| 不进 ⌘` 循环 | `IgnoresCycle` | Apple 文档 |
| Mission Control 里不当作窗口缩略图、不移动 | `Stationary` | Apple 文档（"like the desktop window"） |
| App Exposé 不显示 | `Stationary`（另外非 Normal 层级默认 Transient 就是"在 Exposé 中隐藏"，但 Transient 会让它在 Mission Control / 显示桌面时消失，不要） | **未验证** |
| 「窗口」菜单 / Dock 右键窗口列表不列出 | `setExcludedFromWindowsMenu(true)`；NSPanel 本来也不列 | Dock 列表部分 **未验证** |
| 不进任务切换（⌘Tab） | ⌘Tab 是应用级的，与窗口无关（与主窗口固定模式的结论相同） | — |
| 窗口还原 / 快照 | 可选 `setRestorable(false)`（Plash、Übersicht 都设）；GPUI 窗口没有 restorationClass，一般不会被还原 | 可选 |
| 第三方窗口管理器不平铺 | 非激活面板 + 非 0 层级 | **未验证** |
| ⌘W / ⌘M | `mac_menu::close_window` 已跳过收纳箱；Minimize 建议同样跳过 | 代码层面 |
| 启动时不闪 | `show: false` 建窗 → 设属性 → `orderFrontRegardless` | GPUI `1291-1295` |

## 7. 建议的 `ns.rs` 新增 API（风格同现有文件：按窗口号查找、主线程、返回 bool）

```rust
use objc2_app_kit::{NSPanel, NSWindowAnimationBehavior, NSWindowCollectionBehavior, NSWindowLevel};
use objc2_core_graphics::{CGWindowLevelForKey, CGWindowLevelKey};

/// 收纳箱层级：桌面图标层之上一层（= -2147483602）
pub fn fence_level() -> NSWindowLevel {
    CGWindowLevelForKey(CGWindowLevelKey::DesktopIconWindowLevelKey) as NSWindowLevel + 1
}

/// 收纳箱建窗后（窗口仍隐藏）调用一次：桌面层 + 所有空间 + 不受 Mission Control / ⌘H 影响，然后显示。
/// 不触发 GPUI 回调（orderFrontRegardless 会触发遮挡状态回调，GPUI 只是开始绘制）
pub fn make_desktop_layer(number: isize) -> bool {
    with_window(number, |window| {
        window.setLevel(fence_level());
        window.setCollectionBehavior(
            NSWindowCollectionBehavior::CanJoinAllSpaces
                | NSWindowCollectionBehavior::Stationary
                | NSWindowCollectionBehavior::IgnoresCycle
                | NSWindowCollectionBehavior::FullScreenNone,
        );
        window.setCanHide(false);
        window.setHidesOnDeactivate(false);
        window.setExcludedFromWindowsMenu(true);
        window.setAnimationBehavior(NSWindowAnimationBehavior::None);
        if let Some(panel) = window.downcast_ref::<NSPanel>() {
            // 不要调 setFloatingPanel：它会改层级
            panel.setBecomesKeyOnlyIfNeeded(false);
        }
        window.orderFrontRegardless();
    })
    .is_some()
}

/// e2e 自检用：把实际生效的属性写进日志（CGWindowList 只给得出层级）
pub fn describe_window(number: isize) -> Option<String> {
    with_window(number, |w| {
        format!(
            "level={} behavior={:#x} canHide={} excludedFromWindowsMenu={} hidesOnDeactivate={} \
             nonactivating={} panel={}",
            w.level(),
            w.collectionBehavior().0,
            w.canHide(),
            w.isExcludedFromWindowsMenu(),
            w.hidesOnDeactivate(),
            w.styleMask().contains(objc2_app_kit::NSWindowStyleMask::NonactivatingPanel),
            w.downcast_ref::<NSPanel>().is_some(),
        )
    })
}
```

- 以上 `setLevel` / `setCollectionBehavior` / `setCanHide` / `setHidesOnDeactivate` / `setExcludedFromWindowsMenu` / `setAnimationBehavior` /
  `orderFrontRegardless` / `makeKeyWindow` 在 objc2-app-kit 0.3.2 `NSWindow.rs`（`751-756`、`1068-1084`、`1113`、`1174`、`1322-1327`、`1441-1458`），
  `setBecomesKeyOnlyIfNeeded` / `setWorksWhenModal` 在 `NSPanel.rs:97-111`；类型检查通过（草图用的是同名调用）。
- 新增 cargo 特性：objc2-app-kit `NSPanel`；新依赖 `objc2-core-graphics = { version = "0.3.2", default-features = false, features = ["CGWindowLevel"] }`
  （已在 lock 里，或干脆写常量 `i32::MIN + 45 + 1` 不加依赖）；做 §5.3 时再加 objc2-foundation `NSAppleEventManager` / `NSAppleEventDescriptor`。
  `scripts/mac-check/Cargo.toml` 逐项同步（它的注释要求与 app 一致）。
- 建窗顺序（mac）：`spec.kind = GpuiWindowKind::PopUp`（`cfg(target_os = "macos")`）、`spec.show = false`、`spec.focus = false` →
  `windows::open_window`（`prepare_after_open` 隐藏红绿灯、关 hidesOnDeactivate；`set_client_rect` 摆位）→ 同一前台任务队列里排在它后面的
  `embed_after_open` 的 mac 分支调 `make_desktop_layer`（取代 Windows 的"嵌入桌面后再显示"）。
- 改名：`start_rename` 里（mac）先 `window.activate_window()` 再 focus 输入框。
- 不要对收纳箱调 `set_topmost` / `bring_to_front` / `cx.activate(true)`（后者会激活应用，失去非激活的意义）。
- 收纳箱是否允许"点一下就把它在同层里提到最前"（多个收纳箱重叠时）：非激活面板单击不会 orderFront，若需要可在 mouse down 时
  `order_front`（`orderFrontRegardless`，仍在同层，不激活）。

## 8. CI（GitHub macOS runner，`pc-gpui-e2e-macos.yml`）能验证什么

现有工具：`maclib.py` 的 Quartz 窗口枚举、CGEvent 键鼠、AX、OCR、`osascript`。**注意**：`maclib.list_windows` 用了
`kCGWindowListExcludeDesktopElements`（`maclib.py:137`），Apple 对它的定义是"elements of the desktop, including the background picture and
desktop icons"，按什么判定没有写——桌面层的收纳箱窗口**可能被它过滤掉**（**未验证**），收纳箱用例要另写一个只用
`kCGWindowListOptionOnScreenOnly` 的枚举；`windows_of(pid, layer=0)` 默认只要 0 层，也要传 `layer=None`。

| 用例（建议） | 断言 | 可行性 |
|---|---|---|
| F-level | 收纳箱窗口 `kCGWindowLayer == CGWindowLevelForKey(kCGDesktopIconWindowLevelKey) + 1`（pyobjc `Quartz.CGWindowLevelForKey`） | 可靠 |
| F-order | `OnScreenOnly` 列表是前→后顺序：`open <临时目录>` 打开一个 Finder 窗口与收纳箱重叠，收纳箱排在所有 0 层窗口之后、Finder 桌面图标窗口（若有）之前 | 可靠（Finder 桌面窗口在 runner 上是否存在只打印） |
| F-props | 应用 debug 日志里 `describe_window` 的输出：behavior 位、canHide=false、excludedFromWindowsMenu=true、hidesOnDeactivate=false、panel=true、nonactivating=true；隔 5 秒再打一次确认没被改回 | 可靠（应用自报，证明"设了且没被重设"，不证明系统行为） |
| F-hide | System Events `set visible of process … to false`（等价 ⌘H）→ 主窗口不在屏幕上、收纳箱仍在；再用 pyobjc `NSRunningApplication.runningApplicationWithProcessIdentifier_(pid).hide()`（等价别的应用「隐藏其他」）重复一次 | 可靠 |
| F-reopen | 收纳箱可见 → ⌘W 关主窗口 → `open <app>` → 主窗口重新出现、没有第二个进程（M08 的变体）。**直接回答"收纳箱是否计入 hasVisibleWindows"** | 可靠（`open` 走 LaunchServices reopen，与点 Dock 同一事件；真点 Dock 不做） |
| F-hidden-reopen | 隐藏应用 → `open <app>` → 主窗口与收纳箱都可见（若采用 §5.3 handler，验证"取消隐藏不依赖 AppKit reopen 处理"） | 可靠 |
| F-nonactivating | `activate_other("Finder")` → CGEvent 单击收纳箱空白处 → `frontmost_pid()` 仍是 Finder | 可靠（PopUp 方案的核心断言） |
| F-rename | Finder 在前台 → 双击收纳箱标题 → `type_text`（CGEvent HID + Unicode）→ 回车 → 库里 `fences.title` 与收纳文件夹名变了；前台仍是 Finder | 可靠（验证非激活 key 面板收键盘）；输入法候选窗验证不了 |
| F-move / F-resize | CGEvent 拖标题栏 / 拖缩放手柄 → CGWindowList 几何变化、库里几何落库 | 可靠（M10 已有拖窗口的先例） |
| F-scroll | CGEvent 滚轮（带 ⌃ 或 ⌘ 标志）→ 库里图标大小变化 | 较可靠 |
| F-open | 收纳箱里放一个 `.txt`，双击 → 出现 TextEdit 进程 | 较可靠（TextEdit 首次启动可能弹 iCloud 面板，只看进程） |
| F-drop | `open <临时目录>` 的 Finder 窗口里用 CGEvent 把文件拖到收纳箱 → 文件进入收纳文件夹 | 可尝试、易抖（Finder 图标位置要靠 AX / 列表视图定位）；**不要用 ~/Desktop**（TCC 弹框 runner 上没人点） |
| F-cycle | 主窗口 + 设置窗口打开时发 ⌘`，AX `AXFocusedWindow` 从不是收纳箱 | 可做，价值一般，信息性 |
| F-showdesktop | 触发显示桌面（CGEvent F11 = keycode 103，或 `Mission Control.app/Contents/MacOS/Mission Control 1`——两种触发方式都 **未验证**）→ Finder 窗口被推开、收纳箱几何不变且仍在屏幕上 | 信息性（runner 的快捷键 / 动画不保证） |
| Spaces / Mission Control 缩略图 / App Exposé / Stage Manager / 点按墙纸显示桌面 / Sonoma 小组件 / Sequoia 平铺 / Retina / 多屏 / 光标形状 / 中文输入法 | — | **不能在 runner 上可靠断言**：没有创建 Space 的公开 API，Stage Manager 开关要改 `com.apple.WindowManager` 并重启 WindowManager，runner 单屏 1x。靠用户反馈 |

## 9. 未验证清单（按风险排序）

1. 非激活 key 面板里中文输入法是否正常（改名）。
2. 单击 GPUIView 是否真的不会让非激活面板成为 key（若会，也无害；只是 `activate_window()` 变成冗余）。
3. NSPanel 不计入 `hasVisibleWindows` 在 macOS 15 上是否仍成立（Apple 文档如此；F-reopen 直接验证）。
4. 别的应用「隐藏其他」是否尊重 `canHide = NO`（F-hide 的第二步验证）。
5. Stage Manager、点按墙纸显示桌面、显示桌面模式下点收纳箱的行为。
6. `kCGWindowListExcludeDesktopElements` 是否过滤 desktopIcon+1 层的窗口（e2e 换枚举方式即可绕开）。
7. Finder 拖放：GPUI 返回 Copy 时 Finder 不动原文件（只影响光标"+"号）。
8. 应用不在前台时缩放手柄的光标形状。
9. 透明圆角处的点击是否穿透到墙纸。

## 来源

Apple：
- [NSWindow.CollectionBehavior](https://developer.apple.com/documentation/appkit/nswindow/collectionbehavior-swift.struct)、
  [auxiliary](https://developer.apple.com/documentation/appkit/nswindow/collectionbehavior-swift.struct/auxiliary)、
  [canJoinAllApplications](https://developer.apple.com/documentation/appkit/nswindow/collectionbehavior-swift.struct/canjoinallapplications)
- [Setting Window Collection Behavior（归档）](https://developer.apple.com/library/archive/documentation/Cocoa/Conceptual/WinPanel/Articles/SettingWindowCollectionBehavior.html)
- [applicationShouldHandleReopen(_:hasVisibleWindows:)](https://developer.apple.com/documentation/appkit/nsapplicationdelegate/applicationshouldhandlereopen(_:hasvisiblewindows:))
- [How Cocoa Applications Handle Apple Events（归档）](https://developer.apple.com/library/content/documentation/Cocoa/Conceptual/ScriptableCocoaApplications/SApps_handle_AEs/SAppsHandleAEs.html)
- [NSWindow.canHide](https://developer.apple.com/documentation/appkit/nswindow/canhide)、
  [NSApplication.orderedWindows](https://developer.apple.com/documentation/appkit/nsapplication/orderedwindows)、
  [activateAllWindows](https://developer.apple.com/documentation/appkit/nsapplication/activationoptions/activateallwindows)
- [NSPanel.becomesKeyOnlyIfNeeded](https://developer.apple.com/documentation/appkit/nspanel/becomeskeyonlyifneeded)、
  [NSNonactivatingPanelMask](https://developer.apple.com/documentation/appkit/nsnonactivatingpanelmask)、
  [nonactivatingPanel](https://developer.apple.com/documentation/appkit/nswindow/stylemask-swift.struct/nonactivatingpanel)
- [kCGWindowListExcludeDesktopElements](https://developer.apple.com/documentation/coregraphics/cgwindowlistoption/excludedesktopelements)
- [Developer Forums 724143：NSApplicationActivateAllWindows does not work](https://developer.apple.com/forums/thread/724143)

开源实现：
- Plash：`https://github.com/sindresorhus/Plash`（当前已闭源，源码取自 `refs/pull/185/head` 88ecaab，`Plash/DesktopWindow.swift`、`AppState.swift`、`App.swift`、`Utilities.swift`）
- Übersicht：[`Uebersicht/UBWindow.m`](https://github.com/felixhageloh/uebersicht/blob/master/Uebersicht/UBWindow.m)、`UBApplication.m`、`UBWindowGroup.m`
- Hammerspoon：[`extensions/canvas/libcanvas.m`](https://github.com/Hammerspoon/hammerspoon/blob/master/extensions/canvas/libcanvas.m)、`Canvas.h`
- Electron：[`shell/browser/native_window_mac.mm`](https://github.com/electron/electron/blob/main/shell/browser/native_window_mac.mm)、
  [`docs/api/structures/base-window-options.md`](https://github.com/electron/electron/blob/main/docs/api/structures/base-window-options.md)
- Chromium：[`chrome/browser/app_controller_mac.mm`](https://github.com/chromium/chromium/blob/main/chrome/browser/app_controller_mac.mm)（`applicationShouldHandleReopen` 1853 起）
- cocoa-dev 2004-03「Clicking Dock Icon creates a new window」（lists.apple.com/archives/cocoa-dev/2004/Mar/msg00967.html；本次只拿到搜索摘要，原帖未能直接打开）
- Cindori「Make a floating panel in SwiftUI for macOS」（cindori.com/developer/floating-panel，非激活面板 + `canBecomeKey` 的常见写法；只看了搜索摘要）

本机源码：
- `gpui-pre-macos-0.3.9/src/window.rs`：96-105（常量）、130（GPUIPanel）、284-286 / 3551-3556（acceptsFirstMouse）、429-510（窗口类方法）、
  987-993（tabbing）、995-1028（styleMask / 类）、1076-1085（拖放注册）、1171（setMovable）、1210-1259（按类型设层级 / 行为 / 跟踪区）、
  1291-1295（显示）、1791-1809（activate）、2220-2231（start_window_move）、2499-2517（跟踪区）、2789-2960（鼠标事件）、
  3148-3172（key 状态与假 didBecomeKey）、3600-3650（拖放回调）
- `gpui-pre-macos-0.3.9/src/platform.rs`：88-172（delegate 方法注册）、290-316（菜单，`"Window"` 才设 windowsMenu）、661-727（activate / hide）、
  1305-1345（did_finish_launching）、1369-1379（should_handle_reopen）
- `gpui-pre-0.3.9/src/app.rs:302-313`（on_reopen）、`src/platform.rs:2703-2731`（WindowKind）
- `objc2-app-kit-0.3.2/src/generated/NSWindow.rs`、`NSPanel.rs`；`objc2-foundation-0.3.2/src/generated/NSAppleEventManager.rs:55-100`（生成的
  `setEventHandler_…` 被 `objc2-core-services` 特性门控）；`objc2-core-graphics-0.3.2/src/generated/CGWindowLevel.rs`、`src/lib.rs:40-60`；
  `objc2-0.6.5/src/macros/define_class.rs`
- 本仓库：`pc-gpui/crates/app/src/platform/macos/{ns,mod}.rs`、`services/fences.rs:131-220`、`services/mac_menu.rs:56, 103-137`、
  `windows/mod.rs:160-296`、`windows/fence/mod.rs:669-682`、`lib.rs:52-57`、`scripts/e2e-macos/maclib.py:133-160`
