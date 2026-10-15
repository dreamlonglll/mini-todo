# 技术设计：pc-gpui

> 依据：`research/backend-port.md`（后端移植地图）、`research/gpui-api.md`（GPUI API 速查）、
> `research/ui-main-window.md` / `ui-editor-windows.md` / `ui-settings-fence.md`（UI 规格）。
> 参考截图（Vue 版在浏览器中按真实窗口尺寸渲染）：
> `/tmp/claude-0/-home-user-mini-todo/9b3d7942-6fc3-57a4-8b18-2becb0556be0/scratchpad/ref-ui/screens/*.png`（重跑方法见同目录 README.md）。
> Element Plus 原版图标 SVG（293 个，kebab-case 文件名）：
> `/tmp/claude-0/-home-user-mini-todo/9b3d7942-6fc3-57a4-8b18-2becb0556be0/scratchpad/ep-icons/*.svg`。

## 1. 目录

```text
pc-gpui/
├── Cargo.toml            # [workspace] members = ["crates/core", "crates/app"]，resolver = "2"
├── Cargo.lock            # 提交
├── rust-toolchain.toml   # 不加（沿用 stable）
├── crates/
│   ├── core/             # package `mini-todo-core`，lib `mini_todo_core`；不依赖 gpui / tauri / tokio
│   └── app/              # package `mini-todo`，bin `mini-todo`；依赖 core + gpui-kit =0.7.3 + 平台 crate
└── README.md
```

`target/` 由仓库根 `.gitignore` 忽略（确认 `pc-gpui/target` 被忽略）。

## 2. core crate（mini-todo-core）

按 `research/backend-port.md` §9 实施，要点：

- **复制**（不是移动）`pc/src-tauri/src` 的 `db/**`、`services/webdav.rs`、`services/secret.rs`、`services/fence_fs/**`，
  保持 `crate::db::…` / `crate::services::…` 模块路径不变；`pc/` 下文件一律不改。
- `commands/*.rs` 去掉 `#[tauri::command]` / `State` / `AppHandle` 外壳，成为普通阻塞函数，放到：
  `todo.rs`、`data.rs`、`sync/`（sync_cmd）、`settings.rs`（settings_cmd + window.rs 中设置 / screen_configs 读写）、
  `holiday.rs`（改 `reqwest::blocking`）、`reminder.rs`（services/notification.rs 的纯逻辑：到期查询、投递回调、
  重复提醒推进、补发、通知窗口定位算法与槽位）、`window_logic.rs`（window.rs 纯函数：贴边状态机等）、
  `fence/mod.rs` + `fence/items.rs`（fence.rs / fence_items.rs 中与窗口无关的部分；需要 owner HWND 的以 `Option<isize>` 参数传入，
  需要通知 UI 的以回调传入）。
- 断开两处反向依赖（`data.rs:245`、`sync_cmd.rs:605` 调 `reload_runtime_prefs`）：core 不刷新运行时偏好，由 app 处理。
- 原 `emit("sync-completed")` 之类改为返回值（`SyncReport`）由 app 广播。
- 新增 `lunar.rs`：用 `tyme4rs`（lunar-javascript 同一作者 6tail 的 Rust 库）实现 `getLunarInfo` / `getLunarDisplayText`
  的等价物（日、月、干支年、节气、传统节日、是否初一、完整文本；显示优先级：节日 > 节气 > 农历日（初一显示月份））。
  用 `pc/node_modules/lunar-javascript` 生成 2024-01-01 ~ 2027-12-31 的对照数据（提交为测试 fixture JSON），单测逐日比对。
- 所有原有测试随模块迁移并通过（271 个，`todo.rs` 中构造 `tauri::ipc::InvokeBody` 的 1 个改写为对 `save_image(bytes, ext)` 的测试）。
- 依赖版本与 `pc/src-tauri/Cargo.toml` 保持一致（rusqlite 0.32 bundled、reqwest 0.12 blocking+json、windows 0.58 等），去掉 tokio。
- `Database::new_in_memory` 改为 `#[cfg(any(test, feature = "test-util"))]`，加 feature `test-util`。

## 3. app crate（mini-todo）—— 总体架构

### 3.1 进程与线程

- 主线程：GPUI `Application`（`QuitMode::Explicit`，托盘常驻，关掉所有窗口不退出），所有窗口与 Win32 窗口操作都在主线程。
- 数据库：`Arc<Database>`（core 的 `Database` 内部是 `Mutex<Connection>`）。读写放 `cx.background_executor().spawn(...)`，
  结果回到前台更新 Entity。小查询（单条 get）也走后台，避免卡帧。
- 后台：提醒调度（对齐整分钟）、自动同步定时器、5s 变更计数轮询、200ms Win32 tick（嵌入守护、贴边隐藏、屏幕组合检测）——
  全部用 GPUI 前台定时任务（`cx.spawn` + `Timer::after`）驱动，阻塞工作派发到 background executor。

### 3.2 全局状态（替代 Pinia store + Tauri 事件）

- `AppState`（`Entity`，注册为 Global 句柄）：`todos: Vec<Todo>`、`change_seq`、`settings`（AppSettings + 窗口相关运行时值）、
  `view_mode`、`show_calendar`、`is_fixed`、`is_embedded`、`syncing`、`font_family/font_size`、`theme` 等。
  所有窗口 `cx.observe(&app_state, …)` 刷新。
- 旧事件 → 新机制：
  | 旧事件 | 新机制 |
  |---|---|
  | `sync-completed` | 同步完成后 `AppState::apply_sync_report` |
  | `data-imported` | 导入完成后 `AppState::reload_all` |
  | `app-settings-changed` / `todo-font-changed` | 直接改 `AppState.settings` 并 `cx.notify()` |
  | `subtask-memory-ready/-init/-save` | 编辑窗口持有 `Entity<DraftSubtasks>`，子任务窗口直接读写它 |
  | `fence-items-changed` | 监听回调经 channel → 前台 → 对应 FenceView `Entity` 更新 |
  | 托盘 4 个事件 | tray-icon 事件 → channel → 前台任务直接调用对应动作 |

### 3.3 窗口

| 旧 WebView（label） | 新 GPUI 视图 | 备注 |
|---|---|---|
| main | `MainWindow` | 无系统标题栏、透明背景、自绘标题栏 |
| editor-* | `EditorWindow` | 只读 ⇄ 编辑 |
| subtask-editor-* | `SubtaskWindow` | |
| settings | `SettingsWindow` | 模态于主窗口（规格见 ui-settings-fence.md） |
| completed | `CompletedWindow` | |
| notification-* | `NotificationWindow` | 定位算法在 core::reminder |
| fence-{id} | `FenceWindow`（Windows） | 嵌入桌面层 |

GPUI（0.3.9，Windows）缺少的窗口能力用 Win32 直接补（`platform/` 模块，`cfg(windows)`，经 `HasWindowHandle` 取 HWND）：
`SetWindowPos`（位置 / 尺寸，物理像素）、置顶 / 置底、`ShowWindow` 显隐、ex style（TOOLWINDOW/APPWINDOW）、owner=Progman、
`WM_WINDOWPOSCHANGING` 子类化（替代 tao always_on_bottom）、拖动窗口（`ReleaseCapture` + `WM_NCLBUTTONDOWN/HTCAPTION`，
因为 `start_window_move` 在 Windows 上是空实现）、显示器枚举（物理像素 + DPI，屏幕组合 ID）。非 Windows 上这些函数为 no-op
或用 GPUI 自带能力近似（Linux 只用于 CI 与截图验证）。

### 3.4 样式还原

- 不使用 gpui-component 的默认主题外观去"近似"；以 UI 规格文档里的数值为准，自绘为主（div + 样式），
  需要复杂交互的控件（文本输入、多行编辑器、下拉、日期 / 时间选择、颜色选择、滑块、开关、复选框、弹层、对话框、Markdown 渲染）
  用 gpui-component，并通过 Theme 与样式参数调成 Element Plus 外观（主色 `#409EFF`，自绘部分 `#3B82F6`，以规格为准）。
- 字体：Windows 上 `Segoe UI` + `Microsoft YaHei`（与 CSS font-family 一致）；Linux 上用系统 CJK 字体（Noto Sans CJK）。
- 图标：Element Plus SVG（`assets/icons/*.svg`），`svg().path("icons/xxx.svg")` 着色。

### 3.5 平台集成

见 backend-port.md §9.3：tray-icon + muda（Windows）、notify-rust（系统通知，AUMID 与旧版一致）、auto-launch（同名同键）、
单实例（命名互斥量 + message-only 窗口唤起）、rfd（带过滤器的打开 / 保存对话框）、flexi_logger（2MB 轮转，路径同旧版）、
embed-resource（exe 图标与版本信息；GPUI 已嵌清单，不要重复）。

## 4. app crate 基础框架（Foundation，先于各窗口实现）

### 4.1 模块布局（各窗口实现者只改自己的文件，共享文件改动要小且可合并）

```text
crates/app/
├── Cargo.toml          # gpui-kit =0.7.3、raw-window-handle 0.6、mini-todo-core、log、flexi_logger、
│                       # [windows] windows 0.62（与 gpui 同版本，避免再多一份）、tray-icon、notify-rust、auto-launch、rfd …
├── build.rs            # Windows：embed-resource / winresource 嵌 exe 图标（资源 ID 1，GPUI 用它做窗口图标）+ 版本信息；不嵌 manifest
├── assets/
│   ├── icons/ep/*.svg  # 用到的 Element Plus 图标（fill=currentColor），文件名 kebab-case 与 @element-plus/icons-vue 一致
│   └── icons/app/*.svg # 自定义图标（如四方格图标，见 ui-main-window.md 附录 A）
└── src/
    ├── main.rs         # 单实例 → 日志 → Database → Application(QuitMode::Explicit) → 主题 → 打开主窗口 / 后台任务
    ├── assets.rs       # AppAssets：EP 图标（include_bytes 表）+ gpui_kit::assets::Assets 串联
    ├── theme.rs        # 颜色 / 字号 / 圆角 / 阴影 token（浅 / 深两套），EP 调色板（#409EFF 系），应用调色板（#3B82F6 系）
    ├── state.rs        # AppState（Entity + Global 句柄）：todos、settings、view_mode、show_calendar、fixed / embedded、
    │                   # syncing、字体…；fetch_todos 单飞 + 过期丢弃；所有写操作经 core 在后台执行后刷新
    ├── db.rs           # Db global（Arc<Database>）；`spawn_db(cx, |db| …) -> Task<Result<T, String>>`
    ├── windows/        # 每个窗口一个文件；mod.rs 是 WindowManager（按种类打开 / 聚焦 / 记录句柄 / 模态标志 / 子窗口关闭回调 /
    │                   # 在当前显示器居中）
    ├── ui/             # 共享的 Element Plus 风格控件：icon、button（el-button 各变体）、tooltip（EP dark tooltip）、
    │                   # message（ElMessage toast，窗口内顶部居中叠放）、message_box（ElMessageBox.confirm）、input（EP 外观的
    │                   # 单行输入，基于 gpui-component InputState）、scrollbar 样式、title_bar_drag（拖动窗口）…
    │                   # 各窗口需要的其它控件（日期 / 时间 / 颜色选择、下拉、开关、滑块、复选框…）由对应窗口实现者在 ui/ 下新建文件
    ├── services/       # scheduler（提醒、自动同步、5s 变更轮询、200ms tick）、sync（同步编排与提示文案）、notify、tray、
    │                   # autostart、single_instance、logging
    └── platform/       # mod.rs：跨平台接口；windows.rs（Win32）；fallback.rs（非 Windows：用 GPUI 能力近似或 no-op）
```

### 4.2 约定

- 颜色 / 尺寸**只**从 `theme.rs` 取（新增 token 加到 theme.rs），数值以 UI 规格文档为准，单位逻辑像素。
- 窗口外观：主窗口 / 通知 / 收纳箱 `WindowBackgroundAppearance::Transparent` + `titlebar: None`，Root 背景覆盖为透明；
  其它窗口不透明、无系统标题栏（旧版 `decorations: false`），用自绘标题栏。
- 拖动窗口：Windows 上 `start_window_move()` 是空操作 → `platform::begin_window_drag(window)`
  （Win32 `ReleaseCapture` + `SendMessageW(WM_NCLBUTTONDOWN, HTCAPTION)`，与 tao `drag_window` 相同）；Linux 用 `window.start_window_move()`。
- HWND：`raw_window_handle::HasWindowHandle::window_handle(window)`（UFCS，`Window` 有同名固有方法）。
- GPUI 缺失的窗口能力（位置、显隐、置顶、置底、ex style、owner、子类化、显示器物理信息）统一走 `platform::*`；
  子类化不得使用 `GWLP_USERDATA`（GPUI 占用），用 `SetWindowSubclass`。
- 后台：阻塞工作一律 `cx.background_executor().spawn`；回前台用 `WeakEntity::update`（返回 Result，不 panic）。
- 本地图片：`img(PathBuf)`（`&str` 会被当成 URI）；Markdown 用 `gpui_kit::base::TextView::markdown(..).image_source(..)` 映射 `minitodo-image://`。
- 文案逐字照抄规格文档。日志 `log::*`。
- debug 专用环境变量（release 忽略，与旧版一致）：`MINI_TODO_DATA_DIR`、`MINI_TODO_FENCES_ROOT`、`MINI_TODO_SCREEN_ID_FILE`；
  新增 `MINI_TODO_DEBUG_OPEN`（启动后额外打开指定窗口，供截图脚本使用，格式见 `pc-gpui/scripts/README.md`）。

### 4.3 验证工具

- `pc-gpui/scripts/shot.sh`：Xvfb + lavapipe 跑真实应用（隔离数据目录、样例数据），按 `MINI_TODO_DEBUG_OPEN` 打开窗口，
  用 `xdotool` 找窗口、`import` 截图到指定目录；支持 `xdotool` 交互（悬停、点击）后再截图。
- 样例数据：`pc-gpui/scripts/sample-data.json`（导出格式 4.0，内容与参考截图 harness 的 fixtures.mjs 一致），
  经 core 的 example（`cargo run -p mini-todo-core --example seed -- <data-dir> <json>`）导入隔离数据目录。
- 时间：参考截图冻结在 2026-10-15 10:00 Asia/Shanghai；需要时用 `faketime` 运行应用。

### 4.4 分工（Foundation 完成并提交后并行）

| 子任务 | 范围 |
|---|---|
| W1 主窗口 | 标题栏、列表、待办行、子任务行、拖拽排序、四象限、日历（农历 / 节假日 / 跨天条）、FAB 与操作模式、模态遮罩、5s 轮询、自动同步、版本检查、窗口位置 / 尺寸 / 屏幕配置记忆、固定模式 / 嵌入桌面 / 贴边隐藏与唤起、托盘 |
| W2 编辑类窗口 | 编辑窗口（只读 ⇄ 编辑、全部表单项、子任务面板、内存子任务）、子任务窗口、Markdown 编辑与渲染（图片、链接白名单）、已完成窗口、应用内通知窗口、提醒调度、系统通知 |
| W3 设置与收纳箱 | 设置窗口全部分区（含 WebDAV、导入导出、屏幕配置、开机自启）、桌面收纳箱窗口（嵌入、几何、拖入、网格、右键菜单、外观、折叠、屏幕组合布局、监听刷新）|
