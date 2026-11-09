# 第 1 步（PC core 数据层）交付备忘：待更新的文档、e2e 影响与对接要点

> 本步只改了 `pc-gpui/crates/core`（外加 app 测试里让工作区编译通过的几处字面量），没有改 `CLAUDE.md`、`.trellis/spec/**`、
> `crates/core/README.md` 与 e2e 脚本。下面按文件 / 章节列出需要统一更新的内容。契约以 `contract.md` 为准（含 §12 修订 1：
> `listId` 缺失 = 未知）；core 的代码注释里有 4 处引用 `.trellis/tasks/11-09-multi-list-windows/contract.md`（`ids.rs`、`lists.rs`、
> `db/migrations.rs` 的 v31、`sync/tests.rs`），合并时请确认 `contract.md` 已提交（编写时它只在主 checkout 里、未跟踪）。

## 0. 提交（分支 `worktree-agent-a752448658028fe46`）

| 提交 | 内容 |
|---|---|
| `29959ff` | 迁移 v31（`todo_lists` + 触发器、`todos.list_id`、`list_tombstones`、`window_prefs` / `window_layouts` + 种子）、`Todo.listId`、`TodoList` 模型 |
| `1338e77` | `core::lists`、`core::window_prefs`、`core::ids`；待办建到 / 移到清单；删屏幕配置连带窗口布局 |
| `9fd75d8` | WebDAV 同步：`todoLists` / `listTombstones`、清单合并、强制拉取 / 推送、`SyncReport` 清单计数 |
| `d0aecf2` | 导出格式 4.1、导入 4.1 / 4.0 的清单语义 |
| `e3b89fa` | v31 可重跑（`list_id` 列已存在时不再 ALTER） |
| `5b4a5f4` | 本备忘初版 |
| `554bda6` | **契约 §12 修订**：`listId` 缺失 = 未知（`todos.list_id` 可空、存量为 NULL、迁移作废基准 ETag、合并采纳 / 沿用、补传条件、导入旧备份写 1） |

> 用 `554bda6` 之前的 v31 建过的**开发 / 测试库**里 `todos.list_id` 还是 `NOT NULL DEFAULT 1`（v31 可重跑，但不会改已有列的约束），
> 合并同步时写 NULL 会撞约束、被当作坏记录跳过——请删掉重建。v31 没有发布过，用户库不受影响。

## 1. 需要更新的文档

### 1.1 `CLAUDE.md`

- **「项目结构」** `crates/core/src/`：
  - `db/` 一行：迁移 `v1~v30` → `v1~v31`
  - 新增三行：`lists.rs`（待办清单 CRUD、名称校验、删除时待办移到默认清单、有效清单兜底）、
    `window_prefs.rs`（清单 / 日历窗口的设备本地属性与按屏幕组合的布局）、`ids.rs`（新清单 id：毫秒 × 1000 + 随机，与 cloud 同一算法）
- **「核心功能 → 数据导入导出」**：
  - 导出版本 `4.0` → `4.1`（`EXPORT_VERSION`）；导出带顶层 `todoLists`（含默认清单），归属已知的待办带 `listId`（未知不写）；
    清单墓碑与窗口属性不导出
  - 导入兼容 v3.0 / v4.0 / v4.1；4.1 的清单按"恢复备份"语义（保留 id、`updated_at` 刷新、备份里没有的清单写清单墓碑并删除、
    默认清单始终保留只取备份里的名字），4.1 备份里缺 `listId` 的待办恢复成未知；3.0 / 4.0（没有 `todoLists`）现有清单不动，
    待办写成**具体的**默认清单 1
- **「数据库设计 → 迁移版本」**：`当前 v1~v30` → `v1~v31`，追加：
  - v31：多待办清单。`todo_lists(id, name, sort_order, created_at, updated_at)` 接 `local_seq` 触发器（参与同步；id 不用
    AUTOINCREMENT，名称不加唯一约束）；默认清单 id 1「待办清单」、`2000-01-01 00:00:00`；`todos.list_id INTEGER`（**可空**，NULL =
    归属未知，不等于默认清单；存量待办为 NULL；不加外键）；作废 `webdav_remote_etag`（升级后第一次同步无条件 GET，存量待办尽快学到归属）；
    `list_tombstones(entity_id, deleted_at)`（不计数）；设备本地的 `window_prefs`（每窗口：`window_key` = `list:<id>` / `calendar`，
    `hidden` / `dark` / `view_mode` / `expand_all` / `calendar_filter`）与 `window_layouts`（每 (窗口, 屏幕组合)：`x / y / width / height /
    is_fixed`，随 `window_prefs` 级联删除），不接 `local_seq`；种子：`list:1` 继承原主窗口（`text_theme` / `view_mode` / 各 `screen_configs`），
    `calendar` 隐藏 = `!show_calendar`，开着日历时按左 40% / 右 60% 切开；可重跑
- **「主要数据表」**：新增 `todo_lists`（待办清单，参与同步）、`list_tombstones`（清单删除墓碑，保留 30 天）、
  `window_prefs`（清单 / 日历窗口的设备本地属性）、`window_layouts`（清单 / 日历窗口按屏幕组合的布局与锁定）
- **「数据导入导出与同步 → 关键文件」**：模型加 `TodoList`、`ListTombstone`；清单 API `lists.rs`；清单合并 / 墓碑
  `db/sync_store.rs`（`merge_remote_lists`、`force_pull_replace_lists`、`force_push_prepare_lists`）；窗口属性 `window_prefs.rs`
- **「sync-data.json.gz 结构」**：`"version": "4.1"`；归属已知的 todo 带 `"listId": <清单 id>`（缺失 / `null` = **未知**，不等于 1）；新增
  `"todoLists": [ { id, name, sortOrder, createdAt, updatedAt } ]`（全部未删除清单，含 id 1）与
  `"listTombstones": [ { "entityId": …, "deletedAt": "YYYY-MM-DD HH:MM:SS" } ]`（不进 `tombstones`）
- **「合并规则」**追加两条：
  - 清单与待办同构（LWW 平局保留本地、`deletedAt >= updatedAt` 压制、并集、墓碑并集取较大、30 天）；远端没有 `todoLists` 键 = 没有
    清单信息，本地清单一条不删；默认清单不可删除，`entityId = 1` 的清单墓碑一律忽略；收到清单墓碑不改写待办（读侧兜底进默认清单）；
    被删清单的窗口记录同一事务里清理
  - 待办的 `listId`（契约 §12）：缺失 / `null` = 未知，未知永远不覆盖已知——远端胜出但没有 `listId` 时沿用本地的归属；远端有、本地未知时
    不论 LWW 谁胜（含平局、本地较新）都采纳远端的值、不改 `updatedAt`；上传 / 导出时未知不写 `listId`
- **「当前同步覆盖范围」**：新增行
  - `todo_lists` | 是 | 清单名称 / 顺序，`todoLists`；默认清单 id 1 不可删除
  - `list_tombstones` | 是 | 独立顶层键 `listTombstones`（不进 `tombstones`，旧版本经 extra 原样带回），保留 30 天
  - `window_prefs` / `window_layouts` | 否 | 清单 / 日历窗口的设备本地属性与布局（同 `screen_configs`）；也不进导入导出
  - `todos` 行说明加"含 `listId`（归属未知时不写）"
- **「维护检查清单」**第 3 条可补一句：参与同步的新表照 `todo_lists` 的做法（触发器 + `sync_store` 读写 / 合并 / 墓碑 +
  `build_sync_doc` 显式顶层键 + `doc_differs`）；第 4 条可补：旧版本不认识的新记录字段要区分"缺失 = 未知"与默认值，见 `listId`
- **「WebDAV 同步流程」**图：合并一行加"清单（todoLists / listTombstones）同一事务；待办归属未知时采纳远端的 listId"
- **「数据库设计」** 也可注明：`SyncReport` 新增 `lists_inserted / lists_updated / lists_deleted` 与 `lists_changed()`

### 1.2 `.trellis/spec/backend/sync-protocol.md`

- **§2 Signatures**：
  - `sync_meta.local_seq` 由 `todos` / `subtasks` / `todo_lists` 的触发器维护
  - PC `db/sync_store.rs` 新增：`merge_remote_lists(tx, Option<&[RemoteList]>, &[ListTombstone], cutoff)`、
    `force_pull_replace_lists(..)`、`force_push_prepare_lists(tx, Option<&[Value]>, &[ListTombstone], now)`、
    `parse_list_tombstone`、`record_list_tombstone`（id 1 不存）、`load_list_tombstones`、`purge_list_tombstones_before`
  - `SyncReport` 新增 `listsInserted / listsUpdated / listsDeleted`，`local_changed()` 含清单，`lists_changed()`；
    采纳远端 `listId`（见 §3.3 第 9 条）计入 `todosUpdated`
- **§3.1 Document**：`"version": "4.1"`；todo 的 `listId`（整数；**缺失 / null = 未知**，不等于 1；类型错误按无法识别的记录跳过）；
  `todoLists`、`listTombstones`（见 1.1）；"写入方每次上传都写出全部未删除的清单（含 id 1）与保留期内的全部清单墓碑；
  待办的归属未知时不写 `listId`"
- **§3.3 Merge** 追加：
  - 第 8 条：清单规则（同 1.1「合并规则」），并注明两端一致（PC `merge_remote_lists`、cloud `sync/merge.rs`）
  - 第 9 条（契约 §12）：远端胜出但缺 `listId` → 沿用本地的；远端有、本地未知 → 不论 LWW 谁胜都采纳、不改 `updatedAt`（PC：`UPDATE … SET
    list_id = ? WHERE … AND list_id IS NULL`，触发器让 `local_seq` 加一，但本次同步收尾时 `synced_seq` 取合并后的值，不会因此上传）
- **§3.4 WebDAV requests** 的 Self-heal 行补两点（PC）：
  - 远端缺本机的清单 / 清单墓碑时照常补传；远端**没有 `todoLists` 键**时按"只有种子状态的默认清单"
    （id 1、`2000-01-01 00:00:00`）比较——刚升级、没改过默认清单的设备不会为它多上传一次
  - 远端某条待办的 `listId` **缺失或为 null**、而本机同一条**已知**（任何值，含 1）时上传一次补回（`sync/mod.rs::remote_lost_list_ids`）
  - 新增一行「升级后第一次同步」：迁移 v31 清空 `webdav_remote_etag`，第一次智能同步是无条件 GET
- **§3.5 Force operations**：ForcePull 让本地清单 = 远端（删本地独有、不写墓碑；默认清单永远保留；清单墓碑表替换为远端；
  远端没有 `todoLists` 键时清单与清单墓碑都不动），远端待办缺 `listId` 时保留本地已知的归属；ForcePush 为远端独有的清单写墓碑（id 1 除外），
  远端较新 / 远端墓碑会压制的本地清单 `updated_at` 推到 superseding，并先让本地未知的待办采纳远端已知的 `listId`
- **§3.6 设备本地数据** 表新增：`window_prefs` 表 | v31 | 清单窗口 `list:<id>` / 日历窗口 `calendar` 的隐藏 / 深色 / 视图 / 展开全部 /
  日历筛选；`window_layouts` 表 | v31 | 每 (窗口, 屏幕组合) 的客户区物理像素几何与锁定。规则：不接 `local_seq`；清单被删除（本机 / 同步 /
  强制拉取 / 导入）时窗口记录在同一事务里删除；删屏幕配置连带删该组合的窗口布局。测试：`migrations::v31_window_tables_are_device_local_and_layouts_cascade`、
  `window_prefs::window_prefs_are_device_local`
- **§6 Tests Required**：加 `sync_store` 的清单合并测试（`remote_lists_merge_by_lww_and_ties_keep_local` 等）与 listId 规则测试
  （`missing_list_id_means_unknown_and_never_overrides_a_known_list`、`unknown_local_todos_adopt_a_known_remote_list_whoever_wins`、
  `force_pull_keeps_a_known_list_when_the_remote_does_not_know_it`、`force_push_prepare_adopts_known_remote_lists_for_unknown_local_todos`），
  以及 `sync/tests.rs` 的 `lists_and_list_deletions_sync_in_both_directions`、`old_devices_keep_list_keys_and_their_edits_keep_the_local_list`、
  `upgraded_old_device_learns_list_ids_without_uploading`、`two_devices_creating_lists_offline_keep_both`、`force_push_and_force_pull_cover_lists`

### 1.3 `.trellis/spec/pc-gpui/data-compat.md`

- **§3 规则表**加两行：
  - 「同步文档新顶层键」：显式声明成 `SyncData` 字段（`#[serde(default)]`，**不**放进 `extra`），每次上传都写出；旧版本经自己的 `extra`
    原样带回；需要删除语义的新实体用独立墓碑键，不进共用 `tombstones`（旧版本丢弃未知 `entityType`）
  - 「旧版本不认识的新记录字段」：旧版本按自己的本地版本重新序列化**全部**记录、丢掉新字段（平局保留它自己的版本）。所以这类字段的
    "缺失"要当作**未知**而不是默认值：合并时未知不覆盖已知、远端已知时采纳，存储上要能表达未知（`listId` → `todos.list_id` 可空）
- **新增一节「多清单与旧版本（≤ 3.0.6 / 2.5.0）」**（替换初版里"旧设备改过的待办落进默认清单（接受的退化）"的说法）：
  - 旧版本打开 v31 库：只记警告照常运行；`todos.list_id` 可空、没有默认值，旧版本按显式列 INSERT 的待办是"未知"，新版本照常显示在
    默认清单、同步时学到真实归属；不认识清单表
  - 旧设备上传会丢掉**所有**它重新序列化的待办的 `listId`。新设备把缺失当作未知：平局保留本地，它改过的待办（更新）内容取它的、
    **归属沿用本地清单**；cloud 与还没同步过的设备先看到未知（显示在默认清单），新设备随后补传一次把 `listId` 补回
    （`remote_lost_list_ids`：远端缺 / null、本机已知；两台新设备之间不会来回，旧设备只在自己有变更时才上传）
  - **先跑旧版本、后升级的设备**：迁移后存量待办全是未知、`updatedAt` 与远端相同，迁移又清空了基准 ETag → 第一次同步无条件 GET，
    平局也采纳远端的 `listId`，学到真实归属且不触发上传；之后它上传时不会把文档里的 `listId` 改成 1（测试
    `upgraded_old_device_learns_list_ids_without_uploading`）
  - `todoLists` / `listTombstones` 经旧版本 `extra` 原样带回，清单不丢、已删清单不复活（测试
    `old_devices_keep_list_keys_and_their_edits_keep_the_local_list`）；旧版本「用本地覆盖云端」也带回 `extra`，它上传的待办都没有
    `listId`——其它设备把它当未知、沿用本地归属，**不再**全部落进默认清单（PRD「Technical Notes」最后一条的退化随 §12 消失）
  - 旧版本对**看不懂的远端**强制推送时 `extra` 为空，文档里没有清单信息：新设备不删本地清单，并因"远端缺本机清单"补传
  - `EXPORT_VERSION` 同时是同步文档的 `version`，现在写 `"4.1"`；旧 PC 与 cloud 都不校验它（cloud `doc.rs` 原样透传）
  - 旧版本导入 4.1 备份：`ExportData` 不拒绝未知字段，`todoLists` 被忽略、待办的 `listId` 被丢弃（测试
    `a_4_1_export_still_parses_with_the_4_0_shape`）

### 1.4 `pc-gpui/crates/core/README.md`

- 开头"同一迁移（v1~v29；v30 起…）"→ 补"v31 多待办清单"；"同一导出格式（4.0）"→"4.1（兼容 3.0 / 4.0）"
- **目录**：加 `lists.rs`、`window_prefs.rs`、`ids.rs`；`examples/seed.rs` 说明"导出格式 4.0"→"4.0 / 4.1"
- **`db`**：模型列表加 `TodoList`、`ListTombstone`、`DEFAULT_LIST_ID` / `DEFAULT_LIST_NAME` / `DEFAULT_LIST_CREATED_AT`、
  `TODO_LIST_COLUMNS` / `todo_list_from_row`；`Todo.list_id: Option<i64>`（`None` = 未知，序列化时不写）；`Database::new()` 一行"迁移 v1~v30"→"v1~v31"
- **`todo`**：`create` 说明加"`list_id` 缺省写默认清单 1（新建总是已知），指定的清单不存在报"清单不存在""；`update` 加"`list_id: Some`
  移到另一个清单（必须存在）；`None` 不动归属（未知保持未知）"；新增 `move_to_list(db, id, list_id) -> R<Todo>`（只改 `list_id`、
  `updated_at` 严格递增；存储的已是该清单时不改；未知时显式移动总会写成已知）
- **新增 `lists` 一节**（见本文 §2.1 表）
- **新增 `window_prefs` 一节**（见本文 §2.2 表）
- **新增 `ids` 一节**：`compose_id`、`new_id`、`insert_with_fresh_id`、`is_primary_key_conflict`、`MAX_SAFE_ID` / `MAX_ID_ATTEMPTS` /
  `FALLBACK_MIN_ID`（与 cloud `api/ids.rs` 逐字一致）
- **`data`**：`EXPORT_VERSION = "4.1"`；`export_json` 带 `todoLists`、未知归属不写 `listId`；`import_json` 兼容 3.0 / 4.0 / 4.1 + 清单语义
  （3.0 / 4.0 → 具体的 1；4.1 的未知 → 未知）
- **`sync`**：`SyncReport` 加 `lists_inserted / lists_updated / lists_deleted`、`lists_changed()`；`SyncData` 加 `todo_lists` / `list_tombstones`
- **`settings`**：`delete_screen_config` 说明改为"连带删除该组合的 `fence_layouts` 与 `window_layouts`"
- 「窗口状态留在 app」一行可补：清单 / 日历窗口的属性与布局持久化在 core `window_prefs`

### 1.5 第 6 步要改的 e2e / 脚本（本步没改、没跑）

- `pc-gpui/scripts/e2e-linux/run_e2e.py`
  - **F12**（约 530 行）：`data.get("version") == "4.0"` → `"4.1"`（导出版本）
  - **L01**（`make_legacy_db` 约 1136 行起、`l01` 约 1194 / 1199 / 1200 行）：docstring"最新库（v30）"→ v31；日志断言
    `"数据库将从 v27 升级到 v30"` → `v31`；返回文案 / 用例标题 `v28~v30` → `v28~v31`。旧库构造本身不改也能跑（v31 已可重跑，
    遗留的清单表 / 列 / 触发器不会让迁移失败）；想要真正的 v27 库，再剥掉 v31：`DROP TRIGGER trg_todo_lists_{insert,update,delete}_local_seq`、
    `DROP TABLE window_layouts / window_prefs / list_tombstones / todo_lists`、`DROP INDEX idx_todos_list_id`、`ALTER TABLE todos DROP COLUMN list_id`
    （可再加一条断言：升级后存量待办的 `list_id` 为 NULL、`webdav_remote_etag` 为空）
  - **L03**（约 1228–1237 行）：插入的"更新版本号"`31` → `32`，日志断言 `比本程序支持的 v30 新` → `v31`，标题 `(v31)` → `(v32)`
  - **L02** 断言 `view_mode` 设置会被点标题栏视图按钮改写——拆窗后视图改成按窗口存（`window_prefs`），由第 4 步决定是否还写全局键
  - **S 组**可加一条：旧格式远端（待办没有 `listId`）+ 本机已知归属 → 补传一次后远端带回 `listId`
- `pc-gpui/scripts/e2e-linux/README.md` L01 / L03 两行同上
- `pc-gpui/scripts/e2e-macos/run_e2e.py` P01 从源码读 `LATEST_VERSION`，不用改
- `pc-gpui/scripts/sample-data.json`（`"version": "4.0"`）照常可导入（待办写成默认清单 1）；需要清单样例时再改成 4.1 并加 `todoLists`

## 2. 给第 4 / 5 步（app）的接口

阻塞函数，放后台调用（`db::spawn_db`）。`R<T>` = `Result<T, String>`（中文错误）。

### 2.1 `mini_todo_core::lists`

| 函数 | 说明 |
|---|---|
| `list(db) -> R<Vec<TodoList>>` | 全部清单，按 `(sortOrder, id)`；默认清单缺失时先补建 |
| `get(db, id) -> R<TodoList>` | 不存在 `清单不存在`（`LIST_NOT_FOUND`） |
| `create(db, name) -> R<TodoList>` | 去首尾空白、1~50 字、不区分大小写不重名（`已有同名清单「<现有名称>」`）；排最后；新窗口直接进入改名由界面做 |
| `rename(db, id, name) -> R<TodoList>` | 默认清单也可；名称没变不写库；空名报 `清单名称不能为空`（界面"空名恢复原名"） |
| `delete(db, id) -> R<usize>` | 返回移到默认清单的待办数；默认清单 `默认清单不能删除`；同时删掉它的窗口记录、从日历筛选移出；归属未知的待办不动 |
| `todo_count(db, id) -> R<usize>` | 存储的 `list_id = id` 的待办数（含已完成，不含未知），删除确认框"N 项待办将移到「待办清单」"用 |
| `effective_list_id(list_id: Option<i64>, &[TodoList]) -> i64` | 读侧兜底：未知（`None`）或清单不存在 → 1。列表 / 四象限 / 已完成 / 日历筛选的归属都用它 |
| `normalize_name(&str) -> R<String>`、`names_conflict(a, b) -> bool` | 输入框预校验 |
| `DEFAULT_LIST_ID`、`DEFAULT_LIST_NAME`、`MAX_LIST_NAME_CHARS`、`LIST_NOT_FOUND` | 常量 |

待办：`Todo.list_id: Option<i64>`（`None` = 归属未知，显示在默认清单）；`CreateTodoRequest.list_id: Option<i64>`（`None` = 默认清单，
新建总是写具体值）、`UpdateTodoRequest.list_id: Option<i64>`（`None` = 不动归属）、`todo::move_to_list(db, id, list_id) -> R<Todo>`；
目标清单不存在都报 `清单不存在`。显示归属一律过 `effective_list_id`。app 目前 `build_create_request` 传 `list_id: None`
（`windows/editor/save.rs`），第 4 / 5 步改成打开方的清单。

**编辑表单的「清单」下拉**：只有用户真的改了选择才在 `UpdateTodoRequest` 里带 `list_id: Some(..)`。归属未知的待办下拉预选的是有效清单
（默认清单），用户没动它就必须传 `None`——否则每次编辑都会把未知写成已知的 1，以新版本盖掉别的设备上的真实归属（契约 §12-6）。

### 2.2 `mini_todo_core::window_prefs`

| 项 | 说明 |
|---|---|
| `WindowKey::{List(i64), Calendar}`、`parse` / `as_key` / `list_id`、`Display` | key 字符串 `list:<id>` / `calendar` |
| `ViewMode::{List, Quadrant}`、`as_str` / `parse` | 清单窗口视图 |
| `WindowPrefs { key, hidden, dark, view_mode, expand_all, calendar_filter: Option<Vec<i64>> }`、`WindowPrefs::new(key)` | 没有记录时的默认值：显示、浅色、列表、不展开、全部清单 |
| `get_prefs(db, key) -> R<Option<WindowPrefs>>` | `None` = 本机没有窗口记录（同步来的新清单 → 自动开窗） |
| `list_prefs(db) -> R<Vec<WindowPrefs>>` | 全部记录（按 key；看不懂的 key 跳过） |
| `update_prefs(db, key, &UpdateWindowPrefs) -> R<Option<WindowPrefs>>` | 只改给出的字段（`hidden` / `dark` / `view_mode` / `expand_all`），没有记录先按默认值补建；清单不存在 → `None`、不写 |
| `set_calendar_filter(db, Option<&[i64]>) -> R<Option<Vec<i64>>>` | `None` = 全部；去掉不存在的清单、去重升序；空 → 全部；返回实际保存值 |
| `get_layout(db, key, screen_id) -> R<Option<WindowLayout>>`、`layouts_for_screen(db, screen_id) -> R<Vec<WindowLayout>>` | 客户区物理像素 + `is_fixed`，按屏幕组合 |
| `save_layout(db, &WindowLayout) -> R<bool>`、`save_layouts(db, &[WindowLayout]) -> R<usize>` | upsert；清单不存在的跳过（`false` / 不计数）；「重置位置」用批量版 |
| `delete_window(db, key) -> R<()>` | 删属性 + 全部布局 |

要点：

- 迁移种子：`list:1` 已带原主窗口的深色 / 视图 / 各屏幕组合布局与锁定；`calendar` 的 `hidden` = 原来没开日历，开着日历时已有右侧布局。
  "原来没开日历：日历窗口第一次显示时贴默认清单右侧、与之等高"由界面在 `calendar` 没有当前组合布局时计算
- **展开全部子任务**（`ui-state.json` 的 `subtaskExpandAll`）需要 app 首次启动时迁移到 `list:1` 的 `expand_all`；core 没有提供"已迁移"标记，
  建议迁移后把该键从 `ui-state.json` 删掉，避免重复覆盖
- 默认清单窗口"启动时总是显示"：core 照存 `hidden`，由 app 启动时忽略 `list:1` 的 `hidden`
- "显示日历"（设置 → 常规 / 托盘勾选）= `calendar` 的 `!hidden`；同步键 `show_calendar` / `text_theme` / `view_mode` / `is_fixed` 只作迁移初始值
- 同步：`SyncReport::lists_changed()` 为真时重读清单、开 / 关清单窗口（同步来的清单删除时 core 已删掉窗口记录，app 只需关窗）。
  app 的 `services::sync::has_local_data_changes` 现在只看待办 / 子任务计数，`describe_sync_report` 也不显示清单计数，第 5 步要补。
  升级后第一次同步会把学到的归属计入 `todos_updated`（提示里是"更新待办 N"），界面随之重新拉待办，属预期
- 屏幕配置页：仍按 `screen_configs` 列出屏幕组合；`settings::delete_screen_config` 已连带删 `window_layouts`。如果拆窗后不再为主窗口写
  `screen_configs`，新出现的屏幕组合就不会出现在设置页里——要么继续用 `save_screen_config` 登记组合（显示名），要么给 core 加一个
  从 `window_layouts` / `fence_layouts` 汇总组合的查询

## 3. 与 cloud 的对齐点（合并时请与 cloud 代理的实现逐条核对）

1. `todoLists[].sortOrder`：PC 读写用 **i64**（读侧不因范围丢清单）；cloud 若按 i32 校验 `PATCH`，PC 也能读
2. 重名错误文案用**现有清单**的名称：`已有同名清单「Work」`（输入 `work` 时）；不区分大小写用 Rust `to_lowercase()` 比较（去首尾空白后）
3. `entityId = 1` 的清单墓碑：PC 解析时丢弃、从不写出；默认清单无论如何不删
4. 远端文档没有 `todoLists` 键：合并、强制拉取都不动本地清单（强制拉取连清单墓碑也不动）
5. todo 的 `listId`：**缺失 / `null` = 未知**（契约 §12，不再是 1）；类型错误（如字符串）与其它字段一样按"无法识别的记录"跳过。
   远端胜出但缺 `listId` → 沿用本地；远端有、本地未知 → 不论 LWW 谁胜都采纳、不改 `updatedAt`；写出时未知不带 `listId`
6. 同步文档 `version` 变成 `"4.1"`（`EXPORT_VERSION` 兼作同步文档版本）
7. 新清单 id：`ids.rs` 常量与 cloud 逐字一致（`MAX_SAFE_ID = 2^53 − 1`、`MAX_ID_ATTEMPTS = 8`、`FALLBACK_MIN_ID = 1e15`）；
   PC 的随机数用标准库 `RandomState`（没有新增依赖，`Cargo.lock` 未改）
8. PC 端特有（cloud 不需要对应实现）：清单删除时清理窗口记录；补传（远端 `listId` 缺失 / null、本机已知任何值）；"没有 `todoLists` 键 ≡
   种子状态的默认清单"的上传判断；强制拉取不让远端的未知覆盖本地已知；强制推送前先采纳远端已知的归属；迁移 v31 作废基准 ETag

## 4. 契约 §12（修订 1：`listId` 缺失 = 未知）在 PC 的落点

| 条 | 落点（`pc-gpui/crates/core/src/`） | 测试 |
|---|---|---|
| 1 缺失 / null = 未知；类型错误跳过 | `db/models.rs` `Todo.list_id: Option<i64>`（`#[serde(default)]`） | `models::missing_or_null_list_id_means_unknown` |
| 2 远端胜出缺 `listId` → 沿用本地 | `db/sync_store.rs` `merge_remote`（远端较新分支补上本地的 `list_id`）；强制拉取同理（`force_pull_replace`） | `sync_store::missing_list_id_means_unknown_and_never_overrides_a_known_list`、`force_pull_keeps_a_known_list_when_the_remote_does_not_know_it`、`sync::tests::old_devices_keep_list_keys_and_their_edits_keep_the_local_list` |
| 3 远端已知、本地未知 → 平局 / 本地较新也采纳，不改 `updatedAt`，不额外上传 | `sync_store::adopt_list_id`（`… AND list_id IS NULL`），在 `merge_remote` 平局 / 本地较新分支调用；强制推送前也调用（`force_push_prepare`） | `sync_store::unknown_local_todos_adopt_a_known_remote_list_whoever_wins`、`force_push_prepare_adopts_known_remote_lists_for_unknown_local_todos`、`sync::tests::upgraded_old_device_learns_list_ids_without_uploading`（断言 PUT 次数为 0、`updated_at` 不变） |
| 4 有效清单：未知 → 1 | `lists::effective_list_id(Option<i64>, &[TodoList])` | `lists::effective_list_falls_back_to_the_default_list` |
| 5 上传 / 导出：未知不写 `listId` | `Todo.list_id` 的 `skip_serializing_if = "Option::is_none"`（上传文档、导出都经 serde） | `models::unknown_list_id_is_not_written_but_known_ones_are`、`data::export_4_1_carries_lists_and_round_trips`、`sync::tests::upgraded_old_device_learns_list_ids_without_uploading` |
| 6 只有显式设置清单才变已知 | `todo::create`（总写具体值）、`todo::update`（`list_id: None` 不动）、`todo::move_to_list`（`list_id IS NOT ?` 对 NULL 成立）、`lists::delete` 只改 `list_id = X` 的行 | `todo::unknown_list_stays_unknown_until_a_list_is_set_explicitly`、`lists::deleting_a_list_moves_its_todos_to_the_default_list_and_writes_a_tombstone` |
| 7 存储可空、迁移存量为 NULL、新建写具体值 | `db/migrations.rs` v31 `ALTER TABLE todos ADD COLUMN list_id INTEGER`（无默认值） | `migrations::v31_creates_the_default_list_and_leaves_existing_todos_unknown`、`every_historical_version_upgrades_with_its_data` |
| 8 补传：远端缺、本机已知（任何值） | `sync/mod.rs` `remote_lost_list_ids`（远端缺键或 null、文档里同一条有整数 `listId`） | `sync::tests::old_devices_keep_list_keys_and_their_edits_keep_the_local_list`（含已知的 1） |
| 9 导入 3.0 / 4.0：缺失 → 具体的 1 | `data::import_records`（没有 `todoLists` 的备份把 `None` 改成 `Some(1)`；4.1 备份的未知保持未知） | `data::importing_a_4_0_backup_leaves_existing_lists_alone`、`export_4_1_carries_lists_and_round_trips` |
| 10 升级后第一次同步无条件 GET | v31 迁移里 `settings_kv::set_setting(conn, "webdav_remote_etag", "")` | `migrations::v31_creates_the_default_list_and_leaves_existing_todos_unknown`、`sync::tests::upgraded_old_device_learns_list_ids_without_uploading`（断言 GET 不带条件） |
| 11 cloud 侧 | 不涉及 PC | — |
