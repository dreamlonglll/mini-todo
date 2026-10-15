# Core Parity（`pc/` 与 `pc-gpui/` 并存期间的双改规则）

> `pc-gpui/crates/core`（`mini-todo-core`）是从 `pc/src-tauri/src` **复制**（不是移动）出来的业务核心；`pc/` 在用户于 Windows 上
> 实机验收 GPUI 版之前保留，两版共用同一数据目录、同一数据库、同一 WebDAV 同步文档，用户可以随时切换、多台设备也可能混用。
> 因此在 `pc/` 删除之前，**凡是影响数据的改动都必须两边一起改**。`pc/` 删除后本文件归档，`backend/*` 的契约改为只指向 core。

---

## 1. Scope / Trigger

- 改 `pc-gpui/crates/core/**` 或 `pc/src-tauri/src/{db,services,commands}/**` 的任何业务逻辑。
- 新增数据库迁移、settings 键、`todos` / `subtasks` 字段、同步文档字段、导出格式字段，或改合并 / 时间 / 墓碑规则。
- 改旧前端 `pc/src/utils/*.ts` 中已被 1:1 移植到 app 的纯逻辑（见 §2.3）。

---

## 2. 文件对应

### 2.1 原样复制（必须保持一致）

| `pc/src-tauri/src/` | `pc-gpui/crates/core/src/` | 已知差异（`diff` 只应出现这些） |
|---|---|---|
| `db/**`（connection、migrations、models、settings_kv、sync_store、time、paths、backup） | `db/**` | `connection.rs`：`Database::new_in_memory` 的 cfg 为 `any(test, feature = "test-util")` |
| `services/webdav.rs` | `services/webdav.rs` | 无 |
| `services/secret.rs` | `services/secret.rs` | 无 |
| `services/fence_fs/**` | `services/fence_fs/**` | `ops.rs`：1 个测试的路径字面量改为按平台拼接 |

### 2.2 改写（去掉 Tauri 外壳，逻辑须保持等价）

| `pc/src-tauri/src/` | `pc-gpui/crates/core/src/` |
|---|---|
| `commands/todo.rs` | `todo.rs`（`save_subtask_image` → `save_image(bytes, ext)`） |
| `commands/data.rs`（`export_data_internal` / `import_data_raw` / `read_app_settings` / `write_app_settings`） | `data.rs`（`export_json` / `import_json` / `export_to_file` / `import_from_file` / `read_app_settings` / `write_app_settings`） |
| `commands/sync_cmd.rs`（`SyncData` / `run_sync_blocking` / `build_sync_doc`） | `sync/mod.rs`（同名；测试在 `sync/tests.rs`） |
| `commands/settings_cmd.rs` + `window.rs` 的设置 / `screen_configs` + `fence.rs` 的屏幕组合 ID | `settings.rs` |
| `commands/holiday.rs` | `holiday.rs`（async reqwest → blocking） |
| `services/notification.rs` 的纯逻辑 | `reminder.rs` |
| `commands/window.rs` + `lib.rs` 的纯函数 | `window_logic.rs` |
| `commands/fence.rs` / `commands/fence_items.rs` 中与窗口无关的部分 | `fence/mod.rs` / `fence/items.rs` |
| 窗口 / 平台部分（`window.rs` 的 Win32、托盘、通知窗口、建窗） | 不在 core：`pc-gpui/crates/app/src/{services,platform,windows}/`（见 `crates/core/README.md`「不在 core 的旧命令」） |

逐函数对应（"旧命令"列）见 `pc-gpui/crates/core/README.md`「公开 API」。

### 2.3 旧前端纯逻辑 → app

| `pc/src/utils/` | `pc-gpui/` |
|---|---|
| `datetime.ts` | `crates/app/src/util/datetime.rs`（存储格式仍以 core `db::time` 为准） |
| `imageRef.ts` | `crates/app/src/util/image_ref.rs` |
| `fileLink.ts` | `crates/app/src/util/file_link.rs` |
| `editorShortcuts.ts` | `crates/app/src/util/editor_shortcuts.rs` |
| `syncReport.ts` | `crates/app/src/services/sync.rs`（提示文案） |
| `notify.ts` | `crates/app/src/ui/message.rs`（`error_with`） |
| `quadrant.ts` / `color.ts` | `crates/app/src/windows/main_window/logic.rs`、`theme.rs` |
| `fence.ts` | `crates/app/src/windows/fence/logic.rs` |
| `lunar.ts` | `crates/core/src/lunar.rs`（`tyme4rs`；对照数据 `tests/fixtures/lunar_fixture.json`） |

原 vitest 用例已移植为对应 Rust 模块的单测。

---

## 3. 规则

| 改动 | 规则 | 不遵守的后果 |
|---|---|---|
| 数据库迁移 | 两边加**同一版本号、同一 SQL**，`LATEST_VERSION` 相等；迁移测试两边都加 | 迁移器只跑"高于当前版本"的迁移、没有降级保护：只加在一边时，另一版打开新库会在不认识的结构上运行；两边各自加了不同的 v30 时，先跑的那版占住 v30，另一版的 v30 永远不会执行 |
| settings 键 | 两边的 `AppSettings`（`#[serde(default)]`）、`read_app_settings` / `write_app_settings`、需要同步时 `SYNCED_SETTING_KEYS` 一起改；设备本地键按 `backend/sync-protocol.md` §3.6 登记 | 只认识旧键的一版上传设置块时会丢掉新键，其它设备随后按 LWW 把它恢复成默认值 |
| `todos` / `subtasks` 字段 | 两边的 `models.rs`、`sync_store` 行读写、导入导出一起改；cloud `src/model.rs` 允许字段同步加 | 旧一版合并 / 上传时丢字段；cloud 拒绝写入（400） |
| 同步 / 合并 / 时间 / 墓碑规则 | 两边 + cloud 三处一致（`backend/sync-protocol.md` 的每条规则对 core 同样适用） | 两个写入方对同一文档得出不同结果，数据丢失或"复活" |
| 导出格式 | 两边 `EXPORT_VERSION` 与字段一致；导入兼容旧版本 | 一版导出的备份另一版导不进 |
| 图片 / 文件名 / 路径规则（`is_safe_image_name`、数据目录、收纳根目录） | 两边一致 | 同步过来的图片在另一版裂图或被拒；切换版本后找不到数据 |
| 已移植的前端纯逻辑（§2.3） | 修 `pc/src/utils` 的逻辑 bug 时同步修 app 对应模块（反之亦然） | 两版对同一数据显示 / 解析不一致 |
| 纯界面 / 交互 | 不强制双改；以 pc-gpui 为准，`pc/` 只做必要修复。两版行为不一致时在 PR / 交付说明里写明 | — |

额外约定：

- 一个改动的两边放在**同一个提交 / PR** 里，提交说明写明"pc 与 pc-gpui core 同步修改"。
- 两边的 `cargo test` 都要绿（`pc/src-tauri` 与 `pc-gpui/`），同步相关的再跑两套 Linux e2e（`pc/scripts/e2e-linux/`、`pc-gpui/scripts/e2e-linux/`）。
- 新增设备本地数据时，同时更新 `backend/sync-protocol.md` §3.6 与 `CLAUDE.md`「当前同步覆盖范围」。
- 不要把 core 的改进"顺手"只做在 core（例如重构 `sync_store`）：§2.1 的目录必须能用 `diff` 证明一致。

---

## 4. 自查命令（在仓库根目录）

```bash
# 原样复制的目录：只应出现 §2.1 表里的已知差异
diff -r pc/src-tauri/src/db pc-gpui/crates/core/src/db
diff pc/src-tauri/src/services/webdav.rs pc-gpui/crates/core/src/services/webdav.rs
diff pc/src-tauri/src/services/secret.rs pc-gpui/crates/core/src/services/secret.rs
diff -r pc/src-tauri/src/services/fence_fs pc-gpui/crates/core/src/services/fence_fs
# 迁移版本一致
grep -n "pub const LATEST_VERSION" pc/src-tauri/src/db/migrations.rs pc-gpui/crates/core/src/db/migrations.rs
# 两边测试
(cd pc/src-tauri && cargo test --locked)
(cd pc-gpui && cargo test --workspace --locked)
```

---

## 5. Wrong vs Correct

### Wrong — 只在 core 加迁移

```rust
// pc-gpui/crates/core/src/db/migrations.rs
(30, migration_v30),            // 新增 settings 键 foo
pub const LATEST_VERSION: i32 = 30;
// pc/src-tauri/src/db/migrations.rs 仍停在 29：用户切回旧版后，旧版上传的设置块里没有 foo，
// 其它设备上的新版按 settingsUpdatedAt 把 foo 恢复成默认值
```

### Correct — 两边同号同 SQL，连同设置读写与同步键

```text
pc/src-tauri/src/db/migrations.rs      (30, migration_v30) + LATEST_VERSION = 30
pc-gpui/crates/core/src/db/migrations.rs  完全相同的 migration_v30
两边 db/models.rs AppSettings 加 #[serde(default)] foo；两边 data.rs read/write_app_settings；需要同步时两边 SYNCED_SETTING_KEYS
CLAUDE.md「迁移版本」与「维护检查清单」更新
```
