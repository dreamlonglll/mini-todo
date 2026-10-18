# Desktop Window E2E Thinking Guide

> **Purpose**: Before claiming a main-window mode / style / Z-order change — or any desktop-fence
> window change — "works", run the checks below. Unit tests cannot see Win32 state; the only proof
> is the live window.
>
> 自动化套件：`pc-gpui/scripts/e2e-windows/`（真实 GPUI 窗口 + 真实鼠标键盘输入 + Win32 只读断言 + 截图取色 + 读 `data.db`；
> 用例表与本地运行方法见其 README）。CI：`.github/workflows/pc-gpui-e2e-windows.yml`，在一次性的 GitHub 托管 Windows 虚拟机上跑
> （门槛见 `.trellis/spec/pc-gpui/verification.md` §6）。GPUI 窗口没有 DOM / CDP：按钮位置按布局常量（逻辑像素）点击，会随内容移动的
> 目标先截图按颜色识别再点。旧 Tauri 版的 CDP / `e2e.ps1` / `fence.ps1` 工具已随旧版删除。
>
> macOS：`pc-gpui/scripts/e2e-macos/`（CI `.github/workflows/pc-gpui-e2e-macos.yml`，门槛见 `verification.md` §7），做法见文末「macOS runner」。

---

## When to Use

- [ ] 改了主窗口模式：`pc-gpui/crates/app/src/services/window_mode.rs`（`plan_mode` / `apply_mode` / 200ms 轮询 / 贴边隐藏）
- [ ] 改了 `platform/embed.rs`、`platform/subclass.rs`（`FLAG_BOTTOM` / `FLAG_LOCK_GEOMETRY`）、`platform/windows.rs` 的样式 / owner / Z 序 / 几何
- [ ] 改了启动时 `screen_configs` / `is_fixed` / `fixed_embed_desktop` 的恢复
- [ ] 改了收纳箱窗口：`services/fences.rs` 的建窗参数与嵌入守护、`windows/fence/` 的移动 / 缩放 / 折叠 / 外部拖入、`fence_layouts` 的写入时机
- [ ] 改了会移动用户文件的收纳箱路径（core `services/fence_fs`、`fence::items::{move_in, restore, move_to}`、`fence::delete`）
- [ ] 改了通知窗口定位、托盘、单实例、开机自启、Toast、DPAPI，或 release profile / `build.rs`
- [ ] 改了 macOS 的窗口平台层（`platform/macos/`、`mac_geometry.rs`）、建窗 / 红绿灯（`windows/mod.rs`）、应用菜单（`services/mac_menu.rs`）、
      菜单栏图标 / 单实例 socket / LaunchAgent / 系统通知 / 日志的 mac 分支、`scripts/macos/bundle.sh`
- [ ] 改了 macOS 的收纳箱：桌面层（`ns::make_desktop_layer`、`services/fences.rs` 的 mac 分支）、取色面板、core `services/fence_fs/mac.rs`

---

## 开发版与安装版并存（在自己的机器上跑之前）

1. **能用 CI 就用 CI**（`workflow_dispatch` 触发 `pc-gpui-e2e-windows.yml`）：套件会最小化其它窗口、重启 Explorer、改开机自启注册表项、
   移动测试文件，在一次性虚拟机上跑最安全。
2. **必须带隔离变量启动开发版**：`MINI_TODO_DATA_DIR` + `MINI_TODO_FENCES_ROOT`（测收纳箱屏幕组合时再加
   `MINI_TODO_SCREEN_ID_FILE`），都指向会话临时目录（`run_e2e.py` 的隔离用例自动设置）。不带就不要启动——开发版会打开真实库、
   被单实例转交给安装版。变量含义见 `.trellis/spec/backend/desktop-fences.md` §3.6。
3. **按 PID / exe 路径（`pc-gpui\target\debug\mini-todo.exe`）定位开发版**的进程与窗口；绝不操作安装版的进程、窗口、托盘图标
   （开发版托盘提示为"Mini Todo（开发版）"）；不切换托盘「开机自启动」（注册表项 `HKCU\…\Run\mini-todo` 与安装版共用；
   套件 I06 会写入再删除它，已开启开机自启的机器跑完要重新打开）。
4. **不隔离的 P 组**（真实数据目录、日志位置、单实例唤起）只在一次性虚拟机 / 没装正式版的机器上加 `--allow-real-profile` 跑。
5. **Explorer 重启**（S06 / F06 会 `taskkill explorer.exe` 再拉起）每次先征得用户授权，结束后确认 Explorer 已恢复。
6. 结束前清理测试数据目录、测试收纳根目录和桌面上的测试文件。
7. **安装版的收纳箱会和测试收纳箱重叠**：F 组新建的收纳箱在光标所在显示器右上角找空位，测试库里没有用户的收纳箱，常常正好建在
   用户收纳箱的位置。两者都在桌面层，外部拖入会落进**安装版**的收纳箱，把测试文件移进用户真实的收纳文件夹（2026-10-16 实测：
   F04 / F08 / F09 / F11 拖入"失败"，文件其实进了 `%USERPROFILE%\MiniTodo收纳箱\`；F05 的 Win+D 命中了安装版收纳箱）。
   跑 F 组前先临时隐藏安装版的收纳箱窗口（按安装版 PID 找 `Zed::Window` / `收纳箱`，`ShowWindow(SW_HIDE)`，跑完 `SW_SHOWNOACTIVATE`；
   隐藏期间稳妥起见跳过会重启 Explorer 的 F06——安装版守护重挂时会不会把它重新显示没验证过），或请用户把它挪开；跑完检查真实收纳根目录里有没有 `e2e-` 前缀的文件。

## 输入安全

- 套件用**真实输入**（`SetCursorPos` + `SendInput`、真实 OLE 拖放源）。在用户机器上跑 = 接管用户的鼠标键盘：只在用户离开、会话解锁、
  前台不是管理员权限程序时跑；中文输入法切到英文；测试期间不动鼠标键盘。
- 教训（旧版）：一次拖动源脚本的辅助线程没做 per-monitor DPI 感知，`SetCursorPos(-2300,1200)` 被虚拟化到 (0,1500)，鼠标按下落到了
  用户最大化的窗口上。每个发输入的线程先设 per-monitor DPI 感知，并先核对目标坐标命中的是开发版窗口；Win+D / Win+M 之后恢复用户的窗口。
- 不要用 `SendKeys`（走输入法，文字会被转换）。

---

## Checklist

1. **Assert the Win32 state, not the UI** — owner、ex style（TOOLWINDOW / APPWINDOW / TOPMOST / NOACTIVATE）、`WS_MINIMIZEBOX`、
   `WS_SYSMENU`（所有窗口都没有，`DWMWA_CAPTION_BUTTON_BOUNDS` 宽度为 0）、Z 序、客户区几何、`WM_NCHITTEST` 结果；
   逐字段对照 `.trellis/spec/backend/window-modes.md` §3 的模式表与 `desktop-fences.md` §3.1 的状态表。
2. **Show Desktop**（Win+D）**/ Minimize All**（Win+M）：嵌入的窗口从不最小化、不隐藏、几何不变；经典固定被最小化后一个 tick 内还原。
3. **Something must be able to cover it** — 普通顶层窗口（`probe-window.ps1` 探针）盖在上面时必须赢 `WindowFromPoint`。
4. **Real input** — 编辑类窗口从主窗口打开后第一次点击就拿到前台并接受按键；固定模式下拖标题栏 / 边框不动。
5. **Explorer restart** — 窗口存活、重挂到新 Progman、Win+D 仍免疫。
6. **Process restart** — 按 PID 结束再启动，状态与几何从 `screen_configs` / `fences` / `fence_layouts` 恢复。
7. **Loop the transitions** several times — 同一 HWND、进程存活、每一步之后几何不变。
8. **Put the user's mode back** before you leave (they run fixed mode daily) and delete test todos through the app, not raw SQL.

## 收纳箱专属检查项

细节与断言见 `.trellis/spec/backend/desktop-fences.md` §3.1 与 §6。

1. 每个收纳箱 owner = 桌面宿主 / TOOLWINDOW / 无 APPWINDOW / 无 MINIMIZEBOX / 无 TOPMOST / 无 NOACTIVATE，且新建、折叠、锁定、改名、
   改外观、显示隐藏、重置位置之后都不变；新建与「显示/隐藏所有收纳箱」时任务栏不出现按钮。
2. Win+D：收纳箱从不最小化；普通应用窗口能盖住它。
3. Explorer 重启：全部收纳箱第一次守护 tick 内重挂到新 Progman。
4. 拖动 / 缩放结束后 `fences` 与当前组合 `fence_layouts` 同时落库，且可见区域 = 库里的几何；启动夹回、屏幕组合切换、程序挪窗**不**落库。
   吸附（e2e F12–F15）：落在阈值内吸到目标的边、拖动中**目标**收纳箱的对应边截图有 `#409EFF`、松手后消失；按住 Alt 不吸。手势要够长
   （GPUI 的拖动移动超过 2px 才开始、窗口超过 3px 才动：只想挪 3px 时顺带横向挪一段），主窗口先最小化（它盖在桌面层的收纳箱之上）。
5. 混合 DPI：折叠高度按目标显示器缩放（150% → 48px，125% → 40px）——CI 虚拟机是单屏，需用户实机验收。
6. 外部拖入：新实例第一次拖入、改名回车后 / 单击标题后紧接着的第一次拖入都要生效（GPUI 输入模态问题，e2e F04 / F10 / F11）。
7. 文件层只在测试收纳根目录与桌面上 `__fence_e2e_` 前缀的测试文件上操作；绝不移动、改名、删除用户真实的桌面文件。
8. macOS（runner，MF 组）：上面 1–3 是 Windows 的状态；mac 上断言的是层级（`kCGWindowLayer` = 桌面图标层 + 1、排在所有 layer ≥ 0 的窗口之后）、
   应用日志自报的 collectionBehavior / `canHide` / 非激活面板，以及行为（⌘H 后仍在、点它前台应用不变、有它时 Dock 唤起主窗口）。
   文件层一律在临时目录（`MINI_TODO_FENCES_ROOT` + `MINI_TODO_DESKTOP_DIR` + 临时来源目录），**绝不碰** `~/Desktop` / `~/Documents` / `~/Downloads`。

---

## Gotchas Seen

- 前台是管理员权限程序（部分游戏客户端）或屏幕已锁定时，模拟输入与 Shell `ToggleDesktop()` 都会被丢弃。
- PowerShell parses `0xFFFFFFFF` as `-1`; wheel deltas need the explicit `+ 2^32` wrap.
- The window rect can move a few pixels while you are paused and the user pokes at it — snapshot the rect at the start of a loop, not from memory.
- GPUI 窗口在左 / 右 / 下有不可见边框（96 DPI 下各 8px）：断言几何时比较**客户区**，不要比较 `GetWindowRect`。
- 无 GPU 的 CI 虚拟机上 GPUI 走 D3D11 WARP 软件渲染，截图取色依赖桌面是纯色（`set-display.ps1`）。

---

## macOS runner（`pc-gpui/scripts/e2e-macos/`）

维护者没有 Mac，mac 的"live window"只有 GitHub 托管的 macOS runner（`macos-15`，arm64）。做法与 Windows 套件同一思路（真实应用 + 真实输入 +
只读断言 + 截图 + 读 `data.db`），工具换成 Quartz / Vision / AX（pyobjc）：

- **环境事实**：runner 自动登录图形会话、不休眠、纯黑壁纸；镜像的 TCC 给 `/bin/bash`、`osascript` 与 runner agent（负责进程）预授权了
  屏幕录制、辅助功能、AppleEvents（System Events / 访达）与 PostEvent，并压掉了 macOS 15+ 的"绕过私有窗口选择器"提醒。GPU 是
  Apple Paravirtual device（Metal 可用；Intel runner 上 Metal 多重采样会卡死，别在 Intel runner 上跑 GUI e2e）。屏幕默认 **1024×768pt、1x**：
  比编辑窗口（1080 宽）还窄，AppKit 会把窗口夹到屏幕内——尺寸断言按"期望与屏幕可见区域取小"，开跑前 `set_display.py` 尽力调到 1920×1080。
  通知中心 UI 被卸载：系统通知横幅与授权弹框测不了。
- **启动**：先 `scripts/macos/bundle.sh <二进制> - <目录>` 包成 `.app`（没有 bundle id 时系统通知被 GPUI 禁用），再**直接执行**
  `<app>/Contents/MacOS/mini-todo`——环境变量（`MINI_TODO_DATA_DIR` / `MINI_TODO_DEBUG_OPEN`，只在 debug 构建生效）能传进去，
  `NSBundle.mainBundle` 仍是这个 `.app`。要测 `open <app>`（Dock / LaunchServices 唤起）先 `lsregister -f` 登记。
- **窗口**：`CGWindowListCopyWindowInfo` 按 `kCGWindowOwnerPID` 过滤，读 `kCGWindowBounds`（全局点、主屏左上原点，与 CGEvent 坐标同一套）、
  `kCGWindowNumber`（= NSWindow `windowNumber` = 本应用的 `platform::Hwnd`）、`kCGWindowLayer`（置顶判定）。
- **截图**：`screencapture -x -o -l <windowNumber>` 截单个窗口（被遮挡也能截）。**没有屏幕录制权限时它照样输出 PNG，只是只有壁纸**
  （runner-images#8782）：一定要断言"截图确有内容"（不透明像素、颜色数、非主色像素占比），界面上有没有字再用 OCR 断言——mini-term 的 mac 包曾连续
  6 个版本一个字都不画没人发现（GPUI 漏开 `font-kit`）。
- **OCR**：Vision `VNRecognizeTextRequest`（accurate、`zh-Hans` + `en-US`），小图先放大；去空白、统一全角标点后子串 / 最长公共子串匹配。
  数据异步加载，不满足时隔 2 秒重截重试。mac 字体与 Linux 不同，不能拿 `regress.py` 的截图当基线。
- **菜单**：AX 读应用菜单（`menu bar 1`，`AXMenuBar`）与菜单栏状态项（`AXExtrasMenuBar` 的第一个子元素）。对状态项 `AXPress` 时 AppKit 弹出菜单、
  弹菜单期间应用不回 AX 请求——超时（-25204）是正常的，读不到菜单再真实右键点它（tray-icon 只在右键按下时弹菜单）；菜单项用 `AXPress`。
- **键鼠**：`CGEventPost(kCGHIDEventTap)` 等同真实输入，发给光标下 / 前台的应用：**发键盘事件前先切到前台并确认前台进程是被测应用**
  （AX `AXFocusedApplication`），切不到时退到 `CGEventPostToPid`（直接投给该进程，不会误按到别的程序）并记 BAD。快捷键用 keycode +
  `kCGEventFlagMaskCommand`（⌘Enter = 36）。从 runner 的 shell 拉起的进程不一定会自己激活：`NSRunningApplication.activateWithOptions` →
  System Events `set frontmost` → 点窗口顶部空白处，依次尝试。
- **文字输入用粘贴**：runner 上经 HID 事件流（`CGEventPost(kCGHIDEventTap)`）发的 `CGEventKeyboardSetUnicodeString` 逐字注入（虚拟键码 0，
  `maclib.type_text`）**进不了 GPUI 的文字输入框**——普通窗口（M09 新建待办）与收纳箱（MF08 改名）都一样，窗口明明是 key、应用在前台；
  同样的事件用 `CGEventPostToPid` 直接投给进程却能进。所以文字一律 `pbcopy` + ⌘A + ⌘V（键盘事件仍走 HID），逐字注入只作首选尝试
  （OCR 认不出就改粘贴）；`CGEventPostToPid` 只用来诊断（它绕过前台应用 / key 窗口的路由，证明不了真实键盘到得了窗口）。不要把
  "逐字注入没进去"当成应用的焦点问题去改应用（MF08 就这样误判过一轮）。
- **拖窗口**：左键在标题栏上沿（y + 6pt，按钮之上的留白）按下 → 分步拖动 → 抬起，按 CGWindowList 前后的位置判定；AX 设置窗口位置 / 尺寸
  （`AXPosition` / `AXSize`）用来判定固定模式下"改尺寸无效"。
- **断言数据**：debug 套件读 `<MINI_TODO_DATA_DIR>/data.db`；release 套件不隔离，读 `~/Library/Application Support/mini-todo/data.db`、
  `~/Library/Logs/com.tauri-app.mini-todo/mini-todo.log`、`/tmp/com_tauri_app_mini_todo_si.sock`、`~/Library/LaunchAgents/mini-todo.plist`
  ——**只在一次性 VM 上跑**（`--allow-real-profile`）。
- **在自己的 Mac 上跑**（目前没有人这么做，留给有 Mac 的贡献者）：终端要有「屏幕录制」与「辅助功能」权限；smoke 组会接管鼠标键盘；
  不要加 `--wallpaper`（真的换壁纸）、不要跑 `set_display.py`（改分辨率）、不要跑 release 组（读写真实数据目录、socket 与 LaunchAgent，
  会和装着的 mini-todo 冲突）；M04 的 AppleScript `quit` 第一次会弹自动化授权。
- **收纳箱（桌面层窗口）**：
  - **窗口枚举要含桌面元素**：`maclib.list_windows` 默认带 `kCGWindowListExcludeDesktopElements`，Apple 没写它按什么判定"桌面元素"，桌面图标层 + 1
    的收纳箱可能被它滤掉；收纳箱用例一律 `windows_of(pid, layer=None, include_desktop=True)`，收纳箱 = 本进程负层级的窗口（或日志自报过的窗口号，
    层级不对时也找得到、交给层级断言判失败）。参照值 `Quartz.CGWindowLevelForKey(kCGDesktopIconWindowLevelKey) + 1`（`maclib.desktop_icon_level`）。
  - **读不到的属性让应用自报**：collectionBehavior、`canHide`、是否 NSPanel / 非激活在 CGWindowList / AX 里都没有，核对应用日志
    `[fence] 收纳箱窗口 #<窗口号> 已放到桌面层：level=… behavior=0x…`（只证明"设了且没被重设"，不证明系统行为）。
  - **真实文件拖放源**：访达窗口里的图标位置难定位，用 pyobjc 起一个小拖放源进程（`e2e-macos/drag_source.py`：窗口里按下即
    `beginDraggingSessionWithItems:` 放文件 `NSURL`），CGEvent 在它上面按下、慢速拖到收纳箱内容区再松开；没生效时换旧式 `NSFilenamesPboardType`
    再试一次作诊断。
  - **主窗口挡着收纳箱**：主窗口（layer 0）盖住的地方点不到收纳箱、也拖不进；1024×768 的 runner 上居中的主窗口常与右上角的收纳箱重叠，
    交互前先把主窗口拖开（`clear_main_off`）。
  - **非激活**：先 `activate_other("Finder")` 再点收纳箱，断言 `frontmost_pid()` 仍是 Finder；别的应用发起的隐藏用
    `NSRunningApplication.hide()` 模拟。改名（MF08）同样从访达在前台时开始，但改名会**有意激活应用**（预期前台变成 mini-todo，面板成为 key）；
    输入按上面的"文字输入用粘贴"（⌘A + ⌘V，回车也走 HID），失败时再用 `CGEventPostToPid` 区分"按键没路由到面板"与"面板没进入改名"
    （见 desktop-fences.md §M1「改名」）。
  - **预置数据**：需要已有收纳箱 / 项目的用例启动前直接写库 + 建文件夹（`seed_fence`），不依赖拖入。
  - **隐私授权框**：runner 有图形会话、没人点，授权框会让后台移动线程**永久阻塞**；所有来源 / 移回目标都在 `--work` 下的临时目录。
  - 测不了：Spaces（runner 只有一个桌面空间）、台前调度、调度中心 /「显示桌面」、中文输入法候选窗、Retina / 多屏、真实 `~/Desktop` 的 TCC 流程。
- **取回结果**：产物下载不可用时用注解 API，再不行从 job 日志按 PNG 头切分还原截图（`verification.md` §7.3）。

→ Executable details: `.trellis/spec/backend/window-modes.md` §6, `.trellis/spec/backend/desktop-fences.md` §6,
`pc-gpui/scripts/e2e-windows/README.md`; macOS: `window-modes.md`「Scenario: macOS」, `desktop-fences.md`「场景：macOS 上的收纳箱」§M5,
`pc-gpui/scripts/e2e-macos/README.md`.
