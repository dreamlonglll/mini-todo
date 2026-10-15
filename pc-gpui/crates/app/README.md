# mini-todo（app crate）

GPUI 版 PC 端的界面、窗口与平台集成：package `mini-todo`，lib `mini_todo` + bin `mini-todo`（`main.rs` 只调用
`mini_todo::run()`；模块都是 `pub` 的，所以基础框架里暂时没人调用的接口不会被当成死代码）。
依赖 `gpui-kit =0.7.3`（gpui-pre 0.3.9 + gpui-component 0.7.3）与 `mini-todo-core`（业务逻辑，见 `../core/README.md`）。

本文件是**给并行实现者（W1 主窗口 / W2 编辑类窗口 / W3 设置与收纳箱）的接口说明**。设计依据：
`.trellis/tasks/10-15-gpui-rewrite/info.md` §3-4；UI 规格：`research/ui-main-window.md`、`ui-editor-windows.md`、
`ui-settings-fence.md`；GPUI API：`research/gpui-api.md`（API 以 `~/.cargo/registry/src/index.crates.io-*/` 源码为准）。

## 1. 构建与验证（在 `pc-gpui/` 下）

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
# Windows 目标交叉检查（覆盖全部 cfg(windows) 代码；只做 check / clippy，不要交叉链接或 release）
cargo clippy --workspace --all-targets --target x86_64-pc-windows-gnu -- -D warnings
# 运行（隔离数据目录，不碰真实数据）
MINI_TODO_DATA_DIR=/tmp/mt cargo run -p mini-todo
# Xvfb 截图（见 ../../scripts/README.md）
scripts/shot.sh --name main-list --window "Mini Todo" /tmp/shots
scripts/shot.sh --open main,gallery --window 控件样例 /tmp/shots   # ui/ 控件样例
```

参考截图：`/tmp/claude-0/-home-user-mini-todo/9b3d7942-6fc3-57a4-8b18-2becb0556be0/scratchpad/ref-ui/screens/*.png`。

## 2. 模块与负责人

```text
src/
├── lib.rs            run()：日志 → 单实例 → 数据库 → Application(QuitMode::Explicit) → init → 主题 → UiLayer →
│                     Db / WindowManager / AppState → 单实例唤起监听 → 主窗口 → 启动钩子 → 加载 → DEBUG_OPEN
├── main.rs           bin 入口（release 为 Windows 子系统）
├── assets.rs         AppAssets（EpIcon / AppIcon 嵌入表 + gpui-kit 默认资源）
├── theme.rs          颜色 / 字体 / 圆角 / 间距 / 阴影 / 时长 token；gpui-component 主题调成 Element Plus 外观
├── db.rs             Db 全局（Arc<Database>）、spawn_db
├── state.rs          AppState（全局状态 + 待办 / 子任务 / 设置写操作）
├── windows/          WindowManager（mod.rs）+ 每个窗口一个文件
│   ├── main_window.rs    W1（现为简版）
│   ├── editor.rs  subtask.rs  completed.rs  notification.rs     W2（占位）
│   ├── settings.rs  fence.rs                                    W3（占位）
│   └── gallery.rs        调试用控件样例（MINI_TODO_DEBUG_OPEN=gallery）
├── ui/               Element Plus 风格共享控件（button / icon / input / message / message_box / tooltip / title_bar / layers）
├── services/         logging、single_instance、sync、autostart（已实现）；tray、auto_sync（W1）、scheduler（W2）为空实现
└── platform/         mod.rs 跨平台接口；windows.rs（Win32）；fallback.rs（Linux）；embed.rs（嵌入桌面，W1 实现，现为空实现）
```

### 2.1 并行开发的文件约定（避免合并冲突）

| 文件 | 规则 |
|---|---|
| 自己负责的窗口文件、新建的 `ui/*.rs`、新建的 `services/*.rs` | 随便改 |
| `windows/mod.rs` | 只在需要时**追加** `DebugOpen` 项（W3 收纳箱相关的除外都已经有了）；不要改已有函数签名 |
| `state.rs` | W1 实现 `toggle_fixed_mode` / `save_window_state` 等标了 `TODO(W1)` 的方法；需要新的**跨窗口**共享字段时在结构体末尾加并注明负责人；只属于某个窗口的状态放在自己的视图实体里 |
| `theme.rs` | 新 token 加在文件末尾、按窗口分节（`// ---- W2：编辑窗口 ----`），数值照规格 |
| `ui/` 已有文件 | 只修 bug，不改已有 API；需要变体时加 builder 方法 |
| `platform/mod.rs`、`platform/windows.rs` | W1 可补实现；W2 / W3 需要新的平台能力先找 W1 或加在 `platform/` 新文件里 |
| `platform/embed.rs` | **W1 实现**（主窗口嵌入桌面与收纳箱共用），W3 只调用 |
| `lib.rs` | 已留好启动钩子（`services::tray::start`、`services::auto_sync::start`、`services::scheduler::start`、`windows::fence::restore_all`），填各自的函数体即可，不用改 `lib.rs` |
| `Cargo.toml` | 已预先加好：`async-channel`、`futures`、`chrono`、`dirs`、`serde(_json)`、`url`、`regex`、`auto-launch 0.5`；Windows：`windows 0.62`（features 见文件）、`tray-icon 0.21`、`rfd 0.16`。确实要加依赖时加在 `[dependencies]` 末尾并注明用途；`windows` 缺 feature 时追加到列表末尾 |

## 3. 约定（必读）

1. **颜色 / 尺寸只从 `theme.rs` 取**；文案逐字照抄规格。日志一律 `log::*`（落盘），不要 `println!`。
2. **字体**：每个窗口根元素 `.font(theme::ui_font())`（Windows `Segoe UI` + `Microsoft YaHei` 回退；Linux `Liberation Sans` +
   `Noto Sans CJK SC`）、`.text_size(theme::FONT_SIZE)`、`.line_height(relative(theme::LINE_HEIGHT))`。待办标题等用户字体用
   `theme::todo_font(&state.todo_font_family)`。CSS `font-weight: 600` → `theme::WEIGHT_SEMIBOLD`，`500` → Regular。
3. **窗口外观**：全部无系统标题栏（`WindowSpec::options()` 已处理）；主窗口 / 通知 / 收纳箱 `spec.transparent = true`
   （Root 背景自动覆盖为透明，界面自己画圆角底色）；其它窗口不透明白底。
4. **阻塞调用放后台**：`db::spawn_db(cx, |db| core::…)` 返回 `Task<Result<T, String>>`；回前台用 `WeakEntity::update`
   （返回 `Result`，不要用会 panic 的 `AsyncApp::update` 处理可能已销毁的实体）。
5. **App 借用与 Win32**：`SetWindowPos` / `ShowWindow` / 改 style 会同步给 GPUI 窗口过程发消息，App 被借用时（事件处理、
   `update`、`render` 里）GPUI 处理不了 → 几何状态错乱。`platform::*(window, cx, …)` 的写操作已经**自动延后**到下一个前台任务；
   `platform::Hwnd` 上的原始方法是立即执行的，只能在 `cx.spawn` 的 async 块里、`update` 闭包之外调用。
   同理，**任何模态的系统调用**（同步 `SendMessage` 拖窗、同步对话框）都不能在事件处理里直接调。
6. **拖动窗口**：普通标题栏用 `ui::title_bar::drag_region()`（放第一个子元素，Windows 上走系统 `HTCAPTION`：贴边分屏、
   可缩放窗口双击最大化）；拖动区之上的可交互元素必须 `.occlude()`（`EpButton` 已是），文字不用。需要"移动 3px 才拖"时
   自己在 `on_mouse_move` 里调 `platform::begin_window_drag(window, cx)`。Windows 上 `window.start_window_move()` 是空操作。
7. **GPUI 没有 z-index**：后绘制者在上；浮层用 `deferred(anchored()…)`。消息 / 确认框由 `UiLayer` 插件画在 Root 之上，窗口视图不用管。
   圆角容器裁不掉子元素背景（内容遮罩只有矩形）。
8. **SVG 图标必须设颜色**才会绘制（`ui::icon::icon(source, size, color)` 已处理）。本地图片用 `img(PathBuf)`（`&str` 会被当成 URI）。
9. **QuitMode::Explicit**：关掉所有窗口进程不退出（托盘常驻）；Linux 没有托盘，最后一个窗口关闭时退出。

## 4. 接口

### 4.1 `state::AppState`（`Entity` + Global）

```rust
let state = AppState::global(cx);                            // Entity<AppState>
cx.observe(&state, |_, _, cx| cx.notify()).detach();         // 视图跟着刷新（存进 _subscriptions）
cx.subscribe_in(&state, window, |_, _, e: &AppStateEvent, window, cx| …)  // 主窗口：Message { kind, text } → ui::message::show
```

| 字段（pub） | 说明 |
|---|---|
| `todos: Vec<Todo>` | 全部待办（含子任务），`todo::list` 顺序 |
| `view_mode: ViewMode`（`List` / `Quadrant`） | settings `view_mode` |
| `is_fixed`、`fixed_embed_desktop`、`is_dark_theme`、`show_calendar`、`auto_hide_enabled`、`top_on_wake` | 同旧 appStore；`is_dark_theme` = `text_theme == "light"` |
| `window_bg: theme::WindowBackground { color, alpha }` | 深色主题底色 |
| `window_position` / `window_size`（物理像素）、`current_screen_config_id`、`screen_configs` | W1 维护 |
| `subtask_expand_all` | 展开全部子任务（存 `<数据目录>/ui-state.json`，替代 localStorage） |
| `todo_font_family`、`todo_font_size`（12~20，默认 14） | 待办字体 |
| `notification_type: NotificationType`（`System` / `App`） | |
| `has_update`、`latest_version`、`release_url` | 版本检查（W1） |
| `runtime_prefs: RuntimePrefs { top_on_wake, auto_hide_enabled }` | 贴边隐藏 / 唤起置顶缓存 |
| `syncing`、`auto_sync_failing`、`last_sync_report` | 同步状态 |
| `initialized` | 首次加载完成 |

| 方法 | 说明 |
|---|---|
| `main_palette() -> theme::Palette` | 主窗口调色板（深色只在主窗口） |
| `is_embedded_in_desktop()` / `is_classic_fixed()` | `IS_WINDOWS && is_fixed && fixed_embed_desktop` / 固定且未嵌入（无圆角无描边） |
| `pending_todos()`（sortOrder 升序）/ `completed_todos()`（降序）/ `todo_count()` / `todos_by_quadrant()`（仅未完成，非法象限归 1）/ `todo(id)` | 派生值 |
| `load_initial(cx) -> Task<()>` | 启动时 `run()` 已调用：读设置 → 拉待办 |
| `reload_app_settings(cx) -> Task<()>` | 旧 `reloadAppSettings`（含运行时偏好） |
| `fetch_todos(cx) -> Shared<Task<bool>>` / `reload_todos(cx)` | **单飞 + 过期丢弃**（拉取期间有本地写操作则丢弃并补拉）；连续失败只发一次 `加载待办失败：…` 事件 |
| `refresh_if_changed(cx)` | 5s 轮询用：`change_seq` 变了才拉（模态子窗口打开期间调用方跳过） |
| `create_todo(req)` / `update_todo(id, req)` / `toggle_complete(id)` / `update_todo_quadrant(id, q, color)` / `delete_todo(id)` / `reorder_todos(ids)` | 后台执行，成功后更新本地副本；返回 `Task<Result<_, String>>`（core 原始错误文本，调用方加前缀提示，如 `更新待办失败`） |
| `create_subtask(req)` / `update_subtask(id, req)` / `delete_subtask(id)` / `reorder_subtasks(ids)` / `reload_todo(id)` | 子任务（同步到父待办的 `subtasks`）；`reload_todo` = 旧 `loadTodo(false)` |
| `state::update_request() -> UpdateTodoRequest` | 全空的更新请求（`..state::update_request()`） |
| `set_view_mode(mode)` | 失败只记日志 |
| `set_dark_theme(b)` / `set_show_calendar(b)` / `set_auto_hide_enabled(b)` / `set_top_on_wake(b)` / `set_fixed_embed_desktop(b)` / `set_window_background(color, alpha)` | 先改状态再写库；失败回滚，`Err` 已是完整提示文本（`设置深色主题失败：…` 等），直接 `message::error(window, cx, text)` |
| `set_todo_font(family, size)` / `set_notification_type(t)` / `toggle_subtask_expand_all()` / `reload_screen_configs()` | |
| `apply_sync_report(&report)` | 由 `run_sync` 调用（同一份报告只处理一次） |
| `toggle_fixed_mode()` / `save_window_state()` | **TODO(W1)**：现为空实现 |

### 4.2 `services`

```rust
// 同步编排（标题栏按钮、自动同步、设置页立即同步 / 强制拉取 / 强制推送共用）
let task: Task<Result<SyncReport, String>> = services::sync::run_sync(cx, SyncMode::Smart { full: true });
```
- 期间 `AppState.syncing = true`；成功后 `apply_sync_report`：本地数据有变化且主窗口没有模态子窗口 → 重新拉待办；
  `settings_applied` → 重读设置与运行时偏好；同一份报告只处理一次。错误原样返回（`同步正在进行中`、`未配置 WebDAV 服务器`…）。
- 文案（`syncReport.ts` 逐字移植）：`describe_sync_report(&r)`、`describe_skipped_records(&r) -> Option<String>`、
  `has_local_data_changes(&r)`、`is_sync_busy_error(&e)`、`report_message_kind(&r)`（no_changes → info，否则 success）。
- 自动同步定时器：`services::auto_sync::start(cx)`（W1 实现；设置窗口关闭后 WindowManager 会自动再调一次）。

`services::autostart::{is_enabled, enable, disable, set_enabled, refresh_if_enabled, APP_NAME, AUTOSTART_ARG}`：阻塞调用（放后台），
与旧版同名同参数（`HKCU\…\Run\mini-todo = "<exe>" --autostart`）。启动自愈已在 `run()` 里做（开发隔离时跳过）。

`services::single_instance`：已接好（Windows 命名互斥量 + 与旧版 Tauri 插件相同的接收窗口 / `WM_COPYDATA` 格式，新旧版互相识别）；
第二实例唤起时调用 `windows::main_window::show_and_activate`（W1 可改实现，不要改名）。

`services::logging`：`%LOCALAPPDATA%\com.tauri-app.mini-todo\logs\mini-todo.log`（隔离时 `<dir>/logs`），2MB 轮转保留 1 份；
本应用 Info 起，第三方 crate Warn 起。

### 4.3 `windows`：WindowManager 与各窗口入口

```rust
// 在打开方自己的事件处理里（打开方窗口正被借用）：
let parent = ChildOf::window(window, cx);
editor::open(cx, Some(parent), Some(todo_id), EditorMode::View);
// 只有句柄时（托盘）：主窗口为模态打开方，但不设 owner
let parent = ChildOf::handle(main_handle, cx).unowned();
editor::open(cx, Some(parent), None, EditorMode::Edit);
```

| 入口 | 窗口 | 尺寸 / 参数 |
|---|---|---|
| `main_window::open(cx)`、`main_window::show_and_activate(cx)` | 主窗口 | 380×600，最小 320×400，透明，可缩放 |
| `editor::open(cx, parent, todo_id: Option<i64>, EditorMode::{View, Edit}) -> Option<AnyWindowHandle>` | 待办详情 / 新建 | 1080×600，可缩放；`todo_id = None` 为新建（总是编辑）；系统标题 `待办详情` / `新建待办` |
| `subtask::open(cx, parent, subtask_id, SubtaskMode::{View, Edit})` | 子任务 | 800×750；标题 `查看子任务` / `编辑子任务`（W2 可扩展内存子任务参数） |
| `completed::open(cx, parent)` | 已完成 | 460×550 |
| `settings::open(cx, parent, SettingsTab::{General, Appearance, Data, Screen, About})` | 设置 | 680×560，不可缩放；`SettingsTab::label()` = 菜单文案 |
| `notification::open(cx, NotificationContent { todo_id, title, description }) -> Option<AnyWindowHandle>` | 应用内通知 | 320×120，透明、`WindowKind::PopUp`（置顶、不进任务栏）、不抢焦点；core 的槽位与叠放算法已接好，关窗归还槽位；`NotificationContent::from(&Reminder)`；`notification::close_all(cx)` |
| `fence::open(cx, fence_id)`、`fence::restore_all(cx)`、`fence::fence_key(id)` | 收纳箱 | 透明、不可缩放；登记 key `fence-{id}`；`restore_all` 是 W3 的启动钩子 |

`WindowManager`（Global）：

| 方法 | 说明 |
|---|---|
| `open_child(cx, parent: Option<&ChildOf>, kind, key, spec: WindowSpec, build) -> Option<AnyWindowHandle>` | 各窗口 `open` 的公共实现：打开方已有模态子窗口 → 带到前台并返回 `None`；在打开方显示器（整块）居中；登记；Windows 设 owner（去掉 `WS_EX_APPWINDOW`）；记模态关系。**打开方是主窗口时，子窗口关闭后自动 `reload_todos`，设置窗口再重读设置 + `auto_sync::start`** |
| `open_window(cx, &spec, build)`（模块函数） | 不走模态逻辑的底层开窗（Root 包装 + 透明处理） |
| `WindowSpec::new(title, bounds)` + 字段 `resizable / minimizable / transparent / focus / show / kind / min_size / display_id` | 统一窗口参数 |
| `register(cx, kind, key, handle)` / `on_closed(cx, handle, FnOnce(&mut App))` / `close(cx, handle)` | 登记、关闭回调、关闭 |
| `main(cx)` / `all(cx, kind)` / `find(cx, kind, key)` / `kind_of(cx, id)` | 查询 |
| `modal_child(cx, opener_id)` / `has_modal_child` / `main_has_modal_child(cx)` / `focus_modal_child(cx, opener_id)` | 模态关系 |
| `forward_activation(window, cx) -> bool` | 打开方在 `cx.observe_window_activation` 里调用：有模态子窗口就把焦点交给它 |
| `windows::modal_overlay(window, cx, alpha) -> Option<AnyElement>` | 打开方 render 的**最后一个**子元素（主窗口 / 已完成 0.3，编辑窗口 0.15）；打开方要 `cx.observe_global::<WindowManager>(|_, cx| cx.notify())` |
| `windows::centered_bounds(monitor, opener_bounds, size)` | 居中公式：`x = round(mon.x/s + (mon.w/s − W)/2)` |
| `windows::placeholder(id, title, note)` | 占位视图（替换掉即可） |

### 4.4 `ui`：Element Plus 风格控件

| 控件 | 用法 |
|---|---|
| 图标 | `ui::icon::icon(EpIcon::Bell, 12., color) -> Svg`（`IconSource` 也接受 `AppIcon` 与资源路径）；`ui::icon::spinning("id", svg)` 1s 一圈 |
| `EpButton` | `EpButton::new(id).label("保存").primary()`；变体 `.primary() .success() .warning() .danger() .info()`、`.plain() .text() .link() .circle() .small()`、`.icon(EpIcon::Edit).icon_size(14.).icon_gap(12.)`（默认 12px = `:icon` + span）、`.disabled(b) .loading(b) .tooltip("…") .on_click(..)`；实现 `Styled`（`.w_full()` 等）；已 `occlude()` |
| `EpInput` | 状态在构造视图时建：`cx.new(\|cx\| InputState::new(window, cx).placeholder("…"))`；渲染 `EpInput::new(&state).small().disabled(b).cleanable(b).prefix(..).suffix(..)`（`Styled`）；读写 `state.read(cx).value()` / `state.update(cx, \|s, cx\| s.set_value(v, window, cx))`；事件 `ui::input::InputEvent` |
| 原生 title tooltip | `.tooltip(ui::tooltip::native("设置"))`（任意 `.id()` 元素；GPUI 自带定位：光标右下、500ms） |
| EP 深色 tooltip | `EpTooltip::new(id, "每天重复", child).show_after_ms(300).placement(Placement::Top).max_width(px(..))`（带箭头、上方放不下翻到下方、hide 200ms、淡入；`Styled` 作用于包裹层） |
| ElMessage | `ui::message::{success, info, warning, error}(window, cx, text)`、`show_for(window, cx, kind, text, Duration)`、`error_with(window, cx, "保存待办失败", &err)`（= `notifyError`，含 200 字截断）、`show_in(handle, cx, kind, text)`（只有句柄时）、`error_text(prefix, detail)` |
| ElMessageBox.confirm | `ui::message_box::confirm(window, cx, ConfirmOptions::new(title, message).confirm_text("删除").cancel_text("取消").kind(BoxKind::Warning), \|ok, window, cx\| …)`；回调已延后到确认框关闭之后；`message_box::is_open(window, cx)` |
| 标题栏 | `WindowTitleBar::new(id, "待办详情").title_size(17.).padding(12., 20.).action(el).closable(b).draggable(b).on_close(..)`（默认编辑窗口样式，关闭默认 `remove_window()`）；`drag_region()` |
| 浮层插件 | `ui::layers::UiLayer`（`run()` 已注册；消息与确认框画在这里） |

`theme`：`light()` / `Palette::dark(&bg)` / `Palette::for_main(is_dark, &bg)`（应用色 `#3B82F6` 系，字段同 CSS 变量：`primary`、`bg_tertiary`、
`text_secondary`、`border`、`window_bg`、`window_border`…，以及 `shadow_sm/base/md/lg()`）；`ep()`（Element Plus 色 `#409EFF` 系：
`primary_light_3/5/7/9`、`text_regular`、`border_hover`、`fill_light`、`overlay`、`tooltip_bg`…，`box_shadow()` / `box_shadow_light()`）；
`hex(0xRRGGBB)` / `hexa(0xRRGGBBAA)` / `rgba_css(r, g, b, a)` / `todo_color(&todo.color, fallback)` / `hex_to_rgb_channels` / `parse_hex_color`；
`radius::{SM, BASE, MD, LG, XL}`、`space::S1..S6`、`duration::{FAST, BASE, SLOW}`、`FONT_SIZE`、`LINE_HEIGHT`、`EP_HEIGHT_DEFAULT/SMALL`、
`PRESET_BG_COLORS`、`ui_font()` / `todo_font(family)` / `font_family()` / `font_fallbacks()`、`shadow(x, y, blur, spread, color)`。
gpui-component 主题已调成 EP（primary `#409EFF`、前景 `#606266`、placeholder `#A8ABB2`、边框 `#DCDFE6`、圆角 6 / 4、无焦点外环、字号 14），
直接用 `Select` / `Checkbox` / `Switch` / `DatePicker` 等组件时颜色已接近 EP，尺寸按规格再覆盖。

`assets`：`EpIcon::*`（旧前端用到的 54 个 + EP 组件内部的 `CircleClose`、`DArrowLeft/Right`、`Minus`、`ZoomIn/Out`、`ScaleToOriginal`）、
`AppIcon::{QuadGrid, CloseThin, ArrowDownTip, ArrowUpTip}`；新增图标：SVG 放进 `assets/icons/{ep,app}/`，在 `assets.rs` 表里加一行。

### 4.5 `platform`

| 函数（`&Window` 版写操作自动延后执行） | 说明 |
|---|---|
| `hwnd(window) -> Option<Hwnd>` | 只有 Windows 有 |
| `begin_window_drag(window, cx)` | 程序化拖动（Windows：延后 `ReleaseCapture` + `WM_NCLBUTTONDOWN/HTCAPTION`；Linux：`start_window_move`） |
| `toggle_maximize(window, cx)` | |
| `window_rect_physical(window) -> Option<PhysRect>` / `set_window_rect_physical(window, cx, rect)` / `set_window_position_physical(window, cx, x, y)` | 外框物理像素（`SetWindowPos(NOZORDER\|NOACTIVATE)`；Linux 无法移动窗口，no-op） |
| `monitors(cx) -> Vec<MonitorInfo>` / `primary_monitor` / `monitor_of_window(window, cx)` / `monitor_at_cursor(cx)` | `MonitorInfo { display_id, rect, work, scale, primary }`（物理像素；`logical_rect()` / `logical_work()`）；Linux 缩放按 1 |
| `cursor_position_physical()` | Linux `None` |
| `screen_config_id(cx)` | 屏幕组合 ID（core `screen_config_id`，debug 下 `MINI_TODO_SCREEN_ID_FILE` 可覆盖） |
| `set_topmost` / `show_window`（不激活）/ `hide_window` / `is_window_visible` / `set_tool_window` / `set_minimizable` / `set_resizable_frame` / `set_owner` / `restore_if_minimized` / `bring_to_front` | `set_resizable_frame` 只改系统样式，GPUI 的缩放热区仍按建窗时的 `is_resizable` |
| `set_round_corners(window, b)` | Win11 DWM 圆角（主窗口建窗后已调用） |
| `Hwnd::{rect, set_rect, set_position, set_topmost, show, hide, is_visible, is_minimized, restore, is_alive, ex_style, set_tool_window, set_minimizable, set_resizable_frame, set_owner, owner, set_round_corners, class_name}` | 立即执行的原始调用（只在 App 未借用时） |
| `platform::embed::{desktop_host, attach_to_desktop, detach_from_desktop, set_always_on_bottom, needs_reattach}` | 嵌入桌面层，**W1 实现**（现为安全空实现：返回"未嵌入"）；W3 调用 |

### 4.6 `MINI_TODO_DEBUG_OPEN`（仅 debug 构建）

逗号分隔，加载完数据后依次打开（不走模态关系，主窗口不显示遮罩）：`main`、`editor:new`、`editor:<id>[:view|:edit]`、
`subtask:<id>[:view|:edit]`、`settings[:general|appearance|data|screen|about]`、`completed`、`notification[:<todoId>]`、
`fence:<id>`、`gallery`（控件样例）、`toast:<success|info|warning|error>:<文字>`、`confirm:<标题>:<内容>`（后两项在主窗口里弹）。
详见 `../../scripts/README.md`；新增项在 `windows/mod.rs` 的 `DebugOpen` / `parse_debug_open` / `open_debug` 里加。

## 5. 已知限制 / 注意

- Windows 行为（DirectComposition 透明、owner、DWM 圆角、HTCAPTION 拖动、单实例、托盘、Toast）只做了交叉 `clippy`，需要实机验收。
- debug 构建的 Windows exe 在运行时从构建机 cargo registry 编译着色器，只能在构建它的机器上运行；release 必须在装了
  Windows SDK（`fxc.exe`）的 Windows 上构建。
- Xvfb 下透明窗口的透明像素截图为黑色 / 白色（无合成器）；GPUI 在光标离开窗口后不清除悬停（`shot.sh` 已把光标停在标题栏空白处）。
- `windows/main_window.rs` 是简版：只有列表、标题栏按钮（主题 / 视图 / 设置 / 已完成 / 同步）、FAB、遮罩、5s 轮询；其余见 W1 范围。
