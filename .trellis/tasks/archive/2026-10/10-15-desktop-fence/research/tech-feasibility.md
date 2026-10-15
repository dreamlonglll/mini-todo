# Research: 桌面收纳箱（Desktop Fence）技术可行性

- **Query**：每个收纳箱是桌面上独立的小型无边框透明窗口，内放应用/文件图标，双击启动，支持固定（嵌入桌面）、折叠、重命名。调研：多窗口嵌入桌面、文件拖入/拖出、图标提取、启动、文件移入、多显示器/DPI 的可行性与坑。
- **Scope**：mixed（仓库代码 + 本机 cargo registry 中 tao 0.34.5 / wry 0.53.5 / tauri 2.9.5 / tauri-plugin-opener 2.5.3 / open 5.3.3 / windows 0.58 源码 + MS Learn / Tauri GitHub）
- **Date**：2026-10-15
- **版本基线**（`pc/src-tauri/Cargo.lock`）：tauri 2.9.5、tauri-runtime-wry 2.9.3、tao 0.34.5、wry 0.53.5、webview2-com 0.38.2、windows 0.58.0（项目直接依赖）+ 0.61.3（tauri/opener 内部用）、png 0.17.16（已被 tray-icon / muda / ico 间接引入）、@tauri-apps/api 2.9.1

---

## 0. 结论速览

| 主题 | 可行性 | 推荐方案 | 最大风险 |
|---|---|---|---|
| 1. N 个窗口嵌入桌面 | 可行，现有主窗口方案能直接推广 | Rust 侧 `WebviewWindowBuilder` 建窗时就带 `always_on_bottom(true)` / `minimizable(false)` / `maximizable(false)` / `skip_taskbar(true)`，再排队执行泛化后的 `desktop_attach(hwnd)`；状态从全局原子量改为按 label 存的 Map；200 ms 轮询线程额外遍历收纳箱 | 每个收纳箱一个 WebView2，内存开销要实测；多个 bottom 窗口互相重叠时，点击会把被点的那个压到最底层 |
| 2. 文件拖入 / 箱内排序 / 跨箱拖动 | 可行 | **MVP**：保留 Tauri 原生拖放处理（默认开启），外部文件走 `onDragDropEvent`；箱内排序沿用项目已有的 vuedraggable `force-fallback`；跨箱拖动由后端用光标位置做命中测试。**增强**：拖出到桌面/Explorer 用 `drag` crate（OLE `DoDragDrop`） | Windows 上原生拖放处理与 HTML5 DnD 互斥（wry 替换了 WebView2 的 IDropTarget） |
| 3. 图标提取 | 可行 | 在专用 STA 线程里调 `SHCreateItemFromParsingName` → `IShellItemImageFactory::GetImage(SIZE, SIIGBF_ICONONLY \| SIIGBF_BIGGERSIZEOK)` → `GetDIBits` 取 BGRA → 用 `png` 0.17 编码 → 按路径哈希缓存到 `%LOCALAPPDATA%\mini-todo\fence-icons\`，前端用 `convertFileSrc` 加载 | alpha 通道是否预乘需 spike；只有小尺寸图标的旧 exe 放大后会模糊或带透明留白 |
| 4. 启动 | 可行 | Rust 命令里在 STA 线程调 `ShellExecuteExW`（`SEE_MASK_FLAG_NO_UI`，exe 设 `lpDirectory`）；也可以直接用已有的 `tauri_plugin_opener::open_path`（底层就是 `ShellExecuteExW`）；"打开所在位置"用 `reveal_item_in_dir` | `.lnk` 目标丢失时默认会弹 Shell 对话框；收纳箱设为不可激活会导致新启动的程序拿不到前台 |
| 5. 把文件"移入"收纳箱 | 能做但风险高 | 建议 MVP 只做**引用模式**；确实要移入时推荐"收纳箱绑定一个真实文件夹"（类似 Stardock 的 Folder Portal），不建议偷偷移到隐藏目录 | OneDrive 重定向的桌面会把移动同步到所有设备；公共桌面需要管理员权限；重名；崩溃时数据库与文件系统不一致 |
| 6. 多显示器 / DPI | 可行 | 位置存**物理像素**、按"屏幕组合 ID"分组保存（沿用主窗口 `screen_configs` 的思路），尺寸存逻辑像素或连同 scale 一起存；恢复时先定位再定尺寸，并夹回最近显示器的 work area | 显示器断开时系统挪动窗口触发 `Moved`，如果此时落库会覆盖原布局 |

---

## 1. 多个嵌入桌面的窗口

### 1.1 现状：整套方案按"只有一个主窗口"写

| 位置 | 单窗口假设 |
|---|---|
| `pc/src-tauri/src/commands/window.rs:34` | `pub static IS_DESKTOP_MODE: AtomicBool`，全局一个布尔 |
| `window.rs:43` | `static DESKTOP_OWNER: AtomicIsize`，全局只存一个已挂载的 Progman 句柄 |
| `window.rs:1082-1108` `desktop_attach` | 依次 `SetWindowLongPtrW(hwnd, GWLP_HWNDPARENT, host)`、`apply_fixed_ex_style(window, true)`、`SetWindowPos(HWND_BOTTOM)`，最后**写全局** `DESKTOP_OWNER` |
| `window.rs:1119-1137` `desktop_detach` | 恢复样式取决于**主窗口的** `is_fixed_mode()`，并清全局 `DESKTOP_OWNER` |
| `window.rs:1144-1172` `tick_desktop_mode` | 读全局 `DESKTOP_OWNER`，闭包里复核全局 `is_desktop_mode()` |
| `window.rs:1185-1206` `reassert_window_mode_state` | 按全局 `IS_DESKTOP_MODE` / `IS_FIXED_MODE` / `DESIRED_TOPMOST` 分支 |
| `window.rs:949-960` `win32_show` | 按全局模式决定是否绕开 tao |
| `window.rs:1254-1256`、`1315-1317` | 两条命令都写死 `get_webview_window("main")` |
| `pc/src-tauri/src/lib.rs:261-282` | 200 ms 轮询线程：`if is_desktop_mode() { tick main } else if is_fixed_mode() {...}`，只处理 `"main"` |

已经与窗口无关、可以直接复用的部分：`window_hwnd(&WebviewWindow)`（`window.rs:868-876`，通过 raw-window-handle 0.6 取 HWND，绕开了 tauri 用的 windows 0.61 HWND 类型）、`apply_fixed_ex_style(window, bool)`（`879-904`）、`desktop_host()`（`1056-1070`）、纯函数 `needs_reattach(...)`（`1025-1038`，已有单测）。

### 1.2 需要泛化的地方

1. **拆出不碰全局状态的 Win32 核心**：例如 `fn desktop_attach_hwnd(hwnd) -> Option<isize /*host*/>` 和 `fn desktop_detach_hwnd(hwnd, keep_toolwindow: bool)`。主窗口原来的 `desktop_attach/detach` 改成薄包装（继续写 `DESKTOP_OWNER`、继续按 `is_fixed_mode()` 决定样式）；收纳箱的包装写到自己的状态表里。spec 里 "Extensibility" 一节本来就预留了这个接缝（`.trellis/spec/backend/window-modes.md` 第 229-231 行）。
2. **按 label 存状态**：例如 `static FENCES: LazyLock<Mutex<HashMap<String, FenceEmbedState>>>`，其中 `FenceEmbedState { embedded: bool, owner: isize, desired_style: ... }`。`run_on_main_thread` 排队的闭包里要**重新读 Map**（与现在"闭包里复核 `is_desktop_mode()`"的语义一致），排队期间收纳箱可能已经被关闭或取消固定。
3. **轮询线程**（`lib.rs:263-282`）：在现有 if/else 之外单独加一段 `commands::tick_fences(&handle)`，因为收纳箱与主窗口模式互不相关。每个 tick 只调一次 `GetShellWindow()`，然后对每个嵌入中的收纳箱读 `IsWindow(cached)` 和 `GetWindowLongPtrW(hwnd, GWLP_HWNDPARENT)`，再用 `needs_reattach` 判断；需要重挂的才 `get_webview_window(label)?.run_on_main_thread(...)`。**持锁期间不要调用 `run_on_main_thread`**，先收集 label 再释放锁。每个收纳箱每 tick 只有 3 次只读 Win32 调用，10 个收纳箱也可以忽略不计。
4. **Explorer 重启后重挂**：直接复用 `needs_reattach` 的四种情况。spec 实测主窗口在 Explorer 重启时**不会**随 owner 一起销毁，重挂耗时 < 1.5 s（`window-modes.md` 第 123 行）。收纳箱用的是同一机制，预期相同（MS 文档："A thread cannot use DestroyWindow to destroy a window created by a different thread"，跨进程的 owner 销毁不会连带销毁我们的窗口）。仍需在 e2e 里对多窗口复测。
5. **生命周期**：收纳箱窗口 `on_window_event(Destroyed)` 时从 Map 中删除（参照 `services/notification.rs:246-252` 的做法）；应用启动时在 `setup` 里按数据库重建所有收纳箱。

### 1.3 能否在 Rust 里用 `WebviewWindowBuilder` 创建：可以

已有范例 `pc/src-tauri/src/services/notification.rs:222-239`：`.decorations(false).always_on_top(true).resizable(false).skip_taskbar(true).focused(false).visible(true)`，非 macOS 再加 `.transparent(true)`。tauri 2.9.5 的 `WebviewWindowBuilder` 收纳箱需要的方法都有（`tauri-2.9.5/src/webview/webview_window.rs`）：`always_on_bottom`(654)、`minimizable`(561)、`focusable`(607)、`skip_taskbar`(694)、`shadow`(716)、`owner_raw`(761，仅 Windows)、`disable_drag_drop_handler`(1029)、`transparent`(1079)。

推荐的建窗参数（示意）：

```rust
WebviewWindowBuilder::new(app, format!("fence-{id}"), WebviewUrl::App(format!("index.html#/fence?id={id}").into()))
    .decorations(false).transparent(true).shadow(false)
    .always_on_bottom(true)      // 建窗时就写进 tao 的 WindowFlags，之后的 apply_diff 不会再翻它
    .minimizable(false)          // 去掉 WS_MINIMIZEBOX，Win+M 不会最小化它
    .maximizable(false)          // 防止双击拖动区域时最大化（见 1.5）
    .skip_taskbar(true)          // tao 在 set_visible 之前先调 ITaskbarList::DeleteTab，建窗不会闪任务栏
    .focused(false)
    .resizable(true)
    .position(x, y).inner_size(w, h)
    .build()?;
// 然后 run_on_main_thread → desktop_attach_hwnd(hwnd)：owner=Progman + WS_EX_TOOLWINDOW + HWND_BOTTOM
```

源码依据：

- `tao-0.34.5/src/platform_impl/windows/window.rs:1334` 的 `set_skip_taskbar` 在 `:1347` 的 `set_visible` 之前执行，所以 `skip_taskbar(true)` 建窗不会出现任务栏按钮。
- 建窗时带上 `always_on_bottom` / `minimizable(false)`，这些位一开始就在 tao 的 flags 里，之后**不需要**再走 `set_always_on_bottom` / `set_minimizable`，也就绕开了主窗口进入嵌入模式时"tao 调用在前、Win32 在后"的排序约束（`window-modes.md` 规则 2）。

**必须遵守的约束：不能在同步命令里建窗。** tauri 文档原文（`webview_window.rs:62-63`）："On Windows, this function deadlocks when used in a synchronous command and event handlers … You should use `async` commands and separate threads when creating windows." 新建收纳箱的命令必须是 `async fn`，或者在 `setup` / 独立线程里建（notification 那里用的是 `async_runtime::spawn`）。

**可选方案：建窗时直接 `owner_raw(Progman)`**。tao 会走 `Parent::OwnedBy` 分支（`tao .../window.rs:1158-1161`），这样 flags 里**没有 `ON_TASKBAR`**，后续任何 apply_diff 都不会再写 `WS_EX_APPWINDOW`。MS Learn《The Taskbar》："The Shell creates a button on the taskbar whenever an application creates a window that isn't owned"，所以被 owner 持有、又没有 APPWINDOW 的窗口天然不进任务栏，能消除"apply_diff 让任务栏图标闪一下"这一整类问题。代价：
  - `owner_raw` 的参数是 **windows 0.61** 的 `HWND`（`tauri-2.9.5/src/webview/webview_window.rs:47`），项目直接依赖的是 0.58。要么把项目的 windows 升到 0.61（树里已有，不增加编译量，但现有 `window.rs` 要改 API 签名），要么用 `windows061 = { package = "windows", version = "0.61", features = ["Win32_Foundation"] }` 只为构造这个 HWND。
  - Explorer 重启后仍然要用 `GWLP_HWNDPARENT` 重挂，所以 attach 代码还是要有。
  - 未经 e2e 验证。**建议先走已验证的"建窗后 `GWLP_HWNDPARENT`"方案，`owner_raw` 作为后续加固。**

### 1.4 tao `apply_diff` 覆写 ex style 的坑（逐个列出触发点）

- `tao .../window_state.rs:315-323`：`apply_diff` 在 `diff == empty` 时**直接 return**，只有 flag 真的变了才会重写样式。
- `window_state.rs:425-440`：一旦有变化，就按 tao 自己记录的 flags 整体执行 `SetWindowLongW(GWL_STYLE)` + `SetWindowLongW(GWL_EXSTYLE)` + `SWP_FRAMECHANGED`。`to_window_styles`（`:242-300`）里**没有 `WS_EX_TOOLWINDOW`**，而 `ON_TASKBAR` 会加上 `WS_EX_APPWINDOW`。
- **会改 flag、从而触发重写的 tao 调用**：`set_visible`（即 `show()` / `hide()`）、`set_resizable`、`set_minimizable`、`set_maximizable`、`set_decorations`、`set_always_on_top`、`set_always_on_bottom`、`set_focusable`、`set_ignore_cursor_events`、`maximize` / `unmaximize` / `minimize`、窗口处于最大化时调 `set_position`。
- **不触发重写的**：`set_size` / `set_inner_size`（非最大化时）、`set_position`（非最大化时）、`startDragging` / `startResizeDragging`、`set_focus`。所以**折叠/展开只改尺寸，是安全的**。
- **新坑：VISIBLE 位漂移**。tao 不会根据 `WM_SHOWWINDOW` 同步 `WindowFlags::VISIBLE`（`event_loop.rs` / `window.rs` 里没有这段逻辑）。如果收纳箱建成 `visible(false)`，再用 Win32 `ShowWindow` 显示，tao 仍认为 VISIBLE=false，之后任何一次 apply_diff 写进去的 `GWL_STYLE` 都不含 `WS_VISIBLE`，窗口会被隐藏。**结论：收纳箱建窗用 `visible(true)`；要隐藏或显示就统一走 tao 的 `hide()` / `show()`，然后再排队 reassert，不要混用 Win32 `ShowWindow`。**（主窗口的 `win32_show` 只用于本来就可见、从最小化还原的场景，所以没踩到这个坑。）
- `GWLP_HWNDPARENT` 不在 tao 的覆写范围内（`window-modes.md` 规则 5），owner 在 apply_diff 之后仍然有效，只有 TOOLWINDOW 会丢。
- tao 收到 `TaskbarCreated` 消息（任务栏重启）时会重新执行 `set_skip_taskbar`（`event_loop.rs:2278-2281`）。

**建议**：收纳箱"固定 / 解除固定"尽量**不改 tao flag**，只由前端控制能否拖动或缩放（不调 `startDragging`、隐藏缩放手柄）。如果必须 `set_resizable(false)`，就照主窗口的做法，紧接着排队执行 `reassert_fence_state(label)`。

### 1.5 其他坑

1. **多个 bottom 窗口之间的 Z 序**：tao 的钩子（`event_loop.rs:1230-1233`）会把**每一次** `SetWindowPos` 的 `hwndInsertAfter` 改成 `HWND_BOTTOM`。点击（激活）某个收纳箱时，它会落到"紧贴 Progman 之上"，也就是压到其他收纳箱和嵌入态主窗口**下面**。收纳箱之间不重叠时没有影响；重叠时就会出现"点哪个哪个沉下去"的反直觉效果。缓解办法：限制收纳箱不重叠（吸附或避让），或者接受这种行为。
2. **WebView2 内存**：每个收纳箱都是一个 WebView2 controller。它们共享同一个 browser 进程（同一个 user data folder），但 renderer 进程的分配取决于 Chromium 的站点隔离策略。MS Learn《Process model》："creating a new WebView2 instance … with a different domain … will usually start a new renderer process"，同源实例是否复用 renderer 没有保证。**需实测**：开 5~10 个收纳箱，用 `Get-Process msedgewebview2` 统计。收纳箱页面应走独立路由并做代码分割，避免把 Element Plus 全量和 Milkdown 打进去。
3. **双击标题栏会最大化**：tauri 的 drag-region 双击会调 `internal_toggle_maximize`，条件只是 `is_resizable()`（`tauri-2.9.5/src/window/plugin.rs:198-210`），收纳箱是 resizable 的，所以双击标题会被最大化，并触发 apply_diff。收纳箱标题栏**不要用 `data-tauri-drag-region`**，改成项目里已有的 `@mousedown → startDragging()` 写法（如 `pc/src/views/SettingsView.vue:330-333`），再自己处理 dblclick（比如映射为折叠）；同时 `maximizable(false)`。
4. **激活与键盘**：重命名需要键盘焦点，收纳箱必须可激活（不能 `focusable(false)` / `WS_EX_NOACTIVATE`）。always_on_bottom 保证激活后也不会浮上来。同时可激活也是"启动的程序能拿到前台"的前提，见 4.3。
5. **Win10 / Win11 ≤23H2**：Win+D 时被抬起的是 WorkerW，而不是 Progman，收纳箱可能被盖住。spec 已把这种情况列为不保证（`window-modes.md` 第 122、229 行）。对收纳箱这种"桌面功能"影响更明显，**PRD 需要明确是否只承诺 Win11 24H2+**。
6. **"显示桌面图标"被关闭**时，收纳箱仍然显示（它们是独立窗口），是否跟随隐藏属于产品决定。
7. **虚拟桌面（Win+Ctrl+D）**：被 Progman 持有的 tool window 会不会出现在所有虚拟桌面上，**未验证**。

### 1.6 推荐

- Win32 核心按 HWND 泛化，状态按 label 存 Map，轮询线程加 `tick_fences`。
- 建窗时把 tao flag 一次给齐，之后尽量不再改 flag；所有 Win32 样式变更都排在 `run_on_main_thread` 队列里执行。
- e2e 复用 `pc/scripts/e2e/` 的工具链（`e2e.ps1 -Step state` 改为枚举多个 HWND）。

---

## 2. 从桌面/Explorer 拖文件进 Tauri 2 webview（Windows）

### 2.1 现状

- `pc/src-tauri/tauri.conf.json` 里**没有** `dragDropEnabled`，所以用的是默认值 `true`，即 Tauri 原生拖放处理处于开启状态。前端 `pc/src/views/MainView.vue` / `EditorView.vue` 用 `new WebviewWindow(...)` 建的窗口也没有关闭它。
- 正因如此，项目里**所有** vuedraggable 都显式开了 `:force-fallback="true"`（指针事件模拟拖拽，不走 HTML5 DnD）：`pc/src/components/TodoList.vue:59-67`、`pc/src/components/QuadrantView.vue:122-131`、`pc/src/views/EditorView.vue:1225-1233`（另见 `EditorView.vue:1877-1879` 关于 fallback 克隆体 transition 的注释）。
- 全仓库没有用到 `onDragDropEvent` / `DragDrop`；`MarkdownEditor.vue` 里也没有 drop 逻辑。

### 2.2 原理：为什么会互斥（wry 源码）

- `wry-0.53.5/src/webview2/mod.rs:149-157`：只要注册了 drop handler，就先 `ICoreWebView2Controller4::SetAllowExternalDrop(false)`，再 `DragDropController::new(hwnd, handler)`。
- `wry-0.53.5/src/webview2/drag_drop.rs:67` `inject_in_hwnd`：对容器下**所有子窗口**（即 WebView2 的 Chrome_WidgetWin）先 `RevokeDragDrop` 再 `RegisterDragDrop(wry 自己的 IDropTarget)`。WebView2 原来的 drop target 被替换，页面内的 HTML5 `dragover` / `drop` 收不到事件（拖动时一直显示禁止光标）。
- `drag_drop.rs:159-200` `DragEnter`：只认 `CF_HDROP`（`DragQueryFileW` 逐个取路径），否则 `enter_is_valid=false`，直接忽略。有效时 effect **固定为 `DROPEFFECT_COPY`**。坐标用 `ScreenToClient` 转换，得到的是**客户区物理像素**。
- tauri 把它转换为 `DragDropEvent::{Enter{paths, position}, Over{position}, Drop{paths, position}, Leave}`，position 是 `PhysicalPosition`（`tauri-runtime-wry-2.9.3/src/lib.rs:4631-4662`）。前端类型见 `node_modules/@tauri-apps/api/webview.d.ts:22-35`，用法是 `getCurrentWebview().onDragDropEvent(cb)`（`window.d.ts:1249` 也提供同名 API）。
- 关闭方式：配置项 `dragDropEnabled: false`，或 Rust 侧 `WebviewWindowBuilder::disable_drag_drop_handler()`。文档原文："This is required to use HTML5 drag and drop APIs on the frontend on Windows."（`webview_window.rs:1027-1032`）。**这是按窗口生效的设置**，可以只对收纳箱窗口设置，不影响主窗口。
- 相关 issue：tauri-apps/tauri#15138（2026-03，"dragDropEnabled: true blocks HTML5 dragover events … no 'hybrid' mode"）、#14373（open，命名与文档混乱）、#13533。

**由此推出的细节**：

- 能拖进来的只有带文件系统路径的项。"此电脑"、"回收站"、开始菜单中的 UWP 磁贴等没有 `CF_HDROP` 的 shell 虚拟项会被**静默忽略**（是否有路径需 spike）。
- 拖入桌面上的 `.lnk`，得到的是 `.lnk` 文件本身的路径，不是它的目标（wry 只做 `DragQueryFileW`）。
- 坐标换算到 DOM：`x / devicePixelRatio`，再用 `document.elementFromPoint` 算插入位置。
- **UIPI**：如果 mini-todo 以管理员身份运行，从中完整性级别的 Explorer 拖进来会被系统拦截。

### 2.3 方案对比

| 方案 | 外部文件拖入 | 箱内排序 | 跨收纳箱拖动 | 拖出到桌面 | 实现成本 |
|---|---|---|---|---|---|
| **A. 原生处理 + 指针拖拽（推荐 MVP）** | `onDragDropEvent` 拿 paths + position | vuedraggable `force-fallback`（与项目现有写法一致） | SortableJS `onEnd` 时若指针在窗口外 → `invoke('fence_drop_at_cursor', {itemId})` → Rust 用 `app.cursor_position()`（物理像素）对各收纳箱的 `outer_position/outer_size` 做命中测试 → 改数据库 → `emit` 给两个收纳箱刷新 | 不支持 | 低 |
| B. 关闭原生处理 + HTML5 + WebView2 `postMessageWithAdditionalObjects` | HTML5 `drop` → `chrome.webview.postMessageWithAdditionalObjects('fence-drop', e.dataTransfer.files)` → Rust 通过 `webview.with_webview(\|w\| w.controller().CoreWebView2()...add_WebMessageReceived)` 取 `ICoreWebView2WebMessageReceivedEventArgs2::AdditionalObjects` → `ICoreWebView2File::Path` | 原生 HTML5 DnD | 两个 WebView2 同属一个 environment，HTML5 跨窗口拖放**理论可行，需 spike**；系统拖影可以显示在窗口外 | 不支持（DOM 拖动不带 `CF_HDROP`） | 中：要新增 `webview2-com 0.38` + `windows 0.61` 依赖（类型必须和 tauri 一致）；wry 的 IPC handler 也会收到这条消息并尝试当 IPC 解析，会有日志噪音 |
| C. A + OLE 拖出（增强） | 同 A | 同 A | 指针离开源窗口时取消 Sortable，改为 `drag::start_drag(window, DragItem::Files([path]))`。目标收纳箱通过**自己的原生 DragDrop 事件**收到真实路径，查库发现属于其他收纳箱 → 改归属。系统拖影跨窗口可见 | 同一机制，Explorer/桌面收到真实的 `CF_HDROP` | 中：引入 `drag` 2.1.1 / `tauri-plugin-drag` 2.1.1（crabnebula-dev/drag-rs，依赖 windows 0.52，又多一份 windows crate） |

方案 B 依据：MS Learn ICoreWebView2File："You can use this object to obtain the path of a File dropped on WebView2"。在 Chromium 拖放路径（`ui/base/dragdrop/os_exchange_data_provider_win.cc`、`content/browser/web_contents/web_contents_view_aura.cc`）中检索 `.lnk` / `ResolveShortcut` 没有找到相关代码，**预期**得到 `.lnk` 本身，但仍需 spike。

方案 C 依据：drag-rs 的 Windows 实现（`crates/drag/src/platform_impl/windows/mod.rs`）用的是 `OleInitialize` + `SHCreateDataObject` / `IShellItemArray::BindToHandler(BHID_DataObject)` + `IDragSourceHelper::InitializeFromBitmap` + `DoDragDrop`，支持 Copy / Move，回调里返回 `DragResult` 和光标位置。tao 建窗时已经在主线程 `OleInitialize`（`tao .../window.rs:104`）。注意事项：
  - `DoDragDrop` 是模态的，会在主线程跑自己的消息循环。
  - 如果允许 `DROPEFFECT_MOVE`，Explorer 可能做 "optimized move"，返回值不可靠。**拖完以后直接检查源文件是否还在**，或者只允许 COPY / LINK。
  - 发起时机：前端通过 invoke 进入 Rust 时，鼠标通常仍按着；如果已经松开，会立即 drop 在当前位置，属于可接受的边缘情况。

### 2.4 推荐

1. **MVP 用方案 A**：收纳箱窗口保持默认的原生拖放处理，外部文件拖入走 `onDragDropEvent`，箱内排序用 `force-fallback`，跨箱移动由后端按光标做命中测试。限制：fallback 拖影出不了源窗口（会被窗口边缘裁掉）。可以让后端在拖动期间约 30~60 ms 轮询一次光标，向目标收纳箱 `emit('fence-drag-hover')` 来高亮，作为补偿。
2. **增强阶段用方案 C**：同一套 OLE 机制同时解决"跨箱拖动带系统拖影"和"拖回桌面/Explorer"。
3. 方案 B 只有在需要 HTML5 DnD 时才考虑（例如想用原生拖影做箱内排序）。
4. 指针拖出窗口后，页面是否还能持续收到 `mousemove` / `mouseup`（Chromium 按下鼠标时会 SetCapture）**需 spike 确认**，这是方案 A 跨箱拖动的前提。

### 2.5 能否作为拖动源拖回桌面/Explorer（OLE `DoDragDrop`）

**可行**，但 WebView2 自身做不到（DOM 拖动不产生 `CF_HDROP`；`DownloadURL` 格式只是"拖出即下载"的副本，对 `asset://` 协议也不保证有效）。必须由 Rust 发起 `DoDragDrop` 或 `SHDoDragDrop`（`Win32_UI_Shell`，需要 `Win32_System_Com` + `Win32_System_Ole`，`pDropSource` 传 `None` 时使用系统默认 drop source），或直接用 `drag` crate。**建议 MVP 不做。**

---

## 3. Rust 侧图标提取

### 3.1 API 对比

| API | 尺寸 | `.lnk` / 缩略图 | 复杂度 | 结论 |
|---|---|---|---|---|
| `SHGetFileInfoW(SHGFI_ICON\|SHGFI_LARGEICON)` | 只有 32 px（大图标） | 能解析 `.lnk` 目标图标 | 低 | 只适合兜底（可以加 `SHGFI_USEFILEATTRIBUTES` 按扩展名取、不访问磁盘） |
| `SHGetFileInfoW(SHGFI_SYSICONINDEX)` + `SHGetImageList(SHIL_EXTRALARGE=48 / SHIL_JUMBO=256)` + `IImageList::GetIcon(i, ILD_TRANSPARENT)` → HICON → `GetIconInfo` → `GetDIBits` | 48 / 256 | 图标 | 中（HICON → 位图要处理 mask，旧图标 alpha 全为 0） | 备选。SHIL_JUMBO 对只有 48 px 资源的 exe 会返回"256 画布中间一个小图标" |
| **`IShellItemImageFactory::GetImage(SIZE, SIIGBF)`** | 任意（以物理像素请求） | 默认优先缩略图，没有时取图标；`SIIGBF_ICONONLY` 只取图标；能处理 `.lnk` / `.url` / 文件夹 / UWP 快捷方式 | 中 | **推荐** |
| `PrivateExtractIconsW` | 任意 | 只读 exe/ico 资源 | 低 | MS 标注 "not intended for general use"，不推荐 |

MS Learn GetImage 要点（原文）："The default behavior is to load a thumbnail. If there is no thumbnail … retrieves an HBITMAP for the icon"；"An extraction should never be done on a UI thread"；调用方需要 `DeleteObject` 释放返回的位图；`SIIGBF_BIGGERSIZEOK` 允许返回更大的图由调用方自行缩放（Shell 内部用的是 GDI StretchBlt，质量一般）；`SIIGBF_SCALEUP`（Win8+）会强制放大。

### 3.2 推荐流程

```text
[专用 STA 线程 "fence-shell"]  CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE)，整个线程生命周期只初始化一次
  loop recv(job):
    item: IShellItem = SHCreateItemFromParsingName(&HSTRING::from(path), None)?
    factory: IShellItemImageFactory = item.cast()?
    hbmp = factory.GetImage(SIZE{cx:px, cy:px}, SIIGBF_ICONONLY | SIIGBF_BIGGERSIZEOK)?
        // 图片文件想显示缩略图时去掉 ICONONLY（即 RESIZETOFIT=0）
    GetObjectW(hbmp, size_of::<BITMAP>(), &mut bm)   → 宽、高、bmBitsPixel
    hdc = CreateCompatibleDC(None)
    GetDIBits(hdc, hbmp, 0, h, buf, &mut BITMAPINFO{ biHeight: -h /*自上而下*/, biBitCount: 32, biCompression: BI_RGB }, DIB_RGB_COLORS)
    DeleteDC(hdc); DeleteObject(hbmp)
    BGRA → RGBA（交换 B/R）
    如果所有像素 alpha == 0 → 全部置 255（无 alpha 的缩略图或旧图标）
    如果是预乘 alpha → 反预乘 c = c*255/a（见 3.6）
    png::Encoder(Rgba, Eight) 写到缓存文件
```

**PNG 编码**：用 `png = "0.17"`。Cargo.lock 里 png 0.17.16 已经被 `ico` / `muda` / `tauri-codegen` / `tray-icon` 依赖，**新增直接依赖不增加编译量**。不要为此引入 `image` crate（很重）。最新版是 png 0.18.1，用 0.17 是为了与树里的版本合并。

**交给前端**：写成文件后用 `convertFileSrc(path)` 加载。项目里 `tauri.conf.json` 的 `assetProtocol` 已启用且 scope 为 `**`，`MarkdownEditor.vue:90` 已有同样用法。比起 base64 data URL（项目已有 `base64` 0.22 依赖），文件方式不会撑大 IPC 和数据库。

### 3.3 windows 0.58 需要新增的 feature（已在 `windows-0.58.0` 源码核对 cfg）

| Feature | 用途 |
|---|---|
| `Win32_UI_Shell` | `SHCreateItemFromParsingName`、`IShellItem`、`IShellItemImageFactory`、`SIIGBF_*`、`SHGetKnownFolderPath`、`FOLDERID_Desktop` / `FOLDERID_PublicDesktop`、`ShellExecuteExW` / `ShellExecuteW`、`IShellLinkW` + `ShellLink` CLSID、`IFileOperation`、`SHChangeNotify`、`SHGetImageList`、`SHDoDragDrop` |
| `Win32_System_Com` | `CoInitializeEx`、`CoCreateInstance`、`CoTaskMemFree`、`IPersistFile`；`SHCreateItemFromParsingName` 本身也要求这个 feature |
| `Win32_Graphics_Gdi` | `HBITMAP`、`BITMAP`、`BITMAPINFO`、`GetDIBits`、`GetObjectW`、`CreateCompatibleDC`、`DeleteObject`；`IShellItemImageFactory::GetImage` 也要求它 |
| `Win32_UI_Shell_Common` | `ITEMIDLIST`、`ILCreateFromPathW`、`ILFree`、`SHOpenFolderAndSelectItems`、`SHCreateDataObject` |
| `Win32_System_Registry` | `ShellExecuteExW`（`SHELLEXECUTEINFOW.hkeyClass` 是 HKEY） |
| `Win32_Storage_FileSystem` | `IShellLinkW::GetPath`（`WIN32_FIND_DATAW`）、`SetFileAttributesW`、`SHGetFileInfoW` |
| `Win32_UI_Controls` | 只有走 `IImageList` 备选方案时才需要 |
| `Win32_System_Ole` | 只有做拖出（`DoDragDrop` / `SHDoDragDrop`）时才需要 |

现有 feature：`Win32_Foundation`、`Win32_UI_WindowsAndMessaging`、`Win32_Graphics_Dwm`、`Win32_Graphics_DirectWrite`（`pc/src-tauri/Cargo.toml:45`）。

### 3.4 COM 线程

- Shell 对象要求 STA，而且 GetImage 可能很慢（网络路径、离线盘、缩略图生成）。**不要在主线程（tao 事件循环）里调用**，也**不要在 tokio worker 上调用**：worker 线程会被复用，可能已被别的代码初始化成 MTA，此时再调 `CoInitializeEx` 返回 `RPC_E_CHANGED_MODE`。
- 推荐一个常驻的 `std::thread`，接收 `std::sync::mpsc` 任务（`ExtractIcon` / `Launch` / `Reveal` / `ResolveLnk`），用 tokio `oneshot` 把结果回给 async 命令。同一路径的请求要去重。
- `COINIT_DISABLE_OLE1DDE` 是 MS 对 ShellExecuteEx 的推荐写法（见第 4 节）。

### 3.5 缓存

- 键：`sha256(lowercase(path)) + "_" + px`（项目已有 `sha2` 依赖）。校验信息（文件 mtime + size）存在数据库或 sidecar 中，变化时重新提取。
- 路径：`dirs::data_local_dir()/mini-todo/fence-icons/`，与现有 `images/` 目录（`commands/todo.rs:329-334`）、数据库（`db/connection.rs:35-40`）同级。注意代码实际用的是 `%LOCALAPPDATA%`，与 CLAUDE.md 里写的 `%APPDATA%` 不一致。
- `.lnk` 的 mtime 不会随目标 exe 更新而变，所以要提供"刷新图标"操作，或在应用启动时做低频全量校验。
- 收纳箱和路径都是设备相关的，图标缓存**不进 WebDAV / 导出**。

### 3.6 DPI 与坑

- `GetImage` 的 SIZE 是**物理像素**。按 `显示尺寸(逻辑) × scale_factor` 向上取到常用档位 [32, 48, 64, 96, 128, 256] 请求，CSS 再按逻辑尺寸显示，Chromium 缩小的效果不错。收到 `ScaleFactorChanged`（窗口被拖到另一块 DPI 不同的屏）时按新档位重新请求，或者干脆统一缓存 2 倍尺寸。
- **alpha 是否预乘需 spike**：图标位图通常是 32bpp 预乘（PARGB），但社区代码里两种处理方式都有。检测方法：只要有任一像素满足 `max(r,g,b) > a`，就说明不是预乘。如果按错误的假设处理，半透明边缘会出现黑边或白边。
- 只有 32/48 px 资源的旧 exe 按大尺寸请求时，可能得到"大画布中间一个小图标"，或者 GDI 放大后的模糊图。可以计算不透明区域的包围盒，太小时改按 48 px 重新请求，再由 CSS 放大。
- 是否带 `.lnk` 小箭头：箭头是视图层加的 overlay，**预期** GetImage 不带。需 spike。
- 提取失败（目标丢失、权限不足）时，前端显示 Element Plus 图标占位，或者用 `SHGetFileInfoW + SHGFI_USEFILEATTRIBUTES` 按扩展名取通用图标兜底。

---

## 4. 启动项目

### 4.1 `tauri-plugin-opener` 实际做了什么（源码）

- `tauri-plugin-opener-2.5.3/src/open.rs:54-61` `open_path`：`with == None` 时先 `path.metadata()?`，路径不存在会直接返回 IO 错误，可以用来**检测文件丢失**，然后调 `open::that_detached`。
- opener 依赖的 `open` crate 开启了 `shellexecute-on-windows` feature（opener `Cargo.toml` 第 59-61 行）。在 `open-5.3.3/src/windows.rs:40-67` 中：文件走 `ShellExecuteExW`，`lpVerb = NULL`（默认动作）、`nShow = SW_SHOWNORMAL`、**`fMask = 0`**（会显示错误 UI），且**不设置 `lpDirectory`**；目录走 `:71-81` 的 `SHOpenFolderAndSelectItems(folder, [folder])`（为了遵守"展开到打开的文件夹"设置，见 open-rs PR #104）。
- `reveal_item_in_dir`（`reveal_item_in_dir.rs:99-158`）：`SHOpenFolderAndSelectItems`，遇到 `ERROR_FILE_NOT_FOUND` 时退回 `ShellExecuteExW("explore")`。它就是"打开文件所在位置"，比手拼 `explorer /select,` 更稳（不需要处理引号转义）。
- 权限：`capabilities/default.json` 里的 `opener:default` **只允许** `open_url`（http/https/mailto/tel）和 `reveal_item_in_dir`，不允许前端直接调 `open_path`。**启动应该做成自定义 Rust 命令**（Rust 侧 API 不做 scope 检查）。`capabilities/default.json` 的 `windows: ["*"]` 已覆盖收纳箱窗口。

### 4.2 推荐：自定义 `fence_launch(item_id)`，在 STA 线程里调 `ShellExecuteExW`

```text
SHELLEXECUTEINFOW {
  fMask: SEE_MASK_FLAG_NO_UI (| SEE_MASK_NOASYNC) (| SEE_MASK_HMONITOR，在收纳箱所在显示器上打开),
  lpVerb: NULL（默认）/ "runas"（以管理员身份运行）/ "openas"（打开方式）/ "properties"（需 SEE_MASK_INVOKEIDLIST）,
  lpFile: path,
  lpDirectory: 如果是 .exe 则为其父目录（.lnk 自带工作目录，不需要设）,
  nShow: SW_SHOWNORMAL
}
```

MS Learn ShellExecuteEx Remarks（原文）："COM should be initialized before ShellExecuteEx is called … CoInitializeEx(NULL, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE)"。`SEE_MASK_FLAG_NO_UI`："Do not display user interface (UI) error dialogs … Security prompts are exempted"，即 UAC 提示仍会出现。

也可以直接调 `tauri_plugin_opener::open_path(path, None::<&str>)`，结果基本相同，只是没有 `NO_UI` / `lpDirectory` / 动作的控制，而且它在 opener 的 async 命令线程里执行，没有 STA 保证。

### 4.3 各类型行为

| 类型 | 行为 | 说明 |
|---|---|---|
| `.exe` | 正常启动；需要管理员权限的程序会弹 UAC（ShellExecute 会处理提升，CreateProcess 只会返回 `ERROR_ELEVATION_REQUIRED`） | 建议设 `lpDirectory`，否则依赖当前目录的老程序可能出错 |
| `.lnk` | Shell 解析快捷方式并按其中的参数和工作目录启动；指向 `shell:AppsFolder\<AUMID>` 的 IDList 型快捷方式（UWP / 商店应用）也能启动 | **目标丢失**时默认弹"快捷方式存在问题"或搜索对话框；加 `SEE_MASK_FLAG_NO_UI` 后改为返回错误。预检：`CoCreateInstance(ShellLink)` → `IPersistFile::Load` → `IShellLinkW::GetPath(SLGP_RAWPATH)` + `Path::exists`。IDList 型 `.lnk` 的 GetPath 为空，只能直接启动 |
| `.url` | 用默认浏览器打开 | |
| 文件夹 | ShellExecute 默认动作打开 Explorer；opener 用 `SHOpenFolderAndSelectItems` 技巧，会复用已打开的 Explorer 窗口 | |
| UWP（不经 `.lnk`） | `lpFile = "shell:AppsFolder\\<AUMID>"` | 拖入时不一定拿得到路径（见 2.2） |
| 路径不存在 | 先 `metadata()` 检查，前端把该项标灰，提供"移除 / 重新定位"；`.lnk` 还要检查目标 | |

**前台激活的坑**：新进程能否拿到前台，取决于调用方是否持有前台权限。用户刚点过收纳箱，收纳箱处于激活状态，所以没问题。但如果把收纳箱设成 `focusable(false)` / `WS_EX_NOACTIVATE`，新窗口可能在后面打开或只在任务栏闪烁。这一点要和 1.5 第 4 条一起考虑。

---

## 5. 把文件"移入"收纳箱（替代引用）

### 5.1 获取真实桌面路径

- `dirs::desktop_dir()`：项目已依赖 dirs 5，底层 `dirs-sys-0.4.1/src/lib.rs:150-170` 就是 `SHGetKnownFolderPath(FOLDERID_Desktop, 0, NULL)` + `CoTaskMemFree`，**能正确处理 OneDrive / 组策略重定向**（例如 `C:\Users\x\OneDrive\桌面`）。
- 公共桌面：`SHGetKnownFolderPath(&FOLDERID_PublicDesktop, KF_FLAG_DEFAULT, None)`（dirs 没有封装），通常是 `C:\Users\Public\Desktop`。
- 用户看到的桌面是"用户桌面 + 公共桌面"合并后的结果。拖入项的路径前缀属于哪个目录，决定了能否移动。

### 5.2 存放位置对比

| 选项 | 优点 | 风险 |
|---|---|---|
| a. `%LOCALAPPDATA%\mini-todo\fences\<id>\` | 不出现在桌面上，与 OneDrive 无关 | 跨卷时要复制再删除（慢，大文件夹需要进度）；**OneDrive 桌面中的文件移出后，等于从云端删除**：其他设备的桌面上文件消失，备份也随之失去；Files On-Demand 的占位文件移动时会触发下载，离线时会失败；卸载后文件"藏"在 AppData 里，用户不知道去哪找 |
| b. 桌面下的隐藏目录 `<Desktop>\.mini-todo-fences\<id>\`（`SetFileAttributesW(HIDDEN\|SYSTEM)`） | 同卷移动只是 rename（瞬间完成、原子、元数据不变）；卸载后文件仍在 | 用户开启"显示隐藏的项目"后能看到；**OneDrive 会把这次移动同步到所有设备**，其他设备上文件"消失"进隐藏目录，而那些设备上的 mini-todo 并不知道 |
| c. **收纳箱绑定一个真实可见的文件夹（推荐用于移入场景）**，例如 `%USERPROFILE%\收纳箱\<名称>` 或用户自选目录；收纳箱显示该文件夹的内容 | 语义透明，用户随时能在 Explorer 里找到；不依赖数据库与文件系统保持一致；多设备 OneDrive 下也只是一个普通文件夹；卸载无影响 | 需要监听目录变化：项目 `Cargo.toml:42` 已声明 `notify = "7"`，但 `src/` 里**没有使用**；收纳箱里的排序要另外存 |
| d. **引用模式（推荐 MVP）**：只在数据库里存路径，不动文件 | 零数据风险 | 桌面不会变干净，用户需要自己删除桌面上的原图标 |

### 5.3 移动与还原的实现要点（如果选择 a / b）

- 优先用 `IFileOperation`（`CoCreateInstance(FileOperation)`）`MoveItem` + `PerformOperations`，flags 如 `FOF_RENAMEONCOLLISION | FOF_NOCONFIRMMKDIR | FOFX_ADDUNDORECORD`。好处：自动发 Shell 变更通知，桌面立即刷新；跨卷时有进度 UI；碰到权限问题会弹 Explorer 同款的"需要管理员权限，继续"对话框。用 `std::fs::rename` 也行，但之后要补 `SHChangeNotify(SHCNE_RENAMEITEM / SHCNE_UPDATEDIR, SHCNF_PATHW, …)`。
- **崩溃一致性**：先写数据库记录（`status=moving`, `original_path`），再移动，成功后更新为 `stored`。启动时对账：存储目录里有、数据库里没有的文件要提示"还原到桌面"，数据库里有、文件已不存在的项要标记丢失。
- **删除收纳箱时还原**：逐项移回 `original_path` 所在目录；原目录不存在就放回 `dirs::desktop_dir()`。重名时加 " (2)" 后缀（或用 `FOF_RENAMEONCOLLISION`）。
- **公共桌面上的项**：普通用户只有读权限，移动或删除会得到 `ERROR_ACCESS_DENIED`；用 `IFileOperation` 会弹提升对话框，之后还原时还要再提升一次。建议对这类项**强制使用引用模式**（可以复制一份 `.lnk` 到收纳箱存储，但无法从公共桌面隐藏原图标）。
- **重名**：例如用户桌面和公共桌面各有一个 `Chrome.lnk`，或者还原时桌面上已有同名文件。存储时每项放在独立子目录（`<id>/<item-uuid>/name`），或者在数据库中记录原名和实际名。
- 被占用的文件（如打开着的 docx）移动会因共享冲突失败；文件夹内有进程把它当作当前目录时，rename 也会失败。
- 长路径（> MAX_PATH）：`std::fs` 能处理；`IFileOperation` 基本也能。需验证。
- 不要移动 `desktop.ini`（隐藏的系统文件，wry 一般也拖不到它）。
- Explorer 的 Ctrl+Z 不知道我们做过的移动（除非走 `IFileOperation` + `FOFX_ADDUNDORECORD`）。

---

## 6. 多显示器 / DPI

### 6.1 现有主窗口的做法（可以直接借鉴）

- `pc/src/stores/appStore.ts:102-130`：`generateScreenConfigId()` = `显示器数量_宽x高@缩放%_...`（按面积排序），表示一种"屏幕组合"。
- `appStore.ts:172-205`：按组合 ID 读取 `screen_configs`，用 `PhysicalPosition` 恢复位置，**之后**再用 `PhysicalSize` 恢复尺寸，最后才切换模式。
- `screen_configs` 是设备相关数据，不参与同步（CLAUDE.md 同步覆盖范围表）。

### 6.2 推荐存储

- **位置存物理像素**（虚拟屏幕坐标）。tao 是 Per-Monitor v2 DPI 感知的，在混合 DPI 环境下逻辑坐标有歧义：tauri 的 `LogicalPosition` 按窗口**当前**所在屏的 scale 换算，跨屏目标会算错。
- **尺寸**：存逻辑像素（跨 DPI 时视觉大小不变），或者存物理像素并同时记录当时的 scale。跨屏移动后 tao 处理 `WM_DPICHANGED` 时会按系统建议的矩形缩放，**一定要先 set_position、再 set_size**（与主窗口同序），否则尺寸会被二次缩放。
- 每个收纳箱按"屏幕组合 ID"保存一份几何信息（例如新表 `fence_layouts(fence_id, screen_config_id, x, y, w, h, collapsed)`），单屏和双屏布局互不覆盖。
- tauri 2.9.5 可用的 API：`app.available_monitors()`、`app.monitor_from_point(x, y)`、`Monitor::work_area()`（`tauri-2.9.5/src/window/mod.rs:98`，物理矩形）、`app.cursor_position()`。

### 6.3 显示器断开 / 重连

- 断开后系统会把落在消失屏幕上的顶层窗口挪到剩下的屏幕上（Win11 还有"根据显示器连接记住窗口位置"选项，重连时恢复）。被 Progman 持有的 tool window 是否同样被挪、挪到哪里，**未验证**。
- **坑**：系统挪窗会触发 `WindowEvent::Moved`。如果收到 Moved 就落库，会用临时位置**覆盖**原屏幕组合下的布局。应对办法：
  1. 只在用户主动拖动结束后落库（前端 `startDragging` 之后做 debounce），并且只写入**拖动开始时**的屏幕组合 ID；
  2. 后端轮询线程每 1~2 s 比较一次 `available_monitors()` 生成的组合 ID（Tauri 没有显示器变化事件；如果要接 `WM_DISPLAYCHANGE`，就得用 `SetWindowSubclass` 子类化），组合变化时按新组合恢复各收纳箱的位置；没有记录的就用上次的位置，并夹回最近显示器的 `work_area`（`monitor_from_point`，取不到时用 primary）。
- 折叠后展开时，如果下方空间不够，就向上展开（按 `work_area` 计算）。

---

## 7. 新增依赖 / 配置汇总（候选）

```toml
# pc/src-tauri/Cargo.toml
[target.'cfg(windows)'.dependencies]
windows = { version = "0.58", features = [
  "Win32_Foundation", "Win32_UI_WindowsAndMessaging", "Win32_Graphics_Dwm", "Win32_Graphics_DirectWrite",  # 已有
  "Win32_UI_Shell", "Win32_UI_Shell_Common", "Win32_System_Com", "Win32_Graphics_Gdi",                      # 图标 / 启动 / 桌面路径
  "Win32_System_Registry", "Win32_Storage_FileSystem",                                                      # ShellExecuteExW / IShellLinkW::GetPath / 文件属性
  # "Win32_UI_Controls",   # 仅 IImageList 备选方案需要
  # "Win32_System_Ole",    # 仅自研拖出需要
] }
png = "0.17"            # 已在依赖树中，不增加编译量
# drag = "2.1"          # 增强阶段：拖出 / 跨箱拖动（带入 windows 0.52）
```

前端：收纳箱路由 `#/fence?id=`，`getCurrentWebview().onDragDropEvent`；vuedraggable 保持 `:force-fallback="true"`。

---

## Files Found

| File Path | Description |
|---|---|
| `pc/src-tauri/src/commands/window.rs` | 桌面模式实现：`IS_DESKTOP_MODE`(34)、`DESKTOP_OWNER`(43)、`window_hwnd`(868)、`apply_fixed_ex_style`(879)、`win32_show`(949)、`desktop_host`(1056)、`desktop_attach`(1082)、`desktop_detach`(1119)、`tick_desktop_mode`(1144)、`reassert_window_mode_state`(1185)、`leave_desktop_mode`(1213)、`set_window_desktop_mode`(1235) |
| `pc/src-tauri/src/lib.rs:261-282` | 200 ms 轮询线程，只处理 `"main"` |
| `pc/src-tauri/src/services/notification.rs:222-252` | Rust 侧 `WebviewWindowBuilder` 建无边框、透明、不进任务栏窗口的范例 + Destroyed 监听 |
| `pc/src-tauri/tauri.conf.json` | 没有 `dragDropEnabled`（即默认开启）；`assetProtocol` 已启用且 scope 为 `**` |
| `pc/src-tauri/capabilities/default.json` | `windows: ["*"]`；`opener:default` 不允许前端调 `open_path` |
| `pc/src-tauri/Cargo.toml` | windows 0.58 现有 4 个 feature；已有 `dirs` 5、`sha2`、`base64`、`notify` 7（未使用）、`tauri-plugin-opener` |
| `pc/src/components/TodoList.vue:59-67`、`pc/src/components/QuadrantView.vue:122-131`、`pc/src/views/EditorView.vue:1225-1233` | vuedraggable 统一使用 `force-fallback` |
| `pc/src/views/SettingsView.vue:330-333`、`pc/src/views/EditorView.vue:837-840` | `@mousedown → startDragging()` 写法（避开 drag-region 双击最大化） |
| `pc/src/stores/appStore.ts:102-205` | 屏幕组合 ID + 物理坐标恢复（先位置后尺寸） |
| `pc/src-tauri/src/commands/todo.rs:329-334`、`pc/src-tauri/src/db/connection.rs:35-40` | 本地数据目录实际是 `%LOCALAPPDATA%\mini-todo\` |
| `pc/scripts/e2e/` | 窗口模式 e2e 工具（e2e.ps1 / cdp.mjs / mouse.ps1），可扩展到多窗口 |

## Code Patterns（第三方源码核对位置，均在本机 `~/.cargo/registry/src/index.crates.io-*/`）

- `tao-0.34.5/src/platform_impl/windows/window_state.rs:242-300`（`to_window_styles`：没有 TOOLWINDOW，`ON_TASKBAR` 对应 APPWINDOW）、`:315-323`（无 diff 时提前返回）、`:425-440`（整体覆写样式）
- `tao-0.34.5/src/platform_impl/windows/window.rs:104`（OleInitialize）、`:1158-1165`（`OwnedBy` 时不设 `ON_TASKBAR`）、`:1334` → `:1347`（先 skip_taskbar 后 set_visible）
- `tao-0.34.5/src/platform_impl/windows/event_loop.rs:1230-1233`（`ALWAYS_ON_BOTTOM` 钩子）、`:2278-2281`（TaskbarCreated）
- `wry-0.53.5/src/webview2/mod.rs:149-157`、`wry-0.53.5/src/webview2/drag_drop.rs:67`（inject_in_hwnd）、`:159-200`（DragEnter，只认 CF_HDROP、固定 COPY）、`:226`（Drop）
- `tauri-runtime-wry-2.9.3/src/lib.rs:4631-4662`（原生拖放事件映射）
- `tauri-2.9.5/src/webview/webview_window.rs:62-63`（同步命令建窗会死锁）、`:761`（owner_raw）、`:1027-1032`（disable_drag_drop_handler）；`tauri-2.9.5/src/window/plugin.rs:198-210`（toggle_maximize 只检查 resizable）；`tauri-2.9.5/src/webview/mod.rs:181`（`PlatformWebview::controller()`）
- `tauri-plugin-opener-2.5.3/src/open.rs:54-61`、`reveal_item_in_dir.rs:99-158`；`open-5.3.3/src/windows.rs:40-81`
- `dirs-sys-0.4.1/src/lib.rs:150-170`（SHGetKnownFolderPath）
- `windows-0.58.0/src/Windows/Win32/UI/Shell/mod.rs`：`SHCreateItemFromParsingName`(2783，要求 System_Com)、`IShellItemImageFactory::GetImage`(18894，要求 Graphics_Gdi)、`ShellExecuteExW`(4590，要求 System_Registry)、`SHOpenFolderAndSelectItems`(3779，要求 Shell_Common)、`SHDoDragDrop`(3073，要求 Com + Ole)、`SIIGBF_*`(27376-27385)

## External References

- [MS Learn – IShellItemImageFactory::GetImage](https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nf-shobjidl_core-ishellitemimagefactory-getimage)：SIIGBF 各标志的含义；不要在 UI 线程提取；调用方负责 DeleteObject
- [MS Learn – ShellExecuteExW](https://learn.microsoft.com/en-us/windows/win32/api/shellapi/nf-shellapi-shellexecuteexw)：调用前初始化 STA COM（COINIT_DISABLE_OLE1DDE）
- [MS Learn – SHELLEXECUTEINFOW](https://learn.microsoft.com/en-us/windows/win32/api/shellapi/ns-shellapi-shellexecuteinfow)：`SEE_MASK_FLAG_NO_UI` / `SEE_MASK_HMONITOR` / `SEE_MASK_ASYNCOK`
- [MS Learn – The Taskbar](https://learn.microsoft.com/en-us/windows/win32/shell/taskbar)：被 owner 持有的窗口不建任务栏按钮；TOOLWINDOW / APPWINDOW 规则
- [MS Learn – Window Features (Owned Windows)](https://learn.microsoft.com/en-us/windows/win32/winmsg/window-features)
- [MS Learn – WebView2 Process model](https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/process-model)：同一 user data folder 共享 browser 进程，renderer 数量取决于站点隔离
- [MS Learn – ICoreWebView2File](https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/icorewebview2file)、[chrome.webview.postMessageWithAdditionalObjects](https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/javascript/webview)：方案 B 拿到拖入文件真实路径的官方途径
- [tauri-apps/tauri#15138](https://github.com/tauri-apps/tauri/issues/15138)：dragDropEnabled 与 HTML5 DnD 互斥，没有 hybrid 模式；[#14373](https://github.com/tauri-apps/tauri/issues/14373)：文档问题；[#13533](https://github.com/tauri-apps/tauri/issues/13533)
- [Tauri JS API – webview.onDragDropEvent](https://v2.tauri.app/reference/javascript/api/namespacewebview/#ondragdropevent)
- [crabnebula-dev/drag-rs](https://github.com/crabnebula-dev/drag-rs)：`drag` / `tauri-plugin-drag` 2.1.1（2026-05），支持 Windows 拖出，内部用 windows 0.52 + DoDragDrop
- [open-rs PR #104](https://github.com/Byron/open-rs/commit/7595da7d655e910f3cb9d26b96d3947f2acf2522)：目录改用 SHOpenFolderAndSelectItems
- [windows-rs feature 搜索](https://microsoft.github.io/windows-docs-rs/)：按 API 名查所需 feature（本文已对照 0.58 源码 cfg 核实）
- [docs.rs – png 0.17](https://docs.rs/png/0.17.16/png/)

## Related Specs

- `.trellis/spec/backend/window-modes.md`：主窗口三种模式的 Win32 状态表、排序规则 1-6、Explorer 重启的错误矩阵、e2e 步骤，以及"owner=Progman 而不是 SetParent"的设计决策。收纳箱必须遵守同一套排序规则；它的 Extensibility 段预留了按窗口泛化的接缝。

## Caveats / Not Found（需要 spike 验证）

1. 多个收纳箱 WebView2 的**实际内存占用**（同源实例是否共享 renderer）。
2. 用 `GWLP_HWNDPARENT` 挂到 Progman 的 N 个窗口在 Explorer 重启后都能存活并重挂（目前只对单窗口实测过）；建窗时直接 `owner_raw(Progman)` 的行为未测试。
3. 方案 A：指针拖出源窗口后，页面能否持续收到 `mousemove` / `mouseup`（跨箱拖动的前提）。
4. 方案 B：同一个 environment 下两个 WebView2 之间的 HTML5 跨窗口拖放；Chromium 对拖入 `.lnk` 是否解析为目标（源码检索未发现解析逻辑）。
5. `IShellItemImageFactory::GetImage` 返回位图的 alpha 是否预乘；`.lnk` 图标是否带箭头 overlay。
6. 从开始菜单拖 UWP 或 Win32 应用磁贴进来时，数据对象里有没有 `CF_HDROP`。
7. 显示器断开时，被 Progman 持有的 tool window 会被系统怎样挪动；虚拟桌面下这类窗口的可见性。
8. Win10 / Win11 ≤23H2 下 Win+D 会盖住收纳箱（已知限制，需要在 PRD 里定下支持范围）。
