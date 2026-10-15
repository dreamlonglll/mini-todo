# GPUI 约定（`pc-gpui/crates/app`）

> app crate 的可执行约定。来源：`pc-gpui/crates/app/README.md` §3「约定」与 §2.1「维护规则」、
> `.trellis/tasks/archive/2026-10/10-15-gpui-rewrite/research/gpui-api.md` §0「结论速览」，以及实现过程中踩过的坑。
> 版本：`gpui-kit =0.7.3`（re-export gpui-pre 0.3.9、gpui-pre-platform、gpui-base 0.7.3、gpui-component 0.7.3）。
> API 以本机 `~/.cargo/registry/src/index.crates.io-*/gpui-pre-0.3.9`、`gpui-pre-windows-0.3.9`、`gpui-component-0.7.3`
> 等源码为准——gpui-pre 是 Zed 的每周快照，网上的示例、文档常常对不上这个版本。
> 接口清单（`AppState` 方法、`WindowManager`、`ui::*` 控件、`platform::*`、`services::*`）见 app README §4，这里不重复。

---

## 1. Scope / Trigger

- 任何 `pc-gpui/crates/app/**` 的改动。
- 尤其：新窗口种类、新控件、动到 `platform/` / `services/window_mode.rs` / `services/fences.rs`、改 `theme.rs`、
  新 debug 开关、升级 `gpui-kit`。

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
  HWND 在主线程取好（`isize`），移动派到后台。

### 2.2 App 借用期间禁止同步 Win32 调用（`platform` 延后）

- `SetWindowPos` / `ShowWindow` / 改 style / `SendMessage` 会**同步**进入 GPUI 的窗口过程（`WM_MOVE` / `WM_SIZE` / `WM_ACTIVATE`…）。
  如果此刻 App 正被借用（事件处理、`update` 闭包、`render`），GPUI 拿不到 App，只能丢掉这次几何变化 → 窗口尺寸 / 位置状态错乱，
  模态移动循环里甚至 panic。
- `platform::*(window, cx, …)` 形式（接收 `&Window`）的**写操作已自动延后**到下一个前台任务，可以在事件处理里直接调用。
- `platform::Hwnd` 上的方法、`platform::embed::*`、`subclass::set_flag` 是**立即执行**的原始调用：只能在 `cx.spawn` 的 async 块里、
  `update` 闭包**之外**调用（200ms 守护轮询、窗口模式切换都是这么写的）。
- 读操作（`GetWindowRect`、显示器枚举、光标位置）不发消息，随时可调。
- 任何**模态**系统调用（同步拖窗循环、同步文件 / 取色对话框）都不能在事件处理里直接调：`platform::file_dialog` / `fence_win::choose_color`
  已放独立线程，`begin_window_drag` 已延后。
- 窗口代码里**不直接写 Win32**：新能力加在 `platform/`（`windows.rs` 实现 + `fallback.rs` 的 no-op / 近似 + `mod.rs` 跨平台签名）。
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
  `ui_font()`：Windows `Segoe UI` + `Microsoft YaHei` 回退；Linux `Liberation Sans` + `Noto Sans CJK SC`（与参考截图一致）。
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
| `MINI_TODO_FENCES_ROOT` | 收纳根目录（代替 `%USERPROFILE%\MiniTodo收纳箱`） |
| `MINI_TODO_SCREEN_ID_FILE` | 文件内容非空时替代屏幕组合 ID |
| `MINI_TODO_DEBUG_OPEN` | 加载完数据后额外打开的窗口（截图 / e2e 用，格式见 `pc-gpui/scripts/README.md`）；这些窗口不走模态关系 |
| `MINI_TODO_DEBUG_LATEST_VERSION` | 版本检查不联网，直接当作最新版本 |
| `MINI_TODO_DEBUG_DIALOG_PATH` | 导出 / 导入的文件对话框不弹出，直接返回该路径 |
| `MINI_TODO_DEBUG_WINDOWS_UI` | `1` 时 Linux 上也显示 Windows 专有的设置项 |
| `MINI_TODO_DEBUG_FENCE_STATE` | `onedrive` / `drag-hover`：收纳箱直接显示 OneDrive 提示 / 拖入悬停框 |

- 前三个与旧版同名同义（core `db::paths`、`services::fence_fs`、`settings::screen_config_id` 读取）。新增 debug 开关同样只在
  `cfg(debug_assertions)` 下读取，并写进 app README §5 与 `pc-gpui/scripts/README.md`。
- 开机自启注册表项（`HKCU\…\Run\mini-todo`）与单实例互斥量和旧版 / 安装版共用：隔离运行时绝不刷新 / 切换它们。

### 2.12 平台差异

- 目标平台 Windows 10/11。Linux 只用于 CI 与截图 / e2e：没有托盘、单实例、开机自启、嵌入桌面、收纳箱真实文件操作；
  `platform/fallback.rs` 为 no-op 或用 GPUI 能力近似（Linux 上无法程序化移动窗口）。
- `QuitMode::Explicit`：Windows 上关掉所有窗口进程不退出（托盘常驻）；Linux 没有托盘，最后一个窗口关闭时退出。
- Windows 专有设置项按 `cfg!(windows)` 显示（`MINI_TODO_DEBUG_WINDOWS_UI=1` 可在 Linux 上显示以对照参考截图）。
- 所有 `cfg(windows)` 代码必须通过 `x86_64-pc-windows-gnu` 交叉 clippy（见 [verification.md](./verification.md) §3）。

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
   （`windows::open_window` 的客户区校正、`platform::FrameInsets` 的单测依赖它）；文件拖放是否仍不改输入模态、`on_drop` 是否仍要求
   `is_hovered`（`ui::external_drop` 依赖它）；透明窗口（DirectComposition）实现；`QuitMode` 默认值；`Root` / `RootPlugin` 接口
   （`UiLayer` 依赖）；`gpui_kit::base::TextView::markdown(..).image_source(..)`；`img(&str)` 的 URI 判定；换行器规则；
   Windows release 构建的 `fxc.exe` 着色器预编译（`gpui-pre-windows/build.rs`）。
2. 对齐 app 的 `windows` crate 版本与 gpui-pre-windows 一致。
3. 若开启 `gpui-kit/test-support`：旧 lock 会出现 `cocoa` / `unicode-properties` 版本冲突，删掉 lock 重新生成（见 gpui-api.md §C1）。
4. 跑 [verification.md](./verification.md) 的**全部**门槛：Linux fmt / clippy / test、Windows 交叉 clippy、`regress.py` 全部场景
   （`--before` 指向升级前的输出，逐张对照）与 `--smoke`、Linux e2e、Windows e2e（debug + release 两个 job）。
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
