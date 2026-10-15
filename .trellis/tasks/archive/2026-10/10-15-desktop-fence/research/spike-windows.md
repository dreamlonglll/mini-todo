# Spike：收纳箱窗口层（PR0，A 工作线）

- **日期**：2026-10-15
- **环境**：Windows 11 Home China 10.0.26300（24H2 之后的版本，Win+D 抬起 Progman 的新桌面层级）；
  双屏混合 DPI：主屏 `\\.\DISPLAY2` 3840×2160 @150%（工作区 3840×2088），副屏 `\\.\DISPLAY1` 2560×1600 @125%，
  在主屏左侧（x = -2560）
- **被测**：开发版 `pc/src-tauri/target/debug/mini-todo.exe`（vite dev server，未打包的前端），PR1 实现的收纳箱窗口
  （`commands/fence.rs` 建窗 + `window.rs`「始终嵌入桌面的附属窗口」）
- **隔离**：`MINI_TODO_DATA_DIR` / `MINI_TODO_FENCES_ROOT` 指向会话临时目录；WebView2 用户数据目录随之落到
  `<data>\EBWebView\EBWebView`（`memory` 输出里每个 msedgewebview2 进程的 `--user-data-dir` 已核实），
  与正在运行的安装版 2.5.0（PID 38132）的浏览器进程完全分开
- **工具**：`pc/scripts/e2e/fence.ps1`（新增）、`probe-window.ps1`（新增）、`cdp.mjs`（已排除 `#/fence` 作为默认目标）

---

## 结论速览

| 问题 | 结论 |
|---|---|
| 内存：1 / 3 / 5 个收纳箱 | **每个收纳箱一个独立 renderer 进程**（同源也不共享），开发版每个约 +45~60 MB 私有内存（renderer ≈45 MB + GPU 进程增长）。5 个收纳箱时进程树私有内存 ≈ 主窗口单独时的 2.1~2.4 倍。Chromium `--process-per-site` 可让所有窗口共用一个 renderer，5 个收纳箱时总私有内存从 ≈439 MB 降到 ≈336 MB（见 §1），但要全局改 WebView2 启动参数，留给主会话决定 |
| N 个挂在 Progman 下的窗口在 Explorer 重启后能否全部重挂 | **能**。5 个收纳箱全部存活、全部在第一次轮询内重挂到新 Progman（Explorer 被杀后约 1.5 s 内）；之后 Win32 状态与重启前一致 |
| 两次 Win+D：收纳箱可见 / 其它应用窗口能恢复 | **通过**（真实 Win+D 按键）：第一次 Win+D 后 5 个收纳箱与嵌入态主窗口始终可见（从未最小化 / 隐藏，中心命中自己，截图确认显示在桌面上）；**第二次 Win+D 后探针应用窗口恢复到原位置、之前的前台窗口重新获得焦点**，NoFences PR #14 的副作用没有复现。Win+M 同样不影响收纳箱 |
| 多个收纳箱重叠时点击其中一个，Z 序是否被成组重排 | **不会成组重排**（真实点击）：被点的收纳箱升到桌面层这一组的最上面（其它收纳箱、嵌入态主窗口之上），其余收纳箱相对顺序不变；所有普通应用窗口仍盖在它上面。与 research 预测的"点哪个哪个沉下去"相反，但更符合直觉。**不需要**子类化 `WM_WINDOWPOSCHANGING` |
| 显示 / 隐藏（tao `show()` / `hide()`）会不会闪任务栏 | 不会出现任务栏按钮：UIA 轮询任务栏 3 次切换均未见按钮；ex style 采样显示 `show()` 后有约 **1.7 ms** 处于"可见 + `WS_EX_APPWINDOW` + 无 `WS_EX_TOOLWINDOW`"，随即被排队的重挂改回（§5） |
| 新建收纳箱会不会闪任务栏 | 不会（UIA 轮询 4 s 未见按钮）；`skip_taskbar(true)` 在 `set_visible` 之前生效 |

---

## 1. 内存占用

测量方法：`fence.ps1 -Step memory` 汇总开发版进程树（`mini-todo.exe` + 它拉起的 `msedgewebview2.exe` 及子进程）的
`PrivateMemorySize64` 与 `WorkingSet64`。收纳箱用 `create_fence` 命令逐个新建（经 CDP 调用），每档等 10~12 s 稳定后读数。

| 状态 | 进程数（webview2 / renderer） | 私有内存合计 | 工作集合计 |
|---|---|---|---|
| 只有主窗口 | 7（6 / 1） | ≈208 MB | ≈410 MB |
| + 1 个收纳箱 | 8（7 / 2） | ≈261 MB | ≈504 MB |
| + 3 个收纳箱 | 10（9 / 4） | ≈403 MB | ≈688 MB |
| + 5 个收纳箱 | 12（11 / 6） | ≈507 MB（重启后复测 ≈439 MB） | ≈863 MB |
| + 5 个收纳箱，`--process-per-site` | 7（6 / **1**） | ≈336 MB | ≈472 MB |

- 每个收纳箱窗口各自一个 renderer（≈41~50 MB 私有 / ≈82~87 MB 工作集），GPU 进程随窗口数增长（85 → 123~155 MB）
- 同一个 user data folder 下 browser 进程是共享的（只有一个），只有 renderer 按窗口增加
- 数据来自开发版：页面经 vite 以未打包模块加载，release 的 JS 堆会小一些，但"每窗口一个 renderer"的结构不变
- `--process-per-site` 的代价：所有窗口共用一个 renderer，一个页面卡死 / 崩溃会连累主窗口与全部收纳箱；而且它是
  **整个 WebView2 环境**的启动参数（同一 user data folder 下所有 WebView 必须一致），要在 `tauri.conf.json` 的主窗口
  `additionalBrowserArgs` 里设，并保留 wry 默认的 `--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection`。
  属于全局行为变更，本轮**不实现**，作为可选优化交给主会话决定
- 本次测量里 `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS`（为 CDP 设置）会覆盖 wry 的默认参数，对内存影响可忽略

## 2. Explorer 重启后多窗口重挂

`fence.ps1 -Step restart-explorer`（用户已授权重启 explorer.exe）：

```
before: 5 个收纳箱 owner=0x102E4（旧 Progman），主窗口为普通模式 owner=0
progman back after 0.0s（脚本先等 1.5 s）: 0x20730
fence ×5  alive=True  reattached=0.0s（第一次 100 ms 轮询即已是新 Progman）
main alive=True
```

- 5 个收纳箱全部存活：跨进程的 owner 被销毁不会连带销毁我们的窗口（与主窗口 09-07 的结论一致）
- 200 ms 轮询线程 `tick_embedded_windows` 用 `needs_reattach`（与主窗口同一纯函数）判定，全部在一个 tick 内排队重挂
- 重挂后全部 `ownerIsProgman=True / toolWindow=True / appWindow=False / minimizeBox=False`
- 小现象：重挂后某个收纳箱的 Z 序落在几个隐身的 Shell CoreWindow（"Windows 输入体验"等，cloaked）之上、仍在所有
  普通应用窗口之下；探针窗口照样盖住它。不影响可见行为，未处理
- 第二轮复测（主窗口嵌入 + 收纳箱共存）：见 `e2e-progress.md`

## 3. 两次 Win+D（NoFences PR #14 副作用）

**结论：通过。** 在用户离开、会话解锁且前台不是游戏时用真实按键完成（`fence.ps1 -Step win-d-key`，即 `keybd_event` 的 Win+D）。
本机这一版本的 Win+D 是"最小化所有应用窗口"（探针 `everIconic=True`、坐标变为 -32000）：

| 步骤 | 收纳箱 ×5 + 嵌入态主窗口 | 探针（普通应用窗口） |
|---|---|---|
| 第一次 Win+D | everIconic=False, everHidden=False, rect 不变, centerHitSelf=True；截图里 5 个收纳箱（含折叠态的标题栏）都显示在桌面上 | 最小化 |
| 第二次 Win+D | 同上，rect 不变（中心被恢复的应用窗口盖住，centerHitSelf=False 属正常） | **恢复**：iconicNow=False、rect 回到 (2400,150)-(3400,650)，原前台窗口重新获得焦点 |
| Win+M | everIconic/everHidden=False，centerHitSelf=True | 最小化（随后用 Shell `UndoMinimizeALL()` 恢复了用户的窗口） |

测试步骤（已固化）：

```powershell
pwsh -NoProfile -File fence.ps1 -Step probe-start -X 2600 -Y 100 -W 900 -H 500   # 与收纳箱重叠的普通应用窗口
pwsh -NoProfile -File mouse.ps1 -Action click -X <探针标题栏>                       # 让探针成为前台
pwsh -NoProfile -File fence.ps1 -Step win-d-key     # 第一次：收纳箱 everIconic/everHidden=False、centerHitSelf=True
pwsh -NoProfile -File fence.ps1 -Step win-d-key     # 第二次：探针 iconicNow=False、visibleNow=True、centerHitSelf=True
pwsh -NoProfile -File fence.ps1 -Step probe-state   # 探针仍可见、可成为前台
```

实测中遇到的环境限制（已写进 e2e README）：前台是以管理员权限运行的程序（本次是游戏客户端 `LeagueClientUx`）时，
`keybd_event` / `mouse_event` 的模拟输入全部被丢弃，Shell `ToggleDesktop()` 也不生效；屏幕锁定时同理；用户在玩游戏时
主动暂停，等用户离开、会话解锁后再做。这期间收纳箱在所有尝试下都保持 `everIconic=False / everHidden=False / rect 不变`。

## 4. 重叠收纳箱的 Z 序

**结论：不会成组重排，不需要子类化 `WM_WINDOWPOSCHANGING`。** 真实输入步骤：把"收纳箱 4"拖到与"收纳箱 3"重叠，
再真实点击下面那个（"收纳箱 3"露出的标题栏）：

```
拖动后：         z[3] 收纳箱 3   z[5] 工作 软件   z[6] 安装版主窗口   z[7] 收纳箱 4（刚被拖动 / 激活，前台）
点击收纳箱 3 后：z[4] 工作 软件   z[5] 安装版主窗口   z[6] 收纳箱 4   z[7] 收纳箱 3（前台）
重叠区命中：     点击前是收纳箱 4，点击后是收纳箱 3；探针（普通应用窗口）始终盖住两者
```

- 被激活的收纳箱升到 Progman owned 组的最上面（桌面层内"置前"），其它收纳箱相对顺序不变，没有成组重排
- 与 research 1.5 的推断（tao 的 `ALWAYS_ON_BOTTOM` 钩子会让被点的窗口沉到最底）相反：激活时系统对 owned 窗口组的
  调整没有被钩子改成沉底。结果更符合直觉（点哪个哪个到前面），且始终在所有普通应用窗口之下
- 每次挂载都 `SetWindowPos(HWND_BOTTOM)`，所以新建 / 重挂的收纳箱先落在组内最底层（紧贴 Progman）；嵌入态主窗口
  进入嵌入时同样落到 z[1]
- 新建位置默认找不重叠的空位（§6），日常使用中重叠需要用户主动拖到一起

## 5. 显示 / 隐藏与任务栏

- `toggle_fences_visibility` 只走 tao `show()` / `hide()`，紧跟 `reassert_embedded_window`
- ex style 高频采样（每次约 42 万个样本）：
  - `hide()`：`vis=True tool=True` → 76 ms `vis=False tool=False app=True`（apply_diff 重写）→ 0.6 ms 后排队的重挂改回
    `tool=True app=False`；隐藏期间就已恢复工具窗口样式
  - `show()`：57 ms `vis=True tool=True`（`ShowWindow` 先于样式重写，显示瞬间仍是工具窗口）→ 62.7 ms
    `tool=False app=True` → 64.4 ms 改回。**可见 + APPWINDOW 的窗口约 1.7 ms**
- UIA 轮询任务栏按钮（约 20 ms 一次），3 次切换、1 次新建都没有出现收纳箱按钮
- 如果以后在别的机器上看到闪烁：可改为建窗时 `owner_raw(Progman)`（tao 走 `Parent::OwnedBy`，flags 里没有
  `ON_TASKBAR`，`apply_diff` 永远不会写 `WS_EX_APPWINDOW`），见 tech-feasibility 1.3

## 6. 其它实测结论（PR1 实现相关）

- 建窗参数一次给齐后，收纳箱的 Win32 状态：`ownerIsProgman=True、toolWindow=True、appWindow=False、minimizeBox=False、
  thickFrame=False（resizable(false)）、noActivate=False、topmost=False`；整个测试过程中（折叠 / 展开 / 锁定 / 改名 /
  改外观 / 显示隐藏 / 重置位置）都没有变化
- 收纳箱保持可激活（未设 `focusable(false)` / `WS_EX_NOACTIVATE`），集成阶段双击启动的程序才能拿到前台
- 混合 DPI：折叠态高度必须按**目标位置所在显示器**的缩放算。第一版取窗口当前缩放，挪到 125% 副屏时折叠高度是
  48 px（应为 40）、重置回 150% 主屏时是 40 px（应为 48）；已改为 `title_scale()` 按目标工作区取缩放并加单测，复测正确
- 跨 DPI 移动时"先 set_position 再 set_size"有效：位置触发 `WM_DPICHANGED` 后 tao 按建议矩形缩放，随后的物理尺寸
  `set_size` 把尺寸定准
- 新建位置：从光标所在显示器工作区的右上角开始，从右往左找不重叠的空位（5 个收纳箱在 3840 宽的主屏上排成一行，
  间距 24 px = 16 DIP×1.5）；删除一个后新建的会补进空位，标题复用空出来的"收纳箱"
- 启动夹回：库里坐标 (9000, 5000) 的收纳箱启动后显示在主屏右下角 (3288, 1584)，库里坐标保持不变（不落库）
- **缩放不能用 `startResizeDragging`**：它发的是 `WM_NCLBUTTONDOWN(HTBOTTOMRIGHT…)`，而 Windows 只对带 `WS_THICKFRAME`
  的窗口进入系统缩放循环；收纳箱为了"锁定 / 折叠时不能缩放、不参与 Aero Snap、锁定解锁不碰 tao flag"建成
  `resizable(false)`，真实拖动手柄毫无反应。改为前端缩放手柄捕获指针（`setPointerCapture`）、按位移算新矩形，每帧调
  `set_fence_bounds`（一次 `SetWindowPos(SWP_NOZORDER | SWP_NOACTIVATE)`，拖左 / 上边不抖），松手后 `commit_fence_geometry`。
  真实拖动右下角 +100/+100、左上角 +100/+100 均正确落库，嵌入样式不变
- 真实拖动标题栏移动：按下后移动超过 3 px 才 `startDragging`，松手后约 300 ms 落库（与窗口位置一致）；锁定的收纳箱拖不动
- 真实双击标题进入改名（输入框获得焦点）；改名中点到桌面，窗口失去焦点即提交
- 真实 OLE 拖文件（WinForms 拖动源、`CF_HDROP` 临时文件）经过折叠的收纳箱：进入时展开到 504 px，离开后收回到 48 px，
  再进入又展开；放下后鼠标移开即收回（PR1 不处理文件，源文件不受影响）
- 托盘三项（真实右键开发版托盘图标、真实点击菜单项）：「新增收纳箱」在光标所在显示器找到空位新建（"收纳箱 3"）；
  「显示/隐藏所有收纳箱」两次切换全部隐藏 / 显示；「重置收纳箱位置」全部排回主屏一行并落库
- 渲染首帧：`index.html` 内联的白色 body 背景在 main.scss 加载前可能闪一下白；release 构建里样式表是 `<head>` 中的
  `<link>`，首帧之前就加载，预计只影响 dev。未逐帧录屏确认

## 7. 实测发现的主窗口启动竞态（已修）

开发版在"主窗口嵌入桌面 + 5 个收纳箱"状态下重启，主窗口最终 `owner=Progman` 但 `toolWindow=False / appWindow=True`
（会出现在任务栏、参与 Alt+Tab）。逐帧跟踪 ex style：

```
475 ms  minbox=False             <- set_window_desktop_mode 里的 set_minimizable(false)
482 ms  owner=Progman tool=True  <- 紧接着的 desktop_attach（同步命令在主线程，run_on_main_thread 就地执行）
519 ms  sizebox=False tool=False app=True  <- 前端早先发出的 setResizable(false) 这时才执行，apply_diff 抹掉 TOOLWINDOW
（之后再没有人补回来）
```

原因：前端 `applyFixedMode` 先 `await setResizable(false)`（异步窗口插件命令，只是把消息排进事件循环）再调同步命令
`set_window_desktop_mode`。主线程正处在 WebView2 的嵌套消息泵里（同时在创建收纳箱窗口）时，同步命令先于排队的
`SetResizable` 执行，而里面的 `run_on_main_thread` 是就地执行、不是排队，于是"重申排在 tao 操作之后"的前提不成立。
这是上游就有的潜在竞态，启动时并发恢复收纳箱让它稳定复现。

修复（`window.rs`）：200 ms 轮询线程在桌面模式、经典固定模式、收纳箱三处都检查 `WS_EX_TOOLWINDOW` / `WS_EX_APPWINDOW`
（纯函数 `lacks_tool_window_style`，有单测），被抹掉就从轮询线程排队重申——从非主线程排队一定排在已入队的 tao 操作
之后。修复后同一场景 717 ms 时样式自愈，最终状态正确；正常情况下每个 tick 只多一次 `GetWindowLongW`。
残留：竞态命中时约有 200 ms 主窗口带 APPWINDOW，任务栏可能闪一下。
