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
│                     Db / WindowManager / AppState → 整个应用的提示（notice）→ 单实例唤起监听 → 清单 / 日历窗口
│                     （block_windows::start）→ 后台服务（托盘 / 自动同步 / 提醒调度 / 收纳箱恢复）→ 开机自启自愈 → 加载
│                     → 更新日志（升级后第一次运行时弹出，services::changelog）→ （debug）MINI_TODO_DEBUG_OPEN
├── main.rs           bin 入口（release 为 Windows 子系统）
├── assets.rs         AppAssets（EpIcon / AppIcon 嵌入表 + gpui-kit 默认资源）；应用内更新日志的原文是 assets/changelog.md
│                     （services::changelog 用 include_str! 嵌入，写法见文件头）
├── theme.rs          颜色 / 字体 / 圆角 / 间距 / 阴影 / 时长 token（按窗口分节：清单 / 日历窗口、设置 `settings_tokens()`、
│                     收纳箱 `fence_tokens()`、编辑类窗口与 Markdown `editor_theme::*`、下拉菜单 `ep_menu(dark)`、吸附高亮）；颜色工具；
│                     gpui-component 主题调成 EP
├── db.rs             Db 全局（Arc<Database>）、spawn_db
├── state.rs          AppState（全局状态 + 待办 / 子任务 / 清单 / 窗口属性 / 设置写操作；替代 Pinia store 与跨 WebView 事件）
├── util/             旧前端 utils 的 1:1 移植（datetime、image_ref、file_link、editor_shortcuts），带原 vitest 用例；
│                     version（版本号比较 compare_versions，含 semver 预发布排序；版本检查与更新日志共用）
├── windows/
│   ├── mod.rs        WindowManager（登记 / 模态子窗口 / 关闭回调 / 居中）、WindowSpec、modal_overlay、DebugOpen
│   ├── block.rs      BlockKey { List(id), Calendar }（DEFAULT_LIST = List(1)）、BlockDrag + BlockDragHost（清单 / 日历窗口的自绘移动、
│   │                 8 个缩放手柄、标题栏双击判断）、min_physical_size
│   ├── drag.rs       自绘拖动公共部分（与收纳箱共用）：阈值、ResizeDir / resize_rect、光标换算、place_client_rect、手柄、render_snap_guides
│   ├── snap.rs       吸附纯逻辑（SnapKey { Fence, Block }、snap_move / snap_resize、SnapScene::targets_at、group_guides / changed_targets）
│   ├── list_window.rs + list_window/   清单窗口（原主窗口，每个清单一个）：title_bar（清单名 / 版本红点（只在默认清单窗口）/
│   │                 同步 / 展开全部 / 已完成 / 视图 / 日夜 / 锁 / 设置）、name（双击改名、右键「重命名 / 删除清单」、删除确认）、
│   │                 list、todo_item（行 / 子任务 / 拖拽手柄 / 删除确认）、quadrant、row_menu（待办行右键「移动到 ▸」）、
│   │                 logic（纯逻辑：提醒文案、拖拽排序）；FAB（建到本清单）与操作模式、遮罩
│   ├── calendar_window.rs + calendar_window/   日历窗口（单例）：title_bar（月份导航 / 今天 / 清单筛选 / 日夜 / 锁 / 设置）、
│   │                 grid（农历 / 节假日 / 跨天条）、filter（清单筛选下拉）、logic（网格、跨天条、first_rect / default_width）
│   ├── editor/       待办详情 / 编辑：mod（窗口、加载、Esc / Ctrl+Enter、弹层关闭）、view（标题栏与只读排版）、
│   │                 form（表单与底部栏）、save（防重入保存、暂存子任务落库）、subtasks（子任务面板、拖拽排序、
│   │                 从文件导入）、desc_dialog（描述放大编辑：源码 / 预览分栏）
│   ├── subtask.rs    子任务查看 / 编辑（库内子任务或新建待办的暂存子任务）
│   ├── completed.rs  已完成（只列打开它的清单；搜索、恢复、删除）
│   ├── changelog.rs  更新日志（升级后自动弹出：默认清单窗口为打开方、只显示升级区间；设置 → 关于：设置窗口为打开方、全部版本）
│   ├── notification.rs  应用内通知（透明、置顶、不抢焦点；core 槽位与叠放算法）
│   ├── settings/     设置：mod（窗口、菜单、加载）、general、appearance、data（导入导出 / WebDAV / 强制同步）、
│   │                 screen（屏幕配置）、about（版本 / 检查更新 / 更新日志）
│   ├── fence/        桌面收纳箱：mod（窗口、标题栏、拖动 / 缩放（自绘拖动 + 吸附高亮，公共部分在 drag.rs / snap.rs）/ 折叠 / 改名、拖入）、grid（项目网格、图标懒加载、
│   │                 右键菜单（含隐藏 / 取消隐藏；macOS「在访达中显示」）、隐藏项过滤与半透明、箱内排序、Ctrl+滚轮
│   │                 （macOS 另认 ⌘+滚轮））、panels（外观 / 删除确认 / 云同步提示（Windows OneDrive、macOS iCloud）、
│   │                 「更多」菜单（含显示隐藏的图标））、logic（纯函数）
│   └── gallery.rs    控件样例（仅调试）
├── ui/               Element Plus 风格共享控件（见 §4.4，每种控件只有一个实现）
├── services/         进程级服务（见 §4.2）
└── platform/         GPUI 没有的窗口能力（见 §4.5）：mod.rs 跨平台接口、windows.rs（Win32）、
                      macos/（mod.rs GPUI 胶水 + ns.rs AppKit 原始调用，objc2；含收纳箱桌面层与取色面板）、
                      mac_geometry.rs（macOS 坐标换算，全平台编译带单测）、fallback.rs（Linux）、embed.rs（Windows 嵌入桌面层）、
                      subclass.rs（窗口子类过程）、fence_win.rs（收纳箱「自定义颜色」：Windows ChooseColorW / macOS NSColorPanel）、
                      file_dialog.rs（带过滤器的打开 / 保存对话框）、clipboard.rs（粘贴图片时直接读剪贴板，Windows）
```

### 2.1 维护规则

| 改动 | 规则 |
|---|---|
| 新控件 | 放在 `ui/` 下单独文件、在 `ui/mod.rs` 表里登记；同一种 Element Plus 控件只保留一个实现（缺能力就给现有控件加 builder 方法，不要另写一份） |
| 新颜色 / 尺寸 | 加到 `theme.rs` 对应窗口的分节里，数值照抄 UI 规格；界面代码不写字面量颜色 |
| 跨窗口共享状态 | 放 `AppState`（注明由谁维护）；只属于某个窗口的状态放在该窗口的视图实体里 |
| 新窗口种类 | `WindowKind` + 窗口模块的 `open()`（走 `WindowManager::open_child`）；需要截图时在 `DebugOpen` / `parse_debug_open` / `open_debug` 加一项并更新 `scripts/README.md` |
| 新平台能力 | 加在 `platform/`（Windows 实现 + macOS 实现 + `fallback.rs` 的 no-op / 近似），不要在窗口代码里直接调 Win32 / AppKit。macOS 的 objc2 调用写进 `platform/macos/ns.rs`（不依赖 GPUI，`scripts/mac-check` 能在 Linux 上交叉检查），坐标换算写进 `mac_geometry.rs`（Linux 上跑单测），`macos/mod.rs` 只做胶水 |
| 新依赖 | 加在 `Cargo.toml` 并注明用途；`windows` crate 缺 feature 时追加到列表末尾（与 gpui 同为 0.62，不要再引入一份） |
| 新 debug 开关 | 只在 `cfg(debug_assertions)` 下读取（release 一律忽略），写进 §5 与 `scripts/README.md` |
| 改动可见界面 | 跑 `scripts/regress.py`（相关场景 `--only`），对照图逐张看过再提交 |

## 3. 约定（必读）

1. **颜色 / 尺寸只从 `theme.rs` 取**；文案逐字照抄规格。日志一律 `log::*`（落盘），不要 `println!`。
2. **字体**：每个窗口根元素 `.font(theme::ui_font())`（Windows `Segoe UI` + `Microsoft YaHei` 回退；macOS 系统界面字体
   `.SystemUIFont` + `PingFang SC`；Linux `Liberation Sans` + `Noto Sans CJK SC`）、`.text_size(theme::FONT_SIZE)`、`.line_height(relative(theme::LINE_HEIGHT))`。待办标题等用户字体用
   `theme::todo_font(&state.todo_font_family)`。CSS `font-weight: 600` → `theme::WEIGHT_SEMIBOLD`，`500` → Regular。
3. **窗口外观**：全部无系统标题栏（`WindowSpec::options()` 已处理）；清单 / 日历窗口 / 通知 / 收纳箱 `spec.transparent = true`
   （Root 背景自动覆盖为透明，界面自己画圆角底色）；其它窗口不透明白底。
4. **阻塞调用放后台**：`db::spawn_db(cx, |db| core::…)` 返回 `Task<Result<T, String>>`；回前台用 `WeakEntity::update`
   （返回 `Result`，不要用会 panic 的 `AsyncApp::update` 处理可能已销毁的实体）。
5. **App 借用与 Win32**：`SetWindowPos` / `ShowWindow` / 改 style 会同步给 GPUI 窗口过程发消息，App 被借用时（事件处理、
   `update`、`render` 里）GPUI 处理不了 → 几何状态错乱。`platform::*(window, cx, …)` 的写操作已经**自动延后**到下一个前台任务；
   `platform::Hwnd` 上的原始方法是立即执行的，只能在 `cx.spawn` 的 async 块里、`update` 闭包之外调用。
   同理，**任何模态的系统调用**（同步 `SendMessage` 拖窗、同步对话框）都不能在事件处理里直接调；会移动用户文件的
   收纳箱操作（带 owner HWND）绝不在 UI 主线程调用（见 `../core/README.md`）。
6. **拖动窗口**：编辑 / 子任务 / 已完成 / 设置等窗口的标题栏用 `ui::title_bar::drag_region()`（放第一个子元素，Windows 上走系统
   `HTCAPTION`：贴边分屏、可缩放窗口双击最大化）；拖动区之上的可交互元素必须 `.occlude()`（`EpButton` 已是），文字不用。Windows 上
   `window.start_window_move()` 是空操作。要"移动 3px 才拖"、拖动中还要改位置（吸附对齐）时不进系统拖动循环
   （`platform::begin_window_drag` 中途改不了位置），用自绘拖动：`on_drag` + `on_drag_move`（捕获阶段、不看悬停）+ 屏幕物理坐标的光标 +
   `on_next_frame` 每帧摆一次窗，松手后再读客户区——收纳箱（`windows/fence/mod.rs`）与清单 / 日历窗口（`windows/block.rs` 的 `BlockDrag`，
   建窗 `resizable: false`、窗口内 8 个缩放手柄，没有 Aero Snap / 双击最大化）都是这样，公共部分在 `windows/drag.rs`，约定见 spec gpui-conventions §2.3。
   macOS 上标题栏同样是自绘的：系统红绿灯建窗后由 `open_window` 隐藏，`WindowSpec` 设了 `app_owns_titlebar_drag`（AppKit 不再从
   顶部标题栏区域拖窗、不延迟标题栏上的点击），`drag_region` 的窗口拖动走 `start_window_move`。
7. **GPUI 没有 z-index**：后绘制者在上；浮层用 `deferred(anchored()…)`（弹层统一走 `ui::ep_popper::popper`）。消息 / 确认框由
   `UiLayer` 插件画在 Root 之上，窗口视图不用管。圆角容器裁不掉子元素背景（内容遮罩只有矩形）。
8. **SVG 图标必须设颜色**才会绘制（`ui::icon::icon(source, size, color)` 已处理）。本地图片用 `img(PathBuf)`（`&str` 会被当成 URI）。
9. **QuitMode::Explicit**：进程何时退出由应用自己决定 —— Windows / macOS 上关闭清单 / 日历窗口（Alt+F4、⌘W）只是隐藏
   （`block_windows::request_close`），所有窗口都关了也不退出，托盘（菜单栏图标）常驻，托盘左键 / 单实例 / Dock（`mac_menu::on_reopen`）
   唤起所有清单窗口（`window_mode::bring_all_to_front`）；只有托盘「退出」/ ⌘Q / 应用菜单「退出」退出。Linux 没有托盘、不能程序化隐藏，
   关窗是真的关，最后一个窗口关闭时退出（`lib.rs`）。
10. **键盘**：键绑定的 action 先于 `on_key_down` 派发；编辑类窗口的 Esc / Ctrl+Enter 是 action，弹层打开时由窗口自己先收起弹层
    （`any_popup_open` / `close_popups`），控件自身的 Esc 处理只在没有绑定的窗口里生效。

## 4. 接口

### 4.1 `state::AppState`（`Entity` + Global）

```rust
let state = AppState::global(cx);                            // Entity<AppState>
cx.observe(&state, |_, _, cx| cx.notify()).detach();         // 视图跟着刷新（存进 _subscriptions）
// AppStateEvent::Message { kind, text }（加载待办失败）由 services::notice 订阅，显示在看得见的清单 / 日历窗口里
```

| 字段（pub） | 说明 |
|---|---|
| `todos: Vec<Todo>` | 全部待办（含子任务），`todo::list` 顺序 |
| `lists: Vec<TodoList>`、`lists_revision: u64` | 全部清单（按 `sortOrder`、id；随待办一起加载）；清单列表每被替换一次 `lists_revision` 加一，`services::block_windows` 观察它开 / 关清单窗口 |
| `fixed_embed_desktop`、`auto_hide_enabled`、`top_on_wake` | 全局偏好（作用于每个锁定的窗口）；锁定本身按窗口在 `services::window_mode` |
| `window_bg: theme::WindowBackground { color, alpha }` | 深色主题底色（全局） |
| `current_screen_config_id`、`screen_configs` | `services::window_mode` 维护（启动 / 重置位置时算组合 ID） |
| （私有）窗口属性的内存副本 | 每个清单 / 日历窗口的隐藏、日 / 夜、列表 / 四象限、展开全部、日历筛选（core `window_prefs`）；读写见下表。同步键 `view_mode` / `text_theme` / `show_calendar` / `is_fixed` 只作迁移 v31 的初始值，不再读写；`ui-state.json` 的展开全部启动时迁给默认清单窗口（`migrate_legacy_ui_state`） |
| `todo_font_family`、`todo_font_size`（12~20，默认 14） | 待办字体 |
| `notification_type: NotificationType`（`System` / `App`） | |
| `has_update`、`latest_version`、`release_url` | 版本检查（`services::update_check`） |
| `runtime_prefs: RuntimePrefs { top_on_wake, auto_hide_enabled }` | 贴边隐藏 / 唤起置顶缓存 |
| `syncing`、`auto_sync_failing`、`last_sync_report` | 同步状态 |
| `initialized` | 首次加载完成 |

| 方法 | 说明 |
|---|---|
| `palette_for(key) -> theme::Palette` / `is_dark(key)` | 清单 / 日历窗口的调色板（深色按窗口；其它窗口固定浅色） |
| `window_prefs(key)` / `has_window_record(key)` / `view_mode(key)` / `expand_all(key)` / `calendar_visible()` | 窗口属性（`BlockKey`；`calendar_visible` = 日历窗口没隐藏） |
| `effective_list_id(todo)` / `list(id)` / `list_name(id)` | 有效清单（core `lists::effective_list_id`：归属未知或清单不存在 → 默认清单）与清单查询 |
| `pending_todos_in(list_id)`（sortOrder 升序）/ `todo_count_in(list_id)` / `todos_by_quadrant_in(list_id)`（仅未完成，非法象限归 1）/ `todo(id)` | 按有效清单的派生值 |
| `calendar_todos()` / `calendar_filter()` | 日历窗口显示的待办（全部清单，按筛选）；`calendar_filter()` 只算现有清单、空了按"全部"（`None`） |
| `set_startup_snapshot(lists, prefs)` | `block_windows::start` 建窗前同步读出的清单与窗口属性（建窗前就要知道深色 / 视图） |
| `load_initial(cx) -> Task<()>` | 启动时 `run()` 已调用：读设置 → 拉待办与清单 |
| `reload_app_settings(cx) -> Task<()>` | 旧 `reloadAppSettings`（含运行时偏好） |
| `fetch_todos(cx) -> Shared<Task<bool>>` / `reload_todos(cx)` | **单飞 + 过期丢弃**（拉取期间有本地写操作则丢弃并补拉）；连续失败只发一次 `加载待办失败：…` 事件 |
| `refresh_if_changed(cx)` | 5s 轮询用：`change_seq` 变了才拉（模态子窗口打开期间调用方跳过） |
| `create_todo(req)` / `update_todo(id, req)` / `toggle_complete(id)` / `update_todo_quadrant(id, q, color)` / `delete_todo(id)` / `reorder_todos(ids)` | 后台执行，成功后更新本地副本；返回 `Task<Result<_, String>>`（core 原始错误文本，调用方加前缀提示，如 `更新待办失败`）。`UpdateTodoRequest.list_id` 只在用户真的改了清单时给 `Some`（否则归属未知的待办会被写成 1） |
| `move_todo_to_list(id, list_id)` | 行右键「移动到」：**先改本地副本**（本窗口里随即消失、目标窗口出现），后台 `core::todo::move_to_list`，失败把归属改回去；`sortOrder` 不变 |
| `create_list(name: Option<String>)` / `rename_list(id, name)` / `delete_list(id)` | 清单 CRUD（core `lists`）；`create_list(None)` 自动取名"清单 / 清单 2…"（`state::next_list_name`）并建好显示中的窗口记录；`delete_list` 成功后清理内存里的日历筛选 |
| `set_window_dark(key, b)` / `set_window_view_mode(key, mode)` / `toggle_window_expand_all(key)` / `set_window_hidden(key, b)` / `set_calendar_filter(Option<Vec<i64>>)`（`None` = 全部） | 窗口属性：先改内存、后台写 core `window_prefs`，失败回滚；`set_calendar_filter` 带写入计数，连续点选时晚到的结果不覆盖后一次 |
| `create_subtask(req)` / `update_subtask(id, req)` / `delete_subtask(id)` / `reorder_subtasks(ids)` / `reload_todo(id)` | 子任务（同步到父待办的 `subtasks`）；`reload_todo` = 旧 `loadTodo(false)` |
| `state::update_request() -> UpdateTodoRequest` | 全空的更新请求（`..state::update_request()`） |
| `set_auto_hide_enabled(b)` / `set_top_on_wake(b)` / `set_fixed_embed_desktop(b)` / `set_window_background(color, alpha)` | 先改状态再写库；失败回滚，`Err` 已是完整提示文本，直接 `message::error(window, cx, text)` |
| `set_todo_font(family, size)` / `set_notification_type(t)` / `reload_screen_configs()` | |
| `apply_sync_report(&report)` | 由 `run_sync` 调用（同一份报告只处理一次；本地数据（含清单）变了且没有清单 / 日历窗口开着模态子窗口 → 重新拉待办与清单） |

锁定与几何不在 `AppState`：`window_mode::is_fixed / is_classic_fixed / toggle_fixed(cx, key)`、`save_window_state(cx, key)`（§4.2）。

### 4.2 `services`

| 模块 | 内容 |
|---|---|
| `logging` | `%LOCALAPPDATA%\com.tauri-app.mini-todo\logs\mini-todo.log`；macOS `~/Library/Logs/com.tauri-app.mini-todo/mini-todo.log`（同 2.5.0 mac 版）；隔离时 `<dir>/logs`。2MB 轮转保留 1 份；本应用 Info 起，第三方 crate Warn 起 |
| `single_instance` | 与旧版 Tauri 插件同名同格式（新旧版互相识别）：Windows 命名互斥量 + 接收窗口 / `WM_COPYDATA`；macOS Unix socket `/tmp/com_tauri_app_mini_todo_si.sock`（`<cwd>\0\0<args 以 \0 连接>`，`single_instance/unix_socket.rs`，退出时删除）。第二实例唤起 `window_mode::bring_all_to_front`（同托盘左键；mac 上先 `cx.activate(true)`） |
| `sync` | `run_sync(cx, SyncMode)`（标题栏按钮、自动同步、设置页立即同步 / 强制拉取 / 强制推送共用）：期间 `AppState.syncing = true`；成功后 `apply_sync_report`（本地数据（含清单）变了且没有清单 / 日历窗口开着模态子窗口 → 重新拉待办与清单；`settings_applied` → 重读设置与运行时偏好）；任何一次同步成功撤回还没显示的「自动同步失败」提示（`notice::retract`）。文案（`syncReport.ts` 逐字移植）：`describe_sync_report`、`describe_skipped_records`、`has_local_data_changes`、`is_sync_busy_error`、`report_message_kind` |
| `auto_sync` | 自动同步定时器（`start` / `stop`；间隔轮询、失败段只提示一次；设置窗口关闭后 WindowManager 自动再 `start`） |
| `autostart` | `is_enabled` / `enable` / `disable` / `set_enabled` / `refresh_if_enabled`（阻塞，放后台）；与旧版同名同参数（Windows `HKCU\…\Run\mini-todo = "<exe>" --autostart`；macOS LaunchAgent `~/Library/LaunchAgents/mini-todo.plist`，`[<可执行文件绝对路径>, --autostart]`，从 App Translocation 临时位置运行时拒绝写入）；开发隔离时不刷新 |
| `tray` | 托盘 / 菜单栏图标（Windows、macOS）：添加待办项（建到默认清单）/ 新增待办清单 / 显示日历（勾选，`set_calendar_checked`）/ 收纳箱三个入口 / 重置位置 / 打开设置 / 开机自启勾选 / 退出（没有「固定模式」）；左键单击唤起所有清单窗口（300ms 去抖，`window_mode::bring_all_to_front`）。图标：Windows exe 资源；macOS 模板图标 `assets/tray-template.png`（36×36，`scripts/macos/gen-tray-icon.py` 生成）。Linux 为 no-op |
| `scheduler` | 提醒调度：启动 5s 后补发错过的重复提醒，此后对齐整分钟 `run_due`；系统通知走 `notify`（macOS 经通道交给前台、交出即视为已投递），应用内通知经通道交给前台建窗（建窗失败不推进、下一分钟重试） |
| `notify` | 系统通知：Windows Toast（AppUserModelID 与旧版一致；notify-rust）；macOS GPUI `show_system_notification`（`UNUserNotificationCenter`，归属 bundle id `com.tauri-app.mini-todo`，需从 ad-hoc 签名的 `.app` 启动，拿不到投递结果） |
| `window_mode/` | 清单 / 日历窗口（`BlockKey`）**按窗口**的普通 / 经典固定 / 固定 + 嵌入桌面、贴边隐藏与唤起（`mod.rs` 运行时，`machine.rs` 每窗口一份的纯状态机 `ModeState`）；几何与锁定按屏幕组合持久化（`store.rs` → core `window_prefs` 的 `window_layouts`；默认清单窗口另外登记 `screen_configs`）；用户拖动松手后保存一次、系统挪窗 500ms 防抖保存；唤起所有窗口（`bring_all_to_front`）/ 重置位置（`reset_all`：全部层叠、解锁、取消隐藏，先写库再开窗）；200ms 轮询遍历所有窗口（Explorer 重启重挂、样式自愈）；集中观察 AppState 的嵌入 / 贴边 / 唤起置顶偏好；`is_shown` 给 `notice`。macOS 同一套（固定 = `setMovable:NO`，置顶 = 浮动层级，无嵌入桌面）；唤起 / 重置前先 `cx.activate(true)` |
| `block_windows` | 清单 / 日历窗口的生命周期：启动开窗（默认清单总是显示，隐藏的不建）、关闭 = 隐藏（`request_close`，不注销）、唤起前 `prepare_wake_all`、「显示日历」（`set_calendar_visible`）、托盘「新增待办清单」（`create_list`，找空位后进入改名）、清单列表变化后对账开 / 关窗口（`reconcile` / `plan_reconcile`，同步来的新清单在主显示器找空位 `new_list_rect`）、5 秒变更轮询与检查更新 |
| `snap` | 吸附组运行时（收纳箱 + 清单 + 日历窗口）：`register` / `unregister`、`scene`（拖动开始时快照其它显示中组员的可见矩形）、`targets`（Alt 不吸附）、`set_guides` / `clear_guides` / `edges`（目标窗口的高亮边，只通知变了的窗口）、`visible_rects` |
| `notice` | 整个应用的提示（自动同步失败、加载待办失败、导入成功、托盘新建清单失败）：`show` / `show_from` / `show_topic` / `retract`，只在看得见的清单 / 日历窗口里挑（`pick_target`：当前激活的 → 发起操作的 → 最近激活过的 → 默认清单），都看不见时先存着（最多 3 条，同类型同文字 / 同主题去重），下次有窗口被激活后 300ms 补显示 |
| `mac_menu` | macOS 应用菜单：Mini Todo（关于 / 设置… ⌘, / 服务 / 隐藏 ⌘H / 隐藏其他 ⌥⌘H / 全部显示 / 退出 ⌘Q）、编辑（撤销 / 重做 / 剪切 / 拷贝 / 粘贴 / 全选，动作是输入框的 `input::*`，剪切等映射系统选择器）、窗口（最小化 ⌘M / 关闭窗口 ⌘W；收纳箱是活动窗口时两者都不做）；`on_reopen`（Dock / Finder 唤起 = `window_mode::bring_all_to_front`；GPUI 只在没有可见窗口时回调）。全平台编译，只在 macOS 上由 `lib.rs` 调用 |
| `update_check` | GitHub 最新 release：`check_for_updates(cx)`（启动，失败静默）、`check(cx) -> Task<Result<UpdateInfo, String>>`（设置「关于 → 检查更新」）、`releases_url(cx)`；结果写进 `AppState.has_update / latest_version / release_url` |
| `changelog` | 应用内更新日志：`parse`（容错：未知标题 / 空段落 / 重复版本 / 代码块里的 `##` 都忽略，按版本从新到旧）、`sections()`（嵌入的 `assets/changelog.md`，`## 未发布` 不显示）、`sections_between(sections, after, upto)`（`(after, upto]`）、`decide(last_run, schema_at_open, current, sections) -> StartupDecision`（纯函数：记录 = 当前什么都不做；没有记录时新装（迁移前库版本 0）只写记录、`< 31` 显示 `<=` 当前的全部、`>= 31` 按上次是 3.1.0-pre；记录较旧显示 `(记录, 当前]`；降级 / 区间为空只写记录）、`check_on_startup(cx)`（`lib.rs` 加载完数据后调用：后台读 core `settings::last_run_version` 与 `Database::schema_version_at_open` → 以默认清单窗口为打开方（`ChildOf::handle(..).unowned()`）弹出 → 写回当前版本）。debug 构建 + `MINI_TODO_DATA_DIR` 时默认不判定、不写记录（`MINI_TODO_DEBUG_CHANGELOG=auto` 恢复）。单测 `current_version_has_a_changelog_section`：工作区版本在更新日志里没有段落时失败 |
| `fences` | 桌面收纳箱生命周期：建窗 + 嵌入（Windows）/ 桌面层（macOS：`PopUp` 面板 → `Hwnd::make_desktop_layer` → 显示，日志自报层级与行为）、启动恢复（Windows / macOS）、新增 / 显示隐藏 / 重置位置、目录监听 → 前台刷新、200ms 嵌入守护（Windows）、约 2 秒一次的屏幕组合检测；建窗后登记进吸附组（`services::snap`），窗口关闭 / 删除时注销 |

### 4.3 `windows`：WindowManager 与各窗口入口

```rust
// 在打开方自己的事件处理里（打开方窗口正被借用）：
let parent = ChildOf::window(window, cx);
editor::open(cx, Some(parent), Some(todo_id), EditorMode::View);
// 只有句柄时（托盘）：默认清单窗口为模态打开方，但不设 owner
let parent = WindowManager::default_list(cx).map(|h| ChildOf::handle(h, cx).unowned());
editor::open(cx, parent, None, EditorMode::Edit);
```

| 入口 | 窗口 | 尺寸 / 参数 |
|---|---|---|
| `services::block_windows`（`start` / `show_list(cx, id)` / `create_list(cx)` / `reconcile(cx)` / `request_close`） | 清单窗口（`WindowKind::List`，key `list:<id>`） | 每个清单一个；380×600，最小 320×400，透明，自绘移动与缩放（建窗不可缩放）；系统标题：默认清单窗口固定 `Mini Todo`，其它 = 清单名；关闭 = 隐藏 |
| `block_windows::set_calendar_visible(cx, bool)` | 日历窗口（`WindowKind::Calendar`，key `calendar`） | 单例；最小 420×480，没有布局时贴默认清单窗口右侧、等高（`calendar_window::logic::first_rect`）；系统标题 `日历`；关闭 = 关掉「显示日历」 |
| `editor::open(cx, parent, todo_id: Option<i64>, EditorMode::{View, Edit}) -> Option<AnyWindowHandle>`、`editor::open_new(cx, parent, list_id)` | 待办详情 / 新建 | 1080×600，可缩放；`todo_id = None` 为新建（总是编辑，「清单」下拉预选默认清单；清单窗口 FAB 用 `open_new` 预选本清单）；表单「四象限」下有「清单」下拉，保存时只有选择 ≠ 预选才带 `list_id`；只读详情有「所属清单」徽章；系统标题 `待办详情` / `新建待办` |
| `subtask::open(cx, parent, subtask_id, SubtaskMode::{View, Edit})`、`subtask::open_draft(cx, parent, drafts, pending_id, mode)` | 子任务 | 800×750；标题 `查看子任务` / `编辑子任务`；`open_draft` 直接读写新建待办窗口的 `DraftSubtasks` |
| `completed::open(cx, parent, list_id)` | 已完成 | 460×550；只列该清单（有效清单）的已完成，恢复后回原清单 |
| `changelog::open(cx, parent, sections, upgraded)`、`changelog::open_all(cx, parent)` | 更新日志（`WindowKind::Changelog`） | 520×600，可缩放（最小 420×400）；自绘标题栏「更新日志」、可滚动的 Markdown 只读渲染（每个版本一个标题：版本号 + 日期）、底部「知道了」，Esc 关闭；`upgraded`（升级后自动弹出，打开方默认清单窗口、不设 owner）开头说明"已更新到 x"；`open_all` 显示 `<=` 当前版本的全部段落（设置 → 关于「更新日志」：打开方是设置窗口，设置窗口显示遮罩 0.3、获得焦点时转交） |
| `settings::open(cx, parent, SettingsTab::{General, Appearance, Data, Screen, About})` | 设置 | 680×560，不可缩放；`SettingsTab::label()` = 菜单文案。「深色主题」切的是打开它的清单 / 日历窗口；常规「展示日历」= `set_calendar_visible`（与托盘勾选同一个设备本地状态）；屏幕配置页每个组合一行，汇总保存了几个清单窗口、日历、几个收纳箱的位置与锁定数（`screen::load_layout_summaries`），删除确认写明连带删除这些布局，当前组合不能删 |
| `notification::open(cx, NotificationContent { todo_id, title, description }) -> Option<AnyWindowHandle>`、`notification::close_all(cx)` | 应用内通知 | 320×120，透明、`WindowKind::PopUp`（置顶、不进任务栏）、不抢焦点；core 槽位与叠放算法，关窗归还槽位 |
| `fence::open(cx, fence_id)`、`fence::restore_all(cx)`、`fence::tray_create_fence / tray_toggle_fences / tray_reset_fences(cx)`、`fence::fence_key(id)` | 收纳箱 | 透明、不可缩放、不可最小化、建窗不显示（定位后再显示）；macOS 上 `kind = PopUp`（不激活应用的 NSPanel；只有双击标题改名时先 `cx.activate(true)` 再 `activate_window()`，宽限期内把 key 从清单窗口拿回来，见 `windows/fence/mod.rs` 模块文档）；登记 key `fence-{id}`；实现在 `services::fences` |
| `gallery::open(cx)` | 控件样例 | 仅调试 |

`WindowManager`（Global）：

| 方法 | 说明 |
|---|---|
| `open_child(cx, parent: Option<&ChildOf>, kind, key, spec: WindowSpec, build) -> Option<AnyWindowHandle>` | 各窗口 `open` 的公共实现：打开方已有模态子窗口 → 带到前台并返回 `None`；在打开方显示器（整块）居中；登记；Windows 设 owner（去掉 `WS_EX_APPWINDOW`）；记模态关系。**打开方是任一清单 / 日历窗口时，子窗口关闭后自动 `reload_todos`，设置窗口再重读设置 + `auto_sync::start`** |
| `open_window(cx, &spec, build)`（模块函数） | 不走模态逻辑的底层开窗（Root 包装 + 透明处理）；建好后（Windows，同一个延后任务）先去掉 `WS_SYSMENU`（否则 DWM 标题栏按钮会从半透明客户区透出来），再按**客户区**把窗口摆到规格位置（GPUI 按逻辑 `window_bounds` 建出的客户区偏高半个底边框），见 §4.5 与 `platform` 模块文档 |
| `WindowSpec::new(title, bounds)` + 字段 `client_rect / resizable / minimizable / transparent / focus / show / kind / min_size / display_id` | 统一窗口参数；`bounds` 是客户区的逻辑矩形，`client_rect` 是精确的物理客户区（清单 / 日历窗口按 `window_layouts` 恢复、收纳箱），`None` 时取 `bounds` × 目标显示器缩放 |
| `register(cx, kind, key, handle)` / `on_closed(cx, handle, FnOnce(&mut App))` / `close(cx, handle)` | 登记、关闭回调、关闭 |
| `block_window(cx, key)`（含原生隐藏着的）/ `default_list(cx)` / `block_key_of(cx, id)` / `all(cx, kind)` / `find(cx, kind, key)` / `kind_of(cx, id)` | 查询 |
| `modal_child(cx, opener_id)` / `has_modal_child` / `any_block_has_modal_child(cx)` / `focus_modal_child(cx, opener_id)` | 模态关系（`any_block_has_modal_child`：任一清单 / 日历窗口开着模态子窗口时 5 秒轮询与同步后的刷新跳过） |
| `forward_activation(window, cx) -> bool` | 打开方在 `cx.observe_window_activation` 里调用：有模态子窗口就把焦点交给它 |
| `windows::modal_overlay(window, cx, alpha) -> Option<AnyElement>` | 打开方 render 的**最后一个**子元素（清单 / 日历窗口 / 已完成 / 设置 0.3，编辑窗口 0.15）；打开方要 `cx.observe_global::<WindowManager>(|_, cx| cx.notify())` |
| `windows::centered_bounds(monitor, opener_bounds, size)` | 居中公式：`x = round(mon.x/s + (mon.w/s − W)/2)` |

### 4.4 `ui`：Element Plus 风格控件（每种一个实现）

| 控件 | 用法 |
|---|---|
| 图标 | `ui::icon::icon(EpIcon::Bell, 12., color) -> Svg`（`IconSource` 也接受 `AppIcon` 与资源路径）；`ui::icon::spinning("id", svg)` 1s 一圈 |
| `EpButton`（`button`） | `EpButton::new(id).label("保存").primary()`；变体 `.primary() .success() .warning() .danger() .info()`、`.plain() .text() .link() .circle() .small()`、`.icon(EpIcon::Edit).icon_size(14.).icon_gap(12.)`、`.disabled(b) .loading(b) .tooltip("…") .on_click(..)`；实现 `Styled`；已 `occlude()` |
| `EpInput`（`input`） | 状态在构造视图时建：`cx.new(\|cx\| InputState::new(window, cx).placeholder("…"))`；渲染 `EpInput::new(&state).small().disabled(b).cleanable(b).prefix(..).suffix(..)`；事件 `ui::input::InputEvent` |
| 弹层（`ep_popper`） | 触发元素（`relative()`）里放 `anchor.measure()`；打开时 render 末尾追加 `ep_popper::popper(&anchor, window, PopperOptions { placement, flip, gap: ep_popper::OFFSET, arrow, width, height }, ep_popper::frame().child(..), on_dismiss)`：按 `TopStart / BottomStart / BottomEnd / Bottom` 定位，放不下时翻转，水平夹回窗口内，点外面（不在触发元素上）调 `on_dismiss`；箭头 12×6 压在边框上（与 EP 一致）。`PopperOptions.dark`（箭头配色）、`frame_for(dark)`（`frame()` = 浅色）；`Anchor::at(point)` 零尺寸锚点（右键菜单）；`Placement::RightStart`（级联子菜单，侧边弹出不画箭头） |
| `EpSelect<V = SharedString>`（`ep_select`） | 有状态视图：`cx.new(\|cx\| EpSelect::new(id, window, cx).with_options(vec![EpSelectOption::new(v, "文案")]))`，可选 `.filterable() .clearable() .small() .placeholder(..)`、`set_options / set_value(Option<V>) / value() / set_disabled / is_open / close`；事件 `EpSelectEvent<V>::Change(Option<V>)`（`None` = 清空）；`EpSelectOption::font(name)` 用自身字体渲染；值类型实现 `SelectValue`（已有 `SharedString` / `i32` / `i64`；字符串值不在选项里时触发框显示值本身） |
| `EpDropdownMenu`（`ep_dropdown`） | `EpDropdownMenu::new(id).item(EpDropdownItem::new("cmd", "文案").icon(..).divided()).max_height(..).on_select(\|cmd: &SharedString, window, cx\| ..)`，放进 `popper()`；高度估算 `EpDropdownMenu::estimated_height(项数, 分隔数)`。`.dark(bool)`（EP 暗黑弹层色，清单 / 日历窗口跟随窗口日 / 夜）、`.width(f32)`（定宽时文字截断）、`.anchor(Anchor)`、`.on_hover(command, hovered)`、`panel_width()` / `panel_height()`；`EpDropdownItem::disabled(b)` / `.submenu()` / `.note(text)`（置灰项同时作 tooltip）/ `.checked(b)` |
| `EpSwitch`（`ep_switch`） | `EpSwitch::new(id).checked(b).disabled(b).loading(b).on_change(\|v: &bool, window, cx\| ..)`（0.3s 过渡） |
| `EpCheckbox`（`ep_checkbox`） | `EpCheckbox::new(id, "周一").checked(b).last(b).on_change(\|v: &bool, window, cx\| ..)` |
| `EpColorPicker`（`ep_color_picker`） | `cx.new(\|cx\| EpColorPicker::new(id, window, cx).predefine(colors))`、`set_value(Option<String>) / value() / set_disabled / is_open / close`；事件 `EpColorPickerEvent::Change(Option<String>)`（OK = 小写 `#rrggbb`，Clear = `None`） |
| `EpRadioButtons`（`ep_radio`）、`EpSlider`（`ep_slider`，`on_input` / `on_change`）、`EpInputNumber`（`ep_input_number`）、`EpDatePicker` / `EpTimePicker`（`ep_date_picker` / `ep_time_picker`） | 见各文件头部 |
| Markdown（`markdown` / `markdown_editor` / `markdown_image_viewer`） | 只读渲染（`minitodo-image://` 映射到本机图片、链接白名单）、源码 + 预览编辑器（图片粘贴 / 拖入上传、拖动手柄调整图片大小——尺寸存在 alt 后缀 `|宽` / `|宽x高`）、图片预览 |
| 外部文件拖入（`external_drop`） | 视图构造时 `ExternalDrop::new()`，render 末尾 `drop.attach(root, DropScope::Window \| Element, on_drop, on_exit)`。**不要用 `.on_drop::<ExternalPaths>()`**：gpui-pre 0.3.9 的文件拖放不改输入模态，`on_drop` 要求 `is_hovered`，窗口最后一次输入是键盘时（打完字、改名回车后）松手被丢掉、悬停状态残留 |
| 原生 title tooltip | `.tooltip(ui::tooltip::native("设置"))`（任意 `.id()` 元素；光标右下、500ms） |
| EP 深色 tooltip | `EpTooltip::new(id, "每天重复", child).show_after_ms(300).placement(Placement::Top).max_width(px(..))` |
| ElMessage | `ui::message::{success, info, warning, error}(window, cx, text)`、`show_for(..)`、`error_with(window, cx, "保存待办失败", &err)`（= `notifyError`，含 200 字截断）、`show_in(handle, cx, kind, text)`、`error_text(prefix, detail)` |
| ElMessageBox.confirm | `ui::message_box::confirm(window, cx, ConfirmOptions::new(title, message).confirm_text("删除").cancel_text("取消").kind(BoxKind::Warning), \|ok, window, cx\| …)`；回调在确认框关闭之后；`message_box::is_open(window, cx)` |
| 标题栏 | `WindowTitleBar::new(id, "待办详情").title_size(17.).padding(12., 20.).action(el).closable(b).draggable(b).on_close(..)`；`drag_region()` |
| 浮层插件 | `ui::layers::UiLayer`（`run()` 已注册；消息与确认框画在这里） |
| 滚动条 | `ui::scrollbar`（非覆盖式细滚动条，主列表 6px、四象限 4px） |

`theme`：`light()` / `Palette::dark(&bg)` / `Palette::for_main(is_dark, &bg)`（应用色 `#3B82F6` 系）；`ep()`（Element Plus 色 `#409EFF` 系，
`box_shadow()` / `box_shadow_light()`）；`ep_menu(dark)`（下拉菜单浅色 / EP 暗黑配色）；`snap_guide_color()` / `SNAP_GUIDE_WIDTH`（吸附高亮，
所有吸附组窗口共用）；`settings_tokens()`、`fence_tokens()`、`editor_theme::*`；`hex` / `hexa` / `rgba_css` / `todo_color` /
`hex_to_rgb_channels` / `parse_hex_color` / `is_same_color`；`radius::*`、`space::*`、`duration::*`、`FONT_SIZE`、`LINE_HEIGHT`、
`PRESET_BG_COLORS`、`ui_font()` / `todo_font(family)`、`shadow(x, y, blur, spread, color)`。gpui-component 主题已调成 EP
（primary `#409EFF`、前景 `#606266`、placeholder `#A8ABB2`、边框 `#DCDFE6`、圆角 6 / 4、无焦点外环、字号 14）。

`assets`：`EpIcon::*`（旧前端用到的图标 + EP 组件内部图标）、`AppIcon::{QuadGrid, CloseThin, ArrowDownTip, ArrowUpTip}`；
新增图标：SVG 放进 `assets/icons/{ep,app}/`，在 `assets.rs` 表里加一行。

### 4.5 `platform`

| 函数（`&Window` 版写操作自动延后执行） | 说明 |
|---|---|
| `hwnd(window) -> Option<Hwnd>` | Windows HWND；macOS NSWindow 的窗口号（每次调用都在 `NSApp.windows` 里重新找，不存对象指针）；Linux `None` |
| `begin_window_drag(window, cx)` / `toggle_maximize(window, cx)` | 程序化发起**系统**拖动（Windows：延后 `ReleaseCapture` + `WM_NCLBUTTONDOWN/HTCAPTION`；macOS / Linux：`start_window_move`；到松手前改不了位置，目前没有调用方——收纳箱是自绘拖动） |
| `client_rect_physical(window)` / `set_client_rect_physical(window, cx, rect)` / `set_client_position_physical(window, cx, x, y)`、`window_rect_physical(window)`、`set_geometry_locked(window, cx, b)` | 物理像素几何，**一律按客户区（= 可见区域）**：GPUI 窗口在左 / 右 / 下有不可见边框（96 DPI 各 8px，可缩放与否都有），外框只用于换算；设置时换算成外框 `SetWindowPos(NOZORDER\|NOACTIVATE)`，跨 DPI 按新边框再校正；macOS `setFrame:display:`（frame = 客户区）；Linux 无法移动窗口，no-op；固定模式锁定位置与尺寸（macOS `setMovable:NO` + 去掉可缩放） |
| `monitors(cx)` / `primary_monitor` / `monitor_of_window(window, cx)` / `monitor_at_cursor(cx)` | `MonitorInfo { display_id, rect, work, scale, primary }`（物理像素；`logical_rect()` / `logical_work()`）；macOS `NSScreen.screens`（主屏在前），物理像素 = 全局点坐标 × 统一系数（所有屏 `backingScaleFactor` 的最大值，单屏 / 同缩放时与 2.5.0 逐像素一致，见 `mac_geometry` 模块文档），`scale` 恒为统一系数；Linux 缩放按 1 |
| `cursor_position_physical()`、`screen_config_id(cx)` | 光标（macOS `NSEvent.mouseLocation`；Linux `None`）、屏幕组合 ID（macOS 按各屏自己的缩放，与 2.5.0 一致；debug 下 `MINI_TODO_SCREEN_ID_FILE` 可覆盖） |
| `creation_bounds(cx, bounds, display_id)` / `physical_scale(window)` | 建窗参数换算（macOS 的 GPUI 按显示器本地坐标建窗，`open_window` 已处理；其它平台原样）/ 逻辑 → 物理像素的系数（Windows / Linux = `scale_factor()`，macOS = 统一系数） |
| `set_topmost` / `show_window`（不激活）/ `hide_window` / `is_window_visible` / `set_tool_window` / `set_minimizable` / `set_resizable_frame` / `set_owner` / `restore_if_minimized` / `bring_to_front` / `set_round_corners` | 窗口样式与显隐；`set_resizable_frame` 只改系统样式，缩放热区由 `subclass` 锁定。macOS：置顶 = `NSFloatingWindowLevel`，显示 `orderFrontRegardless` / 隐藏 `orderOut:`，可缩放 / 可最小化改 style mask（运行时生效）；`set_tool_window` / `set_owner` / `set_round_corners` 是 no-op |
| `Hwnd::{rect, set_rect, set_position, client_rect, set_client_rect, set_client_position, set_topmost, show, hide, is_visible, is_minimized, restore, is_alive, ex_style, set_tool_window, set_minimizable, set_resizable_frame, set_system_menu, set_geometry_locked, set_owner, owner, set_foreground, set_round_corners, class_name}` | 立即执行的原始调用（只在 App 未借用时）；`set_system_menu` 只有 `open_window` 建窗后用（去掉 `WS_SYSMENU`，所有 GPUI 窗口都不带它）；`prepare_after_open` 同样只有 `open_window` 用（macOS：隐藏红绿灯、NSPanel 不随应用失活隐藏；其它平台 no-op） |
| `Hwnd::{make_desktop_layer, describe_layer}` | macOS 收纳箱专用（Windows / Linux no-op 返回 `false` / `None`）：层级 `CGWindowLevelForKey(kCGDesktopIconWindowLevelKey) + 1`、collectionBehavior 整体换成 `CanJoinAllSpaces \| Stationary \| IgnoresCycle \| FullScreenNone`（`0x251`）、`canHide = NO`、`hidesOnDeactivate = NO`、不进「窗口」菜单、无动画；不显示、幂等。之后绝不能对该窗口 `set_topmost`（会把层级改回 0 / 3）。`describe_layer` 返回实际生效的属性（日志 / e2e 用） |
| `embed::{desktop_host, attach_to_desktop, detach_from_desktop, set_always_on_bottom, needs_reattach}` | 嵌入桌面层：owner = Progman + 永远置底（`subclass::FLAG_BOTTOM`）+ `WS_EX_TOOLWINDOW`；Windows 上清单 / 日历窗口与收纳箱共用；非 Windows 为安全空实现（macOS 收纳箱走 `Hwnd::make_desktop_layer`） |
| `subclass::set_flag(hwnd, FLAG_BOTTOM \| FLAG_LOCK_GEOMETRY, on)` | `SetWindowSubclass` 子类过程：置底（`WM_WINDOWPOSCHANGING`）、固定模式锁定位置与尺寸（`WM_NCHITTEST` / `WM_SYSCOMMAND`） |
| `fence_win::choose_color(cx, owner, initial) -> Task<Option<(u8, u8, u8)>>` | 收纳箱「自定义颜色」：Windows `ChooseColorW`（独立线程）；macOS 共享的 `NSColorPanel`（`ns::show_color_panel` / `color_panel_state`；前台任务里显示——会把应用切到前台——之后每 150ms 读一次，面板关掉时返回最后的颜色，没改过视为取消）；Linux `None` |
| `file_dialog::{save_file, open_file}` | 带标题 / 过滤器 / 默认文件名的对话框（Windows `rfd`，独立线程；其它平台 GPUI 自带对话框）；debug 下 `MINI_TODO_DEBUG_DIALOG_PATH` 直接返回路径 |
| `clipboard::{read, from_item, bitmap_to_png}` | Markdown 编辑器粘贴用：Windows 上 `read` 直接枚举剪贴板，按 PNG > JPEG > GIF > WebP > BMP 文件 > `CF_DIB` 取第一个能解码的图片（也认 `image/png` 等 MIME 名格式），附带 `CF_HDROP` 文件与是否有文字；其它平台返回 `None`，用 `from_item` 转换 GPUI 读到的内容；位图由调用方在后台 `bitmap_to_png` |

## 5. 调试开关（只在 debug 构建读取，release 一律忽略）

| 变量 | 作用 |
|---|---|
| `MINI_TODO_DATA_DIR` | 数据目录（库 / 图片 / 备份 / 图标缓存）、日志 `<dir>/logs`；不启用单实例、不刷新开机自启项、不自动弹更新日志也不写本机版本记录（`MINI_TODO_DEBUG_CHANGELOG=auto` 恢复）（与安装版并存实测时必须带上） |
| `MINI_TODO_FENCES_ROOT` | 收纳根目录（代替 `%USERPROFILE%\MiniTodo收纳箱`；macOS `~/MiniTodo收纳箱`） |
| `MINI_TODO_DESKTOP_DIR` | "用户桌面"（core `fence_fs::user_desktop_dir`：收纳箱移回的兜底目标、"不能收纳桌面本身"的保护、云同步提示的"来自桌面"判断）改为该目录；mac e2e 用它避开真实 `~/Desktop`（隐私授权框在 runner 上没人点） |
| `MINI_TODO_SCREEN_ID_FILE` | 文件内容非空时替代屏幕组合 ID（模拟显示器组合变化） |
| `MINI_TODO_DEBUG_OPEN` | 加载完数据后额外打开的窗口，逗号分隔：`main`（默认清单窗口，带到前面）、`list:<id>`（显示清单窗口，没开过就开窗）、`list:new`（同托盘「新增待办清单」，进入改名）、`editor:new`、`editor:<id>[:view\|:edit]`、`subtask:<id>[:view\|:edit]`、`settings[:general\|appearance\|data\|screen\|about]`、`completed`（默认清单的已完成）、`notification[:<todoId>]`、`fence:<id>`、`fence:new`、`calendar`（显示日历窗口，同「展示日历」）、`changelog`（更新日志窗口，全部版本，同设置 → 关于「更新日志」）、`gallery`、`toast:<success\|info\|warning\|error>:<文字>`、`confirm:<标题>:<内容>`（toast / confirm 在默认清单窗口里；不走模态关系，详见 `../../scripts/README.md`） |
| `MINI_TODO_DEBUG_CHANGELOG` | `auto`：数据隔离（`MINI_TODO_DATA_DIR`）下也按正常规则判定是否弹出更新日志、写本机记录（默认隔离时既不弹也不写，e2e / regress 不受干扰；手测时先删掉 settings 的 `last_run_version` 或用旧库） |
| `MINI_TODO_DEBUG_LATEST_VERSION` | 版本检查不联网，直接当作 GitHub 最新版本（如 `v2.6.0`，截图更新红点 / 「版本更新」确认框） |
| `MINI_TODO_DEBUG_DIALOG_PATH` | 设置窗口导出 / 导入的文件对话框不弹出，直接返回该路径（导入导出冒烟） |
| `MINI_TODO_DEBUG_WINDOWS_UI` | `1` 时 Linux 上也显示 Windows 专有的设置项（「固定模式时，嵌入桌面中」），与按 Windows 渲染的参考截图对照 |
| `MINI_TODO_DEBUG_FENCE_STATE` | `onedrive`：收纳箱窗口打开即显示云同步提示面板（Windows / Linux OneDrive 文案，macOS iCloud 文案）；`drag-hover`：显示外部拖入悬停的虚线框（两者在 Linux 上无法真实触发） |

## 6. 已知限制 / 注意

- Windows 行为（DirectComposition 透明、owner、DWM 圆角、HTCAPTION 拖动、嵌入桌面与收纳箱、单实例、托盘、Toast、DPAPI、
  开机自启）在 Linux 上只做了交叉 `clippy`，需要实机验收。
- macOS 的 GPUI 胶水（`platform/macos/mod.rs`、`lib.rs` 里 `cfg(target_os = "macos")` 的几行）只有 CI 的 macOS 作业能编译；
  `platform/macos/ns.rs` 可在 Linux 上用 `scripts/mac-check` 交叉检查，`mac_geometry.rs` / `services/mac_menu.rs` 全平台编译。
  真机行为（红绿灯是否全部隐藏、Retina / 多屏摆窗、固定模式、通知面板在应用失活时是否显示、收纳箱桌面层在 Spaces / 台前调度 / 显示桌面下的表现、
  收纳箱改名时的中文输入法）只能靠 CI 截图与用户反馈。多清单 / 日历窗口（3.1）没有新增 mac 专属代码，共用路径没在 Mac 上跑过：
  ⌘W 隐藏（`orderOut:`）、全部隐藏后点 Dock 带出、新清单窗口进入改名时拿 key、隐藏状态下 `setFrame:` 再显示（「重置位置」）、自绘拖动 / 缩放 / 吸附。
- 清单 / 日历窗口是自绘移动与缩放：没有 Aero Snap、双击标题栏不最大化；有模态子窗口时遮罩挡住手柄、不能缩放；Linux 上拖标题栏不移动窗口
  （GPUI 在 Linux 上不能程序化移动窗口），缩放仍按尺寸生效。
- debug 构建的 Windows exe 在运行时从构建机 cargo registry 编译着色器，只能在构建它的机器上运行；release 必须在装了
  Windows SDK（`fxc.exe`）的 Windows 上构建。
- Xvfb 下透明窗口的透明像素截图为黑色（无合成器）；GPUI 在光标离开窗口后不清除悬停（回归脚本会把光标停在空白处）。
- Linux 上没有系统字体枚举（core `settings::system_fonts` 只在 Windows 实现，与旧版相同），设置里的字体下拉显示 `No data`。
  macOS 用 GPUI 的 CoreText 枚举（`text_system().all_font_names()`，去掉 `.` 开头的私有字体）。
- Markdown 编辑是"源码 + 预览"（旧版 Milkdown 所见即所得无法 1:1 还原，PRD D7）。
