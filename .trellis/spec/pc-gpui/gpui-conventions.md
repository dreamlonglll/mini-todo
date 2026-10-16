# GPUI 约定（`pc-gpui/crates/app`）

> app crate 的可执行约定。来源：`pc-gpui/crates/app/README.md` §3「约定」与 §2.1「维护规则」、
> `.trellis/tasks/archive/2026-10/10-15-gpui-rewrite/research/gpui-api.md` §0「结论速览」，以及实现过程中踩过的坑。
> 版本：`gpui-kit =0.7.3`（re-export gpui-pre 0.3.9、gpui-pre-platform、gpui-base 0.7.3、gpui-component 0.7.3）。
> API 以本机 `~/.cargo/registry/src/index.crates.io-*/gpui-pre-0.3.9`、`gpui-pre-windows-0.3.9`、`gpui-pre-macos-0.3.9`、
> `gpui-pre-apple-0.3.9`、`gpui-component-0.7.3` 等源码为准——gpui-pre 是 Zed 的每周快照，网上的示例、文档常常对不上这个版本。
> macOS 专属约定见 §2.15（任务 `10-16-macos-package`；mac 平台层没有真机验收，结论来自源码与 CI 的 macOS runner）。
> 接口清单（`AppState` 方法、`WindowManager`、`ui::*` 控件、`platform::*`、`services::*`）见 app README §4，这里不重复。

---

## 1. Scope / Trigger

- 任何 `pc-gpui/crates/app/**` 的改动。
- 尤其：新窗口种类、新控件、动到 `platform/` / `services/window_mode.rs` / `services/fences.rs`、改 `theme.rs`、
  新 debug 开关、升级 `gpui-kit`；任何 `cfg(target_os = "macos")` 代码与 `cfg(not(windows))` 分支（它们在 mac 上会被编译、运行，§2.15）。

---

## 2. 规则

### 2.1 阻塞调用一律放后台，回前台用 `WeakEntity`

- `mini-todo-core` 的函数**全部阻塞**（SQLite / 文件 / 网络 / Shell）。在前台（事件处理、`render`、`update`）里直接调用会卡住
  **所有**窗口（同一个 GPUI 主线程）。
- 数据库：`db::spawn_db(cx, |db| mini_todo_core::…)`（需要 `Arc<Database>` 时 `spawn_db_arc`），返回 `Task<Result<T, String>>`；
  其它阻塞工作：`cx.background_executor().spawn(async move { … })`。
- 回前台：在 `cx.spawn` / `cx.spawn_in(window, …)` 的 async 块里 `this.update(cx, …)` / `this.update_in(cx, …)`
  （`WeakEntity`，返回 `Result`，视图已关闭时是 `Err`，忽略即可）。**不要**用 `AsyncApp::update` 去碰可能已销毁的实体
  （app 已释放时 panic）。
- `Task` 被 drop 即取消：要么存进视图字段，要么 `.detach()`。`Subscription` 同理（存进 `_subscriptions`）。
- 跨窗口共享的写操作走 `AppState` 的方法（`create_todo` / `update_todo` / …，内部已放后台并维护本地副本），不要在窗口里
  自己 `spawn_db` 写待办再手动刷新。
- `reqwest::blocking`（core 的同步 / 节假日 / 版本检查）不能在 tokio runtime 里调用；app 不引入 tokio。
- **带 `owner_hwnd` 的收纳箱移动**（`fence::delete`、`fence::items::{move_in, restore, move_to}`）绝不在 UI 主线程调用：
  系统的"需要管理员权限 / 文件正在使用"对话框以收纳箱窗口为父窗口，会同步给主线程发消息，主线程阻塞等待时死锁。
  macOS 不用 `owner_hwnd`，但「文件与文件夹」隐私授权框、下载 iCloud 云端文件、跨卷复制同样阻塞调用线程，规则相同。
  HWND 在主线程取好（`isize`），移动派到后台。

### 2.2 App 借用期间禁止同步 Win32 调用（`platform` 延后）

- `SetWindowPos` / `ShowWindow` / 改 style / `SendMessage` 会**同步**进入 GPUI 的窗口过程（`WM_MOVE` / `WM_SIZE` / `WM_ACTIVATE`…）。
  如果此刻 App 正被借用（事件处理、`update` 闭包、`render`），GPUI 拿不到 App，只能丢掉这次几何变化 → 窗口尺寸 / 位置状态错乱，
  模态移动循环里甚至 panic。
- `platform::*(window, cx, …)` 形式（接收 `&Window`）的**写操作已自动延后**到下一个前台任务，可以在事件处理里直接调用。
- `platform::Hwnd` 上的方法、`platform::embed::*`、`subclass::set_flag` 是**立即执行**的原始调用：只能在 `cx.spawn` 的 async 块里、
  `update` 闭包**之外**调用（200ms 守护轮询、窗口模式切换都是这么写的）。
- 读操作（`GetWindowRect`、显示器枚举、光标位置）不发消息，随时可调。
- 任何**模态**系统调用（同步拖窗循环、同步文件 / 取色对话框）都不能在事件处理里直接调：`platform::file_dialog` / `fence_win::choose_color`（Windows；
  mac 的取色面板非模态，在前台任务里显示后轮询，§2.15）
  已放独立线程，`begin_window_drag` 已延后。
- 窗口代码里**不直接写 Win32 / AppKit**：新能力加在 `platform/`（`windows.rs` 实现 + `macos/` 实现 + `fallback.rs` 的 no-op / 近似 +
  `mod.rs` 跨平台签名；mac 的分层见 §2.15）。macOS 的 `setFrame:` 等同样会同步触发 GPUI 回调，本节规则对 mac 一样成立。
- 子类化窗口过程只用 `SetWindowSubclass`（`platform/subclass.rs`），**不要碰 `GWLP_USERDATA`**（GPUI 用它存窗口状态）。
- GPUI 建窗后**不会**改写 ex style（tao 会），所以旧版为对抗 tao `apply_diff` 写的排队重申不需要；但 GPUI 的缩放热区与
  `WindowControlArea` 命中测试按**建窗时**的 `is_resizable` 计算，没有运行时开关 → 运行时锁定位置 / 尺寸用
  `platform::set_geometry_locked`（子类化 `WM_NCHITTEST` / `WM_SYSCOMMAND`），永远不可缩放的窗口建窗时就给 `resizable: false`。
- 不要给 GPUI 窗口加 `WS_EX_LAYERED`（与 DirectComposition 透明冲突）；不要设 `GPUI_DISABLE_DIRECT_COMPOSITION`（透明失效）。
- **几何一律按客户区（= 可见区域）**：GPUI 窗口在左 / 右 / 下有系统的不可见边框（96 DPI 下各 8px；不可缩放、`PopUp` 的窗口也有——
  没有 `WS_POPUP` 的顶层窗口会被系统补上 `WS_CAPTION`），只有顶边被 GPUI 的 `WM_NCCALCSIZE` 去掉。旧 Tauri 版（已删除）的无边框窗口外框 = 客户区，
  库里的主窗口 / 收纳箱几何、通知的定位算法沿用它，都是客户区。摆窗用 `platform::set_client_rect_physical` / `set_client_position_physical`
  或 `Hwnd::set_client_rect` / `set_client_position`，读用 `client_rect_physical` / `Hwnd::client_rect`；外框（`window_rect_physical`、
  `Hwnd::rect` / `set_rect`）只用于换算。
- GPUI 建窗时 `calculate_window_rect` 把只在底部的边框高度对半分到上下，按 `WindowOptions.window_bounds` 建出的客户区比请求的高
  `height_offset / 2`（96 DPI 下 4px，e2e N02 / 主窗口居中 y=236）。`windows::open_window` 建好后统一按客户区校正一次
  （`WindowSpec::client_rect` 给精确物理矩形，否则 `bounds` × 目标显示器缩放），窗口代码不要再各写一遍。
- 不要子类化 `WM_NCCALCSIZE` 去掉这圈边框：GPUI 只在建窗和 `WM_DPICHANGED` 时缓存边框差值（`border_offset`），之后改边框会让它的
  `WM_GETMINMAXINFO` 最小尺寸、`resize`、`window_bounds` 都按旧差值算错。代价是外框外沿那圈不可见边框仍会接住鼠标（`HTBORDER`）。
- **GPUI 窗口不带 `WS_SYSMENU`**：GPUI 建出的无标题栏窗口（`WindowKind::PopUp` 除外，它本来就没有）带 `WS_SYSMENU`、被系统补上
  `WS_CAPTION`，标题栏样式还在（只是 `WM_NCCALCSIZE` 把客户区顶边拉到窗口顶），DWM 照常在右上角布置系统标题栏按钮
  （`DWMWA_CAPTION_BUTTON_BOUNDS` 非空）并画在客户区内容之下——DirectComposition 透明窗口（收纳箱、深色半透明的主窗口）的半透明处
  会透出一个系统 ×，激活时还带红底。`windows::open_window` 建好后在客户区校正的同一个延后任务里先对**所有**窗口
  `Hwnd::set_system_menu(false)`（排在窗口代码自己的 Win32 操作之前，隐藏建窗的收纳箱显示时已经没有按钮），窗口代码不要再各写一遍，
  运行时改样式也不要把它加回来。只去这一位：`WS_CAPTION` / `WS_MINIMIZEBOX` / `WS_MAXIMIZEBOX` / `WS_THICKFRAME` 不动（外框 ↔ 客户区换算、
  窗口模式、任务栏单击最小化、标题栏双击最大化靠它们）；`WS_SYSMENU` 不改边框厚度，GPUI 缓存的 `border_offset` 仍然有效。代价是
  没有系统菜单（Alt+Space、标题栏右键菜单）；其余系统行为实测不受影响（Win11 26H2，2026-10-16）：普通模式单击任务栏按钮最小化 / 还原
  （S08）、标题栏双击最大化 / 还原（S01）、Win+↑ / Win+↓、拖到屏幕顶边贴靠最大化、Alt+F4。GPUI 运行期不改写 `WS_SYSMENU`（只有本项目不用的 `toggle_fullscreen` 整体重写样式），不加守护，
  由 Windows e2e（W 组、S01–S03 / S07 / S08、F01、N01 断言无 `WS_SYSMENU` 且按钮区宽度为 0；S01 真实双击最大化、S08 真实单击任务栏按钮
  最小化）兜底。

### 2.3 拖动窗口

- Windows 上 `window.start_window_move()` 是**空操作**。
- 普通标题栏：`ui::title_bar::drag_region()` 作为标题栏（`relative()`）的**第一个子元素**；Windows 上走 `WindowControlArea::Drag`
  → `HTCAPTION`（系统拖动：贴边分屏、可缩放窗口双击最大化），其它平台 `start_window_move()`。
- 拖动区之上的**可交互元素必须 `.occlude()`**（`EpButton` 已是），否则 Windows 上点它变成拖窗口；纯文字不用。
- 不可拖动的状态（主窗口固定模式）不放拖动区即可（`WindowSpec` 不暴露 GPUI 的 `is_movable`，保持默认 `true`；
  设成 `false` 会让 Drag 区整体失效）。
- 需要"移动超过 3px 才拖"（收纳箱标题栏）：不用 `drag_region`，在 `on_mouse_move` 里调 `platform::begin_window_drag(window, cx)`
  （Windows：延后执行 `ReleaseCapture` + `WM_NCLBUTTONDOWN/HTCAPTION`）。

### 2.4 窗口外观、透明与 Root

- 所有窗口都经 `WindowManager::open_child`（或底层 `windows::open_window`）打开：它负责 `WindowSpec::options()`（无系统标题栏、
  尺寸 / 最小尺寸 / 显示器）、`Root` 包装、登记、模态关系与 owner。不要直接 `cx.open_window`——没有 `gpui_kit::base::Root` 时
  gpui-component 控件与 `UiLayer`（消息 / 确认框）不工作甚至 panic。
- 透明窗口（主窗口 / 通知 / 收纳箱）：`spec.transparent = true`，`open_window` 会把 Root 背景覆盖为 `transparent_black()`
  （Root 默认铺主题背景色）；圆角底色由视图自己画。其它窗口不透明白底。
- Xvfb 截图里透明像素是黑色（无合成器），对照时忽略圆角外的四个角；真实透明效果只能在 Windows 上看（e2e W01 / I05 取色）。

### 2.5 z 序靠绘制顺序；弹层画在窗口内

- GPUI **没有 z-index**：后绘制者在上（子元素顺序）。遮罩 `windows::modal_overlay(window, cx, alpha)` 必须是打开方 `render` 的
  **最后一个**子元素（打开方要 `cx.observe_global::<WindowManager>(|_, cx| cx.notify())`）。
- 浮层（下拉、日期 / 时间 / 颜色面板、菜单）统一走 `ui::ep_popper::popper`（`deferred(anchored())`，画在本帧最后）：触发元素里放
  `anchor.measure()`，打开时在 render 末尾追加 `popper(..)`。消息与确认框由 `UiLayer` 插件画在 Root 之上，窗口视图不用管。
- **弹层只能画在本窗口内**（Windows 后端不支持 `WindowKind::AnchoredPopup`，本应用也不为弹层开系统窗口）：放不下时 `popper` 翻转、
  水平夹回窗口；在小窗口（主窗口 380×600、收纳箱、通知 320×120）里设计弹层时先确认放得下（`max_height`、换方向）。
- 圆角容器**裁不掉**子元素的背景（GPUI 内容遮罩只有矩形）：贴边的子元素自己带对应圆角。
- GPUI 的外阴影会透过半透明背景显示（CSS 不会）：带阴影的浅色块用 `theme::editor_theme::tint(color, alpha)` 的等效不透明色。

### 2.5.1 外部文件拖入：用 `ui::external_drop`，不要用 `on_drop::<ExternalPaths>`

- gpui-pre 0.3.9 把系统文件拖放转成鼠标事件（`Entered` / `Pending` → `MouseMove`，`Submit` → `MouseUp`），但输入模态按转换前的
  `FileDrop` 判断、不会变成"鼠标"；`on_drop` 只在 `hitbox.is_hovered` 时触发，而 `is_hovered` 在键盘模态下恒为 false。拖放期间鼠标被
  拖放源捕获，目标窗口收不到真实的鼠标移动——于是窗口里最后一次输入是键盘时（收纳箱改名回车后、Markdown 编辑器里打完字）
  **第一次拖入的松手被静默丢掉**，`drag_over` / 悬停状态残留（e2e F10；Linux XDND 同样复现）。
- 放置区：视图构造时 `ExternalDrop::new()`，render 里 `drop.attach(root, DropScope::Window | Element, on_drop, on_exit)`（`on_drag_move`
  记路径 + 窗口级 `MouseUp` / `FileDropEvent::Exited` 监听，命中检测用不看模态的 `should_handle_scroll`）。悬停反馈照常用
  `on_drag_move::<ExternalPaths>` / `drag_over`（它们不看模态）。

### 2.6 图片与图标

- 本地图片用 `img(PathBuf)` / `&Path`。`img("C:\\a.png")` 这种 `&str` 会被当成 URI（`C:` 被解析成 scheme）。
- 图片进 App 级资源缓存，文件变了不会自动刷新：缓存键里带上会变的东西（收纳箱图标键 = 路径 + 修改时间 + 档位），或 `remove_asset`。
- Markdown 图片：存储形式 `minitodo-image://<name>`，渲染经 `util::image_ref::resolve_image_path` 映射到本机 images 目录
  （`ui::markdown` 已处理，基于 `gpui_kit::base::TextView::markdown(..).image_source(..)`）；文件名经 `is_safe_image_name` 校验。
  段落顶层的图片（独占段落、与文字同段）由 `ui::markdown` 自己排（原始尺寸、`max-width: 100%`、与文字垂直居中，同旧版
  `inline-block` + `vertical-align: middle`）：TextView 自己画的行内图片没有宽高属性时只有 0.75 行高、也不受容器宽度约束，不要交给它。
- SVG 是单色蒙版，**必须设颜色才会绘制**：一律 `ui::icon::icon(EpIcon::X, size, color)`。新图标：SVG 放进
  `assets/icons/{ep,app}/`（EP 图标文件名与 `@element-plus/icons-vue` 的 kebab-case 一致），在 `assets.rs` 表里加一行。禁止 emoji。
- 链接点击走 `util::file_link::handle_link_click`（协议白名单）；`cx.open_url` 本身不做任何校验。直接用 gpui-base `TextView` 渲染
  用户内容时必须接管 `on_link_click`，否则它默认直接 `open_url`（`ui::markdown` 已处理，新的渲染入口不要绕过它）。

### 2.7 字体与 theme token 是唯一来源

- 颜色 / 尺寸 / 圆角 / 间距 / 阴影 / 时长**只从 `theme.rs` 取**；新增 token 加到对应窗口的分节（主窗口调色板 `light()` /
  `Palette::for_main`、Element Plus `ep()`、`settings_tokens()`、`fence_tokens()`、`editor_theme::*`），数值照抄 UI 规格。
  界面代码里不写字面量颜色。
- 两套调色板：应用自绘部分 `#3B82F6` 系（`light()` / `Palette`），Element Plus 控件 `#409EFF` 系（`ep()`）；以 UI 规格标注为准。
  深色主题只作用于主窗口（`AppState::main_palette()`）。
- 每个窗口根元素：`.font(theme::ui_font())`、`.text_size(theme::FONT_SIZE)`、`.line_height(relative(theme::LINE_HEIGHT))`。
  `ui_font()`：Windows `Segoe UI` + `Microsoft YaHei` 回退；macOS `.SystemUIFont` + `PingFang SC`（别名只在主字体族上生效，见 §2.15）；
  Linux `Liberation Sans` + `Noto Sans CJK SC`（与参考截图一致）。
  待办标题等用户字体用 `theme::todo_font(&state.todo_font_family)`。CSS `font-weight: 600` → `theme::WEIGHT_SEMIBOLD`，`500` → Regular。
- gpui-component 的主题在 `theme::init` 里已调成 Element Plus（primary `#409EFF`、前景 `#606266`、边框 `#DCDFE6`、圆角 6 / 4、
  无焦点外环、字号 14）；不要用 gpui-component 默认外观去"近似"。
- 文案逐字照抄规格（包括标点与空格）。

### 2.8 控件复用（`ui/`）

- 每种 Element Plus 控件**只有一个实现**：`EpButton`、`EpInput`、`ep_popper`、`EpSelect`、`EpDropdownMenu`、`EpSwitch`、`EpCheckbox`、
  `EpColorPicker`、`EpRadioButtons`、`EpSlider`、`EpInputNumber`、`EpDatePicker` / `EpTimePicker`、`markdown*`、`tooltip`、
  `message`、`message_box`、`WindowTitleBar`、`scrollbar`。缺能力就给现有控件加 builder 方法，不要在窗口里另写一份。
- 新控件：`ui/` 下单独文件，在 `ui/mod.rs` 的表里登记，并加进控件样例窗口（`windows/gallery.rs`，`MINI_TODO_DEBUG_OPEN=gallery`）。
- 有状态控件（`InputState`、`EpSelect`、`EpColorPicker`……）在**构造视图时**创建，不要在 `render` 里 `cx.new`。
- 失败提示：`ui::message::error_with(window, cx, "保存待办失败", &err)`（= 旧 `notifyError`，含 200 字截断）；确认框：
  `ui::message_box::confirm(..)`（回调在确认框关闭之后执行）。

### 2.9 跨窗口状态与刷新（替代 Pinia + Tauri 事件）

- 所有窗口同在一个进程、一个 `App`：跨窗口共享的状态放 `AppState`（`Entity` + Global，注明由谁维护），视图
  `cx.observe(&state, |_, _, cx| cx.notify())`；只属于某个窗口的状态放在该窗口的视图实体里。旧 Tauri 事件到新机制的映射见
  `info.md` §3.2（`sync-completed` → `apply_sync_report`，`data-imported` → 重新加载，`subtask-memory-*` → `Entity<DraftSubtasks>`……）。
- 拉列表只经 `AppState::fetch_todos` / `reload_todos` / `refresh_if_changed`：**单飞 + 过期丢弃**（拉取期间有本地写操作则丢弃并补拉）。
- 主窗口有模态子窗口时后台刷新跳过（`WindowManager::main_has_modal_child`）；子窗口关闭后 `WindowManager` 自动 `reload_todos`，
  设置窗口关闭后再重读设置并 `auto_sync::start`。新增后台刷新路径要遵守同样的规则。
- 新窗口种类：`WindowKind` + 窗口模块的 `open()`（走 `WindowManager::open_child`）；需要截图时在 `windows/mod.rs` 的
  `DebugOpen` / `parse_debug_open` / `open_debug` 加一项并更新 `pc-gpui/scripts/README.md`。

### 2.10 键盘

- 键绑定的 action 先于 `on_key_down` 派发。编辑类窗口的 Esc / Ctrl+Enter 是 action（`util::editor_shortcuts`，key context
  `MiniTodoEditor`）：弹层打开时由窗口自己的处理函数先收起弹层（`any_popup_open` / `close_popups`）。
- 多行编辑器的 Enter 插入换行且不冒泡：`MiniTodoEditor > Input` 上下文额外绑定了 Ctrl+Enter 抢在换行之前。
- `App::bind_keys` **不去重**：全局按键绑定只注册一次（`util::editor_shortcuts::init` 用 Global 标记守卫），不要在每次开窗时注册。

### 2.11 debug 专用开关只在 debug 构建生效

- 只在 `cfg(debug_assertions)` 下读取，release 一律忽略（Windows e2e 的 P01 在 release 套件里验证忽略 `MINI_TODO_DATA_DIR` /
  `MINI_TODO_DEBUG_OPEN`）：

| 变量 | 作用 |
|---|---|
| `MINI_TODO_DATA_DIR` | 数据目录（库 / 图片 / 备份 / 图标缓存）、日志 `<dir>/logs`；**不启用单实例、不刷新开机自启项**。与安装版并存实测时必须带上 |
| `MINI_TODO_FENCES_ROOT` | 收纳根目录（代替 `%USERPROFILE%\MiniTodo收纳箱`；macOS `~/MiniTodo收纳箱`） |
| `MINI_TODO_SCREEN_ID_FILE` | 文件内容非空时替代屏幕组合 ID |
| `MINI_TODO_DEBUG_OPEN` | 加载完数据后额外打开的窗口（截图 / e2e 用，格式见 `pc-gpui/scripts/README.md`）；这些窗口不走模态关系 |
| `MINI_TODO_DEBUG_LATEST_VERSION` | 版本检查不联网，直接当作最新版本 |
| `MINI_TODO_DEBUG_DIALOG_PATH` | 导出 / 导入的文件对话框不弹出，直接返回该路径 |
| `MINI_TODO_DEBUG_WINDOWS_UI` | `1` 时 Linux / macOS 上也显示 Windows 专有的设置项 |
| `MINI_TODO_DEBUG_FENCE_STATE` | `onedrive` / `drag-hover`：收纳箱直接显示云同步提示（OneDrive / macOS iCloud 文案）/ 拖入悬停框 |
| `MINI_TODO_DESKTOP_DIR` | 代替"用户桌面"（core `fence_fs::user_desktop_dir`：收纳箱移回的兜底目标等）；mac e2e 用它避开真实 `~/Desktop` 的隐私授权框 |

- 前三个与旧版同名同义（core `db::paths`、`services::fence_fs`、`settings::screen_config_id` 读取）。新增 debug 开关同样只在
  `cfg(debug_assertions)` 下读取，并写进 app README §5 与 `pc-gpui/scripts/README.md`。
- 开机自启注册表项（`HKCU\…\Run\mini-todo`；mac 为 `~/Library/LaunchAgents/mini-todo.plist`）与单实例互斥量（mac 为
  `/tmp/com_tauri_app_mini_todo_si.sock`）和旧版 / 安装版共用：隔离运行时绝不刷新 / 切换它们。

### 2.12 平台差异

- 目标平台 Windows 10/11 与 macOS 11+（预览，§2.15）。Linux 只用于 CI 与截图 / e2e：没有托盘、单实例、开机自启、嵌入桌面、收纳箱真实文件操作；
  `platform/fallback.rs` 为 no-op 或用 GPUI 能力近似（Linux 上无法程序化移动窗口）。
- **`cfg(not(windows))` 不等于 Linux**：mac 上它同样成立。只给 Linux 的行为写 `cfg(target_os = "linux")`（如"最后一个窗口关闭即退出"），
  Windows + mac 共有的写 `cfg(any(windows, target_os = "macos"))`（托盘、单实例），Linux 的空实现写 `cfg(not(any(windows, target_os = "macos")))`。
- `QuitMode::Explicit`：Windows 上关掉所有窗口进程不退出（托盘常驻）；macOS 同样不退出（菜单栏图标常驻，Dock / 菜单栏图标再唤起，⌘Q 退出）；
  Linux 没有托盘，最后一个窗口关闭时退出（`lib.rs` 的 `cfg(target_os = "linux")`）。
- Windows 专有设置项按 `cfg!(windows)` 显示（`MINI_TODO_DEBUG_WINDOWS_UI=1` 可在 Linux / mac 上显示以对照参考截图）；mac 专有提示按
  `cfg!(target_os = "macos")`（如设置页"WebDAV 密码以明文保存"）。
- 所有 `cfg(windows)` 代码必须通过 `x86_64-pc-windows-gnu` 交叉 clippy（见 [verification.md](./verification.md) §3）；mac 代码见
  [verification.md](./verification.md) §7（`mac-check` 交叉检查 + CI 的 macOS 作业）。

### 2.13 文本排版与已知视觉偏差

- **中文避头尾**：GPUI 的换行器（`gpui-pre/src/text_system/line_wrapper.rs`）在任意 CJK 字符间断行，但不做避头尾——全角标点
  （`，。、；：！？）」`）可能落到行首，折行位置与浏览器可能差一个字。不要为了逐像素一致去改换行器；确有需要时：
  CSS `word-break: break-all` 用 `windows/fence/grid.rs` 的 `break_all`（字符间插零宽空格），按空格折行用
  `main_window/todo_item.rs` 的 `wrapping_words`。
- GPUI 没有文字阴影（收纳箱文件名用错位重绘近似）、线性渐变只有两个色标（取色器色相条分段拼接）、位图不支持变换
  （图片预览旋转在后台用 `image` crate 处理）、自带删除线对中文落在基线附近（`todo_item::strike` 自绘）。
- 滚轮增量方向与 DOM `deltaY` 相反（向上为正）。
- Markdown 编辑是"源码 + 预览"（旧版 Milkdown 所见即所得无法 1:1 还原，PRD D7）；只读渲染、图片、链接白名单与旧版一致。
  与文字同段的图片拆成"文字段 / 图片 / 文字段"排在同一行（放不下换行），文字段是一整块：长到一行放不下时独占一行，
  不像旧版那样接着图片所在行的剩余宽度排。

### 2.14 布局易错点（taffy）

- 百分比高度 / 最小高度（`h_full`、`min_h_full`）只在父元素高度**确定**时生效；父元素只有 `min_h` / `max_h`（如描述框）时按 auto 处理，
  元素缩成内容高度。要"至少撑满父元素、内容更多时随内容增高"：父元素 `flex().flex_col()`，子元素 `flex_grow(1.).flex_shrink_0()`
  （`ui::markdown_editor` 的根元素；缺陷 D1：空描述框只有顶部 10px 能点进编辑）。
- `overflow_y_scroll` 的可滚动范围只按**直接子元素**的边界计算（gpui `Div::prepaint`）：滚动容器的直接子元素不要 `size_full`
  （高度被钉成容器高度，孙元素溢出也滚不动），用 `min_h_full`（子任务内容框）。

### 2.15 macOS（AppKit；gpui-pre-macos 0.3.9）

gpui-pre-macos 能建窗（位置 / 显示器 / 置顶层级 / 可缩放都只在建窗时给）、拖动（`start_window_move`）、缩放（`zoom_window`）、激活，
但**没有**运行时摆窗、改层级、显示 / 隐藏单个窗口、改 style mask；这些在 `platform/macos/ns.rs` 里用 objc2 直接调 NSWindow / NSScreen / NSEvent。
维护者没有 Mac，下面每条都是"不遵守就出错"且只能靠 CI 的 macOS runner 发现的：

- **AppKit 调用只在主线程**：AppKit 对象都是 `MainThreadOnly`；`ns.rs` 每个函数先 `MainThreadMarker::new()`，不在主线程就返回 `None` / `false`
  并只警告一次。GPUI 在 mac 上的前台（事件处理、`cx.spawn` 的前台任务）就是主线程，后台线程（`background_executor`、`spawn_db`、
  `std::thread`）里不要碰 `platform::*`。同理：`UNUserNotificationCenter`（`cx.show_system_notification`）只能在主线程，调度器经通道交给前台
  （`services/scheduler.rs`）；tray-icon / muda 必须在主线程、run loop 已运行后创建——`services::tray::start` 放在 `Application::run` 的回调里
  （GPUI 在 `applicationDidFinishLaunching:` 里执行它）。tray-icon 的菜单 / 点击回调也在主线程，但照样只投 channel，不碰 `App`。
- **`Hwnd` 在 mac 上是 NSWindow 的窗口号（`windowNumber`），不是对象指针**：每次调用都在 `NSApp.windows` 里按窗口号重新找，找不到就当窗口已关、
  什么都不做。不要保存、解引用 NSWindow / NSView 指针：GPUI 关窗后对象可能已释放，地址还可能被新窗口复用。`platform::hwnd(window)` 只在
  持有 `&Window` 的那一刻从 `raw_window_handle` 的 `ns_view` 取窗口号。
- **写操作同样要延后**：`setFrame:` / `setFrameOrigin:` / `orderOut:` / 改 style mask 会同步触发 GPUI 的 `windowDidMove:` / `windowDidResize:`
  等回调，App 被借用时 GPUI 处理不了——与 Win32 一样遵守 §2.2（`&Window` 版本已自动延后；`Hwnd` 原始调用只在 `cx.spawn` 里、`update` 之外）。
  菜单动作在活动窗口的 `update` 里派发（窗口正被借用），碰窗口的处理一律 `cx.defer` 到派发之后（`services/mac_menu.rs` 的 `with_active_window`）。
  读操作（frame、`NSScreen.screens`、`NSEvent.mouseLocation`）与 `ns::prepare_new_window` 不触发回调，随时可调。
- **坐标**（`platform/mac_geometry.rs` 模块文档）：AppKit 是全局点、主屏左下原点、y 向上；GPUI 的逻辑像素 = 点，但窗口 `bounds()` 与建窗的
  `window_bounds` 都是**所在显示器本地**的（以该屏左上角为原点）；`cx.displays()` 的 `bounds()` 原点恒为 (0, 0)。因此：
  - `WindowSpec.bounds` 一律写全局逻辑坐标，由 `windows::open_window` 经 `platform::creation_bounds` 换成显示器本地坐标并补上目标显示器；
    不要绕过 `open_window` 直接 `cx.open_window`，也不要用 `cx.displays()` 排多屏布局（用 `platform::monitors`）
  - 本 crate 的"物理像素" = 全局点坐标（左上原点）× **统一系数**（所有屏 `backingScaleFactor` 的最大值）；逻辑 → 物理用
    `platform::physical_scale(window)`，**不要**用 `window.scale_factor()`（那是窗口所在屏自己的缩放，混合缩放时与统一系数不同）。
    `MonitorInfo.scale` 在 mac 上恒为统一系数
  - 屏幕组合 ID 例外：按各屏自己的缩放算（`platform::screen_config_id` → `mac_geometry::screen_config_inputs`），与 2.5.0 一致
  - 窗口 frame = 客户区（`FullSizeContentView`），没有 Windows 那样的不可见边框；换算逻辑放 `mac_geometry.rs`（全平台编译，Linux 上跑单测）
- **红绿灯**：所有窗口都带 `TitlebarOptions { appears_transparent: true }`，AppKit 会在左上角画系统的关闭 / 最小化 / 缩放按钮，压在自绘标题栏上。
  `windows::open_window` 建窗后**立即**（不延后，不触发回调）`Hwnd::prepare_after_open` 隐藏它们，并关掉 NSPanel（`PopUp` / `Floating` 窗口）的
  `hidesOnDeactivate`、对本该显示的窗口补一次显示。**改 style mask 后 AppKit 可能重建标题栏、红绿灯重新出现，还会换掉第一响应者**（键盘输入
  不再进 GPUI 视图）：所有改 mask 的代码都走 `ns.rs` 的 `update_style`（改完重新隐藏按钮、把第一响应者还回去），不要直接 `setStyleMask`。
- **`app_owns_titlebar_drag: true`**（`WindowSpec::options()`，只对 mac 生效）不要去掉：否则 AppKit 从顶部那条标题栏区域拖窗、为区分双击而
  延迟标题栏上的点击。拖窗走 `start_window_move`（`ui::title_bar::drag_region` 的非 Windows 分支，必须在鼠标按下 / 移动的事件处理里同步调用）。
- **激活**：点菜单栏图标 / 菜单项、单实例转交、Dock 唤起时本应用多半不在前台，新开或唤起的窗口会落在前台应用的窗口之后——先 `cx.activate(true)`
  （`tray::activate_app`、`window_mode::bring_main_to_front`、单实例接收端已处理）。`on_reopen` 只在没有可见窗口时回调：移到屏幕边缘（贴边隐藏）
  的窗口仍算可见，这类状态要在窗口自己的激活回调里处理（主窗口的 `window_mode::on_main_activated`）。
- **AppKit 会约束窗口位置**：带标题栏样式的窗口顶边不能高过菜单栏，`setFrameOrigin:` 可能没到目标位置。依赖"窗口真的到了那里"的逻辑
  （贴边隐藏）要读回实际 frame 判断（mac 的 `Hwnd::set_client_position` 已据实返回成败），不要假设摆窗必然成功。
- **应用菜单与退出**：GPUI 不会自动建主菜单，没有 `set_menus` 就没有 ⌘Q，系统文件面板里的 ⌘C / ⌘V 也不工作（AppKit 靠菜单项的 key equivalent
  派发）。`services::mac_menu::init` 在 `theme::init` 之后调用一次，**快捷键要先于 `set_menus` 绑定**（菜单项才显示快捷键）；编辑菜单的动作就是
  gpui-base 输入框的 `input::*`，快捷键沿用输入框自己的绑定，不重复绑定。关窗口不退出（`QuitMode::Explicit`），`on_reopen` 只在没有可见窗口时回调。
- **激活策略**：GPUI 在 `applicationDidFinishLaunching:` 里按自身状态重设激活策略（默认 Regular），Info.plist 的 `LSUIElement` 会被覆盖；
  要隐藏 Dock 图标只能用 `cx.set_activation_policy(Accessory)`（或建 `Application` 时 `with_windowing(Headless)`），目前不做。
- **字体**：GPUI 只在**主字体族**上把 `.SystemUIFont` 换成系统界面字体 `.AppleSystemUIFont`；回退列表原样交给 CoreText，写 `.SystemUIFont`
  等于没写——回退项要写真实族名 `.AppleSystemUIFont`（`theme::font_family_as_fallback`，`todo_font` 用它）。字体下拉用
  `cx.text_system().all_font_names()`，去掉 `.` 开头的私有字体。Linux 的替身字体（Liberation / Noto）mac 上没有，新增字体 token 时三个平台都要给值。
- **文件对话框**用 GPUI 的 `prompt_for_paths` / `prompt_for_new_path`（原生面板、非模态，没有过滤器与标题）。不要在 mac 上用 rfd 的同步接口：
  它把面板同步派到主队列 `runModal`，对话框期间 GPUI 的前台任务全部停摆。要过滤器时用 rfd 的 async + `set_parent`（sheet）。
- **系统通知**只在从 `.app` 启动时可用（GPUI 没有 bundle id 时只记一条日志，避免系统 abort），且 bundle 至少要 ad-hoc 签名；拿不到投递结果。
  e2e / 本地调试要先 `scripts/macos/bundle.sh <二进制> - <目录>` 包成 `.app`。
- **代码分层**（这样 Linux / Windows 上也能检查大部分 mac 代码）：
  - objc2 的 AppKit 调用写进**不依赖 GPUI 与本 crate 其它模块**的文件（`platform/macos/ns.rs`；同类的还有 `services/autostart.rs`、
    `services/single_instance/unix_socket.rs`、`services/tray/mac_icon.rs`，以及 core 的收纳箱文件层 `services/fence_fs/mac.rs`——core 整体依赖
    bundled SQLite，只能在 Mac 上编译，所以 `mac.rs` 必须是不 `use super::` / `crate::` 的叶子模块，同一份源码按 edition 2021 / 2024 都能编译），
    并在 `scripts/mac-check/src/lib.rs` 里加一行 `#[path]` 模块；改 app / core 的 mac 依赖版本 / 特性时同步改 `scripts/mac-check/Cargo.toml`
    （特性 = 两者的并集，逐项一致）
  - 纯换算 / 纯逻辑写成全平台编译带单测（`mac_geometry.rs`；`mac_menu.rs` 全平台编译、只在 mac 上调用，Linux clippy 能检查它对 GPUI API 的用法），
    用 `#[cfg_attr(not(target_os = "macos"), allow(dead_code))]` 压住非 mac 的未使用警告
  - `platform/macos/mod.rs` 等 GPUI 胶水尽量薄——它只有 CI 的 macOS 作业能编译（[verification.md](./verification.md) §7）
- **测试里的临时目录**：mac 的临时目录 `/var/folders/…` 是 `/private/var/folders/…` 的符号链接，SQLite（库路径）与 FSEvents（`notify`）报告的是
  解析后的路径。比较路径前两边都 `canonicalize`（`db/backup.rs` 的测试），或只在 mac 上 `canonicalize`（Windows 上 `canonicalize` 会变成 `\\?\` 路径，
  反而对不上）。收纳箱的生产代码也要处理（访达拖放给的同样是解析后的路径）：mac 上 `fence_fs::fence_root_dir`、`watch_root_at` 与
  `MINI_TODO_DESKTOP_DIR` 的覆盖目录都先 `canonicalize`（真实 `~/Desktop` 不碰，它受隐私保护）。
- **收纳箱窗口（桌面层的不激活面板）**：mac 上收纳箱建成 `WindowKind::PopUp`（NSPanel + `NonactivatingPanel`，只能建窗时给）、`show: false`，
  建窗后在 `cx.spawn` 里 `Hwnd::make_desktop_layer`（层级 `CGWindowLevelForKey(kCGDesktopIconWindowLevelKey) + 1`、collectionBehavior
  **整体替换**成 `CanJoinAllSpaces | Stationary | IgnoresCycle | FullScreenNone`、`canHide = NO` 等）再 `orderFrontRegardless`。GPUI 建窗后不再改层级，
  本 crate 里会改的只有 `ns::set_floating`（`set_topmost`）——**收纳箱窗口绝不能调它**，也不调 `setFloatingPanel:` / `bring_to_front` /
  `cx.activate(true)`。非激活面板单击不一定成为 key 窗口：要键盘（改名）时先 `window.activate_window()`。菜单动作（⌘M / ⌘W）对收纳箱跳过。
  契约见 [backend/desktop-fences.md](../backend/desktop-fences.md)「场景：macOS 上的收纳箱」。
- **系统取色面板**（`fence_win::choose_color` 的 mac 实现，`ns::show_color_panel` / `color_panel_state`）：`NSColorPanel` 是 `MainThreadOnly` 的共享
  非模态面板，只能在前台任务里碰；要 `setHidesOnDeactivate(false)`（NSPanel 默认失活就藏，会被轮询误判成"关掉了"）；读分量前先
  `colorUsingColorSpace(sRGB)` 并确认 RGB 模型——对非 RGB 颜色读 `redComponent` 抛 ObjC 异常，穿过 Rust 帧直接 abort。
  显示它会激活应用并让当前 key 窗口失去 key（触发 GPUI 回调），所以放在任务里、不在事件处理（App 被借用）里直接做。
- Windows 专有的 `platform::*` 在 mac 上是 no-op：`set_tool_window`（程序坞 / ⌘Tab 是整个应用的）、`set_owner`（mac 子窗口会跟着父窗口移动，语义不同；
  模态只靠遮罩与 `forward_activation`）、`set_round_corners`、`set_system_menu`、`embed::*`（mac 收纳箱走 `Hwnd::make_desktop_layer`）；`ex_style()` 恒 0
  （`window_mode` 的 tool window 自愈只在 Windows 上做）。反过来 `Hwnd::make_desktop_layer` / `describe_layer` 在 Windows / Linux 上是 no-op。

---

## 3. `gpui-kit` 精确钉版本与升级

- `crates/app/Cargo.toml`：`gpui-kit = "=0.7.3"`（**精确**版本）。gpui-pre 是 Zed 的每周快照，次版本之间就有不兼容的 API 变化；
  不要改成 `^`，也不要单独升级 gpui-pre / gpui-component（由 gpui-kit 统一 re-export）。
- `windows` crate：app 用 0.62（与 gpui-pre-windows 同一版本，避免再编一份）；core 用 0.58（沿用旧 Tauri 版代码时的版本，与 app 独立；
  要统一须单独立项，并跑 core 的 Windows 专属测试与 Windows e2e）。
  app 缺 feature 时追加到列表末尾。
- `Cargo.lock` 提交；CI 一律 `--locked`。

升级步骤（每一步都要做，任何一项行为变了就改实现并更新本文件与 app README）：

1. 读新版本源码，逐项核对本项目依赖的行为：`start_window_move` 在 Windows 上是否仍为空操作；`GWLP_USERDATA` 占用；是否开始处理
   `WM_WINDOWPOSCHANGING` / 改写 ex style；`WindowControlArea::Drag` → `HTCAPTION`；缩放热区是否仍按建窗时的 `is_resizable`；
   `WindowKind::PopUp` 的样式映射；`calculate_window_rect` 的边框对半分（客户区偏高）与 `WM_NCCALCSIZE` 只去顶边是否还在
   （`windows::open_window` 的客户区校正、`platform::FrameInsets` 的单测依赖它）；建窗样式是否仍带 `WS_SYSMENU`、运行期是否开始
   改写 `GWL_STYLE`（`open_window` 建窗后去掉 `WS_SYSMENU` 依赖它）；文件拖放是否仍不改输入模态、`on_drop` 是否仍要求
   `is_hovered`（`ui::external_drop` 依赖它）；透明窗口（DirectComposition）实现；`QuitMode` 默认值；`Root` / `RootPlugin` 接口
   （`UiLayer` 依赖）；`gpui_kit::base::TextView::markdown(..).image_source(..)`；`img(&str)` 的 URI 判定；换行器规则；
   Windows release 构建的 `fxc.exe` 着色器预编译（`gpui-pre-windows/build.rs`）。
   macOS（§2.15，gpui-pre-macos / gpui-pre-apple）：gpui-kit 是否仍给 `gpui_platform` 开 `font-kit`（漏开时 mac 上退回 `NoopTextSystem`，
   界面一个字都不画——mini-term issue #82）与 `runtime_shaders`（关了就要构建期 `xcrun metal`）——用
   `cargo tree --locked --offline --target aarch64-apple-darwin -e features -i gpui-pre-apple` 核对；窗口 `bounds()` / `window_bounds` 是否仍是
   显示器本地坐标、`displays()` 原点是否仍恒为 0（`creation_bounds`、`mac_geometry` 依赖它）；`TitlebarOptions` 的 style mask 与红绿灯、
   NSPanel 的 `hidesOnDeactivate`；`app_owns_titlebar_drag`；`.SystemUIFont` 别名只认主字体族；`show_system_notification` 的 bundle 守卫与线程要求；
   `did_finish_launching` 是否仍重设激活策略、`on_reopen` 的回调条件；`set_menus` / `MenuItem::os_action`；`setReleasedWhenClosed:NO`。
   收纳箱（desktop-fences.md「场景：macOS 上的收纳箱」）：`PopUp` 是否仍是带 `NonactivatingPanel` 的 NSPanel、层级 / collectionBehavior 是否仍只在
   `MacWindow::open` 里设（运行期不重设）、`PopUp` 是否仍有 `ActiveAlways` 跟踪区；所有窗口是否仍 `registerForDraggedTypes:` 文件类型、外部拖入回报的
   拖放操作；`start_window_move` 是否仍是 `performWindowDragWithEvent:`；`acceptsFirstMouse` / `canBecomeKeyWindow`。
2. 对齐 app 的 `windows` crate 版本与 gpui-pre-windows 一致；mac 的 `objc2` / `objc2-foundation` / `objc2-app-kit` 与 gpui-pre-macos 一致
   （同步改 `scripts/mac-check/Cargo.toml`）。
3. 若开启 `gpui-kit/test-support`：旧 lock 会出现 `cocoa` / `unicode-properties` 版本冲突，删掉 lock 重新生成（见 gpui-api.md §C1）。
4. 跑 [verification.md](./verification.md) 的**全部**门槛：Linux fmt / clippy / test、Windows 交叉 clippy、`mac-check`、`regress.py` 全部场景
   （`--before` 指向升级前的输出，逐张对照）与 `--smoke`、Linux e2e、Windows e2e（debug + release 两个 job）、macOS e2e（`checks` / `smoke` /
   `release-smoke`）与 `pc-gpui-ci.yml` 的 macos 作业（universal 构建）。
5. 更新 `research/gpui-api.md` 的结论（或在本文件记录差异）、app README 的版本号与约定。

---

## 4. Wrong vs Correct

### Wrong — 在事件处理里同步调用阻塞的 core 函数

```rust
.on_click(cx.listener(|this, _, _, cx| {
    // 卡住所有窗口（SQLite 加锁 / 磁盘 IO 都在 GPUI 主线程上）
    this.todos = mini_todo_core::todo::list(&Db::get(cx)).unwrap_or_default();
    cx.notify();
}))
```

### Correct — 后台执行，`WeakEntity` 回前台

```rust
fn load(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    let task = spawn_db(cx, mini_todo_core::todo::list);
    cx.spawn_in(window, async move |this: WeakEntity<Self>, cx| {
        let result = task.await;
        let _ = this.update_in(cx, |this, window, cx| match result {   // 窗口已关闭时 Err，忽略
            Ok(todos) => { this.todos = todos; cx.notify(); }
            Err(e) => message::error_with(window, cx, "加载已完成待办失败", &e),
        });
    })
    .detach();
}
```

### Wrong — App 借用期间直接动窗口

```rust
.on_click(cx.listener(|_, _, window, _| {
    if let Some(h) = platform::hwnd(window) {
        h.set_rect(rect);   // 立即 SetWindowPos：GPUI 收到 WM_SIZE 时 App 正被借用，几何变化被丢弃
    }
}))
```

### Correct — 用接收 `&Window` 的版本（自动延后），或在 `cx.spawn` 里、`update` 闭包之外调用原始方法

```rust
.on_click(cx.listener(|_, _, window, cx| {
    platform::set_client_rect_physical(window, cx, rect);   // 延后到下一个前台任务（按客户区）
}))
```

### Wrong — 字面量颜色、本地图片传字符串

```rust
div().text_color(rgb(0x606266)).child(img(path.to_string_lossy().to_string()))
```

### Correct

```rust
div().text_color(theme::ep().text_regular).child(img(path.clone()))   // PathBuf
```
