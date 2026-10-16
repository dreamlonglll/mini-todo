# 数据契约与跨版本兼容（core ↔ cloud ↔ 已安装的旧版本）

> 2026-10-16（任务 `10-16-gpui-release-pipeline`）起，旧 Tauri 版（`pc/`）已删除，`pc-gpui/crates/core`（`mini-todo-core`）是 PC 端
> 数据逻辑的唯一实现。原来的"`pc/` 与 `pc-gpui/` 双改规则"（本文件的前身 `core-parity.md`）随之失效；仍然有效的是**数据契约**：
> 同一份数据还有别的读写方，改数据结构时必须把它们一起考虑进去。

---

## 1. Scope / Trigger

- 新增 / 修改数据库迁移、settings 键、`todos` / `subtasks` 字段、同步文档字段、导出格式字段。
- 改合并 / 时间 / 墓碑 / WebDAV 请求规则（`crates/core/src/sync/`、`db/sync_store.rs`、`db/time.rs`、`services/webdav.rs`）。
- 改图片文件名规则（`db::paths::is_safe_image_name`）、数据目录或收纳根目录。

---

## 2. 同一份数据的读写方

| 读写方 | 接触的数据 | 说明 |
|---|---|---|
| PC：`pc-gpui/crates/core` | 本地 SQLite、`sync-data.json.gz`、导出文件 | 唯一维护中的 PC 实现 |
| cloud：`cloud/` | `sync-data.json.gz`（经 WebDAV）、REST 写入 | 独立 crate；合并规则见 `cloud/src/sync/merge.rs`，写入字段契约见 `cloud/src/model.rs` |
| 已安装的旧版本 PC | 本地 SQLite（同一数据目录）、`sync-data.json.gz` | 其它设备上可能还在跑旧 Tauri 版（≤ 2.5.0，Windows 与 macOS 都有）或更早的 3.x；同一台机器上 Windows 安装包原地升级旧版、共用 `%LOCALAPPDATA%\mini-todo`，mac 包拖进「应用程序」替换 2.5.0 mac 版、共用 `~/Library/Application Support/mini-todo`（§2.1），用户也可能装回旧版 |

旧 Tauri 版与当前 core 的 `db/`、`services/{webdav,secret,fence_fs}` 同源（core 由它复制而来），所以现有数据完全互通；
新改动之后旧版本不会跟着变，下表的"后果"列主要就是针对它们。

### 2.1 macOS：与 2.5.0 mac 版共用的身份与数据

3.0.2-pre 起重新提供 mac 包（GPUI 版），与 2.5.0 mac 版（Tauri，`mini-todo_2.5.0_universal.dmg`）是**同一个应用**：

| 项 | 事实 | 后果 / 注意 |
|---|---|---|
| 应用身份 | 同一个 bundle id `com.tauri-app.mini-todo`、同名 `mini-todo.app` / 可执行文件 `mini-todo`（`scripts/macos/bundle.sh`） | 拖进「应用程序」直接替换旧版；通知授权与显示设置挂在同一个 bundle id 上（2.5.0 用已弃用的 `NSUserNotificationCenter`，新版用 `UNUserNotificationCenter`，升级后第一次提醒是否还会请求授权未经验证）；LaunchServices 认为是同一个应用 |
| 数据目录 | 同一个 `~/Library/Application Support/mini-todo/`（core `db/paths.rs` 与 2.5.0 都是 `dirs::data_local_dir()/mini-todo`，`dirs` 5） | 升级即沿用全部数据；首次打开时迁移 v29 / v30（2.5.0 是 v28）。v29 / v30 的收纳箱表在 3.0.2-pre 的 mac 版上建了但不用，之后的版本 mac 也有收纳箱、开始读写它们（见下一行）。装回 2.5.0 时它打开 v30 库只记警告、照常运行（规则同 §3 第一行） |
| 桌面收纳箱（3.0.3-pre 起 mac 也有） | 收纳根目录 `~/MiniTodo收纳箱/<名称>/`（不在数据目录里，2.5.0 没有这个名字，不是"沿用"的名字）；`fences` / `fence_items` / `fence_layouts` 与 `fence_onedrive_notice_shown` 同 Windows，是设备本地数据、不导出不同步；几何同主窗口的物理像素口径（全局点 × 统一系数），屏幕组合 ID 同 `screen_configs` | 装回 3.0.2-pre（mac 上不恢复收纳箱、菜单栏图标没有入口）或 2.5.0（不认这些表）时收纳箱不显示，文件都还在 `~/MiniTodo收纳箱/` 里、可在访达里找回；再升级回来照常恢复。混用缩放下统一系数随显示器组合变化，没有当前组合记录时"最近一次位置"会按新系数解释（被夹回工作区，与主窗口同性质） |
| 日志 | 同一个 `~/Library/Logs/com.tauri-app.mini-todo/mini-todo.log`（2.5.0 tauri-plugin-log `LogDir`） | 与 Windows / Linux 的 `<data_local_dir>/com.tauri-app.mini-todo/logs/` 口径不同（mac 没有 `logs` 子目录、不在数据目录旁边），`services/logging.rs` 按平台分支；开发隔离时三个平台都是 `<MINI_TODO_DATA_DIR>/logs` |
| 单实例 | 同一个 Unix socket `/tmp/com_tauri_app_mini_todo_si.sock`、同一消息格式（`<cwd>\0\0<args 以 \0 连接>`） | 新旧两版互相识别，不会同时打开同一个库；只删自己绑定的那个 socket 文件（按文件身份判断），不会误删另一个实例新建的 |
| 开机自启 | 同一个 LaunchAgent `~/Library/LaunchAgents/mini-todo.plist`（`Label = mini-todo`、`[<可执行文件绝对路径>, --autostart]`） | 旧 plist 在覆盖安装后照样拉起新版（路径不变）；已开启时每次启动刷新路径，从 App Translocation 临时位置运行时不刷新 |
| WebDAV 密码 | 两版都明文存在 settings（core `services/secret.rs` 非 Windows 分支与 2.5.0 逐字相同） | 互通。以后改成钥匙串时要考虑降级：2.5.0 会把钥匙串标记当成明文密码 |
| 屏幕组合 ID | 按各屏自己的缩放算 `{n}_{W}x{H}@{scale%}`（W / H = 点尺寸 × 该屏 `backingScaleFactor`），与 2.5.0（tao `availableMonitors()`）一致（`platform/mac_geometry.rs` 的 `screen_config_inputs`） | 2.5.0 记下的 `screen_configs` 记录按 ID 照常匹配 |
| 主窗口几何（物理像素） | 全局点坐标（主屏左上原点）× **统一系数**（所有屏 `backingScaleFactor` 的最大值）；2.5.0 是"点 × 窗口 / 显示器各自所在屏的缩放" | 所有屏缩放相同（单屏、多块 Retina、多块 1x）时逐像素一致，2.5.0 记下的位置照常恢复；混用缩放（Retina 笔记本 + 1x 外接屏）时口径不同，2.5.0 记下的位置可能恢复到别处或回退为居中。只影响设备本地、不同步的 `screen_configs`，最多丢一次位置；之后两版混用同一台机器时会互相覆盖对方的位置 |
| 界面状态 | 2.5.0 的"展开全部子任务"存在 WKWebView 的 `localStorage`，GPUI 版存在数据目录的 `ui-state.json` | 不迁移，升级后这个开关回到默认 |

改上表任何一项（尤其 bundle id、`.app` 名、socket 路径与格式、plist 名与参数、日志位置、物理像素口径）都会破坏与 2.5.0 的衔接，
按"兼容名字不要改"处理（`CLAUDE.md`「兼容旧版沿用的名字」的 macOS 表）。

---

## 3. 规则

| 改动 | 规则 | 不遵守的后果 |
|---|---|---|
| 数据库迁移 | 只追加新版本号（`db/migrations.rs` 的迁移表 + `LATEST_VERSION`），不改已发布的迁移；新结构对旧程序无害（加表 / 加有默认值的列，不删不改旧程序会读写的列） | 迁移器只跑"高于当前版本"的迁移、没有降级：旧程序打开新库只记一条警告、按现状继续运行（`db/connection.rs`），在它不认识的结构上读写 |
| settings 键 | `AppSettings`（`#[serde(default)]`）、`data::read_app_settings` / `write_app_settings`；需要跨设备同步的再加进 `db::settings_kv::SYNCED_SETTING_KEYS`；设备本地键按 `backend/sync-protocol.md` §3.6 登记 | 不认识新键的一方上传设置块时丢掉它，其它设备随后按 `settingsUpdatedAt` LWW 把它恢复成默认值 |
| `todos` / `subtasks` 字段 | `db/models.rs`（`#[serde(default)]`）、`sync_store` 行读写、`data.rs` 导入导出一起改；cloud `src/model.rs` 的 `TODO_FIELDS` / `SUBTASK_FIELDS` 同步加 | PC 用强类型模型反序列化，旧版本合并 / 上传时丢掉新字段；cloud 拒绝含新字段的写入（400） |
| 同步 / 合并 / 时间 / 墓碑规则 | core 与 cloud 两处一致（`backend/sync-protocol.md` 的每条规则对两边都成立） | 两个写入方对同一文档得出不同结果，数据丢失或"复活" |
| 导出格式 | `data::EXPORT_VERSION` 与字段；导入继续兼容 3.0 / 4.0 | 旧备份导不进，或旧版本导不进新备份 |
| 图片 / 文件名 / 路径规则（`is_safe_image_name`、数据目录、收纳根目录） | core 与 cloud（`cloud/src/sync/images.rs`）一致；数据目录、日志目录、单实例 / 开机自启名字不改（见 `CLAUDE.md`「兼容旧版沿用的名字」） | 同步过来的图片裂图或被拒；升级后找不到数据、新旧版本同时运行 |

额外约定：

- 新增设备本地数据时，同时更新 `backend/sync-protocol.md` §3.6 与 `CLAUDE.md`「当前同步覆盖范围」。
- 改了上表任何一项，`CLAUDE.md`「维护检查清单」逐条走一遍。

---

## 4. 自查命令

```bash
# 迁移版本：迁移表末项与 LATEST_VERSION 一致（单测也会断言）
grep -n "pub const LATEST_VERSION" pc-gpui/crates/core/src/db/migrations.rs
# cloud 允许的记录字段（与 core 的 Todo / SubTask 对照）
grep -n -A40 "^const TODO_FIELDS" cloud/src/model.rs
# 测试：core 的合并 / 迁移 / 导入导出 / WebDAV 互操作；cloud 的合并、mock_dav 场景、"云端写出的记录能被 PC 模型反序列化"（api/integration_tests.rs）
(cd pc-gpui && cargo test -p mini-todo-core --locked)
(cd cloud && cargo test --locked)
# 同步相关改动再跑 Linux e2e 的 S 组（Apache + nginx），见 verification.md §5
```

---

## 5. Wrong vs Correct

### Wrong — 只在 core 加字段

```rust
// pc-gpui/crates/core/src/db/models.rs
pub struct Todo { /* … */ pub estimate_minutes: Option<i64> }
// 忘了 cloud/src/model.rs 的 TODO_FIELDS：AI 经 cloud 写 estimateMinutes 一律 400；
// 也没给 #[serde(default)]：旧备份 / 旧设备上传的记录缺这个字段，反序列化失败被当成坏记录跳过
```

### Correct — 一个提交里走完整条链

```text
db/migrations.rs        (31, migration_v31) + LATEST_VERSION = 31（ALTER TABLE todos ADD COLUMN … DEFAULT …）
db/models.rs            #[serde(default)] estimate_minutes
db/sync_store.rs        行读写；data.rs 导入导出
cloud/src/model.rs      TODO_FIELDS 加 ("estimateMinutes", 对应类型)
CLAUDE.md               「迁移版本」「维护检查清单」更新
```
