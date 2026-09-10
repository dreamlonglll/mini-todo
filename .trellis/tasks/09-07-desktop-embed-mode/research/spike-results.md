# 桌面模式原型验证结果（2026-09-07）

用 `probe.ps1` 从 PowerShell 直接对 mini-todo 主窗口做 Win32 操作，在真实机器上验证两种"嵌入桌面"方案。
没有改任何 Rust 代码；实验对象是 `target\debug\mini-todo.exe`（v2.3.6 调试版）+ vite dev server，
与用户正在运行的安装版实例并存，实验后用快照把 `settings` / `screen_configs` 里被改动的窗口位置回滚。

## 环境

| 项 | 值 |
|---|---|
| OS | Windows 11 Home 25H2 build 26200 |
| 显示器 | DISPLAY1 副屏 2560x1600 @125%（物理 x -2560..0），DISPLAY2 主屏 1920x1080 @100% |
| 壁纸 | Wallpaper Engine 运行中（`WPEDesktopDX11Window` 挂在 Progman 的子 WorkerW 下） |
| 桌面层级 | `Progman`(GetShellWindow) → `SHELLDLL_DefView` → `SysListView32`；`WorkerW` 是 Progman 的**子窗口**（24H2+ 布局），壁纸引擎窗口在其中 |
| 主窗口初始样式 | style `WS_CAPTION\|WS_SYSMENU\|WS_MINIMIZEBOX\|WS_MAXIMIZEBOX\|WS_CLIPSIBLINGS`（无 WS_POPUP），ex `WS_EX_TOOLWINDOW\|WS_EX_WINDOWEDGE\|WS_EX_ACCEPTFILES`，无 WS_EX_LAYERED，DPI awareness = per-monitor v2 |

注意：`FindWindowW("Progman", NULL)` 在 PowerShell 里因为 `$null` 被转成 `""` 而失败，脚本改用 `GetShellWindow()`；
Rust 里用 `FindWindowW` 传 `PCWSTR::null()` 没问题，但 `GetShellWindow` 更省事，建议直接用它。

## 基线：现有固定模式在 Win+D 下的真实表现

`Shell.Application.ToggleDesktop()`（等价 Win+D）后以 20ms 间隔采样 1.5s：

* `IsIconic` 始终为 false，`IsWindowVisible` 始终为 true —— **窗口没有被最小化**
* 但截图里窗口区域只剩壁纸 —— 窗口被抬到顶层的桌面（Progman）**盖住了**
* 期间窗口矩形跳了一次（`SW_RESTORE` 引起的 placement 变化），再次 ToggleDesktop 后回到原位

结论：在这台 Win11 25H2 上，Show desktop 对工具窗口的处理是"把桌面抬到最上面盖住"，不是最小化。
`lib.rs` 里"被最小化就还原"的轮询在这条路径上其实帮不上忙。

## 方案 A：SetParent 挂到 Progman，排在 SHELLDLL_DefView 之上

操作：`WS_POPUP→WS_CHILD`，`SetParent(hwnd, Progman)`，`SetWindowPos(HWND_TOP, 宿主相对坐标)`。

| 检查项 | 结果 |
|---|---|
| Win+D 后可见、不最小化 | 通过（everIconic=false，截图窗口完整可见） |
| Win+D 恢复后仍在原位 | 通过 |
| 透明背景 | 通过，壁纸透出、半透明底正常 |
| 圆角（DWMWA_WINDOW_CORNER_PREFERENCE） | **丢失**，变直角 |
| 鼠标点击 | 通过（点日历"下月"箭头翻页） |
| 从主窗口打开编辑窗口 | 能打开，编辑窗口的 owner 被解析成 Progman |
| **DPI** | **`GetDpiForWindow` 从 120 变成 96**：子窗口继承 Progman 的 DPI（主屏 100%），在 125% 副屏上内容整体缩小 |
| 坐标 | `GetWindowRect` 仍是屏幕坐标；`SetWindowPos` 要用宿主客户区坐标（宿主原点 = 虚拟屏原点 (-2560,-240)） |
| 其它 | 跨进程父子窗口会共享输入队列；tao `apply_diff` 会把 `WS_CHILD` 抹回去，需要兜底 |

DPI 那条在用户当前"125% + 100%"双屏配置下是肉眼可见的退化，无法在不改 WebView 缩放的前提下绕开。

## 方案 B：保持顶层窗口，owner = Progman

操作：去 `WS_MINIMIZEBOX`，加 `WS_EX_TOOLWINDOW`、去 `WS_EX_APPWINDOW`，
`SetWindowLongPtr(GWLP_HWNDPARENT, Progman)`，`SetWindowPos(HWND_BOTTOM)`。

| 检查项 | 结果 |
|---|---|
| Z 序 | 立刻变成 z[1]，紧贴 Progman 之上、所有普通窗口之下；桌面图标（Progman 子窗口）与壁纸引擎窗口都在其下 |
| Win+D 后可见、不最小化 | 通过（everIconic=false，截图完整可见）——owned 窗口跟着被抬起的 Progman 一起浮上来，**不需要任何轮询** |
| Win+M（`Shell.MinimizeAll`） | 通过，`IsIconic=false`，窗口仍可见（无最小化框） |
| Win+D 恢复 | 通过，位置不变 |
| 透明 / 圆角 / DPI | 全部不变（仍是顶层窗口，DPI 120） |
| 鼠标悬停 / 点击 | 通过 |
| 点击桌面空白处（激活 Progman） | 窗口仍在 z[1]，没有被抬起 |
| 点击窗口本身（可激活状态） | **窗口被激活并抬到普通窗口之上**（z[18]），之后 `SetWindowPos(HWND_BOTTOM)` 立刻压回 z[1] |
| 加 `WS_EX_NOACTIVATE` 后点击 | 不再抬起，始终 z[1]；但从它打开的编辑窗口 `GetForegroundWindow` 不是编辑窗口（前台仍留在别的进程） |
| 从主窗口打开编辑窗口 | 编辑窗口 owner = 主窗口，正常显示在主窗口之上 |
| 退出（清 owner、恢复样式、HWND_TOP） | 恢复为普通顶层窗口 |

## 与方案无关的既有现象

* **首次点击丢失**：无论普通顶层、方案 A 还是方案 B，进程启动后（或焦点离开 WebView 后）
  第一次点击只让 WebView 拿到焦点，不会触发页面点击；第二次起正常。对照组是未做任何嵌入的
  普通窗口，同样丢第一次点击，说明这是现有应用的既有行为（疑似 `onFocusChanged → fetchTodos`
  重渲染或 WebView2 焦点处理），不属于本任务。
* 一次实验里调试版进程在最后一次 `restore` 后自行退出，原因未查明；正式实现在进程内完成
  样式/owner 切换，不会走外部 `SetWindowLongPtr`，需要在实现后重点回归"进入/退出桌面模式多次切换"。

## 结论

采用**方案 B**：

* 视觉效果与"嵌入桌面"一致（图标之上、所有窗口之下、Win+D / Win+M 免疫），且透明、圆角、DPI 全部零改动
* 不需要跨进程父子窗口，不需要坐标换算，tao 的位置/尺寸逻辑不用碰
* 需要处理的只有一件事：窗口被激活时会被抬到前面。选择"可激活 + 在 `WM_WINDOWPOSCHANGING`
  里把 `hwndInsertAfter` 强制为 `HWND_BOTTOM`"（Rainmeter OnDesktop 的做法，通过 comctl32
  `SetWindowSubclass` 挂钩），保留正常键盘焦点与子窗口前台行为；`WS_EX_NOACTIVATE` 作为退路
* Explorer 重启：owner 句柄失效，在现有 200ms 轮询里检测 `IsWindow(owner) == false` 或
  `GetShellWindow() != owner` 后重新挂 owner + `HWND_BOTTOM`

方案 A 保留为备选，不推荐：DPI 退化在用户的双屏配置下直接可见，圆角也会丢。

## 未验证项（留到实现后回归）

* Explorer 重启后自动重挂（本次没有在用户机器上杀 explorer）
* 多次进入/退出桌面模式的稳定性（见上面进程退出的观察）
* Win10 上 `GetShellWindow` / owner 行为（当前只在 Win11 25H2 验证）
* `SetWindowSubclass` 钩子与 wry 自身子类化的共存

截图留在会话 scratchpad（`00-baseline.png` … `81-plain-second-click.png`），未入库。
