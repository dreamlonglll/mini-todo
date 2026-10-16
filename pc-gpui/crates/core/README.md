# mini-todo-core

PC 端（`pc-gpui/`，GPUI 版，3.0 起是唯一的 PC 实现）的业务核心库：package `mini-todo-core`，lib `mini_todo_core`。
由旧 Tauri 版的 `src-tauri/src` 复制改写而来。旧版源码已在 3.0 删除，需要对照时从 git 历史取：
`git show 749405a:pc/src-tauri/src/<路径>`（`749405a` 是删除前的最后一个提交）。下文的"旧版 / 旧命令 / 旧前端"都指它。

- **不依赖 GPUI / Tauri / tokio**（`tokio` 只作为 `reqwest::blocking` 的内部实现间接出现）。
- 全部函数是**阻塞**的（数据库 / 文件 / 网络 / Shell）。app 层用 `cx.background_executor().spawn(...)`
  或独立线程调用，再把结果交回前台。`sync::run`、`holiday::fetch`、`sync::test_connection` 用的是
  `reqwest::blocking`，**不能**在 tokio runtime 上下文里调用。
- 错误类型沿用旧代码的 `String`（面向用户的中文提示），与旧版前端看到的文案一致。
- 数据完全兼容旧版：同一数据目录（`db::paths`）、同一迁移（v1~v29；v30 起为 GPUI 版新增，只加带默认值的收纳箱列）、同一同步文档（sync-data.json.gz）、
  同一导出格式（4.0）、同一 settings 键。

## 目录

```text
crates/core/
├── Cargo.toml                 # 依赖版本沿用旧版；feature test-util
├── README.md                  # 本文件
├── src/
│   ├── lib.rs
│   ├── db/                    # ← 原样复制（仅 Database::new_in_memory 的 cfg 改为 any(test, feature = "test-util")）
│   ├── services/
│   │   ├── mod.rs
│   │   ├── webdav.rs          # ← 原样复制
│   │   ├── secret.rs          # ← 原样复制（Windows DPAPI）
│   │   └── fence_fs/          # ← 原样复制（ops.rs 中 1 个测试的路径字面量改为按平台拼接）；mac.rs 为 macOS 后端（GPUI 版新增）
│   ├── todo.rs                # ← commands/todo.rs
│   ├── data.rs                # ← commands/data.rs
│   ├── sync/mod.rs            # ← commands/sync_cmd.rs（测试拆到 sync/tests.rs）
│   ├── settings.rs            # ← commands/settings_cmd.rs + window.rs 的设置 / screen_configs + fence.rs 的屏幕组合 ID
│   ├── holiday.rs             # ← commands/holiday.rs（async reqwest → blocking）
│   ├── reminder.rs            # ← services/notification.rs 的纯逻辑
│   ├── window_logic.rs        # ← commands/window.rs + lib.rs 的纯函数
│   ├── lunar.rs               # 新增：农历（tyme4rs），等价于旧前端 utils/lunar.ts
│   └── fence/
│       ├── mod.rs             # ← commands/fence.rs 中与窗口无关的部分
│       └── items.rs           # ← commands/fence_items.rs 中与窗口无关的部分
├── examples/
│   └── seed.rs                # 把导出格式 4.0 的 JSON 导入隔离数据目录（截图 / 冒烟测试用，见 pc-gpui/scripts/README.md）
└── tests/fixtures/
    ├── gen_lunar.cjs          # 用 lunar-javascript 生成农历对照数据
    └── lunar_fixture.json     # 2024-01-01 ~ 2027-12-31 逐日对照（提交）
```

## 构建与测试（在 `pc-gpui/` 下）

```bash
cargo fmt --all -- --check
cargo clippy -p mini-todo-core --all-targets -- -D warnings
cargo test -p mini-todo-core
# Windows 目标交叉检查（覆盖 cfg(windows) 的 COM / Shell / DPAPI / DirectWrite 代码）
cargo clippy -p mini-todo-core --all-targets --target x86_64-pc-windows-gnu -- -D warnings

# 重新生成农历对照数据（在仓库根目录执行；lunar-javascript 装在临时目录，版本与旧前端锁定的 1.7.7 相同）
mkdir -p /tmp/lunar-js && npm i --prefix /tmp/lunar-js lunar-javascript@1.7.7
NODE_PATH=/tmp/lunar-js/node_modules node pc-gpui/crates/core/tests/fixtures/gen_lunar.cjs
```

PowerShell 写法见 `tests/fixtures/gen_lunar.cjs` 文件头。

Windows 专属测试（`fence_fs` 的 shell / sta / 真实图标提取 / Windows 监听、`secret` 的 DPAPI 往返）只在 Windows 上编译运行；
macOS 专属测试（`fence_fs` 的真实 `renamex_np` 移动往返 / 撞名顺延 / 已锁定 / `UF_HIDDEN` 列举 / 真实图标提取 / iCloud 占位判定）
只在 macOS 上编译运行（CI `pc-gpui-e2e-macos.yml` 的 `checks` 作业）；
`fence_fs::location` 有 1 个测试只在 `--release` 下编译（与旧版相同）。

其它 crate 的测试要用内存库时打开 feature：`mini-todo-core = { path = "...", features = ["test-util"] }`，
然后 `Database::new_in_memory()`。

## 线程与调用约束（app 层必读）

| 约束 | 说明 |
|---|---|
| 阻塞函数放后台 | 数据库、文件、网络、Shell 调用都会阻塞；GPUI 前台只做状态更新 |
| 带 `owner_hwnd` 的收纳箱移动绝不在 UI 主线程调用 | `fence::delete`、`fence::items::{move_in, restore, move_to}`：系统"需要管理员权限" / "文件正在使用"对话框以收纳箱窗口为父窗口，会同步给主线程发消息，主线程阻塞等待时死锁。HWND 在主线程取好（`isize`），移动派到后台 |
| 单同步互斥 | `sync::run` 同一时刻只允许一个（含强制拉取 / 推送），重入立即返回 `Err("同步正在进行中")` |
| 进程级静态量留在 core | 同步互斥 / 轮次计数、收纳箱 `FenceActivity`（删除中 / 移动中登记）、收纳箱临时几何 `SESSION_RECTS` 与屏幕组合检测起点、通知槽位池、通知窗口真实尺寸、托盘唤起代次 |
| 窗口状态留在 app | 固定 / 嵌入模式标志、Progman owner、`TOP_ON_WAKE` 缓存、`AutoHideState` 实例、收纳箱显示 / 隐藏、嵌入窗口登记 |

## 旧事件 → 返回值

| 旧 Tauri 事件 / 调用 | 现在 |
|---|---|
| `sync-completed`（`SyncReport`，仅 `local_changed()`） | `sync::run` 返回 `SyncReport`；app 在 `report.local_changed()` 时通知各窗口刷新 |
| 同步后 `reload_runtime_prefs`（`sync_cmd.rs:605`） | app 在 `report.settings_applied` 时 `settings::runtime_prefs(db)` 并刷新自己的缓存 |
| 导入后 `reload_runtime_prefs`（`data.rs:245`）+ 前端 `data-imported` | `data::import_json` / `import_from_file` 成功后 app 刷新运行时偏好、重新加载待办与设置 |
| `save_settings` 里的 `reload_runtime_prefs` | `settings::save_window_settings` 后 app 刷新运行时偏好 |
| `fence-items-changed`（`emit_to("fence-{id}")`） | `fence::items::FenceRefresh`：`watch` 的回调、`move_in` / `move_to` 的返回值、`refresh(db, id)` |

## 公开 API

签名里的 `Result<_, String>` 省略为 `R<_>`。"旧命令"列是旧 Tauri 版里对应的 `#[tauri::command]`（历史对照，排查行为差异时用）。

### `db`（原样复制）

| 项 | 说明 |
|---|---|
| `Database::new() -> Result<Database, String>` | 打开 `paths::db_path()`；有待执行迁移时先 `VACUUM INTO` 备份；迁移 v1~v30 |
| `Database::open(path: &Path) -> Result<Database, String>` | 打开指定库（同上） |
| `Database::with_connection(f)` / `with_transaction(f)` | 持锁执行；事务为 `BEGIN IMMEDIATE`，`Err` / panic 回滚 |
| `Database::new_in_memory()` | 测试用（`cfg(test)` 或 feature `test-util`） |
| `db::{Todo, SubTask, CreateTodoRequest, UpdateTodoRequest, CreateSubTaskRequest, UpdateSubTaskRequest, AppSettings, ExportData, WindowPosition, WindowSize, ScreenConfig, SaveScreenConfigRequest, Fence, FenceItem, UpdateFenceRequest, Tombstone}` | 模型（serde camelCase，与旧版 / cloud 一致） |
| `db::{DEFAULT_WINDOW_BG_COLOR, DEFAULT_WINDOW_BG_ALPHA, DEFAULT_FENCE_BG_ALPHA}` | 默认值 |
| `db::paths::{app_data_dir, db_path, images_dir, is_safe_image_name, dev_data_dir_override, DEV_DATA_DIR_ENV}` | 数据目录（debug 构建可用 `MINI_TODO_DATA_DIR` 隔离）；app 的日志目录 / 单实例 / 自启判断也看 `dev_data_dir_override()` |
| `db::time::{now_local, format_canonical, parse_local_datetime, parse_local_datetime_with, normalize_datetime, superseding, DefaultTime, CANONICAL_FORMAT, SQL_SET_UPDATED_AT}` | 统一时间格式 `YYYY-MM-DD HH:MM:SS`（界面解析 / 格式化也用它们，禁止手写） |
| `db::settings_kv`、`db::sync_store`、`db::backup`、`db::migrations::LATEST_VERSION` | 设置键值（只在值变化时刷新 `updated_at`）、同步存储、备份、迁移 |

### `todo`

| 函数 | 说明 | 旧命令 |
|---|---|---|
| `list(db) -> R<Vec<Todo>>` | 全部待办（含子任务）：未完成在前、手动排序、新建的在前 | `get_todos` |
| `get(db, id) -> R<Todo>` | 单条（含按 `sort_order, id` 排序的子任务）；不存在 `"待办不存在"` | `get_todo` |
| `change_seq(db) -> R<i64>` | 本地变更计数（触发器维护），主窗口 5s 轮询，变了才 `list` | `get_change_seq` |
| `create(db, &CreateTodoRequest) -> R<Todo>` | 时间规范化（仅日期：提醒 09:00 / 开始 00:00 / 截止 23:59），排在未完成最后 | `create_todo` |
| `update(db, id, &UpdateTodoRequest) -> R<Todo>` | 只改给出字段；`clear_*` 清空；改提醒重置 `notified`；`updated_at` 严格递增 | `update_todo` |
| `delete(db, id) -> R<()>` | 连带子任务，写墓碑 | `delete_todo` |
| `reorder(db, &[i64]) -> R<()>` | 下标即 `sort_order`，只更新变化的行 | `reorder_todos` |
| `reorder_subtasks(db, &[i64]) -> R<()>` | 同上 | `reorder_subtasks` |
| `create_subtask(db, &CreateSubTaskRequest) -> R<SubTask>` | 排在最后 | `create_subtask` |
| `update_subtask(db, id, &UpdateSubTaskRequest) -> R<SubTask>` | 只改给出字段 | `update_subtask` |
| `get_subtask(db, id) -> R<SubTask>` | | `get_subtask` |
| `delete_subtask(db, id) -> R<()>` | 写墓碑 | `delete_subtask` |
| `images_dir() -> R<PathBuf>` | 图片目录（创建后返回）；Markdown 里存 `minitodo-image://<文件名>` | `get_images_dir` |
| `save_image(bytes: &[u8], ext: &str) -> R<PathBuf>` | 粘贴 / 拖入图片：扩展名白名单 png/jpg/jpeg/webp/gif/bmp（大小写不敏感、可带点）、非空、≤20MB；文件名后端生成 `<毫秒>_<6位>.<ext>`；先写临时文件再改名；返回绝对路径 | `save_subtask_image` |
| `import_subtasks_from_paths(db, parent_id, &[String]) -> R<Vec<SubTask>>` | .md/.txt 文件或文件夹（≤5 层、≤1000 个、单个 ≤10MB、不跟随符号链接目录）；UTF-8/BOM/UTF-16/GBK；任一失败整体取消；单事务插入 | `import_subtasks_from_paths` |

### `data`

| 项 | 说明 | 旧命令 |
|---|---|---|
| `EXPORT_VERSION = "4.0"` | 导出 / 同步文档版本 | |
| `export_json(db) -> R<String>` | pretty JSON（待办 + 子任务 + `AppSettings`） | `export_data` |
| `import_json(db, &str) -> R<()>` | 恢复备份语义：导入前备份到 `backups/`、单事务整体替换、保留原 id、消失的记录写墓碑、`updated_at` 刷新为导入时刻；兼容 3.0 / 4.0。**成功后 app 刷新运行时偏好并重新加载** | `import_data` |
| `export_to_file(db, &Path) -> R<()>` | ZIP（内含 `data.json`，Deflated）；默认文件名由 app 生成 `mini-todo-backup-YYYY-MM-DD.zip` | `export_data_to_file` |
| `import_from_file(db, &Path) -> R<()>` | `PK` 魔数区分 ZIP / JSON，再 `import_json` | `import_data_from_file`（以及旧前端"读 JSON 再 `import_data`"） |
| `read_app_settings(&Connection) -> AppSettings` / `write_app_settings(&Connection, &AppSettings)` | 设置块读写（唯一来源） | |
| `settings_version(&Connection)` | 同步设置版本（re-export） | |

### `sync`

| 项 | 说明 | 旧命令 |
|---|---|---|
| `SyncSettings { webdav_url, webdav_username, webdav_password, has_password, auto_sync, sync_interval, last_sync_at, device_id, clear_password }` | 读取时 `webdav_password` 恒空；保存时空 = 保留、`clear_password` = 清空 | |
| `get_settings(db) -> R<SyncSettings>` | 设备 id 不存在时生成 | `get_sync_settings` |
| `save_settings(db, &SyncSettings) -> R<()>` | 密码 Windows 上 DPAPI 加密；换服务器 / 账号清基准与 `synced_seq`；间隔至少 1 | `save_sync_settings` |
| `test_connection(db, url, username, password: Option<&str>) -> R<bool>` | 密码为空用已保存的 | `webdav_test_connection` |
| `enum SyncMode { Smart { full: bool }, ForcePull, ForcePush }` | `full: true` = 标题栏按钮 / 设置页"立即同步"（无条件 GET）；自动同步传 `false`（每第 10 轮自动无条件） | |
| `run(db, SyncMode) -> R<SyncReport>` | 单同步互斥（`"同步正在进行中"`）；未配置 `"未配置 WebDAV 服务器"`；返回后见"旧事件 → 返回值" | `webdav_sync` / `webdav_force_pull` / `webdav_force_push` |
| `SyncReport { status, last_sync_at, todos_*/subtasks_* inserted/updated/deleted, records_skipped, images_uploaded/downloaded, settings_applied }`、`SyncReport::local_changed()`、`SyncStatus { NoChanges, Pulled, Pushed, Merged }` | 结果（界面提示文案按旧 `utils/syncReport.ts`） | |
| `FULL_GET_EVERY = 10`、`SyncData` | 常量 / 同步文档结构 | |

### `settings`

| 函数 | 说明 | 旧命令 |
|---|---|---|
| `get_setting(db, key) -> R<Option<String>>` / `set_setting(db, key, value) -> R<()>` | 白名单（只有 `view_mode`：`list` / `quadrant`），其它键报错 | `get_setting` / `set_setting` |
| `view_mode(db) -> String` / `set_view_mode(db, &str) -> R<()>` | 便捷版，默认 `"list"` | 同上 |
| `system_fonts() -> R<Vec<String>>` | DirectWrite 枚举（排序去重；非 Windows 空） | `get_system_fonts` |
| `font_family(db) -> R<String>` / `set_font_family(db, &str)` | 默认空串 | `get_todo_font_family` / `set_todo_font_family` |
| `font_size(db) -> R<i32>` / `set_font_size(db, i32)` | 默认 14，写入夹到 12~20 | `get_todo_font_size` / `set_todo_font_size` |
| `notification_type(db) -> R<String>` / `set_notification_type(db, &str)` | `system`（默认）/ `app`，其它值存 `system` | `get_notification_type` / `set_notification_type` |
| `struct RuntimePrefs { top_on_wake, auto_hide_enabled }`、`runtime_prefs(db) -> RuntimePrefs` | 轮询要用的缓存值（原 `reload_runtime_prefs` 读取的两个键，默认都 true） | （`reload_runtime_prefs`） |
| `save_window_settings(db, &AppSettings) -> R<()>` | 主窗口保存状态：一个事务写 9 个键，值不变不刷新 `updated_at`；调用方先读真值；**设置窗口里绝不调用**；之后刷新运行时偏好 | `save_settings` |
| `text_theme(db) -> R<String>` / `set_text_theme(db, &str)` | 默认 `"dark"`；`"light"` = 深色主题开启（浅色文字） | `get_text_theme` / `set_text_theme` |
| `fixed_embed_desktop(db) -> bool` / `set_fixed_embed_desktop(db, bool)` | 只写键；app 随后按需切换（Windows：已经典固定且开 → 嵌入；已嵌入且关 → 经典固定） | `get_fixed_embed_desktop` / `set_fixed_embed_desktop` |
| `auto_hide_enabled(db) -> bool` / `set_auto_hide_enabled(db, bool)` | 只写键；状态见 `window_logic::AutoHideState::set_enabled` | `get_auto_hide_enabled` / `set_auto_hide_enabled` |
| `top_on_wake(db) -> bool` / `set_top_on_wake(db, bool)` | 只写键；关闭时 app 立即撤置顶 | `get_top_on_wake` / `set_top_on_wake` |
| `struct WindowBackground { color, alpha }`、`window_background(db)` / `set_window_background(db, color, alpha)` | 默认 `#000000` / 0.45；alpha 夹到 0~1 | `get_window_background` / `set_window_background` |
| `show_calendar(db) -> R<bool>` / `set_show_calendar(db, bool)` | 默认 false | `get_show_calendar` / `set_show_calendar` |
| `screen_config_id_of(&[(w, h, scale)]) -> String` | 屏幕组合 ID `"{n}_{W}x{H}@{scale%}_…"`（面积降序、稳定排序；无显示器 `"unknown"`）。输入是**物理像素**与缩放，按 `EnumDisplayMonitors` 顺序 | （前端 `generateScreenConfigId`、`fence.rs::screen_config_id_of`） |
| `screen_config_id(&[(w, h, scale)]) -> String` | 同上，debug 构建下 `MINI_TODO_SCREEN_ID_FILE` 可覆盖（主窗口与收纳箱都用它） | （`current_screen_config_id`） |
| `screen_config_display_name(&str) -> String` | `"2屏: 3840x2160 (150%) + 1920x1080 (100%)"`；`unknown` → 未知配置，`legacy` → 旧版配置 | （前端 `generateScreenConfigDisplayName`） |
| `UNKNOWN_SCREEN_CONFIG` | `"unknown"` | |
| `get_screen_config(db, id) -> R<Option<ScreenConfig>>` | | `get_screen_config` |
| `save_screen_config(db, &SaveScreenConfigRequest) -> R<ScreenConfig>` | 按 `config_id` upsert（`display_name: None` 保留旧值） | `save_screen_config` |
| `list_screen_configs(db) -> R<Vec<ScreenConfig>>` | `updated_at DESC` | `list_screen_configs` |
| `delete_screen_config(db, id) -> R<()>` | 连带删除该组合的 `fence_layouts` | `delete_screen_config` |
| `update_screen_config_name(db, id, name) -> R<()>` | | `update_screen_config_name` |

### `holiday`

| 项 | 说明 | 旧命令 |
|---|---|---|
| `HolidayInfo { date, name, is_off_day }` | `is_off_day = false` 表示调休上班 | |
| `fetch(year) -> R<Vec<HolidayInfo>>` | 网络优先（`raw.githubusercontent.com/NateScarlet/holiday-cn/master/{year}.json`，超时 10s / 连接 5s），成功写缓存 `<数据目录>/holidays/{year}.json`；失败回退缓存 | `fetch_holidays` |

### `lunar`（新增）

| 项 | 说明 | 旧前端 |
|---|---|---|
| `LunarInfo { day_text, month_text, year_text, solar_term, festival, is_first_day, full_text }` | 农历日 / 月（闰月带"闰"，冬月 / 腊月）/ 干支年（正月初一换年）/ 当天交节的节气 / 传统节日（春节、元宵节、龙头节、端午节、七夕节、中秋节、重阳节、腊八节、除夕）/ 是否初一 / `二〇二四年正月初一` | `getLunarInfo` |
| `lunar_info(NaiveDate) -> LunarInfo` | 公历 1~9998 年；超出范围返回空 | `getLunarInfo` |
| `enum LunarTextKind { Festival, SolarTerm, Lunar }`、`lunar_display_text(NaiveDate) -> (String, LunarTextKind)` | 优先级：节日 > 节气 > 农历日（初一显示月份） | `getLunarDisplayText` |

### `reminder`

调度循环在 app：

```text
sleep(STARTUP_DELAY) → catch_up_missed_repeats(db, deliver)
loop { sleep(duration_until_next_minute(SystemTime::now())) → kind = notification_kind(db) → run_due(db, deliver) }
deliver(&Reminder) -> Result<(), String>：
  System → 系统通知（标题 reminder.title，正文 reminder.body()）
  App    → 应用内通知窗口：label = next_window_label()，slot = NOTIFICATION_SLOTS.acquire()，
           位置 = notification_position(工作区（逻辑像素）, last_actual_size().unwrap_or(REQUESTED_SIZE), slot)，
           建窗失败时 NOTIFICATION_SLOTS.release(slot) 并返回 Err；窗口关闭时 release(slot)；
           拿到真实尺寸后 fit_to_actual_size(...) 返回 Some(新位置) 就挪过去
```

| 项 | 说明 | 旧代码 |
|---|---|---|
| `Reminder { id, title, description, repeat_* , notify_at }`、`Reminder::body() -> &str` | 待发送的提醒；正文缺省 `DEFAULT_BODY = "待办事项提醒"` | `PendingNotification` |
| `enum NotificationKind { System, App }`、`notification_kind(db)` | 每轮重新读（`app` → App，其它 / 读失败 → System） | `get_notification_type` |
| `run_due(db, &mut dyn FnMut(&Reminder) -> R<()>) -> R<()>` | 到期（扣除提前量）未完成未通知，按时间排序逐条投递；回调 `Ok` 才推进（重复 → 下一次，否则 `notified = 1`）；`Err` 只记日志、下一分钟重试；单条出错不影响其它 | `check_and_send_notifications` + `deliver` |
| `catch_up_missed_repeats(db, deliver) -> R<()>` | 启动补发已过点的重复提醒 | 同名 |
| `duration_until_next_minute(SystemTime) -> Duration` | 对齐整分钟 | `sleep_until_next_minute` |
| `STARTUP_DELAY = 5s` | 首次检查前的等待 | |
| `SlotPool { acquire, release }`、`NOTIFICATION_SLOTS` | 堆叠槽位（取最小空闲） | `acquire_slot` / `release_slot` |
| `WorkArea { x, y, width, height }`（逻辑像素）、`FALLBACK_WORK_AREA` | 主显示器工作区（GPUI `visible_bounds()` 即可） | `primary_work_area` |
| `NotificationSize { width, height }`、`::from_physical`、`::differs_from`、`REQUESTED_SIZE`（320×120） | | |
| `notification_position(area, size, slot) -> (f64, f64)` | 右下角起向上堆叠、满列向左换列、全满取模 | 同名 |
| `last_actual_size()` / `remember_actual_size()` / `fit_to_actual_size(area, slot, placed_for, actual) -> Option<(f64, f64)>` | 按真实尺寸校正 | `fit_to_actual_size` |
| `next_window_label() -> String` | `notification_{n}` | `NOTIFICATION_COUNTER` |
| `NOTIFICATION_WIDTH/HEIGHT/MARGIN/SPACING` | 320 / 120 / 20 / 10（逻辑像素） | |

### `window_logic`

app 持有 `AutoHideState` 与 `TOP_ON_WAKE: AtomicBool`，200ms 前台定时器里喂入 Win32 读到的值。

| 项 | 说明 | 旧代码 |
|---|---|---|
| `AutoHideState`（pub 字段）、`DockEdge`、`MonitorBounds`（整块显示器，物理像素）、`WindowRect`（外框，物理像素）、`AutoHideTransition { None, Hide {..}, Restore { anchor } }`、`WindowPersistState` | 贴边隐藏状态机（仅经典固定模式） | `AUTO_HIDE_STATE` 等 |
| `evaluate_auto_hide_transition(&mut state, rect, monitor, cursor: Option<(i32, i32)>, now) -> AutoHideTransition` | 每 tick 调用；`Hide` → 移到 `hidden` 并撤置顶（失败回滚状态）；`Restore` → 移回 `anchor`，`top_on_wake` 时置顶（失败把 `hidden` 设回 true） | `tick_auto_hide` 的判断部分 |
| `apply_runtime_prefs(&mut state, &AtomicBool, top_on_wake, auto_hide_active) -> bool` | 刷新偏好；返回 true = 有功能被关掉，下个 tick 先收尾（`take_disabled_hidden_anchor` 拉回 + 撤置顶） | `reload_runtime_prefs` |
| `take_disabled_hidden_anchor(&mut state) -> Option<WindowPosition>` | 关掉后仍藏着的窗口拉回锚点（只一次） | `release_auto_hide` |
| `AutoHideState::enter_fixed_mode(auto_hide_enabled) -> Option<WindowPosition>` | 进入 / 重复进入经典固定；返回要拉回的锚点 | `set_window_fixed_mode(true)` |
| `AutoHideState::leave_fixed_mode() -> Option<WindowPosition>` | 退出经典固定（回普通或切到嵌入）；先撤置顶再调 | `set_window_fixed_mode(false)` / `set_window_desktop_mode(true)` |
| `AutoHideState::set_enabled(active) -> Option<WindowPosition>` | 设置页开关（`active` = 开且经典固定）；关闭时返回锚点，app 拉回并撤置顶 | `set_auto_hide_enabled` |
| `AutoHideState::wake_from_tray(now) -> Option<WindowPosition>` | 托盘唤起：3s 免隐藏宽限，藏起时返回锚点 | `bring_main_window_to_front` |
| `AutoHideState::persist_state(position, size) -> WindowPersistState` | 隐藏中返回锚点几何 | `get_window_persist_state` |
| `AutoHideState::should_stay_on_top(top_on_wake) -> bool` / `clear_runtime()` | | `should_stay_on_top` / `clear_auto_hide_runtime_state` |
| `begin_tray_wake() -> u64` / `is_latest_tray_wake(u64) -> bool`、`TRAY_WAKE_TOPMOST_DURATION`（600ms） | 唤起置顶 600ms 后只由最新一次撤销（且 `!should_stay_on_top`） | 同名 |
| `accept_tray_click(&Mutex<Option<Instant>>, now) -> bool`、`TRAY_CLICK_DEBOUNCE`（300ms） | 托盘左键去抖 | `lib.rs` 同名 |
| `needs_reattach(cached_owner, owner_alive, shell_now, current_owner) -> bool` | 嵌入桌面（主窗口 / 收纳箱）Explorer 重启后是否重挂 owner | 同名 |
| `lacks_tool_window_style(ex_style: u32) -> bool`、`EX_TOOLWINDOW_BIT` / `EX_APPWINDOW_BIT` | 样式自愈（GPUI 下为防御） | 同名 |
| `is_desktop_host_class(&str) -> bool` | `GetShellWindow()` 类名是否 `Progman`（不是只记日志） | 同名 |
| `reset_window_position(monitor, scale) -> WindowPosition`、`RESET_WINDOW_WIDTH/HEIGHT`（380×600 逻辑） | 托盘「重置位置」：主显示器左上起 10% 边距 | `reset_window_impl` |
| `EDGE_SNAP_THRESHOLD_PX` 等常量 | 10px / 420ms / 3s / 600ms / 4px / 2px / 40px | |

不在 core（app 的 Win32 平台层 `crates/app/src/platform/` 与 `services/window_mode.rs`；括号里是旧版 `src-tauri/src/commands/window.rs`
删除前的行号）：`apply_fixed_ex_style_hwnd`（L829）、
`desktop_host` / `desktop_attach_hwnd` / `desktop_detach`（L1013-1106）、`win32_set_topmost`（L861）、`win32_show`（L894）、
`set_window_bounds_physical`（L1293）、`tick_embedded_windows`（L1336）、`restore_if_minimized`（L618）、
DWM 圆角（`lib.rs` L55）、`MessageBoxW` 致命错误（`lib.rs` L139）；以及 tao `always_on_bottom` 的替代（`WM_WINDOWPOSCHANGING` 子类化）。

### `fence`（收纳箱元数据与几何）

几何一律**物理像素**。显示器信息（工作区、缩放、屏幕组合 ID）由 app 用 Win32 取（`EnumDisplayMonitors` + `GetMonitorInfoW` + `GetDpiForMonitor`），
主显示器排第一，工作区取不到时用整块显示器。

| 项 | 说明 | 旧命令 / 函数 |
|---|---|---|
| `PxRect { x, y, width, height }`、`MonitorArea { work, scale }`、`stored_rect(&Fence)` | | |
| `fence_view(stored, expanded, &[MonitorArea]) -> (PxRect, f64)` | 窗口应占矩形与目标显示器缩放：折叠 = 标题栏高 `round(32 × 缩放)`；展开下方不够时整体上移；夹回标题栏所在工作区 | `fence_view`（`apply_fence_view` / `set_fence_view` / 建窗用它） |
| `title_bar_px(scale)`、`default_fence_size(scale)`、`view_rect`、`pick_work_area`、`clamp_into`、`title_scale`、`default_fence_rect`、`reset_layout`、`layout_after_switch`、`committed_rect`、`effective_rect`、`next_default_title`、`normalize_title`、`normalize_color` | 纯几何 / 命名 | 同名 |
| `fence_label(id)` / `is_fence_label(&str)`、`FENCE_LABEL_PREFIX` | `fence-{id}` | 同名 |
| `FENCE_TITLE_BAR_DIP`（32）、`MIN_WIDTH_DIP` / `MIN_EXPANDED_HEIGHT_DIP`（160）、`CELL_*`、`MIN_ICON_SIZE` / `MAX_ICON_SIZE`（24~128）、`MAX_TITLE_CHARS`（40）、`FALLBACK_WORK_AREA` 等 | 常量 | |
| `Placement::for_screen(db, screen_id)`、`.rect(&Fence)`、`.screen_id` | 该组合下的存储几何：组合记录 → 本次运行临时几何 → 最近一次位置 | `Placement` |
| `list(db) -> R<Vec<Fence>>` / `get(db, id) -> R<Fence>` | | `list_fences` / `get_fence` |
| `update(db, id, &UpdateFenceRequest) -> R<Fence>` | 折叠 / 锁定 / 背景色（`#RRGGBB` 大写）/ 透明度（0~1）/ 图标大小（24~128）/「显示隐藏的图标」（`show_hidden`，v30）；只写库 | `update_fence` |
| `rename(db, id, title) -> R<RenameFenceResult { fence, folder_error }>` | 改标题 + 改收纳文件夹（失败保留旧名并给原因）；app 随后改窗口标题 | `rename_fence` |
| `create(db, work, scale, screen_id) -> R<Fence>` | 光标所在显示器工作区内右上角起找空位、默认标题、先建文件夹再插库、记组合位置；`Fence` 的 x/y/w/h 即存储几何。app：隐藏中先全部显示 → 建窗 → 失败 `rollback_create(db, &fence)` → 成功后确保监听 | `create_fence` / 托盘「新增收纳箱」（`create_fence_at_cursor`） |
| `rollback_create(db, &Fence)` | 删记录 + 删空文件夹 | |
| `prepare_restore(db, screen_id) -> R<Vec<(Fence, PxRect)>>` | 启动恢复：确保文件夹存在、给出各自的存储几何、记屏幕组合检测起点。app：逐个建窗 → 有收纳箱则 `items::watch` → `items::spawn_startup_reconcile` | `restore_fences` |
| `commit_geometry(db, id, screen_id, window_outer: PxRect, expanded_view) -> R<Fence>` | 拖动 / 缩放结束落库（折叠视图保留展开高度），写组合记录 + 最近位置 | `commit_fence_geometry` |
| `reset_positions(db, primary_work, scale, screen_id) -> R<Vec<(Fence, PxRect)>>` | 托盘「重置收纳箱位置」：排开并落库；app 摆窗并全部显示 | `reset_fence_positions` / `reset_all_fence_positions` |
| `observe_screen_config(screen_id) -> Option<String>` / `forget_screen_config()` | 每 ~2s：有收纳箱窗口时传当前 ID，组合变化时返回上一次的 ID；没有窗口时 `forget` | `tick_fence_screen_layouts` |
| `screen_switch(db, previous, current, &[(id, Option<PxRect>, scale)]) -> Vec<(Fence, PxRect)>` | 组合变化后的新存储几何（有记录用记录，否则沿用窗口当前位置、只记内存）；app 再按 `fence_view(.., !fence.collapsed, ..)` 摆窗 | `apply_screen_switch` |
| `delete(db, id, owner_hwnd: Option<isize>) -> R<DeleteFenceResult { deleted, restored, remaining, failures }>` | 先全部移回原位置；文件夹清空并删除才删记录（`deleted = true` → app 销毁窗口）；否则整个保留、删掉已移回项的行，app 用 `items::refresh` 刷新窗口。**后台线程** | `delete_fence` |

app 层负责（`crates/app/src/services/fences.rs`、`windows/fence/`；行号指旧版 `src-tauri/src/commands/fence.rs`）：建窗参数（L833-898：无边框透明、不可缩放 / 最小化、`show:false` → 嵌入 → 显示、
关闭拦截）、`set_fence_bounds`（缩放手柄逐帧 `SetWindowPos`，不落库）、`set_fence_view`（`Placement::for_screen(..).rect(..)` → `fence_view` → 摆窗）、
显示 / 隐藏全部（`FENCES_HIDDEN` 运行时状态，不持久化）、嵌入窗口登记与 200ms 守护。

### `fence::items`（收纳箱内容）

| 项 | 说明 | 旧命令 |
|---|---|---|
| `FenceEntry`、`ItemOpResult`、`ItemOpStatus`、`is_auto_hidden_name`（re-export 自 `services::fence_fs`） | 项目 / 逐项结果；`FenceEntry.hidden`（用户隐藏，来自 `fence_items.hidden`）/ `auto_hidden`（以 `.` 开头）/ `is_hidden()` | |
| `FenceRefresh { fence_id, items }` | 对账后的完整列表（含隐藏项，app 按 `Fence.show_hidden` 过滤显示），交给对应窗口 | `fence-items-changed` |
| `get_items(db, id) -> R<Vec<FenceEntry>>` | 按文件夹对账后返回；不存在或删除中报错 | `get_fence_items` |
| `reconcile(db, id) -> R<Option<Vec<FenceEntry>>>` / `refresh(db, id) -> Option<FenceRefresh>` | 对账（数据库锁内列举文件夹；删除中返回 `None`） | `reconcile_fence` / `reconcile_and_notify` |
| `move_in(db, id, &[String], owner_hwnd) -> R<(FenceMoveInResult { results, onedrive_notice }, Vec<FenceRefresh>)>` | 拖入（文件 / 快捷方式 / 文件夹）；跨箱来源搬运原始路径并返回来源箱刷新；`onedrive_notice`（云同步提示：Windows 是从被 OneDrive 重定向的桌面移入，macOS 是移动前由 iCloud / File Provider 管理的项目）只会为 true 一次。**后台线程** | `fence_move_in` |
| `restore(db, id, file_name, owner_hwnd) -> R<ItemOpResult>` | 移回原目录（不存在则桌面），重名改名。**后台线程** | `fence_restore_item` |
| `move_to(db, id, file_name, target_id, owner_hwnd) -> R<(ItemOpResult, Option<FenceRefresh>)>` | 跨箱移动（搬运原始路径、不搬运隐藏标记），返回目标箱刷新。**后台线程** | `fence_move_item` |
| `open(db, id, file_name) -> R<()>` / `reveal(db, id, file_name) -> R<()>` | Windows：`ShellExecuteExW` / 资源管理器定位；macOS：`NSWorkspace` 打开 / 在访达中显示（其他平台报"仅支持 Windows 与 macOS"） | `fence_open_item` / `fence_reveal_item` |
| `icon(db, id, file_name, size_px) -> R<PathBuf>` | 图标 PNG 缓存路径（档位向上取整，`fence-icons/`），GPUI 用 `img(path)` | `fence_item_icon` |
| `reorder(db, id, &[String]) -> R<()>` | 箱内排序（完整顺序，含隐藏项） | `reorder_fence_items` |
| `set_hidden(db, id, file_name, hidden) -> R<FenceRefresh>` | 右键「隐藏」/「取消隐藏」（v30）：数据库锁内先对账再写 `fence_items.hidden`（文件名不区分大小写），返回最新完整列表；不改文件属性；删除中 / 不存在 / 找不到该项报错 | （GPUI 版新增） |
| `watch(Arc<Database>, impl Fn(FenceRefresh) + Send + 'static) -> R<FenceWatcher>` | 监听收纳根目录（300ms 去抖），回调在 `fence-watch` 线程，经 channel 回前台；app 持有句柄（drop 即停）；首个收纳箱出现后再开 | `ensure_fence_watcher` / `on_fence_change` |
| `spawn_startup_reconcile(Arc<Database>)` | 启动对账 + 20s 后清理图标缓存（独立线程） | 同名 |
| `ONEDRIVE_NOTICE_KEY` | 设备本地 settings 键（不同步、不导出） | |

### `services`（原样复制）

| 模块 | 主要项 |
|---|---|
| `services::webdav` | `WebDavClient`（blocking；条件 GET/PUT、PROPFIND、MKCOL）——同步引擎内部使用 |
| `services::secret` | `protect` / `reveal` / `is_protected`（Windows DPAPI，`dpapi:<base64>`；其它平台明文） |
| `services::fence_fs` | 收纳箱文件层：`fence_root_dir`、`icon_cache_dir`、`user_desktop_dir`、`create_fence_folder`、`ensure_fence_folder`、`rename_fence_folder`、`remove_fence_folder_if_empty`、`fence_folder_path`、`list_fence_items`（点文件照常列出、`auto_hidden`；带隐藏 / 系统属性（Windows 属性、macOS `UF_HIDDEN`）的与 `desktop.ini` / `.DS_Store` 等系统元数据文件跳过）、`is_auto_hidden_name`、`list_fence_folders`、`move_into_fence`、`restore_item`、`move_between_fences`、`restore_all`、`launch_item`、`reveal_in_explorer`、`icon_png`、`prune_icon_cache`、`watch_fence_root`、`FenceWatcher`、`FenceChange`、`is_ubiquitous_item`（macOS iCloud 判定）等。移动 / 打开 / 定位 / 图标：Windows 走 Shell（`shell.rs` / `sta.rs`），macOS 走 `mac.rs`（`renamex_np(RENAME_EXCL)` 撞名顺延 + 跨卷 `NSFileManager`、`NSWorkspace`；规划时不列举收纳根目录以外的目标目录，避免隐私授权框），其他平台报错。debug 下 `MINI_TODO_FENCES_ROOT` 覆盖收纳根目录、`MINI_TODO_DESKTOP_DIR` 覆盖用户桌面 |

## 不在 core 的旧命令

`sync_auto_start_state`、`set_window_desktop_mode`、`set_window_fixed_mode`、`set_auto_hide_cursor_inside`、`get_window_persist_state`（几何由 app 读，
锚点替换用 `AutoHideState::persist_state`）、`reset_window`、`close_notification_window`、`close_all_notification_windows`、`set_fence_view`、
`set_fence_bounds`、`toggle_fences_visibility`：都是纯窗口 / 平台操作，由 app 实现（core 提供其中的判断与几何）。
