# mini-todo（app crate）

GPUI 版 PC 端的界面、窗口与平台集成：package `mini-todo`，lib `mini_todo` + bin `mini-todo`（`main.rs` 只调用
`mini_todo::run()`）。依赖 `gpui-kit =0.7.3`（gpui-pre 0.3.9 + gpui-component 0.7.3，**精确钉版本**）与
`mini-todo-core`（业务逻辑，见 `../core/README.md`）。

本文件面向后续维护者：模块地图、约定、主要接口、调试开关与验证方法。设计依据：
`.trellis/tasks/archive/2026-10/10-15-gpui-rewrite/info.md` §3-4；UI 规格：`research/ui-main-window.md`、`ui-editor-windows.md`、
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
# Xvfb 一键回归：全部场景截图 + 与参考截图的并排对照图 + 端到端冒烟（见 ../../scripts/README.md）
scripts/regress.py --ref <参考截图目录> --out /tmp/regress
scripts/regress.py --smoke --no-build
# 单个场景 / 临时交互
scripts/shot.sh --name main-list --window "Mini Todo" /tmp/shots
scripts/shot.sh --open main,gallery --window 控件样例 /tmp/shots   # ui/ 控件样例
```

参考截图（旧 Tauri 版的 Vue 前端在浏览器中按真实窗口尺寸渲染）是重写期间在仓库外生成的（`ref-ui/harness/run.sh`），
不在仓库里；旧前端已在 3.0 删除，无法再从仓库重新生成。手上有这批截图时用 `--ref` / `MINI_TODO_REF_DIR` 指定目录（**可选**），
否则用 `--before`（改动前跑一遍的输出）做自身回归。

## 2. 模块地图

```text
src/
├── lib.rs            run()：日志 → 单实例 → 数据库 → Application(QuitMode::Explicit) → init → 主题 → UiLayer →
│                     Db / WindowManager / AppState → 单实例唤起监听 → 主窗口 → 后台服务（托盘 / 自动同步 /
│                     提醒调度 / 收纳箱恢复）→ 开机自启自愈 → 加载 → （debug）MINI_TODO_DEBUG_OPEN
├── main.rs           bin 入口（release 为 Windows 子系统）
├── assets.rs         AppAssets（EpIcon / AppIcon 嵌入表 + gpui-kit 默认资源）
├── theme.rs          颜色 / 字体 / 圆角 / 间距 / 阴影 / 时长 token（按窗口分节：主窗口、设置 `settings_tokens()`、
│                     收纳箱 `fence_tokens()`、编辑类窗口与 Markdown `editor_theme::*`）；颜色工具；gpui-component 主题调成 EP
├── db.rs             Db 全局（Arc<Database>）、spawn_db
├── state.rs          AppState（全局状态 + 待办 / 子任务 / 设置写操作；替代 Pinia store 与跨 WebView 事件）
├── util/             旧前端 utils 的 1:1 移植（datetime、image_ref、file_link、editor_shortcuts），带原 vitest 用例
├── windows/
│   ├── mod.rs        WindowManager（登记 / 模态子窗口 / 关闭回调 / 居中）、WindowSpec、modal_overlay、DebugOpen
│   ├── main_window.rs + main_window/   主窗口：title_bar（同步 / 视图 / 固定 / 设置 / 已完成 / 版本红点）、
│   │                 list、todo_item（行 / 子任务 / 拖拽手柄 / 删除确认）、quadrant、calendar（农历 / 节假日 / 跨天条）、
│   │                 logic（纯逻辑：提醒文案、拖拽排序、日历网格、版本比较）；FAB 与操作模式、遮罩、5s 变更轮询
│   ├── editor/       待办详情 / 编辑：mod（窗口、加载、Esc / Ctrl+Enter、弹层关闭）、view（标题栏与只读排版）、
│   │                 form（表单与底部栏）、save（防重入保存、暂存子任务落库）、subtasks（子任务面板、拖拽排序、
│   │                 从文件导入）、desc_dialog（描述放大编辑：源码 / 预览分栏）
│   ├── subtask.rs    子任务查看 / 编辑（库内子任务或新建待办的暂存子任务）
│   ├── completed.rs  已完成（搜索、恢复、删除）
│   ├── notification.rs  应用内通知（透明、置顶、不抢焦点；core 槽位与叠放算法）
│   ├── settings/     设置：mod（窗口、菜单、加载）、general、appearance、data（导入导出 / WebDAV / 强制同步）、
│   │                 screen（屏幕配置）、about（版本 / 检查更新）
│   ├── fence/        桌面收纳箱：mod（窗口、标题栏、拖动 / 缩放 / 折叠 / 改名、拖入）、grid（项目网格、图标懒加载、
│   │                 右键菜单、箱内排序、Ctrl+滚轮）、panels（外观 / 删除确认 / OneDrive 提示、「更多」菜单）、logic（纯函数）
│   └── gallery.rs    控件样例（仅调试）
├── ui/               Element Plus 风格共享控件（见 §4.4，每种控件只有一个实现）
├── services/         进程级服务（见 §4.2）
└── platform/         GPUI 没有的窗口能力（见 §4.5）：mod.rs 跨平台接口、windows.rs（Win32）、fallback.rs（Linux）、
                      embed.rs（嵌入桌面层）、subclass.rs（窗口子类过程）、fence_win.rs（系统取色对话框）、
                      file_dialog.rs（带过滤器的打开 / 保存对话框）
```

### 2.1 维护规则

| 改动 | 规则 |
|---|---|
| 新控件 | 放在 `ui/` 下单独文件、在 `ui/mod.rs` 表里登记；同一种 Element Plus 控件只保留一个实现（缺能力就给现有控件加 builder 方法，不要另写一份） |
| 新颜色 / 尺寸 | 加到 `theme.rs` 对应窗口的分节里，数值照抄 UI 规格；界面代码不写字面量颜色 |
| 跨窗口共享状态 | 放 `AppState`（注明由谁维护）；只属于某个窗口的状态放在该窗口的视图实体里 |
| 新窗口种类 | `WindowKind` + 窗口模块的 `open()`（走 `WindowManager::open_child`）；需要截图时在 `DebugOpen` / `parse_debug_open` / `open_debug` 加一项并更新 `scripts/README.md` |
| 新平台能力 | 加在 `platform/`（Windows 实现 + `fallback.rs` 的 no-op / 近似），不要在窗口代码里直接调 Win32 |
| 新依赖 | 加在 `Cargo.toml` 并注明用途；`windows` crate 缺 feature 时追加到列表末尾（与 gpui 同为 0.62，不要再引入一份） |
| 新 debug 开关 | 只在 `cfg(debug_assertions)` 下读取（release 一律忽略），写进 §5 与 `scripts/README.md` |
| 改动可见界面 | 跑 `scripts/regress.py`（相关场景 `--only`），对照图逐张看过再提交 |

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
   同理，**任何模态的系统调用**（同步 `SendMessage` 拖窗、同步对话框）都不能在事件处理里直接调；会移动用户文件的
   收纳箱操作（带 owner HWND）绝不在 UI 主线程调用（见 `../core/README.md`）。
6. **拖动窗口**：普通标题栏用 `ui::title_bar::drag_region()`（放第一个子元素，Windows 上走系统 `HTCAPTION`：贴边分屏、
   可缩放窗口双击最大化）；拖动区之上的可交互元素必须 `.occlude()`（`EpButton` 已是），文字不用。需要"移动 3px 才拖"时
   自己在 `on_mouse_move` 里调 `platform::begin_window_drag(window, cx)`。Windows 上 `window.start_window_move()` 是空操作。
7. **GPUI 没有 z-index**：后绘制者在上；浮层用 `deferred(anchored()…)`（弹层统一走 `ui::ep_popper::popper`）。消息 / 确认框由
   `UiLayer` 插件画在 Root 之上，窗口视图不用管。圆角容器裁不掉子元素背景（内容遮罩只有矩形）。
8. **SVG 图标必须设颜色**才会绘制（`ui::icon::icon(source, size, color)` 已处理）。本地图片用 `img(PathBuf)`（`&str` 会被当成 URI）。
9. **QuitMode::Explicit**：进程何时退出由应用自己决定 —— Windows 上主窗口关闭后若已没有别的窗口就 `cx.quit()`
   （收纳箱 / 编辑等窗口还开着时进程继续，托盘可再唤起主窗口；见 `windows/main_window.rs`），托盘「退出」直接退出；
   Linux 没有托盘，最后一个窗口关闭时退出（`lib.rs`）。
10. **键盘**：键绑定的 action 先于 `on_key_down` 派发；编辑类窗口的 Esc / Ctrl+Enter 是 action，弹层打开时由窗口自己先收起弹层
    （`any_popup_open` / `close_popups`），控件自身的 Esc 处理只在没有绑定的窗口里生效。

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
| `window_position` / `window_size`（物理像素）、`current_screen_config_id`、`screen_configs` | `services::window_mode` 维护 |
| `subtask_expand_all` | 展开全部子任务（存 `<数据目录>/ui-state.json`，替代 localStorage） |
| `todo_font_family`、`todo_font_size`（12~20，默认 14） | 待办字体 |
| `notification_type: NotificationType`（`System` / `App`） | |
| `has_update`、`latest_version`、`release_url` | 版本检查（`services::update_check`） |
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
| `toggle_fixed_mode()` / `save_window_state()` | 固定模式切换 / 保存窗口状态（实现在 `services::window_mode`） |

### 4.2 `services`

| 模块 | 内容 |
|---|---|
| `logging` | `%LOCALAPPDATA%\com.tauri-app.mini-todo\logs\mini-todo.log`（隔离时 `<dir>/logs`），2MB 轮转保留 1 份；本应用 Info 起，第三方 crate Warn 起 |
| `single_instance` | Windows 命名互斥量 + 与旧版 Tauri 插件相同的接收窗口 / `WM_COPYDATA` 格式（新旧版互相识别）；第二实例唤起 `main_window::show_and_activate` |
| `sync` | `run_sync(cx, SyncMode)`（标题栏按钮、自动同步、设置页立即同步 / 强制拉取 / 强制推送共用）：期间 `AppState.syncing = true`；成功后 `apply_sync_report`（本地数据变了且主窗口没有模态子窗口 → 重新拉待办；`settings_applied` → 重读设置与运行时偏好）。文案（`syncReport.ts` 逐字移植）：`describe_sync_report`、`describe_skipped_records`、`has_local_data_changes`、`is_sync_busy_error`、`report_message_kind` |
| `auto_sync` | 自动同步定时器（`start` / `stop`；间隔轮询、失败段只提示一次；设置窗口关闭后 WindowManager 自动再 `start`） |
| `autostart` | `is_enabled` / `enable` / `disable` / `set_enabled` / `refresh_if_enabled`（阻塞，放后台）；与旧版同名同参数（`HKCU\…\Run\mini-todo = "<exe>" --autostart`）；开发隔离时不刷新 |
| `tray` | 托盘（Windows）：添加待办 / 设置 / 固定模式勾选 / 开机自启勾选 / 收纳箱三个入口 / 重置位置 / 退出；左键单击唤起（300ms 去抖）；Linux 为 no-op |
| `scheduler` | 提醒调度：启动 5s 后补发错过的重复提醒，此后对齐整分钟 `run_due`；系统通知走 `notify`，应用内通知经通道交给前台建窗（建窗失败不推进、下一分钟重试） |
| `notify` | 系统通知（Windows Toast，AppUserModelID 与旧版一致；notify-rust） |
| `window_mode` | 主窗口普通 / 经典固定 / 固定 + 嵌入桌面、贴边隐藏与唤起、位置 / 尺寸与屏幕配置记忆（防抖保存）、托盘唤起 / 重置位置、200ms 轮询（Explorer 重启重挂、样式自愈） |
| `update_check` | GitHub 最新 release：`check_for_updates(cx)`（启动，失败静默）、`check(cx) -> Task<Result<UpdateInfo, String>>`（设置「关于 → 检查更新」）、`releases_url(cx)`；结果写进 `AppState.has_update / latest_version / release_url` |
| `fences` | 桌面收纳箱生命周期：建窗 + 嵌入、启动恢复、新增 / 显示隐藏 / 重置位置、目录监听 → 前台刷新、200ms 嵌入守护、约 2 秒一次的屏幕组合检测 |

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
| `main_window::open(cx)`、`main_window::show_and_activate(cx)` | 主窗口 | 380×600，最小 320×400，透明，可缩放；开日历时按旧版公式加宽 |
| `editor::open(cx, parent, todo_id: Option<i64>, EditorMode::{View, Edit}) -> Option<AnyWindowHandle>` | 待办详情 / 新建 | 1080×600，可缩放；`todo_id = None` 为新建（总是编辑）；系统标题 `待办详情` / `新建待办` |
| `subtask::open(cx, parent, subtask_id, SubtaskMode::{View, Edit})`、`subtask::open_draft(cx, parent, drafts, pending_id, mode)` | 子任务 | 800×750；标题 `查看子任务` / `编辑子任务`；`open_draft` 直接读写新建待办窗口的 `DraftSubtasks` |
| `completed::open(cx, parent)` | 已完成 | 460×550 |
| `settings::open(cx, parent, SettingsTab::{General, Appearance, Data, Screen, About})` | 设置 | 680×560，不可缩放；`SettingsTab::label()` = 菜单文案 |
| `notification::open(cx, NotificationContent { todo_id, title, description }) -> Option<AnyWindowHandle>`、`notification::close_all(cx)` | 应用内通知 | 320×120，透明、`WindowKind::PopUp`（置顶、不进任务栏）、不抢焦点；core 槽位与叠放算法，关窗归还槽位 |
| `fence::open(cx, fence_id)`、`fence::restore_all(cx)`、`fence::tray_create_fence / tray_toggle_fences / tray_reset_fences(cx)`、`fence::fence_key(id)` | 收纳箱 | 透明、不可缩放；登记 key `fence-{id}`；实现在 `services::fences` |
| `gallery::open(cx)` | 控件样例 | 仅调试 |

`WindowManager`（Global）：

| 方法 | 说明 |
|---|---|
| `open_child(cx, parent: Option<&ChildOf>, kind, key, spec: WindowSpec, build) -> Option<AnyWindowHandle>` | 各窗口 `open` 的公共实现：打开方已有模态子窗口 → 带到前台并返回 `None`；在打开方显示器（整块）居中；登记；Windows 设 owner（去掉 `WS_EX_APPWINDOW`）；记模态关系。**打开方是主窗口时，子窗口关闭后自动 `reload_todos`，设置窗口再重读设置 + `auto_sync::start`** |
| `open_window(cx, &spec, build)`（模块函数） | 不走模态逻辑的底层开窗（Root 包装 + 透明处理）；建好后（Windows）按**客户区**把窗口摆到规格位置（GPUI 按逻辑 `window_bounds` 建出的客户区偏高半个底边框，见 §4.5） |
| `WindowSpec::new(title, bounds)` + 字段 `client_rect / resizable / minimizable / transparent / focus / show / kind / min_size / display_id` | 统一窗口参数；`bounds` 是客户区的逻辑矩形，`client_rect` 是精确的物理客户区（主窗口按屏幕配置恢复、收纳箱），`None` 时取 `bounds` × 目标显示器缩放 |
| `register(cx, kind, key, handle)` / `on_closed(cx, handle, FnOnce(&mut App))` / `close(cx, handle)` | 登记、关闭回调、关闭 |
| `main(cx)` / `all(cx, kind)` / `find(cx, kind, key)` / `kind_of(cx, id)` | 查询 |
| `modal_child(cx, opener_id)` / `has_modal_child` / `main_has_modal_child(cx)` / `focus_modal_child(cx, opener_id)` | 模态关系 |
| `forward_activation(window, cx) -> bool` | 打开方在 `cx.observe_window_activation` 里调用：有模态子窗口就把焦点交给它 |
| `windows::modal_overlay(window, cx, alpha) -> Option<AnyElement>` | 打开方 render 的**最后一个**子元素（主窗口 / 已完成 0.3，编辑窗口 0.15）；打开方要 `cx.observe_global::<WindowManager>(|_, cx| cx.notify())` |
| `windows::centered_bounds(monitor, opener_bounds, size)` | 居中公式：`x = round(mon.x/s + (mon.w/s − W)/2)` |

### 4.4 `ui`：Element Plus 风格控件（每种一个实现）

| 控件 | 用法 |
|---|---|
| 图标 | `ui::icon::icon(EpIcon::Bell, 12., color) -> Svg`（`IconSource` 也接受 `AppIcon` 与资源路径）；`ui::icon::spinning("id", svg)` 1s 一圈 |
| `EpButton`（`button`） | `EpButton::new(id).label("保存").primary()`；变体 `.primary() .success() .warning() .danger() .info()`、`.plain() .text() .link() .circle() .small()`、`.icon(EpIcon::Edit).icon_size(14.).icon_gap(12.)`、`.disabled(b) .loading(b) .tooltip("…") .on_click(..)`；实现 `Styled`；已 `occlude()` |
| `EpInput`（`input`） | 状态在构造视图时建：`cx.new(\|cx\| InputState::new(window, cx).placeholder("…"))`；渲染 `EpInput::new(&state).small().disabled(b).cleanable(b).prefix(..).suffix(..)`；事件 `ui::input::InputEvent` |
| 弹层（`ep_popper`） | 触发元素（`relative()`）里放 `anchor.measure()`；打开时 render 末尾追加 `ep_popper::popper(&anchor, window, PopperOptions { placement, flip, gap: ep_popper::OFFSET, arrow, width, height }, ep_popper::frame().child(..), on_dismiss)`：按 `TopStart / BottomStart / BottomEnd / Bottom` 定位，放不下时翻转，水平夹回窗口内，点外面（不在触发元素上）调 `on_dismiss`；箭头 12×6 压在边框上（与 EP 一致） |
| `EpSelect<V = SharedString>`（`ep_select`） | 有状态视图：`cx.new(\|cx\| EpSelect::new(id, window, cx).with_options(vec![EpSelectOption::new(v, "文案")]))`，可选 `.filterable() .clearable() .small() .placeholder(..)`、`set_options / set_value(Option<V>) / value() / set_disabled / is_open / close`；事件 `EpSelectEvent<V>::Change(Option<V>)`（`None` = 清空）；`EpSelectOption::font(name)` 用自身字体渲染；值类型实现 `SelectValue`（已有 `SharedString` / `i32` / `i64`；字符串值不在选项里时触发框显示值本身） |
| `EpDropdownMenu`（`ep_dropdown`） | `EpDropdownMenu::new(id).item(EpDropdownItem::new("cmd", "文案").icon(..).divided()).max_height(..).on_select(\|cmd: &SharedString, window, cx\| ..)`，放进 `popper()`；高度估算 `EpDropdownMenu::estimated_height(项数, 分隔数)` |
| `EpSwitch`（`ep_switch`） | `EpSwitch::new(id).checked(b).disabled(b).loading(b).on_change(\|v: &bool, window, cx\| ..)`（0.3s 过渡） |
| `EpCheckbox`（`ep_checkbox`） | `EpCheckbox::new(id, "周一").checked(b).last(b).on_change(\|v: &bool, window, cx\| ..)` |
| `EpColorPicker`（`ep_color_picker`） | `cx.new(\|cx\| EpColorPicker::new(id, window, cx).predefine(colors))`、`set_value(Option<String>) / value() / set_disabled / is_open / close`；事件 `EpColorPickerEvent::Change(Option<String>)`（OK = 小写 `#rrggbb`，Clear = `None`） |
| `EpRadioButtons`（`ep_radio`）、`EpSlider`（`ep_slider`，`on_input` / `on_change`）、`EpInputNumber`（`ep_input_number`）、`EpDatePicker` / `EpTimePicker`（`ep_date_picker` / `ep_time_picker`） | 见各文件头部 |
| Markdown（`markdown` / `markdown_editor` / `markdown_image_viewer`） | 只读渲染（`minitodo-image://` 映射到本机图片、链接白名单）、源码 + 预览编辑器（图片粘贴 / 拖入上传）、图片预览 |
| 外部文件拖入（`external_drop`） | 视图构造时 `ExternalDrop::new()`，render 末尾 `drop.attach(root, DropScope::Window \| Element, on_drop, on_exit)`。**不要用 `.on_drop::<ExternalPaths>()`**：gpui-pre 0.3.9 的文件拖放不改输入模态，`on_drop` 要求 `is_hovered`，窗口最后一次输入是键盘时（打完字、改名回车后）松手被丢掉、悬停状态残留 |
| 原生 title tooltip | `.tooltip(ui::tooltip::native("设置"))`（任意 `.id()` 元素；光标右下、500ms） |
| EP 深色 tooltip | `EpTooltip::new(id, "每天重复", child).show_after_ms(300).placement(Placement::Top).max_width(px(..))` |
| ElMessage | `ui::message::{success, info, warning, error}(window, cx, text)`、`show_for(..)`、`error_with(window, cx, "保存待办失败", &err)`（= `notifyError`，含 200 字截断）、`show_in(handle, cx, kind, text)`、`error_text(prefix, detail)` |
| ElMessageBox.confirm | `ui::message_box::confirm(window, cx, ConfirmOptions::new(title, message).confirm_text("删除").cancel_text("取消").kind(BoxKind::Warning), \|ok, window, cx\| …)`；回调在确认框关闭之后；`message_box::is_open(window, cx)` |
| 标题栏 | `WindowTitleBar::new(id, "待办详情").title_size(17.).padding(12., 20.).action(el).closable(b).draggable(b).on_close(..)`；`drag_region()` |
| 浮层插件 | `ui::layers::UiLayer`（`run()` 已注册；消息与确认框画在这里） |
| 滚动条 | `ui::scrollbar`（非覆盖式细滚动条，主列表 6px、四象限 4px） |

`theme`：`light()` / `Palette::dark(&bg)` / `Palette::for_main(is_dark, &bg)`（应用色 `#3B82F6` 系）；`ep()`（Element Plus 色 `#409EFF` 系，
`box_shadow()` / `box_shadow_light()`）；`settings_tokens()`、`fence_tokens()`、`editor_theme::*`；`hex` / `hexa` / `rgba_css` / `todo_color` /
`hex_to_rgb_channels` / `parse_hex_color` / `is_same_color`；`radius::*`、`space::*`、`duration::*`、`FONT_SIZE`、`LINE_HEIGHT`、
`PRESET_BG_COLORS`、`ui_font()` / `todo_font(family)`、`shadow(x, y, blur, spread, color)`。gpui-component 主题已调成 EP
（primary `#409EFF`、前景 `#606266`、placeholder `#A8ABB2`、边框 `#DCDFE6`、圆角 6 / 4、无焦点外环、字号 14）。

`assets`：`EpIcon::*`（旧前端用到的图标 + EP 组件内部图标）、`AppIcon::{QuadGrid, CloseThin, ArrowDownTip, ArrowUpTip}`；
新增图标：SVG 放进 `assets/icons/{ep,app}/`，在 `assets.rs` 表里加一行。

### 4.5 `platform`

| 函数（`&Window` 版写操作自动延后执行） | 说明 |
|---|---|
| `hwnd(window) -> Option<Hwnd>` | 只有 Windows 有 |
| `begin_window_drag(window, cx)` / `toggle_maximize(window, cx)` | 程序化拖动（Windows：延后 `ReleaseCapture` + `WM_NCLBUTTONDOWN/HTCAPTION`；Linux：`start_window_move`） |
| `client_rect_physical(window)` / `set_client_rect_physical(window, cx, rect)` / `set_client_position_physical(window, cx, x, y)`、`window_rect_physical(window)`、`set_geometry_locked(window, cx, b)` | 物理像素几何，**一律按客户区（= 可见区域）**：GPUI 窗口在左 / 右 / 下有不可见边框（96 DPI 各 8px，可缩放与否都有），外框只用于换算；设置时换算成外框 `SetWindowPos(NOZORDER\|NOACTIVATE)`，跨 DPI 按新边框再校正（Linux 无法移动窗口，no-op）；固定模式锁定位置与尺寸 |
| `monitors(cx)` / `primary_monitor` / `monitor_of_window(window, cx)` / `monitor_at_cursor(cx)` | `MonitorInfo { display_id, rect, work, scale, primary }`（物理像素；`logical_rect()` / `logical_work()`）；Linux 缩放按 1 |
| `cursor_position_physical()`、`screen_config_id(cx)` | 光标（Linux `None`）、屏幕组合 ID（debug 下 `MINI_TODO_SCREEN_ID_FILE` 可覆盖） |
| `set_topmost` / `show_window`（不激活）/ `hide_window` / `is_window_visible` / `set_tool_window` / `set_minimizable` / `set_resizable_frame` / `set_owner` / `restore_if_minimized` / `bring_to_front` / `set_round_corners` | 窗口样式与显隐；`set_resizable_frame` 只改系统样式，缩放热区由 `subclass` 锁定 |
| `Hwnd::{rect, set_rect, set_position, client_rect, set_client_rect, set_client_position, set_topmost, show, hide, is_visible, is_minimized, restore, is_alive, ex_style, set_tool_window, set_minimizable, set_resizable_frame, set_geometry_locked, set_owner, owner, set_foreground, set_round_corners, class_name}` | 立即执行的原始调用（只在 App 未借用时） |
| `embed::{desktop_host, attach_to_desktop, detach_from_desktop, set_always_on_bottom, needs_reattach}` | 嵌入桌面层：owner = Progman + 永远置底（`subclass::FLAG_BOTTOM`）+ `WS_EX_TOOLWINDOW`；主窗口与收纳箱共用；非 Windows 为安全空实现 |
| `subclass::set_flag(hwnd, FLAG_BOTTOM \| FLAG_LOCK_GEOMETRY, on)` | `SetWindowSubclass` 子类过程：置底（`WM_WINDOWPOSCHANGING`）、固定模式锁定位置与尺寸（`WM_NCHITTEST` / `WM_SYSCOMMAND`） |
| `fence_win::choose_color(cx, owner, initial) -> Task<Option<(u8, u8, u8)>>` | 收纳箱「自定义颜色」：Windows `ChooseColorW`（独立线程）；其它平台 `None` |
| `file_dialog::{save_file, open_file}` | 带标题 / 过滤器 / 默认文件名的对话框（Windows `rfd`，独立线程；其它平台 GPUI 自带对话框）；debug 下 `MINI_TODO_DEBUG_DIALOG_PATH` 直接返回路径 |

## 5. 调试开关（只在 debug 构建读取，release 一律忽略）

| 变量 | 作用 |
|---|---|
| `MINI_TODO_DATA_DIR` | 数据目录（库 / 图片 / 备份 / 图标缓存）、日志 `<dir>/logs`；不启用单实例、不刷新开机自启项（与安装版并存实测时必须带上） |
| `MINI_TODO_FENCES_ROOT` | 收纳根目录（代替 `%USERPROFILE%\MiniTodo收纳箱`） |
| `MINI_TODO_SCREEN_ID_FILE` | 文件内容非空时替代屏幕组合 ID（模拟显示器组合变化） |
| `MINI_TODO_DEBUG_OPEN` | 加载完数据后额外打开的窗口，逗号分隔：`main`、`editor:new`、`editor:<id>[:view\|:edit]`、`subtask:<id>[:view\|:edit]`、`settings[:general\|appearance\|data\|screen\|about]`、`completed`、`notification[:<todoId>]`、`fence:<id>`、`fence:new`、`calendar`、`gallery`、`toast:<success\|info\|warning\|error>:<文字>`、`confirm:<标题>:<内容>`（不走模态关系，详见 `../../scripts/README.md`） |
| `MINI_TODO_DEBUG_LATEST_VERSION` | 版本检查不联网，直接当作 GitHub 最新版本（如 `v2.6.0`，截图更新红点 / 「版本更新」确认框） |
| `MINI_TODO_DEBUG_DIALOG_PATH` | 设置窗口导出 / 导入的文件对话框不弹出，直接返回该路径（导入导出冒烟） |
| `MINI_TODO_DEBUG_WINDOWS_UI` | `1` 时 Linux 上也显示 Windows 专有的设置项（「固定模式时，嵌入桌面中」），与按 Windows 渲染的参考截图对照 |
| `MINI_TODO_DEBUG_FENCE_STATE` | `onedrive`：收纳箱窗口打开即显示 OneDrive 提示面板；`drag-hover`：显示外部拖入悬停的虚线框（两者在 Linux 上无法真实触发） |

## 6. 已知限制 / 注意

- Windows 行为（DirectComposition 透明、owner、DWM 圆角、HTCAPTION 拖动、嵌入桌面与收纳箱、单实例、托盘、Toast、DPAPI、
  开机自启）在 Linux 上只做了交叉 `clippy`，需要实机验收。
- debug 构建的 Windows exe 在运行时从构建机 cargo registry 编译着色器，只能在构建它的机器上运行；release 必须在装了
  Windows SDK（`fxc.exe`）的 Windows 上构建。
- Xvfb 下透明窗口的透明像素截图为黑色（无合成器）；GPUI 在光标离开窗口后不清除悬停（回归脚本会把光标停在空白处）。
- Linux 上没有系统字体枚举（core `settings::system_fonts` 只在 Windows 实现，与旧版相同），设置里的字体下拉显示 `No data`。
- Markdown 编辑是"源码 + 预览"（旧版 Milkdown 所见即所得无法 1:1 还原，PRD D7）。
