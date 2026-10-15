# Research: GPUI 重写 —— 后端（pc/src-tauri/src）移植地图

- **Query**：用 Rust + GPUI（gpui-pre 0.3.9 + gpui-kit 0.7.3）重写 `pc/` 的 Tauri 2 + Vue 桌面端；后端业务逻辑尽量原样复用，
  只替换与 Tauri 耦合的部分。输出逐文件耦合清单、命令 / 事件清单、启动流程、window.rs 详解、通知 / 收纳箱 / 同步等模块边界、
  新 crate 结构与平台 crate 选型、测试迁移方案。
- **Scope**：mixed —— 通读 `pc/src-tauri/src/**`、`Cargo.toml`、`tauri.conf.json`、`capabilities/`、`.trellis/spec/backend/*.md`、
  前端调用方（`pc/src/**`）；GPUI 行为以本机 cargo registry 中 `gpui-pre-0.3.9` / `gpui-pre-windows-0.3.9` / `gpui-kit-0.7.3`
  源码核实（路径 `~/.cargo/registry/src/index.crates.io-*/`）。
- **Date**：2026-10-15
- 行号均指当前仓库文件（`pc/src-tauri/src/...`），命令行号指 `pub fn` / `pub async fn` 所在行（`#[tauri::command]` 在其上一行）。

---

## 0. 摘要（先读这里）

| # | 结论 |
|---|---|
| 1 | `db/**`（9 个文件）、`services/webdav.rs`、`services/secret.rs`、`services/fence_fs/**`（10 个文件）**完全不依赖 Tauri**，可原样复制到 core crate；它们的 `#[cfg(test)]` 测试也原样可跑（`Database::new_in_memory` 是 `#[cfg(test)]`，见 §10）。 |
| 2 | 共 **80 个** `#[tauri::command]`（todo 15、window 24、settings_cmd 9、data 4、holiday 1、notification_cmd 2、fence 11、fence_items 8、sync_cmd 6）。按移植难度分三组：**51 个**函数体只依赖 `Database`（或什么都不依赖），去掉 `#[tauri::command]` / `State` 外壳即成普通函数；**6 个**是纯逻辑但返回后需要 app 层收尾（`webdav_sync` / `webdav_force_pull` / `webdav_force_push` 广播 `sync-completed`；`import_data` / `import_data_from_file` / `save_settings` 刷新运行时偏好缓存）；**23 个**真正操作窗口 / 平台（window.rs 9：`sync_auto_start_state`、`set_window_desktop_mode`、`set_window_fixed_mode`、`set_fixed_embed_desktop`、`set_auto_hide_cursor_inside`、`set_auto_hide_enabled`、`set_top_on_wake`、`get_window_persist_state`、`reset_window`；fence.rs 8：`create_fence`、`rename_fence`、`set_fence_view`、`set_fence_bounds`、`commit_fence_geometry`、`delete_fence`、`toggle_fences_visibility`、`reset_fence_positions`；fence_items 3 个需要 owner HWND 与刷新回调：`fence_move_in`、`fence_restore_item`、`fence_move_item`；notification_cmd 2；`save_subtask_image` 的 `tauri::ipc::Request` 入参 1）。前端从未调用的命令 7 个（§2.10）。 |
| 3 | 业务层对窗口层有 **2 处反向依赖必须断开**：`data.rs:245` 与 `sync_cmd.rs:605` 直接调用 `crate::commands::window::reload_runtime_prefs`（刷新轮询线程用的 `TOP_ON_WAKE` / 贴边隐藏开关缓存）。core 中应删掉这两处调用，由 app 层在"导入成功后"与"`SyncReport.settings_applied == true` 时"自行刷新（§1.2、§8）。 |
| 4 | 后端只主动发出 **6 种事件**：托盘 4 个（`tray-toggle-fixed` / `tray-reset-window` / `tray-add-todo` / `tray-open-settings`，payload `()`）、`sync-completed`（`SyncReport`，仅 `local_changed()` 时）、`fence-items-changed`（`Vec<FenceEntry>`，`emit_to("fence-{id}")`）。其余 6 种（`app-settings-changed`、`todo-font-changed`、`data-imported`、`subtask-memory-*` ×3）是前端窗口间互发。单进程 GPUI 下全部可改为 Entity 事件 / 共享状态（§3）。 |
| 5 | window.rs 里大量逻辑是为了对抗 **tao 0.34 `WindowFlags::apply_diff` 整体重写 `GWL_EXSTYLE`** 与 `run_on_main_thread` 排队语义（§5.3 列了 13 处）。**核实 gpui-pre-windows 0.3.9 源码**：GPUI 在建窗后**从不写 `GWL_EXSTYLE`**（只有 `toggle_fullscreen` 写 `GWL_STYLE`），因此这类"排队重申 / 样式自愈"在 GPUI 下不再是必需（可保留为廉价防御）；但反过来 GPUI **没有** `set_position` / `set_always_on_top` / `set_always_on_bottom` / 运行时 `set_resizable` / `set_minimizable` / `show` / `hide`，也**不处理 `WM_WINDOWPOSCHANGING`** —— tao 的 `ALWAYS_ON_BOTTOM` 钩子（嵌入桌面的 Z 序基石）必须自己用窗口子类化实现（§5.2-F、§5.4）。 |
| 6 | GPUI `PlatformDisplay` 只暴露逻辑像素的 `bounds()` / `visible_bounds()`，**没有缩放比例**，混合 DPI 下逻辑坐标不连续。现有几何全部按**物理像素**存库（`screen_configs`、`fences`、`fence_layouts`），屏幕组合 ID 需要"物理宽 × 高 + 缩放%"（`screen_config_id_of`）。→ app 层需要一个 Win32 显示器模块（`EnumDisplayMonitors` + `GetMonitorInfoW(rcMonitor/rcWork)` + `GetDpiForMonitor`）与 Win32 定位（`SetWindowPos`），不要用 GPUI 的逻辑坐标做持久化（§5.2-Q、§7.1）。 |
| 7 | 通知调度器的纯逻辑（到期 SQL、逐条投递 / 推进、重复提醒计算、槽位、定位算法）全部可复制，18 个测试随之迁移；只需把 `tauri::async_runtime::spawn + tokio::time::sleep` 换成线程 / GPUI 定时器，把"发系统通知 / 开应用内通知窗口"抽成回调（§6）。系统通知建议用 **notify-rust 4.x**（即当前 `tauri-plugin-notification 2.3.3` 在 Windows 上实际走的 `notify-rust 4.11.7 → tauri-winrt-notification 0.7.2` 链路，可在后台线程调用、`show()` 返回 `Result`），GPUI 自带的 `App::show_system_notification` 可作备选但只能在主线程调用、无错误返回、强制静音（§6.4）。 |
| 8 | `holiday.rs` 用的是 **async reqwest**（需要 tokio reactor）；GPUI 的执行器不是 tokio。core 中应改为 `reqwest::blocking` 并在后台线程执行（`webdav.rs` 已是 blocking，不受影响）。core 不再需要 `tokio` 依赖（§8）。 |
| 9 | 自动同步定时器、主窗口位置 / 尺寸保存与恢复、屏幕配置 ID 生成、固定模式的进入顺序，目前**在前端**（`MainView.vue`、`stores/appStore.ts`）；移植时这部分逻辑要从 TS 搬到 app 层（§4.9、§5.2-O）。 |
| 10 | `src-tauri` 共 **271 个** `#[test]`：db 54、services 98（fence_fs 62）、commands 118、lib 1。除 `todo.rs` 中 1 个直接构造 `tauri::ipc::InvokeBody` 的测试需改写外，全部是纯逻辑 / 内存库 / tiny_http mock，可随模块进入 core（§10）。 |

---

## 1. 逐文件耦合清单

### 1.1 总表

结论图例：**A 原样复制**（无 Tauri 依赖）；**B 去壳复制**（函数体纯逻辑，只需去掉 `#[tauri::command]` / `State` / `AppHandle` 外壳）；
**C 需改写**（逻辑依赖 Tauri 窗口 / 运行时）；**D 替换为其它 crate**；**X 删除**（GPUI 下无意义）。

| 文件 | 行数 | Tauri / 插件 API（行号） | 其它平台依赖 | 结论 |
|---|---|---|---|---|
| `main.rs` | 6 | `mini_todo_lib::run()`（L5）；`#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]`（L2） | — | C：改为 GPUI `main`，保留 `windows_subsystem` 属性 |
| `build.rs` | 3 | `tauri_build::build()` | — | D：exe 图标 / 版本资源改用 `embed-resource` 或 `winresource`；DPI / Common-Controls 清单由 gpui-pre 默认 feature `windows-manifest` 提供（§5.4） |
| `lib.rs` | 598 | `tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem}` L9；`tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent}` L10；`tauri::{Emitter, Manager}` L11；`tauri_plugin_autostart::ManagerExt` L12；`tauri::WebviewWindow` L56/L80；`tauri::webview::Color` L81；`tauri_plugin_log::{RotationStrategy, Target, TargetKind, TimezoneStrategy}` L101-116；`tauri::Builder` L188；`tauri_plugin_single_instance::init` L194；`plugin(...)` L208-216；`app.asset_protocol_scope().allow_directory` L230；`app.manage` L241/L243；`app.get_webview_window("main")` L245/348/354/358/364/382；`app.autolaunch()` L272/L390；`TrayIconBuilder` L336-426；`app.default_window_icon()` L342；`window.emit` L349/359/365/385；`app.exit(0)` L407；`tray.app_handle()` L422；`app.handle()` L429/433/437；`generate_handler!` L474-569；`generate_context!()` L570；`.run(\|_, _\| {})` L572 | Win32：`DwmSetWindowAttribute(DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND)` L55-77；`MessageBoxW` L139-156；`raw_window_handle` L57 | C（整体改写）。可原样复制的片段：`accept_tray_click` L45-52（+测试）、`open_database_or_exit` L123-137、`show_fatal_error` L139-162、圆角函数的 Win32 主体 |
| `commands/mod.rs` | 19 | 仅 `pub mod` + `pub use *` | — | X（由 core / app 的模块树取代） |
| `commands/data.rs` | 863 | `use tauri::{AppHandle, Manager}` L19；`run_blocking` L319-330（`tauri::async_runtime::spawn_blocking` + `app.state::<Database>()`）；4 个命令 L332-350；**`crate::commands::window::reload_runtime_prefs(db)` L245** | `zip` / `chrono` / `serde_json` | B：除 `run_blocking`、4 个命令外壳与 L245 外全部纯逻辑 |
| `commands/fence.rs` | 2112 | `use tauri::{AppHandle, LogicalSize, Manager, PhysicalPosition, PhysicalSize, State, WebviewUrl, WebviewWindow, WebviewWindowBuilder}` L33-36；`tauri::Monitor`（`work_area/position/size/scale_factor`）L724-762；`app.available_monitors()` L751/L770；`app.primary_monitor()` L756/L960；`app.cursor_position()` + `monitor_from_point` L957-959；`app.get_webview_window` / `webview_windows()` L708-718；`WebviewWindowBuilder` L857-873；`set_position/set_size/set_min_size` L876-877、L910-919；`on_window_event(CloseRequested/Destroyed)` L884-890；`show()/hide()` L894、L930-934；`set_title` L1275；`outer_position/outer_size/scale_factor` L1204-1213、L1341-1342；`destroy()` L1437；`tauri::async_runtime::spawn` L1040、L1051；`app.state::<Database>()` / `handle.state` L948/1053/1092/1179/1266/1420；`crate::setup_window_rounded_corners` L880 | 无直接 Win32（经 window.rs） | 拆分：L103-444 几何 / 命名纯函数、L446-693 数据库、`plan_delete` L1386-1407、`rename_fence_and_folder` L540-585 → **A**；窗口 / 显示器相关 L695-1232 与命令外壳 → **C** |
| `commands/fence_items.rs` | 1123 | `use tauri::{AppHandle, Emitter, Manager, State, WebviewWindow}` L29；`blocking()` 用 `tauri::async_runtime::spawn_blocking` L137-145；`window.hwnd()` L153；**`app.emit_to(fence_label(id), "fence-items-changed", items)` L385**；`app.try_state::<FenceWatcherState>()` L422；`app.state::<Database>()` L406/455/529/563/678/718/766/776/791 | — | 大部分 **A**：`FenceActivity` 登记 L57-134、DB 读写 / 对账 L165-329、`reconcile_fence` L340-379、`affected_fence_ids` L396-403、`source_fence_folder` L507-514；**C**：`reconcile_and_notify`（emit_to）、`ensure_fence_watcher`（托管状态）、`spawn_startup_reconcile`、8 个命令外壳 |
| `commands/holiday.rs` | 131 | 仅 `#[tauri::command]` L108 | `reqwest::Client`（**async**）L72-91 | B，但需把 async reqwest 改为 blocking（§8） |
| `commands/notification_cmd.rs` | 26 | `tauri::Manager`；`app_handle.get_webview_window(label)` / `webview_windows()` / `window.close()` | — | C（GPUI 窗口句柄管理，见 §6.3） |
| `commands/settings_cmd.rs` | 177 | 仅 `tauri::State` L3 | Win32 DirectWrite `IDWriteFactory::GetSystemFontCollection` L45-93 | B（`get_system_fonts` 也可换成 `cx.text_system().all_font_names()`） |
| `commands/sync_cmd.rs` | 2961 | `use tauri::{AppHandle, Emitter, Manager, State}` L56；`webdav_test_connection` L234-250（`spawn_blocking` + `app.state`）；`run_sync_command` L535-552（`spawn_blocking` + `app.state` + **`app.emit("sync-completed", &report)` L547**）；**`crate::commands::window::reload_runtime_prefs(db)` L605** | `flate2`、`reqwest::blocking`（经 webdav.rs） | B：除上述外壳外全部纯逻辑（`SyncEngine` L656-1011 等） |
| `commands/todo.rs` | 1257 | `use tauri::{AppHandle, Manager, State}` L11；`save_subtask_image(request: tauri::ipc::Request)` L420-438（读 header `x-image-ext` 与 body，`tauri::async_runtime::spawn_blocking`）；`image_bytes(&tauri::ipc::InvokeBody)` L472-488；`import_subtasks_from_paths` L570-581（`spawn_blocking` + `app.state`）；测试 L1074-1122 用 `tauri::ipc::InvokeBody` | `encoding_rs` | B：`image_bytes` 与 `save_subtask_image` 外壳改写（GPUI 里粘贴 / 拖入直接得到 `Vec<u8>` / 路径） |
| `commands/window.rs` | 2489 | `use tauri::{Manager, State, WebviewWindow, Window}` L11；`tauri::menu::CheckMenuItem<tauri::Wry>` L76/L104；`WebviewWindow` 方法：`outer_position/outer_size` L437-438、`current_monitor/primary_monitor` L448-452、`is_minimized` L619/L936、`set_position` 多处、`set_size` L1873、`set_resizable` L1879、`set_always_on_top` L919、`set_always_on_bottom` L1380/L1462、`set_minimizable` L1381/L1461、`show` L927、`unminimize` L942、`set_focus` L1728/L1759、`run_on_main_thread` L1134/L1182/L1260/L1385、`window_handle()`（raw_window_handle）L788-801、`label()`、`app_handle()`；`app_handle.get_webview_window("main")` 多处 | Win32：`GetClassNameW/GetCursorPos/GetShellWindow/GetWindowLongPtrW/GetWindowLongW/IsWindow/SetWindowLongPtrW/SetWindowLongW/SetWindowPos/ShowWindow` L13-21 | 拆分：贴边隐藏状态机 L199-612（除 `get_window_rect`/`get_monitor_bounds`/`get_cursor_position`）、`apply_runtime_prefs`、`take_disabled_hidden_anchor`、`needs_reattach`、`lacks_tool_window_style`、`is_desktop_host_class`、`begin_tray_wake`/`is_latest_tray_wake`、`save_settings_inner`、设置读写、screen_configs SQL L1889-2048 → **A/B**；Win32 句柄核心（`apply_fixed_ex_style_hwnd`、`desktop_attach_hwnd`、`window_class_name`、`desktop_host`）→ **A**（按 HWND，不碰 Tauri）；其余模式切换 / tick / 托盘勾选 → **C** |
| `db/mod.rs` | 11 | — | — | A |
| `db/backup.rs` | 155 | — | `rusqlite` `VACUUM INTO` | A |
| `db/connection.rs` | 215 | — | — | A（`new_in_memory` 为 `#[cfg(test)]` L71-79，见 §10） |
| `db/migrations.rs` | 1434 | — | — | A（v1~v29，`LATEST_VERSION = 29` L130） |
| `db/models.rs` | 436 | — | `serde` camelCase | A |
| `db/paths.rs` | 162 | — | `dirs::data_local_dir()` | A（`MINI_TODO_DATA_DIR` 仅 debug） |
| `db/settings_kv.rs` | 242 | — | — | A |
| `db/sync_store.rs` | 1657 | — | — | A |
| `db/time.rs` | 409 | — | `chrono` | A |
| `services/mod.rs` | 6 | — | — | A |
| `services/notification.rs` | 1071 | `tauri::async_runtime` L10、`tauri::Manager` L11、`tauri::WebviewUrl` L12、`tauri::WebviewWindowBuilder` L13、`tauri_plugin_notification::NotificationExt` L14；`start_scheduler(AppHandle)` L204-221（`async_runtime::spawn` + `tokio::time::sleep`）；`app_handle.state::<Database>()` L248/L601；`app_handle.notification().builder().title().body().show()` L306-312；`WebviewWindowBuilder::new(...)` L345-360；`window.on_window_event(Destroyed/Resized)` L367-377；`window.scale_factor/outer_size/set_position(LogicalPosition)` L399-416；`app_handle.primary_monitor()` + `monitor.work_area()/position()/size()/scale_factor()` L424-451 | — | 拆分：槽位 L17-45、`WorkArea`/`NotificationSize`/`notification_position` L47-118、SQL 与查询 L120-198、`deliver` 骨架、`mark_as_notified`/`advance_repeat`/`calc_next_occurrence`/`next_weekly`/`next_monthly`/`last_day_of_month` L453-624、`sleep_until_next_minute` 的时间计算 → **A**；调度循环、两种投递、窗口定位 → **C/D** |
| `services/secret.rs` | 169 | — | Win32 DPAPI `CryptProtectData/CryptUnprotectData`（cfg(windows)） | A |
| `services/webdav.rs` | 868 | — | `reqwest::blocking` | A |
| `services/fence_fs/mod.rs` | 65 | 仅注释提到 `tauri::async_runtime::spawn_blocking` L21 | — | A |
| `services/fence_fs/folder.rs` | 567 | — | std::fs | A |
| `services/fence_fs/icon.rs` | 513 | — | `png`；Win32 `IShellItemImageFactory`（经 shell.rs） | A |
| `services/fence_fs/location.rs` | 259 | — | `dirs`；`crate::db::paths::app_data_dir()` L65 | A（core 内部依赖，保持模块路径即可） |
| `services/fence_fs/naming.rs` | 450 | — | — | A |
| `services/fence_fs/open.rs` | 77 | — | `ShellExecuteExW` / `SHOpenFolderAndSelectItems`（经 shell.rs）、`explorer.exe /select` | A |
| `services/fence_fs/ops.rs` | 1280 | — | `IFileOperation`（经 shell.rs） | A |
| `services/fence_fs/shell.rs` | 494 | — | `windows 0.58` COM / Shell（`#[implement]`，cfg(windows)） | A |
| `services/fence_fs/sta.rs` | 143 | — | COM STA 线程（cfg(windows)） | A |
| `services/fence_fs/watch.rs` | 250 | 测试名 `watcher_can_live_in_tauri_state`（L169，只是 `Send` 断言） | `notify 7` | A |

### 1.2 必须断开 / 改写的跨模块依赖

| 调用方 | 被调用 | 位置 | 说明 / 处理 |
|---|---|---|---|
| `data::import_data_raw` | `window::reload_runtime_prefs` | data.rs:245 | 导入改写了 `top_on_wake` / `auto_hide_enabled`。core 删除此调用；app 在导入成功后调用自己的 `reload_runtime_prefs` |
| `sync_cmd::run_sync_blocking` | `window::reload_runtime_prefs` | sync_cmd.rs:603-606 | 仅 `report.settings_applied` 时调用。core 删除；app 根据返回的 `SyncReport.settings_applied` 刷新 |
| `window::save_settings` | `reload_runtime_prefs` | window.rs:735 | 同上，app 层在保存后刷新 |
| `lib.rs` setup | `reload_runtime_prefs` | lib.rs:240 | 启动时灌缓存，app 层保留 |
| `fence::open_fence_window` | `crate::setup_window_rounded_corners`（lib.rs） | fence.rs:880 | app 平台层函数 |
| `fence.rs` | `window::{embed_window_in_desktop, reassert_embedded_window, release_embedded_window, set_window_bounds_physical}` | fence.rs:43-46 | app 平台层 |
| `fence.rs` | `fence_items::{begin_deleting, blocking, delete_item, ensure_fence_watcher, fence_originals, is_deleting, is_moving, owner_hwnd, reconcile_and_notify, spawn_startup_reconcile}` | fence.rs:38-41 | 前 6 个纯逻辑进 core；`blocking`/`owner_hwnd`/`ensure_fence_watcher`/`reconcile_and_notify`/`spawn_startup_reconcile` 是 app 层 |
| `fence_items.rs` | `fence::{fence_label, fence_window, load_fence, load_fences}` | fence_items.rs:31 | `load_fence(s)` 进 core；`fence_window`/`fence_label` 是 app 层窗口登记 |
| `sync_cmd.rs` | `data::{adopt_settings_json, read_app_settings, settings_version, EXPORT_VERSION}` | sync_cmd.rs:33 | 都在 core，保持 |
| `fence_fs::location::icon_cache_dir` | `crate::db::paths::app_data_dir` | location.rs:65 | 都在 core，保持 |
| 测试 `window::tests::save_settings_only_touches_changed_keys` | `crate::commands::data::{read_app_settings, settings_version}` | window.rs:2390、2403 | 迁移后改模块路径 |
| 测试 `fence_items::tests::onedrive_notice_is_claimed_once_and_stays_device_local` | `super::super::data::read_app_settings` | fence_items.rs:1019 | 迁移后改模块路径 |

### 1.3 配置文件

**`Cargo.toml`（58 行）依赖映射**

| 现有依赖 | 用途 | GPUI 版去向 |
|---|---|---|
| `tauri = { version = "2", features = ["tray-icon", "protocol-asset", "macos-private-api"] }` | 运行时、窗口、托盘、asset 协议 | `gpui-kit = "0.7.3"`（内含 `gpui-pre =0.3.9`）+ `tray-icon` |
| `tauri-plugin-opener` | 前端 `openUrl` / `revealItemInDir` | `cx.open_url` / `cx.reveal_path`（GPUI 自带）或 `open` crate；收纳箱已有 `fence_fs::reveal_in_explorer` |
| `tauri-plugin-notification` | 系统通知 | `notify-rust 4.x`（§6.4） |
| `tauri-plugin-dialog` | 前端 `save` / `open` | `rfd 0.16`（plugin-dialog 2.6.0 本身依赖 `rfd 0.16.0`，见 Cargo.lock） |
| `tauri-plugin-fs` | 前端 `readTextFile` | `std::fs`（core 的 `import_from_file` 已覆盖 JSON / ZIP） |
| `tauri-plugin-autostart` | 自启 | `auto-launch 0.5`（plugin-autostart 2.5.1 的底层，Cargo.lock 为 0.5.0） |
| `tauri-plugin-single-instance ~2.3.7` | 单实例 | 自实现（§4.6） |
| `tauri-plugin-log ~2.8.0` | 日志文件 + 轮转 | `flexi_logger` / `log4rs` 等（§9.3） |
| `log` `encoding_rs` `serde` `serde_json` `rusqlite 0.32 bundled` `chrono` `dirs 5` `base64` `reqwest 0.12 (json, blocking)` `urlencoding` `flate2` `zip 2 (deflate)` `notify 7` `png 0.17` | 业务 | 原样进 core（`urlencoding` 只被通知窗口 URL 用，GPUI 下可删） |
| `tokio = { features = ["full"] }` | 仅 notification.rs 的 `tokio::time::sleep` 与 Tauri 运行时 | core 不再需要（§6、§8） |
| `windows 0.58`（features 见 Cargo.toml L46）、`windows-core 0.58`、`raw-window-handle 0.6` | Win32 | core 保留 0.58（`#[implement]` 需与 windows-core 同版本）；注意 GPUI 用 `windows 0.62`（§5.4），两版共存可编译，跨边界传 HWND 用 `isize` |
| dev：`tiny_http 0.12` | sync_cmd 测试的 mock WebDAV | 原样进 core 的 dev-dependencies |

**`tauri.conf.json`**

| 项 | 值 | GPUI 版 |
|---|---|---|
| `identifier` | `com.tauri-app.mini-todo` | 仍需要：日志目录 `%LOCALAPPDATA%\com.tauri-app.mini-todo\logs`、Windows 通知 AUMID、单实例命名 |
| `productName` / `version` | `mini-todo` / `2.5.0` | `env!("CARGO_PKG_VERSION")`（前端 `getVersion()` 用于检查更新） |
| `app.windows[0]`（主窗口） | `title "Mini Todo"`、`380×600`、`minWidth 320`、`minHeight 400`、`resizable true`、`decorations false`、`transparent true`、`alwaysOnTop false`、`visible true`、`center true`、`shadow false` | `WindowOptions { window_bounds, window_min_size: 320×400, is_resizable: true, titlebar: None/透明, window_background: Transparent, kind: Normal }` |
| `security.csp` / `assetProtocol.scope` / `dangerousDisableAssetCspModification` | WebView 安全边界 | X（无 WebView） |
| `bundle.windows.webviewInstallMode` | `embedBootstrapper` | X |
| `bundle.icon` | `icons/32x32.png` 等 + `icons/icon.ico` | 托盘图标 / exe 资源复用 `pc/src-tauri/icons/` |

**`capabilities/default.json`**：只服务于 Tauri IPC 权限模型（窗口 / webview / opener / notification / dialog / fs / autostart 权限与 `fs:scope`），GPUI 下整体删除（X）。

---

## 2. Tauri 命令清单（80 个）

列说明：**线程** = sync（Tauri 2 同步命令跑在主线程）/ async（tokio worker）/ async+blk（内部 `spawn_blocking`）。
**前端调用方** = `grep "invoke('xxx'"` 结果（`pc/src` 下，排除 `*.test.ts`）。**去向**：core = 进入纯业务 crate；app = 留在 GPUI 层。

### 2.1 todo.rs（15）

| 命令 | 行 | 参数 | 返回 | 线程 | 作用 | 前端调用方 | 去向 |
|---|---|---|---|---|---|---|---|
| `get_todos` | 17 | `db: State<Database>` | `Vec<Todo>` | sync | `sync_store::load_todos_with_subtasks(conn, "completed ASC, sort_order ASC, created_at DESC")`（一次查子任务按 parent 分组） | `stores/todoStore.ts`、`views/CompletedView.vue` | core |
| `get_todo` | 25 | `db`, `id: i64` | `Todo` | sync | `load_todo`（含按 `sort_order, id` 排序的子任务），不存在 `Err("待办不存在")` | `views/EditorView.vue` | core |
| `get_change_seq` | 34 | `db` | `i64` | sync | `sync_store::local_seq`（v28 触发器维护） | `stores/todoStore.ts`（5s 轮询） | core |
| `create_todo` | 100 | `db`, `data: CreateTodoRequest` | `Todo` | sync | 三个时间字段规范化（提醒 09:00 / 开始 00:00 / 截止 23:59），事务内 `sort_order = max(未完成)+1` | `views/EditorView.vue` | core |
| `update_todo` | 138 | `db`, `id: i64`, `data: UpdateTodoRequest` | `Todo` | sync | 动态拼 SET（clear_* 标志、改提醒重置 `notified`、重复提醒相关），追加 `SQL_SET_UPDATED_AT` | `todoStore.ts`、`CompletedView.vue`、`EditorView.vue` | core |
| `delete_todo` | 258 | `db`, `id` | `()` | sync | 事务内 `delete_todo_with_tombstones`（连带子任务墓碑） | `todoStore.ts`、`CompletedView.vue` | core |
| `reorder_todos` | 266 | `db`, `ids: Vec<i64>` | `()` | sync | `apply_order(Todos)`：下标即 sort_order，只更新变化行 | `todoStore.ts` | core |
| `reorder_subtasks` | 275 | `db`, `ids: Vec<i64>` | `()` | sync | `apply_order(Subtasks)` | `EditorView.vue` | core |
| `create_subtask` | 310 | `db`, `data: CreateSubTaskRequest` | `SubTask` | sync | 事务内 `sort_order = max+1` | `EditorView.vue` | core |
| `update_subtask` | 327 | `db`, `id`, `data: UpdateSubTaskRequest` | `SubTask` | sync | 动态 SET + `SQL_SET_UPDATED_AT` | `EditorView.vue`、`SubtaskEditorView.vue` | core |
| `get_subtask` | 381 | `db`, `id` | `SubTask` | sync | 单条读取 | `SubtaskEditorView.vue` | core |
| `delete_subtask` | 388 | `db`, `id` | `()` | sync | `delete_subtask_with_tombstone` | `EditorView.vue` | core |
| `get_images_dir` | 405 | — | `String` | sync | 创建并返回 `images/` 绝对路径 | `utils/imageRef.ts` | core |
| `save_subtask_image` | 421 | `request: tauri::ipc::Request<'_>`（body = 原始字节或数字数组；header `x-image-ext`） | `String`（图片绝对路径） | async+blk | 扩展名白名单 `png/jpg/jpeg/webp/gif/bmp`、≤20MB、`store_image` 生成 `<毫秒>_<6位>.<ext>`（先写 `.<name>.part` 再改名） | `components/MarkdownEditor.vue` | core（签名改为 `save_image(bytes: &[u8], ext: &str)`） |
| `import_subtasks_from_paths` | 571 | `app: AppHandle`, `parent_id: i64`, `paths: Vec<String>` | `Vec<SubTask>` | async+blk | 递归收集 .md/.txt（≤5 层、≤1000 个、单个 ≤10MB，跳过符号链接目录），UTF-8/BOM/UTF-16/GBK 解码，任一失败整体取消，单事务插入 | `EditorView.vue`（两处：选文件、选文件夹） | core（阻塞，app 放后台执行） |

### 2.2 window.rs（24）

| 命令 | 行 | 参数 | 返回 | 线程 | 作用 | 前端调用方 | 去向 |
|---|---|---|---|---|---|---|---|
| `sync_auto_start_state` | 121 | `enabled: bool` | `()` | sync | 回写托盘「开机自启动」勾选 | `views/SettingsView.vue` | app |
| `save_settings` | 733 | `db`, `settings: AppSettings` | `()` | sync | 一个事务写 `is_fixed/fixed_embed_desktop/window_position/window_size/auto_hide_enabled/top_on_wake/window_bg_color/window_bg_alpha/text_theme`（只在值变时刷 `updated_at`），然后 `reload_runtime_prefs` | `stores/appStore.ts`（`saveWindowState`） | core（`save_settings_inner` L739-760）+ app 刷新缓存 |
| `set_text_theme` | 766 | `db`, `theme: String` | `()` | sync | 只写 `text_theme` | `appStore.ts` | core |
| `get_text_theme` | 773 | `db` | `String`（默认 `"dark"`，`"light"` = 深色主题开启） | sync | 读 | `appStore.ts` | core |
| `set_window_desktop_mode` | 1400 | `app_handle`, `db`, `enabled: bool` | `()` | sync | 主窗口进入 / 退出"固定 + 嵌入桌面"（非 Windows：true 回退 `set_window_fixed_mode(true)`，false no-op） | `appStore.ts`（`applyFixedMode` / `applyNormalMode`） | app |
| `set_window_fixed_mode` | 1477 | `app_handle`, `db`, `fixed: bool` | `()` | sync | 主窗口进入 / 退出经典固定模式（互斥清理、贴边状态复位、撤置顶、重申样式、托盘勾选） | `appStore.ts` | app |
| `get_fixed_embed_desktop` | 1553 | `db` | `bool`（缺键 false） | sync | 读 | `appStore.ts` | core |
| `set_fixed_embed_desktop` | 1564 | `app_handle`, `db`, `enabled: bool` | `()` | sync | 写键；Windows 上已固定时立即切换嵌入 / 经典 | `appStore.ts`（设置窗口触发） | core 写键 + app 切换 |
| `set_auto_hide_cursor_inside` | 1586 | `inside: bool` | `()` | sync | 写 `AUTO_HIDE_STATE.cursor_inside_window`（仅取不到光标时作后备，Windows 上基本不用） | `views/MainView.vue`（mouseenter/leave） | app（GPUI 下可直接由 hover 状态更新） |
| `get_auto_hide_enabled` | 1594 | `db` | `bool`（默认 true） | sync | 读 | `appStore.ts` | core |
| `set_auto_hide_enabled` | 1599 | `app_handle`, `db`, `enabled: bool` | `()` | sync | 写键；更新状态机（`enabled && is_fixed_mode()`）；关闭时拉回藏起的窗口、撤置顶 | `appStore.ts` | core 写键 + app |
| `get_top_on_wake` | 1652 | `db` | `bool`（默认 true） | sync | 读 | `appStore.ts` | core |
| `set_top_on_wake` | 1657 | `app_handle`, `db`, `enabled: bool` | `()` | sync | 写键 + `TOP_ON_WAKE`；关闭时立即撤置顶 | `appStore.ts` | core 写键 + app |
| `get_window_background` | 1687 | `db` | `WindowBackground { color: String, alpha: f64 }` | sync | 读 `window_bg_color`（默认 `#000000`）/ `window_bg_alpha`（默认 0.45） | `appStore.ts` | core |
| `set_window_background` | 1703 | `db`, `color: String`, `alpha: f64` | `()` | sync | 事务写两键（alpha clamp 0~1） | `appStore.ts` | core |
| `get_window_persist_state` | 1795 | `app_handle` | `WindowPersistState { position, size }` | sync | 主窗口物理外框；贴边隐藏中返回锚点位置 / 尺寸 | `appStore.ts`（`saveWindowState`） | app |
| `reset_window` | 1831 | `window: Window` | `()` | sync | 主窗口重置到主显示器 10% 边距、380×600 逻辑像素、`set_resizable(true)` | **前端未调用**（托盘走 `reset_webview_window`） | app |
| `get_screen_config` | 1914 | `db`, `config_id: String` | `Option<ScreenConfig>` | sync | 按组合 ID 读 | `appStore.ts`（`initSettings`） | core |
| `save_screen_config` | 1939 | `db`, `config: SaveScreenConfigRequest` | `ScreenConfig` | sync | 按 `config_id` upsert（`display_name` 为 null 时保留旧值） | `appStore.ts` | core |
| `list_screen_configs` | 1983 | `db` | `Vec<ScreenConfig>`（`updated_at DESC`） | sync | 读全部 | `appStore.ts` | core |
| `delete_screen_config` | 2012 | `db`, `config_id: String` | `()` | sync | 事务删 `screen_configs` 行 **和** `fence_layouts WHERE screen_config_id = ?` | `appStore.ts` | core |
| `update_screen_config_name` | 2019 | `db`, `config_id`, `display_name` | `()` | sync | 改显示名 | **前端未调用** | core |
| `get_show_calendar` | 2038 | `db` | `bool`（默认 false） | sync | 读 | `appStore.ts` | core |
| `set_show_calendar` | 2045 | `db`, `show: bool` | `()` | sync | 写 | `appStore.ts` | core |

### 2.3 settings_cmd.rs（9）

| 命令 | 行 | 参数 | 返回 | 线程 | 作用 | 前端调用方 | 去向 |
|---|---|---|---|---|---|---|---|
| `get_setting` | 23 | `db`, `key: String` | `Option<String>` | sync | 白名单 `FRONTEND_SETTINGS`（L9，仅 `view_mode`）内读取 | `todoStore.ts` | core（GPUI 下白名单意义减弱，可改成具名函数 `get_view_mode`） |
| `set_setting` | 32 | `db`, `key`, `value` | `()` | sync | 白名单 + 取值校验（`list`/`quadrant`） | `todoStore.ts` | core |
| `get_system_fonts` | 45 | — | `Vec<String>` | sync | DirectWrite 枚举系统字体族（排序去重；非 Windows 空） | `SettingsView.vue` | core 或 GPUI `text_system().all_font_names()` |
| `get_todo_font_family` | 95 | `db` | `String`（默认空串） | sync | 读 | `appStore.ts`、`SettingsView.vue` | core |
| `set_todo_font_family` | 103 | `db`, `font_family: String` | `()` | sync | 写 | `SettingsView.vue` | core |
| `get_todo_font_size` | 111 | `db` | `i32`（默认 14） | sync | 读 | `appStore.ts`、`SettingsView.vue` | core |
| `set_todo_font_size` | 121 | `db`, `font_size: i32` | `()` | sync | clamp 12..=20 后写 | `SettingsView.vue` | core |
| `get_notification_type` | 131 | `db` | `String`（默认 `system`） | sync | 读 | `SettingsView.vue` | core |
| `set_notification_type` | 144 | `db`, `notification_type: String` | `()` | sync | 非 `system`/`app` 一律存 `system` | `SettingsView.vue` | core |

### 2.4 data.rs（4）

| 命令 | 行 | 参数 | 返回 | 线程 | 作用 | 前端调用方 | 去向 |
|---|---|---|---|---|---|---|---|
| `export_data` | 333 | `app` | `String`（pretty JSON） | async+blk | `export_data_internal` | **前端未调用** | core |
| `import_data` | 338 | `app`, `json_data: String` | `()` | async+blk | `import_data_raw`（备份 → 事务替换 → 墓碑 → 写设置 → 刷新运行时缓存） | `SettingsView.vue`（非 ZIP：先 `readTextFile` 再调） | core + app 刷新 |
| `export_data_to_file` | 343 | `app`, `file_path: String` | `()` | async+blk | ZIP（内含 `data.json`，Deflated） | `SettingsView.vue` | core |
| `import_data_from_file` | 348 | `app`, `file_path: String` | `()` | async+blk | 按 `PK` 魔数判断 ZIP / JSON 后 `import_data_raw` | `SettingsView.vue` | core + app 刷新 |

### 2.5 holiday.rs（1）

| 命令 | 行 | 参数 | 返回 | 线程 | 作用 | 前端调用方 | 去向 |
|---|---|---|---|---|---|---|---|
| `fetch_holidays` | 109 | `year: i32` | `Vec<HolidayInfo { date, name, isOffDay }>` | async（async reqwest） | 先网络（`raw.githubusercontent.com/NateScarlet/holiday-cn/master/{year}.json`，超时 10s / 连接 5s），成功写缓存 `<app_data_dir>/holidays/{year}.json`；失败回退缓存 | `utils/holiday.ts` | core（改 blocking） |

### 2.6 notification_cmd.rs（2）

| 命令 | 行 | 参数 | 返回 | 线程 | 作用 | 前端调用方 | 去向 |
|---|---|---|---|---|---|---|---|
| `close_notification_window` | 5 | `app_handle`, `window_label: String` | `()` | sync | 关闭指定标签窗口 | `views/NotificationView.vue`（点击 / 关闭按钮） | app |
| `close_all_notification_windows` | 17 | `app_handle` | `()` | sync | 关闭所有 `notification_*` | **前端未调用** | app |

### 2.7 fence.rs（11）

| 命令 | 行 | 参数 | 返回 | 线程 | 作用 | 前端调用方 | 去向 |
|---|---|---|---|---|---|---|---|
| `list_fences` | 1238 | `db` | `Vec<Fence>`（`sort_order, id`） | sync | 读 | `components/FenceItemGrid.vue`（「移动到…」菜单） | core |
| `get_fence` | 1244 | `db`, `id` | `Fence` | sync | 读 | `views/FenceView.vue` | core |
| `create_fence` | 1251 | `app` | `Fence` | async | `create_fence_at_cursor`（与托盘同一实现） | **前端未调用** | core（记录 / 文件夹）+ app（显示器、建窗） |
| `rename_fence` | 1258 | `app`, `id: i64`, `title: String` | `RenameFenceResult { fence, folderError }` | async+blk | `rename_fence_and_folder`（持库锁改名 + 改文件夹），然后 `window.set_title` | `FenceView.vue` | core + app 改窗口标题 |
| `update_fence` | 1284 | `db`, `id`, `patch: UpdateFenceRequest` | `Fence` | sync | `update_fence_row`（只写给出字段；颜色 `#RRGGBB` 大写；alpha 0~1；图标 24~128） | `FenceView.vue`（两处） | core |
| `set_fence_view` | 1296 | `app`, `db`, `id`, `expanded: bool` | `()` | sync | `apply_fence_view`（折叠 / 展开形态，不写库） | `FenceView.vue` | app（几何计算 `fence_view` 在 core） |
| `set_fence_bounds` | 1315 | `app`, `id`, `x: i32`, `y: i32`, `width: u32`, `height: u32` | `()` | sync | `set_window_bounds_physical`（一次 `SetWindowPos(NOZORDER\|NOACTIVATE)`），不写库 | `FenceView.vue`（缩放手柄逐帧） | app |
| `commit_fence_geometry` | 1334 | `app`, `db`, `id`, `expanded_view: bool` | `Fence` | sync | 读窗口外框 → `committed_rect` → `save_fence_geometry`（`fences` + 当前组合 `fence_layouts`）→ 移除 `SESSION_RECTS` | `FenceView.vue` | core（存库）+ app（读外框） |
| `delete_fence` | 1416 | `app`, `id` | `DeleteFenceResult { deleted, restored, remaining, failures }` | async | 登记删除中 → `restore_all`（blocking，owner HWND）→ `plan_delete` → 删记录 + `destroy()` 或保留并对账通知 | `FenceView.vue` | core（还原 / 计划 / 记录）+ app（销毁窗口、通知） |
| `toggle_fences_visibility` | 1476 | `app` | `bool`（切换后是否可见） | sync | `toggle_all_fences` | **前端未调用**（托盘） | app |
| `reset_fence_positions` | 1482 | `app` | `()` | sync | `reset_all_fence_positions` | **前端未调用**（托盘） | core（`reset_layout` + 写库）+ app（显示器、摆窗） |

### 2.8 fence_items.rs（8）

| 命令 | 行 | 参数 | 返回 | 线程 | 作用 | 前端调用方 | 去向 |
|---|---|---|---|---|---|---|---|
| `get_fence_items` | 527 | `app`, `id` | `Vec<FenceEntry>` | async+blk | `reconcile_fence`；`None` → `Err("收纳箱 #{id} 不存在或正在删除")` | `FenceView.vue` | core |
| `fence_move_in` | 549 | `app`, `id`, `paths: Vec<String>` | `FenceMoveInResult { results, onedriveNotice }` | async+blk | 登记移动中 → 识别跨箱来源（移动前读旧行 original_path）→ `move_into_fence(paths, folder, owner)` → upsert 行 / 删旧行 → OneDrive 一次性提示 → 对来源收纳箱 `reconcile_and_notify` | `FenceView.vue`（`onDragDropEvent` 的 drop） | core（+ owner HWND、通知回调由 app 注入） |
| `fence_restore_item` | 669 | `app`, `id`, `file_name: String` | `ItemOpResult` | async+blk | 登记 → `restore_item(folder, name, original, owner)` → 成功删行 | `FenceItemGrid.vue` | core |
| `fence_move_item` | 705 | `app`, `id`, `file_name`, `target_id: i64` | `ItemOpResult` | async+blk | 同箱报错；登记两箱 → `move_between_fences` → 删旧行 / upsert 新行（搬运原始路径）→ 通知目标箱 | `FenceItemGrid.vue` | core |
| `fence_open_item` | 764 | `app`, `id`, `file_name` | `()` | async+blk | `is_valid_item_name` 校验 → `launch_item`（`ShellExecuteExW`） | `FenceItemGrid.vue` | core |
| `fence_reveal_item` | 774 | `app`, `id`, `file_name` | `()` | async+blk | `reveal_in_explorer` | `FenceItemGrid.vue` | core |
| `fence_item_icon` | 784 | `app`, `id`, `file_name`, `size_px: u32` | `String`（缓存 PNG 绝对路径） | async+blk | `icon_png`（档位向上取整，`fence-icons/` 缓存） | `FenceItemGrid.vue`（`convertFileSrc` 加载） | core |
| `reorder_fence_items` | 800 | `db`, `id`, `file_names: Vec<String>` | `()` | sync | `reorder_rows` | `FenceItemGrid.vue` | core |

### 2.9 sync_cmd.rs（6）

| 命令 | 行 | 参数 | 返回 | 线程 | 作用 | 前端调用方 | 去向 |
|---|---|---|---|---|---|---|---|
| `get_sync_settings` | 146 | `db` | `SyncSettings`（`webdavPassword` 恒空、`hasPassword`、`deviceId` 不存在时生成） | sync | `read_sync_settings` | `MainView.vue`（手动同步前检查、`startAutoSync`）、`SettingsView.vue` | core |
| `save_sync_settings` | 170 | `db`, `settings: SyncSettings` | `()` | sync | 密码：`clearPassword` > 新密码 DPAPI 加密 > 保留（旧明文顺手加密）；换服务器 / 账号清基准与 `synced_seq` | `SettingsView.vue` | core |
| `webdav_test_connection` | 235 | `app`, `url`, `username`, `password: Option<String>`（空用已存密码） | `bool` | async+blk | `WebDavClient::test_connection` | `SettingsView.vue` | core |
| `webdav_sync` | 518 | `app`, `full: Option<bool>` | `SyncReport` | async+blk | 智能同步（`SyncGuard` 互斥；每第 10 轮或 `full` 无条件 GET）；`local_changed()` 时 `emit("sync-completed")` | `MainView.vue`（标题栏按钮 `{full:true}` L653、自动同步无参 L680）、`SettingsView.vue`（`runSyncCommand` 立即同步 `{full:true}` L552） | core + app 广播 |
| `webdav_force_pull` | 525 | `app` | `SyncReport` | async+blk | 本地 := 远端 | `SettingsView.vue` L567 | core + app 广播 |
| `webdav_force_push` | 531 | `app` | `SyncReport` | async+blk | 远端 := 本地 | `SettingsView.vue` L582 | core + app 广播 |

### 2.10 前端未调用的命令（7）

`reset_window`、`update_screen_config_name`、`export_data`、`close_all_notification_windows`、`create_fence`、`toggle_fences_visibility`、
`reset_fence_positions`（后三者是托盘入口的同一实现，也供 e2e 经 CDP 调用）。另有托盘「重置位置」用的
`reset_webview_window`（非命令，window.rs:1840，lib.rs:355 调用）与 `reset_window` 共用 `reset_window_impl`。移植时这些只需保留其内部实现（托盘 / e2e 用）。

### 2.11 前端直接使用的 Tauri 插件 / API（不经 invoke，app 层需要替代）

| API | 位置 | 用途 | GPUI 版替代 |
|---|---|---|---|
| `@tauri-apps/plugin-dialog` `save` | `SettingsView.vue:323`（标题"导出待办数据"，默认名 `mini-todo-backup-YYYY-MM-DD.zip`，过滤 ZIP） | 导出 | `rfd::FileDialog::save_file()`（GPUI `prompt_for_new_path` 无过滤器 / 标题） |
| `plugin-dialog` `open` | `SettingsView.vue:347`（过滤 zip/json）、`EditorView.vue:716`（多选 md/txt）、`:747`（选文件夹） | 导入数据、导入子任务 | `rfd` `pick_file` / `pick_files` / `pick_folder`（GPUI `PathPromptOptions` 只有 `files/directories/multiple/prompt`，无过滤器） |
| `plugin-fs` `readTextFile` | `SettingsView.vue:372` | 非 ZIP 导入时读 JSON | `std::fs::read_to_string` 或直接用 core `import_from_file` |
| `plugin-opener` `openUrl` | `TitleBar.vue:117`、`SettingsView.vue:608`、`utils/fileLink.ts:115` | 打开发布页 / 链接 | `cx.open_url(url)` 或 `open::that` |
| `plugin-opener` `revealItemInDir` | `utils/fileLink.ts:104` | `file:///` 链接定位 | `cx.reveal_path` 或 `fence_fs::reveal_in_explorer` |
| `plugin-autostart` `enable/disable/isEnabled` | `SettingsView.vue:125/282/285` | 设置页自启开关 | `auto-launch`（与托盘共用一个实例） |
| `api/app` `getVersion` | `appStore.ts` | 版本检查 | `env!("CARGO_PKG_VERSION")` |
| `api/core` `convertFileSrc` | `FenceItemGrid.vue`、`utils/imageRef.ts` | 本地图片 / 图标 | GPUI `img(PathBuf)` |
| `api/window` `availableMonitors/primaryMonitor/currentMonitor`、`setPosition/setSize(Physical*)`、`setResizable`、`startDragging`、`onMoved/onResized/onFocusChanged/onCloseRequested` | `appStore.ts`、`MainView.vue`、`FenceView.vue`、`EditorView.vue`、`CompletedView.vue`、`CalendarView.vue` | 窗口几何与事件 | GPUI `Window`（`start_window_move`、`observe_window_bounds`、`on_window_should_close`）+ Win32 定位（§5.4） |
| `api/webviewWindow` `new WebviewWindow(label, {...})` | `MainView.vue:456/526/590`（`completed-*`、`editor-*`、`settings-*`）、`EditorView.vue:970`（`subtask-{mode}-*`）、`CompletedView.vue:129`（`editor-*`） | 子窗口（全部 `decorations:false`、`transparent:false`、多数 `parent: appWindow`） | `cx.open_window`；注意 Tauri 的 `parent` = owner（tao `OwnedBy`，不带 `WS_EX_APPWINDOW`），GPUI Normal 窗口一律 `WS_EX_APPWINDOW`、无 owner（§5.4） |
| `api/webview` `onDragDropEvent` | `FenceView.vue:555` | 外部文件拖入收纳箱 | GPUI 每窗口注册了 `IDropTarget`（gpui-pre-windows `window.rs:1144-1160`），用 `ExternalPaths` 拖放事件 |

---

## 3. 事件清单

### 3.1 后端发出的事件

| 事件名 | payload | 发送方（文件:行） | 方式 | 接收方（文件:行） | 接收后行为 |
|---|---|---|---|---|---|
| `tray-toggle-fixed` | `()` | lib.rs:349（托盘 `toggle_fixed`） | `main_window.emit`（Tauri 2 的 `emit` 为全局广播） | `MainView.vue:293` | `appStore.toggleFixedMode()`（含 `saveWindowState`） |
| `tray-reset-window` | `()` | lib.rs:359（托盘 `reset`，在 `reset_webview_window` 之后） | 同上 | `MainView.vue:297` | 固定中 → `toggleFixedMode()`；否则清防抖并 `saveWindowState()`；最后 `initSettings()` |
| `tray-add-todo` | `()` | lib.rs:365 | 同上 | `MainView.vue:315` | `openEditor(undefined, true)`（新建、居中于屏幕、无 parent） |
| `tray-open-settings` | `()` | lib.rs:385（先 `show()` + `set_focus()` 主窗口） | 同上 | `MainView.vue:319` | `openSettings()` |
| `sync-completed` | `SyncReport`（camelCase；`status` snake_case） | sync_cmd.rs:547，仅 `report.local_changed()` | `app.emit` 全局 | `MainView.vue:377`、`SettingsView.vue:143` | 主窗口 `applySyncReport`（按需 `fetchTodos`、设置变更时重载设置）；设置窗口刷新同步状态显示 |
| `fence-items-changed` | `Vec<FenceEntry>`（对账后按 `sort_order` 排好的完整列表） | fence_items.rs:385（`reconcile_and_notify`，触发点：目录监听回调 `on_fence_change` L405-418、`fence_move_in` 的来源箱 L656-658、`fence_move_item` 的目标箱 L743、`delete_fence` 中止后 fence.rs:1457） | `app.emit_to("fence-{id}", ...)`，窗口不存在不投递 | `FenceView.vue:550`（`appWindow.listen`，与 `get_fence_items` 共用序号丢弃晚到旧列表） | `setItems(payload)` |

### 3.2 前端窗口之间互发的事件（不经后端，单进程 GPUI 下变成共享状态 / Entity 事件）

| 事件名 | payload | 发送方 | 接收方 | 语义 |
|---|---|---|---|---|
| `app-settings-changed` | `{ key: 'showCalendar' \| 'autoHide' \| 'topOnWake' \| 'fixedEmbedDesktop' \| 'windowBackground' \| 'theme' \| 'sync' \| 'update' }` | `SettingsView.vue:162`（`notifyAppSettingChanged`） | `MainView.vue:335` | 主窗口按 key 从库重载对应设置；`sync` → 重装自动同步定时器；`fixedEmbedDesktop` 只重载值（窗口切换已由后端完成） |
| `todo-font-changed` | 无 | `SettingsView.vue:218/227` | `MainView.vue:330` | 重载字体设置 |
| `data-imported` | 无 | `SettingsView.vue:376` | `MainView.vue:323` | `fetchTodos` + 重载设置 + 提示"数据导入成功" |
| `subtask-memory-ready` | `{ pendingId }` | `SubtaskEditorView.vue:66` | `EditorView.vue:931` | 新建待办未入库时子任务窗口就绪 |
| `subtask-memory-init` | `{ pendingId, title, content }` | `EditorView.vue:933` | `SubtaskEditorView.vue:57` | 编辑窗口把内存中的子任务交给子任务窗口 |
| `subtask-memory-save` | `{ pendingId, title, content }` | `SubtaskEditorView.vue:76` | `EditorView.vue:939` | 子任务窗口把编辑结果交回 |

### 3.3 后端订阅的原生事件

| 事件 | 位置 | 行为 |
|---|---|---|
| `TrayIconEvent::Click { button: Left, button_state: Up }` | lib.rs:412-425 | 300ms 去抖（`accept_tray_click`，`Instant` 单调）后 `bring_main_window_to_front` |
| 托盘菜单 `on_menu_event` | lib.rs:345-411 | 见 §4.3 |
| 收纳箱 `WindowEvent::CloseRequested` | fence.rs:887 | `api.prevent_close()`（Alt+F4 关不掉） |
| 收纳箱 `WindowEvent::Destroyed` | fence.rs:888 | `release_embedded_window(label)` |
| 通知窗口 `WindowEvent::Destroyed` | notification.rs:369 | `release_slot` 归还堆叠槽位 |
| 通知窗口 `WindowEvent::Resized(inner)` | notification.rs:371-375 | `fit_to_actual_size` 按真实尺寸重新定位 |
| 单实例回调 | lib.rs:194-201 | argv 含 `--autostart` 静默；否则 `bring_main_window_to_front` |
| 目录监听回调（非 Tauri 事件） | fence_items.rs:433（线程 `fence-watch`） | `on_fence_change` → 对账 → `emit_to` |

前端还订阅了 `tauri://destroyed` / `tauri://error`（子窗口生命周期，`MainView.vue:470/476/541/548/605/614`、`EditorView.vue:983/989`、`CompletedView.vue:144/150`）以及 `onFocusChanged` / `onMoved` / `onResized` / `onCloseRequested`（`MainView.vue:278-288/365`、`FenceView.vue:552-553`、`CalendarView.vue:241`、`CompletedView.vue:178`）。

### 3.4 GPUI 下的映射建议

- 单进程无 IPC：`sync-completed` / `data-imported` / `app-settings-changed` / `todo-font-changed` 可合并为一个全局状态 Entity（例如 `AppState`）上的 `EventEmitter` 事件或版本号 + `cx.notify()`；`subtask-memory-*` 变成编辑器 Entity 的共享内存子任务列表。
- `fence-items-changed` 的来源在后台线程（`fence-watch`、阻塞线程池），需要经 channel / `AsyncApp::update` 回到前台再更新对应收纳箱的 Entity。
- 托盘事件来自 tray-icon 的回调（Win32 窗口过程里执行），**不要在回调里直接借用 GPUI `App`**（GPUI `App` 是 `RefCell`，重入会 panic；也对应 Tauri 里"菜单回调中建窗会死锁"的同类问题），一律经 channel 投递到 `cx.spawn` 的前台循环处理。

---

## 4. lib.rs 启动流程

### 4.1 时序（`run()` L181-575）

| 步骤 | 行 | 内容 | GPUI 版 |
|---|---|---|---|
| 1 | 183-186 | `db::paths::dev_data_dir_override()`；有值时 `isolate_webview_user_data`（设 `WEBVIEW2_USER_DATA_FOLDER=<dir>\EBWebView`） | 保留开发隔离判断；WebView2 隔离删除 |
| 2 | 188-205 | **单实例插件第一个注册**（隔离时不注册） | 进程入口最先做单实例检测（§4.6） |
| 3 | 207-216 | 依次注册 log / opener / notification / dialog / fs / autostart（`MacosLauncher::LaunchAgent`，参数 `["--autostart"]`） | 初始化 logger、auto-launch 等 |
| 4 | 218-234 | 隔离时 `log::warn!` 一条，并 `asset_protocol_scope().allow_directory(images_dir / icon_cache_dir)` | 只保留日志 |
| 5 | 238-243 | `open_database_or_exit()` → `reload_runtime_prefs(&database)` → `app.manage(database)` → `app.manage(FenceWatcherState::default())` | `Database` 放进 `Arc` / GPUI Global；同样先灌运行时缓存 |
| 6 | 245-253 | 主窗口：`remember_main_window`（缓存 HWND）、Windows `setup_window_rounded_corners`、macOS 透明背景 | 建主窗口后缓存 HWND、设圆角（§5.2-R） |
| 7 | 256-334 | 托盘菜单项创建、自启自愈、保存 `CheckMenuItem` 引用（`set_tray_toggle_fixed_item` / `set_tray_auto_start_item`） | tray-icon + muda（§4.3） |
| 8 | 336-426 | `TrayIconBuilder`：图标 `default_window_icon()`、隔离时 tooltip "Mini Todo（开发版）"、`show_menu_on_left_click(false)`、菜单事件、左键单击 | 同 |
| 9 | 429 | `NotificationService::start_scheduler(app.handle().clone())` | §4.5 |
| 10 | 432-433 | Windows：`commands::restore_fences(app.handle())`（异步建窗） | §7 |
| 11 | 437-470 | 200ms 轮询线程 | §4.4 |
| 12 | 474-569 | `invoke_handler` 注册 80 个命令 | 删除 |
| 13 | 570-574 | `build(generate_context!())` + `run(\|_, _\| {})`（空事件处理：未拦截 `ExitRequested`） | `Application::run`，`QuitMode::Explicit`（§4.9） |

前端侧的启动部分（主窗口 `appStore.initSettings`，见 §5.2-O）：生成屏幕组合 ID → 读 `auto_hide_enabled` / `top_on_wake` / `fixed_embed_desktop` / 背景 / 主题 → `get_screen_config` → 有记录则 `setPosition` + `setSize`（物理像素）后若 `isFixed` 再 `applyFixedMode()`；无记录则居中 380×600 逻辑并 `saveWindowState()` → 列屏幕配置 → 日历 / 字体设置。`MainView` 再装：`onCloseRequested`（保存状态）、`onMoved`/`onResized`（500ms 防抖保存）、4 个托盘事件、`data-imported`、`todo-font-changed`、`app-settings-changed`、`onFocusChanged`（聚焦刷新）、`sync-completed`、自动同步定时器、5s 变更轮询（`get_change_seq`）。

### 4.2 插件注册与 GPUI 替代

| 插件 | 配置 | 替代 |
|---|---|---|
| `tauri_plugin_single_instance::init` | 回调 L194-201；仅非隔离 | 自实现（§4.6） |
| `log_plugin(dev_data_dir)` | L98-117 | §4.8 |
| `tauri_plugin_opener::init()` | 仅前端用 | GPUI `open_url` / `reveal_path` |
| `tauri_plugin_notification::init()` | notification.rs 用 | notify-rust（§6.4） |
| `tauri_plugin_dialog::init()` | 仅前端用 | rfd |
| `tauri_plugin_fs::init()` | 仅前端 `readTextFile` | std::fs |
| `tauri_plugin_autostart::init(LaunchAgent, Some(vec!["--autostart"]))` | 托盘 + 设置页 | auto-launch（§4.7） |

### 4.3 托盘菜单

菜单顺序（L308-328）：`add_todo` →（Windows：分隔线、`new_fence`、`toggle_fences`、`reset_fences`）→ 分隔线 `separator1` → `toggle_fixed` → `reset` → `open_settings` → `auto_start` → 分隔线 `separator2` → `quit`。

| id | 文案 | 类型 | 平台 | 行为（行号） | 勾选同步 |
|---|---|---|---|---|---|
| `add_todo` | 添加待办项 | MenuItem | 全部 | emit `tray-add-todo`（L362-367） | — |
| `new_fence` | 新增收纳箱 | MenuItem | Windows | `commands::spawn_create_fence(app)` → `async_runtime::spawn(create_fence_at_cursor)`（L368-371；"菜单回调里不能直接建窗（Windows 上会死锁）"） | — |
| `toggle_fences` | 显示/隐藏所有收纳箱 | MenuItem | Windows | `toggle_all_fences(app)`（L372-374） | — |
| `reset_fences` | 重置收纳箱位置 | MenuItem | Windows | `reset_all_fence_positions(app)`，失败仅 `warn`（L375-379） | — |
| `toggle_fixed` | 固定模式 | CheckMenuItem，初值 `is_fixed_mode()`（启动时恒 false） | 全部 | emit `tray-toggle-fixed`（L347-351），由主窗口前端执行切换 | `sync_tray_fixed_checked`（window.rs:97-101）在 `set_window_fixed_mode` 末尾 L1541、`set_window_desktop_mode` 进入 L1471、`leave_desktop_mode` L1388 按真实状态回写；经典固定与嵌入桌面都显示为勾选；**不依赖 muda 自动翻转** |
| `reset` | 重置位置 | MenuItem | 全部 | `reset_webview_window(main)`（→ `reset_window_impl`，§5.2-P），再 emit `tray-reset-window`（L352-361） | — |
| `open_settings` | 打开设置 | MenuItem | 全部 | 主窗口 `show()` + `set_focus()` 后 emit `tray-open-settings`（L380-387） | — |
| `auto_start` | 开机自启动 | CheckMenuItem，初值 `autolaunch.is_enabled()` | 全部 | 按当前状态 `enable()` / `disable()`，失败 `warn`；然后读 `is_enabled()` 实际值 `sync_tray_auto_start_checked`（L388-405） | 设置页切换后前端调 `sync_auto_start_state(enabled)`（window.rs:121） |
| `quit` | 退出 | MenuItem | 全部 | `app.exit(0)`（L406-408） | — |

托盘图标：`app.default_window_icon()`（来自 `bundle.icon`，Windows 为 `icons/icon.ico`）；`show_menu_on_left_click(false)`（左键不弹菜单）；
左键单击抬起：300ms 去抖后 `bring_main_window_to_front`（L412-425；以前的双击分支已合并）。右键弹菜单。

### 4.4 200ms 轮询线程（L437-470，普通 `std::thread`，非主线程）

```text
loop {
  sleep(200ms)
  tick_embedded_windows(&handle)                 // 每 tick：收纳箱 Explorer 重启重挂 / 样式自愈（与主窗口模式无关）
  ticks += 1; if ticks % 10 == 0 { tick_fence_screen_layouts(&handle) }   // 约 2s：屏幕组合变化 → 收纳箱换布局
  if is_desktop_mode()      { tick_desktop_mode(main) }                 // 嵌入桌面：Explorer 重启重挂 / 样式自愈
  else if is_fixed_mode()   { restore_if_minimized(main);               // 经典固定：被最小化立刻还原
                              tick_fixed_style(main);                   //   样式被 tao 抹掉时重申
                              tick_auto_hide(main) }                    //   贴边隐藏 / 唤起状态机
}
```

| 子任务 | 位置 | 读什么 | 写什么 | GPUI 版注意 |
|---|---|---|---|---|
| `tick_embedded_windows` | window.rs:1336-1371 | `EMBEDDED_WINDOWS` 锁内：`GetShellWindow`、每窗口 `IsWindow` / owner / `GWL_EXSTYLE` | 锁外对需要的窗口 `reassert_embedded_window`（`run_on_main_thread` 排队 `desktop_attach_hwnd`） | 在 GPUI 前台定时器里执行可直接调用 attach |
| `tick_fence_screen_layouts` | fence.rs:1162-1175 | 无收纳箱窗口时置 `LAST_SCREEN_ID=None`；否则 `current_screen_config_id` | 变化时 `apply_screen_switch`（L1178-1232） | 需 Win32 显示器枚举（§5.2-Q） |
| `tick_desktop_mode` | window.rs:1113-1145 | `DESKTOP_OWNER`、`IsWindow(owner)`、`GetShellWindow`、owner、ex style | 排队 `desktop_attach` | 同上 |
| `restore_if_minimized` | window.rs:618-624 | `is_minimized()` | `ShowWindow(SW_RESTORE)` + `SW_SHOW`（固定模式绕开 tao） | GPUI：`IsIconic` + `ShowWindow` |
| `tick_fixed_style` | window.rs:1150-1165 | ex style | `reassert_window_mode_state` | GPUI 下非必需（§5.3） |
| `tick_auto_hide` | window.rs:659-725 | `PENDING_AUTO_HIDE_RELEASE`、`AUTO_HIDE_STATE.enabled`、光标、窗口外框、所在显示器（**整块显示器**，不是工作区） | `set_position` 到隐藏位 / 锚点；置顶 / 撤置顶；重申样式 | 轮询线程拿不到 GPUI `Window`（非 `Send`）：建议整个 tick 放进 GPUI 前台任务（`cx.spawn` + `background_executor().timer(200ms)`），或在后台线程只用缓存的 HWND 做只读 Win32，写操作投递回前台 |

注意：Tauri 下轮询线程调 `window.outer_position()` 等会跨线程往返主线程（这也是缓存 `MAIN_HWND` 的原因，window.rs:61-66、786-802）。GPUI 下在后台线程对 GPUI 窗口调用 `SetWindowPos` 会同步给主线程发消息，主线程若在等后台则死锁；**推荐把 tick 放在前台执行**。

### 4.5 通知调度器（services/notification.rs:204-221）

`async_runtime::spawn` 一个任务：先 `sleep 5s`（等应用初始化）→ `catch_up_missed_repeats`（启动补发已过点未通知的重复提醒）→ 循环 `sleep_until_next_minute`（按 UNIX 时间对齐到下一整分）→ `check_and_send_notifications`（每次重新读 `notification_type`，`query_pending(due_reminders_sql)`，逐条 `deliver`）。详见 §6。

### 4.6 单实例

- 现状：`tauri-plugin-single-instance ~2.3.7` 第一个注册（L193-204），**开发隔离时不注册**。回调在**首个实例**里执行，参数为第二个实例的 `argv` / `cwd`：`argv` 含 `--autostart` → 什么都不做（开机自启拉起的重复实例不打扰）；否则 `log::info!` + `bring_main_window_to_front(app)`。第二个实例随后退出，不初始化其它插件、不碰数据库（避免两套提醒调度 / 同步循环）。
- 插件内部实现（据 tauri-plugin-single-instance 2.x Windows 源码记忆，本机 registry 无该 crate 源码，需核实）：`CreateMutexW("{identifier}-sim")`，已存在则 `FindWindowW("{identifier}-sic", "{identifier}-siw")` 并 `SendMessageW(WM_COPYDATA)` 发送 `cwd|argv...`，然后退出；首实例创建该隐藏窗口接收 `WM_COPYDATA` 后回调。
- GPUI 版：进程入口（建 GPUI `Application` 之前）做命名互斥量检测；转发 argv 可用 message-only 窗口 + `WM_COPYDATA`（GPUI 主线程有 Win32 消息循环）或命名管道 / `interprocess` 本地 socket（后台线程接收后投递到前台）。保留"`--autostart` 静默"与"开发隔离不启用"两条规则。

### 4.7 开机自启

- 插件：`tauri_plugin_autostart::init(MacosLauncher::LaunchAgent, Some(vec!["--autostart"]))`（L213-216），底层 `auto-launch 0.5.0`（Cargo.lock）。Windows 上写 `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`，值名为应用名（据 plugin-autostart 2.5 源码记忆为 `package_info().name` = `mini-todo`，需核实），值为 `"<exe>" --autostart`。
- **自启自愈**（L268-280）：setup 时 `is_enabled()` 为真且非开发隔离 → 再 `enable()` 一次，把注册表项刷新为当前 exe 路径（便携版目录移动 / 换安装版后旧路径失效）。
- 三个入口：托盘 `auto_start`（L388-405）、设置页前端 `enable/disable/isEnabled`（`SettingsView.vue:125/282/285`）+ `sync_auto_start_state`、启动自愈。
- GPUI 版：直接用 `auto-launch`（`AutoLaunchBuilder::new().set_app_name("mini-todo").set_app_path(exe).set_args(&["--autostart"])`），值名与参数保持一致可沿用老版本写下的注册表项。

### 4.8 日志（`log_plugin` L98-117）

| 项 | 值 |
|---|---|
| 目标 | 文件 + 标准输出 |
| 文件位置 | `TargetKind::LogDir { file_name: None }` → `%LOCALAPPDATA%\com.tauri-app.mini-todo\logs\mini-todo.log`（文件名取应用名）；开发隔离时 `TargetKind::Folder { path: <MINI_TODO_DATA_DIR>/logs }` |
| 级别 | `Info` |
| 轮转 | `max_file_size(2 * 1024 * 1024)` + `RotationStrategy::KeepSome(1)`（保留 1 份旧日志） |
| 时间 | `TimezoneStrategy::UseLocal` |
| 背景 | release 是 Windows 子系统程序无控制台（`main.rs:2`），日志必须落盘；业务代码统一 `log::info!/warn!/error!`（CLAUDE.md 注意事项 7） |

### 4.9 窗口关闭 / 退出行为

| 场景 | 现状 | GPUI 版注意 |
|---|---|---|
| 托盘「退出」 | `app.exit(0)`（L407） | `cx.quit()` |
| 主窗口关闭 | 主窗口无关闭按钮（TitleBar 无 close/minimize）；Alt+F4 时前端 `onCloseRequested` 只保存状态、不阻止（`MainView.vue:278`）；`run` 的事件闭包为空（L572-574），Tauri 默认"所有窗口关闭则退出"——有收纳箱窗口（`prevent_close`）时进程继续 | GPUI 在非 macOS 默认 `QuitMode::LastWindowClosed`（gpui-pre `app.rs:458-466`、`3181-3183`），托盘常驻应用需 `QuitMode::Explicit`；主窗口的 `on_window_should_close` 中保存状态 |
| 收纳箱关闭 | `CloseRequested → prevent_close`；删除走 `destroy()` | `window.on_window_should_close(cx, \|_, _\| false)`；删除用 `remove_window()` |
| 通知窗口 | `close_notification_window` → `close()` → `Destroyed` 释放槽位 | 窗口关闭回调里释放槽位 |
| 致命错误 | 数据库打不开：`log::error!` + `show_fatal_error`（Windows `MessageBoxW(MB_OK\|MB_ICONERROR)`，其它平台 `eprintln!`）+ `exit(1)`（L123-162） | 原样复制 |

### 4.10 开发隔离（debug 专用，release 不读）

`MINI_TODO_DATA_DIR`（`db/paths.rs:22-57`）：数据目录、日志 `<dir>/logs`、WebView2 用户数据、**不注册单实例**、**不做自启自愈**、托盘 tooltip "Mini Todo（开发版）"、放行 asset 协议目录。
`MINI_TODO_FENCES_ROOT`（`fence_fs/location.rs:11`）、`MINI_TODO_SCREEN_ID_FILE`（`fence.rs:788-795`）。GPUI 版保留前两类语义，WebView2 / asset 协议两条删除。

---

## 5. window.rs 详解（2489 行）

### 5.1 全局状态

| 静态量 | 行 | 类型 | 含义 | 写入点 |
|---|---|---|---|---|
| `IS_FIXED_MODE` | 24 | `AtomicBool` | 经典固定模式 | `set_window_fixed_mode` L1501；`set_window_desktop_mode` 进入时置 false L1439 |
| `IS_DESKTOP_MODE` | 35 | `AtomicBool` | 固定 + 嵌入桌面（与上互斥） | `set_window_desktop_mode` L1466；`leave_desktop_mode` L1379 |
| `DESKTOP_OWNER` | 44 | `AtomicIsize` | 主窗口挂上的 Progman 句柄，0 = 未挂 | `desktop_attach` L1045；`desktop_detach` L1105 |
| `TOP_ON_WAKE` | 51 | `AtomicBool`（默认 true） | 贴边唤起时临时置顶（轮询线程用的缓存） | `reload_runtime_prefs`、`set_top_on_wake` L1665、两个模式命令 L1458/L1503 |
| `PENDING_AUTO_HIDE_RELEASE` | 55 | `AtomicBool` | 偏好被关后欠一次窗口收尾，下个 tick 处理 | `reload_runtime_prefs` L159；`tick_auto_hide` L660 消费 |
| `TRAY_WAKE_GENERATION` | 59 | `AtomicU64` | 托盘唤起代次（600ms 后撤置顶只认最新一次） | `begin_tray_wake` L1781 |
| `MAIN_HWND` | 66 | `AtomicIsize` | 主窗口 HWND 缓存（setup 时取） | `remember_main_window` L805-817 |
| `DESIRED_TOPMOST` | 73 | `AtomicBool` | 固定模式下期望的置顶态（tao 内部认知会漂移） | `set_window_always_on_top` L915 |
| `TRAY_TOGGLE_FIXED` / `TRAY_AUTO_START` | 76 / 104 | `OnceLock<CheckMenuItem<Wry>>` | 托盘勾选项引用 | lib.rs:332/334 |
| `AUTO_HIDE_STATE` | 310 | `OnceLock<Mutex<AutoHideState>>` | 贴边隐藏状态机 | 多处 |
| `EMBEDDED_WINDOWS` | 1219 | `Mutex<BTreeMap<String, EmbeddedWindowState{hwnd, owner}>>` | 始终嵌入桌面的附属窗口（收纳箱）登记 | `embed_window_in_desktop` / `attach_embedded_window` / `release_embedded_window` |

**状态 → Win32 结果**（spec window-modes.md §3）：

| 模式 | IS_FIXED | IS_DESKTOP | owner（`GWLP_HWNDPARENT`） | ex style | `WS_MINIMIZEBOX` | Z 序 |
|---|---|---|---|---|---|---|
| 普通 | false | false | 0 | `WS_EX_APPWINDOW`，无 TOOLWINDOW | 有 | 普通顶层 |
| 经典固定 | true | false | 0 | `WS_EX_TOOLWINDOW`，无 APPWINDOW | 有 | 顶层；仅 `top_on_wake` 唤起时 `TOPMOST` |
| 固定 + 嵌入 | false | true | Progman（`GetShellWindow()`） | `WS_EX_TOOLWINDOW`，无 APPWINDOW，无 TOPMOST | **无** | 紧贴 Progman 之上（所有应用窗口之下、桌面图标之上），之后每次 `SetWindowPos` 被 tao `ALWAYS_ON_BOTTOM` 钩子压回 `HWND_BOTTOM` |

持久化：`settings.is_fixed`（经典或嵌入都为 true）、`settings.fixed_embed_desktop`（偏好）、`screen_configs.is_fixed`。**没有** `is_desktop` 列 / 键，"嵌入"= `is_fixed && fixed_embed_desktop`。

### 5.2 机制逐个说明

#### A. 窗口句柄获取与缓存（`window_hwnd` L786-802、`remember_main_window` L805-817）

- 目的：轮询线程每 200ms 需要主窗口 HWND；Tauri 的 `window_handle()` 在非主线程要跨线程往返主线程（注释"D3"）。
- 做法：主窗口 label 为 `"main"` 时优先用 `MAIN_HWND`（`IsWindow` 校验），否则 `raw_window_handle::HasWindowHandle` → `RawWindowHandle::Win32`。
- GPUI：`gpui::Window` 实现 `HasWindowHandle`（gpui-pre `window.rs:7572`）；`Window` 不是 `Send`，建窗后在前台取一次 `isize` 缓存即可（GPUI 使用 windows 0.62，本仓库 0.58，跨版本只传 `isize`）。

#### B. 任务栏样式（`apply_fixed_ex_style` L819-825 / `apply_fixed_ex_style_hwnd` L829-850）

- 目的：固定 / 嵌入时不进任务栏、不参与 Alt+Tab。
- Win32：`GetWindowLongW(GWL_EXSTYLE)` → fixed：`|= WS_EX_TOOLWINDOW, &= !WS_EX_APPWINDOW`；否则反之 → 有变化才 `SetWindowLongW` + `SetWindowPos(0,0,0,0, SWP_NOMOVE|SWP_NOSIZE|SWP_NOZORDER|SWP_NOACTIVATE|SWP_FRAMECHANGED)`（让任务栏立即生效）。
- 依赖 tao：tao 的 `apply_diff` 会整体重写 `GWL_EXSTYLE` 把它抹掉，所以调用时机必须排在 tao 操作之后。
- GPUI：Normal 窗口建窗即带 `WS_EX_APPWINDOW`（gpui-pre-windows `window.rs:492-511`），之后不再被框架改写；可直接调用本函数，无需排队。若要避免"可见 + APPWINDOW"的瞬间闪烁，可建窗 `show: false` → 改样式 → 再显示（GPUI `show:false` 时初始 placement 推迟到 `activate()` 才设置，见 §5.4，自行 `ShowWindow` 前需先 `SetWindowPos` 定位）。

#### C. 置顶（`win32_set_topmost` L861-886、`set_window_always_on_top` L914-920、`DESIRED_TOPMOST`）

- 目的：贴边唤起 / 托盘唤起时临时把固定窗口抬到最前（盖过全屏 / 最大化窗口）。
- Win32：固定模式下 `SetWindowPos(HWND_TOPMOST / HWND_NOTOPMOST, SWP_NOMOVE|SWP_NOSIZE|SWP_NOACTIVATE)`——只翻 `WS_EX_TOPMOST` 位、不带 `SWP_FRAMECHANGED`，不会冒出任务栏按钮；非固定走 tao `set_always_on_top`。
- 依赖 tao：tao 的 `set_always_on_top` 触发 `apply_diff` 覆写 ex style（注释引用 tao-0.34.5 `window_state.rs:439-440`）；改走 Win32 后 tao 内部 `ALWAYS_ON_TOP` 认知会漂移，后续任何 tao 操作可能按旧认知撤销置顶 → 用 `DESIRED_TOPMOST` 记期望值，`reassert_window_mode_state` 补回。
- 时序约束（D1）：退出固定 / 进入桌面模式时**先**撤置顶**再**清 `IS_FIXED_MODE`（L1435-1439、L1492-1498），否则落到 tao 通路成了 no-op，窗口解锁后一直置顶。
- GPUI：没有置顶 API，统一走 Win32；`DESIRED_TOPMOST` 用于对抗 tao 漂移的意义消失，但"期望置顶态"本身仍可作为状态保留。

#### D. 显示 / 还原（`win32_show` L894-911、`show_window` L923-928、`unminimize_window` L935-943、`restore_if_minimized` L618-624）

- 目的：固定 / 嵌入模式下显示或从最小化还原，不让任务栏图标闪现。
- Win32：`ShowWindow(SW_RESTORE)` / `ShowWindow(SW_SHOW)`；`unminimize_window` 先判 `is_minimized()`（`SW_RESTORE` 对最大化窗口会还原尺寸、对普通窗口会激活）。
- 依赖 tao：tao 的 `show`/`unminimize` 走 flag 通路覆写 ex style；tao 的 `MINIMIZED` flag 由窗口过程按 `WM_SIZE` 自行同步（注释引 tao `event_loop.rs:1286`），绕开不会漂移。
- GPUI：没有 show/hide API；`activate()` 会在 iconic 时 `ShowWindowAsync(SW_RESTORE)` 并强行前台（合成 Alt 键）。固定模式守护直接用 `IsIconic` + `ShowWindow`。

#### E. 经典固定模式（`set_window_fixed_mode` L1476-1544）

前端顺序（`appStore.applyFixedMode` / `applyNormalMode`，`appStore.ts:396-421`）：`setResizable(false)` → `invoke(fixedEmbedDesktop && IS_WINDOWS ? 'set_window_desktop_mode' : 'set_window_fixed_mode')`；退出时 `setResizable(true)` → 两个命令都以 false 调一遍（幂等）。

后端序列：

1. 取主窗口（label `"main"`，不取调用方窗口），不存在 `Err("主窗口不存在")`。
2. `fixed && is_desktop_mode()` → `leave_desktop_mode`（§F）。
3. `!fixed` → 先 `set_window_always_on_top(false)`（Win32 撤置顶，D1）。
4. `IS_FIXED_MODE.store(fixed)`；从库读 `auto_hide_enabled`、`top_on_wake` → `TOP_ON_WAKE`。
5. `!fixed` → 再走一次 `set_window_always_on_top(false)`（这次落到 tao，清 tao 自己记的置顶）。
6. 贴边状态：先取"藏起时的锚点"；fixed → `enabled = auto_hide_enabled`、清 hidden / docked_edge / 计时；否则整个状态复位。有锚点则 `set_position(anchor)` 拉回。
7. `reassert_window_mode_state`（排队补任务栏样式 + 期望置顶态）。
8. `sync_tray_fixed_checked(fixed)`。

固定模式的"不可拖拽、不可缩放"在前端：`setResizable(false)` + 不调用 `startDragging`（TitleBar 拖动区判定）；后端无锁定逻辑。

GPUI 注意：GPUI 的 `is_resizable` / `is_movable` 是**建窗时**写进窗口状态的（`WindowCreateContext`，命中测试 `events.rs:955-1015` 用它们决定 `HTCAPTION` / `HTTOP*` 等），**没有运行时 setter**。固定模式下禁止缩放需要：自行管理拖拽（不调 `start_window_move`）+ 用 `SetWindowLongW(GWL_STYLE)` 去 `WS_THICKFRAME` 或子类化 `WM_NCHITTEST`；或者像收纳箱那样主窗口建成 `is_resizable: false`、缩放用应用内手柄 + `SetWindowPos`。

#### F. 固定 + 嵌入桌面（`set_window_desktop_mode` L1399-1474、`desktop_host` L1013-1028、`desktop_attach(_hwnd)` L1039-1077、`desktop_detach` L1087-1106、`leave_desktop_mode` L1378-1389）

- 目的：窗口位于桌面图标之上、所有应用窗口之下；Win+D / Win+M / "显示桌面"时随 Progman 一起被抬起（Win11 24H2+ 抬起的是 Progman），不最小化、不进任务栏 / Alt+Tab。
- **方案**：顶层窗口 + `GWLP_HWNDPARENT = Progman`（owner）+ tao `always_on_bottom`（`WM_WINDOWPOSCHANGING` 钩子把每次 `SetWindowPos` 的 `hwndInsertAfter` 改成 `HWND_BOTTOM`）+ 去 `WS_MINIMIZEBOX` + `WS_EX_TOOLWINDOW`。系统保证 owned 窗口在 owner 之上，于是 `HWND_BOTTOM` 实际落在"紧贴 Progman 之上"。**不用 `SetParent`**（子窗口化后 DPI 与圆角退化，spike 否决）。
- `desktop_host()`：`GetShellWindow()`；空 → `None`（Explorer 未运行，等下个 tick）；类名非 `Progman`（Win10 / Win11 ≤23H2 / 第三方 shell）→ 记日志但仍返回（Win+D 可能被盖住，已知限制）。`is_desktop_host_class` 纯函数 L956-958。
- `desktop_attach_hwnd(hwnd)`（L1055-1077，主窗口与收纳箱共用、不碰全局状态）：owner 不同才 `SetWindowLongPtrW(GWLP_HWNDPARENT, host)` → `apply_fixed_ex_style_hwnd(hwnd, true)` → `SetWindowPos(HWND_BOTTOM, SWP_NOMOVE|SWP_NOSIZE|SWP_NOACTIVATE [|SWP_FRAMECHANGED 仅 owner 刚换过])` → 返回 host。主窗口包装 `desktop_attach` 把 host 写 `DESKTOP_OWNER`。
- 进入序列（Windows，L1419-1473）：
  1. `enabled == false`：在桌面模式则 `leave_desktop_mode`；无论如何 `reassert_window_mode_state`（兜底前端紧跟的 `setResizable`）；返回。
  2. 处于经典固定 → `set_window_always_on_top(false)`（先撤置顶）→ `IS_FIXED_MODE=false` → 贴边状态复位（藏起则 `set_position(anchor)` 拉回）。
  3. `TOP_ON_WAKE` 按库刷新（之后切回固定时用）。
  4. **tao 通路在前**：`set_minimizable(false)` → `set_always_on_bottom(true)`（两次 `apply_diff`）。
  5. **再** `IS_DESKTOP_MODE.store(true)`（太早置位会让轮询线程把 attach 排在两次 `apply_diff` 之前 → TOOLWINDOW 被抹、任务栏闪）。
  6. `reassert_window_mode_state` → 排队 `desktop_attach`。
  7. `sync_tray_fixed_checked(true)`。
- 退出（`leave_desktop_mode` L1378-1389）：先 `IS_DESKTOP_MODE=false`（排队中的 tick / reassert 闭包据此跳过）→ `set_always_on_bottom(false)` → `set_minimizable(true)` → 排队 `desktop_detach` → `sync_tray_fixed_checked(false)`（直接切到经典固定时随后会再回写 true）。
- `desktop_detach`：`SetWindowLongPtrW(GWLP_HWNDPARENT, 0)` → `apply_fixed_ex_style(window, is_fixed_mode())`（**按执行时**的模式：嵌入 → 经典固定时保持 TOOLWINDOW，不闪）→ `SetWindowPos(HWND_TOP, …|SWP_FRAMECHANGED)` → `DESKTOP_OWNER=0`。必须排在 `set_always_on_bottom(false)` 之后，否则 `HWND_TOP` 被钩子改回 `HWND_BOTTOM`。
- 非 Windows：`enabled=true` 回退 `set_window_fixed_mode(true)`（D5：同步来的偏好不能让固定失败）；`false` → `Ok(())`。
- `set_fixed_embed_desktop`（L1563-1583）：写键；非 Windows 直接返回；`enabled && is_fixed_mode()` → `set_window_desktop_mode(true)`；`!enabled && is_desktop_mode()` → `set_window_fixed_mode(true)`。主窗口前端收到 `app-settings-changed{fixedEmbedDesktop}` 只重载值、**不再应用模式**（避免与后端排队的 attach/detach 竞争）。
- `bring_main_window_to_front` 在桌面模式下（L1726-1731）：只 `show_window` + `set_focus` + 重申，不碰置顶（点击激活也会被钩子压回底部）。
- **GPUI 移植要点**：
  - GPUI 没有 `always_on_bottom`，**必须自己实现** `WM_WINDOWPOSCHANGING` 钩子：对主窗口 / 收纳箱 HWND 调 `SetWindowSubclass`（comctl32，`windows` crate 的 `Win32_UI_Shell` feature，已在 Cargo.toml 中）安装子类过程，在钩子标志开启且 `WINDOWPOS.flags` 不含 `SWP_NOZORDER` 时把 `hwndInsertAfter` 改为 `HWND_BOTTOM`（tao 的实现即"标志开启时改 `hwndInsertAfter`"；window.rs:1291 注释"tao 的 `ALWAYS_ON_BOTTOM` 钩子只改 `hwndInsertAfter`，带 `SWP_NOZORDER` 时无效"）。子类过程运行在主线程窗口过程里，读标志应用 `AtomicBool`。
  - GPUI 没有运行时 `set_minimizable`：嵌入时用 `SetWindowLongW(GWL_STYLE, style & !WS_MINIMIZEBOX)`，退出时加回（GPUI 只在 `toggle_fullscreen` 写 `GWL_STYLE`，不会冲掉）。
  - 因为 GPUI 不再整体重写 ex style，进入 / 退出的"tao 在前、标志在后、Win32 排队"顺序约束可简化为同步执行：设标志 → 去 MINIMIZEBOX → owner → ex style → `HWND_BOTTOM`；退出：关钩子标志 → 加回 MINIMIZEBOX → owner=0 → ex style（按目标模式）→ `HWND_TOP`。
  - GPUI `activate()`（`window.rs:815`）会 `SetActiveWindow/SetFocus/SetForegroundWindow` 并合成 Alt 键输入——对嵌入窗口调用会触发 Z 序变化，依赖上面的钩子压回底部。
  - 子窗口（编辑器、设置等）在 Tauri 中以主窗口为 owner（`parent: appWindow`）；GPUI 的 Normal 窗口无 owner 且带 `WS_EX_APPWINDOW`。若要保持"子窗口不进任务栏 / 随主窗口"行为，需建窗后自行设 `GWLP_HWNDPARENT` 与 ex style。

#### G. Explorer 重启重挂（`needs_reattach` L971-981、`tick_desktop_mode` L1113-1145）

- 纯函数：`shell_now == 0 → false`；否则 `cached_owner == 0 || !owner_alive || shell_now != cached_owner || current_owner != cached_owner`（6 个单测）。
- tick：读 `DESKTOP_OWNER`、`IsWindow(owner)`、`GetShellWindow()`、`GetWindowLongPtrW(GWLP_HWNDPARENT)`、`GetWindowLongW(GWL_EXSTYLE)`；`needs_reattach || (shell_now != 0 && lacks_tool_window_style(ex))` → `run_on_main_thread(if is_desktop_mode() { desktop_attach })`（闭包里再核对模式）。实测 Explorer 被杀约 1.5s 内重挂，主窗口不随旧 owner 销毁。
- GPUI：Explorer 重启检测**仍然必要**；`lacks_tool_window_style` 分支变为防御性。

#### H. 样式自愈与重申（`lacks_tool_window_style` L994-996、`tick_fixed_style` L1150-1165、`reassert_window_mode_state` L1178-1199）

- `lacks_tool_window_style(ex)`：`ex & 0x80 == 0 || ex & 0x40000 != 0`（位值写死，全平台可测）。
- `reassert_window_mode_state`：`run_on_main_thread` 排队；闭包执行时：桌面模式 → `desktop_attach`；否则 `apply_fixed_ex_style(is_fixed_mode())`，固定时再 `win32_set_topmost(DESIRED_TOPMOST)`。调用点：模式切换、贴边 Hide/Restore 之后（`set_position` 在最大化态会经 `apply_diff`）、`reset_window_impl`（`set_resizable(true)` 之后）、托盘唤起、`set_auto_hide_enabled` / `set_top_on_wake` 关闭时、`release_auto_hide`。
- 背景（spec Ordering rule 7）：Tauri 同步命令跑在主线程，其中的 `run_on_main_thread` **就地执行**而不是排队；启动时主线程处在 WebView2 嵌套消息泵里，前端先发出的异步 `setResizable` 可能晚于同步命令执行，`apply_diff` 抹掉 TOOLWINDOW 且无人补回。于是 200ms 轮询按**实际**样式自愈（从非主线程排队一定排在已入队的 tao 操作之后）。
- GPUI：这一整套是 tao 专属问题，GPUI 下非必需；保留 `lacks_tool_window_style` 检查的成本只是每 tick 一次 `GetWindowLongW`。

#### I. 收纳箱的嵌入登记（L1201-1371）

- `embed_window_in_desktop(window)`：取 HWND，`EMBEDDED_WINDOWS[label] = {hwnd, owner: 0}`，然后 `reassert_embedded_window`（排队 `attach_embedded_window`）。
- `attach_embedded_window`：执行时重查登记表与 `IsWindow`（排队期间可能已删除），`desktop_attach_hwnd` 成功则回写 owner。
- `release_embedded_window(label)`：`Destroyed` 事件里注销。
- `tick_embedded_windows`：锁内只做只读 Win32 调用并收集 label，锁外排队重挂（**持锁时绝不 `run_on_main_thread`**）。
- 前提：收纳箱 tao flag 建窗时一次给齐（`always_on_bottom/minimizable(false)/skip_taskbar`），之后只有 `show()`/`hide()` 会 `apply_diff`，必须紧跟 `reassert_embedded_window`。
- GPUI：登记表可改为 app 层的 `HashMap<fence_id, isize>`；需要同一个 `WM_WINDOWPOSCHANGING` 子类钩子；显示 / 隐藏直接 `ShowWindow(SW_SHOWNOACTIVATE / SW_HIDE)`（不会改 ex style，无需重挂，但重挂幂等无害）。

#### J. 一次性设位置与尺寸（`set_window_bounds_physical` L1293-1323）

- Windows：一次 `SetWindowPos(hwnd, null, x, y, w, h, SWP_NOZORDER|SWP_NOACTIVATE)`（拖左 / 上边缩放时位置尺寸同时变，分两步会抖）；带 `SWP_NOZORDER` 时 tao 的底部钩子不生效、不经 `apply_diff`；最小尺寸仍由 tao `WM_GETMINMAXINFO` 兜底。非 Windows：`set_position` + `set_size`。
- GPUI：GPUI 公开 API 只有 `Window::resize(size)`（`window.rs:2892`，内部异步 `SetWindowPos(SWP_NOMOVE)`），**没有 set_position / set_bounds**。主窗口恢复位置、贴边隐藏、重置、收纳箱摆放、通知窗口校正全部需要这个 Win32 函数。最小尺寸：GPUI 在窗口过程里处理 `WM_GETMINMAXINFO`（gpui-pre-windows `events.rs:101`、`209-217`，`ptMinTrackSize = window_min_size × 缩放`），但 `min_size` 是建窗时写入的不可变字段（`window.rs:48/534`），没有运行时 setter。收纳箱折叠 / 展开时的 `set_min_size`（160×32 ↔ 160×160 逻辑）无法照搬：建窗时取较小值（160×32）并在应用逻辑里夹紧展开态，或在自建子类过程中改写 `WM_GETMINMAXINFO`。

#### K. 贴边自动隐藏与唤起（L199-725，仅经典固定模式）

| 常量 | 值 | 行 |
|---|---|---|
| `EDGE_SNAP_THRESHOLD_PX` | 10 | 199 |
| `EDGE_HIDE_DELAY` | 420ms | 200 |
| `TRAY_WAKE_KEEP_VISIBLE` | 3s | 205 |
| `TRAY_WAKE_TOPMOST_DURATION` | 600ms | 209 |
| `HIDDEN_VISIBLE_STRIP_PX` | 4 | 210 |
| `WAKE_HOTZONE_WIDTH_PX` | 2 | 211 |
| `WAKE_RANGE_PADDING_PX` | 40 | 212 |

- 状态 `AutoHideState`（L259-286）：`enabled, hidden, cursor_inside_window(默认 true), docked_edge, monitor_bounds, anchor_position, anchor_size, edge_stick_started_at, keep_visible_until`。
- 纯函数：`detect_docked_edge`（四边距离 ≤10px 取最近）、`calc_anchor_and_hidden_position`（锚点夹在显示器内；隐藏位只露 4px）、`cursor_in_wake_range`（贴边 2px 热区，沿边方向窗口两侧各放宽 40px）、`should_wake_hidden_window`、`evaluate_auto_hide_transition`（L522-612）：
  - 隐藏态：光标进唤起范围（无光标时看 `cursor_inside_window`）→ `Restore{anchor}`。
  - 显示态：未贴边 → 清计时；换边 → 重新计时；光标在窗口内**或唤起范围内**（滞回 D2）→ 续期并清托盘宽限；托盘宽限期内 → 续期；贴边满 420ms → `Hide{anchor, hidden, size, edge, monitor}`。
- 平台取值：`get_cursor_position`（Windows `GetCursorPos`，其它平台 None）、`get_window_rect`（`outer_position/outer_size` 物理）、`get_monitor_bounds`（`current_monitor` → 退 `primary_monitor`，**整块显示器 position/size，不是工作区**）。
- `tick_auto_hide`：先处理 `PENDING_AUTO_HIDE_RELEASE`（`release_auto_hide`：拉回锚点、撤置顶、重申）；`enabled` 为假直接返回（D3：不白跑跨线程查询）；Hide → `set_position(hidden)`，失败回滚状态，成功 `set_window_always_on_top(false)` + 重申；Restore → `set_position(anchor)`，失败恢复 hidden，成功且 `TOP_ON_WAKE` → 置顶 + 重申。
- `get_window_persist_state` 在隐藏态返回锚点而非屏幕外位置（L1812-1821），保存窗口状态不会存成屏幕外坐标。
- `should_stay_on_top()`（L185-190）：`TOP_ON_WAKE && enabled && !hidden && docked_edge.is_some()`。
- GPUI：纯状态机原样进 core；`set_position` 用 §J 的 Win32；置顶用 §C；光标 `GetCursorPos`；显示器边界用 `MonitorFromWindow` + `GetMonitorInfoW.rcMonitor`。

#### L. 托盘唤起（`bring_main_window_to_front` L1721-1778）

1. 桌面模式：`show_window` + `set_focus` + 重申，返回。
2. 贴边隐藏开着：设 `keep_visible_until = now + 3s`、清计时；藏起中则清 hidden 并 `set_position(anchor)`。
3. `unminimize_window` → `show_window` → `set_window_always_on_top(true)` → `set_focus`（只走 `SetForegroundWindow`，不触发 `apply_diff`）→ 重申。
4. `begin_tray_wake()` 取代次，起线程 sleep 600ms：仍是最新代次且 `!should_stay_on_top()` → 撤置顶 + 重申。
- 调用方：托盘左键单击、单实例回调。GPUI：`SetForegroundWindow` 可能被前台锁限制，GPUI 的 `activate()` 用合成 Alt 键绕过（`window.rs:833-860`），可复用该思路或直接调用 `activate_window()`（注意它会顺带 `set_window_placement`）。

#### M. 运行时偏好缓存（`reload_runtime_prefs` L141-161、`apply_runtime_prefs` L165-180）

- 读 `top_on_wake`、`auto_hide_enabled` → `TOP_ON_WAKE.swap`、`state.enabled = auto_hide_enabled && is_fixed_mode()`（新开启时清贴边计时）；有功能被关掉 → `PENDING_AUTO_HIDE_RELEASE=true`，由下个 tick 收尾。
- 调用点：启动、`save_settings`、导入（data.rs:245）、同步应用了远端设置（sync_cmd.rs:605）。GPUI：app 层持有这份缓存；core 的导入 / 同步通过返回值告知（§1.2）。

#### N. 最小化守护（`restore_if_minimized`）

经典固定模式每 tick：`is_minimized()` → `unminimize_window` + `show_window`（固定模式无任务栏入口，被 Win+D / Win+M 最小化后点不回来）。嵌入模式下去掉了 `WS_MINIMIZEBOX` 不会被最小化。

#### O. 主窗口状态保存与恢复（后端命令 + 前端逻辑）

| 环节 | 位置 | 内容 |
|---|---|---|
| 保存触发 | `MainView.vue:278-290` | `onCloseRequested`（立即）、`onMoved`/`onResized`（500ms 防抖）、模式切换后（`appStore.toggleFixedMode` 末尾）、托盘重置后 |
| 保存流程 | `appStore.saveWindowState`（`appStore.ts:~505-548`） | `get_window_persist_state`（物理外框，隐藏态取锚点）→ 确保有屏幕组合 ID → `save_screen_config({configId, displayName, windowX, windowY, windowWidth, windowHeight, isFixed})` → `save_settings({isFixed, fixedEmbedDesktop, windowPosition, windowSize, autoHideEnabled, topOnWake, windowBgColor, windowBgAlpha, textTheme})`（前端必须先 load 真值，否则把 `topOnWake` / `fixedEmbedDesktop` 刷成默认） |
| 恢复流程 | `appStore.initSettings`（`appStore.ts:155-250`） | 生成 ID → 读偏好 → `get_screen_config(id)`：有 → `setPosition(Physical)` + `setSize(Physical)`，再 `isFixed` 时 `applyFixedMode()`（先定位后固定）；无 → `isFixed=false`，主显示器居中 380×600 逻辑（`position + (size - 380*scale)/2`），`saveWindowState()` 建初始记录 |
| 显示名 | `generateScreenConfigDisplayName`（`appStore.ts:139-152`） | `unknown` → "未知配置"、`legacy` → "旧版配置"；否则 `"{n}屏: {WxH} ({scale}%) + …"` |
| 注意 | — | 设置窗口里绝不能调用 `saveWindowState`（会把设置窗口几何存成主窗口的），各设置命令都是"只写一个键" |

GPUI：整段逻辑从 TS 搬到 app 层；几何一律物理像素 + Win32（GPUI 的 `bounds()` 是逻辑像素）。

#### P. 重置窗口（`reset_window_impl` L1845-1887）

`clear_auto_hide_runtime_state()` → `set_window_always_on_top(false)`（否则唤起置顶永久残留）→ 主显示器：`margin_x = (size.width * 0.1 / scale) as i32`、`margin_y` 同理，`x = position.x + margin_x`、`y = position.y + margin_y`，`set_position(Physical(x, y))`，`set_size(Logical(380, 600))`，`set_resizable(true)` → `reassert_window_mode_state`（`set_resizable` 会重写 ex style）。托盘 `reset` 随后 emit `tray-reset-window`，前端退出固定模式并重新 `initSettings`。

#### Q. 屏幕配置（screen_configs）与组合 ID

- 表列：`id, config_id, display_name, window_x, window_y, window_width, window_height, is_fixed, created_at, updated_at`（`SCREEN_CONFIG_COLUMNS` L1894）。
- `save_screen_config`：`INSERT … ON CONFLICT(config_id) DO UPDATE SET display_name = COALESCE(?2, display_name), …, updated_at = datetime('now','localtime')`，返回保存后的行。
- 删除时连带删除该组合的 `fence_layouts`（`delete_screen_config_rows` L1998-2008，单测覆盖）。
- **组合 ID 规则**（前端 `appStore.generateScreenConfigId` `appStore.ts:106-134` 与 Rust `fence.rs::screen_config_id_of` L163-178 逐字一致）：
  - 输入：`availableMonitors()` 枚举顺序的（物理宽, 物理高, 缩放比例）
  - 缩放% = `round(scale × 100)`
  - 按像素面积**降序稳定排序**（同面积保持枚举顺序）
  - 输出 `"{显示器数}_{W}x{H}@{scale%}_{W}x{H}@{scale%}…"`；无显示器或读失败为 `"unknown"`
  - 例：`"2_2560x1440@125_1920x1080@100"`；单测 `screen_config_id_matches_the_frontend_format`
  - debug 下 `MINI_TODO_SCREEN_ID_FILE` 覆盖（仅 Rust 侧）
- GPUI：`PlatformDisplay` 没有缩放比例也没有物理尺寸（`platform.rs:606-641`），组合 ID 需要 Win32 枚举（`EnumDisplayMonitors` 顺序 + `GetMonitorInfoW.rcMonitor` 物理尺寸 + `GetDpiForMonitor(MDT_EFFECTIVE_DPI)/96`）；为了与库里已有记录匹配，枚举顺序与缩放取值应与 tao 一致（tao 的 `available_monitors` 也是 `EnumDisplayMonitors` 顺序，需在 Windows 上实测核对）。Rust 纯函数 `screen_config_id_of` 可直接复用，主窗口与收纳箱统一走它（不再有前端一份）。

#### R. 圆角（lib.rs:55-77 `setup_window_rounded_corners`）

`DwmSetWindowAttribute(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND)`；主窗口（setup）与每个收纳箱（fence.rs:880）。GPUI 未设置该属性（gpui-pre-windows 只在 `util.rs:140` 与背景外观处调用 DWM），建窗后自行调用即可；透明背景用 `WindowBackgroundAppearance::Transparent`（GPUI 支持，`window.rs:926`）。

### 5.3 为对抗 tao `apply_diff` / Tauri 线程语义而写的逻辑（GPUI 下的处理）

| 逻辑 | 位置 | 对抗的行为 | GPUI 下 |
|---|---|---|---|
| `reassert_window_mode_state` 排队重申 | window.rs:1178-1199 及约 12 个调用点 | `set_resizable` / `set_always_on_top` / 最大化态 `set_position` 等经 `apply_diff` 整体重写 `GWL_EXSTYLE` | GPUI 建窗后不写 `GWL_EXSTYLE` → 不需要；模式切换时同步设置一次即可 |
| `win32_set_topmost` 绕开 tao 置顶 + `DESIRED_TOPMOST` | 861-886、73、915 | tao 置顶 `apply_diff` + `SWP_FRAMECHANGED` 闪任务栏；tao 置顶认知漂移 | GPUI 无置顶 API，本来就只能 Win32；漂移问题消失 |
| `win32_show` / `unminimize_window` | 894-943 | tao `show`/`unminimize` 走 flag 通路 | GPUI 无 show API，直接 `ShowWindow` |
| 进入嵌入的顺序：`set_minimizable(false)` → `set_always_on_bottom(true)` → 再置 `IS_DESKTOP_MODE` → 排队 attach | 1460-1468 | 两次 `apply_diff`；轮询线程早排队会被抹 | GPUI 无运行时 minimizable / always_on_bottom → `GWL_STYLE` 去 `WS_MINIMIZEBOX` + 自建子类钩子；顺序约束消失 |
| `leave_desktop_mode` 先清标志再撤 tao flag 再排队 detach | 1378-1389 | `ALWAYS_ON_BOTTOM` 钩子会把 `HWND_TOP` 改回 `HWND_BOTTOM` | 自建钩子：先关钩子标志再 `HWND_TOP` |
| `desktop_detach` 按执行时 `is_fixed_mode()` 设样式 | 1087-1106 | 排队执行的时序 | 同步执行即可（按目标模式） |
| `tick_fixed_style` + `lacks_tool_window_style` | 1150-1165、994-996 | 同步命令里 `run_on_main_thread` 就地执行，晚到的 `SetResizable` 抹样式 | 竞态不存在；可保留为防御 |
| `tick_desktop_mode` 中的 `lacks_tool_window_style` 分支 | 1130-1131 | 同上 | 同上（`needs_reattach` 分支仍必须） |
| `tick_embedded_windows` 中的 `lacks_tool_window_style` 分支 | 1355-1356 | 收纳箱 `show()/hide()` 的 `apply_diff` | 用 `ShowWindow` 显示 / 隐藏不改 ex style；Explorer 重启分支仍必须 |
| `show()/hide()` 后紧跟 `reassert_embedded_window` | fence.rs:893-896、927-937 | 同上 | 不需要（幂等可留） |
| 收纳箱 tao flag 建窗时一次给齐、之后禁止改 | fence.rs:852-873；spec desktop-fences §3.1 | `set_resizable/set_minimizable/set_always_on_*/set_maximizable` 都会 `apply_diff` | GPUI 建窗参数 `is_resizable:false, is_minimizable:false, show:false` → Win32 打 owner / 样式 / 钩子 → 显示；不支持运行时改动本身就不是问题 |
| `set_window_bounds_physical` 一次 `SetWindowPos(NOZORDER)` | 1293-1323 | tao 两步 `set_position`/`set_size` 抖动 + 底部钩子 | GPUI 只能这样做（无 set_position） |
| `MAIN_HWND` 缓存 | 61-66、786-817 | 非主线程 `window_handle()` 往返主线程 | GPUI `Window` 非 `Send`，同样需要缓存 `isize`，或 tick 放前台 |
| `run_on_main_thread` 的使用（排队 attach / detach / reassert） | 1134、1182、1260、1385 | 必须排在主线程事件队列中 tao 操作之后 | GPUI 前台任务本身在主线程；直接调用 |
| 托盘 / 菜单回调里不建窗（转 `async_runtime::spawn`） | lib.rs:368-371、fence.rs:1035-1045 | Windows 上同步命令 / 菜单回调里建 WebView 窗口死锁 | GPUI：tray 回调里不碰 `App`，经 channel 投递到前台任务（§3.4） |
| `blocking()` / 带 owner HWND 的移动不在主线程 | fence_items.rs:136-159 | 系统 UAC / "文件正在使用"对话框以收纳箱窗口为父，会给主线程同步发消息，主线程等待则死锁 | **仍然适用**：GPUI 主线程同样运行窗口过程，移动必须放 `background_executor` / 线程 |

### 5.4 gpui-pre 0.3.9（Windows）平台层事实（源码核实，用于上面的判断）

| 事实 | 源码位置（`~/.cargo/registry/src/index.crates.io-*/`） |
|---|---|
| 建窗样式：`WindowKind::PopUp` → `WS_EX_TOOLWINDOW \| WS_EX_TOPMOST`、style `0`；其余（Normal / Floating）→ `WS_EX_APPWINDOW` + `WS_SYSMENU` [+`WS_THICKFRAME\|WS_MAXIMIZEBOX` 若 `is_resizable`] [+`WS_MINIMIZEBOX` 若 `is_minimizable`]；Dialog → `WS_EX_DLGMODALFRAME` + `WS_POPUP\|WS_CAPTION`，owner = `GetActiveWindow()` 并禁用之（模态）；默认再加 `WS_EX_NOREDIRECTIONBITMAP` | `gpui-pre-windows-0.3.9/src/window.rs:464-515` |
| 只有 Dialog 有 owner（`parent_hwnd`），其它窗口 `CreateWindowExW(..., parent=None)` | 同上 L464-477、L550-563 |
| 建窗后只在 `toggle_fullscreen` 写 `GWL_STYLE`；全库无 `GWL_EXSTYLE` 写入 | `window.rs:321/350`；`util.rs:81-85`（`set_window_long` 包装） |
| 不处理 `WM_WINDOWPOSCHANGING`（窗口过程分发表无此消息） | `events.rs:90-160` |
| `is_movable/is_resizable/is_minimizable` 建窗时存入状态，命中测试使用，无运行时 setter | `window.rs:520-545`、`events.rs:955-1015` |
| `Window` 实现 `raw_window_handle::HasWindowHandle` | `gpui-pre-0.3.9/src/window.rs:7572-7576`；`gpui-pre-windows/src/window.rs:595-603` |
| `Window` 公开几何 API：`bounds()`、`window_bounds()`、`resize(size)`、`start_window_move()`、`start_window_resize(edge)`、`activate_window()`、`minimize_window()`、`set_window_title`、`set_background_appearance`、`display(cx)`、`scale_factor()`；**无** set_position / show / hide / always_on_top | `gpui-pre-0.3.9/src/window.rs:2371-3091、6625-6640` |
| `PlatformWindow::resize` 与 `activate` 都通过 `executor.spawn` 异步执行 Win32（`SetWindowPos(SWP_NOMOVE)`；`activate` = `set_window_placement` + iconic 时 `SW_RESTORE` + `SetActiveWindow/SetFocus` + 合成 Alt 键 + 前台） | `gpui-pre-windows/src/window.rs:655-678、815-868` |
| `show: false` 建窗时初始 placement 存起来，`activate()` 时才 `SetWindowPlacement` | `window.rs:577-590、367-390` |
| `PlatformDisplay`：`id/uuid/bounds/visible_bounds/refresh_interval/default_bounds`（逻辑像素），**无缩放比例**；Windows 实现内部有 `scale_factor`、`physical_bounds`（`rcWork` 作 visible_bounds） | `gpui-pre-0.3.9/src/platform.rs:606-641`；`gpui-pre-windows/src/display.rs:27-160` |
| 退出策略：`QuitMode::Default` 在非 macOS 等于 `LastWindowClosed`；`Application::with_quit_mode` / `App::set_quit_mode` | `gpui-pre-0.3.9/src/app.rs:227-228、458-466、1955、3181-3183` |
| 系统通知：`App::set_app_identity(id, name)` + `App::show_system_notification(SystemNotification{tag,title,body,actions})`；Windows 实现注册 `HKCU\Software\Classes\AppUserModelId\{id}`（DisplayName）、`SetCurrentProcessExplicitAppUserModelID`、`CreateToastNotifierWithId`；toast 带 `<audio silent="true">`；错误只 `log_err`，无返回值 | `app.rs:1829-1845`；`gpui-pre-windows/src/system_notifications.rs`；`platform.rs:893-923` |
| 文件对话框：`prompt_for_paths(PathPromptOptions{files, directories, multiple, prompt})`、`prompt_for_new_path(directory, suggested_name)`，无过滤器 / 标题 | `platform.rs:2802-2811`；`app.rs:1904-1910` |
| `open_url` / `reveal_path` / `open_with_system` | `gpui-pre-windows/src/platform.rs:786、838、852` |
| 外部拖放：每个窗口 `RegisterDragDrop`（`IDropTarget` 实现） | `gpui-pre-windows/src/window.rs:1144-1160、1519-1522` |
| 依赖 `windows 0.62` / `windows-core 0.62` / `raw-window-handle 0.6` | `gpui-pre-windows-0.3.9/Cargo.toml:120-200` |
| 默认 feature `windows-manifest` 用 `embed-resource` 嵌入清单：PerMonitorV2 DPI、Common-Controls 6.0、SegmentHeap；GPUI 运行时不调用 `SetProcessDpiAwareness*` | `gpui-pre-0.3.9/Cargo.toml [features]`、`build.rs:22-35`、`resources/windows/gpui.manifest.xml` |
| 窗口关闭：`on_should_close` 回调返回 false 即拦截 `WM_CLOSE` | `events.rs:320-325`；`window.rs:6870 on_window_should_close` |
| 处理 `WM_GETMINMAXINFO`：`ptMinTrackSize` 来自建窗参数 `window_min_size`（不可变） | `gpui-pre-windows/src/events.rs:101、209-217`；`window.rs:48、534` |

---

## 6. services/notification.rs

### 6.1 纯逻辑（可原样复制，含 18 个测试）

| 项 | 行 | 说明 |
|---|---|---|
| `acquire_slot` / `release_slot` + `NOTIFICATION_SLOTS` | 23、32-45 | 取最小空闲槽位；窗口销毁归还（重复归还无害） |
| `WorkArea` / `NotificationSize::{from_physical, differs_from}` | 47-89 | 物理 → 逻辑；差 ≥0.5 逻辑像素才算不同 |
| `REQUESTED_SIZE` 320×120、`NOTIFICATION_MARGIN` 20、`SPACING` 10 | 26-29、65-68 | 逻辑像素 |
| `LAST_ACTUAL_SIZE` + `last_actual_size` / `remember_actual_size` | 71、91-97 | 记住上次真实尺寸，后续通知直接按它摆 |
| `notification_position(area, size, slot)` | 103-118 | rows = `floor((h - 2·margin + spacing)/(size.h+spacing)).max(1)`，cols 同理；`slot %= rows·cols`；`col = slot / rows`、`row = slot % rows`；`x = area.x + area.w - margin - size.w - col·(size.w+spacing)`；`y = area.y + area.h - margin - size.h - row·(size.h+spacing)` —— 右下角起向上堆叠、满列向左换列、全满取模复用 |
| `parse_notify_at` | 121-123 | `parse_local_datetime_with(s, DefaultTime::Notify)`（兼容空格 / `T`，仅日期 09:00） |
| `PendingNotification` / `PENDING_COLUMNS` / `pending_from_row` | 126-154 | 重复相关列类型错误按"未设置" |
| `due_reminders_sql` | 158-168 | `completed=0 AND notified=0 AND notify_at IS NOT NULL AND datetime(notify_at, '-' \|\| COALESCE(notify_before,0) \|\| ' minutes') <= datetime('now','localtime') ORDER BY datetime(notify_at), id` |
| `missed_repeats_sql` | 171-181 | 启动补发：`repeat_enabled=1 AND notified=0 AND datetime(notify_at) <= now` |
| `query_pending` | 184-198 | 单行读取失败只记日志跳过 |
| `deliver` 骨架 | 261-284 | 发送失败：只记日志、**不推进**（下一分钟重试）；成功：重复 → `advance_repeat`，否则 `mark_as_notified`；推进失败只记日志 |
| `mark_as_notified` | 455-464 | `notified=1` + `SQL_SET_UPDATED_AT`（随同步传播） |
| `advance_repeat` | 471-504 | `notify_at` 解析失败 → 标记已通知并报错（避免每分钟弹）；月重复无 `repeat_month_day` 时以当前日为锚并写回 |
| `calc_next_occurrence` / `next_weekly` / `next_monthly` / `last_day_of_month` | 507-624 | 循环推进直到 > now（上限 366×5 次）；周：星期 1-7 列表，跨周跳 `interval-1` 周；月：锚点日钳位到月末 |
| `sleep_until_next_minute` 的时间计算 | 224-244 | 对齐到下一个 UNIX 整分（可改成返回 `Duration` 的纯函数） |
| `get_notification_type` | 287-296 | 读 `notification_type`，默认 `system` |

### 6.2 系统通知边界（`send_system_notification` L299-315）

`app_handle.notification().builder().title(title).body(description.unwrap_or("待办事项提醒")).show()`，返回 `Result`，失败时 `deliver` 不推进。
调用线程：Tauri 异步运行时的 worker 线程（非主线程）。Windows 上 `tauri-plugin-notification 2.3.3` → `notify-rust 4.11.7` → `tauri-winrt-notification 0.7.2`（Cargo.lock）；
据 plugin 源码记忆（本机无源码，需核实）：仅在 exe 不位于 `target\debug` / `target\release` 时设置 AUMID = `identifier`（`com.tauri-app.mini-todo`），否则用 PowerShell 的 AUMID；安装包创建的开始菜单快捷方式带该 AUMID。

### 6.3 应用内通知窗口（`send_app_notification` L318-387 + `fit_to_actual_size` L392-418 + `primary_work_area` L424-451）

| 步骤 | 内容 |
|---|---|
| label | `notification_{NOTIFICATION_COUNTER++}` |
| 槽位 | `acquire_slot` |
| 工作区 | 主显示器 `work_area()`（物理，扣任务栏）÷ `scale_factor` → 逻辑；取不到工作区（0 尺寸）退回整块显示器；取不到主显示器按 1920×1080@1 |
| 初始位置 | `notification_position(area, last_actual_size() 或 REQUESTED_SIZE, slot)`（逻辑像素） |
| URL | `index.html#/notification?title=…&description=…&label=…`（urlencoding） |
| Builder | `.title("通知") .inner_size(320, 120) .position(x, y) .decorations(false) .always_on_top(true) .resizable(false) .skip_taskbar(true) .focused(false) .visible(true)`，非 macOS `.transparent(true)` |
| 失败 | 归还槽位，返回 `Err` → 不推进提醒 |
| 事件 | `Destroyed` → `release_slot`；`Resized(inner)` → `fit_to_actual_size(Some(inner))` |
| 校正 | 建好后立即 `fit_to_actual_size(None)`：按真实外框 ÷ 缩放算逻辑尺寸、记住；与假设尺寸不同则重算位置 `set_position(LogicalPosition)`（Linux/GTK 会把 320×120 撑成 320×200） |
| 前端 | `NotificationView.vue`：显示标题 / 描述，点击任意处或关闭按钮 → `close_notification_window(label)`；无自动消失 |

GPUI 映射：`WindowKind::PopUp`（正好是 `WS_EX_TOOLWINDOW | WS_EX_TOPMOST`，不进任务栏、置顶）+ `focus: false` + `is_resizable: false` + 透明背景；工作区用 GPUI `primary_display().visible_bounds()`（逻辑像素，已是 rcWork ÷ 缩放）即可直接套 `notification_position`；GPUI 窗口尺寸由我们控制，不会被 GTK 撑高，`fit_to_actual_size` 可简化。窗口关闭回调里 `release_slot`。

### 6.4 系统通知 crate 选型

| 方案 | 线程 | 错误返回 | AUMID / 注册 | 声音 | 跨平台 | 说明 |
|---|---|---|---|---|---|---|
| **notify-rust 4.x**（推荐） | 任意线程（现状即在 tokio worker 上调用） | `show()` → `Result` | 需 `.app_id("com.tauri-app.mini-todo")`；未注册的 AUMID 不显示 → 安装器创建带该 AUMID 的开始菜单快捷方式，或启动时写 `HKCU\Software\Classes\AppUserModelId\{id}`（GPUI 的做法） | 默认提示音（与现状一致） | Windows（tauri-winrt-notification）/ Linux（zbus/dbus，CI 与 e2e 在 Linux）/ macOS | 与现在 `tauri-plugin-notification` 的实际链路相同，行为差异最小；调度器可继续在后台线程运行，`deliver` 的"失败不推进"语义保持 |
| tauri-winrt-notification 0.7/0.8 直接用 | 任意线程 | `Result` | 同上（`Toast::new(app_id)`，有 `POWERSHELL_APP_ID` 兜底） | 默认 | 仅 Windows | 少一层封装；Linux 需另行处理 |
| GPUI `App::show_system_notification` | **仅前台**（需要 `&App`） | 无（只 `log_err`） | `set_app_identity` 自动注册 HKCU AUMID | 强制静音 | 取决于各平台实现 | 零新依赖；但调度器要把每条提醒投递回前台，且无法判断失败、不推进语义会变 |

---

## 7. fence.rs / fence_items.rs

### 7.1 收纳箱窗口创建（`open_fence_window` fence.rs:833-898）

| 参数 | 值 | 原因（spec desktop-fences §3.1） | GPUI 映射 |
|---|---|---|---|
| label / URL | `fence-{id}` / `index.html#/fence?id={id}` | 窗口标识 | app 层 `HashMap<i64, WindowHandle<FenceView>>` |
| `title` | `fence.title` | — | `titlebar.title` |
| `position` / `inner_size` | 物理 ÷ 目标显示器缩放（逻辑）；建好后 `set_position(Physical)` → `set_size(Physical)` 校正 | 混合 DPI 差几个像素 | 建窗后 Win32 `SetWindowPos` 按物理像素定位（先位置后尺寸，或一次性） |
| `min_inner_size` | `(160, 展开 ? 160 : 32)` 逻辑 | 折叠 / 展开时调整 | `window_min_size` 建窗后不可变（GPUI 用它答复 `WM_GETMINMAXINFO`），建议取 160×32 并在应用内夹紧展开态（§5.2-J） |
| `decorations(false)` `transparent(true)` `shadow(false)` | 无边框透明，圆角靠 DWM | — | `titlebar: None`、`window_background: Transparent`、DWM 圆角 |
| `skip_taskbar(true)` `visible(true)` `focused(false)` | 建窗不闪任务栏 | — | `show: false` → 打 TOOLWINDOW / owner → `ShowWindow(SW_SHOWNOACTIVATE)`；`focus: false` |
| `always_on_bottom(true)` | 之后任何 `SetWindowPos` 被压回 `HWND_BOTTOM` | — | 子类钩子（§5.2-F） |
| `minimizable(false)` / `maximizable(false)` / `resizable(false)` | Win+M 不最小化；防最大化；无 `WS_THICKFRAME`，缩放走前端手柄 + `set_fence_bounds` | — | `is_minimizable: false`、`is_resizable: false`（同时去掉 `WS_MAXIMIZEBOX`）；拖动用 `start_window_move`（仅未锁定且超过 3px 阈值） |
| **不设** `focusable(false)` | 必须可激活（双击打开的程序要拿前台） | — | 不加 `WS_EX_NOACTIVATE` |
| 事件 | `CloseRequested → prevent_close`；`Destroyed → release_embedded_window` | Alt+F4 关不掉 | `on_window_should_close → false` |
| 之后 | `embed_window_in_desktop`；`FENCES_HIDDEN` 时 `hide()` + 重挂 | — | 登记 HWND + attach；隐藏用 `ShowWindow(SW_HIDE)` |

必须在 async 上下文建窗（Windows 上同步命令 / 菜单回调里建 WebView 窗口死锁）：托盘走 `spawn_create_fence`（`async_runtime::spawn`），启动走 `restore_fences`（`async_runtime::spawn`）。

### 7.2 几何与布局

| 函数 | 行 | 性质 | 内容 |
|---|---|---|---|
| `PxRect` 及方法 | 107-144 | 纯 | 物理像素矩形、交叠面积、膨胀、点距 |
| `screen_config_id_of` | 163-178 | 纯 | §5.2-Q |
| `effective_rect` | 181-191 | 纯 | 当前组合记录 → `SESSION_RECTS` → `fences.x/y/w/h` |
| `layout_after_switch` / `committed_rect` | 195-221 | 纯 | 切换后几何 / 落库几何（折叠视图保留展开高度） |
| `next_default_title` | 224-233 | 纯 | `收纳箱` / `收纳箱 2` … |
| `default_fence_rect` | 238-290 | 纯 | 右上角起找不重叠位置，找不到斜向叠放 |
| `pick_work_area` / `clamp_into` / `view_rect` / `title_scale` / `fence_view` | 294-377 | 纯 | 折叠 = 标题栏高 `round(32 × 目标显示器缩放)`；展开下方不够时整体上移；按标题栏所在工作区夹回 |
| `reset_layout` | 381-417 | 纯 | 托盘重置排布 |
| `normalize_title` / `normalize_color` | 420-444 | 纯 | 标题去控制字符、非空、≤40 字符；颜色 `#RRGGBB` 大写 |
| `monitor_work_rect` / `monitor_areas` | 724-762 | Tauri `Monitor` | 主显示器排第一；工作区 0 尺寸时退回整块 → **改 Win32** |
| `current_screen_config_id` | 765-783 | Tauri | → Win32 枚举 + 纯函数 |
| `Placement::{current, for_screen, rect}` | 798-826 | DB + 纯 | `for_screen(db, id)` 可进 core |
| `apply_fence_view` | 904-921 | Tauri 窗口 | 展开：位置 → 尺寸 → 抬最小高度；折叠：降最小高度 → 位置 → 尺寸 |
| `set_fences_visible` / `toggle_all_fences` | 927-944 | Tauri 窗口 | `FENCES_HIDDEN` 运行时状态，不持久化 |
| `create_fence_at_cursor` | 947-1025 | 混合 | 光标所在显示器工作区（`cursor_position` + `monitor_from_point`，退主显示器，再退 1920×1080@1）；尺寸 `(88×4+8×2) × (32+96×3+8×2)` DIP；先建文件夹（blocking）→ 插库（失败回滚文件夹）→ `save_layout` → 建窗（失败删记录 + 回滚文件夹）→ `ensure_fence_watcher` |
| `restore_fences` | 1050-1088 | 混合 | 逐个 `ensure_fence_folder` → `open_fence_window(placement.rect)` → 有收纳箱则 `ensure_fence_watcher` → `spawn_startup_reconcile` |
| `reset_all_fence_positions` | 1091-1155 | 混合 | 主显示器工作区 `reset_layout` → 事务写 `fences` + 当前组合 `fence_layouts` → 清 `SESSION_RECTS` → `apply_fence_view` → 若隐藏则全部显示 |
| `tick_fence_screen_layouts` / `apply_screen_switch` | 1162-1232 | 混合 | 组合变化：有记录用记录，没有沿用窗口当前外框（高度 < 2×标题栏视为折叠视图）记进 `SESSION_RECTS`（不写库）→ `apply_fence_view` |

### 7.3 几何命令链路

```text
用户拖动标题栏（>3px，未锁定）→ startDragging → onMoved/onResized（闸门内续期 3000ms）→ 停 300ms → commit_fence_geometry(id, expandedView)
用户拖缩放手柄 → 每帧 set_fence_bounds(x,y,w,h)（SetWindowPos，不写库）→ pointerup → commit_fence_geometry
折叠按钮 / 悬停临时展开 → update_fence({collapsed})（只写库）+ set_fence_view(expanded)（只改窗口）
commit_fence_geometry → 读外框（物理）→ committed_rect → save_fence_geometry（fences + 当前组合 fence_layouts，同一事务）→ SESSION_RECTS.remove
```

不落库：启动夹回、显示器变化导致的系统挪窗、`set_fence_view`、屏幕组合切换、`set_fence_bounds` 本身（前端 `GeometryCommitGate`，`pc/src/utils/fence.ts`）。

### 7.4 目录监听 → emit_to 链路

```text
ensure_fence_watcher(app)            fence_items.rs:421-442   （首个收纳箱出现时：启动恢复有收纳箱 / 新建后）
  └ fence_fs::watch_fence_root(cb)   watch.rs:157-161         notify 递归监听 %USERPROFILE%\MiniTodo收纳箱（不存在则创建）
      └ 线程 fence-watch：debounce_loop（静默 300ms 回调，持续变化最多 1.5s 必回调） watch.rs:73-127
          └ cb(FenceChange{folders, rescan})
              └ on_fence_change(app, change)   fence_items.rs:405-418  load_fences → affected_fence_ids（rescan 全部 / 文件夹名不区分大小写匹配）
                  └ reconcile_and_notify(app, db, id)   L382-392
                      ├ reconcile_fence(db, id)   L340-379  一个事务内：删除中 → None；读 folder_name；list_fence_items（失败则 ensure_fence_folder 后重试）；reconcile_rows
                      └ app.emit_to("fence-{id}", "fence-items-changed", Vec<FenceEntry>)
FenceWatcher 句柄存在托管状态 FenceWatcherState(Mutex<Option<FenceWatcher>>)，drop 即停止
```

GPUI：回调运行在 `fence-watch` 线程，`reconcile_fence` 可在该线程直接执行（纯 core），结果经 channel 投递到前台更新对应收纳箱 Entity；`FenceWatcher` 放在 app 全局状态中持有。

### 7.5 纯文件层 `services/fence_fs`（与 Tauri、数据库都无关，全部 A）

| 模块 | 对外函数（mod.rs 重新导出） | 线程 / 平台 |
|---|---|---|
| `location.rs` | `fence_root_dir`、`icon_cache_dir`、`user_desktop_dir`、`desktop_is_onedrive_redirected`、`path_starts_with_ci`、`same_path_ci`（另有 `fence_root_path`、`public_desktop_dir`、`is_under_onedrive`） | 全平台；`MINI_TODO_FENCES_ROOT` 仅 debug |
| `naming.rs` | `is_valid_item_name`（另有 `sanitize_folder_name`、`dedupe_name`、`unique_file_name`、`restore_file_name`、`display_name` 等） | 纯 |
| `folder.rs` | `create_fence_folder`、`ensure_fence_folder`、`rename_fence_folder`、`remove_fence_folder_if_empty`、`fence_folder_path`、`list_fence_items`、`list_fence_folders`、`FenceEntry` | std::fs |
| `ops.rs` | `move_into_fence`、`restore_item`、`move_between_fences`、`restore_all`、`ItemOpResult`、`ItemOpStatus`、`RestoreAllResult`（另有 `plan_move_in`、`plan_restore`、`classify` 等纯函数） | Windows 经 `shell::perform_moves`（新开 STA 线程 `fence-move`，`IFileOperation`，`FOF_RENAMEONCOLLISION\|FOF_NOCONFIRMMKDIR\|FOFX_NOMINIMIZEBOX`）；非 Windows `Err("桌面收纳箱仅支持 Windows")`；**阻塞，且带 owner HWND 时绝不能在主线程调用** |
| `open.rs` | `launch_item`（`ShellExecuteExW`，`SEE_MASK_FLAG_NO_UI\|NOASYNC`，新 STA `fence-launch`）、`reveal_in_explorer`（`SHOpenFolderAndSelectItems`，失败退 `explorer /select,`） | 阻塞 |
| `icon.rs` | `icon_png`（档位 `[16,24,32,48,64,96,128,256]` 向上取整，常驻 STA `fence-icon` 上 `IShellItemImageFactory(SIIGBF_ICONONLY)`，缓存 `<app_data_dir>\fence-icons\{FNV1a64}-{mtime}-{len}-{档位}.png`）、`prune_icon_cache` | 阻塞 |
| `watch.rs` | `watch_fence_root`、`FenceChange`、`FenceWatcher` | 内部线程 `fence-watch` |
| `shell.rs` / `sta.rs` | 内部（cfg(windows)） | COM STA |

### 7.6 收纳箱命令中可进入 core 的部分与需要 app 注入的东西

| 命令 / 内部函数 | core 部分 | app 注入 |
|---|---|---|
| `get_fence_items` | `reconcile_fence` | 后台执行 |
| `fence_move_in` | 登记、跨箱识别、`move_into_fence`、upsert / 删行、OneDrive 提示 | `owner_hwnd: Option<isize>`（收纳箱窗口 HWND，在前台取）；"对来源收纳箱刷新"的通知回调 |
| `fence_restore_item` / `fence_move_item` | 同上 | owner HWND、目标收纳箱刷新回调 |
| `fence_open_item` / `fence_reveal_item` / `fence_item_icon` | `item_path` 校验 + fence_fs | 后台执行 |
| `rename_fence` | `rename_fence_and_folder` | 改窗口标题 |
| `delete_fence` | `begin_deleting` → `fence_originals` → `restore_all` → `plan_delete` → 删行或删已移回项行 | owner HWND；`deleted` 时关窗；否则刷新通知 |
| `create_fence` / `reset_fence_positions` / 屏幕切换 | 标题、默认位置、`reset_layout`、写库、`Placement` | 显示器 / 光标 / 建窗 / 摆窗 |

---

## 8. sync_cmd.rs / data.rs / todo.rs / settings_cmd.rs / holiday.rs：纯逻辑与依赖

| 文件 | 纯逻辑（变成普通 Rust API） | 依赖 AppHandle / Tauri 的部分 | 改造方式 |
|---|---|---|---|
| `sync_cmd.rs` | `SyncSettings` + `read_sync_settings` / `save_sync_settings_inner` / `ensure_device_id` / `stored_password`（L95-231）；`SyncData` 与宽容反序列化 / gzip / `decode_sync_doc`（L261-387）；`SyncStatus` / `SyncReport`（L393-464）；`SyncGuard`、`SMART_SYNC_ROUNDS`、`is_full_get_round`、`wants_full_get`（L470-501）；`load_connection`、`run_sync_blocking`（L554-608，去掉 L603-606）；`SyncEngine::{read_pre_state, merge, snapshot, finish, put_doc, version_after_put, wait_before_retry, sync, force_pull, force_push}`（L656-1011）；`apply_remote_settings_if_newer`、JSON 合并、`build_sync_doc`、`ImageTransfer`（L1014-1457） | `webdav_test_connection`（`spawn_blocking` + `app.state`）；`run_sync_command`（`spawn_blocking` + `app.state` + `app.emit("sync-completed")`）；L605 `reload_runtime_prefs` | core 暴露 `run_sync(db, mode) -> Result<SyncReport, String>`（阻塞，内含 `SyncGuard` 互斥）与 `test_connection(db, url, user, Option<pw>)`；app 在后台执行后：`report.local_changed()` → 广播刷新；`report.settings_applied` → 刷新运行时偏好 |
| `data.rs` | `EXPORT_VERSION`、`read_app_settings`、`write_app_settings`、`REMOTE_SETTING_FIELDS` / `SettingKind`、`apply_settings_json`、`adopt_settings_json`、`export_data_internal`、`import_data_raw`（去掉 L244-245）、`import_records`、`export_to_file`、`import_from_file` | `run_blocking`（L319-330）、4 个命令外壳 | core 暴露 `export_json` / `import_json` / `export_to_file(path)` / `import_from_file(path)`（阻塞）；app 导入成功后刷新运行时偏好 + 广播"数据已导入" |
| `todo.rs` | 全部业务函数：`load_todo`、`load_subtask`、`normalize_time_field`、`TodoTimes`、`create_todo_inner`、`update_todo_inner`、`apply_order`、`update_subtask_inner`、`image_extension`、`check_image_size`、`random_suffix`、`generate_image_name`、`store_image`、导入相关（`collect_import_files`、`collect_dir`、`decode_text`、`read_import_files`、`insert_imported_subtasks`、`import_subtasks_inner`） | `image_bytes(&InvokeBody)`、`save_subtask_image(Request)`、`import_subtasks_from_paths(AppHandle)` 外壳；其余命令仅 `State<Database>` | 去 `State`；新增 `save_image(bytes, ext) -> Result<PathBuf, String>`（组合 `image_extension` + `check_image_size` + `store_image`）；`image_bytes` 的数字数组分支（postMessage 回退）在 GPUI 下无意义 |
| `settings_cmd.rs` | 全部（白名单、字体、通知类型、DirectWrite 枚举） | 仅 `State<Database>` | 去 `State` |
| `holiday.rs` | `HolidayInfo`、`HolidayCnResponse` 解析、缓存读写 L37-60、"网络优先、失败回退缓存"策略 L108-131 | 无（只是 `#[tauri::command]`），但 `fetch_from_network` 是 **async reqwest**（L64-104） | 改 `reqwest::blocking::Client`（同样 10s / 5s 超时），在后台线程调用；或 app 内单独起 tokio runtime |
| `window.rs` 中的设置 / 屏幕配置部分 | `get_*_value`、`save_settings_inner`（L739-760）、文本主题、底色、日历、`fixed_embed_desktop` / `auto_hide_enabled` / `top_on_wake` 的键读写、screen_configs 全部 SQL（L1889-2048） | 同一命令里混着窗口操作（`set_auto_hide_enabled` / `set_top_on_wake` / `set_fixed_embed_desktop`） | core 只做"写键"；窗口副作用留在 app |
| `services/notification.rs` | 见 §6.1 | 见 §6.2 / §6.3 | core 暴露"查询到期 / 补发 → 投递回调 → 推进"的函数；调度循环在 app（线程或 GPUI 定时器） |

`run_blocking` / `blocking` / `tauri::async_runtime::spawn_blocking` 的统一替代：core 函数本身是阻塞的，app 用 `cx.background_executor().spawn(async move { core::... })`（GPUI 后台线程池，非 tokio——`reqwest::blocking` 在其中可用，因为它只要求"不在 tokio runtime 上下文中"）。

---

## 9. 建议的新 crate 结构

### 9.1 目录

```text
pc-gpui/
├── Cargo.toml                         # [workspace] members = ["crates/core", "crates/app"]
├── crates/
│   ├── core/                          # package mini-todo-core，lib；不依赖 gpui / tauri / tokio
│   │   ├── Cargo.toml                 # rusqlite(bundled) serde serde_json chrono dirs base64 reqwest(blocking,json) flate2 zip
│   │   │                              # encoding_rs notify png log；[target.windows] windows 0.58 + windows-core 0.58
│   │   │                              # [dev] tiny_http 0.12；[features] test-util（导出 Database::new_in_memory）
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── db/                    # ← 原样：backup connection migrations models paths settings_kv sync_store time
│   │       ├── services/
│   │       │   ├── webdav.rs          # ← 原样
│   │       │   ├── secret.rs          # ← 原样
│   │       │   └── fence_fs/          # ← 原样（10 个文件）
│   │       ├── todo.rs                # ← commands/todo.rs 去壳
│   │       ├── data.rs                # ← commands/data.rs 去壳，删 L245
│   │       ├── sync/                  # ← commands/sync_cmd.rs 去壳，删 L605（可按 doc / engine / images / settings 拆文件）
│   │       ├── settings.rs            # ← settings_cmd.rs + window.rs 的设置读写 / screen_configs
│   │       ├── holiday.rs             # ← 改 blocking reqwest
│   │       ├── reminder.rs            # ← services/notification.rs 纯逻辑（含通知窗口定位算法与槽位）
│   │       ├── window_logic.rs        # ← window.rs 纯函数：贴边状态机、needs_reattach、lacks_tool_window_style、
│   │       │                          #    is_desktop_host_class、apply_runtime_prefs、托盘唤起代次、accept_tray_click
│   │       └── fence/
│   │           ├── mod.rs             # ← fence.rs 纯几何 / 命名 / DB / plan_delete / rename_fence_and_folder / Placement::for_screen
│   │           └── items.rs           # ← fence_items.rs 登记 / 对账 / 行读写 / 命令主体（owner HWND 与通知回调作为参数）
│   └── app/                           # package mini-todo（bin），依赖 mini-todo-core + gpui-kit 0.7.3 + 平台 crate
│       ├── Cargo.toml / build.rs      # exe 图标与版本资源（embed-resource / winresource）
│       └── src/
│           ├── main.rs                # 单实例 → 日志 → Database → Application(QuitMode::Explicit).run
│           ├── state.rs               # 全局 Entity：数据版本、设置、同步状态、运行时偏好缓存（TOP_ON_WAKE 等）
│           ├── platform/              # 仅 cfg(windows) 的 Win32：hwnd.rs（取 / 缓存 HWND）、style.rs（ex style、MINIMIZEBOX）、
│           │                          #   desktop.rs（desktop_host / attach / detach / WM_WINDOWPOSCHANGING 子类）、monitors.rs
│           │                          #   （EnumDisplayMonitors + DPI → 屏幕组合 ID、工作区）、geometry.rs（SetWindowPos 物理像素）、
│           │                          #   corners.rs（DWM 圆角）
│           ├── tray.rs                # tray-icon + muda；事件经 channel 回前台
│           ├── single_instance.rs
│           ├── autostart.rs           # auto-launch
│           ├── notify.rs              # notify-rust（系统通知）+ 应用内通知窗口
│           ├── logging.rs
│           ├── scheduler.rs           # 提醒调度（线程或前台定时器）、200ms tick、自动同步定时器、5s 变更检查
│           └── ui/                    # main / editor / subtask / settings / completed / notification / fence 视图
```

### 9.2 core 公开 API 草案（签名均为阻塞函数；错误类型沿用 `String`，与现有代码及测试一致）

```rust
// ---- db（原样）----
pub use db::{Database, models::*, time, settings_kv, sync_store, paths, backup, migrations};
impl Database {
    pub fn new() -> Result<Database, String>;                       // paths::db_path()，迁移前备份
    pub fn open(path: &Path) -> Result<Database, String>;
    pub fn with_connection<F, R>(&self, f: F) -> rusqlite::Result<R> where F: FnOnce(&Connection) -> rusqlite::Result<R>;
    pub fn with_transaction<F, R, E>(&self, f: F) -> Result<R, E> where F: FnOnce(&mut Transaction<'_>) -> Result<R, E>, E: From<rusqlite::Error>;
    #[cfg(any(test, feature = "test-util"))] pub fn new_in_memory() -> rusqlite::Result<Database>;
}

// ---- todo ----
pub mod todo {
    pub fn list(db: &Database) -> Result<Vec<Todo>, String>;                                  // get_todos
    pub fn get(db: &Database, id: i64) -> Result<Todo, String>;                               // get_todo
    pub fn change_seq(db: &Database) -> Result<i64, String>;                                  // get_change_seq
    pub fn create(db: &Database, req: &CreateTodoRequest) -> Result<Todo, String>;
    pub fn update(db: &Database, id: i64, req: &UpdateTodoRequest) -> Result<Todo, String>;
    pub fn delete(db: &Database, id: i64) -> Result<(), String>;
    pub fn reorder(db: &Database, ids: &[i64]) -> Result<(), String>;
    pub fn reorder_subtasks(db: &Database, ids: &[i64]) -> Result<(), String>;
    pub fn create_subtask(db: &Database, req: &CreateSubTaskRequest) -> Result<SubTask, String>;
    pub fn update_subtask(db: &Database, id: i64, req: &UpdateSubTaskRequest) -> Result<SubTask, String>;
    pub fn get_subtask(db: &Database, id: i64) -> Result<SubTask, String>;
    pub fn delete_subtask(db: &Database, id: i64) -> Result<(), String>;
    pub fn images_dir() -> Result<PathBuf, String>;                                          // 创建后返回
    pub fn save_image(bytes: &[u8], ext: &str) -> Result<PathBuf, String>;                   // 白名单 + ≤20MB + store_image
    pub fn import_subtasks_from_paths(db: &Database, parent_id: i64, paths: &[String]) -> Result<Vec<SubTask>, String>;
}

// ---- data（导入导出）----
pub mod data {
    pub const EXPORT_VERSION: &str = "4.0";
    pub fn export_json(db: &Database) -> Result<String, String>;
    pub fn import_json(db: &Database, json: &str) -> Result<(), String>;                      // 调用方随后刷新运行时偏好
    pub fn export_to_file(db: &Database, path: &Path) -> Result<(), String>;                 // ZIP(data.json)
    pub fn import_from_file(db: &Database, path: &Path) -> Result<(), String>;               // ZIP / JSON
    pub fn read_app_settings(conn: &Connection) -> AppSettings;
}

// ---- settings / screen configs ----
pub mod settings {
    pub struct RuntimePrefs { pub top_on_wake: bool, pub auto_hide_enabled: bool }
    pub fn runtime_prefs(db: &Database) -> RuntimePrefs;
    pub fn save_window_settings(db: &Database, s: &AppSettings) -> Result<(), String>;      // save_settings_inner
    pub fn text_theme(db: &Database) -> Result<String, String>;  pub fn set_text_theme(db: &Database, theme: &str) -> Result<(), String>;
    pub fn auto_hide_enabled(db: &Database) -> bool;             pub fn set_auto_hide_enabled(db: &Database, v: bool) -> Result<(), String>;
    pub fn top_on_wake(db: &Database) -> bool;                   pub fn set_top_on_wake(db: &Database, v: bool) -> Result<(), String>;
    pub fn fixed_embed_desktop(db: &Database) -> bool;           pub fn set_fixed_embed_desktop(db: &Database, v: bool) -> Result<(), String>;
    pub fn window_background(db: &Database) -> Result<WindowBackground, String>;
    pub fn set_window_background(db: &Database, color: &str, alpha: f64) -> Result<(), String>;
    pub fn show_calendar(db: &Database) -> Result<bool, String>; pub fn set_show_calendar(db: &Database, v: bool) -> Result<(), String>;
    pub fn view_mode(db: &Database) -> Result<Option<String>, String>; pub fn set_view_mode(db: &Database, v: &str) -> Result<(), String>;
    pub fn font_family(db: &Database) -> Result<String, String>;  pub fn set_font_family(db: &Database, v: &str) -> Result<(), String>;
    pub fn font_size(db: &Database) -> Result<i32, String>;       pub fn set_font_size(db: &Database, v: i32) -> Result<(), String>;
    pub fn notification_type(db: &Database) -> String;            pub fn set_notification_type(db: &Database, v: &str) -> Result<(), String>;
    pub fn system_fonts() -> Result<Vec<String>, String>;         // DirectWrite（非 Windows 空）
    // screen_configs
    pub fn screen_config_id_of(monitors: &[(u32, u32, f64)]) -> String;
    pub fn screen_config_display_name(config_id: &str) -> String;                              // 由 appStore.generateScreenConfigDisplayName 移植
    pub fn get_screen_config(db: &Database, id: &str) -> Result<Option<ScreenConfig>, String>;
    pub fn save_screen_config(db: &Database, req: &SaveScreenConfigRequest) -> Result<ScreenConfig, String>;
    pub fn list_screen_configs(db: &Database) -> Result<Vec<ScreenConfig>, String>;
    pub fn delete_screen_config(db: &Database, id: &str) -> Result<(), String>;               // 连带 fence_layouts
    pub fn rename_screen_config(db: &Database, id: &str, name: &str) -> Result<(), String>;
}

// ---- sync ----
pub mod sync {
    pub use SyncSettings, SyncReport, SyncStatus;
    pub enum SyncMode { Smart { full: Option<bool> }, ForcePull, ForcePush }
    pub fn settings(db: &Database) -> Result<SyncSettings, String>;
    pub fn save_settings(db: &Database, s: &SyncSettings) -> Result<(), String>;
    pub fn test_connection(db: &Database, url: &str, user: &str, password: Option<&str>) -> Result<bool, String>;
    pub fn run(db: &Database, mode: SyncMode) -> Result<SyncReport, String>;                  // 含 SyncGuard；"同步正在进行中"
}

// ---- holiday ----
pub mod holiday { pub fn fetch(year: i32) -> Result<Vec<HolidayInfo>, String>; }               // blocking reqwest

// ---- reminder（通知调度纯逻辑）----
pub mod reminder {
    pub struct Reminder { pub id: i64, pub title: String, pub description: Option<String> }
    pub enum Kind { System, App }
    pub fn kind(db: &Database) -> Kind;
    pub fn duration_until_next_minute(now: SystemTime) -> Duration;
    /// 查询到期提醒，逐条调用 deliver；deliver 返回 Ok 才推进（重复 → 下一次，否则 notified=1）
    pub fn run_due(db: &Database, deliver: &mut dyn FnMut(&Reminder) -> Result<(), String>) -> Result<(), String>;
    pub fn catch_up_missed_repeats(db: &Database, deliver: &mut dyn FnMut(&Reminder) -> Result<(), String>) -> Result<(), String>;
    // 应用内通知窗口
    pub struct WorkArea { pub x: f64, pub y: f64, pub width: f64, pub height: f64 }
    pub struct NotificationSize { pub width: f64, pub height: f64 }
    pub const REQUESTED_SIZE: NotificationSize;
    pub fn notification_position(area: WorkArea, size: NotificationSize, slot: u32) -> (f64, f64);
    pub struct SlotPool; impl SlotPool { pub fn acquire(&self) -> u32; pub fn release(&self, slot: u32); }
}

// ---- window_logic（纯，app 用来驱动 Win32）----
pub mod window_logic {
    pub struct AutoHideState { .. }  pub enum AutoHideTransition { None, Hide { .. }, Restore { anchor } }
    pub struct MonitorBounds { .. }  pub struct WindowRect { .. }  pub enum DockEdge { .. }
    pub fn evaluate_auto_hide_transition(s: &mut AutoHideState, rect: WindowRect, monitor: MonitorBounds, cursor: Option<(i32, i32)>, now: Instant) -> AutoHideTransition;
    pub fn apply_runtime_prefs(s: &mut AutoHideState, top_on_wake_cell: &AtomicBool, top_on_wake: bool, auto_hide_active: bool) -> bool;
    pub fn take_disabled_hidden_anchor(s: &mut AutoHideState) -> Option<WindowPosition>;
    pub fn needs_reattach(cached_owner: isize, owner_alive: bool, shell_now: isize, current_owner: isize) -> bool;
    pub fn lacks_tool_window_style(ex_style: u32) -> bool;
    pub fn is_desktop_host_class(name: &str) -> bool;
    pub fn accept_tray_click(last: &Mutex<Option<Instant>>, now: Instant) -> bool;
}

// ---- fence ----
pub mod fence {
    pub use services::fence_fs::{FenceEntry, ItemOpResult, ItemOpStatus};
    pub struct PxRect { pub x: i32, pub y: i32, pub width: i32, pub height: i32 }
    pub struct MonitorArea { pub work: PxRect, pub scale: f64 }
    pub fn list(db: &Database) -> Result<Vec<Fence>, String>;  pub fn get(db: &Database, id: i64) -> Result<Fence, String>;
    pub fn create(db: &Database, work: PxRect, scale: f64, screen_id: &str) -> Result<Fence, String>;   // 标题 / 位置 / 文件夹 / 插库 / 布局；建窗失败时调用 rollback_create
    pub fn rollback_create(db: &Database, fence: &Fence);
    pub fn rename(db: &Database, id: i64, title: &str) -> Result<RenameFenceResult, String>;
    pub fn update(db: &Database, id: i64, patch: &UpdateFenceRequest) -> Result<Fence, String>;
    pub fn view_rect(stored: PxRect, expanded: bool, areas: &[MonitorArea]) -> (PxRect, f64);          // fence_view
    pub fn effective_rect(db: &Database, fence: &Fence, screen_id: &str) -> PxRect;                     // Placement + SESSION_RECTS
    pub fn commit_geometry(db: &Database, id: i64, screen_id: &str, window: PxRect, expanded_view: bool) -> Result<Fence, String>;
    pub fn reset_positions(db: &Database, work: PxRect, scale: f64, screen_id: &str) -> Result<Vec<(Fence, PxRect)>, String>;
    pub fn screen_switch(db: &Database, previous: &str, current: &str, windows: &[(i64, PxRect, f64)]) -> Vec<(i64, PxRect)>;
    pub fn delete(db: &Database, id: i64, owner_hwnd: Option<isize>) -> Result<DeleteFenceResult, String>;
    pub mod items {
        pub fn reconcile(db: &Database, id: i64) -> Result<Option<Vec<FenceEntry>>, String>;
        pub fn move_in(db: &Database, id: i64, paths: &[String], owner_hwnd: Option<isize>) -> Result<(FenceMoveInResult, Vec<i64> /* 需刷新的来源箱 */), String>;
        pub fn restore(db: &Database, id: i64, name: &str, owner_hwnd: Option<isize>) -> Result<ItemOpResult, String>;
        pub fn move_to(db: &Database, id: i64, name: &str, target: i64, owner_hwnd: Option<isize>) -> Result<ItemOpResult, String>;
        pub fn open(db: &Database, id: i64, name: &str) -> Result<(), String>;
        pub fn reveal(db: &Database, id: i64, name: &str) -> Result<(), String>;
        pub fn icon(db: &Database, id: i64, name: &str, size_px: u32) -> Result<PathBuf, String>;
        pub fn reorder(db: &Database, id: i64, names: &[String]) -> Result<(), String>;
        pub fn watch(on_change: impl Fn(FenceChange) + Send + 'static) -> Result<FenceWatcher, String>;
        pub fn affected_ids(db: &Database, change: &FenceChange) -> Vec<i64>;
        pub fn startup_reconcile_and_prune(db: &Database);                                             // 对账 + 等 20s + prune_icon_cache
    }
}
```

（`SESSION_RECTS` / `LAST_SCREEN_ID` / `FENCE_ACTIVITY` / `NOTIFICATION_SLOTS` 等进程级静态量可原样留在 core；`FENCES_HIDDEN`、`EMBEDDED_WINDOWS`、`IS_FIXED_MODE` / `IS_DESKTOP_MODE` / `DESKTOP_OWNER` / `TOP_ON_WAKE` 等窗口状态留在 app。）

### 9.3 app 层平台 crate 选型

| 需求 | 现状 | 建议 | 备注 |
|---|---|---|---|
| UI / 窗口 | Tauri + WebView2 + Vue | `gpui-kit 0.7.3`（`gpui-pre =0.3.9` + gpui-component） | `QuitMode::Explicit`；PerMonitorV2 清单由 gpui 默认 feature `windows-manifest` 提供 |
| 托盘 | Tauri tray（内部 `tray-icon 0.21.3` + `muda 0.17.1`，Cargo.lock） | `tray-icon` + `muda`（同一库） | 必须在有 Win32 消息循环的线程（GPUI 主线程）创建；`TrayIconBuilder::with_menu / with_menu_on_left_click(false) / with_icon / with_tooltip`；`TrayIconEvent` / `MenuEvent` 的事件处理器在窗口过程里触发 → channel → 前台；`CheckMenuItem::set_checked` 按真实状态回写（muda 点击会自动翻转勾选） |
| 系统通知 | plugin-notification → notify-rust 4.11.7 → tauri-winrt-notification 0.7.2 | `notify-rust 4.x`（§6.4） | AUMID 注册 |
| 开机自启 | plugin-autostart 2.5.1 → `auto-launch 0.5.0` | `auto-launch` | 值名 / 参数与旧版一致（§4.7）；保留"开发隔离不自愈" |
| 单实例 | plugin-single-instance 2.3.7 | 自实现：`CreateMutexW` + `WM_COPYDATA`（message-only 窗口）或命名管道 | §4.6；`--autostart` 静默 |
| 文件对话框 | plugin-dialog 2.6.0 → `rfd 0.16.0` | `rfd`（`FileDialog`/`AsyncFileDialog`：`set_title`、`add_filter`、`set_file_name`、`set_parent(&window)`——GPUI `Window` 实现 `HasWindowHandle`） | GPUI `prompt_for_paths` 无过滤器 / 标题，只适合不需要过滤的场景 |
| 打开链接 / 文件 | plugin-opener 2.5.3 → `open 5.3.3` | `cx.open_url` / `cx.reveal_path` / `cx.open_with_system`（GPUI 自带），或 `open` crate | 收纳箱打开 / 定位继续用 `fence_fs::launch_item` / `reveal_in_explorer` |
| 日志 | plugin-log 2.8.0（`fern 0.7`）：文件 + stdout、Info、2MB、保留 1 份、本地时间 | `flexi_logger`（按大小轮转 + 保留文件数 + 本地时间 + 同时 stderr/stdout）或 `log4rs`（size trigger + fixed window roller） | 路径沿用 `%LOCALAPPDATA%\com.tauri-app.mini-todo\logs\mini-todo.log`；开发隔离 `<dir>/logs` |
| DPAPI | `services/secret.rs` | 原样（core） | — |
| 系统字体 | DirectWrite（settings_cmd） | 原样，或 `cx.text_system().all_font_names()` | — |
| 异步 / 后台 | tokio（Tauri 运行时）+ `spawn_blocking` | GPUI `background_executor().spawn` / `std::thread` | core 不依赖 tokio |
| exe 资源 | `tauri_build::build()` | `embed-resource` / `winresource`（图标、版本信息） | 清单已由 gpui 提供，避免重复嵌入 |

---

## 10. 现有测试分布与迁移

### 10.1 分布（`#[test]` 计数，共 271）

| 文件 | 数 | 依赖 | 迁移后位置 | 处理 |
|---|---|---|---|---|
| `db/backup.rs` | 2 | 临时目录 | core | 原样 |
| `db/connection.rs` | 3 | 临时目录 | core | 原样 |
| `db/migrations.rs` | 14 | 内存库 | core | 原样 |
| `db/paths.rs` | 4 | 纯 | core | 原样 |
| `db/settings_kv.rs` | 4 | 内存库 | core | 原样 |
| `db/sync_store.rs` | 17 | 内存库 | core | 原样 |
| `db/time.rs` | 10 | 纯 | core | 原样 |
| `services/notification.rs` | 18 | 纯 + 内存库 | core `reminder` | 原样（`NotificationService::calc_next_occurrence` 等改为模块函数时同步改调用名） |
| `services/secret.rs` | 4 | 纯（Windows 走 DPAPI） | core | 原样 |
| `services/webdav.rs` | 14 | 纯 | core | 原样 |
| `services/fence_fs/*` | 62（folder 5、icon 10、location 6、naming 14、open 1、ops 15、shell 4、sta 3、watch 4） | 临时目录、COM（Windows） | core | 原样；`icon.rs` L492 与 `watch.rs` L195 的测试为 `#[cfg(windows)]`，`location.rs` L181 为 `#[cfg(not(debug_assertions))]`（只在 `cargo test --release` 编译），`shell.rs` / `sta.rs` 整个模块 `cfg(windows)` |
| `commands/data.rs` | 11 | 内存库 | core `data` | 原样 |
| `commands/fence.rs` | 28 | 纯 + 内存库（`rename_fence_and_folder` 注入假改名函数） | core `fence` | 原样（窗口相关函数没有测试） |
| `commands/fence_items.rs` | 13 | 内存库、`FENCE_ACTIVITY` 静态量（用 987_654 等大 id 避免并行冲突） | core `fence::items` | 原样；L1019 改 `read_app_settings` 路径 |
| `commands/settings_cmd.rs` | 1 | 内存库 | core | 原样 |
| `commands/sync_cmd.rs` | 30 | 内存库 + **进程内 tiny_http mock WebDAV**（L1471-1942，Apache / nginx 语义） | core `sync` | 原样；`tiny_http` 进 core dev-dependencies；测试用 `SyncEngine { retry_delay }` 缩短等待 |
| `commands/todo.rs` | 15 | 内存库、临时目录 | core `todo` | `image_bytes_accept_raw_bodies_and_postmessage_byte_arrays`（L1074-1122）构造 `tauri::ipc::InvokeBody`，**需改写**为对新 `save_image(bytes, ext)` 的测试或删除；`symlinked_folders_are_not_followed_during_recursion`（L1244）为 `#[cfg(unix)]` |
| `commands/window.rs` | 20 | 纯 + 内存库 | core `window_logic` / `settings` | 原样；`save_settings_only_touches_changed_keys` L2390/2403 改 `data` 路径；`only_the_latest_tray_wake_may_drop_the_topmost` 依赖进程级 `TRAY_WAKE_GENERATION` |
| `lib.rs` | 1 | 纯 | core `window_logic`（或 app） | `tray_clicks_are_debounced_with_a_monotonic_clock` 原样 |

### 10.2 保持通过的要点

1. **模块路径**：core 保持 `db::` / `services::` 顶层模块名不变，`crate::db::...`、`crate::services::...` 的内部引用无需改；`commands::*` 的内容搬到新模块后，只需改少数跨模块引用（§1.2 表中测试两处 + `sync` 引用 `data`）。
2. **`Database::new_in_memory`** 目前是 `#[cfg(test)]`（connection.rs:71-79），只在 core 自己的测试里可见；若 app crate 也要写集成测试，在 core 加 `features = ["test-util"]` 并改成 `#[cfg(any(test, feature = "test-util"))]`。
3. **进程级静态量**：`SYNC_RUNNING`、`SMART_SYNC_ROUNDS`、`FENCE_ACTIVITY`、`NOTIFICATION_SLOTS`、`TRAY_WAKE_GENERATION` 等被多个测试共享，现有测试已按"并行安全"写法（独立 id、只断言相对关系），搬迁时不要改成每测试重置。
4. **tiny_http mock**：`sync_cmd.rs` 测试在进程内起 HTTP 服务器，依赖 `reqwest::blocking`；core 不引入 tokio，测试照常（blocking client 自带 runtime）。
5. **CI**：现在 `.github/workflows/pc-ci.yml` 在 Windows（fmt / clippy / `cargo test --locked --no-fail-fast`）与 Linux（clippy / test）上跑 `pc/src-tauri`。迁移后 core 的测试可在两平台单独跑（`cargo test -p mini-todo-core`），不需要 GPUI 的系统依赖；app crate 在 Linux 上编译 GPUI 需要额外系统库（X11/Wayland/Vulkan 相关），可只做 `cargo check`。
6. **Windows 专属测试**：`fence_fs` 的 COM / Shell 测试、`secret.rs` 的 DPAPI 往返、`icon.rs::extracts_real_icons_into_cache`、`watch.rs` 的 Windows 测试只在 Windows runner 上执行，保持现状。
7. **e2e**：`pc/scripts/e2e-linux/`（selenium + tauri-driver + 真实 WebDAV + cloud）与 `pc/scripts/e2e/`（PowerShell + CDP 操作 WebView）都绑定 WebView / CDP，GPUI 版无法直接复用；其中的 Win32 状态断言（`e2e.ps1 -Step json`、`fence.ps1 -Step json`：owner、ex style、Z 序、Win+D 探针）与 WebView 无关，可沿用。

---

## Related Specs

- `.trellis/spec/backend/window-modes.md` —— 主窗口三种模式的 Win32 状态表、Ordering rules 1-7、`lacks_tool_window_style` 自愈的来龙去脉（§5 主要依据）。
- `.trellis/spec/backend/desktop-fences.md` —— 收纳箱建窗参数表、允许 / 禁止的窗口操作、几何落库时机、`FenceActivity`、文件层规则、屏幕组合布局（§7 主要依据）。
- `.trellis/spec/backend/sync-protocol.md` —— 同步文档、K1 时间、合并规则、WebDAV 请求规则、设备本地数据（§8 的 sync 部分；core 必须原样保持）。
- `CLAUDE.md` —— 数据目录、日志路径、维护检查清单、debug 环境变量。

## External References（本机 cargo registry 源码，未联网）

- `gpui-pre-0.3.9`：`src/app.rs`（QuitMode、set_app_identity、show_system_notification、prompt_for_*）、`src/window.rs`（Window API、HasWindowHandle）、`src/platform.rs`（WindowOptions、WindowKind、PlatformDisplay、PathPromptOptions、SystemNotification）、`resources/windows/gpui.manifest.xml`、`build.rs`。
- `gpui-pre-windows-0.3.9`：`src/window.rs`（建窗样式、activate、resize、拖放）、`src/events.rs`（消息分发、命中测试、WM_CLOSE）、`src/display.rs`、`src/system_notifications.rs`、`src/platform.rs`。
- `gpui-kit-0.7.3`：`Cargo.toml`（依赖 `gpui-pre =0.3.9`、gpui-base、gpui-component）、`src/lib.rs`。
- `notify-rust-4.18.2`、`tauri-winrt-notification-0.8.1`、`open-5.4.4`（registry 中存在）；`pc/src-tauri/Cargo.lock` 中的 `tauri-plugin-*`、`tray-icon 0.21.3`、`muda 0.17.1`、`auto-launch 0.5.0`、`rfd 0.16.0`、`fern 0.7.1` 版本信息。

## Caveats / Not Found

- **tao 源码不在本机 registry**：`ALWAYS_ON_BOTTOM` 钩子"只改 `hwndInsertAfter`"的描述来自 window.rs:1291 注释与 spec window-modes.md 的 Design Decision；实现自定义钩子前建议对照 tao 0.34.5 `platform_impl/windows/event_loop.rs` 的 `WM_WINDOWPOSCHANGING` 分支。
- **tauri-plugin-single-instance / plugin-autostart / plugin-notification 源码不在本机**：§4.6 的互斥量 / 窗口命名、§4.7 的注册表值名、§6.2 的 AUMID 设置条件均为记忆，标注了"需核实"。
- GPUI 的 `windows-manifest` 默认 feature 是否经 gpui-kit 传递到最终 exe（lib crate 的 `embed-resource` 链接行为）未在 Windows 上实测，需在首个可运行版本中用 `mt.exe` / 资源查看器确认 PerMonitorV2 生效。
- 屏幕组合 ID 的枚举顺序与缩放取值（tao `available_monitors` vs 自写 `EnumDisplayMonitors` + `GetDpiForMonitor`）需要在多显示器 Windows 上实测，确保与库里已有 `screen_configs.config_id` / `fence_layouts.screen_config_id` 匹配。
- 未逐行阅读 `db/sync_store.rs`、`db/migrations.rs`、`services/webdav.rs`、`services/fence_fs/{ops,shell,icon,folder}.rs` 的函数体（已确认无 Tauri 依赖并列出签名 / 测试），它们按"原样复制"处理，不影响本移植地图。
