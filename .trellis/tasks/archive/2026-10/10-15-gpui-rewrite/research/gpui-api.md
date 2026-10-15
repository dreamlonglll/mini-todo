# Research: GPUI（gpui-kit 0.7.3 / gpui-pre 0.3.9 / gpui-component 0.7.3）面向实现者的 API 速查

- **Query**: 用 Rust + GPUI 重写 mini-todo（Windows 桌面端），整理 gpui-kit `=0.7.3` 体系（gpui-pre 0.3.9、gpui-pre-windows、gpui-pre-platform、gpui-base 0.7.3、gpui-component 0.7.3、gpui-kit-assets 0.7.3）的应用/窗口/渲染/输入/拖放/平台能力/组件/测试 API，签名以本机源码为准
- **Scope**: internal（只读 cargo registry 源码）+ 本机实测（scratchpad 下独立探针工程，未动仓库与 `/tmp/claude-0/gpui-probe` 的构建）
- **Date**: 2026-10-15

> 路径约定：下文 `$REG` = `~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f`。
> `G` = `$REG/gpui-pre-0.3.9`，`W` = `$REG/gpui-pre-windows-0.3.9`，`L` = `$REG/gpui-pre-linux-0.3.9`，
> `P` = `$REG/gpui-pre-platform-0.3.9`，`WG` = `$REG/gpui-pre-wgpu-0.3.9`，`B` = `$REG/gpui-base-0.7.3`，
> `C` = `$REG/gpui-component-0.7.3`，`A` = `$REG/gpui-kit-assets-0.7.3`，`K` = `$REG/gpui-kit-0.7.3`。
>
> 导入约定：`use gpui_kit::*;` 就是 GPUI 本体（`K/src/lib.rs`：`pub use ::gpui::*;`）；`gpui_kit::component`
> = gpui-component，`gpui_kit::base` = gpui-base，`gpui_kit::assets` = gpui-kit-assets，`gpui_kit::platform` =
> gpui-pre-platform。GPUI 的 derive 宏（`IntoElement`/`Action`/`Render`）输出里的 `gpui::` 路径会被
> `G-macros/src/gpui_pre_facade_paths.rs` 自动改写成 `gpui_kit::`，所以**应用只需依赖 gpui-kit 一个 crate，不需要 `extern crate gpui_kit as gpui`**。

---

## 0. 结论速览（影响架构选择的限制，按重要性）

1. **没有"移动窗口 / 显示隐藏窗口 / 置底 / 改 ex style"的 GPUI API**。`Window` 只有 `resize(size)`（`W/src/window.rs:655`，`SWP_NOMOVE`）、`minimize_window`、`zoom_window`、`activate_window`、`remove_window`。改位置、`ShowWindow(SW_HIDE)`、owner=Progman、`WS_EX_TOOLWINDOW` 等都要拿 HWND 自己调 Win32。GPUI **不会**回写 `GWL_EXSTYLE`（Windows 后端只在全屏切换时改 `GWL_STYLE`），也**不处理 `WM_WINDOWPOSCHANGING`**；要"永远置底"必须子类化窗口过程（`SetWindowSubclass`），并且**不能碰 `GWLP_USERDATA`**（GPUI 用它存窗口状态，`W/src/window.rs:1455`）。
2. **普通窗口带 `WS_EX_APPWINDOW`**（进任务栏）；`WindowKind::PopUp` = `WS_EX_TOOLWINDOW | WS_EX_TOPMOST`、style 0（不可缩放）；`Floating` 在 Windows 上与 `Normal` 完全相同；`Dialog` 以 `GetActiveWindow()` 为 owner 并禁用它（模态）。透明背景走 DirectComposition（`WS_EX_NOREDIRECTIONBITMAP` + 预乘 alpha 交换链）+ `SetWindowCompositionAttribute`。（`W/src/window.rs:492-515, 920-940`）
3. **Windows 上 `start_window_move()` 是空操作**（只在 macOS/Linux 实现）；自绘标题栏拖动靠 `.window_control_area(WindowControlArea::Drag)` → `WM_NCHITTEST` 返回 `HTCAPTION`（`W/src/events.rs:955-980`），`is_movable: false` 时 Drag 区域失效。
4. **默认"关掉最后一个窗口就退出"**（`QuitMode::Default` 在非 macOS = `LastWindowClosed`，`G/src/app.rs:3180`）。托盘常驻必须 `cx.set_quit_mode(QuitMode::Explicit)` 或 `Application::with_quit_mode(QuitMode::Explicit)`。
5. **GPUI 没有托盘、没有单实例、没有开机自启**；但**有**系统通知（Windows Toast，需先 `cx.set_app_identity`）、原生 TaskDialog（`window.prompt`）、文件打开/保存对话框（**无扩展名过滤**）、剪贴板图片、`open_url`/`reveal_path`/`open_with_system`。主线程是标准 `GetMessageW/DispatchMessageW` 循环（`W/src/platform.rs:597`），所以在主线程创建的托盘隐藏窗口（如 `tray-icon` crate）能正常收消息。
6. **Windows 构建限制**：release 构建的 build.rs 必须在 **Windows 主机**上找到 `fxc.exe`（Windows SDK 或 `GPUI_FXC_PATH`）预编译 HLSL；debug 构建在**运行时**从 `env!("CARGO_MANIFEST_DIR")/src/shaders.hlsl`（构建机 cargo registry 的绝对路径）编译着色器 → **debug exe 拷到别的机器跑不起来**；从 Linux 交叉编译 Windows release 会因缺 `shaders_bytes.rs` 编译失败（`W/build.rs`、`W/src/directx_renderer.rs:1830,1869`）。gpui 默认 feature `windows-manifest` 已嵌入 PerMonitorV2 DPI + Common-Controls v6 清单，**应用不要再嵌第二份 manifest**；窗口图标取 exe 里资源 ID 1 的 ICON。
7. **gpui-component 的 `TextView`（Markdown）不暴露图片解析钩子**；要把 `minitodo-image://<name>` 映射到本地文件，需直接用 **`gpui_kit::base::TextView::markdown(..).image_source(|uri| ..)`**（已编译验证）。另外 **`img("C:\\a.png")` 这种 `&str` 会被当成 URI**（`url::Url` 能把 `C:` 解析成 scheme），本地文件必须传 `PathBuf`/`&Path`。
8. **无头截图 + 交互测试在本机 Linux 已跑通**：`HeadlessAppContext::with_platform(.., gpui_kit::platform::current_headless_renderer)` 在**无 DISPLAY** 时用 wgpu + mesa lavapipe 渲出 PNG（含中文），并能 `window.click("id")` / `find("id").checked()`；另外真实 X11 窗口在 `Xvfb + lavapipe` 下渲染正常、`xdotool` 点击能触发 `on_click`。注意：开启 `gpui-kit/test-support` 会把 gpui-fast 系列拉进 Cargo.lock（弱依赖特性导致），沿用旧 lock 会出现 `cocoa`/`unicode-properties` 版本冲突，需重新生成 lock。
9. 其它易错点：`AsyncApp::update` 返回 `R`（app 已释放时 **panic**），弱引用请用 `WeakEntity::update(cx, ..) -> Result<R>`；Root 默认铺满主题背景色（透明窗口要覆盖 Root 的 `bg`）；GPUI 内容遮罩只有矩形，**圆角容器裁不掉子元素背景**；Windows 上 GPUI 窗口收到 `WM_MOUSEACTIVATE` 一律 `MA_ACTIVATE`（点击即激活）；拖入文件 Windows 只认 `CF_HDROP`，并且对系统**总是回 `DROPEFFECT_COPY`**。

---

## A. 应用与窗口（gpui-pre）

### A1. 启动、Root、退出

**结论**：`gpui_kit::application()` → `.with_assets(..)` → `.run(|cx| { gpui_kit::init(cx); ... })`。用 gpui-component 时窗口根视图必须是 `gpui_kit::base::Root`（`gpui_kit::open_window` 自动包一层；`cx.open_window` 需要你自己包）。

签名（源码）：

```rust
// P/src/gpui_platform.rs:12      （gpui_kit::application 即此函数）
pub fn application() -> gpui::Application;
pub fn headless() -> gpui::Application;
pub fn current_platform(headless: bool) -> Rc<dyn Platform>;

// G/src/app.rs:177-345  impl Application
pub fn with_assets(self, asset_source: impl AssetSource) -> Self;
pub fn with_quit_mode(self, mode: QuitMode) -> Self;          // Default | LastWindowClosed | Explicit
pub fn with_http_client(self, http_client: Arc<dyn HttpClient>) -> Self; // 默认 NullHttpClient：http 图片加载不了
pub fn run<F>(self, on_finish_launching: F) where F: 'static + FnOnce(&mut App);
pub fn on_open_urls<F>(&self, callback: F) -> &Self where F: 'static + FnMut(Vec<String>);
pub fn on_reopen<F>(&self, callback: F) -> &Self where F: 'static + FnMut(&mut App);

// K/src/lib.rs:167-201
pub fn init(cx: &mut App);   // = gpui_component::init（含 gpui_base::init、主题、按键绑定等），必须先于开窗
pub fn open_window<V: Render>(
    options: WindowOptions,
    cx: &mut App,
    build: impl FnOnce(&mut Window, &mut App) -> Entity<V>,
) -> Result<(AnyWindowHandle, Entity<V>)>;   // 内部 cx.open_window + Root::new(view, ..)

// G/src/app.rs:1500
pub fn open_window<V: 'static + Render>(
    &mut self,
    options: crate::WindowOptions,
    build_root_view: impl FnOnce(&mut Window, &mut App) -> Entity<V>,
) -> anyhow::Result<WindowHandle<V>>;

// 退出相关  G/src/app.rs
pub fn quit(&self);                                   // Windows: PostQuitMessage(0)
pub fn set_quit_mode(&mut self, mode: QuitMode);      // :1955
pub fn on_app_quit<Fut>(&self, on_quit: impl FnMut(&mut App) -> Fut + 'static) -> Subscription
    where Fut: 'static + Future<Output = ()>;         // :2716，不能取消退出
pub fn on_window_closed(&self, on_closed: impl FnMut(&mut App, WindowId) + 'static) -> Subscription;
```

- `QuitMode::Default`：macOS = Explicit，其它平台 = LastWindowClosed（`G/src/app.rs:456-466, 3180-3188`）。Windows 平台层自身不会因为窗口为 0 而退出（`W/src/platform.rs:1269 close_one_window` 只维护列表），所以 `Explicit` 下 0 个窗口也能常驻。
- `gpui_kit::open_window` vs `cx.open_window`：前者返回 `(AnyWindowHandle, Entity<V>)`，窗口根是 `base::Root`；后者返回 `WindowHandle<V>`，根是你给的视图。**gpui-component 的 Dialog/Notification/Sheet/Tooltip 层都挂在 Root 的插件上**（`C/src/root.rs:20`，缺 Root 时 `window.open_dialog` 会 panic："component window state is missing"），所以用组件的窗口一律要有 Root。
- Root 默认样式：`font_family(theme.font_family)`、`bg(theme.tokens.background)`、`text_color(theme.foreground)`（`C/src/root.rs:443-452`）；Root 实例上设置的样式会覆盖插件默认值（`B/src/root.rs:53-57`）。透明窗口需要自己构造 Root 并覆盖背景（见 A2 示例）。

```rust
use gpui_kit::*;
use gpui_kit::component::*;

fn main() {
    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)        // 组件默认图标；需要更多图标见 B3
        .with_quit_mode(QuitMode::Explicit)          // 托盘常驻
        .run(|cx| {
            gpui_kit::init(cx);
            cx.set_app_identity("com.minitodo.app", "Mini Todo"); // 系统通知前必须设置（A8）
            let (_win, _view) = gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| MainView::new(window, cx))
            })
            .expect("open window");
        });
}
```

### A2. `WindowOptions` 全字段与 Windows 映射

`G/src/platform.rs:2491-2560`（默认值 `:2662-2686`）：

| 字段 | 类型 / 默认 | Windows 实现（`W/src/window.rs:438-592`） |
|---|---|---|
| `window_bounds` | `Option<WindowBounds>` / `None`（None 时按活动窗口层叠或显示器居中 1536×1095，`G/src/window.rs:1508`） | 逻辑像素；见下方坐标说明 |
| `titlebar` | `Option<TitlebarOptions>` / `Some{title:None, appears_transparent:false, ..}` | `None` 或 `appears_transparent: true` ⇒ `hide_title_bar=true`：`WM_NCCALCSIZE` 去掉标题栏（保留可缩放窗口的左右下边框作为缩放热区），自己做 resize hit-test |
| `focus` | `bool` / `true` | `false` ⇒ `SW_SHOWNOACTIVATE` |
| `show` | `bool` / `true` | `false` ⇒ 创建但不显示，初始 placement 存起来，`activate_window()` 时应用（未实测，建议自己 `ShowWindow`） |
| `kind` | `WindowKind` / `Normal` | `PopUp` ⇒ exstyle `WS_EX_TOOLWINDOW|WS_EX_TOPMOST`、style `0`；`Dialog` ⇒ `WS_POPUP|WS_CAPTION` + `WS_EX_DLGMODALFRAME`、owner=活动窗口并 `EnableWindow(owner,false)`；`Normal`/`Floating` ⇒ `WS_SYSMENU`(+`WS_THICKFRAME|WS_MAXIMIZEBOX` 若可缩放)(+`WS_MINIMIZEBOX` 若可最小化) + `WS_EX_APPWINDOW`；`AnchoredPopup` ⇒ Windows 直接返回 `PopupNotSupportedError` |
| `is_movable` | `bool` / `true` | 仅影响 `WindowControlArea::Drag` 是否返回 `HTCAPTION`；程序移动不受影响 |
| `app_owns_titlebar_drag` | `bool` / `false` | 仅 macOS |
| `inactive_frame_interval` | `Option<Duration>` / `Some(33.333ms)` | 窗口不活动时动画帧节流到 ~30fps；桌面嵌入窗口长期不活动，要满帧动画设 `None` |
| `is_resizable` | `bool` / `true` | 决定 `WS_THICKFRAME|WS_MAXIMIZEBOX` 与自绘缩放热区 |
| `is_minimizable` | `bool` / `true` | `WS_MINIMIZEBOX` |
| `display_id` | `Option<DisplayId>` / `None`（主显示器） | `DisplayId` = `HMONITOR` 值；bounds 中心不在该显示器上时改用该显示器的居中默认位置（`W/src/window.rs:1575-1601`） |
| `window_background` | `WindowBackgroundAppearance` / `Opaque` | `Transparent` ⇒ `SetWindowCompositionAttribute(ACCENT=2)`；`Blurred` ⇒ ACCENT=4（亚克力）；`MicaBackdrop`/`MicaAltBackdrop` ⇒ `DWMWA_SYSTEMBACKDROP_TYPE`（Win11 22621+） |
| `app_id` | `Option<String>` | Windows 未使用（Linux/Wayland） |
| `window_min_size` | `Option<Size<Pixels>>` | `WM_GETMINMAXINFO` |
| `window_decorations` | `Option<WindowDecorations>`（`Server`/`Client`） | 仅 X11/Wayland；Windows 永远是 `Decorations::Server` |
| `icon` | `Option<Arc<image::RgbaImage>>` | 仅 X11；Windows 图标来自 exe 资源 ID 1（`W/src/platform.rs:1633 load_icon`） |
| `tabbing_identifier` | `Option<String>` | 仅 macOS |

**透明实现与对改 ex style 的影响**：
- 交换链：默认 `CreateSwapChainForComposition` + `DXGI_ALPHA_MODE_PREMULTIPLIED`（DirectComposition），窗口带 `WS_EX_NOREDIRECTIONBITMAP`；设环境变量 `GPUI_DISABLE_DIRECT_COMPOSITION=1|true` 会改用 `CreateSwapChainForHwnd` + `ALPHA_MODE_IGNORE`，**透明失效**（`W/src/directx_renderer.rs:26, 1310-1350`、`W/src/platform.rs:249`）。
- 窗口类 `Zed::Window`、`hbrBackground` 黑色画刷（`W/src/window.rs:1424-1440`）。
- 之后自己 `SetWindowLongPtrW(GWL_EXSTYLE, ..)` 增删 `WS_EX_TOOLWINDOW/WS_EX_APPWINDOW/WS_EX_NOACTIVATE` 不会被 GPUI 覆盖（GPUI 不再写 exstyle）。**不要加 `WS_EX_LAYERED`**（与 `WS_EX_NOREDIRECTIONBITMAP`/DirectComposition 路线冲突；源码推断，未实测）。
- 所有窗口都会 `SetPropW("NonRudeHWND")`（避免无边框窗口被当作全屏、挡任务栏，`W/src/window.rs:1678`）。

**透明无边框窗口示例（已编译验证）**：

```rust
fn open_transparent(cx: &mut App) -> gpui_kit::Result<WindowHandle<gpui_kit::base::Root>> {
    cx.open_window(
        WindowOptions {
            titlebar: None,                                        // 无系统标题栏
            window_background: WindowBackgroundAppearance::Transparent,
            kind: WindowKind::Normal,
            is_resizable: false,
            focus: false,
            window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                point(px(100.), px(100.)), size(px(360.), px(600.)),
            ))),
            ..Default::default()
        },
        |window, cx| {
            let view = cx.new(|cx| MainView::new(window, cx));
            // Root 默认 bg = theme 背景色；实例样式优先级更高，覆盖成透明
            cx.new(|cx| gpui_kit::base::Root::new(view, window, cx).bg(transparent_black()))
        },
    )
}
```

**坐标单位（Windows）**：`Bounds<Pixels>` 是**逻辑像素、全局屏幕坐标**，但换算因子是"该显示器自己的缩放"：逻辑原点 = 物理原点 / 该显示器 DPI 缩放（`W/src/display.rs:40-70`，`logical_point(x, y, scale)`）。也就是说多显示器混合 DPI 时，各显示器的逻辑坐标系并不连续。`WindowsDisplay::bounds()` = `rcMonitor`/scale，`visible_bounds()` = `rcWork`/scale（工作区），`physical_bounds()`（内部）= 物理像素。直接调 `SetWindowPos` 时请用"逻辑值 × 目标显示器 scale"。

### A3. 原生句柄、窗口几何、显示器、窗口事件

**HWND**：`Window` 实现 `raw_window_handle::HasWindowHandle`（`G/src/window.rs:7572`），但 `Window` 还有同名固有方法 `window_handle() -> AnyWindowHandle`，**必须用 UFCS 调 trait 方法**；需自行依赖 `raw-window-handle = "0.6"`（gpui 未 re-export）。

```rust
fn hwnd_of(window: &Window) -> Option<isize> {             // 已编译验证
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    match HasWindowHandle::window_handle(window).ok()?.as_raw() {
        RawWindowHandle::Win32(h) => Some(h.hwnd.get()),   // NonZeroIsize
        _ => None,
    }
}
```

**窗口读写 API**（`G/src/window.rs`）：

```rust
pub fn bounds(&self) -> Bounds<Pixels>;            // :2863 全局逻辑坐标（客户区）
pub fn window_bounds(&self) -> WindowBounds;       // :2524 含 Maximized/Fullscreen 状态与还原尺寸
pub fn viewport_size(&self) -> Size<Pixels>;       // :2928
pub fn resize(&mut self, size: Size<Pixels>);      // :2892 只改尺寸
pub fn scale_factor(&self) -> f32;                 // :3091
pub fn display(&self, cx: &App) -> Option<Rc<dyn PlatformDisplay>>; // :3076
pub fn is_maximized(&self) -> bool;  pub fn is_fullscreen(&self) -> bool;
pub fn is_window_active(&self) -> bool;  pub fn is_window_hovered(&self) -> bool;
pub fn activate_window(&self);     // :6625 Windows: 还原最小化 + SetActiveWindow/SetFocus + 模拟 Alt + SetForegroundWindow
pub fn minimize_window(&self);     // :6635 ShowWindowAsync(SW_MINIMIZE)
pub fn zoom_window(&self);         // :2993 SW_MAXIMIZE
pub fn toggle_fullscreen(&self);
pub fn remove_window(&mut self);   // :2384 关闭并销毁窗口
pub fn start_window_move(&self);   // :3006 Windows 为空实现！
pub fn window_controls(&self) -> WindowControls;    // Windows 返回默认值
pub fn set_window_title(&mut self, title: &str);
pub fn set_background_appearance(&self, b: WindowBackgroundAppearance); // 运行时切换透明/不透明
pub fn mouse_position(&self) -> Point<Pixels>;      // 窗口内坐标
pub fn modifiers(&self) -> Modifiers;
```

**显示器**（`G/src/app.rs:1556-1590`、`G/src/platform.rs:607-643`）：

```rust
cx.displays() -> Vec<Rc<dyn PlatformDisplay>>;  cx.primary_display();  cx.find_display(id);
trait PlatformDisplay {
    fn id(&self) -> DisplayId;                 // Windows: HMONITOR，不稳定（插拔后会变）
    fn uuid(&self) -> Result<Uuid>;            // Windows: 由 szDevice(\\.\DISPLAYn) 生成的 v5 UUID，可持久化
    fn bounds(&self) -> Bounds<Pixels>;        // 整屏（逻辑）
    fn visible_bounds(&self) -> Bounds<Pixels>;// 工作区（排除任务栏）
    fn default_bounds(&self) -> Bounds<Pixels>;
    fn refresh_interval(&self) -> Option<Duration>;
}
cx.observe_displays(|event: DisplayEvent, cx| ..) -> Subscription;  // Added/Removed/Changed{..}
Bounds::centered(display_id: Option<DisplayId>, size, cx);  Bounds::maximized(display_id, cx);
```

**窗口事件订阅**（`G/src/app/context.rs:402-500`，`G/src/window.rs:2298-2330, 6870`）：

```rust
cx.observe_window_bounds(window, |this, window, cx| { /* WM_MOVE 与 WM_SIZE 都会触发 */ }) -> Subscription;
cx.observe_window_activation(window, |this, window, cx| { window.is_window_active(); });
cx.observe_window_appearance(window, |this, window, cx| { window.appearance(); }); // 系统明暗
cx.observe_window_visibility(window, |this, window, cx| { .. });
cx.observe_window_display(window, |this, window, cx| { .. });   // 跨显示器
window.on_window_should_close(cx, |window, cx| -> bool { false /* 返回 false 阻止关闭(Alt+F4/WM_CLOSE) */ });
cx.on_window_closed(|cx, window_id| { .. });                     // App 级
```

`Subscription` 被 drop 就失效，记得存进视图字段或 `.detach()`。

### A4. 多窗口与跨线程

**结论**：同进程内多窗口共享一个 `App`；跨窗口通信用 `Entity`（共享模型 + `cx.observe/subscribe`）、`Global`、或 `WindowHandle::update`。非 GPUI 线程 → UI：**async channel + `cx.spawn` 前台任务循环 `recv().await`**（无需轮询，已编译验证）。

```rust
// 句柄（G/src/window.rs:7369-7570）
impl<V: 'static + Render> WindowHandle<V> {
    pub fn update<C, R>(&self, cx: &mut C, update: impl FnOnce(&mut V, &mut Window, &mut Context<V>) -> R) -> Result<R>;
    pub fn read<'a>(&self, cx: &'a App) -> Result<&'a V>;
    pub fn entity<C>(&self, cx: &C) -> Result<Entity<V>>;
    pub fn is_active(&self, cx: &mut App) -> Option<bool>;
}
impl AnyWindowHandle {
    pub fn window_id(&self) -> WindowId;
    pub fn downcast<T: 'static>(&self) -> Option<WindowHandle<T>>;
    pub fn update<C, R>(self, cx: &mut C, update: impl FnOnce(AnyView, &mut Window, &mut App) -> R) -> Result<R>;
}
cx.windows() -> Vec<AnyWindowHandle>;   cx.active_window() -> Option<AnyWindowHandle>;
cx.update_window(handle, |root_view, window, cx| ..) -> Result<T>;   // AppContext

// Global（G/src/global.rs）
impl Global for MyState {}
cx.set_global(MyState{..}); cx.global::<MyState>(); cx.global_mut::<MyState>();
cx.update_global::<MyState, _>(|g, cx| ..); cx.observe_global::<MyState>(|cx| ..);

// 实体（G/src/app/entity_map.rs:485-530, 792-845）
entity.update(cx, |this, cx| ..) -> R;                 // Entity
weak.update(cx, |this, cx| ..) -> Result<R>;           // WeakEntity，实体已释放返回 Err
weak.update_in(cx, |this, window, cx| ..) -> Result<R>;
cx.notify(); cx.emit(evt); cx.subscribe(&other, |this, other, evt: &E, cx| ..); cx.observe(&other, |this, other, cx| ..);
cx.subscribe_in(&other, window, |this, other, evt: &E, window, cx| ..);

// 异步（G/src/app/context.rs:230, 690；G/src/app/async_context.rs）
cx.spawn(async move |weak_this: WeakEntity<T>, cx: &mut AsyncApp| { .. }) -> Task<R>;   // Context<T>
cx.spawn_in(window, async move |weak_this, cx: &mut AsyncWindowContext| { .. });
cx.spawn(async move |cx: &mut AsyncApp| { .. });                                         // App
cx.background_executor().spawn(async move { /* Send，跑线程池 */ }) -> Task<R>;
cx.background_executor().timer(Duration) -> Task<()>;   // 没有独立的 gpui::Timer 类型
task.detach(); task.detach_and_log_err(cx);              // Task 被 drop 即取消
AsyncApp::update(&self, |cx: &mut App| ..) -> R;         // 注意：app 已释放时 panic
```

**推荐写法：后台线程（如 notify 文件监听、tray-icon 回调、WebDAV 同步线程）→ UI**：

```rust
fn background_to_ui(cx: &mut App, view: Entity<MainView>) {     // 已编译验证
    let (tx, rx) = async_channel::unbounded::<String>();         // async-channel 2.5（与 gpui 同版本）
    std::thread::spawn(move || {
        let _ = tx.send_blocking("from worker".to_string());     // 任意非 GPUI 线程
    });
    let weak = view.downgrade();
    cx.spawn(async move |cx| {
        while let Ok(msg) = rx.recv().await {                   // 前台执行器，唤醒即 PostMessage 到主线程
            if weak.update(cx, |this, cx| { this.items.push(msg.into()); cx.notify(); }).is_err() {
                break;                                           // 视图已销毁
            }
        }
    })
    .detach();
}

// 周期任务（5s 轮询 change_seq 之类）
fn periodic(cx: &mut Context<MainView>) {
    cx.spawn(async move |this, cx| loop {
        cx.background_executor().timer(Duration::from_secs(5)).await;
        let r = cx.background_executor().spawn(async move { /* 阻塞 IO / SQL */ 42u32 }).await;
        if this.update(cx, |this, cx| cx.notify()).is_err() { break; }
    })
    .detach();
}
```

### A5. 渲染与样式

**结论**：视图 = `impl Render`（有状态，`Entity<V>`）；无状态组件 = `#[derive(IntoElement)] + impl RenderOnce`。`div()` 是 Tailwind 风格 builder；交互状态（hover/active/滚动/拖拽/tooltip）需要 `.id(..)`（变成 `Stateful<Div>`）。

```rust
pub trait Render: 'static + Sized { fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement; }
pub trait RenderOnce: 'static { fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement; }
```

**Styled 方法**（`G/src/styled.rs` + 宏 `G-macros/src/styles.rs`）：
- 布局：`flex() flex_col() flex_row() flex_1() flex_none() flex_grow() flex_shrink_0() flex_wrap() items_center() items_start() justify_between() justify_center() gap_N() gap(px) grid() grid_cols(n) col_span(n)`；gpui-component 另有 `h_flex()/v_flex()` 函数与 `.h_flex()/.v_flex()` 方法（`B/src/styled.rs:38-120`）。
- 尺寸：`w(px|rems|relative)`、`h(..)`、`size(..)`、`min_w/min_h/max_w/max_h(..)`、`size_full() w_full() h_full()`；后缀刻度 `_0 _0p5 _1 … _128 _auto _px _full _1_2 _1_3…`（4px = 1 档 = 0.25rem）。
- 间距：`p_N px_N py_N pt/pb/pl/pr`、`m_N mx/my/mt/...`，或带参版本 `p(px(6.))`。
- 圆角：`rounded_none/xs/sm/md/lg/xl/2xl/3xl/full`（2/4/6/8/12/16/24/9999px，rem 基准），`rounded(px(..))`，`rounded_t/b/l/r/tl/...`。
- 边框：`border_1()` … `border_8()`、`border_t_1()` 等、`border(px)`、`border_color(c)`、`border_dashed()`。
- 背景/阴影/透明：`bg(c | linear_gradient(..))`、`shadow_2xs/xs/sm/md/lg/xl/2xl/none`、`shadow(vec![BoxShadow::new(ox, oy, color).blur_radius(px).spread_radius(px).inset()])`（`G/src/style.rs:349-392`）、`opacity(f32)`。
- 文本：`text_color(c) text_size(px) text_xs/sm/base/lg/xl/2xl/3xl font_weight(FontWeight::BOLD) font_family("..") line_height(..) italic() underline() line_through() text_center()`；溢出：`truncate()`、`text_ellipsis()`、`line_clamp(n)`、`whitespace_nowrap()`、`text_overflow(..)`。
- 定位：`relative() absolute() top_N/left_N/right/bottom/inset_0`、`top(px(..))`。**没有 z-index**：后绘制者在上（子元素顺序），浮层用 `deferred(..)`（延迟到本帧最后绘制，`with_priority(n)` 控制层级），遮挡用 `.occlude()`/`.block_mouse_except_scroll()`。
- 溢出：`overflow_hidden()`；滚动 `overflow_y_scroll()`/`overflow_x_scroll()`（仅 `Stateful`，需 `.id()`）+ `.track_scroll(&ScrollHandle)`（`ScrollHandle::new()`、`offset()`、`scroll_to_item(ix)`、`bounds_for_item(ix)`）。gpui-component 的 `ScrollableElement::overflow_y_scrollbar()` 带样式化滚动条。
- 光标：`cursor_pointer() cursor_move() cursor_grab() cursor_text() cursor_ew_resize() cursor_nwse_resize()` 等；`cursor(CursorStyle::..)`。
- 可见性：`visible() invisible() hidden()`。

**颜色/长度构造**：`rgb(0xRRGGBB) -> Rgba`、`rgba(0xRRGGBBAA)`、`hsla(h,s,l,a)`（**全部 0..1**，`G/src/color.rs:424`）、`white() black() transparent_black()`；`Hsla::opacity(f)`、`.alpha(a)`、`.blend(other)`；gpui-component `hsl(h°, s%, l%)`（0..360/0..100）和 `Colorize` trait（`lighten/darken/mix/to_hex/parse_hex`，`C/src/theme/color.rs`）。长度：`px(f32)`、`rems(f32)`（1rem = Theme.font_size，组件把 rem 设为主题字号）、`relative(0.5)`、`auto()`；`point(x,y)`、`size(w,h)`、`Bounds::new(origin,size)`。

**条件与分组**：`FluentBuilder`（prelude）：`.when(cond, |this| ..)`、`.when_some(opt, |this, v| ..)`、`.when_else(..)`、`.map(|this| ..)`。`.group("row")` + 子元素 `.group_hover("row", |s| s.visible())` / `.group_active(..)`；`.hover(|s| s.bg(..))`、`.active(|s| ..)`（active 仅 Stateful）、`.focus(|s| ..)`。

**图片**（`G/src/elements/img.rs`）：

```rust
pub fn img(source: impl Into<ImageSource>) -> Img;
// ImageSource::{Resource(Uri|Path|Embedded), Render(Arc<RenderImage>), Image(Arc<Image>), Custom(fn)}
img(PathBuf::from(r"C:\..\a.png")).w(px(200.)).object_fit(ObjectFit::Contain)   // Fill|Contain|Cover|ScaleDown|None
    .with_fallback(|| div().child("裂图").into_any_element())
    .with_loading(|| div().into_any_element());
```

- `&str/String`：能被 `url::Url` 解析就当 URI（经 http client 拉取），否则当 **AssetSource 路径**（Embedded）；**Windows 绝对路径字符串会被误判成 URI**，本地文件请用 `PathBuf/&Path/Arc<Path>`（走 `fs::read`，`img.rs:634`）。
- 剪贴板/内存图片：`Arc<Image>`（`Image::from_bytes(ImageFormat::Png, bytes)`）。
- 加载结果进 App 级资源缓存，不会因文件变化刷新；需要时 `ImageSource::remove_asset`/`cx.remove_asset`。

**SVG**（`G/src/elements/svg.rs`）：`svg().path("icons/x.svg")`（从 AssetSource 读）/`.external_path(..)`（磁盘路径）/`.data(&bytes)`；**单色蒙版，必须设置 `text_color(..)` 才会绘制**；`with_transformation(Transformation::rotate(radians|percentage) / scale / translate)`（只影响绘制不影响布局）。

**动画**（`G/src/elements/animation.rs`）：

```rust
Animation::new(Duration).repeat().with_easing(ease_in_out)   // linear/quadratic/ease_in_out/ease_out_quint()/bounce(..)/pulsating_between(..)
element.with_animation(id, animation, |el, delta: f32| el.opacity(delta));   // AnimationExt，id 必须稳定
// 转圈（已编译验证；与 gpui-component Spinner 同法）
Icon::new(IconName::LoaderCircle).small().with_animation(
    "spin", Animation::new(Duration::from_secs(1)).repeat(),
    |icon, delta| icon.transform(Transformation::rotate(percentage(delta))));
```

动画遵守 `cx.reduce_motion()`（`cx.set_reduce_motion(true)` 时直接渲染最终态；测试截图时有用）。gpui-component 也有现成 `spinner::Spinner::new()`。

**低级绘制与浮层**：`canvas(|bounds, window, cx| prepaint_state, |bounds, state, window, cx| { window.paint_quad(..) })`；`deferred(anchored().position(point).anchor(Anchor::TopLeft).snap_to_window().child(menu)).with_priority(1)`（`Anchor`: TopLeft/TopRight/BottomLeft/BottomRight/TopCenter/BottomCenter…；`position_mode(AnchoredPositionMode::Window|Local)`）。

### A6. 输入事件、动作、焦点

```rust
// G/src/elements/div.rs:768-1306（InteractiveElement）与 :1306-1720（StatefulInteractiveElement）
.on_mouse_down(MouseButton::Left, |ev: &MouseDownEvent, window, cx| ..)   // ev.position/modifiers/click_count
.on_mouse_up(MouseButton::Left, ..)  .on_mouse_move(|ev: &MouseMoveEvent, ..| ..)  .on_mouse_down_out(..)
.on_scroll_wheel(|ev: &ScrollWheelEvent, ..| ..)  // ev.delta: ScrollDelta::{Lines(Point<f32>), Pixels(Point<Pixels>)}, ev.modifiers.control
.on_key_down(|ev: &KeyDownEvent, ..| ..)          // ev.keystroke.{key, key_char, modifiers}, ev.is_held
.capture_key_down(..)  .on_modifiers_changed(..)
.on_click(|ev: &ClickEvent, window, cx| ..)       // Stateful；ev.click_count()、ev.is_right_click()、ev.modifiers()
.on_aux_click(..)  .on_hover(|hovered: &bool, ..| ..)  .tooltip(|window, cx| -> AnyView)
// gpui-base：.on_double_click(..)（InteractiveElementExt，仅 Stateful，B/src/event.rs:44）
cx.listener(|this: &mut V, ev: &E, window, cx: &mut Context<V>| ..)   // 把回调接到视图状态
cx.stop_propagation(); window.prevent_default();
```

Ctrl+滚轮缩放（已编译验证）：

```rust
.on_scroll_wheel(cx.listener(|this, ev: &ScrollWheelEvent, _, cx| {
    if ev.modifiers.control {
        let dy = match ev.delta {
            ScrollDelta::Lines(p) => p.y,
            ScrollDelta::Pixels(p) => p.y.as_f32() / 20.0,
        };
        this.zoom = (this.zoom + dy * 0.1).clamp(0.5, 2.0);
        cx.stop_propagation();
        cx.notify();
    }
}))
```

**动作 + 按键绑定 + 焦点**（已编译验证）：

```rust
gpui_kit::actions!(mini_todo, [Save, CloseEditor]);         // 单元动作（K/src/lib.rs:53 的 kit 版宏）
// 带数据的动作：#[derive(Clone, PartialEq, gpui_kit::Action)] #[action(namespace = mini_todo, no_json)] struct OpenTodo { id: i64 }
cx.bind_keys([
    KeyBinding::new("ctrl-enter", Save, Some("TodoEditor")),   // G/src/keymap/binding.rs:33 new(keystrokes, action, context)
    KeyBinding::new("escape", CloseEditor, Some("TodoEditor")),
]);
v_flex()
    .key_context("TodoEditor")
    .track_focus(&self.focus)                // self.focus = cx.focus_handle()
    .on_action(cx.listener(|this, _: &Save, window, cx| this.save(window, cx)))
impl Focusable for MainView { fn focus_handle(&self, _: &App) -> FocusHandle { self.focus.clone() } }
self.focus.focus(window, cx);  window.focus(&handle, cx);  handle.is_focused(window);  window.blur(cx);
```

按键分发规则（`G/src/window.rs:6150-6165`）：同一按键匹配到的绑定按优先级逐个分发，处理函数 **不调用 `cx.propagate()` 就停止**。gpui-base 的 Input 在 `"Input"` 上下文绑定了 `enter`/`shift-enter`/`secondary-enter`(=Windows 的 ctrl-enter)/`escape`（`B/src/input/base/state.rs:135-185`）。单行 Input 的 Enter 处理会 `cx.propagate()`（外层 `ctrl-enter` 绑定还能收到）；**Textarea/Editor 的 Enter 会插入换行且不传播**，外层 `Save` 收不到——要么订阅 `InputEvent::PressEnter { secondary: true, .. }`（插入换行后仍会 emit），要么在 `Some("Input")` 上下文追加更晚注册的绑定（后注册优先，未实测）。IME：Windows 后端处理 `WM_IME_STARTCOMPOSITION/WM_IME_COMPOSITION` 并按光标位置放置候选框（`W/src/events.rs`、`update_ime_position`）；gpui-component 的 Input 系列已实现 `EntityInputHandler`；自绘文本输入需自己实现该 trait。

### A7. 拖放

**应用内拖拽**（列表排序、四象限改象限；已编译验证）：

```rust
// G/src/elements/div.rs:1620 on_drag / :1191 drag_over / :1229 on_drop / :1048 on_drag_move / :1239 can_drop
#[derive(Clone)] struct DraggedItem(usize);
impl Render for DraggedItem { fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
    div().px_2().py_1().bg(rgb(0xE5E7EB)).rounded_md().child(format!("#{}", self.0)) } }

div().id(("row", ix))
    .on_drag(DraggedItem(ix), |d: &DraggedItem, _offset: Point<Pixels>, _window, cx| cx.new(|_| d.clone()))
    .drag_over::<DraggedItem>(|style, _d, _window, _cx| style.border_t_2().border_color(rgb(0x3B82F6)))
    .on_drop(cx.listener(move |this, d: &DraggedItem, _window, cx| { /* 重排 */ cx.notify(); }))
    .on_drag_move::<DraggedItem>(cx.listener(|this, ev: &DragMoveEvent<DraggedItem>, _, cx| {
        // ev.bounds（本元素）+ ev.event.position → 判断插在上半/下半；ev.drag(cx) 取拖拽值
    }))
```

- `on_drag` 的构造闭包第二个参数是"按下点相对元素原点的偏移"；预览视图默认画在光标处，需要自己按偏移对齐（参见 `G/examples/drag_drop.rs`）。
- 没有现成的"可排序列表"组件（gpui-component 中 sortable/reorder 只出现在 Table 列、Dock 里，不可直接复用）。
- 拖出窗口给系统：`.external_drag_payload::<T>(|v, window, cx| Some(ExternalDragPayload::Files(FileDragPaths::new([(path, is_dir)]))))`，但**只有 Wayland 实现了 `start_external_drag`**，Windows 不支持（`W/` 中无实现）。

**从资源管理器拖入文件**（Windows 支持，IDropTarget：`W/src/window.rs:1140-1295`）：
- `DragEnter`（只认 `CF_HDROP`，否则 `DROPEFFECT_NONE`）→ `FileDropEvent::Entered { position, paths }`；`DragOver` → `Pending { position }`；`DragLeave` → `Exited`；`Drop` → `Submit { position }`。effect 一律回 `DROPEFFECT_COPY`（源程序不会删除源文件；"移动"要应用自己做），也无法按位置拒绝。系统拖拽图像由 `IDropTargetHelper` 绘制。
- `Window::dispatch_event` 把它转成内部拖拽：Entered 时创建 `active_drag = ExternalPaths`，Pending→`MouseMove`，Submit→`MouseUp`（并 `cx.activate(true)`），Exited→清除拖拽（`G/src/window.rs:5656-5706`）。因此直接用 `drag_over::<ExternalPaths>` / `on_drop::<ExternalPaths>`：

```rust
div().id("drop")
    .drag_over::<ExternalPaths>(|s, _paths, _, _| s.bg(rgb(0xDBEAFE)))        // "拖文件经过时临时展开"可在这里或 on_drag_move 里改状态
    .on_drop(cx.listener(|this, paths: &ExternalPaths, _, cx| {
        for p in paths.paths() { /* &PathBuf */ }
        cx.notify();
    }))
    .on_file_drop_exit(cx.listener(|this, _: &FileDropEvent, _, cx| { /* 拖离窗口 */ }))
```

### A8. 平台能力

```rust
// 剪贴板 G/src/app.rs:1722-1760；类型 G/src/platform.rs:2969-3410
cx.write_to_clipboard(ClipboardItem::new_string(text));
cx.write_to_clipboard(ClipboardItem::new_image(&Image::from_bytes(ImageFormat::Png, bytes)));
if let Some(item) = cx.read_from_clipboard() {
    for e in item.entries() { match e {
        ClipboardEntry::String(s) => s.text(),
        ClipboardEntry::Image(img) => (img.format, &img.bytes),   // Windows: PNG/GIF/JPG/SVG 注册格式或 CF_DIB(→Bmp)
        ClipboardEntry::ExternalPaths(p) => p.paths(),            // 资源管理器复制的文件 CF_HDROP
    }; }
}
// 打开 / 定位（Windows: ShellExecuteW "open" / SHOpenFolderAndSelectItems，均在后台线程）
cx.open_url(url);            // 不做任何协议校验，白名单需自己做
cx.reveal_path(&path);       // 资源管理器中定位并选中
cx.open_with_system(&path);
// 文件对话框（返回 futures::channel::oneshot::Receiver）
let rx = cx.prompt_for_paths(PathPromptOptions { files: true, directories: false, multiple: false, prompt: Some("导入".into()) });
cx.spawn(async move |_cx| if let Ok(Ok(Some(paths))) = rx.await { /* Vec<PathBuf> */ }).detach();
let rx = cx.prompt_for_new_path(&dir, Some("mini-todo.zip"));   // Ok(Ok(Some(PathBuf)))
// 原生消息框（TaskDialogIndirect）G/src/window.rs:6667
let rx = window.prompt(PromptLevel::Warning, "确定删除？", Some("不可恢复"), &["删除", "取消"], cx); // rx.await -> Ok(按钮索引)
// 系统通知（Windows Toast）G/src/app.rs:1829-1865
cx.set_app_identity("com.minitodo.app", "Mini Todo");          // 未打包应用必须先调用：设 AUMID 并写 HKCU\Software\Classes\AppUserModelId\{id}
cx.show_system_notification(SystemNotification { tag: "todo-1".into(), title: "提醒".into(), body: "…".into(), actions: vec![] });
cx.dismiss_system_notification("todo-1");
cx.on_system_notification_response(|resp: SystemNotificationResponse, cx| { /* resp.tag, resp.action_id */ });
// 其它：cx.set_menus(..)（Windows 只存储菜单数据，无原生菜单栏）、cx.on_system_wake/on_system_sleep、cx.prevent_idle_sleep(..)
```

- 文件对话框：打开对话框**不支持扩展名过滤**（`W/src/platform.rs:1531`，只有 `FOS_FILEMUSTEXIST/ALLOWMULTISELECT/PICKFOLDERS`）；保存对话框固定 "All files (*.*)"（`:1612`）。需要过滤器可另用 `rfd` 等库。
- Toast 内容：ToastGeneric 两行文本 + 可选按钮，`audio silent=true`，无图标（`W/src/system_notifications.rs:157-188`）。gpui-component 的 `Notification::system()` / `NotificationDelivery` 也能把应用内通知转发到系统通知中心（`C/src/notification.rs:297-320`）。
- **托盘**：GPUI 无。主线程消息循环是通用 `GetMessageW(None)`，在主线程（`run` 回调内）创建的隐藏消息窗口能收到消息，事件经 A4 的 channel 写回 UI。
- **单实例 / 开机自启 / 全局热键**：GPUI 无，需自行实现（命名互斥量、注册表 Run 键等）。

### A9. 文本与字体

```rust
cx.text_system().add_fonts(vec![Cow::Borrowed(include_bytes!("../fonts/x.ttf"))])?;  // G/src/text_system.rs:295
cx.text_system().all_font_names();
// 测量：window.text_system().shape_line(text: SharedString, font_size: Pixels, runs: &[TextRun], force_width: Option<Pixels>) -> ShapedLine（.width）
```

- 默认字体 `.SystemUIFont`：Windows 解析为 `SPI_GETICONTITLELOGFONT` 的字体（中文 Windows 通常是 Microsoft YaHei UI，取不到时 Segoe UI，`W/src/direct_write.rs:1896-1918`）；缺字由 DirectWrite 系统回退补（中文字形依赖系统语言）。要固定中文字体：`Theme::update(cx, |t| t.font_family = "Microsoft YaHei UI".into())`；也可给 `Font { fallbacks: Some(FontFallbacks::from_fonts(vec![..])) }`（Windows 会追加系统回退映射，`direct_write.rs:398-452`）。
- Linux 用 cosmic-text + fontconfig 系统字体；Xvfb/CI 机器要装 CJK 字体（本机已有 Noto CJK，截图中文正常）。
- 字号：gpui-component 把 `rem` 设为 `Theme.font_size`（默认 16px，`C/src/root.rs:439`），`text_sm()` 等是 rem 相对值。

---

## B. gpui-component 0.7.3（含 gpui-base）

导入：`use gpui_kit::component::*;`（`ActiveTheme, Theme, ThemeMode, IconName, Icon, Sizable, Disableable, Selectable, StyledExt, h_flex, v_flex, WindowExt, TitleBar, Root(=base::Root)…`）+ 各子模块 `gpui_kit::component::{button, input, checkbox, …}`。通用约定：
- `Sizable`：`.xsmall() .small() .large() .with_size(Size::Size(px))`；`Disableable::disabled(bool)`；`Selectable::selected(bool)`。
- 受控组件（Checkbox/Switch/Radio/TabBar）：组件只"请求"新值，宿主在回调里写回状态并 `cx.notify()`。
- 有状态组件 = `XxxState` 实体（`cx.new(|cx| XxxState::new(window, cx))`，**在构造视图时创建，不要在 render 里 new**）+ 无状态渲染元素 `Xxx::new(&state)`；事件 `cx.subscribe_in(&state, window, |this, state, ev: &XxxEvent, window, cx| ..)`。
- 国际化：`gpui_kit::component::set_locale("zh-CN")`（`C/src/lib.rs:163`；`C/locales/ui.yml` 含 zh-CN/zh-HK/zh-TW），影响日历星期、占位文案等。

### B1. Root 与主题 Theme

```rust
// C/src/theme/mod.rs
Theme::global(cx) -> &Theme;     cx.theme() -> &Theme   // ActiveTheme for App（Context 也可 deref 调用）
Theme::update(cx, |theme: &mut Theme| { .. }) -> R;     // 推荐写入口：同步 colors/tokens/Base 投影并刷新所有窗口
Theme::global_mut(cx) -> &mut Theme;                   // 只改字段本身，不同步 tokens（不推荐）
Theme::change(ThemeMode::Dark, None, cx);               // 切换明暗（加载注册的 light/dark 主题）
Theme::sync_system_appearance(Some(window), cx);        // 跟随系统
theme.is_dark();  theme.mode
```

`Theme` 字段：`colors: ThemeColor`（`Deref/DerefMut` 到 Theme，可直接 `t.primary = ..`）、`tokens`、`mode`、`font_family`（默认 `.SystemUIFont`）、`font_size`（16px）、`mono_font_family`、`mono_font_size`（13px）、`radius`（6px）、`radius_lg`（8px）、`shadow`、`focus_ring`、`transparent`、`scrollbar_mode`、`notification: NotificationSettings{placement, margins, max_items, width, delivery}`、`list`、`sheet`、`motion`。

`ThemeColor` 全部字段（`C/src/theme/theme_color.rs`，类型均为 `Hsla`）：
accent, accent_foreground, accordion, background, border, button, button_active, button_foreground, button_hover, button_danger(+_active/_foreground/_hover), button_info(+…), button_primary(+…), button_secondary(+…), button_success(+…), button_warning(+…), group_box, group_box_foreground, caret, chart_1..chart_5, chart_bullish, chart_bearish, chart_grid, danger, danger_active, danger_foreground, danger_hover, description_list_label, description_list_label_foreground, drag_border, drop_target, foreground, info, info_active, info_foreground, info_hover, input, link, link_active, link_hover, list, list_active, list_active_border, list_even, list_head, list_hover, muted, muted_foreground, popover, popover_foreground, primary, primary_active, primary_foreground, primary_hover, progress_bar, ring, scrollbar, scrollbar_thumb, scrollbar_thumb_hover, secondary, secondary_active, secondary_foreground, secondary_hover, selection, sidebar, sidebar_accent, sidebar_accent_foreground, sidebar_border, sidebar_foreground, sidebar_primary, sidebar_primary_foreground, skeleton, slider_bar, slider_thumb, success, success_foreground, success_hover, success_active, switch, switch_thumb, tab, tab_active, tab_active_foreground, tab_bar, tab_bar_segmented, tab_foreground, table, table_active, table_active_border, table_even, table_head, table_head_foreground, table_foot, table_foot_foreground, table_hover, table_row_border, title_bar, title_bar_border, status_bar, status_bar_border, warning, warning_active, warning_hover, warning_foreground, overlay, window_border, red, red_light, green, green_light, blue, blue_light, yellow, yellow_light, magenta, magenta_light, cyan, cyan_light。

```rust
Theme::update(cx, |t| {                        // 已编译验证
    t.font_family = "Microsoft YaHei UI".into();
    t.font_size = px(14.);
    t.radius = px(8.);
    t.primary = rgb(0x3B82F6).into();
    t.notification.placement = Anchor::TopCenter;   // 类 ElMessage 顶部居中
});
```
注意：在同一个 `update` 闭包里改 `mode` 会重新加载主题覆盖颜色，切模式后再另起一次 `update` 改颜色（文档注释 `C/src/theme/mod.rs:270-283`）。

### B2. Button（`C/src/button/button.rs`）

```rust
Button::new(id)
    .label("新增").icon(IconName::Plus)                      // icon-only：只给 icon
    .primary() / .secondary() / .danger() / .warning() / .success() / .info() / .ghost() / .link() / .text()  // ButtonVariants
    .custom(ButtonCustomVariant::new(cx).color(..).foreground(..).hover(..).active(..))
    .outline()                                               // 描边样式（可与 variant 组合）
    .rounded(ButtonRounded::Large)                           // None/Small/Medium(默认)/Large/Size(px)；圆形按钮 .rounded(ButtonRounded::Size(px(999.)))
    .small() / .xsmall() / .large() / .compact()
    .tooltip("删除").tooltip_placement(..)
    .disabled(true)  .loading(true).loading_icon(..)  .selected(true)  .toggled(true)  .dropdown_caret(true)
    .on_click(|ev: &ClickEvent, window, cx| ..)  .on_hover(|hovered: &bool, window, cx| ..)
```
另有 `ButtonGroup`、`DropdownButton`、`Toggle`（`C/src/button/mod.rs`）。

### B3. Icon / IconName 与自定义 SVG

- 两个枚举：`gpui_kit::component::IconName`（**组件默认集 106 个**，`A/default-icons.txt`，`Assets` 只嵌入这些）；`gpui_kit::assets::IconName`（**完整 Lucide 目录 1830 个**，`IconName::ALL`、`.path()`）。两者都实现 `IconNamed`，都能喂给 `Icon::new(..)` / `Button::icon(..)`，但完整目录的 SVG 需要 `AllAssets`（全量嵌入）或 `icon_assets!` 宏挑选嵌入。
- 默认集（component IconName）：ALargeSmall, ArrowDown, ArrowLeft, ArrowRight, ArrowUp, Asterisk, Ban, Battery*, Bell, BookOpen, Bot, Building2, Calendar, CaseSensitive, ChartPie, Check, ChevronDown/Left/Right/Up, ChevronsUpDown, CircleAlert, CircleCheck, CircleUser, CircleX, Close, Copy, Cpu, Dash, Delete, EllipsisVertical, Ellipsis, ExternalLink, EyeOff, Eye, FileText, File, FolderClosed, FolderOpen, Folder, Frame, GalleryVerticalEnd, Github, Globe, HardDrive, HeartOff, Heart, Inbox, Info, Inspector, LayoutDashboard, LoaderCircle, Loader, Map, Maximize, MemoryStick, Menu, Mic, Minimize, Minus, Moon, Network, Palette, PanelBottomOpen, PanelBottom, PanelLeftClose, PanelLeftOpen, PanelLeft, PanelRightClose, PanelRightOpen, PanelRight, Pause, Play, Plus, Redo2, Redo, RefreshCw, Replace, ResizeCorner, RotateCw, Search, Settings2, Settings, SortAscending, SortDescending, SquareTerminal, Square, StarFill, StarOff, Star, Sun, ThumbsDown, ThumbsUp, TriangleAlert, Undo2, Undo, User, WindowClose, WindowMaximize, WindowMinimize, WindowRestore。
- 注意：默认集的 `Delete` 是"退格键"图标（不是垃圾桶）；垃圾桶是完整目录里的 `Trash`。

Element Plus → Lucide 建议（`*` = 不在默认集，需 icon_assets!/AllAssets；全部已核对存在 `A/assets/icons/*.svg`）：

| Element Plus | 建议 IconName | Element Plus | 建议 IconName |
|---|---|---|---|
| Plus | `Plus` | Delete | `Trash`* |
| Edit | `SquarePen`* / `Pencil`* | Check | `Check` / `CircleCheck` |
| Close | `Close`（= lucide x）/ `CircleX` | Lock / Unlock | `Lock`* / `LockOpen`* |
| Refresh | `RefreshCw` | Setting | `Settings` |
| Calendar | `Calendar` / `CalendarDays`* | List | `List`* / `ListTodo`* |
| Grid | `LayoutGrid`* / `Grid2x2`* | Bell | `Bell`（`BellRing`*/`BellOff`*） |
| Clock / AlarmClock | `Clock`* / `AlarmClock`* | ArrowLeft/Right/Up/Down | `ArrowLeft` 等 / `ChevronLeft` 等 |
| MoreFilled | `Ellipsis` / `EllipsisVertical` | Search | `Search` |
| Upload / Download | `Upload`* / `Download`* | Folder / FolderOpened | `Folder` / `FolderOpen` |
| Document | `FileText` | View / Hide | `Eye` / `EyeOff` |
| Sunny / Moon | `Sun` / `Moon` | Pin（固定） | `Pin`* / `PinOff`* |
| Picture / 上传图片 | `Image`* / `ImagePlus`* | Rank（拖动手柄） | `GripVertical`* |
| RefreshRight（重复） | `Repeat`* | 云同步 | `CloudUpload`* / `Cloud`* |
| Warning | `TriangleAlert` | InfoFilled | `Info` |
| Link | `ExternalLink`（`Link`*） | DocumentCopy | `Copy` |

挑选嵌入 + 与默认 Assets 串联（已编译并运行验证；注意 `Assets::load` 对缺失路径返回 **Err** 而不是 `Ok(None)`，所以先查自选集）：

```rust
gpui_kit::assets::icon_assets!(pub AppIcons, [Trash, Pencil, SquarePen, Pin, PinOff, Lock, LockOpen,
    List, LayoutGrid, Clock, AlarmClock, Upload, Download, ImagePlus, GripVertical, Repeat, CloudUpload, Save, Grid2x2]);

pub struct AppAssets;
impl gpui_kit::AssetSource for AppAssets {
    fn load(&self, path: &str) -> gpui_kit::Result<Option<std::borrow::Cow<'static, [u8]>>> {
        if let Some(bytes) = AppIcons.load(path)? { return Ok(Some(bytes)); }
        gpui_kit::assets::Assets.load(path)
    }
    fn list(&self, path: &str) -> gpui_kit::Result<Vec<SharedString>> {
        let mut v = AppIcons.list(path)?; v.extend(gpui_kit::assets::Assets.list(path)?); Ok(v)
    }
}
// gpui_kit::application().with_assets(AppAssets)
Icon::new(gpui_kit::assets::IconName::Trash).small().text_color(cx.theme().danger);
Button::new("pin").ghost().icon(gpui_kit::assets::IconName::Pin);
```

自定义 SVG：`Icon::default().path("icons/my.svg")`（路径须能被 AssetSource 加载）或 `Icon::default().data(include_bytes!("my.svg"))`（`C/src/icon.rs:108-139`）；也可实现 `IconNamed` 做自己的枚举。`Icon` 方法：`.text_color(c)`、`.with_size(..)/.small()`、`.rotate(radians)`、`.transform(Transformation)`、`Styled`（`size_4()` 等）。SVG 用 `currentColor` 描边，颜色来自 text_color。

### B4. Input / Textarea / Editor（`B/src/input/*`，`C/src/input/*`）

```rust
// 状态（gpui-base 的同一引擎，不同模式）
InputState::new(window, cx)        // = InputBaseState<InputMode> 单行
    .placeholder("标题").default_value("x").masked(true).pattern(regex).validate(|s, cx| bool)
    .mask_pattern(..).clean_on_escape().submit_on_enter(bool)
    .step(1.0).min(0.).max(1440.)                   // NumberInput 用
TextareaState::new(window, cx)     // 多行
    .rows(5).auto_grow(min_rows, max_rows).soft_wrap(true).searchable(true).submit_on_enter(false)
EditorState::new(window, cx)       // 代码编辑器：默认行号、tab=2、缩进线、可搜索
    .language("markdown").line_number(false).folding(false).soft_wrap(true)

// 读写（B/src/input/base/state.rs:989-1430）
state.read(cx).value() -> SharedString
state.update(cx, |s, cx| s.set_value("新值", window, cx));        // 需要 window；接受 &str/String/SharedString/…
s.set_placeholder(..,window,cx); s.set_disabled(bool, cx); s.set_readonly(bool, cx); s.focus(window, cx);
s.insert(..)/s.replace(..)/s.select_all(..)/s.cursor_position()/s.set_cursor_position(..)

// 事件（B/src/input/base/state.rs:127）
pub enum InputEvent { Change, PressEnter { secondary: bool, shift: bool }, Focus, Blur }
cx.subscribe_in(&state, window, |this, state, ev: &InputEvent, window, cx| ..)

// 渲染元素
Input::new(&input_state).cleanable(true).prefix(..).suffix(..).disabled(b).readonly(b).bordered(b).appearance(b).mask_toggle()
    .on_paste(|item: &ClipboardItem, window, cx| -> bool { /* true=已处理(图片粘贴) */ })
Textarea::new(&textarea_state).h(px(120.)).on_paste(..)
Editor::new(&editor_state).h_full().on_paste(..)
NumberInput::new(&input_state)      // + NumberInputEvent::Step(StepAction)
```

已编译验证示例：

```rust
let title = cx.new(|cx| InputState::new(window, cx).placeholder("标题"));
let notes = cx.new(|cx| TextareaState::new(window, cx).auto_grow(3, 10));
let sub = cx.subscribe_in(&title, window, |this, state, ev: &InputEvent, window, cx| match ev {
    InputEvent::Change => { let v: SharedString = state.read(cx).value(); }
    InputEvent::PressEnter { secondary, .. } => {}
    InputEvent::Focus | InputEvent::Blur => {}
});
```

- Markdown 源码高亮：`EditorState::new(..).language("markdown")` 需要 feature `tree-sitter-markdown`（及 inline：`tree-sitter-markdown-inline`），gpui-kit 同名 feature 透传（`K/Cargo.toml`）。不开 feature 时按纯文本显示。
- 图片粘贴：`on_paste` 拿到 `ClipboardItem`（含 `ClipboardEntry::Image` / `ExternalPaths`），返回 `true` 阻止默认文本插入，然后自己保存文件并 `insert("![](minitodo-image://name)")`。
- 软换行：Textarea/Editor `soft_wrap(bool)`；行号仅 Editor（`line_number`）。

### B5. 其它表单控件（本节与 B6 的代码已在探针工程中编译验证）

```rust
// Checkbox / Switch（C/src/checkbox.rs、switch.rs）——受控
Checkbox::new("agree").label("同意").checked(self.v).on_change(cx.listener(|this, v: &bool, _, cx| { this.v = *v; cx.notify(); }))
Switch::new("sw").checked(b).label("..").color(c).on_change(..)          // on_click 是 on_change 的别名
// Radio / RadioGroup（C/src/radio.rs）
RadioGroup::horizontal("prio").children(["高", "中", "低"]).selected_index(Some(ix))
    .on_change(cx.listener(|this, ix: &usize, _, cx| ..))
// Select（C/src/select.rs）：状态 SelectState<D: SelectDelegate>，Vec<T: SelectItem> / SearchableVec<T> 自带实现；&'static str/String/SharedString 已实现 SelectItem
let sel = cx.new(|cx| SelectState::new(SearchableVec::new(vec!["5 分钟", "15 分钟", "30 分钟"]), Some(IndexPath::new(0)), window, cx));
Select::new(&sel).placeholder("提前提醒").cleanable(true).menu_width(px(200.)).small()
cx.subscribe_in(&sel, window, |this, _, ev: &SelectEvent<SearchableVec<&'static str>>, _, cx| { let SelectEvent::Confirm(v) = ev; });
sel.update(cx, |s, cx| s.set_selected_index(Some(IndexPath::new(1)), window, cx)); s.selected_value()
// 自定义项：impl SelectItem for MyItem { type Value = i64; fn title(&self)->SharedString; fn value(&self)->&i64; fn render(..) }
// Slider（SliderState::new().min(0.).max(1.).step(0.05).default_value(0.8)；SliderEvent::{Change, Release}(SliderValue)）
Slider::new(&slider_state).horizontal()
// DatePicker（C/src/time/date_picker.rs）：日期 + 可选时间
let dp = cx.new(|cx| DatePickerState::new(window, cx).time_precision(TimePrecision::Minute).date_format("%Y-%m-%d %H:%M"));
DatePicker::new(&dp).placeholder("截止时间").cleanable(true)
// DatePickerEvent::Change(DateTime::Single(Option<NaiveDateTime>))；dp.update(cx, |s, cx| s.set_date_time(naive_dt, window, cx))（set_date(date, window, cx) 只改日期）
// 纯时间：TimeField（gpui_kit::component::time_field::{TimeField, TimeFieldState, TimeFieldEvent::Change(NaiveTime)}，HourCycle::H23/H12）
// 日历：Calendar::new(&CalendarState) —— 选择器型日历（CalendarEvent::Selected(Date)），不支持在日格里放自定义内容；
//       整月待办视图需自己用 grid 画（CalendarState::days()/month_days() 可复用日期计算）
// ColorPicker（ColorPickerState::new(window,cx).default_value(hsla)；ColorPickerEvent::Change(Option<Hsla>)）
ColorPicker::new(&cp).featured_colors(vec![..]).label("颜色")
```

### B6. 导航与浮层

```rust
// Tabs（C/src/tab/*）
TabBar::new("tabs").segmented()/.pill()/.underline()/.outline()
    .selected_index(self.tab).children([Tab::new().label("常规"), Tab::new().label("同步")])
    .on_click(cx.listener(|this, ix: &usize, _, cx| { this.tab = *ix; cx.notify(); }))
// Popover（C/src/popover.rs）
Popover::new("p").anchor(Anchor::BottomLeft).trigger(Button::new("t").label("更多"))
    .content(|state: &mut PopoverState, window, cx| div().child("..")).open(b).on_open_change(|open: &bool, w, cx| ..)
// 右键菜单（ContextMenuExt，返回 ContextMenu<_>，必须放在链尾）
div().id("row").child(..).context_menu(|menu, window, cx| {
    menu.item(PopupMenuItem::new("编辑").icon(IconName::Settings).on_click(|_, w, cx| ..))
        .separator()
        .item(PopupMenuItem::new("删除").disabled(false).on_click(..))
        .menu("动作项", Box::new(SomeAction)).menu_with_check("固定", b, Box::new(..))
        .submenu("移动到…", window, cx, |m, w, cx| m.menu("收纳箱 2", Box::new(MoveTo)))   // 签名 (label, window, cx, f)
})
// 下拉菜单（DropdownMenu trait，Button 已实现）
Button::new("more").ghost().icon(IconName::Ellipsis).dropdown_menu(|menu, window, cx| menu.item(..))
// Tooltip：组件上 .tooltip("文本")；任意 Stateful div：
div().id("x").tooltip(|window, cx| Tooltip::new("拖动排序").build(window, cx))
// Dialog / 确认框（WindowExt，C/src/window_ext.rs）——builder 会在重建层时多次调用，状态实体要在外面创建
window.open_dialog(cx, |dialog, window, cx| dialog.title("编辑").child(..).footer(..).width(px(480.))
    .on_ok(|_, window, cx| true /* 返回 true 关闭 */).on_cancel(|_, w, cx| true).overlay_closable(true).keyboard(true));
window.open_alert_dialog(cx, |alert, _, _| alert.confirm().title("删除待办").description("确定删除？")
    .ok_text("删除").ok_variant(ButtonVariant::Danger).cancel_text("取消")
    .on_ok(|_, window, cx| { window.push_notification("已删除", cx); true }));
window.close_dialog(cx); window.has_active_dialog(cx);
// 注意：AlertDialog 文档注释里的 `.warning()` 在 0.7.3 中并不存在；用 .confirm()/.icon(..)
// 消息提示（ElMessage 替代）
window.push_notification(Notification::success("已保存"), cx);          // info/success/warning/error；From<&str>
window.push_notification(Notification::new().title("同步失败").message(err).with_type(NotificationType::Error).autohide(false), cx);
// 默认 5s 自动隐藏、TopRight；Theme.notification.placement 可改全局位置；.id::<T>() + remove_notification::<T>() 去重
// Sheet（侧边抽屉）：window.open_sheet(cx, |sheet, w, cx| sheet.title(..).child(..))
```

### B7. 展示类与布局类

- `Tag::primary()/secondary()/danger()/success()/warning()/info()/custom(color, fg, border)/color(ColorName)` + `.outline()/.rounded_full()`（`C/src/tag.rs`）。
- `Separator::horizontal()/vertical()/horizontal_dashed()` + `.label("或")/.color(c)`。
- `Label::new("文本").secondary("副文本").masked(b).highlights("关键词")`；`Kbd::new(Keystroke::parse("ctrl-enter")?)`。
- `Collapsible::new().open(b).content(..)`（`C/src/collapsible.rs`），`accordion::Accordion`。
- 滚动：`div().id("list").overflow_y_scrollbar()`（`scroll::ScrollableElement`，带主题滚动条）；`scroll::Scrollbar::vertical(&scroll_handle)` 手动挂。
- 列表：`list::{List, ListState, ListDelegate, ListItem}`（委托式，支持搜索/选择/分节/加载更多）；虚拟化 `v_virtual_list(view_entity, id, Rc<Vec<Size<Pixels>>>, |view, range, window, cx| -> Vec<_>)`（`B/src/virtual_list.rs:139`）；或 GPUI 原生 `uniform_list(id, count, |range, window, cx| ..)` / `list(ListState, ..)`。
- `TitleBar::new().child(..)`：自绘标题栏，高 34px，Windows 上右侧最小化/最大化/关闭用 `window_control_area(Min/Max/Close)` 交给系统 hit-test，空白区 `WindowControlArea::Drag`（`C/src/title_bar.rs`）；配套 `TitleBar::window_options()`（`titlebar: Some{appears_transparent: true}`）。它自带渐变背景与底边框，可以用 Styled 覆盖（`.bg(transparent_black()).border_0()`）放进透明窗口；但"固定模式不可拖拽"需要 `is_movable: false` 或不用它。
- `WindowBorder`：仅 Linux 客户端装饰生效，Windows 下原样返回子元素（`C/src/window_border.rs:136-145`）；注释写明 GPUI 只有矩形内容遮罩，圆角外框无法裁掉子元素背景。
- **Sortable**：不存在现成组件，用 A7 的 on_drag/drop 自建。

### B8. Markdown 渲染（TextView）

**结论**：渲染在 gpui-base（`B/src/text/*`），GFM 全支持；组件层 `component::text::TextView` 是包装（主题样式 + 代码高亮），**但没有 `image_source`**。需要映射 `minitodo-image://` 时直接用 `gpui_kit::base::TextView`（主题默认样式由组件 init 时 `TextViewDefaults` 全局安装，`C/src/text/mod.rs:61-75`，所以样式一致）。

- 解析：`markdown` crate，`ParseOptions::gfm()`（`B/src/text/markdown_ext.rs:393`）→ 表格、任务列表（`ListItem { checked }`）、删除线（`Node::Delete`）、脚注、数学、内联 HTML（简单子集）、图片、链接、代码块。
- 默认链接点击：左键/中键 → `cx.open_url(url)`（无白名单）；`.on_link_click(..)` 完全接管（`B/src/text/text_view.rs:106-123`）。
- 默认图片：`data:` URL 内联解码，其他一律当 URI（经 HTTP client，默认是 Null → 加载失败）；`.image_source(resolver)` 覆盖所有图片的来源（含测量）。

```rust
// 已编译验证
gpui_kit::base::TextView::markdown("desc", markdown_text)     // id 需稳定；也有 html(..)
    .image_source(move |uri: &SharedUri| {
        let s: &str = uri.as_ref();
        match s.strip_prefix("minitodo-image://") {
            Some(name) => ImageSource::from(images_dir.join(name)),   // PathBuf → Resource::Path → fs::read
            None => ImageSource::from(uri.clone()),
        }
    })
    .on_link_click(|url: &SharedString, _ev: &ClickEvent, _window, cx| {
        if url.starts_with("https://") || url.starts_with("http://") || url.starts_with("mailto:") { cx.open_url(url); }
    })
    .selectable(true)          // 默认 true
    .scrollable(false)         // true 时需要父级定高，内部用 gpui::list 虚拟化
    .max_lines(3)              // 列表里摘要展示
    .style(gpui_kit::base::TextViewStyle::default().with_paragraph_gap(rems(0.5)).with_link(c)
        .with_heading(|level: u8| StyleRefinement::default() /* 按级别返回样式 */))
```

- 组件版（不需要改图片时）：`gpui_kit::component::text::TextView::markdown(id, text)` 或 `text::markdown(text)`，额外有 `.code_block_actions(..)`、`.table_actions(..)`、`.markdown_extensions(..)`、`.plugin(..)`；代码块语法高亮需 `tree-sitter*` feature。
- 有状态版本：`TextViewState`（`TextView::new(&state)`），适合流式/大文档。

---

## C. 编译与平台

### C1. gpui-kit features（`K/Cargo.toml`）

| feature | 作用 |
|---|---|
| `default = ["component", "assets"]` | gpui-component + gpui-kit-assets（默认 106 图标） |
| `component` / `assets` | 同上，可单独关 |
| `test-support` | GPUI 测试设施（`#[gpui_kit::test]`、`TestAppContext`、`VisualTestContext`、`HeadlessAppContext`、`gpui_kit::platform::current_headless_renderer`、`gpui_kit::test::*`）；应放 `[dev-dependencies]` |
| `inspector` | GPUI 元素检查器 |
| `profiler` | 帧事件埋点 |
| `decimal` | 组件 decimal 支持 |
| `tree-sitter`、`tree-sitter-<lang>`、`tree-sitter-languages` | 代码编辑器/代码块高亮（Markdown 用 `tree-sitter-markdown`，含 inline 用 `tree-sitter-markdown-inline`） |
| `speech` | 语音识别（WinRT） |
| `gpui-fast` | 切换到 GPUI Fast 后端 |

gpui-kit 依赖 `gpui-pre` 默认 feature（含 `windows-manifest`）；`gpui_platform` 依赖带 `font-kit, x11, wayland, runtime_shaders`。

**lockfile 坑（实测）**：`test-support` 里有 `"gpui-fast?/test-support"` 这类弱依赖特性，Cargo 解析时会把 gpui-fast-* 0.1.8 全部写进 Cargo.lock（不编译）。拿一份不含 test-support 的旧 lock 再加 dev-dep `features=["test-support"]`，会报 `cocoa =0.26.0` 与 `0.26.1`、`unicode-properties =0.1.3` 与 `0.1.4` 冲突；删掉 lock 重新 `cargo generate-lockfile` 即可（会选 cocoa 0.26.0、unicode-properties 0.1.3）。

### C2. Windows 构建要求

- 工具链：MSVC（`windows` crate 0.62）。
- **release**：`W/build.rs` 仅在**构建脚本运行于 Windows**（`#[cfg(target_os = "windows")]` 指 host）且非 debug 时调用 `fxc.exe` 编译 `shaders.hlsl`/`color_text_raster.hlsl`，生成 `OUT_DIR/shaders_bytes.rs` 并 `include!`。查找顺序：`GPUI_FXC_PATH` → `where.exe fxc.exe` → 注册表里最新 Windows 10 SDK 的 `bin/<ver>/x64/fxc.exe`，找不到直接 panic。⇒ **release 必须在装了 Windows SDK 的 Windows 机器（或 CI 的 windows runner）上构建**；Linux 交叉编译 release 会缺文件编译失败。
- **debug**：运行时 `D3DCompileFromFile(env!("CARGO_MANIFEST_DIR")/src/*.hlsl)`（`W/src/directx_renderer.rs:1802-1865`），依赖构建机 registry 中的源文件与系统 `d3dcompiler_47.dll` ⇒ debug 产物只能在构建它的那台机器上运行。
- manifest：gpui 默认 `windows-manifest` feature 通过 `embed-resource` 嵌入 `resources/windows/gpui.manifest.xml`（PerMonitorV2、Common-Controls v6、SegmentHeap、asInvoker）。应用若再用 winres/embed-resource 嵌 manifest 会重复（资源 ID 1 的 RT_MANIFEST）；只嵌图标（ICON 资源 ID 1，GPUI 窗口类图标即取它）即可。
- 运行时：DirectX 11 + DirectComposition（Win10+）；COM/OLE 由 GPUI 初始化（`RegisterDragDrop`、`CoCreateInstance(CLSID_DragDropHelper)`）。

### C3. Linux 构建与运行依赖

- 构建（本机 Ubuntu 24.04 已具备）：`libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev libx11-xcb-dev libxcb1-dev libfontconfig-dev libfreetype-dev libvulkan-dev`（xkbcommon 0.8 链接系统库，font-kit 走 fontconfig/freetype，wgpu 走 Vulkan/GL）。
- 运行：Vulkan ICD（真机 GPU 或 `mesa-vulkan-drivers` 的 lavapipe）；wgpu 实例后端写死 `VULKAN | GL`（`WG/src/wgpu_context.rs:474`），环境变量 `WGPU_BACKEND` 无效；`ZED_DEVICE_ID=0x....` 可指定适配器。正常窗口路径**接受 CPU（lavapipe）适配器**，只有设备丢失重建时才拒绝软件渲染（`wgpu_renderer.rs:2235`）。
- 显示后端选择：有 `WAYLAND_DISPLAY` 优先 Wayland，否则有 `DISPLAY` 用 X11，都没有则**无头平台**（窗口能布局/处理输入但不绘制）；`ZED_HEADLESS=1` 强制无头（`L/src/linux.rs:39`、`display_connection.rs:23`）。X11 缩放：`GPUI_X11_SCALE_FACTOR=1`（或 `randr`）。

### C4. release 体积建议（未在本任务中实测 Windows 体积）

- 本机 debug 探针二进制 ≈ 747MB（几乎全是调试信息）。release 建议：
  ```toml
  [profile.release]
  opt-level = 3        # 或 "s"
  lto = "fat"          # 或 "thin"
  codegen-units = 1
  strip = true
  panic = "abort"      # 确认不依赖 unwind 后再用
  ```
- 图标：用 `icon_assets!` 只嵌需要的图标，不要 `AllAssets`（1830 个 SVG）；不开 `tree-sitter-*` 除非需要 Markdown 源码高亮（每个语法几百 KB 起）。

### C5. 无头截图 / 自动化测试（重点，已实测）

三条路线，按推荐顺序：

**① `HeadlessAppContext` + wgpu 无头渲染器：像素级截图 + 交互（Linux 实测通过，无需 DISPLAY）**

- API：`HeadlessAppContext::with_platform(text_system, asset_source, renderer_factory)`（`G/src/app/headless_app_context.rs:60`），`capture_screenshot(window) -> Result<image::RgbaImage>`（内部 `window.render_to_image()`）。
- 渲染器：`gpui_kit::platform::current_headless_renderer()`：Linux = `WgpuHeadlessRenderer::new()`（任意 Vulkan/GL 适配器，lavapipe 可用），macOS = Metal，**Windows 返回 `Ok(None)`（没有无头渲染器，截图会报错）**（`P/src/gpui_platform.rs:113-133`）。
- 文本系统用真实平台的（`current_platform(true).text_system()`），中文字形与布局是真的；`TestWindow` 默认 scale_factor = 2.0，所以 360×180 逻辑尺寸截出 720×360。
- 动画按墙钟时间推进，截图前 `cx.set_reduce_motion(true)` 才能拿到最终态（实测：不设时 Checkbox 勾选后截图只有黑底没有对勾）。
- 元素定位：`gpui_kit::test::TestWindowExt` 的 `find/try_find/within/click/click_at/right_click/double_click/hover/scroll/drag/drag_to/press/input/render_frame`；组件自带 `.test_support()` 注册，自定义元素需 `div().id("x").test_support()`（`gpui_kit::TestSupportExt`，非 test-support 构建下是零成本空操作），快照 `ElementSnapshot::{checked, selected, expanded, value, bounds, visible, focused, disabled, label}`。

可运行示例（scratchpad 工程实测：`cargo test --test shot` 输出 720×360 PNG，中文/按钮/勾选均正确）：

```toml
# Cargo.toml
[dependencies]
gpui-kit = "=0.7.3"
[dev-dependencies]
gpui-kit = { version = "=0.7.3", features = ["test-support"] }
image = { version = "0.25", default-features = false, features = ["png"] }
```

```rust
// tests/shot.rs
use gpui_kit::component::{ActiveTheme as _, button::{Button, ButtonVariants as _}, checkbox::Checkbox};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, Bounds, Context, HeadlessAppContext, IntoElement, ParentElement as _, Render,
    Styled as _, Window, WindowBounds, WindowOptions, div, px, size};
use std::sync::Arc;

struct Demo { checked: bool }
impl Render for Demo {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div().size_full().p_4().flex().flex_col().gap_2()
            .bg(cx.theme().background).text_color(cx.theme().foreground)
            .child("Hello 你好 mini-todo")
            .child(Checkbox::new("agree").label("同意").checked(self.checked)
                .on_change(cx.listener(|this, v: &bool, _, cx| { this.checked = *v; cx.notify(); })))
            .child(Button::new("ok").primary().label("确定"))
    }
}

#[test]
fn headless_screenshot() {
    let mut cx = HeadlessAppContext::with_platform(
        gpui_kit::platform::current_platform(true).text_system(),
        Arc::new(gpui_kit::assets::Assets),
        gpui_kit::platform::current_headless_renderer,
    );
    cx.update(gpui_kit::init);
    cx.update(|cx| cx.set_reduce_motion(true));     // 动画直接取最终态
    let (handle, _view) = cx.update(|cx| gpui_kit::open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds { origin: Default::default(), size: size(px(360.), px(180.)) })),
            focus: false, show: false, ..Default::default()
        },
        cx, |_, cx| cx.new(|_| Demo { checked: false }),
    )).unwrap();
    cx.update_window(handle, |_, window, cx| window.render_frame(cx)).unwrap();
    let before = cx.capture_screenshot(handle).expect("headless renderer");
    before.save("before.png").unwrap();
    cx.update_window(handle, |_, window, cx| {
        window.click("agree", cx);
        window.render_frame(cx);
        assert_eq!(window.find("agree").checked(), Some(true));
    }).unwrap();
    let after = cx.capture_screenshot(handle).unwrap();
    after.save("after.png").unwrap();
    assert_ne!(before.as_raw(), after.as_raw());
}
```

运行：`env -u DISPLAY -u WAYLAND_DISPLAY cargo test --test shot -- --nocapture`（可加 `VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json` 固定 lavapipe）。异步结果（如 Select 提交走 `defer_in`）用 `#[gpui_kit::test] async fn` + `cx.wait_for(handle, Duration, |window, cx| cond).await`（`gpui_kit::test::TestAppContextExt`，`K/src/test.rs:410`）。

**② `#[gpui_kit::test]` + `TestAppContext`：纯逻辑/交互断言（全平台，含 Windows CI）**

- `TestPlatform` 默认 `NoopTextSystem`（字形尺寸是假的），没有像素；适合状态、焦点、动作、事件流断言。参考 `K/tests/controls.rs`（Checkbox/Switch/Tab/Select）、`K/tests/common/mod.rs`（用生产入口 `gpui_kit::open_window` 开测试窗口并 downcast 成 `WindowHandle<Root>`）。
- 测试模块不要 `use gpui_kit::*;`（开 test-support 后 glob 会把 GPUI 的 `test` 属性宏引进来遮蔽内建 `#[test]`，`K/tests/test_macro.rs` 注释），显式导入所需类型。
- `TestAppContext` 还提供 `simulate_keystrokes/simulate_input/dispatch_action/simulate_prompt_answer/simulate_path_prompt_response/opened_url/shown_system_notifications/simulate_window_resize/simulate_display_added` 等（`G/src/app/test_context.rs:191-660`），可验证 `open_url` 白名单、系统通知、文件对话框流程。
- `VisualTestAppContext`（带真平台）**仅 macOS** 编译（`G/src/app.rs:40`）。

**③ 真实应用 + Xvfb + lavapipe + xdotool（端到端，实测通过）**

用已构建好的 `gpui-kit` 探针二进制实测：窗口正常渲染（中文、按钮），`xdotool mousemove 641 417 click 1` 触发了按钮 `on_click`（stdout 打印 "Clicked!"）。脚本要点：

```bash
Xvfb :97 -screen 0 1280x800x24 &            # 等 xdpyinfo -display :97 成功
export DISPLAY=:97; unset WAYLAND_DISPLAY
export VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json   # lavapipe
export GPUI_X11_SCALE_FACTOR=1
./target/debug/app &                        # 等窗口出现：xdotool search --onlyvisible --name ''
import -window root shot.png                # 或 xwd -root | convert xwd:- shot.png
xdotool mousemove X Y click 1               # 交互；xdotool type / key ctrl+Return
```

真实 X11 窗口没有 `render_to_image`（只有测试平台实现），截图只能用 X 工具抓屏；默认窗口尺寸 1536×1095 会被裁到屏幕大小。这条路线验证的是 Linux 后端，不覆盖 Windows 特有行为（嵌入桌面、ex style、DirectComposition 透明、IDropTarget）。

---

## Files Found

| File Path | Description |
|---|---|
| `K/src/lib.rs` | gpui-kit 门面：re-export、`application`/`init`/`open_window`、`actions!` |
| `K/src/test.rs`、`K/TESTING.md`、`K/tests/*.rs` | UI 集成测试 API（TestWindowExt/TestAppContextExt）与用例 |
| `G/src/platform.rs` | `WindowOptions`、`WindowKind`、`WindowBackgroundAppearance`、`PlatformWindow`/`PlatformDisplay`、剪贴板/通知类型 |
| `G/src/app.rs`、`G/src/app/{context,async_context,entity_map,headless_app_context,test_context}.rs` | App/Context/AsyncApp/实体/无头与测试上下文 |
| `G/src/window.rs` | Window API、事件派发（含 FileDrop 转换、按键分发）、HasWindowHandle |
| `G/src/elements/{div,img,svg,animation,anchored,deferred,canvas,list,uniform_list}.rs` | 元素与交互 trait |
| `G/src/styled.rs`、`G-macros/src/styles.rs` | Styled 方法与刻度 |
| `W/src/window.rs`、`W/src/events.rs`、`W/src/platform.rs`、`W/src/display.rs` | Windows 后端：样式映射、透明、IDropTarget、hit-test、消息循环、对话框、显示器 |
| `W/src/directx_renderer.rs`、`W/build.rs` | DirectComposition 交换链、着色器编译（fxc / 运行时） |
| `W/src/system_notifications.rs`、`W/src/clipboard.rs`、`W/src/direct_write.rs` | Toast、剪贴板格式、字体解析 |
| `P/src/gpui_platform.rs` | `application`/`current_platform`/`current_headless_renderer` |
| `L/src/linux.rs`、`L/src/linux/display_connection.rs`、`WG/src/wgpu_context.rs` | Linux 后端选择、无头、wgpu 适配器选择 |
| `B/src/root.rs`、`C/src/root.rs` | Root 与组件 WindowState 插件（背景/字体/浮层） |
| `C/src/theme/{mod,theme_color,color}.rs` | 主题 |
| `C/src/{button/button,icon,checkbox,switch,radio,select,popover,tooltip,notification,title_bar,window_border,window_ext}.rs`、`C/src/menu/*`、`C/src/dialog/*`、`C/src/tab/*`、`C/src/time/*` | 组件 |
| `B/src/input/base/state.rs`、`C/src/input/*` | 输入引擎与 Input/Textarea/Editor |
| `B/src/text/{text_view,node,markdown_ext}.rs`、`C/src/text/{mod,compat}.rs` | Markdown TextView（image_source、链接处理） |
| `A/src/lib.rs`、`A/build.rs`、`A/default-icons.txt`、`A/assets/icons/` | 图标枚举生成、默认集、`icon_assets!` |

## Related Specs

- `.trellis/spec/backend/desktop-fences.md` — 现有 Tauri 版收纳箱/嵌入桌面契约（owner=Progman、always_on_bottom、`WS_EX_TOOLWINDOW`、`reassert`），对照 A2/A3 的 GPUI 窗口样式映射重新实现。
- `.trellis/spec/backend/sync-protocol.md` — 同步协议（与 UI 框架无关）。

## Caveats / Not Found

- 本次实测只在 Linux：headless 截图（wgpu+lavapipe）、Xvfb 真实窗口、xdotool 交互、`icon_assets!` 串联、以及本文全部"已编译验证"代码片段（在 scratchpad 独立工程 `cargo test --no-run` 通过）。**所有 Windows 行为结论均来自源码阅读，未在 Windows 上运行**。
- `show: false` 后用 `activate_window()` 显示的行为（`SetWindowPlacement` 的 showCmd）未验证；建议用 Win32 `ShowWindow`。
- 在 `Some("Input")` 上下文追加 `ctrl-enter` 绑定以拦截 Textarea 的 Enter 属于按源码推断的技巧，未实测。
- 不加 `WS_EX_LAYERED` 的建议基于 DirectComposition 实现推断，未实测。
- 没有找到：托盘、单实例、开机自启、窗口移动/显示隐藏 API、文件对话框过滤器、Windows 无头渲染器、可排序列表组件、组件版 TextView 的图片解析钩子。
- release 体积数据未测（只给配置建议）。
