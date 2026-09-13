# Research: tao/wry 在 Windows 上的透明实现，以及 SetParent 嵌入桌面后透明能否保留

- **Query**: Tauri 2（tao 0.34.5 + wry 0.53.5 + WebView2）在 Windows 上如何实现 `transparent: true`；把主 HWND 用 `SetParent` 变成 explorer 桌面窗口（`WorkerW` / `Progman`）的 `WS_CHILD` 子窗口后，逐像素透明是否还能保留
- **Scope**: mixed（本地 crate 源码 + 项目源码 + 本机窗口树实测 + Microsoft Learn / GitHub 先例）
- **Date**: 2026-09-07
- **锁定版本**（`pc/src-tauri/Cargo.lock`）: tauri 2.9.5、tauri-runtime-wry 2.9.3、tao 0.34.5、wry 0.53.5、webview2-com 0.38.2
- **研究机器**: Windows 11 Home 10.0.26200（≥ 24H2 "raised desktop" 桌面结构，见 §3.6）
- crate 源码根目录: `%USERPROFILE%\.cargo\registry\src\index.crates.io-1949cf8c6b5b557f\`，下文 `tao/…` 指 `tao-0.34.5/src/platform_impl/windows/…`，`wry/…` 指 `wry-0.53.5/src/webview2/…`

---

## 结论与建议

### (a) 朴素 `SetParent`（保持 `transparent: true` 不动）最可能的结果

1. **tao 的透明只是创建时一次性的 DWM 注册，且只对顶层窗口有效。** tao 在 `CreateWindowExW` 之后调用一次 `DwmEnableBlurBehindWindow(hwnd, 空 region)`（`tao/window.rs:1283-1297`），此后 `WindowFlags::TRANSPARENT` 位再也不被读取（全 crate 只有 `window.rs:1136` 一处 set）。Microsoft 文档明确写着该函数 "can be called only on top-level windows. An error occurs when this function is called on other window types"。窗口被 `SetParent` 变为子窗口后，DWM 对它的"用重定向位图 alpha 合成"这条通路失去了文档保证。
2. **WebView2 的画面本身大概率仍会出现**（它渲染在自己的跨进程 `Intermediate D3D Window` 子窗口上，`WS_EX_LAYERED|WS_EX_NOREDIRECTIONBITMAP`，DirectComposition 目标可以绑定子窗口），**但 CSS 透明区域最可能显示为黑色/不透明**，而不是透出桌面。证据：
   - tauri-plugin-wallpaper 的实测注释：Tauri 窗口被 `SetParent` 到 WorkerW 后，WebView 没覆盖到的客户区 inset 区域"would leave a black stripe"（`src/platform/windows/attacher.rs:124-129`）——说明在子窗口状态下，宿主 HWND 自己没有内容的像素呈现为**黑**而不是透明。
   - tauri #15947：blur-behind hack 一旦与 DWM 合成树重建竞争，透明区就变黑，作者的解释是"an invalid/uninitialized surface"。
   - electron-as-wallpaper 专门提供 `transparent` 选项，在 `SetParent` 后再给窗口加 `WS_EX_LAYERED` + `SetLayeredWindowAttributes(0, 255, LWA_ALPHA)`（`src/window.rs:14-32`）——这个选项存在本身就说明 Chromium 窗口被重挂后透明会丢、需要额外处理。
   - 唯一可能"意外成功"的情形：24H2+ 上 Progman 是 `WS_EX_NOREDIRECTIONBITMAP`（本机实测，§3.6），顶层窗口根本没有 GDI 表面，非分层子窗口无处可画，理论上 WebView2 的 DComp 内容可能直接与桌面各层合成。但这与上面 tauri-plugin-wallpaper 的"黑条"观察相悖，只能靠原型验证。
3. **样式会被 tao 悄悄改回。** `SetParent` 不会自己改 `WS_CHILD`/`WS_POPUP`（文档），必须手动设 `WS_CHILD`；而 tao 任何 flag 变化（`set_visible`/`set_resizable`/`set_always_on_top`/`set_minimized`/…）都会在 `WindowFlags::apply_diff` 里**整体覆写** `GWL_STYLE`/`GWL_EXSTYLE`（`tao/window_state.rs:426-441`），`WS_CHILD`、任何手动加的 `WS_EX_LAYERED`/`WS_EX_NOREDIRECTIONBITMAP` 都会被抹掉，变成"有 parent 但没有 `WS_CHILD` 位"的非法状态。项目已经为 `WS_EX_TOOLWINDOW` 踩过同一个坑（`pc/src-tauri/src/commands/window.rs:858-887`）。
4. **圆角会残留。** 项目在 `lib.rs:38-59` 设了 `DWMWCP_ROUND`；tauri-plugin-desktop-underlay #85 实测：重挂到 WorkerW 后圆角仍在、四角露出空隙，两个插件都在 `SetParent` 后补 `DWMWCP_DONOTROUND`。
5. **坐标语义会错位。** `outer_position()` 用 `GetWindowRect` 返回屏幕坐标，`set_outer_position()` 用 `SetWindowPos(x, y)`，而子窗口的 `SetWindowPos` 坐标是相对父窗口客户区（§5）。本机 Progman 客户区原点在 `(-2560, -240)`（多显示器虚拟屏左上），单显示器机器上碰巧为 `(0,0)` 会掩盖这个 bug。
6. **DPI 变化不再通知 tao。** 子窗口只收 `WM_DPICHANGED_BEFOREPARENT/AFTERPARENT`，tao 只处理 `WM_DPICHANGED`（`tao/event_loop.rs:1915`），`scale_factor()` 会陈旧（§5.4）。
7. **本机实测旁证**：研究期间曾观察到 debug 版 mini-todo 主窗口（hwnd `0x6004A`）处于 Progman 第一个子窗口位置（在 `SHELLDLL_DefView` 之上，即"图标之上"），style `0x54CB0000`（`WS_CHILD|WS_VISIBLE|WS_CLIPSIBLINGS|WS_CAPTION|WS_SYSMENU|WS_MIN/MAXIMIZEBOX`），ex `0x190`（`WS_EX_TOOLWINDOW|WS_EX_WINDOWEDGE|WS_EX_ACCEPTFILES`），**没有** `WS_EX_LAYERED`/`WS_EX_NOREDIRECTIONBITMAP`；数分钟后它又回到顶层。说明有并行原型正在做朴素 `SetParent` + 手动 `WS_CHILD`。本研究没有截图，无法断言当时的视觉效果。

### (b) 获得逐像素透明的候选技术（按可信度排序）

| 序 | 技术 | 可信度 | 关键证据 | 主要代价 / 风险 |
|---|---|---|---|---|
| 1 | **不重挂，保持顶层窗口，用 Z 序模拟"桌面模式"**：`WS_EX_TOOLWINDOW`（固定模式已有）+ 常驻 `HWND_BOTTOM`（tao `always_on_bottom` 会在 `WM_WINDOWPOSCHANGING` 里强制 `hwndInsertAfter = HWND_BOTTOM`，`tao/event_loop.rs:1229-1233`；或直接 Win32 `SetWindowPos`）+ 对抗 Win+D（子类化 `WM_WINDOWPOSCHANGING` 拦截移到 `(-32000,-32000)` 的请求，tauri-plugin-wallpaper `pinner.rs`；或 Rainmeter 的 helper-window 探测 Show Desktop 后重排 Z 序） | **最高**。透明通路完全不变（顶层 + blur-behind），"在图标之上"天然满足（顶层窗口永远在 Progman 之上） | Rainmeter 的 "On desktop" 就是这样做的：皮肤是顶层 `WS_EX_LAYERED|WS_EX_TOOLWINDOW` 窗口，Z 序相对桌面图标宿主窗口维护（`Library/System.cpp:245-330, 412-540`，`Library/Skin.cpp:987-1064`），**从不 `SetParent`**；tauri-plugin-desktop-underlay FAQ 也建议 `always_on_bottom` + `ignore_cursor_events` 作为替代 | 仍是顶层窗口：被其它窗口覆盖（这是想要的）；Win+D 处理是启发式；`HWND_BOTTOM` 在别的窗口激活时可能闪一下；必须绕开 tao `apply_diff` 的样式覆写（项目已有 `win32_set_topmost`/`win32_show` 的绕行模式可复用） |
| 2 | **`SetParent` 到 Progman（DefView 之上）+ 把宿主 HWND 变成分层子窗口**：设 `WS_CHILD`，`WS_EX_LAYERED` + `SetLayeredWindowAttributes(0, 0xFF, LWA_ALPHA)`；在 build ≥ 26100 上再 `DwmSetWindowAttribute(DWMWA_REDIRECTIONBITMAP_ALPHA, TRUE)` 让（空的）重定向位图 alpha 被采用；`DWMWCP_DONOTROUND` | **中等，必须原型验证** | 微软给 Lively 的 24H2 指引："your application ... will now need to create its own WS_EX_LAYERED child HWND ... This window should likely be a SetLayeredWindowAttributes(bAlpha=0xFF) window"（Lively `WinDesktopCore.cs:129-146`）；本机实测 24H2 的 `SHELLDLL_DefView` 就是 `WS_EX_LAYERED` + `LWA_ALPHA=255` 的子窗口且"draw mostly transparent"；Wallpaper Engine 的桌面窗口也是 `WS_EX_LAYERED|WS_EX_NOREDIRECTIONBITMAP` 子窗口（§3.6）；electron-as-wallpaper 的 `transparent` 选项就是这套样式；`DWMWA_REDIRECTIONBITMAP_ALPHA` 文档："Enables or disables the use of the alpha channel in the window's redirection bitmap ... premultiplied ... supported starting with Windows 11 Build 26100"；DirectComposition 官方文档有"layered child window + DComp"的完整范例 | (1) 分层子窗口需要 manifest 声明 Win8+ 兼容（`supportedOS` GUID），tauri-build 默认 manifest **没有**（§3.2）；(2) tao 窗口类是 `CS_OWNDC`（`tao/window.rs:1360`），文档说 `WS_EX_LAYERED` "cannot be used if the window has a class style of either CS_OWNDC or CS_CLASSDC"——但 tao 自己的 `set_ignore_cursor_events` 也照加不误，实际行为未知；(3) `DWMWA_REDIRECTIONBITMAP_ALPHA` 只有 26100+；(4) tao `apply_diff` 会抹掉这些样式；(5) 跨进程父子窗口会把输入队列串起来（Raymond Chen），键盘焦点行为未知 |
| 2b | 同上，但宿主 HWND 用 **`WS_EX_NOREDIRECTIONBITMAP`**（没有表面，所有像素都来自 WebView2 的 DComp 视觉）。两种做法：(i) 创建后 `SetWindowLong` 加 ex-style（效果未知，tao 只在创建期用它，`tao/window_state.rs:264-266`）；(ii) **一开始就作为子窗口创建**——tauri 2.9.5 提供 `WebviewWindowBuilder::parent_raw(HWND)`（`tauri-2.9.5/src/webview/webview_window.rs:773`）→ tao `Parent::ChildOf` → `WS_CHILD` 且自动去掉 `WS_CAPTION`（`tao/window_state.rs:267-275`），但 `no_redirection_bitmap` 在 tauri 2.9.5 没有暴露（tauri PR #15410 于 2026-07-03 才合并） | 中等偏低 | Chromium 自己的 `Intermediate D3D Window` 就是 `WS_CHILD|WS_EX_LAYERED|WS_EX_NOREDIRECTIONBITMAP` 的跨进程子窗口（`ui/gl/child_window_win.cc:102-112`，本机实测一致）；Wallpaper Engine 同款 | 方案 (ii) 意味着切换模式要销毁重建主窗口（WebView 重载、状态迁移）；方案 (i) 行为无文档 |
| 3 | **Composition（visual）hosting**：`CreateCoreWebView2CompositionController` + 自建 DComp 视觉树挂到 `WS_EX_NOREDIRECTIONBITMAP` 窗口 | 低（现阶段） | wry PR #1762（open，目标 0.56）已实现，含输入转发；WebView2 官方 "Windowed vs Visual hosting" 文档 | 需要未发布的 wry + tauri-runtime-wry 适配，或本项目绕开 tauri 自建 webview；工程量最大 |
| 4 | `UpdateLayeredWindow` 逐像素 alpha（自己提供整幅 ARGB 位图） | 不可行 | WebView2 像素只能靠 `CapturePreview` 拿到，性能不可接受；且 `SetLayeredWindowAttributes` 调用过后 `UpdateLayeredWindow` 会失败直到重设样式位 | — |
| 5 | 挂到 WorkerW（图标**之下**，Lively/两个 Tauri 插件的做法） | 不满足需求 | 需求是"图标之上"；且这些先例全部使用不透明内容（Lively 加载完成后把 `DefaultBackgroundColor` 重置为 `White`，§4.1） | — |

**建议**：先做方案 1 的原型（改动最小、透明零风险），把它作为保底；同时用一个独立的最小原型验证方案 2/2b 在本机 26200 上的真实合成结果（见 (c)）。若方案 2 在 26100+ 成立而在旧版 Windows 10/11 不成立，可按 OS build 分流：≥ 26100 走方案 2，否则退回方案 1。

### (c) 只能靠原型才能确定的问题

见文末「未验证 / 需要原型验证」清单。

---

## 1. tao 0.34.5 在 Windows 上对透明窗口做了什么

### 1.1 唯一的透明动作：创建后一次 `DwmEnableBlurBehindWindow`（空 region）

`tao/window.rs:1283-1297`：

```rust
  // making the window transparent
  if attributes.transparent && !pl_attribs.no_redirection_bitmap {
    // Empty region for the blur effect, so the window is fully transparent
    let region = CreateRectRgn(0, 0, -1, -1);

    let bb = DWM_BLURBEHIND {
      dwFlags: DWM_BB_ENABLE | DWM_BB_BLURREGION,
      fEnable: true.into(),
      hRgnBlur: region,
      fTransitionOnMaximized: false.into(),
    };

    let _ = DwmEnableBlurBehindWindow(real_window.0, &bb);
    let _ = DeleteObject(region.into());
  }
```

- 原理：DWM 文档（blur-ovw）说明"When you apply the blur-behind effect to a subregion of the window, **the alpha channel of the window is used for the nonblurred area**"。region 为空 → 整个窗口都是"非模糊区" → DWM 采用重定向位图的 alpha。Windows 8 起不再有模糊效果，但这条 alpha 通路仍在，这就是 tauri 透明窗口一直能工作的原因。
- **只在创建时执行一次**。`WindowFlags::TRANSPARENT`（`tao/window_state.rs:85`）在 `tao/window.rs:1136` 被 set 之后全 crate 无任何读取；`apply_diff` 不会重放它。`tao/event_loop.rs:929` 还留着 `// FIXME: detect WM_DWMCOMPOSITIONCHANGED and call DwmEnableBlurBehindWindow if necessary`。
- 若 `no_redirection_bitmap`（`WS_EX_NOREDIRECTIONBITMAP`）为 true 则跳过——窗口没有重定向表面，天然透明。tao 通过 `WindowBuilderExtWindows::with_no_redirection_bitmap`（`tao-0.34.5/src/platform/windows.rs:294, 346-347`）暴露，映射为 `WindowFlags::NO_BACK_BUFFER` → `WS_EX_NOREDIRECTIONBITMAP`（`tao/window_state.rs:264-266`）。**tauri 2.9.5 / tauri-runtime-wry 2.9.3 没有暴露它**（grep 无结果）；tauri PR #15410 "feat(core): add no_redirection_bitmap API on Windows" 已于 2026-07-03 合并，用于消除透明窗口首帧白块。
- 不使用：`WS_EX_LAYERED`（透明用途）、`SetLayeredWindowAttributes`、`UpdateLayeredWindow`、`DwmExtendFrameIntoClientArea`、`DWMWA_REDIRECTIONBITMAP_ALPHA`（tao 0.34.5 与 0.35.2 均无）。

### 1.2 窗口类与背景擦除：tao 自己不画任何像素

- 窗口类 `tao/window.rs:1358-1371`：`style: CS_HREDRAW | CS_VREDRAW | CS_OWNDC`，`hbrBackground: HBRUSH::default()`（NULL）。
- `WM_ERASEBKGND`（`tao/event_loop.rs:1132-1149`）只在 `window_state.background_color` 为 `Some` 时 `FillRect`，否则交给 `DefSubclassProc`；NULL 背景刷意味着什么都不擦。tauri-runtime-wry 只有配置了 `backgroundColor` 才会 `window.background_color(color)`（`tauri-runtime-wry-2.9.3/src/lib.rs:908-909`），mini-todo 没配。→ 重定向位图保持初始的全 0（alpha 0），配合 blur-behind 即完全透明；页面的 rgba(…, 0.45) 半透明底色由 WebView2 的 DComp 表面提供。
- 注意 `CS_OWNDC`：Microsoft "Extended Window Styles" 文档：`WS_EX_LAYERED` "cannot be used if the window has a class style of either CS_OWNDC or CS_CLASSDC"。但 tao 的 `set_ignore_cursor_events` 仍会加 `WS_EX_TRANSPARENT | WS_EX_LAYERED`（`tao/window_state.rs:285-287`），tauri 用户实际能用（#15947 里正是高频切它导致偶发黑屏）。文档与实践不一致，方案 2 依赖这一点，需要原型确认。

### 1.3 `to_window_styles()` 对 `decorations:false, transparent:true` 的输出

`tao/window_state.rs:242-301`：

```rust
  pub fn to_window_styles(self) -> (WINDOW_STYLE, WINDOW_EX_STYLE) {
    let (mut style, mut style_ex) = (Default::default(), Default::default());
    style |= WS_CAPTION | WS_CLIPSIBLINGS | WS_SYSMENU;
    style_ex |= WS_EX_WINDOWEDGE | WS_EX_ACCEPTFILES;
    if self.contains(WindowFlags::RESIZABLE)   { style |= WS_SIZEBOX; }
    if self.contains(WindowFlags::MAXIMIZABLE) { style |= WS_MAXIMIZEBOX; }
    if self.contains(WindowFlags::MINIMIZABLE) { style |= WS_MINIMIZEBOX; }
    if self.contains(WindowFlags::VISIBLE)     { style |= WS_VISIBLE; }
    if self.contains(WindowFlags::ON_TASKBAR)  { style_ex |= WS_EX_APPWINDOW; }
    if self.contains(WindowFlags::ALWAYS_ON_TOP) { style_ex |= WS_EX_TOPMOST; }
    if self.contains(WindowFlags::NO_BACK_BUFFER) { style_ex |= WS_EX_NOREDIRECTIONBITMAP; }
    if self.contains(WindowFlags::CHILD) {
      style |= WS_CHILD; // This is incompatible with WS_POPUP if that gets added eventually.
      // Remove decorations window styles for child
      if !self.contains(WindowFlags::MARKER_DECORATIONS) {
        style &= !WS_CAPTION;
        style_ex &= !WS_EX_WINDOWEDGE;
      }
    }
    if self.contains(WindowFlags::POPUP)     { style |= WS_POPUP; }
    if self.contains(WindowFlags::MINIMIZED) { style |= WS_MINIMIZE; }
    if self.contains(WindowFlags::MAXIMIZED) { style |= WS_MAXIMIZE; }
    if self.contains(WindowFlags::IGNORE_CURSOR_EVENT) { style_ex |= WS_EX_TRANSPARENT | WS_EX_LAYERED; }
    if self.intersects(MARKER_EXCLUSIVE_FULLSCREEN | MARKER_BORDERLESS_FULLSCREEN) { style &= !WS_OVERLAPPEDWINDOW; }
    if self.contains(WindowFlags::RIGHT_TO_LEFT_LAYOUT) { style_ex |= WS_EX_LAYOUTRTL | WS_EX_RTLREADING | WS_EX_RIGHT; }
    if !self.contains(WindowFlags::FOCUSABLE) { style_ex |= WS_EX_NOACTIVATE; }
    (style, style_ex)
  }
```

对 mini-todo 主窗口（`decorations:false, transparent:true, shadow:false`, `Parent::None`）：

| 配置 | `GWL_STYLE` | `GWL_EXSTYLE` |
|---|---|---|
| `resizable: true` | `WS_CAPTION \| WS_CLIPSIBLINGS \| WS_SYSMENU \| WS_SIZEBOX \| WS_MAXIMIZEBOX \| WS_MINIMIZEBOX \| WS_VISIBLE` | `WS_EX_WINDOWEDGE \| WS_EX_ACCEPTFILES \| WS_EX_APPWINDOW` |
| `resizable: false` | 同上去掉 `WS_SIZEBOX`（`WS_THICKFRAME`） | 同上 |

- 本机实测与此一致：发布版 2.3.8 顶层窗口 style `0x14CF0000`、ex `0x00040110`；debug 版（固定模式，项目自己去掉了 `WS_EX_APPWINDOW`、加了 `WS_EX_TOOLWINDOW`）style `0x14CB0000`、ex `0x00000190`。
- **`WS_POPUP` 不会被设置**：只有 `Parent::OwnedBy`（tauri `owner_raw`）才设 `POPUP`（`tao/window.rs:1159-1161`）；`Parent::None` 设 `ON_TASKBAR`（`window.rs:1163-1166`）。
- **`WS_CHILD` 只在创建期由 `Parent::ChildOf` 产生**（`tao/window.rs:1152-1158`，对应 tauri `WebviewWindowBuilder::parent_raw`，`tauri-runtime-wry/src/lib.rs:1111-1117` → `with_parent_window`）。创建后没有任何 tao API 能把 `CHILD` 位打开或关闭。
- `decorations:false` 时 `WS_CAPTION` 仍留在 style 里；标题栏是靠 `WM_NCCALCSIZE` 返回 0 消掉的（创建期 `tao/window.rs:1392-1423`，子类化后 `tao/event_loop.rs:2152-2203`）。`shadow:false` → `MARKER_UNDECORATED_SHADOW` 为 false → 不加 inset（`calculate_insets_for_dpi`，`tao/util.rs:444-467`），整个窗口矩形就是客户区。这一点对嵌入有利：tauri-plugin-wallpaper 提到 `shadow: true` 的可缩放无边框窗口有 8px 隐形 inset 会留黑边（`attacher.rs:124-129`），mini-todo 没有。
- `to_adjusted_window_styles()`（`window_state.rs:304-312`）仅供 `AdjustWindowRectEx` 用，无装饰时去掉 `WS_CAPTION|WS_THICKFRAME`。

### 1.4 `apply_diff` 会整体覆写样式（对任何手动样式改动都是致命的）

`tao/window_state.rs:426-441`：

```rust
    if diff != WindowFlags::empty() {
      let (style, style_ex) = new.to_window_styles();
      unsafe {
        SendMessageW(window, *event_loop::SET_RETAIN_STATE_ON_SIZE_MSG_ID, Some(WPARAM(1)), Some(LPARAM(0)));
        // This condition is necessary to avoid having an unrestorable window
        if !new.contains(WindowFlags::MINIMIZED) {
          SetWindowLongW(window, GWL_STYLE, style.0 as i32);
          SetWindowLongW(window, GWL_EXSTYLE, style_ex.0 as i32);
        }
        let mut flags = SWP_NOZORDER | SWP_NOMOVE | SWP_NOSIZE | SWP_FRAMECHANGED;
        ...
        let _ = SetWindowPos(window, None, 0, 0, 0, 0, flags);
```

触发点包括 `set_visible`、`set_resizable`、`set_minimizable/maximizable/closable`、`set_always_on_top/bottom`、`set_decorations`、`set_undecorated_shadow`、`set_fullscreen`、`set_ignore_cursor_events`、`set_focusable`、`set_rtl`、`set_maximized/minimized`，以及 `set_outer_position`/`set_inner_size` 内部的 `f.set(MAXIMIZED, false)`（`tao/window.rs:232-238, 300-306`，仅当 MAXIMIZED 原本为 true 才会产生 diff）。项目文档 `pc/src-tauri/src/commands/window.rs:858-887` 已记录此行为并给出绕行样板（`win32_set_topmost` / `win32_show`）。桌面模式必须把 `WS_CHILD`/`WS_EX_LAYERED`/`WS_EX_NOREDIRECTIONBITMAP` 纳入同一套"绕开 tao、事后补样式"的保护。

### 1.5 `WM_WINDOWPOSCHANGING` 的 `ALWAYS_ON_BOTTOM` 强制

`tao/event_loop.rs:1229-1235`：只要 flags 含 `ALWAYS_ON_BOTTOM`，每次 `SetWindowPos` 都被改写为 `hwndInsertAfter = HWND_BOTTOM`。作为顶层窗口这正是方案 1 想要的；若作为 Progman 的子窗口则会把自己压到 `SHELLDLL_DefView` 之下（图标之下），嵌入模式下**不能**开 `always_on_bottom`。

### 1.6 项目现有的 Win32 干预点

- `pc/src-tauri/src/lib.rs:38-59` `setup_window_rounded_corners`：`DwmSetWindowAttribute(DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND)`。
- `pc/src-tauri/src/commands/window.rs:831-856` `apply_fixed_ex_style`：固定模式 `WS_EX_TOOLWINDOW` / 去 `WS_EX_APPWINDOW` + `SWP_FRAMECHANGED`；`868-887` `win32_set_topmost`；`900+` `win32_show`；`535-541` `restore_if_minimized`（固定模式被 Win+D 最小化后轮询还原）。

---

## 2. wry 0.53.5 对 WebView2 的透明与宿主方式

### 2.1 Windowed hosting，无 composition controller

`wry/mod.rs:363-411` `create_controller`：

```rust
      if let Ok(env10) = env10 {
        let controller_opts = env10.CreateCoreWebView2ControllerOptions()?;
        if let Some((r, g, b, mut a)) = background_color {
          if let Ok(opts3) = controller_opts.cast::<ICoreWebView2ControllerOptions3>() {
            if a != 0 { a = 255; }
            opts3.SetDefaultBackgroundColor(COREWEBVIEW2_COLOR { R: r, G: g, B: b, A: a })?;
          }
        }
        controller_opts.SetIsInPrivateModeEnabled(incognito)?;
        env10.CreateCoreWebView2ControllerWithOptions(hwnd, &controller_opts, &handler)?;
      } else {
        env.CreateCoreWebView2Controller(hwnd, &handler)?
      }
```

- 全 crate 没有 `CreateCoreWebView2CompositionController`；composition（visual）hosting 由 wry PR #1762（open，"feat(windows): add DirectComposition (visual) hosting for WebView2"，计划进 0.56）引入，包含 `WebViewBuilderExtWindows::with_composition_visual_target` 与 `register_composition_visual_target(hwnd, visual)`，并自带鼠标/触控/光标/焦点转发。
- 透明色两次下发：`attributes.transparent` → `background_color = Some((0,0,0,0))`（`mod.rs:126-130`）进 `ControllerOptions3`；`init_webview` 再调 `set_background_color(controller, (0,0,0,0))`（`mod.rs:447-450` → `ICoreWebView2Controller2::SetDefaultBackgroundColor`，`mod.rs:1790-1808`）。alpha 非 0 一律强制 255（`mod.rs:391-393, 1795-1797`），因为 WebView2 文档："The only supported alpha values are 0 and 255, all other values will result in E_INVALIDARG"；"In the case of a transparent DefaultBackgroundColor WebView will render hosting app content as the background"。
- tauri-runtime-wry 把窗口的 `transparent` 同时传给 tao 与 wry：`window.transparent(config.transparent)`（`lib.rs:851`）、`WebViewBuilder…with_transparent(webview_attributes.transparent)`（`lib.rs:4589`），主窗口走 `WebviewKind::WindowContent` → `build(&window)`（`lib.rs:4993` 只有多 webview 才 `build_as_child`）。

### 2.2 容器 HWND 与尺寸同步

- `create_container_hwnd`（`wry/mod.rs:179-279`）：类 `WRY_WEBVIEW`，`hbrBackground` NULL；`WS_CHILD | WS_CLIPCHILDREN [| WS_VISIBLE]`，ex-style 0；创建后 `SetWindowPos(HWND_TOP, SWP_ASYNCWINDOWPOS|SWP_NOACTIVATE|SWP_NOMOVE|SWP_NOOWNERZORDER|SWP_NOSIZE)`。非 child 模式尺寸 = 父窗口 `GetClientRect`（`parent_bounds`, `mod.rs:1189-1196`）。
- 父窗口子类化 `parent_subclass_proc`（`mod.rs:1198-1266`，安装于 tao HWND，`mod.rs:534-537`）：
  - `WM_SIZE`（非最小化）→ `GetClientRect(parent)` → `controller.SetBounds({0,0,w,h})` + 对 `controller.ParentWindow()`（即 `WRY_WEBVIEW`）`SetWindowPos(0,0,w,h, SWP_ASYNCWINDOWPOS|SWP_NOACTIVATE|SWP_NOZORDER)`；
  - `WM_MOVE`/`WM_MOVING` → `controller.NotifyParentWindowPositionChanged()`；
  - `WM_SETFOCUS`/`WM_ENTERSIZEMOVE` → `controller.MoveFocus(PROGRAMMATIC)`。
  - 这些消息对子窗口同样会到达（`SetParent` 后由 Windows 正常投递），所以尺寸同步逻辑本身不依赖顶层身份。
- `set_bounds_inner`（`mod.rs:1419-1444`）、`resize_to_parent`（`1455-1458`）；`reparent()`（`mod.rs:1671-1690`）是把 `WRY_WEBVIEW` 在两个 tao 窗口之间搬家（tauri `Webview::reparent`），与"搬 tao 窗口本身"无关。
- `WM_SETFOCUS` 到 `WRY_WEBVIEW` 时把焦点转给第一个子窗口（`mod.rs:191-199`）。

### 2.3 本机实测的 HWND 链（发布版 pid 21840 与 debug 版 pid 25156 一致）

```
Tauri Window ('Mini Todo')                 style WS_VISIBLE|WS_CAPTION|WS_CLIPSIBLINGS[|WS_THICKFRAME]   ex WS_EX_WINDOWEDGE|WS_EX_ACCEPTFILES|(WS_EX_APPWINDOW | WS_EX_TOOLWINDOW)   corner=2(ROUND)
 ├ TAURI_DRAG_RESIZE_BORDERS (仅发布版/可缩放时)   WS_CHILD|WS_VISIBLE|WS_CLIPSIBLINGS
 └ WRY_WEBVIEW                              WS_CHILD|WS_VISIBLE|WS_CLIPCHILDREN
    └ Chrome_WidgetWin_0                    WS_CHILD|WS_VISIBLE|WS_CLIPSIBLINGS|WS_CLIPCHILDREN   (+ 二十余个 WS_EX_TRANSPARENT 的 1px 覆盖子窗口)
       └ Chrome_WidgetWin_1 ('Mini Todo', 浏览器进程 pid 24444)   WS_CHILD|WS_VISIBLE|…   ex WS_EX_NOREDIRECTIONBITMAP
          ├ Chrome_RenderWidgetHostHWND ('Chrome Legacy Window')   ex WS_EX_TRANSPARENT
          └ Intermediate D3D Window (GPU 进程 pid 35000)   WS_CHILD|WS_VISIBLE|WS_DISABLED|WS_CLIPSIBLINGS   ex WS_EX_LAYERED|WS_EX_NOREDIRECTIONBITMAP|WS_EX_TRANSPARENT|WS_EX_NOPARENTNOTIFY
```

- `TAURI_DRAG_RESIZE_BORDERS` 来自 `tauri-runtime-wry-2.9.3/src/undecorated_resizing.rs:107-140`（无边框可缩放窗口的拖拽边框辅助子窗口）。
- `Intermediate D3D Window` 与 Chromium `ui/gl/child_window_win.cc:102-112` 完全一致：

```cpp
  // WS_EX_NOPARENTNOTIFY and WS_EX_LAYERED make the window transparent for
  // input. WS_EX_NOREDIRECTIONBITMAP avoids allocating a
  // bitmap that would otherwise be allocated with WS_EX_LAYERED, the bitmap is
  // only necessary if using Gdi objects with the window.
  const HWND window = CreateWindowEx(
      WS_EX_NOPARENTNOTIFY | WS_EX_LAYERED | WS_EX_TRANSPARENT |
          WS_EX_NOREDIRECTIONBITMAP,
      reinterpret_cast<wchar_t*>(g_window_class), L"",
      WS_CHILDWINDOW | WS_DISABLED | WS_VISIBLE, 0, 0, /*width*/ 1, ...
```

  含义：**WebView2 的全部像素来自 GPU 进程在这个跨进程分层子窗口上绑定的 DirectComposition 视觉树**（`dcomp_presenter.cc:46-47` `child_window_.Initialize(); layer_tree_->Initialize(child_window_.window(), ...)`），alpha 由 swapchain 携带；tao/wry/`WRY_WEBVIEW` 三层 HWND 都不画像素。透明效果最终取决于 DWM 把这棵视觉树叠在什么之上。WebView2 内部本来就在做跨进程 `SetParent`（WebView2Feedback #985 中微软工程师 jamesoli："we do a cross process SetParent internally"）。

### 2.4 WebView2 运行时侧的透明回归（与本任务无直接关系，但排障时要知道）

- WebView2Feedback #5481：Runtime 145.x canary 曾让 `DefaultBackgroundColor` alpha=0 失效（灰/白底），145 beta / 146 canary 已修；有 Tauri 用户在该 issue 报告同样现象。
- #5492：Runtime 144.x 透明窗口出现经典标题栏伪影（`WM_NCACTIVATE`/`WM_NCPAINT` 相关）。
- #5668：alpha=0 的透明 WebView2 仍会吞掉鼠标（不会自动 click-through）。

---

## 3. Windows 平台事实（官方文档）

### 3.1 DWM blur-behind 只对顶层窗口有效

- [DwmEnableBlurBehindWindow](https://learn.microsoft.com/en-us/windows/win32/api/dwmapi/nf-dwmapi-dwmenableblurbehindwindow) Remarks："Beginning with Windows 8, calling this function doesn't result in the blur effect"；"Some Windows Graphics Device Interface (GDI) operations don't preserve alpha values, so you should take care when presenting child windows because the alpha values they contribute are unpredictable"；"**This function can be called only on top-level windows. An error occurs when this function is called on other window types.**"
- [DWM Blur Behind Overview](https://learn.microsoft.com/en-us/windows/win32/dwm/blur-ovw)："When you apply the blur-behind effect to a subregion of the window, the alpha channel of the window is used for the nonblurred area. This can cause an unexpected transparency in the nonblurred region"——这正是 tao 依赖的副作用。
- 现代替代：[DWMWINDOWATTRIBUTE](https://learn.microsoft.com/en-us/windows/win32/api/dwmapi/ne-dwmapi-dwmwindowattribute) `DWMWA_REDIRECTIONBITMAP_ALPHA`："Enables or disables the use of the alpha channel in the window's redirection bitmap. If this attribute is set to true, the window must contain premultiplied alpha values in each pixel. If it is false, the alpha is ignored and the redirection bitmap is treated as fully opaque. This attribute defaults to false. This value is supported starting with Windows 11 Build 26100." 文档没有限定顶层/子窗口。

### 3.2 分层子窗口（`WS_EX_LAYERED` on `WS_CHILD`）：Windows 8 起支持，需要 manifest

- [Extended Window Styles](https://learn.microsoft.com/en-us/windows/win32/winmsg/extended-window-styles) `WS_EX_LAYERED`："This style cannot be used if the window has a class style of either CS_OWNDC or CS_CLASSDC. Windows 8: The WS_EX_LAYERED style is supported for top-level windows and child windows. Previous Windows versions support WS_EX_LAYERED only for top-level windows."
- [SetLayeredWindowAttributes](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setlayeredwindowattributes) / [UpdateLayeredWindow](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-updatelayeredwindow) 参数说明重复了同一句 Windows 8 支持子窗口的说明；SLWA Remarks："once SetLayeredWindowAttributes has been called for a layered window, subsequent UpdateLayeredWindow calls will fail until the layering style bit is cleared and set again"。
- [Window Features › Layered Windows](https://learn.microsoft.com/en-us/windows/win32/winmsg/window-features)："After the CreateWindowEx call, the layered window will not become visible until the SetLayeredWindowAttributes or UpdateLayeredWindow function has been called for this window." → 方案 2 必须调用 `SetLayeredWindowAttributes(…, 255, LWA_ALPHA)`。
- [Using Windows › Using Layered Windows](https://learn.microsoft.com/en-us/windows/win32/winmsg/using-windows)："**In order to use layered child windows, the application has to declare itself Windows 8-aware in the manifest.** For windows 10/11, one can include this compatibility snippet in its app.manifest": `<compatibility xmlns="urn:schemas-microsoft-com:compatibility.v1"><application><supportedOS Id="{8e0f7a12-bfb3-4fe8-b9a5-48fd50a15a9a}" /></application></compatibility>`。
- **tauri-build 2.5.3 默认 manifest**（`tauri-build-2.5.3/src/windows-app-manifest.xml`）只声明了 Common-Controls 6.0 依赖，**没有 `supportedOS`**；项目 `pc/src-tauri/build.rs` 只是 `tauri_build::build()`。要用分层子窗口必须通过 `tauri_build::WindowsAttributes::app_manifest(...)`（`tauri-build-2.5.3/src/lib.rs:222-262, 285-335`）提供自定义 manifest。
- 两种机制：`SetLayeredWindowAttributes` = 色键 / 整窗常量 alpha；`UpdateLayeredWindow` = 应用自供 ARGB 位图的逐像素 alpha。Rainmeter 用后者（`Library/Skin.cpp:3189-3206`），但 WebView2 内容不在 GDI 位图里，本项目无法用后者。

### 3.3 DirectComposition 在子窗口 / 分层窗口中可用

- [IDCompositionDevice::CreateTargetForHwnd](https://learn.microsoft.com/en-us/windows/win32/api/dcomp/nf-dcomp-idcompositiondevice-createtargetforhwnd) Remarks："The window can be a top-level window or a child window. In either case, the window can be a layered window, but in all cases the window must belong to the calling process." 并描述每个窗口的四个概念层：GDI 内容（最底）→ 非 topmost 视觉树 → 子窗口内容 → topmost 视觉树。
- [DirectComposition Basic concepts](https://learn.microsoft.com/en-us/windows/win32/directcomp/basic-concepts)："the composition target window, can be a top-level window or a child window. Also, the composition target window can be a layered window".
- [How to animate the bitmap of a layered child window](https://learn.microsoft.com/en-us/windows/win32/directcomp/how-to--animate-the-bitmap-of-a-layered-child-window)：官方范例 `CreateWindowEx(WS_EX_LAYERED, …, WS_CHILD | WS_CLIPSIBLINGS, …)` + `SetLayeredWindowAttributes` + `CreateSurfaceFromHwnd` + `DWMWA_CLOAK`。说明"分层子窗口 + DComp"是受支持的组合。
- Chromium 的透明渲染表面本身就是分层子窗口上的 DComp 目标（§2.3），这在每一个 WebView2 里都在运行。

### 3.4 `DWMWA_WINDOW_CORNER_PREFERENCE` 与子窗口

- 官方 [Apply rounded corners](https://learn.microsoft.com/en-us/windows/apps/desktop/modernize/apply-rounded-corners) 只谈"top-level window"，未说明子窗口。
- 经验证据：tauri-plugin-desktop-underlay #85 —— 窗口 `SetParent` 到 WorkerW 后圆角仍然存在并露出空隙；tauri-plugin-wallpaper 在 `SetParent` 后调 `DwmSetWindowAttribute(DWMWCP_DONOTROUND)` 解决（`src/platform/windows/corners.rs`, `attacher.rs:101-105`），README 称有效。→ 属性对重挂后的窗口起作用；mini-todo 目前主动设 `DWMWCP_ROUND`，嵌入时要改成 `DONOTROUND`（或保留圆角但接受四角露底）。
- 本机 `DwmGetWindowAttribute(33)` 对原生子窗口（DefView 等）返回错误、对顶层 tao 窗口返回 2，与"属性针对顶层/曾为顶层的窗口"一致。

### 3.5 `SetParent` 语义、DPI 与跨进程

- [SetParent](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setparent) Remarks："For compatibility reasons, SetParent does not modify the WS_CHILD or WS_POPUP window styles ... if hWndNewParent is not NULL and the window was previously a child of the desktop, you should clear the WS_POPUP style and set the WS_CHILD style before calling SetParent"；"Unexpected behavior or errors may occur if hWndNewParent and hWndChild are running in different DPI awareness modes"（Windows 10 1703+ 跨进程会强制重置子窗口进程的 DPI awareness）。本机 Progman 与 tao 窗口的 `GetAwarenessFromDpiAwarenessContext` 都是 2（PER_MONITOR_AWARE；tao 用 PMv2，`tao/dpi.rs:20-40`），该检查应能通过。
- Raymond Chen, [Is it legal to have a cross-process parent/child or owner/owned window relationship?](https://devblogs.microsoft.com/oldnewthing/20130412-00/?p=4683)：合法，但"Creating a cross-thread parent/child ... relationship implicitly attaches the input queues of the threads"，"some window messages are blocked between processes"，"things will definitely stop working if you change that other window from a top-level window to a child window"（这里被改的是我们自己的窗口，explorer 只是父方，风险小一些）。
- WebView2Feedback #985：`SetParent` 必须在 `EnsureCoreWebView2` 完成之后，否则 WebView 不可见；DPI awareness 不一致时 `SetParent` 返回 `ERROR_INVALID_STATE`。

### 3.6 24H2+ 桌面窗口结构（本机实测 + 微软给 Lively 的说明 + Rainmeter 注释）

本机（build 26200）`GetWindow` 枚举结果（Z 序自上而下）：

```
000102A6 Progman 'Program Manager'   rect=(-2560,-240)-(1920,1360)  WS_POPUP|WS_VISIBLE   ex=WS_EX_NOREDIRECTIONBITMAP|WS_EX_TOOLWINDOW   dpiAwareness=2
  000102AA SHELLDLL_DefView          同尺寸   WS_CHILD|WS_VISIBLE   ex=WS_EX_LAYERED   SetLayeredWindowAttributes(key=0, alpha=255, LWA_ALPHA)
    000102AC SysListView32 'FolderView'
  0005039C WorkerW                   同尺寸   WS_CHILD|WS_VISIBLE|WS_DISABLED   ex=WS_EX_TRANSPARENT|WS_EX_TOOLWINDOW|WS_EX_NOACTIVATE
    00020044 WPEDesktopDX11Window 'WPELiveWallpaper' (Wallpaper Engine)   WS_CHILD|WS_VISIBLE   ex=WS_EX_LAYERED|WS_EX_NOREDIRECTIONBITMAP|WS_EX_NOACTIVATE   LWA(alpha=255)
    000704CE WPECloneView            WS_CHILD|WS_VISIBLE   ex=WS_EX_LAYERED|WS_EX_NOREDIRECTIONBITMAP|WS_EX_NOACTIVATE
```

微软对 Lively 作者的说明（引自 `src/Lively/Lively/Core/WinDesktopCore.cs:129-146`）：

```
When the desktop is split out from the list view window (aka the "raised desktop") we no
longer create multiple top-level HWNDs to support this scenario. Instead, the top-level
"Progman" window is now created with WS_EX_NOREDIRECTIONBITMAP (so there is no GDI content
for that window at all) and the shell DefView child window is a WS_EX_LAYERED child window.
When the desktop is raised, we create a child WorkerW window that is z-ordered under the
DefView that will render the wallpaper. The DefView window will draw mostly transparent with
just the icons and text.

If your application forces the "raised desktop" state, it will now need to create its own
WS_EX_LAYERED child HWND that is z-ordered under the DefView window but above the WorkerW
window. This window should likely be a SetLayeredWindowAttributes(bAlpha=0xFF) window so
that you can do DX blt presents to it and not suffer performance issues.
```

推论：在 24H2+ 上，桌面各层（图标层 DefView、壁纸层 WorkerW 内的第三方窗口）全部是 **`WS_EX_LAYERED` 子窗口**，由 DWM 逐个合成；DefView 以 `LWA_ALPHA=255` 却"draw mostly transparent"，意味着 DWM 对这类分层子窗口采用了逐像素 alpha（很可能借助 26100 引入的 `DWMWA_REDIRECTIONBITMAP_ALPHA` 或 DComp 内容）。"图标之上"= 作为 Progman 的子窗口、Z 序排在 DefView 之前（本机曾观察到的 mini-todo 原型正是这个位置）。Rainmeter `Library/System.cpp:247-271` 用 `GetProcAddress(user32, "GetCurrentMonitorTopologyId")` 判定 24H2（10.0.26100.2454+），此时"桌面图标宿主"就是 Progman 本身。旧版（Win10 / Win11 ≤ 23H2）结构是顶层 `WorkerW`（含 DefView）+ 另一个顶层 `WorkerW`（壁纸）+ 顶层 `Progman`，此时父窗口是普通有重定向表面的顶层窗口，行为可能不同。

### 3.7 坐标与 DPI 消息

- [GetWindowRect](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getwindowrect)："The dimensions are given in screen coordinates".
- [SetWindowPos](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setwindowpos)：X/Y "in client coordinates"（子窗口相对父窗口客户区；顶层窗口相对屏幕）。
- [WM_DPICHANGED](https://learn.microsoft.com/en-us/windows/win32/hidpi/wm-dpichanged) 面向顶层窗口；[WM_DPICHANGED_AFTERPARENT](https://learn.microsoft.com/en-us/windows/win32/hidpi/wm-dpichanged-afterparent)："For Per Monitor v2 top-level windows, this message is sent to all HWNDs in the child HWND tree of the window that is undergoing a DPI change ... There is no default handling of this message in DefWindowProc."
- [GetDpiForWindow](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getdpiforwindow) 对任意 HWND 有效（本机 tao 窗口与其所有子窗口都返回 120）。

---

## 4. 先例

### 4.1 Lively Wallpaper（rocksdanister/lively，分支 `core-separation`）

- WebView2 播放器是独立进程的 WinForms 窗体（`src/Lively/Lively.Player.WebView2/Form1.cs`）：`CreateParams` 加 `WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE`（110-119 行）；`BackColor` 设为主题深/浅色（84-96 行）；`DefaultBackgroundColor = Color.Transparent` **只在初始化期间**（134-136 行），`NavigationCompleted` 时 `// Restore default. webView.DefaultBackgroundColor = Color.White;`（295-298 行）。→ **Lively 不做透明网页壁纸**，透明色只是为了遮首帧白闪。
- 宿主侧 `src/Lively/Lively/Core/WinDesktopCore.cs:122-222` `SetupDesktopLayer`：`SendMessageTimeout(progman, 0x052C, 0xD, 0x1)` 生成 WorkerW；24H2（`Progman` 带 `WS_EX_NOREDIRECTIONBITMAP`）时取 `FindWindowEx(progman, null, "WorkerW", null)`；用 `WindowEventHook(EVENT_OBJECT_DESTROY)` 监听 WorkerW 被销毁（explorer 重启）。壁纸窗口在 DefView **之下**。

### 4.2 electron-as-wallpaper（meslzy）

- `src/attach.rs:61-100`：找 WorkerW → `SetParent(hwnd, worker_w)` → 若 `options.transparent` 则 `toggle_window_transparent(hwnd, true)`；`src/window.rs:14-32`：

```rust
pub fn toggle_window_transparent(hwnd: HWND, transparent: bool) {
    if transparent {
        unsafe {
            let styles = WindowsAndMessaging::GetWindowLongA(hwnd, WindowsAndMessaging::GWL_EXSTYLE);
            WindowsAndMessaging::SetWindowLongA(hwnd, WindowsAndMessaging::GWL_EXSTYLE,
                styles | WindowsAndMessaging::WS_EX_LAYERED.0 as i32);
            WindowsAndMessaging::SetLayeredWindowAttributes(hwnd, COLORREF::default(), 255,
                WindowsAndMessaging::LWA_ALPHA).unwrap();
        }
    } else { /* 去掉 WS_EX_LAYERED */ }
}
```

  这是"Chromium 窗口重挂后要透明就加分层样式"的直接先例（Electron 与 WebView2 同为 Chromium/DComp 渲染）。README 未说明效果细节，本研究未复现。

### 4.3 tauri-plugin-wallpaper（meslzy，2026-08 仍在更新，46 star）

- `src/platform/windows/attacher.rs`：找/生成 WorkerW（含 24H2 分支，72-76 行）→ `SetParent(hwnd, worker_w)`（99 行）→ `DWMWCP_DONOTROUND`（101-105 行）→ 按显示器矩形相对 WorkerW 坐标 `SetWindowPos`（107-118 行）→ 扩大窗口抵消无边框可缩放窗口的 `WM_NCCALCSIZE` inset，注释原话 "which would leave a black stripe at the monitor edge"（124-129 行）。
- `src/platform/windows/input.rs`：全局 raw input 转发到 `Chrome_WidgetWin_1`（"Tauri Window -> WRY_WEBVIEW -> Chrome_WidgetWin_0 -> Chrome_WidgetWin_1"，125-134 行）；键盘需要真实焦点，通过 tauri 给窗口真焦点。`helper.rs` 监听 `TaskbarCreated` 重挂。`pinner.rs`：子类化 `WM_WINDOWPOSCHANGING` 拦截 Win+D 的 `(-32000,-32000)` 移动（README "Pin Mode"）。
- 没有任何透明处理；目标是图标之下。

### 4.4 tauri-plugin-desktop-underlay（Charlie-XIAO，142 star）

- `src/core/windows.rs:28-58`：纯 `SetParent(hwnd, worker_w)` / `SetParent(hwnd, None)`，不改样式。
- Issue #85（中文）：切到 WorkerW 后出现圆角与边框；用户的自救是 `SetParent` 后去掉 `WS_CAPTION|WS_THICKFRAME|…` 并加 `WS_POPUP`。
- FAQ："setting a window as a desktop underlay disables all types of user interactions ... Alternatively, you may consider using Tauri's `always_on_bottom` and `ignore_cursor_events` features".

### 4.5 tauri / tao / wry / WebView2Feedback issue

- tauri #4261 "[feat] Display window behind desktop icons"（closed → 建议做插件）：amrbashir 贴了 `0x052C` + `SetParent` 的最小代码；meslzy 在此宣布 tauri-plugin-wallpaper。
- tauri #15947（open）："transparent always-on-top window sporadically renders transparent areas / desktop wallpaper black"，作者读 tao 源码得出同样结论（blur-behind hack + `WS_EX_LAYERED` 切换竞争），指出 #15410 `noRedirectionBitmap` 是结构性修复。
- tauri PR #15410（merged 2026-07-03）：暴露 tao `with_no_redirection_bitmap` 为 `noRedirectionBitmap` 配置。
- wry PR #1762（open）：composition hosting，见 §2.1。
- tao #1176 / wry #1658（closed）：`with_decorations(false)+with_transparent(true)` 标题栏闪现（已修，tao PR #1296）。
- WebView2Feedback：#985（SetParent 后不可见 / DPI）、#5668（透明仍吞输入）、#20 / #547（windowless / offscreen 需求，open）、#5481 / #5492（运行时透明回归）。GitHub 搜索 `SetParent` / `WorkerW` / `wallpaper` / `layered` 在 WebView2Feedback 没有找到"透明子窗口"专题 issue。

### 4.6 Rainmeter（逐像素透明桌面小部件的成熟实现）

- 皮肤窗口：顶层 `WS_EX_LAYERED | WS_EX_TOOLWINDOW`（`Library/Skin.cpp:292`），`UpdateLayeredWindow(..., ULW_ALPHA)`（`3189-3206`）。
- "On desktop" Z 序：`Skin::ChangeZPos`（`Skin.cpp:987-1064`）对 `ZPOSITION_ONDESKTOP` 使用 `HWND_BOTTOM` 或插到 `RainmeterSystem` 辅助窗口之后；`System::PrepareHelperWindow`（`System.cpp:457-521`）把辅助窗口放在桌面图标宿主之上/最底；`System::CheckDesktopState`（`523-540`）用 `FindWindowEx(nullptr, desktopIconsHostWindow, L"RainmeterSystem", …)` 探测 Show Desktop 状态并重排。`GetDesktopIconsHostWindow`（`System.cpp:273-330`）区分 24H2 前后。**全程没有 `SetParent`。**

### 4.7 Wallpaper Engine（本机运行中观察）

`WPEDesktopDX11Window` / `WPECloneView` 作为 WorkerW 的子窗口，样式 `WS_CHILD|WS_VISIBLE`，ex `WS_EX_LAYERED|WS_EX_NOREDIRECTIONBITMAP|WS_EX_NOACTIVATE`，`SetLayeredWindowAttributes(alpha=255)`。与 §3.6 微软指引及 Chromium D3D 窗口同款。

---

## 5. tao 对子窗口的位置 / 尺寸 / DPI 处理

### 5.1 读取（屏幕坐标）

- `outer_position()`：`util::get_window_rect` = `GetWindowRect` → `(rect.left, rect.top)`（`tao/window.rs:213-217`，`tao/util.rs:65-68`）→ **屏幕坐标**。
- `inner_position()`：`ClientToScreen`（`window.rs:220-226`）→ 屏幕坐标。
- `inner_size()` / `outer_size()`：`GetClientRect` / `GetWindowRect` 宽高（`window.rs:255-270`，`util.rs:478-502`）→ 与父子无关。

### 5.2 写入（子窗口时是父客户区坐标）

`tao/window.rs:229-252`：

```rust
  pub fn set_outer_position(&self, position: Position) {
    let (x, y): (i32, i32) = position.to_physical::<i32>(self.scale_factor()).into();
    ... WindowState::set_window_flags(..., |f| f.set(WindowFlags::MAXIMIZED, false)) ...
    unsafe {
      let _ = SetWindowPos(self.window.0, None, x, y, 0, 0,
        SWP_ASYNCWINDOWPOS | SWP_NOZORDER | SWP_NOSIZE | SWP_NOACTIVATE);
      let _ = InvalidateRgn(self.window.0, None, false);
    }
  }
```

- 作为 Progman 子窗口时，`x,y` 相对 Progman 客户区原点（本机 `(-2560,-240)`），而 `outer_position()` 返回屏幕坐标 → 直接 `set_position(get_position())` 会把窗口挪 `(2560, 240)`。嵌入模式下所有位置写入都需先 `ScreenToClient(parent)` 换算（`MapWindowPoints(HWND_DESKTOP, parent, …)`）。tauri-plugin-wallpaper `attacher.rs:107-118` 也是先做这个换算。
- `set_inner_size` → `util::set_inner_size_physical`（`util.rs:99-126`）：`adjust_window_rect`（按 `GWL_STYLE`，无装饰时先去掉 `WS_CAPTION|WS_SIZEBOX`，`util.rs:128-141`）+ `SetWindowPos(SWP_NOMOVE)` → 子窗口下正常。
- `SWP_ASYNCWINDOWPOS`：对跨进程父窗口，窗口位置请求会异步投递到拥有窗口的线程（我们自己的线程），不阻塞。

### 5.3 事件

- `WM_WINDOWPOSCHANGED` → `WindowEvent::Moved(windowpos.x, windowpos.y)`（`tao/event_loop.rs:1239-1253`）：子窗口时是父客户区坐标；前端/后端任何依赖 `Moved` 的逻辑（项目的贴边自动隐藏 `tick_auto_hide` 用的是 `get_window_rect`，屏幕坐标，不受影响）需要区分。
- `WM_SIZE` → `Resized`（`1255-1280`）不受影响。
- `WM_NCHITTEST`（`2206-2258`）用 `util::window_rect`（屏幕）与 lParam 光标屏幕坐标比较 → 一致。
- `WM_NCCALCSIZE`（`2152-2203`）继续返回 0 → 子窗口不会画标题栏；`WS_CAPTION` 残留只影响 `AdjustWindowRect` 类计算（已被 tao 剔除）。

### 5.4 DPI

- `WM_DPICHANGED`（`tao/event_loop.rs:1915-2146`）：更新 `window_state.scale_factor`、发 `ScaleFactorChanged`、按建议矩形 `SetWindowPos`。**该消息只发给顶层窗口**；成为子窗口后 DPI 变化时收到的是 `WM_DPICHANGED_BEFOREPARENT/AFTERPARENT`（tao 未处理）→ `scale_factor()` 陈旧 → `set_outer_position` / `set_inner_size` 的逻辑像素换算错误、前端 `window.scaleFactor()` 错误。WebView2 自行用 `GetDpiForWindow`（wry `util::hwnd_dpi`）与 `ShouldDetectMonitorScaleChanges` 处理光栅缩放，不依赖 tao。
- 在桌面模式下窗口不会被拖到另一显示器，DPI 变化只来自用户改缩放/热插拔，频率低；可在切回顶层时重算。
- `SetParent` 的 DPI awareness 检查见 §3.5；两侧都是 per-monitor aware。

### 5.5 其它会被顶层假设影响的 tao 行为

- `set_focus` → `force_window_active` → `SetForegroundWindow`（`window.rs:175-186, 1500-1527`）：对子窗口相当于激活 Progman（explorer）。
- `set_skip_taskbar` → `ITaskbarList::DeleteTab`（`1529-1539`）：子窗口本来不进任务栏。
- `set_minimized` / `WM_SYSCOMMAND SC_MINIMIZE`：子窗口最小化语义（`WS_MINIMIZE` 图标化到父客户区左下）与顶层不同，Win+D 不会再最小化它，项目的 `restore_if_minimized` 在桌面模式下应停用。
- `WindowFlags::apply_diff` 的 `ShowWindow(SW_SHOW/SW_HIDE)`、`SetWindowPos(HWND_TOPMOST/NOTOPMOST/BOTTOM)`：对子窗口 `HWND_TOPMOST` 无意义，`HWND_BOTTOM` 会压到 DefView 之下。

---

## 6. 现有项目代码的关联点

| 文件 | 关联 |
|---|---|
| `pc/src-tauri/tauri.conf.json:14-29` | `decorations:false, transparent:true, shadow:false, resizable:true` |
| `pc/src-tauri/src/lib.rs:38-59` | `DWMWCP_ROUND`；嵌入时需改 `DONOTROUND` |
| `pc/src-tauri/src/commands/window.rs:831-856, 868-887, 900+` | 固定模式的 ex-style 维护与绕开 tao `apply_diff` 的样板 |
| `pc/src-tauri/src/commands/window.rs:535-541` | `restore_if_minimized`（Win+D 对策，子窗口模式下不再需要） |
| `pc/src-tauri/build.rs` | 仅 `tauri_build::build()`，无自定义 manifest（方案 2 需要） |
| `pc/src/types/app.ts:19-22`, `pc/src/stores/appStore.ts:454-476` | `DEFAULT_BG_ALPHA = 0.45`、`--app-bg-alpha` CSS 变量、`set_window_background` |

### Related Specs

- `.trellis/spec/` 下目前只有 frontend 与 guides 规范，没有 Windows 后端 / 窗口管理相关规范。

---

## 未验证 / 需要原型验证

1. **朴素 `SetParent(Progman)` + `WS_CHILD` 后透明区域到底显示什么**（黑 / 白 / 透出桌面 / 透出但被 DefView 图标遮挡）——本研究没有截图；且要分别在 24H2+（Progman `WS_EX_NOREDIRECTIONBITMAP`）与旧结构（顶层 WorkerW 有重定向表面的 Win10 / Win11 ≤ 23H2）上验证，两者合成路径不同。
2. **方案 2**：`WS_CHILD` + `WS_EX_LAYERED` + `SetLayeredWindowAttributes(0, 255, LWA_ALPHA)` 是否让 WebView2 的 DComp alpha 透出桌面；是否需要再加 `DWMWA_REDIRECTIONBITMAP_ALPHA=TRUE`（26100+）；tao 窗口类 `CS_OWNDC` 是否阻止分层生效；tauri 默认 manifest 缺 `supportedOS` 时分层子窗口是否被系统忽略（需要对比加/不加自定义 manifest）。
3. **方案 2b**：创建后用 `SetWindowLong` 加 `WS_EX_NOREDIRECTIONBITMAP` 是否有效（文档未定义）；或用 `parent_raw(progman)` 从一开始创建为子窗口时 WebView2 能否正常创建、透明是否成立（此时 tao 的 `DwmEnableBlurBehindWindow` 对子窗口必然失败）。
4. 嵌入后的 **Z 序控制**：能否稳定停在 `SHELLDLL_DefView` 之前（"图标之上"），explorer 何时会重排子窗口（切换壁纸、"raised desktop" 状态变化、explorer 重启后父窗口消失需要重挂）。
5. **输入**：作为 Progman 子窗口，鼠标点击/滚轮/拖拽是否直达 WebView2（跨进程输入队列已串联）；键盘焦点（编辑待办标题）是否可获得，`SetFocus`/`SetForegroundWindow` 对跨进程子窗口的实际行为；IME 是否正常。
6. **tao 状态漂移**：切到子窗口后，哪些 tauri 调用（`show/hide`、`setResizable`、`setAlwaysOnTop`、`setPosition`、`setSize`）会触发 `apply_diff` 抹掉 `WS_CHILD`/分层样式，需要逐一列出并在项目里包一层"桌面模式守卫"。
7. **坐标换算**：`set_position` 需要的父客户区偏移在多显示器（本机 Progman 原点 `(-2560,-240)`）与单显示器上的表现；`Moved` 事件在前端的消费者是否受影响。
8. **DPI**：嵌入期间修改显示缩放后，tao `scale_factor()` 陈旧的具体后果，以及切回顶层时是否自愈（`WM_DPICHANGED` 只在真正变化时发）。
9. **圆角**：`DWMWCP_DONOTROUND` 在子窗口状态下是否立即生效，切回顶层后能否恢复 `ROUND`。
10. **方案 1（不重挂）**：`HWND_BOTTOM` 常驻 + 拦截 `(-32000,-32000)` 的 Win+D 对策在 Win11 26200 上是否可靠；`WS_EX_NOACTIVATE` 是否影响文本输入；是否会与项目现有固定模式（贴边隐藏、`restore_if_minimized`）冲突。
11. **WebView2 运行时版本**：运行时 144/145 的透明回归（#5481/#5492）是否影响用户机器；`DefaultBackgroundColor` alpha=0 在子窗口宿主下是否有其它差异。
12. 跨进程父子窗口带来的**稳定性**：explorer 卡顿/重启时对本进程消息循环的影响（输入队列串联），以及 `SetParent(hwnd, None)` 脱离后是否需要重设 `WS_POPUP`/清 `WS_CHILD` 并 `SWP_FRAMECHANGED`。

---

## 引用来源汇总

crate 源码（本地 registry）：
- `tao-0.34.5/src/platform_impl/windows/window.rs` 1116-1353（`init`）、1283-1297（blur-behind）、1355-1380（窗口类）、1382-1436（创建期 `window_proc`）、213-270（位置/尺寸读写）、1152-1167（parent 分支）
- `tao-0.34.5/src/platform_impl/windows/window_state.rs` 77-124（flags）、242-312（`to_window_styles`）、315-464（`apply_diff`）
- `tao-0.34.5/src/platform_impl/windows/event_loop.rs` 677-717（隐藏事件窗口 `WS_EX_LAYERED`）、929（FIXME）、1132-1149（`WM_ERASEBKGND`）、1151-1236（`WM_WINDOWPOSCHANGING`）、1239-1280（`Moved`/`Resized`）、1915-2146（`WM_DPICHANGED`）、2152-2258（`WM_NCCALCSIZE`/`WM_NCHITTEST`）
- `tao-0.34.5/src/platform_impl/windows/util.rs` 65-141、444-502；`dpi.rs` 20-40、71-109
- `tao-0.34.5/src/platform/windows.rs` 255-350（`with_parent_window`/`with_owner_window`/`with_no_redirection_bitmap`）
- `wry-0.53.5/src/webview2/mod.rs` 108-177、179-279、363-411、415-557、1189-1286、1419-1458、1671-1690、1790-1808；`util.rs`
- `tauri-runtime-wry-2.9.3/src/lib.rs` 851、908-909、1105-1117、4586-4589、4993；`undecorated_resizing.rs` 107-140
- `tauri-2.9.5/src/webview/webview_window.rs` 732-783（`parent`/`owner`/`parent_raw`）
- `tauri-build-2.5.3/src/windows-app-manifest.xml`、`src/lib.rs` 222-335

Microsoft Learn：
- https://learn.microsoft.com/en-us/windows/win32/api/dwmapi/nf-dwmapi-dwmenableblurbehindwindow
- https://learn.microsoft.com/en-us/windows/win32/dwm/blur-ovw
- https://learn.microsoft.com/en-us/windows/win32/api/dwmapi/ne-dwmapi-dwmwindowattribute （`DWMWA_REDIRECTIONBITMAP_ALPHA`、`DWMWA_CLOAK`、`DWMWA_WINDOW_CORNER_PREFERENCE`）
- https://learn.microsoft.com/en-us/windows/apps/desktop/modernize/apply-rounded-corners
- https://learn.microsoft.com/en-us/windows/win32/winmsg/extended-window-styles
- https://learn.microsoft.com/en-us/windows/win32/winmsg/window-features （Child Windows / Layered Windows）
- https://learn.microsoft.com/en-us/windows/win32/winmsg/using-windows （Using Layered Windows，manifest 要求）
- https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setlayeredwindowattributes
- https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-updatelayeredwindow
- https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setparent
- https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setwindowpos
- https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getwindowrect
- https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getdpiforwindow
- https://learn.microsoft.com/en-us/windows/win32/hidpi/wm-dpichanged
- https://learn.microsoft.com/en-us/windows/win32/hidpi/wm-dpichanged-afterparent
- https://learn.microsoft.com/en-us/windows/win32/api/dcomp/nf-dcomp-idcompositiondevice-createtargetforhwnd
- https://learn.microsoft.com/en-us/windows/win32/directcomp/basic-concepts
- https://learn.microsoft.com/en-us/windows/win32/directcomp/how-to--animate-the-bitmap-of-a-layered-child-window
- https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/windowed-vs-visual-hosting
- https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/icorewebview2controller2
- https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/icorewebview2compositioncontroller

其它：
- Raymond Chen, https://devblogs.microsoft.com/oldnewthing/20130412-00/?p=4683
- Chromium `ui/gl/child_window_win.cc`, `ui/gl/dcomp_presenter.cc`（chromium.googlesource.com, main）
- https://github.com/rocksdanister/lively （`core-separation`: `Lively.Player.WebView2/Form1.cs`, `Lively/Core/WinDesktopCore.cs`）
- https://github.com/meslzy/electron-as-wallpaper （`src/attach.rs`, `src/window.rs`）
- https://github.com/meslzy/tauri-plugin-wallpaper （`src/platform/windows/{attacher,corners,pinner,input,helper}.rs`, `readme.md`）
- https://github.com/Charlie-XIAO/tauri-plugin-desktop-underlay （`src/core/windows.rs`, `FAQ.md`, issue #85）
- https://github.com/rainmeter/rainmeter （`Library/System.cpp`, `Library/Skin.cpp`）
- https://github.com/tauri-apps/tauri/issues/4261 、/issues/15947 、/pull/15410
- https://github.com/tauri-apps/wry/pull/1762
- https://github.com/MicrosoftEdge/WebView2Feedback/issues/985 、/issues/5668 、/issues/5481 、/issues/5492 、/issues/20 、/issues/547
- 本机实测脚本（只读枚举窗口树）：`C:\Users\12197\AppData\Local\Temp\claude\D--Git-mini-todo\41d43c4b-0dab-420d-8e8a-3eac927cbf98\scratchpad\desktop-tree3.ps1`、`minitodo-tree.ps1`
