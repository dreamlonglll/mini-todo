# Research: Win32 桌面嵌入模式（窗口位于桌面图标之上、所有普通窗口之下，并对 Win+D / 显示桌面免疫）

- **Query**: Windows 桌面小组件 / 壁纸软件如何把窗口"放到桌面上"（桌面图标之上、所有应用窗口之下），使 Win+D / 显示桌面 / Win+M 不会最小化或遮盖它；要求保留 DWM 逐像素透明。评估 (A) SetParent 进图标宿主、(B) 顶层窗口 + owner=Progman + Rainmeter 式 Z 序守护、(C) SetParent 进壁纸 WorkerW 三条路线。
- **Scope**: mixed —— 外部一手来源（GitHub 源码、Microsoft Learn、Raymond Chen、StackOverflow）+ 内部代码（`pc/src-tauri`、tao 0.34.5、wry 0.53.5、tauri 2.9.5、windows 0.58）
- **Date**: 2026-09-07
- **来源快照**: Lively 分支 `core-separation`（2026-04-30 推送）；Rainmeter `master`（2026-09-03）；ScreenPlay `master`；Microsoft Learn 当日抓取；本地 cargo registry 中的 tao-0.34.5 / wry-0.53.5 / tauri-2.9.5 / windows-0.58.0

> 证据等级标注：**[文档]** = Microsoft 官方文档明确写明；**[源码]** = 在开源项目源码 / 提交记录 / 维护者评论中观察到；**[推断]** = 由文档规则 + 源码观察推导，未实际运行验证；**[未验证]** = 没有一手证据。

---

## 结论与建议

1. **三条路线里，只有 B（保持顶层窗口 + Rainmeter 式 Show-desktop 检测与重排）在"必须保留 DWM 逐像素透明"这一硬约束下有直接先例。** Rainmeter 皮肤本身就是 `WS_EX_LAYERED | WS_EX_TOOLWINDOW` + `WS_POPUP` 的顶层窗口，没有 parent、没有 owner，从未调用 `SetParent`/`GWLP_HWNDPARENT`（[源码] `Library/Skin.cpp:291-303`；全文 grep 无 `SetParent`/`HWNDPARENT`）。它靠一个常驻 `HWND_BOTTOM` 的哨兵窗口 + 250 ms 定时器 + `EVENT_SYSTEM_FOREGROUND` 钩子检测"显示桌面"，再把皮肤重新插到被 Explorer 抬升的桌面宿主窗口之上（[源码] `Library/System.cpp:22-117, 457-610`）。Win11 24H2 的层级变化已由 Rainmeter 于 2025-04 修复（[源码] commit `b1128fb21e`, PR #413, issue #377）。
2. **A / C（`SetParent` 进 explorer.exe 的窗口树）是壁纸软件（Lively / ScreenPlay / Wallpaper Engine 类）的做法，但它们的窗口都是"不透明、全屏、不要键盘焦点"的**：透明靠 `SetLayeredWindowAttributes(alpha=0xFF)` 即完全不透明（微软对 24H2 的官方建议原话，[源码] Lively issue #2074），输入靠全局 RawInput / 低级钩子再 `PostMessage` 转发（[源码] Lively `InputUtil.cs`、ScreenPlay `windowsintegration.cpp:524-546`）。对 mini-todo 而言，A / C 会同时撞上三个硬问题：(a) tao 的透明实现是顶层窗口的 `DwmEnableBlurBehindWindow` + WebView2 透明背景色，子窗口没有独立 DWM 合成面（[源码] tao `window.rs:1284-1296`，[推断]）；(b) 跨进程 parent/child 隐式附着输入队列，键盘焦点/WebView2 焦点不可靠（[文档/权威] Raymond Chen 2013、SO 3460542、SO 75060992）；(c) tao 任何 flag 变更都会整体重写 `GWL_STYLE`，把手工加的 `WS_CHILD` 抹掉（[源码] tao `window_state.rs:425-440`；本仓库 `window.rs:858-866` 已为 `WS_EX_TOOLWINDOW` 踩过同一个坑）。
3. **"显示桌面"其实是两件事叠加**（[源码] Rainmeter 维护者 Brian Ferguson 在 #377 的说明 + `System.cpp` 逻辑）：(i) Explorer 把"可最小化"的窗口最小化；(ii) Explorer 把桌面图标宿主（≤23H2 是装着 `SHELLDLL_DefView` 的 `WorkerW`，24H2 是 `Progman` 本身）抬到 Z 序上方并设为前台，盖住没被最小化的窗口。所以方案 B 需要同时做到：**不被最小化**（去掉 `WS_MINIMIZEBOX`，Tauri 配置 `minimizable:false` 即可，[推断]，见原型 1）+ **被抬升后重新插到桌面宿主之上**（Rainmeter 算法）+ **桌面恢复时回到 `HWND_BOTTOM`**。
4. **owner = Progman 只建议作为可选增强而非基础**：它在 24H2 上理论上能让窗口跟着 Progman 一起被抬升（[文档] "An owned window is always above its owner in the z-order"），但在 Win10 / Win11 ≤23H2 上被抬升的是 `WorkerW` 而非 `Progman`，owner=Progman 起不到作用（[推断]）；且 owner 被销毁时被拥有窗口一并销毁（[文档] DestroyWindow）—— Explorer 重启会连带杀掉主窗口，除非收到 `TaskbarCreated` 前先把 owner 清空（[推断]）；跨进程 owner/owned 同样附着输入队列（[权威] Raymond Chen）。
5. **建议的实施顺序**：先做三个廉价原型（见文末"未验证 / 需要原型验证"第 1-3 条）：① `minimizable:false` 后 Win+D / Win+M 是否还最小化；② 在 Win10/11 23H2 与 24H2 上分别观察 Win+D 时 `GetForegroundWindow()` 与桌面宿主的 Z 序变化；③ 用 `SetWindowSubclass` 拦 `WM_WINDOWPOSCHANGING` 强制 `SWP_NOZORDER` 后点击窗口是否仍留在底层。三者都通过再实现 B 的完整状态机。

---

## 1. 桌面窗口层级（Windows 10 / Windows 11 ≤23H2 / Windows 11 24H2+）

### 1.1 传统层级（Win7 ~ Win11 23H2）

Rainmeter `Library/System.cpp:247-256`（[源码]，注释原文）：

```cpp
// Windows 11 24H2 reordered the desktop shell window hierarchy.
//
// Spy++ output before Windows 11 24H2:
//
//   0x00010190 "" WorkerW
//     ...
//     0x000100EE "" SHELLDLL_DefView
//       0x000100F0 "FolderView" SysListView32
//   0x00100B8A "" WorkerW
//   0x000100EC "Program Manager" Progman
```

Lively `WinDesktopCore.cs:163-173` 的注释与之一致，并标出 `0x00100B8A "" WorkerW  <-- This is the WorkerW instance we are after!`（壁纸软件要挂进去的那个空 WorkerW）。

要点：
- `Progman`（类名 `Progman`，标题 "Program Manager"）是 `GetShellWindow()` 返回的 Shell 桌面窗口（[文档] GetShellWindow: "Retrieves a handle to the Shell's desktop window"）。Rainmeter 用 `GetShellWindow()` 并校验类名为 `Progman`（`System.cpp:220-245`）。
- 默认状态下 `SHELLDLL_DefView`（内含 `SysListView32 "FolderView"` 图标列表）直接挂在 `Progman` 下；当桌面进入"raised desktop"状态（壁纸幻灯片切换、淡入淡出动画，或收到 `0x052C`），Explorer 会创建顶层 `WorkerW`，把 `SHELLDLL_DefView` 挪进其中，再在它后面创建另一个空的顶层 `WorkerW` 画壁纸（[源码] CodeProject 2014 文章 "Draw Behind Desktop Icons in Windows 8+"；Lively `DesktopUtil.cs:39-47, 65-75` 注释 "When this fails (picture rotation is turned ON), then look for the WorkerW windows list"）。
- 所以"图标宿主"在 ≤23H2 上是动态的：可能是 `Progman`，也可能是某个 `WorkerW`。Rainmeter 只在 DefView 的父窗口是 `WorkerW` 时才认为存在可用的宿主（`System.cpp:290-329`）。

### 1.2 Windows 11 24H2（build 26100+）

Lively 作者于 2024-04-01 在 issue #2074 贴出的微软官方回复（[源码] https://github.com/rocksdanister/lively/issues/2074#issuecomment-2030662089 ，Lively 源码 `WinDesktopCore.cs:129-147` 原样引用）：

> When the desktop is split out from the list view window (aka the "raised desktop") we no longer create multiple top-level HWNDs to support this scenario. Instead, the top-level "Progman" window is now created with WS_EX_NOREDIRECTIONBITMAP (so there is no GDI content for that window at all) and the shell DefView child window is a WS_EX_LAYERED child window. When the desktop is raised, we create a child WorkerW window that is z-ordered under the DefView that will render the wallpaper. The DefView window will draw mostly transparent with just the icons and text.
>
> If your application forces the "raised desktop" state, it will now need to create its own WS_EX_LAYERED child HWND that is z-ordered under the DefView window but above the WorkerW window. This window should likely be a SetLayeredWindowAttributes(bAlpha=0xFF) window so that you can do DX blt presents to it and not suffer performance issues.

Rainmeter `System.cpp:258-265`（[源码]）：

```cpp
// Spy++ output after Windows 11 24H2:
//
//   0x000100EC "Program Manager" Progman
//     0x000100EE "" SHELLDLL_DefView
//       0x000100F0 "FolderView" SysListView32
//     0x00100B8A "" WorkerW
//
// So if we're on 24H2+, we should be using the shell window (Progman) instead of WorkerW.
```

Lively PR #2050（2023-12-23，针对 Insider 26002+）给出相同结构（[源码] https://github.com/rocksdanister/lively/pull/2050 ）。

2025-09-04 Lively 作者转述微软工程师的补充（[源码] https://github.com/rocksdanister/lively/issues/2074#issuecomment-3253799666 ）：

> The "raised desktop" state where the DefView is a child layered window is the default state for builds >= 26100. However the DefView does not render with transparency today unless you have HDR enabled on the system or there is a slideshow animation going... Starting in the upcoming 27XXX build number releases the DefView will always render mostly transparent, eliminating your need to do the SendMessage() stuff to force it to be transparent. Sending the messge should be ok even on those builds.
> So short answer is looking for WS_EX_NOREDIRECTIONBITMAP is the easiest way to know you should be using a correctly z-ordered layered child window.

Rainmeter 维护者对 24H2 的概括（[源码] https://github.com/rainmeter/rainmeter/issues/377#issuecomment-2564245420 ）："The problem is, Windows 11 24H2 no longer spawns a "worker" window that Rainmeter has used since Windows XP to layer the skins with."

### 1.3 未公开消息 `0x052C`

- 出处：CodeProject 文章 "Draw Behind Desktop Icons in Windows 8+"（Gerald Degeneve，2014；原站已下线，Web Archive 可读）。作者用 Spy++ 监视 Progman 消息，发现更换壁纸时 Progman 首先收到用户自定义消息 `0x052C`，发送该消息即可让 Progman 生成位于图标之后的 WorkerW（[源码] 文章原文："I extended the test program to send exactly this user defined message (0x052C) to the Program Manager... After receiving the message, the Program Manager creates the WorkerW window."）。文章代码注释被 Lively 原样沿用：`// Send 0x052C to Progman. This message directs Progman to spawn a WorkerW behind the desktop icons. If it is already there, nothing happens.`（Lively `WinDesktopCore.cs:152-154`）。
- 参数：CodeProject 原文与 ScreenPlay 24H2 分支用 `wParam=0, lParam=0`；Lively 与 ScreenPlay 非 24H2 分支用 `wParam=0xD, lParam=0x1`（Lively `WinDesktopCore.cs:155-161`；ScreenPlay `windowsintegration.cpp:131, 147`）。两种参数都有项目在用，语义差异没有一手文档（[未验证]）。
- 24H2 上仍然有效：issue #2074 中 2025-02-15 有人称"微软移除了 0x052C"，但 2025-06-08 的评论明确反驳"**The 0x052C api call still works in 24h2**. The reason why it is not working is because of the default desktop hierarchy has changed"，微软工程师 2025-09 也说 "Sending the messge should be ok even on those builds"（[源码]）。
- 副作用：24H2 上 `SystemParametersInfo(SPI_SETDESKWALLPAPER)` 会销毁当前 WorkerW，Lively 在 24H2 分支里直接跳过刷新（`WinDesktopCore.cs:1003-1010`，注释 "Otherwise will destroy the current WorkerW"）；SO 78169263 的回答也观察到 24H2 预览版换壁纸/主题会关闭 WorkerW（[源码]）。
- **对本任务的意义**：`0x052C` 只用于"在图标之后画东西"（方案 C）。方案 A / B 不需要它。

### 1.4 三家项目如何识别 24H2

| 项目 | 判定方式 | 位置 |
|---|---|---|
| Lively | `HasExtendedStyle(progman, WS_EX_NOREDIRECTIONBITMAP)`（微软工程师推荐的判法） | `WinDesktopCore.cs:148-150` |
| Rainmeter | `GetProcAddress(GetModuleHandle(L"user32"), "GetCurrentMonitorTopologyId") != nullptr`，注释说明只在 10.0.26100.2454+ 存在 | `System.cpp:266-271` |
| ScreenPlay | `GetWindowsBuildNumber() >= 26100` | `windowsintegration.cpp:35-39` |

### 1.5 定位逻辑（综合三家，Rust 风格伪码）

```rust
struct DesktopLayers {
    progman: HWND,            // GetShellWindow() / FindWindowW("Progman")
    def_view: HWND,           // SHELLDLL_DefView（图标）
    icons_host: HWND,         // def_view 的父窗口：≤23H2 是 WorkerW 或 Progman；24H2 恒为 Progman
    wallpaper_workerw: HWND,  // 图标之后画壁纸的 WorkerW（仅方案 C 需要）
    layered_defview: bool,    // 24H2 "raised desktop with layered DefView"
}

unsafe fn locate_desktop_layers(spawn_wallpaper_workerw: bool) -> Option<DesktopLayers> {
    let progman = GetShellWindow();                      // Rainmeter: 再校验类名 == "Progman"
    if progman.is_invalid() { return None; }
    let layered_defview = (GetWindowLongPtrW(progman, GWL_EXSTYLE) as u32 & WS_EX_NOREDIRECTIONBITMAP.0) != 0; // Lively

    if spawn_wallpaper_workerw {
        // 仅方案 C：强制 raised desktop
        SendMessageTimeoutW(progman, 0x052C, WPARAM(0xD), LPARAM(0x1), SMTO_NORMAL, 1000, None);
    }

    if layered_defview {
        // 24H2：一切都在 Progman 之下（Lively 194-205；ScreenPlay 110-141；Rainmeter 281-288）
        let def_view = FindWindowExW(progman, None, w!("SHELLDLL_DefView"), None).ok()?;
        // ScreenPlay：在 Progman 的 WorkerW 子窗口里挑“不含 DefView”的那个；
        // Lively：直接取第一个 WorkerW，再用 EnsureWorkerWZOrder 把它压到最后一个子窗口
        let mut w = FindWindowExW(progman, None, w!("WorkerW"), None).ok();
        while let Some(h) = w {
            if FindWindowExW(h, None, w!("SHELLDLL_DefView"), None).is_err() { break; }
            w = FindWindowExW(progman, h, w!("WorkerW"), None).ok();
        }
        return Some(DesktopLayers { progman, def_view, icons_host: progman,
                                    wallpaper_workerw: w.unwrap_or_default(), layered_defview });
    }

    // ≤23H2：遍历顶层窗口，找“含 SHELLDLL_DefView 的窗口”，它的下一个兄弟 WorkerW 即壁纸层
    // （Lively 174-192；ScreenPlay 156-167；Rainmeter 312-325 另加 IsWindowVisible + 同进程校验）
    let mut found = None;
    EnumWindows(Some(enum_proc), LPARAM(&mut found as *mut _ as isize));
    unsafe extern "system" fn enum_proc(top: HWND, lp: LPARAM) -> BOOL {
        let out = &mut *(lp.0 as *mut Option<(HWND, HWND, HWND)>);
        if let Ok(dv) = FindWindowExW(top, None, w!("SHELLDLL_DefView"), None) {
            let next_workerw = FindWindowExW(None, top, w!("WorkerW"), None).unwrap_or_default();
            *out = Some((top, dv, next_workerw));
            return BOOL(0);
        }
        BOOL(1)
    }
    let (icons_host, def_view, wallpaper_workerw) = found?;
    Some(DesktopLayers { progman, def_view, icons_host, wallpaper_workerw, layered_defview })
}
```

Rainmeter 在 ≤23H2 上还多做两件事（`System.cpp:290-329`）：缓存 `c_DefView`，每次先 `IsWindow(c_DefView)` + `GetAncestor(c_DefView, GA_PARENT)` 校验父窗口仍是 `WorkerW`；如果 DefView 的父窗口是 `Progman`（桌面未 raised），返回 `nullptr`。

---

## 2. Rainmeter 的 ZPosition "OnDesktop" 实现

### 2.1 皮肤窗口本身：顶层、无 owner、无 parent

`Library/Skin.cpp:289-303`（[源码]）：

```cpp
void Skin::Initialize()
{
	m_Window = CreateWindowEx(
		WS_EX_LAYERED | WS_EX_TOOLWINDOW,
		METERWINDOW_CLASS_NAME,
		nullptr,
		WS_POPUP,
		CW_USEDEFAULT, CW_USEDEFAULT, CW_USEDEFAULT, CW_USEDEFAULT,
		nullptr,          // hWndParent：无 parent、无 owner
		nullptr,
		GetRainmeter().GetModuleInstance(),
		this);
	...
	// Mark the window to ignore the Aero peek
	IgnoreAeroPeek();   // DwmSetWindowAttribute(m_Window, DWMWA_EXCLUDED_FROM_PEEK, TRUE)  (Skin.cpp:378-382)
```

结论：**Rainmeter 皮肤从不 reparent，也不设 owner**（`Skin.cpp` / `System.cpp` 全文无 `SetParent`、无 `GWLP_HWNDPARENT`）。它是 `WS_POPUP`（无 `WS_SYSMENU`、无 `WS_MINIMIZEBOX`）+ `WS_EX_TOOLWINDOW`（不进任务栏 / Alt+Tab，[文档] Extended Window Styles）+ `WS_EX_LAYERED`（逐像素透明）。

### 2.2 两个辅助窗口（`RainmeterSystem` 类）

`Library/System.cpp:51-117`（[源码]）：

```cpp
#define ZPOS_FLAGS	(SWP_NOMOVE | SWP_NOSIZE | SWP_NOOWNERZORDER | SWP_NOACTIVATE | SWP_NOSENDCHANGING)

enum INTERVAL
{
	INTERVAL_SHOWDESKTOP    = 250,
	INTERVAL_RESTOREWINDOWS = 100,
	...
};

void System::Initialize(HINSTANCE instance)
{
	...
	wc.lpszClassName = L"RainmeterSystem";
	c_Window = CreateWindowEx(WS_EX_TOOLWINDOW, MAKEINTATOM(className), L"System",
		WS_POPUP | WS_DISABLED, ...);                 // 哨兵：永远在 HWND_BOTTOM
	{
		DpiUtil::DpiUnawareScope dpiUnaware;
		c_HelperWindow = CreateWindowEx(WS_EX_TOOLWINDOW, MAKEINTATOM(className), L"PositioningHelper",
			WS_POPUP | WS_DISABLED, ...);             // 锚点：皮肤都插在它后面
	}
	...
	SetWindowPos(c_Window, HWND_BOTTOM, 0, 0, 0, 0, ZPOS_FLAGS);
	SetWindowPos(c_HelperWindow, HWND_BOTTOM, 0, 0, 0, 0, ZPOS_FLAGS);
	...
	c_WinEventHook = SetWinEventHook(EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_FOREGROUND, nullptr, MyWinEventProc,
		0, 0, WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS);
	SetTimer(c_Window, TIMER_SHOWDESKTOP, INTERVAL_SHOWDESKTOP, nullptr);   // 250 ms
}
```

两个辅助窗口的窗口过程对 `WM_WINDOWPOSCHANGING` 一律追加 `SWP_NOZORDER`（`System.cpp:612-628`），即除了 Rainmeter 自己带 `SWP_NOSENDCHANGING` 的调用外，谁也改不了它们的 Z 序。

### 2.3 检测 "Show desktop"：`CheckDesktopState`

`System.cpp:523-560`（[源码]）：

```cpp
bool System::CheckDesktopState(HWND desktopIconsHostWindow)
{
	HWND hwnd = nullptr;

	if (desktopIconsHostWindow && IsWindowVisible(desktopIconsHostWindow))
	{
		hwnd = FindWindowEx(nullptr, desktopIconsHostWindow, L"RainmeterSystem", L"System");
	}

	bool stateChanged = (hwnd && !c_ShowDesktop) || (!hwnd && c_ShowDesktop);

	if (stateChanged)
	{
		c_ShowDesktop = !c_ShowDesktop;
		...
		PrepareHelperWindow(desktopIconsHostWindow);
		ChangeZPosInOrder();
		WindowOcclusionTracker::HandleShowDesktopChange();

		if (c_ShowDesktop)
			SetTimer(c_Window, TIMER_SHOWDESKTOP, INTERVAL_RESTOREWINDOWS, nullptr);   // 100 ms
		else
			SetTimer(c_Window, TIMER_SHOWDESKTOP, INTERVAL_SHOWDESKTOP, nullptr);      // 250 ms
	}
	return stateChanged;
}
```

原理：`FindWindowEx(nullptr, hwndChildAfter = 桌面宿主, "RainmeterSystem", "System")` 在顶层窗口链中**从桌面宿主之后**往下找哨兵窗口。哨兵常驻 `HWND_BOTTOM`，正常状态下桌面宿主也在最底部、哨兵在它前面，找不到；一旦 Explorer 把桌面宿主抬升到哨兵之上，就能找到 → 判定进入 "Show desktop"。老版本（2015，sha `1b054fc4`）函数上方注释即 `** Changes the "Show Desktop" state.`。

触发时机有两个：250 ms 定时器（`System.cpp:633-638`）与 `EVENT_SYSTEM_FOREGROUND` 钩子（`System.cpp:562-610`）：

```cpp
void CALLBACK System::MyWinEventProc(...)
{
	if (event == EVENT_SYSTEM_FOREGROUND)
	{
		if (!c_ShowDesktop)
		{
			if (ShouldUseShellWindowAsDesktopIconsHost())        // 24H2+
			{
				if (hwnd == GetDefaultShellWindow())            // Progman 成为前台
				{
					... while (loop < max && !CheckDesktopState(hwnd)) { Sleep(2); ++loop; }
				}
				return;
			}
			// ≤23H2：explorer 进程的某个 WorkerW 成为前台
			if (GetClassName(hwnd, ...) && wcscmp(className, L"WorkerW") == 0 && BelongToSameProcess(GetDefaultShellWindow(), hwnd))
			{
				// 等 SHELLDLL_DefView 被挪进这个 WorkerW（最多 5×2ms）
				while (loop < max && FindWindowEx(hwnd, nullptr, L"SHELLDLL_DefView", L"") == nullptr) { Sleep(2); ++loop; }
				if (loop < max) { ... while (loop < max && !CheckDesktopState(hwnd)) { Sleep(2); ++loop; } }
			}
		}
	}
}
```

### 2.4 Windows 在 "Show desktop" 时到底做了什么（证据链）

| 行为 | ≤23H2 | 24H2 | 证据等级 |
|---|---|---|---|
| 桌面宿主成为前台窗口 | explorer 进程的一个 `WorkerW` 成为前台；`SHELLDLL_DefView` 随后被挪进该 WorkerW（Rainmeter 要等它出现） | `Progman` 自己成为前台 | [源码] `System.cpp:562-610`；Lively `IsDesktop()` 也以 `GetForegroundWindow()==WorkerW/Progman` 判定"在桌面上"（`WinDesktopCore.cs:1277-1281`） |
| 桌面宿主被抬到 Z 序上方，盖住没被最小化的窗口 | 该 WorkerW 被抬到哨兵之上 | Progman 被抬到哨兵之上 | [源码] `CheckDesktopState` 的判定方式本身；Rainmeter 文档 "-1 Bottom: The skin will **not** stay visible when showing the desktop"（Bottom 模式不重排 → 被盖住而非最小化）；Brian: "Windows just 'hides' the windows (aka skins)…until the 'Restore Desktop' function is activated"；用户 JGKle 观察 "any other action that shows a window - or even just opening the start menu - brings it back" |
| 哪些窗口被最小化 | 无官方文档说明 | 同左 | [文档] 仅有 `Shell.ToggleDesktop`（"It either hides all open windows to show the desktop or it hides the desktop by showing all open windows"）与 `Shell.MinimizeAll`（"Minimizes all of the windows on the desktop"）的描述 |
| `WS_EX_TOOLWINDOW` 是否豁免最小化 | **否**：mini-todo 固定模式已是 `WS_EX_TOOLWINDOW`，仍会被 Win+D / Win+M 最小化（本仓库 `window.rs:531-541` 注释与 200 ms 还原轮询即为此而写） | — | [源码] 本仓库 |
| `WS_MINIMIZEBOX` 是否决定最小化 | Rainmeter 皮肤（`WS_POPUP`，无 `WS_SYSMENU`/`WS_MINIMIZEBOX`）不被最小化，docs 称 On Desktop "will stay visible when showing the desktop"；SO 55849911：WPF `ResizeMode=NoResize`（去掉 min/max box）的窗口按 Win+D 时 `WindowState` 不变 | — | [推断]：三处观察一致指向"不可最小化的窗口不会被最小化"，但没有文档 → 原型 1 |
| owner 是否影响 | Rainmeter 无 owner 也不被最小化 | — | [源码]，owner 不是必要条件 |
| 24H2 之前的"漏洞" | Explorer 生成的顶层 WorkerW 是可以被第三方插入其上方的普通顶层窗口 | 不再生成顶层 WorkerW；Brian: "Microsoft has closed a loophole in their 'Show Desktop' functionality that various programs used to keep their windows shown" | [源码] issue #377 |

### 2.5 重排：`PrepareHelperWindow` / `ChangeZPos` / `GetBackmostTopWindow`

`System.cpp:456-521`（[源码]，注释原文保留）：

```cpp
// Moves the helper window to the reference position.
void System::PrepareHelperWindow(HWND desktopIconsHostWindow)
{
	SetWindowPos(c_Window, HWND_BOTTOM, 0, 0, 0, 0, ZPOS_FLAGS);  // always on bottom

	if (c_ShowDesktop && desktopIconsHostWindow)
	{
		// Set WS_EX_TOPMOST flag
		SetWindowPos(c_HelperWindow, HWND_TOPMOST, 0, 0, 0, 0, ZPOS_FLAGS);

		// Find the "backmost" topmost window
		HWND hwnd = desktopIconsHostWindow;
		while (hwnd = ::GetNextWindow(hwnd, GW_HWNDPREV))
		{
			if (GetWindowLongPtr(hwnd, GWL_EXSTYLE) & WS_EX_TOPMOST)
			{
				// Insert the helper window after the found window
				if (0 != SetWindowPos(c_HelperWindow, hwnd, 0, 0, 0, 0, ZPOS_FLAGS)) { ... return; }
			}
		}
	}
	else
	{
		// Insert the helper window to the bottom
		SetWindowPos(c_HelperWindow, HWND_BOTTOM, 0, 0, 0, 0, ZPOS_FLAGS);
	}
}
```

`Skin.cpp:987-1065`（[源码]）：

```cpp
void Skin::ChangeZPos(ZPOSITION zPos, bool all)
{
	HWND winPos = HWND_NOTOPMOST;
	...
	case ZPOSITION_ONBOTTOM:
		if (all)
		{
			if (System::GetShowDesktop())
			{
				// Insert after the system window temporarily to keep order
				winPos = System::GetWindow();
			}
			else
			{
				// Insert after the helper window
				winPos = System::GetHelperWindow();
			}
		}
		else winPos = HWND_BOTTOM;
		break;

	case ZPOSITION_NORMAL:
		if (all || !GetRainmeter().IsNormalStayDesktop()) break;
	case ZPOSITION_ONDESKTOP:
		if (System::GetShowDesktop())
		{
			winPos = System::GetHelperWindow();
			if (all) { /* Insert after the helper window */ }
			else
			{
				// Find the "backmost" topmost window
				while (winPos = ::GetNextWindow(winPos, GW_HWNDPREV))
				{
					if (GetWindowLongPtr(winPos, GWL_EXSTYLE) & WS_EX_TOPMOST)
					{
						// Insert after the found window
						if (FALSE != SetWindowPos(m_Window, winPos, 0, 0, 0, 0, ZPOS_FLAGS)) break;
					}
				}
				return;
			}
		}
		else
		{
			if (all) winPos = System::GetHelperWindow();   // Insert after the helper window
			else winPos = HWND_BOTTOM;
		}
		break;
	}
	SetWindowPos(m_Window, winPos, 0, 0, 0, 0, ZPOS_FLAGS);
}
```

`System.cpp:331-351`：

```cpp
// Returns the first window whose position is not ZPOSITION_ONDESKTOP,
// ZPOSITION_BOTTOM, or ZPOSITION_NORMAL.
HWND System::GetBackmostTopWindow()
{
	HWND winPos = c_HelperWindow;
	// Skip all ZPOSITION_ONDESKTOP, ZPOSITION_BOTTOM, and ZPOSITION_NORMAL windows
	while (winPos = ::GetNextWindow(winPos, GW_HWNDPREV)) { ... }
	return winPos;
}
```

`Skin.cpp:4628-4652`（关键：阻止点击激活把窗口抬起来）：

```cpp
LRESULT Skin::OnWindowPosChanging(UINT uMsg, WPARAM wParam, LPARAM lParam)
{
	LPWINDOWPOS wp = (LPWINDOWPOS)lParam;
	...
	if (m_State != STATE_REFRESHING)
	{
		if (m_WindowZPosition == ZPOSITION_NORMAL && GetRainmeter().IsNormalStayDesktop() && System::GetShowDesktop())
		{
			if (!(wp->flags & (SWP_NOOWNERZORDER | SWP_NOACTIVATE)))
			{
				// Set window on top of all other ZPOSITION_ONDESKTOP, ZPOSITION_BOTTOM, and ZPOSITION_NORMAL windows
				wp->hwndInsertAfter = System::GetBackmostTopWindow();
			}
		}
		else if (m_WindowZPosition == ZPOSITION_ONDESKTOP || m_WindowZPosition == ZPOSITION_ONBOTTOM)
		{
			// Do not change the z-order. This keeps the window on bottom.
			wp->flags |= SWP_NOZORDER;
		}
	}
```

`Skin.cpp:1083-1086` 注释说明了 `HWND_TOPMOST/HWND_NOTOPMOST` 语义："ChangeZPos() only makes sure the window is in the right z-order band. Since HWND_NOTOPMOST/HWND_TOPMOST do not move a window that is already in that band, raise the window explicitly. Bottom-most windows are left alone as they are meant to stay below everything else."

整体状态机（Rust 风格伪码，仅 OnDesktop 一种模式）：

```rust
// 前提：desktop_window 自身对 WM_WINDOWPOSCHANGING 追加 SWP_NOZORDER（子类化），
//       sentinel / helper 两个隐藏 WS_POPUP|WS_DISABLED|WS_EX_TOOLWINDOW 窗口同样如此
const ZPOS: SET_WINDOW_POS_FLAGS = SWP_NOMOVE | SWP_NOSIZE | SWP_NOOWNERZORDER | SWP_NOACTIVATE | SWP_NOSENDCHANGING;

fn tick(state: &mut State) {                       // 250 ms；show_desktop 期间 100 ms；EVENT_SYSTEM_FOREGROUND 时也调
    let host = desktop_icons_host();               // ≤23H2：装着 DefView 的可见 WorkerW（否则 None）；24H2：Progman
    let below = host.filter(|h| IsWindowVisible(*h).as_bool())
                    .and_then(|h| FindWindowExW(None, h, w!("MiniTodoSentinel"), None).ok());
    let show_desktop = below.is_some();
    if show_desktop == state.show_desktop { return; }
    state.show_desktop = show_desktop;

    SetWindowPos(sentinel, HWND_BOTTOM, 0,0,0,0, ZPOS);
    if show_desktop {
        SetWindowPos(helper, HWND_TOPMOST, 0,0,0,0, ZPOS);
        // 从桌面宿主往上找到第一个 WS_EX_TOPMOST 窗口，把 helper 插到它后面（= 非 topmost 带的最上方）
        let mut h = host.unwrap();
        while let Ok(prev) = GetWindow(h, GW_HWNDPREV) {
            if GetWindowLongPtrW(prev, GWL_EXSTYLE) as u32 & WS_EX_TOPMOST.0 != 0 {
                if SetWindowPos(helper, prev, 0,0,0,0, ZPOS).is_ok() { break; }
            }
            h = prev;
        }
    } else {
        SetWindowPos(helper, HWND_BOTTOM, 0,0,0,0, ZPOS);
    }
    // 目标窗口插到 helper 之后：显示桌面时 = 抬升后的桌面宿主之上、所有 topmost 之下；恢复时 = 底部
    SetWindowPos(desktop_window, helper, 0,0,0,0, ZPOS);
}
```

### 2.6 定时器间隔与 24H2 修复历史

- 间隔：`INTERVAL_SHOWDESKTOP = 250` ms（常态），`INTERVAL_RESTOREWINDOWS = 100` ms（显示桌面期间，为了尽快抓到"恢复窗口"）（`System.cpp:31-37`）。
- Rainmeter 文档 `AlwaysOnTop=-2`（On Desktop）："The skin will stay visible when showing the desktop {Win-D} and stay behind other normal application windows."；`NormalStayDesktop` 默认 1："This keeps the skins in the correct 'Z order' when the Windows 'Show Desktop' button is clicked."（[文档] https://docs.rainmeter.net/manual/settings/skin-sections/#AlwaysOnTop 、https://docs.rainmeter.net/manual/settings/rainmeter-section/#NormalStayDesktop ；源码默认值 `Rainmeter.cpp:136, 1832`）。
- 修复历史（[源码]）：
  - issue #377（2024-06-09）"On Windows 11 24H2: showing desktop will hide rainmeter widgets"，83 条评论，2025-04 锁定。
  - commit `a68e6941b1`（2024-11-26）"System: Attempt to fix Win+D issue on Windows 11 24H2 … Previously the desktop icons were children of `WorkerW`, but they are now the children of `Program` (i.e. the shell window). Inspired by rocksdanister/lively#2050" → 同日被 `8301f41533` 回滚。
  - commit `b1128fb21e`（2025-04-10，PR #413）"Fix 'Show Desktop' hiding skins on Windows 11 24H2 … We now account for the changes in the desktop WorkerW hierarchy and limit the change to recent builds of Windows 11"。核心改动：24H2+ 直接以 `Progman`（含 `SHELLDLL_DefView` 时）作为桌面宿主，`EVENT_SYSTEM_FOREGROUND` 命中 `Progman` 时调用 `CheckDesktopState`。

---

## 3. "owner = Progman" 技巧（`SetWindowLongPtr(hwnd, GWLP_HWNDPARENT, progman)`）

### 3.1 文档保证

- `GWLP_HWNDPARENT`："Sets a new owner for a top-level window." 以及 "Do not call SetWindowLongPtr with the GWLP_HWNDPARENT index to change the parent of a child window. Instead, use the SetParent function. … A window can have either a parent or an owner, or neither, but never both simultaneously."（[文档] SetWindowLongPtrW）
- 被拥有窗口的三条规则（[文档] Window Features → Owned Windows）：
  1. "An owned window is always above its owner in the z-order."
  2. "The system automatically destroys an owned window when its owner is destroyed."
  3. "An owned window is hidden when its owner is minimized."
  同页还写 "After creating an owned window, an application cannot transfer ownership of the window to another window." —— 与 `GWLP_HWNDPARENT` 的描述矛盾；实践中运行时改 owner 是主流做法，Electron 的 `NativeWindowViews::SetParentWindow` 就是这么做的（[源码] `shell/browser/native_window_views.cc:1543-1555`，注释："For do this we must NOT use the ::SetParent function, instead we must use the ::GetWindowLongPtr or ::SetWindowLongPtr functions with "nIndex" set to "GWLP_HWNDPARENT" which actually means the window owner."）。
- `SetWindowPos` 文档："When a topmost window is made non-topmost, its owners and its owned windows are also made non-topmost windows. A non-topmost window can own a topmost window, but the reverse cannot occur."；`SWP_NOOWNERZORDER`："Does not change the owner window's position in the Z order."（[文档]）

### 3.2 对 Show desktop 的意义（[推断]）

- **24H2**：被抬升的是 `Progman` 本身（§2.4）。按规则 1，owner=Progman 的窗口会跟着 Progman 一起处于其上方 → 理论上不需要定时器就能"跟着桌面一起浮上来"。未实际验证。
- **Win10 / Win11 ≤23H2**：被抬升的是装着 DefView 的 `WorkerW`，`Progman` 仍在最底部。owner=Progman 只保证"在 Progman 之上"，仍会被抬升的 WorkerW 盖住 → **在旧系统上无效**。这也是 Rainmeter 不用 owner、而是用哨兵检测 + 重排的原因（[源码] 推断自 §2.3-2.5）。
- 唯一找到的公开示例是 SO 27783313 的回答（kero，0 分，"tested on Win7-x86"）：`HWND hwndOwner = GetWindow(GetWindow(GetTopWindow(0), GW_HWNDLAST), GW_CHILD); SetWindowLong(hwndMain, GWL_HWNDPARENT, (LONG) hwndOwner);` —— 取 Z 序最底的顶层窗口（Progman）的第一个子窗口为 owner（子窗口作 owner 时系统会改用其顶层父窗口，[文档] Owned Windows）。同一回答自述第一版"remove WS_MINIMIZEBOX & add WS_EX_TOPMOST"是 "not really working"。证据等级低。

### 3.3 已知陷阱

| 陷阱 | 说明 | 证据 |
|---|---|---|
| Explorer 重启会销毁我们的主窗口 | "DestroyWindow automatically destroys the associated child or owned windows when it destroys the parent or owner window." explorer.exe 退出/崩溃 → Progman 被销毁 → 被拥有窗口一起被销毁。跨进程时还容易挂死（SO 3460542："it's easy to get hangs when the child is in another process"） | [文档] DestroyWindow；[权威] SO 3460542 |
| 输入队列附着 | Raymond Chen："Creating a cross-thread parent/child **or owner/owned** window relationship implicitly attaches the input queues of the threads which those windows belong to, and this attachment is transitive" | [权威] https://devblogs.microsoft.com/oldnewthing/20130412-00/?p=4683 |
| 激活仍会抬升窗口 | owner 关系只约束"在 owner 之上"，点击窗口激活后系统仍把它放到同类型窗口顶端（[文档] Z-Order："The system positions the active window at the top of the z-order for windows of the same type"）。Rainmeter 用 `WM_WINDOWPOSCHANGING` + `SWP_NOZORDER` 压住 | [文档] + [源码] `Skin.cpp:4647-4651` |
| Alt+Tab / 任务栏 | 与 owner 无关，由 `WS_EX_TOOLWINDOW`（不进任务栏、不进 Alt+Tab）决定 | [文档] Extended Window Styles |
| DWM | 顶层窗口不变，DWM 合成不受 owner 影响（Rainmeter 皮肤即为顶层 layered 窗口且透明正常） | [源码] 类比 |
| 与 tao 的冲突 | tao 只在创建时支持 owner（`with_owner_window` / Tauri `owner_raw`，tao `platform/windows.rs:267-277`，Tauri `webview_window.rs:761`）；运行时改 `GWLP_HWNDPARENT` tao 不感知，但 tao 也不会覆写它（`set_window_flags` 只重写 `GWL_STYLE`/`GWL_EXSTYLE`，`window_state.rs:437-440`） | [源码] |

---

## 4. `SetParent` 跨进程

### 4.1 文档层面（[文档] SetParent）

- "For compatibility reasons, SetParent does not modify the WS_CHILD or WS_POPUP window styles of the window whose parent is being changed. … if hWndNewParent is not NULL and the window was previously a child of the desktop, you should clear the WS_POPUP style and set the WS_CHILD style before calling SetParent."
- "When you change the parent of a window, you should synchronize the UISTATE of both windows. For more information, see WM_CHANGEUISTATE and WM_UPDATEUISTATE."
- DPI 感知不一致时："Unexpected behavior or errors may occur if hWndNewParent and hWndChild are running in different DPI awareness modes." 表格：`SetParent (Cross-Proc)` 在 Windows 10 1703+ 的结果是 **"Forced reset (of child window's process)"** —— 即我们整个进程的 DPI 感知上下文可能被强制重置。tao 进程默认 `DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2`（tao `dpi.rs:20-29`），explorer.exe 的上下文需在原型中用 `GetWindowDpiAwarenessContext(progman)` + `AreDpiAwarenessContextsEqual` 核对（[未验证]）。
- 当前文档已不再写"输入队列附着"，该说明来自 Raymond Chen（§3.3）与 `AttachThreadInput` 文档："By using the AttachThreadInput function, a thread can share its input states (such as keyboard states and the current focus window) with another thread."

### 4.2 权威解读

- Raymond Chen（2013）："Yes, it is technically legal. It is also technically legal to juggle chainsaws. … some window messages are blocked between processes. … things will definitely stop working if you change that other window from a top-level window to a child window."
- Adrian McCarthy（SO 3460542，37 分采纳）："when you set up the parent/child relationship among windows in different threads, Windows attaches those input queues together, forcing the message processing to be synchronous … a hang in the processing for one window effectively hangs the other process. … If possible, disconnect the parent-child relationship before destroying either window."
- SO 75060992（WinForms 宿主 + 跨进程 WPF 子窗口）：快速打字时 `WM_KEYDOWN` 到达但 `WM_CHAR` 丢失；去掉 `WS_CHILD` 问题消失但引入其他问题 —— 跨进程 child 的键盘输入不可靠的实例（[源码]）。

### 4.3 WebView2 在被 reparent 到 explorer 的窗口下时的键盘焦点

- wry 的 WebView2 宿主方式是 **HWND hosting**：`env.CreateCoreWebView2Controller(hwnd, &handler)`（wry `src/webview2/mod.rs:404-406`），不是 visual/composition hosting。
- 键盘焦点链：宿主 HWND 收到 `WM_SETFOCUS` → wry 子类化过程调用 `controller.MoveFocus(COREWEBVIEW2_MOVE_FOCUS_REASON_PROGRAMMATIC)`（`mod.rs:1237-1240`）；子 webview 模式下 `WM_SETFOCUS` → `SetFocus(第一个子窗口)`（`mod.rs:191-197`）。也就是说键盘能不能进 WebView2，取决于**我们的 HWND 能否拿到 `WM_SETFOCUS`**。作为 explorer.exe 顶层窗口的子窗口，点击时被激活的顶层窗口是 Progman/WorkerW（explorer 线程），焦点是否会派给我们的子窗口只能靠隐式附着的输入队列决定 —— 没有任何项目证明这条路可行（[未验证]）。
- Lively 的做法是根本不依赖焦点：全局 RawInput（`RIDEV_EXINPUTSINK`，`RawInputMsgWindow.xaml.cs:40-50`，注释 "ExInputSink flag makes it work even when not in foreground and async"）捕获鼠标/键盘 → 仅当 `IsDesktop()`（前台是原始 WorkerW 或 Progman，`WinDesktopCore.cs:1277-1281`）时 → `PostMessageW` 把 `WM_MOUSEMOVE/WM_LBUTTONDOWN/WM_LBUTTONUP/WM_RBUTTON*` 和 `WM_KEYDOWN/WM_KEYUP` 投递给壁纸的 `InputHandle`（`InputUtil.cs:36-72`；键盘转发注释 "context code; Note: Alt key combos wont't work"）。对 WebView2 壁纸，`InputHandle` 是 `Chrome_WidgetWin_0` 下的 `Chrome_WidgetWin_1` 子窗口（`WebWebView2.cs:271-284`）。
- ScreenPlay 用 `SetWindowsHookEx(WH_MOUSE_LL / WH_KEYBOARD_LL)` 低级钩子，把事件转成 Qt 的 `QMouseEvent`（`windowsintegration.cpp:524-546`；`winwindow.cpp:72-134`）。
- 上述转发方案都是"壁纸在图标之下、真实输入被 DefView 吃掉"的场景。方案 A（在图标之上）鼠标可能直接命中我们的子窗口（[推断]），但键盘 / IME 仍是问题。

### 4.4 与 tao 的冲突（[源码]）

- tao `WindowFlags::to_window_styles()` 生成的样式恒含 `WS_CAPTION | WS_CLIPSIBLINGS | WS_SYSMENU`，只有 `WindowFlags::CHILD`（仅创建时 `with_parent_window` / Tauri `parent_raw` 才会设）才会加 `WS_CHILD`（`window_state.rs:241-270`）。
- `set_window_flags` 在任何 flag 变化时整体 `SetWindowLongW(GWL_STYLE)` + `SetWindowLongW(GWL_EXSTYLE)` + `SWP_FRAMECHANGED`（`window_state.rs:425-450`）。`set_visible`、`set_resizable`、`set_always_on_top`、`set_always_on_bottom`、`set_minimizable` 等全部走这条路。手工 `SetParent` + `WS_CHILD` 会在下一次 tao flag 变更时被抹掉。本仓库 `window.rs:858-866, 894-898, 953` 已为 `WS_EX_TOOLWINDOW` 记录了同类问题及绕行方式。
- Tauri 2.9.5 提供 `WebviewWindowBuilder::parent_raw(HWND)` / `owner_raw(HWND)`（`tauri-2.9.5/src/webview/webview_window.rs:761-783`）：若走 A/C，应在**创建时**就以 Progman/WorkerW 为 parent，让 tao 自己持有 `CHILD` flag；代价是"桌面模式 ⇄ 普通模式"切换需要销毁重建窗口。

---

## 5. 检测 Explorer 重启并重新挂接

| 机制 | 说明 | 谁在用 | 证据 |
|---|---|---|---|
| `TaskbarCreated` 注册消息 | "When the taskbar is created, it registers a message with the TaskbarCreated string and then broadcasts this message to **all top-level windows**." 另注："On Windows 10, the taskbar also broadcasts this message when the DPI of the primary display changes." | Lively：`WndProc_TaskbarCreated` 收到后取 `Shell_TrayWnd` 所属 PID，与上次比较，PID 变了才认定 Explorer 崩溃（注释 "Detect explorer crash because otherwise dpi change also sends WM_TASKBARCREATED"），30 s 内多次重启则弹错；随后 `ResetWallpaperAsync()` → 关闭全部壁纸 → 重新 `SetupDesktopLayer()`（失败再等 500 ms 重试）→ 重新设置（`WinDesktopCore.cs:1075-1100, 556-585, 1283-1287`） | [文档] Taskbar；[源码] |
| `SetWinEventHook(EVENT_OBJECT_DESTROY)` 钉在 WorkerW 线程 | WorkerW 被销毁时立即得到通知；24H2 分支重新定位层级并用 `SetWindowPos(item, shellDLL_DefView, …)` 重排 | Lively `WinDesktopCore.cs:101-119, 224-255` | [源码] |
| 会话解锁后 `IsWindow(workerW)` | issue #802 的补丁：解锁后句柄失效则重置 | Lively `WinDesktopCore.cs:1102-1130` | [源码] |
| 每 250 ms 重取 `GetShellWindow()` + `IsWindow(c_DefView)` + `GetAncestor` 校验 | 不依赖 `TaskbarCreated`；皮肤是顶层窗口，Explorer 重启后不需要"重新挂接"，只需重新找宿主 | Rainmeter `System.cpp:220-245, 290-329` | [源码] |
| Wallpaper Engine | 闭源，无法核实 | — | [未验证] |

对本任务的推论（[推断]）：
- 方案 B：窗口是顶层，Explorer 重启时什么都不会坏；只需像 Rainmeter 一样每次 tick 重新取宿主句柄。若额外设了 owner=Progman，必须在 Progman 销毁前清掉 owner，否则主窗口被连带销毁（§3.3）—— 但 explorer 崩溃时没有"之前"，只能靠 `EVENT_OBJECT_DESTROY` 钩子抢在前面，可靠性存疑。
- 方案 A/C：子窗口随 Progman/WorkerW 一起被销毁（[文档] DestroyWindow），需要像 Lively 一样整个重建；且**子窗口收不到广播给顶层窗口的 `TaskbarCreated`**，需要另建一个隐藏顶层/消息窗口接收（Lively 用独立的 `WndProcMsgWindow`）。

---

## 6. 坐标处理

| 问题 | 结论 | 证据 |
|---|---|---|
| 子窗口的 `SetWindowPos` / `MoveWindow` 坐标相对于什么 | "For a top-level window, the position and dimensions are relative to the upper-left corner of the screen. For a child window, they are relative to the upper-left corner of the parent window's client area." | [文档] MoveWindow；SetWindowPos 的 X/Y 亦标注 "in client coordinates" |
| `GetWindowRect` 是否仍返回屏幕坐标 | 是："The dimensions are given in screen coordinates that are relative to the upper-left corner of the screen." 对子窗口同样成立 | [文档] GetWindowRect |
| 父窗口原点是否等于虚拟屏原点 `(SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN)` | 无文档。Lively 的 span 模式直接把壁纸放在 `(0,0)`、尺寸取 `GetWindowRect(workerW)` 的宽高（`WinDesktopCore.cs:531-551`），说明 WorkerW 客户区覆盖整个虚拟屏且无边框；但 Lively 的 per-screen 模式并不假设原点，而是先把窗口按屏幕坐标摆好，再 `MapWindowPoints(handle, workerW, ref rect, 2)` 转成 WorkerW 客户区坐标，`SetParent` 后用该坐标 `SetWindowPos(..., SWP_NOZORDER)`（`WinDesktopCore.cs:492-526`）。ScreenPlay 则用 `newX = oldRect.left - parentRect.left`（`GetWindowRect(worker)`）并乘以 DPI 比例（`windowsintegration.cpp:296-317`） | [源码]；"原点 == 虚拟屏原点"本身 [未验证]，实现时一律用 `MapWindowPoints` 或 `GetWindowRect(parent)` 求偏移 |
| DPI | ScreenPlay 注释："The new position should be relative to the WorkerW window's coordinates. Adjust the position based on the ratio of the window's DPI scale factor to the target monitor's DPI scale factor."（`windowsintegration.cpp:210-230`）；Rainmeter 的 `PositioningHelper` 用 DPI-unaware 作用域创建以复现旧坐标映射（`System.cpp:76-93`） | [源码] |
| tao 侧 | tao 的 `outer_position` 用 `GetWindowRect`（屏幕坐标），`set_position` 直接 `SetWindowPos`（按顶层窗口语义传屏幕坐标）；变成子窗口后两者语义不再一致，`window.rs` 里贴边隐藏逻辑（`tick_auto_hide`）会错位 | [推断]，见原型 6 |

---

## 7. 三方案对比（针对 mini-todo：Tauri 2.x / tao 0.34.5 / wry 0.53.5 / WebView2，主窗口透明、无边框）

| 维度 | (A) `SetParent` 进图标宿主（Progman 24H2 / WorkerW ≤23H2），Z 序在 `SHELLDLL_DefView` 之上 | (B) 保持顶层 + 去 `WS_MINIMIZEBOX` + Rainmeter 式检测/重排（owner=Progman 可选） | (C) `SetParent` 进壁纸 `WorkerW`（图标之下） |
|---|---|---|---|
| Win+D / 显示桌面 免疫 | 子窗口随父窗口一起被抬升（[文档] "When a window comes to the top of z-order, so do its child windows"）；子窗口不会被单独最小化（[推断]）。实际放在图标之上的先例：SO 78169263 Edit 2（GLFW 窗口，2024）、SO 2619331（Win7），但都没报告 Win+D 结果 → **[推断]** | Rainmeter 十余年实践，24H2 已修（[源码] PR #413）。前提"不被最小化"需去掉 `WS_MINIMIZEBOX`（[推断]，原型 1）。24H2 上 owner=Progman 可能免去定时器（[推断]），≤23H2 无效 | 同 A（Lively/ScreenPlay/Wallpaper Engine 的日常状态，[源码]）；但在图标之下不满足需求 |
| Win+M（最小化全部） | 子窗口不参与（[推断]） | 取决于 `WS_MINIMIZEBOX`（[推断]，原型 1） | 同 A |
| 透明（DWM 逐像素） | **高风险**。tao 透明 = 顶层窗口 `DwmEnableBlurBehindWindow(空区域)` + WebView2 透明背景（tao `window.rs:1284-1296`；wry `mod.rs:126-130, 448-450`），依赖 DWM 对顶层窗口重定向面的合成；成为他进程顶层窗口的子窗口后没有独立合成面。微软 24H2 指南要求的子窗口是 `WS_EX_LAYERED` + `SetLayeredWindowAttributes(0xFF)`（不透明）；`WS_EX_LAYERED` 子窗口 Win8+ 才支持且只有整体 alpha / 色键（[文档] SetLayeredWindowAttributes），逐像素 alpha 需 `UpdateLayeredWindow` 自己喂位图，与 WebView2 HWND 渲染不兼容 → [推断] 大概率丢失透明 | **无变化**：仍是顶层窗口，DWM 合成照旧；Rainmeter 皮肤即为顶层 layered 透明窗口 | 同 A |
| 鼠标输入 | 位于图标之上，鼠标应直接命中（[推断]）；但激活会落到 explorer 线程 | 正常 | 被 DefView 遮挡，须像 Lively/ScreenPlay 一样全局捕获后 `PostMessage` 转发（[源码]） |
| 键盘 / IME / 焦点 | **高风险**：跨进程 parent/child 隐式附着输入队列（Raymond Chen）；wry 焦点链依赖宿主 HWND 收到 `WM_SETFOCUS`（wry `mod.rs:1237-1240`）；SO 75060992 报告丢键；Lively 干脆不依赖焦点 | 正常（与现在固定模式相同） | 同 A，且 Lively 注明 Alt 组合键不可转发 |
| Explorer 重启 | 子窗口随 Progman/WorkerW 一起被销毁（[文档] DestroyWindow）→ 需重建 Tauri 窗口；收不到 `TaskbarCreated`（只广播给顶层窗口），需另建接收窗口 | 顶层窗口不受影响；每 tick 重取 `GetShellWindow()` 即可（Rainmeter）。若设 owner=Progman 则同样会被销毁，需 `EVENT_OBJECT_DESTROY` 钩子抢先清 owner（可靠性存疑） | 同 A（Lively：PID 比较 + 全量重建） |
| 与 tao/wry 的兼容性 | tao 任何 flag 变更重写 `GWL_STYLE` 抹掉 `WS_CHILD`；除非创建时 `parent_raw`（模式切换需重建窗口）；DPI 感知不一致会"Forced reset (of child window's process)"（[文档] SetParent） | 只需：`minimizable:false`、子类化拦 `WM_WINDOWPOSCHANGING`（tao 的 `with_msg_hook` 是 `GetMessage` 级钩子，拦不到 `SendMessage` 直达的 `WM_WINDOWPOSCHANGING`，需 `SetWindowSubclass`）、一个 200-250 ms tick（本仓库已有 200 ms 固定模式线程 `lib.rs:256-272`） | 同 A |
| 24H2 vs ≤23H2 分支 | 两套宿主定位逻辑（§1.5）；≤23H2 宿主 WorkerW 会在 raised/un-raised 之间动态出现/消失 | 两套宿主定位逻辑（Rainmeter 已写好，`System.cpp:266-329`） | 两套 + `0x052C` |
| 复杂度 | 高（reparent + 透明重做 + 输入转发 + 重建） | 中（状态机 + 子类化 + 两个隐藏哨兵窗口） | 高，且不满足"在图标之上" |
| 结论 | 不推荐（透明与键盘两项与需求冲突） | **推荐** | 不适用 |

---

## 8. 本项目相关的既有事实（内部）

- `pc/src-tauri/tauri.conf.json` 主窗口：`decorations:false, transparent:true, shadow:false, alwaysOnTop:false, resizable:true`，未设置 `minimizable` → tao 默认 `MINIMIZABLE` 置位，样式含 `WS_MINIMIZEBOX`（tao `window_state.rs:252-254`）。
- 固定模式：`apply_fixed_ex_style` 加 `WS_EX_TOOLWINDOW`、去 `WS_EX_APPWINDOW`（`window.rs:831-856`）；200 ms 后台线程在固定模式下调用 `restore_if_minimized`（被 Win+D / Win+M 最小化就立刻 `unminimize + show`）与 `tick_auto_hide`（`lib.rs:256-272`；`window.rs:531-541`）。这与 Rainmeter 用户 JGKle 的"检测后立刻恢复"思路相同，Brian 指出其副作用：之后再按 Win+D 会变成"隐藏/恢复本窗口"而不是恢复其他窗口（issue #377，2024-12-31 两条评论）。
- 已有 DWM 调用：`DwmSetWindowAttribute(DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND)`（`lib.rs:37-58`），顶层窗口属性，子窗口化后无意义。
- `windows` 0.58 crate 当前 features：`Win32_Foundation, Win32_UI_WindowsAndMessaging, Win32_Graphics_Dwm, Win32_Graphics_DirectWrite`（`Cargo.toml:45`）。所需 API 归属（本地 registry 核对）：
  - `Win32_UI_WindowsAndMessaging`（已启用）：`SetParent`、`SetWindowLongPtrW`/`GetWindowLongPtrW`、`GWLP_HWNDPARENT`、`FindWindowW`/`FindWindowExW`、`EnumWindows`/`EnumChildWindows`、`GetWindow`/`GetTopWindow`、`GetAncestor`、`GetShellWindow`、`SendMessageTimeoutW`、`RegisterWindowMessageW`、`IsWindow`/`IsWindowVisible`、`GetWindowThreadProcessId`、`GetSystemMetrics`、`SetLayeredWindowAttributes`、`HWND_BOTTOM`、`EVENT_OBJECT_DESTROY`/`EVENT_SYSTEM_FOREGROUND`/`WINEVENT_OUTOFCONTEXT` 常量
  - `Win32_UI_Accessibility`（需新增）：`SetWinEventHook`
  - `Win32_Graphics_Gdi`（需新增）：`MapWindowPoints`
  - `Win32_UI_HiDpi`（需新增）：`GetWindowDpiAwarenessContext`、`AreDpiAwarenessContextsEqual`、`GetDpiForWindow`
  - `Win32_UI_Shell`（需新增）：`SetWindowSubclass`（拦 `WM_WINDOWPOSCHANGING`）

---

## 未验证 / 需要原型验证

1. **`WS_MINIMIZEBOX` 与 Win+D / Win+M**（最便宜、最关键）：把 `tauri.conf.json` 主窗口设 `"minimizable": false`（tao 去掉 `WS_MINIMIZEBOX`），在 Win10 / Win11 23H2 / Win11 24H2 上分别按 Win+D、Win+M、点任务栏"显示桌面"，观察 `IsIconic(hwnd)`。若不再被最小化，则方案 B 的第一半成立；若仍被最小化，再试去掉 `WS_SYSMENU`/`WS_CAPTION`（tao 无对应 flag，只能子类化后手改并防止被 tao 重写）。
2. **Explorer 抬升行为复核**：用一个隐藏的 `HWND_BOTTOM` 哨兵窗口 + Spy++，在三种系统上按 Win+D，确认 (a) `GetForegroundWindow()` 变为 WorkerW（≤23H2）/ Progman（24H2）；(b) `FindWindowEx(nullptr, 宿主, 哨兵类名, ...)` 能找到哨兵；(c) 恢复窗口后宿主回到底部。这直接验证 Rainmeter 算法在当前补丁级别的 24H2 上仍有效（Rainmeter 4.5.22+ 用户反馈已通过，但仍应自测）。
3. **`WM_WINDOWPOSCHANGING` 子类化**：`SetWindowSubclass` 拦截并追加 `SWP_NOZORDER` 后，点击主窗口 / 拖动子任务 / 打开编辑窗口时主窗口是否仍留在底层；确认 tao 自己的 `SetWindowPos`（`set_position`、`set_size`、`apply_diff` 里的 `HWND_TOPMOST/NOTOPMOST/BOTTOM`）在加了 `SWP_NOZORDER` 后不出错（注意 Rainmeter 自己的重排调用带 `SWP_NOSENDCHANGING` 绕过此拦截）。
4. **owner=Progman（仅 24H2）**：`SetWindowLongPtrW(hwnd, GWLP_HWNDPARENT, progman)` 后 Win+D 是否无需 tick 即保持可见；是否引入输入延迟/卡顿（输入队列附着）；模拟 `taskkill /f /im explorer.exe` 观察主窗口是否被销毁、进程是否挂死。
5. **DPI 感知上下文**：`GetWindowDpiAwarenessContext(progman)` 与本进程是否 `AreDpiAwarenessContextsEqual`；若走 A/C，跨进程 `SetParent` 是否触发 "Forced reset (of child window's process)"。
6. **方案 A/C 的透明性**（若坚持验证）：把一个最小 tao+wry 透明窗口 `SetParent` 到 Progman 下、Z 序放 DefView 之上，看 WebView2 的透明像素显示为什么（黑 / 上次内容 / 壁纸）；再试 `WS_EX_LAYERED + SetLayeredWindowAttributes(alpha)` 与 `WS_EX_NOREDIRECTIONBITMAP`。
7. **方案 A/C 的键盘**：同上原型中点击 WebView2 输入框，观察 `WM_SETFOCUS` 是否到达宿主 HWND、`WM_CHAR` 是否丢失、IME 候选窗是否出现。
8. **`0x052C` 参数差异**：`(0,0)` 与 `(0xD,0x1)` 在 24H2 上的行为是否相同（仅方案 C 需要）。
9. **Aero Peek**：是否需要像 Rainmeter 一样设置 `DWMWA_EXCLUDED_FROM_PEEK`，使鼠标悬停任务栏"显示桌面"按钮时窗口不被 Peek 隐藏（Rainmeter `Skin.cpp:378-382`，未查阅 DWM 文档）。
10. **`TaskbarCreated` 在 DPI 变化时的误报**：本窗口若监听该消息用于重定位宿主，需像 Lively 一样比较 `Shell_TrayWnd` 的 PID。
11. **Wallpaper Engine 的具体做法**：闭源，本文所有关于它的说法均来自第三方（Lively issue #2074 中用户报告 24H2 上 Wallpaper Engine 2.5.0.28 同样失效），[未验证]。
12. **未找到一手文档**：Windows 对 "Show desktop" 最小化哪些窗口的官方说明（Old New Thing 站内搜索与搜索引擎在本次会话中被限流，未能检索到相关文章）。

---

## 来源清单

### 源码（GitHub）
- Lively Wallpaper（分支 `core-separation`）
  - https://github.com/rocksdanister/lively/blob/core-separation/src/Lively/Lively/Core/WinDesktopCore.cs
  - https://github.com/rocksdanister/lively/blob/core-separation/src/Lively/Lively.Common/Helpers/Shell/DesktopUtil.cs
  - https://github.com/rocksdanister/lively/blob/core-separation/src/Lively/Lively.Common/Helpers/WindowUtil.cs
  - https://github.com/rocksdanister/lively/blob/core-separation/src/Lively/Lively.Common/Helpers/InputUtil.cs
  - https://github.com/rocksdanister/lively/blob/core-separation/src/Lively/Lively/Views/WindowMsg/RawInputMsgWindow.xaml.cs
  - https://github.com/rocksdanister/lively/blob/core-separation/src/Lively/Lively/Core/Wallpapers/WebWebView2.cs
  - issue #2074（含微软官方回复与 24H2 讨论）: https://github.com/rocksdanister/lively/issues/2074
  - PR #2050: https://github.com/rocksdanister/lively/pull/2050
- Rainmeter（`master`）
  - https://github.com/rainmeter/rainmeter/blob/master/Library/System.cpp
  - https://github.com/rainmeter/rainmeter/blob/master/Library/Skin.cpp
  - https://github.com/rainmeter/rainmeter/blob/master/Library/Rainmeter.cpp
  - 2015 旧版 System.cpp（注释对照）: https://github.com/rainmeter/rainmeter/blob/1b054fc4853fea6724d5226da519a44a2fe31237/Library/System.cpp
  - issue #377: https://github.com/rainmeter/rainmeter/issues/377
  - PR #413 / commit b1128fb21e: https://github.com/rainmeter/rainmeter/commit/b1128fb21ee21a0370997f3fabf050915ff911a1
  - commit a68e6941b1（已回滚）: https://github.com/rainmeter/rainmeter/commit/a68e6941b16dd1ac073e6e8883576d11e0c52900
  - 文档 AlwaysOnTop: https://docs.rainmeter.net/manual/settings/skin-sections/#AlwaysOnTop
  - 文档 NormalStayDesktop: https://docs.rainmeter.net/manual/settings/rainmeter-section/#NormalStayDesktop
- ScreenPlay（`master`）
  - https://github.com/kelteseth/ScreenPlay/blob/master/ScreenPlayWallpaper/src/windowsintegration.cpp
  - https://github.com/kelteseth/ScreenPlay/blob/master/ScreenPlayWallpaper/src/winwindow.cpp
- Electron `SetParentWindow`（GWLP_HWNDPARENT 用法）: https://github.com/electron/electron/blob/main/shell/browser/native_window_views.cc
- CodeProject "Draw Behind Desktop Icons in Windows 8+"（Web Archive）: https://web.archive.org/web/2024/https://www.codeproject.com/Articles/856020/Draw-Behind-Desktop-Icons-in-Windows-plus

### Microsoft Learn
- SetParent: https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setparent
- SetWindowLongPtrW（GWLP_HWNDPARENT）: https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setwindowlongptrw
- Window Features（Owned Windows / Z-Order）: https://learn.microsoft.com/en-us/windows/win32/winmsg/window-features
- SetWindowPos: https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setwindowpos
- MoveWindow（子窗口坐标语义）: https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-movewindow
- GetWindowRect: https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getwindowrect
- MapWindowPoints: https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-mapwindowpoints
- GetShellWindow: https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getshellwindow
- GetSystemMetrics（SM_XVIRTUALSCREEN）: https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getsystemmetrics
- DestroyWindow: https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-destroywindow
- AttachThreadInput: https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-attachthreadinput
- SetLayeredWindowAttributes: https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setlayeredwindowattributes
- Extended Window Styles: https://learn.microsoft.com/en-us/windows/win32/winmsg/extended-window-styles
- Window Styles: https://learn.microsoft.com/en-us/windows/win32/winmsg/window-styles
- Taskbar（TaskbarCreated）: https://learn.microsoft.com/en-us/windows/win32/shell/taskbar
- Shell.ToggleDesktop: https://learn.microsoft.com/en-us/windows/win32/shell/shell-toggledesktop
- Shell.MinimizeAll: https://learn.microsoft.com/en-us/windows/win32/shell/shell-minimizeall

### 权威博客 / StackOverflow
- Raymond Chen, "Is it legal to have a cross-process parent/child or owner/owned window relationship?"（2013-04-12）: https://devblogs.microsoft.com/oldnewthing/20130412-00/?p=4683
- SO 3459874 "Good or evil - SetParent() win32 API between different processes"（Adrian McCarthy 采纳答）: https://stackoverflow.com/a/3460542
- SO 22131449（David Heffernan）: https://stackoverflow.com/a/22131525
- SO 75060992 "Keyboard input lost when using cross-process parent/child windows": https://stackoverflow.com/questions/75060992
- SO 74734565 "How to pin my window to the desktop (like rainmeter)…"（Anders 采纳答）: https://stackoverflow.com/a/74738066
- SO 27783313 "Make a window a part of the desktop"（GWL_HWNDPARENT 技巧）: https://stackoverflow.com/questions/27783313
- SO 55849911 "How can I detect a hidden Desktop?"（NoResize 与 Win+D）: https://stackoverflow.com/questions/55849911
- SO 2619331 "DirectX Desktop": https://stackoverflow.com/questions/2619331
- SO 78169263 "Prevent drawing over a child window under WorkerW…": https://stackoverflow.com/questions/78169263

### 本地依赖源码（cargo registry）
- tao 0.34.5: `src/platform_impl/windows/window.rs`（transparent 1284-1296；set_always_on_bottom 840-850）、`window_state.rs`（to_window_styles 241-300；apply_diff 338-372；set_window_flags 425-450）、`platform/windows.rs`（with_msg_hook 60-90；with_parent_window/with_owner_window 260-277）、`dpi.rs:20-29`
- wry 0.53.5: `src/webview2/mod.rs`（transparent 126-130, 385-401, 438-450；HWND hosting 404-406；WM_SETFOCUS 183-200, 1237-1240）
- tauri 2.9.5: `src/webview/webview_window.rs:732-783`（parent / owner / owner_raw / parent_raw）
- windows 0.58.0: `src/Windows/Win32/UI/WindowsAndMessaging/mod.rs`、`UI/Accessibility/mod.rs`、`Graphics/Gdi/mod.rs`、`UI/HiDpi/mod.rs`

### 本仓库
- `pc/src-tauri/tauri.conf.json`（主窗口配置）
- `pc/src-tauri/src/lib.rs:37-58, 256-272`
- `pc/src-tauri/src/commands/window.rs:531-541, 811-910`
- `pc/src-tauri/Cargo.toml:44-47`
