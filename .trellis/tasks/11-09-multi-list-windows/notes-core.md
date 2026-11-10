# 第 1 步（PC core 数据层）交付备忘：待更新的文档、e2e 影响与对接要点

> 本步只改了 `pc-gpui/crates/core`（外加 app 测试里让工作区编译通过的几处字面量），没有改 `CLAUDE.md`、`.trellis/spec/**`、
> `crates/core/README.md` 与 e2e 脚本。下面按文件 / 章节列出需要统一更新的内容。契约以 `contract.md` 为准（含 §12 修订 1：
> `listId` 缺失 = 未知；§13 修订 2：`listId` 按 `listUpdatedAt` 做字段级 LWW、清单墓碑永不过期，**取代 §12 的第 2、3、8 条**——
> 下文凡与 §13 冲突的旧说法以 §5 为准）；core 的代码注释里有 4 处引用 `.trellis/tasks/11-09-multi-list-windows/contract.md`（`ids.rs`、`lists.rs`、
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
| `263b7ad` | **契约 §13 修订**：`todos.list_updated_at` + `Todo.listUpdatedAt`、显式设置清单时刷新、归属字段级合并（`ListAssignment`）、补传条件、强制推送 / 拉取、清单墓碑永不过期、导入刷新归属版本 |
| `f5a0445` | §13 的同步引擎层单测（F1 两台设备两种顺序、§13-6 补传、F3 降级再升级、清单墓碑超过 30 天） |

> 用 `554bda6` 之前的 v31 建过的**开发 / 测试库**里 `todos.list_id` 还是 `NOT NULL DEFAULT 1`（v31 可重跑，但不会改已有列的约束），
> 合并同步时写 NULL 会撞约束、被当作坏记录跳过——请删掉重建。用 `263b7ad` 之前的 v31 建过的库**没有 `todos.list_updated_at` 列**
> （库已是 v31，不会再跑 v31），读待办会报 `no such column: list_updated_at`——同样删掉重建，或手动
> `ALTER TABLE todos ADD COLUMN list_updated_at TEXT`。v31 没有发布过，用户库不受影响（契约 §13-8：直接改 v31，不加 v32）。

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
    归属未知，不等于默认清单；存量待办为 NULL；不加外键）与 `todos.list_updated_at TEXT`（可空，`list_id` 自己的版本，契约 §13；
    存量为 NULL；只有显式设置清单时才写）；作废 `webdav_remote_etag`（升级后第一次同步无条件 GET，存量待办尽快学到归属）；
    `list_tombstones(entity_id, deleted_at)`（不计数，永不过期）；设备本地的 `window_prefs`（每窗口：`window_key` = `list:<id>` / `calendar`，
    `hidden` / `dark` / `view_mode` / `expand_all` / `calendar_filter`）与 `window_layouts`（每 (窗口, 屏幕组合)：`x / y / width / height /
    is_fixed`，随 `window_prefs` 级联删除），不接 `local_seq`；种子：`list:1` 继承原主窗口（`text_theme` / `view_mode` / 各 `screen_configs`），
    `calendar` 隐藏 = `!show_calendar`，开着日历时按左 40% / 右 60% 切开；可重跑
- **「主要数据表」**：新增 `todo_lists`（待办清单，参与同步）、`list_tombstones`（清单删除墓碑，永不过期）、
  `window_prefs`（清单 / 日历窗口的设备本地属性）、`window_layouts`（清单 / 日历窗口按屏幕组合的布局与锁定）
- **「数据导入导出与同步 → 关键文件」**：模型加 `TodoList`、`ListTombstone`；清单 API `lists.rs`；清单合并 / 墓碑
  `db/sync_store.rs`（`merge_remote_lists`、`force_pull_replace_lists`、`force_push_prepare_lists`）；窗口属性 `window_prefs.rs`
- **「sync-data.json.gz 结构」**：`"version": "4.1"`；归属已知的 todo 带 `"listId": <清单 id>`（缺失 / `null` = **未知**，不等于 1），
  有版本时一并带 `"listUpdatedAt": "YYYY-MM-DD HH:MM:SS"`（归属自己的版本，契约 §13；未知时两个键都不写）；新增
  `"todoLists": [ { id, name, sortOrder, createdAt, updatedAt } ]`（全部未删除清单，含 id 1）与
  `"listTombstones": [ { "entityId": …, "deletedAt": "YYYY-MM-DD HH:MM:SS" } ]`（不进 `tombstones`）
- **「合并规则」**追加两条：
  - 清单与待办同构（LWW 平局保留本地、`deletedAt >= updatedAt` 压制、并集、墓碑并集取较大），但清单墓碑**永不过期**（契约 §13-10）；
    远端没有 `todoLists` 键 = 没有清单信息，本地清单一条不删；默认清单不可删除，`entityId = 1` 的清单墓碑一律忽略；收到清单墓碑不改写待办
    （读侧兜底进默认清单）；被删清单的窗口记录同一事务里清理
  - 待办的归属（契约 §12 + §13）：`(listId, listUpdatedAt)` 字段级 LWW，与记录的 `updatedAt` 无关。记录级结果照旧；结果记录存在时，
    归属取所有可见版本（本地合并前的版本——在应用墓碑之前读出——与远端版本，哪怕被墓碑压制）里最新的：未知（缺失 / `null`）最旧，
    已知但没有版本（缺失 / 无法识别）次旧，有版本的按时间比较，一样新时取较大的 `listId`。采纳不改 `updatedAt`、不额外上传；
    合并后远端那一版的归属比本机旧（含远端缺 `listId`）时补传一次。畸形 / 本机无法识别的远端记录不参与。上传 / 导出时未知两个键都不写
- **「当前同步覆盖范围」**：新增行
  - `todo_lists` | 是 | 清单名称 / 顺序，`todoLists`；默认清单 id 1 不可删除
  - `list_tombstones` | 是 | 独立顶层键 `listTombstones`（不进 `tombstones`，旧版本经 extra 原样带回），永不过期
  - `window_prefs` / `window_layouts` | 否 | 清单 / 日历窗口的设备本地属性与布局（同 `screen_configs`）；也不进导入导出
  - `todos` 行说明加"含 `listId` / `listUpdatedAt`（归属未知时都不写）"
  - 「数据库设计 → 主要数据表」或「合并规则」的墓碑一句"保留 30 天"改成"待办 / 子任务墓碑保留 30 天，清单墓碑永不过期"
- **「维护检查清单」**第 3 条可补一句：参与同步的新表照 `todo_lists` 的做法（触发器 + `sync_store` 读写 / 合并 / 墓碑 +
  `build_sync_doc` 显式顶层键 + `doc_differs`）；第 4 条可补：旧版本不认识的新记录字段要区分"缺失 = 未知"与默认值，见 `listId`
- **「WebDAV 同步流程」**图：合并一行加"清单（todoLists / listTombstones）同一事务；待办归属按 listUpdatedAt 字段级合并"；
  "需要上传？"一行加"远端那一版的归属比本机旧"
- **「数据库设计」** 也可注明：`SyncReport` 新增 `lists_inserted / lists_updated / lists_deleted` 与 `lists_changed()`

### 1.2 `.trellis/spec/backend/sync-protocol.md`

- **§2 Signatures**：
  - `sync_meta.local_seq` 由 `todos` / `subtasks` / `todo_lists` 的触发器维护
  - PC `db/sync_store.rs` 新增：`merge_remote_lists(tx, Option<&[RemoteList]>, &[ListTombstone])`、
    `force_pull_replace_lists(tx, Option<&[RemoteList]>, &[ListTombstone])`（§13 后都没有 `cutoff` 参数）、
    `force_push_prepare_lists(tx, Option<&[Value]>, &[ListTombstone], now)`、`parse_list_tombstone`、`record_list_tombstone`（id 1 不存）、
    `load_list_tombstones(conn)`（全部，永不过期；`purge_list_tombstones_before` 已删除）；归属 `ListAssignment`
    （`new` / `of` / `from_value` / `is_newer_than` / `newest`，契约 §13-3 的比较规则）
  - PC `db/time.rs`：`SQL_SET_LIST_UPDATED_AT`（无条件，配合 `WHERE list_id IS NOT ?`）、`SQL_SET_LIST_UPDATED_AT_IF_MOVED`
    （多字段 UPDATE 用，一个匿名占位符 `?` 是新的 `list_id`）；`sync/mod.rs`：`remote_assignments_stale`（补传判定）
  - `SyncReport` 新增 `listsInserted / listsUpdated / listsDeleted`，`local_changed()` 含清单，`lists_changed()`；
    只采纳归属（见 §3.3 第 9 条）同样计入 `todosUpdated`
- **§3.1 Document**：`"version": "4.1"`；todo 的 `listId`（整数；**缺失 / null = 未知**，不等于 1；类型错误按无法识别的记录跳过）与
  `listUpdatedAt`（规范时间，`listId` 自己的版本；缺失 / 无法识别 = 已知但没有版本；非字符串当作缺失、不让整条记录作废）；
  `todoLists`、`listTombstones`（见 1.1）；"写入方每次上传都写出全部未删除的清单（含 id 1）与全部清单墓碑（永不过期）；
  待办的归属未知时 `listId` / `listUpdatedAt` 都不写"
- **§3.3 Merge** 追加：
  - 第 8 条：清单规则（同 1.1「合并规则」），并注明两端一致（PC `merge_remote_lists`、cloud `sync/merge.rs`）；清单墓碑永不过期
  - 第 9 条（契约 §12 + §13）：归属字段级 LWW（同 1.1「合并规则」第二条）。PC：`merge_remote` 开头 `load_assignments`（应用墓碑之前），
    逐条取远端 / 合并前 / 当前三者最新的；远端胜出时随记录写入，平局 / 本地较新 / 远端被墓碑压制时 `set_assignment` 只改两列、不改
    `updated_at`（触发器让 `local_seq` 加一，但本次同步收尾时 `synced_seq` 取合并后的值，不会因此上传）
- **§3.4 WebDAV requests** 的 Self-heal 行补两点（PC）：
  - 远端缺本机的清单 / 清单墓碑时照常补传；远端**没有 `todoLists` 键**时按"只有种子状态的默认清单"
    （id 1、`2000-01-01 00:00:00`）比较——刚升级、没改过默认清单的设备不会为它多上传一次
  - 远端某条待办那一版的归属比上传文档里的旧（含远端 `listId` 缺失 / null、本机已知；同时间不同 `listId` 时持有较大值的一端）时上传一次
    （`sync/mod.rs::remote_assignments_stale`，契约 §13-6；取代 §12 的 `remote_lost_list_ids`）
  - 新增一行「升级后第一次同步」：迁移 v31 清空 `webdav_remote_etag`，第一次智能同步是无条件 GET
- **§3.5 Force operations**：ForcePull 让本地清单 = 远端（删本地独有、不写墓碑；默认清单永远保留；清单墓碑表替换为远端；
  远端没有 `todoLists` 键时清单与清单墓碑都不动），远端待办缺 `listId` 时保留本地已知的归属（两列一起），远端已知时连同版本照搬；
  ForcePush 为远端独有的清单写墓碑（id 1 除外），远端较新 / 远端墓碑会压制的本地清单 `updated_at` 推到 superseding；待办归属：
  本地未知 → 采纳远端的归属，远端的归属更新 → 本地归属的版本推到 `superseding(now, 远端版本)`
- **§3.6 设备本地数据** 表新增：`window_prefs` 表 | v31 | 清单窗口 `list:<id>` / 日历窗口 `calendar` 的隐藏 / 深色 / 视图 / 展开全部 /
  日历筛选；`window_layouts` 表 | v31 | 每 (窗口, 屏幕组合) 的客户区物理像素几何与锁定。规则：不接 `local_seq`；清单被删除（本机 / 同步 /
  强制拉取 / 导入）时窗口记录在同一事务里删除；删屏幕配置连带删该组合的窗口布局。测试：`migrations::v31_window_tables_are_device_local_and_layouts_cascade`、
  `window_prefs::window_prefs_are_device_local`
- **§6 Tests Required**：加 `sync_store` 的清单合并测试（`remote_lists_merge_by_lww_and_ties_keep_local` 等）与 listId 规则测试
  （`missing_list_id_means_unknown_and_never_overrides_a_known_list`、`unknown_local_todos_adopt_a_known_remote_list_whoever_wins`、
  `force_pull_keeps_a_known_list_when_the_remote_does_not_know_it`、`force_push_prepare_adopts_known_remote_lists_for_unknown_local_todos`），
  以及 `sync/tests.rs` 的 `lists_and_list_deletions_sync_in_both_directions`、`old_devices_keep_list_keys_and_their_edits_keep_the_local_list`、
  `upgraded_old_device_learns_list_ids_without_uploading`、`two_devices_creating_lists_offline_keep_both`、`force_push_and_force_pull_cover_lists`；
  §13 的测试见本文 §5.1 的表

### 1.3 `.trellis/spec/pc-gpui/data-compat.md`

- **§3 规则表**加两行：
  - 「同步文档新顶层键」：显式声明成 `SyncData` 字段（`#[serde(default)]`，**不**放进 `extra`），每次上传都写出；旧版本经自己的 `extra`
    原样带回；需要删除语义的新实体用独立墓碑键，不进共用 `tombstones`（旧版本丢弃未知 `entityType`）
  - 「旧版本不认识的新记录字段」：旧版本按自己的本地版本重新序列化**全部**记录、丢掉新字段（平局保留它自己的版本）。所以这类字段的
    "缺失"要当作**未知**而不是默认值：合并时未知不覆盖已知、远端已知时采纳，存储上要能表达未知（`listId` → `todos.list_id` 可空）
- **新增一节「多清单与旧版本（≤ 3.0.6 / 2.5.0）」**（替换初版里"旧设备改过的待办落进默认清单（接受的退化）"的说法）：
  - 旧版本打开 v31 库：只记警告照常运行；`todos.list_id` 可空、没有默认值，旧版本按显式列 INSERT 的待办是"未知"，新版本照常显示在
    默认清单、同步时学到真实归属；不认识清单表
  - 旧设备上传会丢掉**所有**它重新序列化的待办的 `listId` / `listUpdatedAt`。新设备把缺失当作未知（最旧的归属）：它改过的待办（更新）
    内容取它的、**归属取本地的**（字段级合并，契约 §13）；cloud 与还没同步过的设备先看到未知（显示在默认清单），新设备随后补传一次补回
    （`remote_assignments_stale`；两台新设备之间不会来回，旧设备只在自己有变更时才上传）
  - **降级到 3.0.x 再升级回来**（F3）：旧程序改记录时不碰 `list_id` / `list_updated_at`，留下陈旧的归属；升级后别处更新的归属
    （`listUpdatedAt` 更新）照样胜出，不会被本机陈旧的值永久分叉（测试 `a_downgraded_device_learns_the_newer_list_after_upgrading`，
    `sync_store` 与 `sync/tests.rs` 各一个）
  - **≤ 2.4.x 的写入方**（没有 `extra`、没有 `tombstones`）会抹掉 `todoLists` / `listTombstones` / `listId`，**不支持**与多清单版本混用
    （契约 §13-12，F8）
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
  `TODO_LIST_COLUMNS` / `todo_list_from_row`；`Todo.list_id: Option<i64>`（`None` = 未知，序列化时不写）与
  `Todo.list_updated_at: Option<String>`（归属的版本，契约 §13；没有时不写，归属未知时恒为 `None`）；`sync_store::ListAssignment`；
  `time::SQL_SET_LIST_UPDATED_AT` / `SQL_SET_LIST_UPDATED_AT_IF_MOVED`；`Database::new()` 一行"迁移 v1~v30"→"v1~v31"
- **`todo`**：`create` 说明加"`list_id` 缺省写默认清单 1（新建总是已知，`list_updated_at` = 现在），指定的清单不存在报"清单不存在""；
  `update` 加"`list_id: Some` 移到另一个清单（必须存在；清单真的变了才刷新 `list_updated_at`）；`None` 不动归属（未知保持未知）"；
  新增 `move_to_list(db, id, list_id) -> R<Todo>`（只改 `list_id`，`list_updated_at` 与 `updated_at` 都严格递增；存储的已是该清单时不改；
  未知时显式移动总会写成已知）
- **新增 `lists` 一节**（见本文 §2.1 表）
- **新增 `window_prefs` 一节**（见本文 §2.2 表）
- **新增 `ids` 一节**：`compose_id`、`new_id`、`insert_with_fresh_id`、`is_primary_key_conflict`、`MAX_SAFE_ID` / `MAX_ID_ATTEMPTS` /
  `FALLBACK_MIN_ID`（与 cloud `api/ids.rs` 逐字一致）
- **`data`**：`EXPORT_VERSION = "4.1"`；`export_json` 带 `todoLists`、未知归属不写 `listId` / `listUpdatedAt`；`import_json` 兼容
  3.0 / 4.0 / 4.1 + 清单语义（3.0 / 4.0 → 具体的 1；4.1 的未知 → 未知；已知归属的 `listUpdatedAt` 刷新为导入时刻、严格晚于本机原来的版本）
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
  - **S 组**可加一条：旧格式远端（待办没有 `listId`）+ 本机已知归属 → 补传一次后远端带回 `listId`（与 `listUpdatedAt`）；
    以及 §13 的 F1：两台设备一台「移动到」、一台拿旧副本改标题，两种同步顺序都收敛到移动后的清单
  - L01 的旧库构造如果要剥掉 v31，还要 `ALTER TABLE todos DROP COLUMN list_updated_at`（可加断言：升级后存量待办两列都是 NULL）
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
§13 之后：带着与存储相同的清单（`Some(原值)`）不会刷新归属的版本（无害）；但未知时带 `Some(1)` 会写出带**最新版本**的 1，
在字段级合并里压过别处真实的「移动到」，所以上面这条仍然必须遵守。`Todo.list_updated_at` 只给同步 / 导出用，界面不需要读写。

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
   归属按 `(listId, listUpdatedAt)` 字段级 LWW（契约 §13，取代原先的"沿用 / 采纳"），细节与 cloud 的逐条核对见本文 §5.2
6. 同步文档 `version` 变成 `"4.1"`（`EXPORT_VERSION` 兼作同步文档版本）
7. 新清单 id：`ids.rs` 常量与 cloud 逐字一致（`MAX_SAFE_ID = 2^53 − 1`、`MAX_ID_ATTEMPTS = 8`、`FALLBACK_MIN_ID = 1e15`）；
   PC 的随机数用标准库 `RandomState`（没有新增依赖，`Cargo.lock` 未改）
8. PC 端特有（cloud 不需要对应实现）：清单删除时清理窗口记录；"没有 `todoLists` 键 ≡ 种子状态的默认清单"的上传判断；强制拉取不让
   远端的未知覆盖本地已知（远端已知时连同版本照搬）；强制推送前先采纳远端已知的归属、远端的归属更新时把本机的版本推过它；
   迁移 v31 作废基准 ETag；导入刷新归属的版本

## 4. 契约 §12（修订 1：`listId` 缺失 = 未知）在 PC 的落点

> 第 2、3、8 条已被 §13 取代：`adopt_list_id`、`remote_lost_list_ids` 已删除，下表这三行是历史落点，现状见 §5.1
> （由 `merge_remote` 的字段级合并、`force_push_prepare` 与 `remote_assignments_stale` 承担）。

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

## 5. 契约 §13（修订 2：`listId` 字段级 LWW）在 PC 的落点

### 5.1 各条落点（`pc-gpui/crates/core/src/`）

| 条 | 落点 | 测试 |
|---|---|---|
| 1 `listUpdatedAt` = 归属自己的版本 | `db/models.rs` `Todo.list_updated_at`（`de_lenient_time`：字符串照收，`null` / 其它类型当作缺失）；`todo_from_row` 在归属未知时丢掉版本；`db/sync_store.rs` `ListAssignment`（规范化构造 `new` / `of` / `from_value`） | `models::list_updated_at_is_lenient_and_only_written_when_present` |
| 2 只有显式设置清单才写，`max(now, 旧值 + 1 秒)` | `todo::create`（`datetime('now','localtime')`）、`todo::move_to_list`（`SQL_SET_LIST_UPDATED_AT`，`WHERE list_id IS NOT ?1`）、`todo::update`（`SQL_SET_LIST_UPDATED_AT_IF_MOVED`：清单真的变了才刷新）、`lists::delete`（移进默认清单时刷新）；`db/time.rs` 两个 SET 片段 | `todo::list_version_only_moves_when_the_list_is_set_explicitly`、`lists::deleting_a_list_moves_its_todos_to_the_default_list_and_writes_a_tombstone` |
| 3 新旧比较：未知 < 已知无版本 < 按时间；一样新取较大 `listId` | `ListAssignment::rank`（`(级别, 规范时间, listId)` 字典序）、`is_newer_than`、`newest` | `sync_store::list_assignments_order_by_their_own_version`、`equally_new_assignments_pick_the_larger_list_id` |
| 4 结果记录存在时取所有可见版本里最新的；墓碑之前读出本地归属 | `sync_store::merge_remote`：开头 `load_assignments`（应用墓碑之前），逐条 `best = 远端 ⊔ 合并前 ⊔ 当前`；插入 / 远端胜出时 `with_assignment` 随记录写入；平局 / 本地较新 / 远端被墓碑压制且本地还在时 `set_assignment` | `sync_store::a_stale_copy_cannot_undo_a_newer_move_in_either_order`（F1）、`tombstones_do_not_hide_assignment_versions`（F2 / F4）、`a_downgraded_device_learns_the_newer_list_after_upgrading`（F3）；`sync::tests::a_stale_copy_cannot_undo_a_move_in_either_order`（两台设备、两种顺序） |
| 5 采纳不改 `updatedAt`、不多上传 | `set_assignment` 只改两列；同步收尾 `synced_seq` 取合并后的 `local_seq` | `sync::tests::upgraded_old_device_learns_list_ids_without_uploading`（PUT 次数 0、`updated_at` 不变） |
| 6 补传：远端那一版的归属比本地旧 | `sync/mod.rs` `remote_assignments_stale` + `doc_assignments`（上传文档 vs 远端，同 id 取最新、畸形的跳过），在 `doc_differs` 里 | `sync::tests::a_stale_assignment_on_the_remote_is_fixed_by_one_upload`、`old_devices_keep_list_keys_and_their_edits_keep_the_local_list` |
| 7 上传 / 导出：未知两个键都不写 | serde `skip_serializing_if` + `normalize_todo_lenient` / `normalize_todo` / `todo_from_row` / `insert_todo_row` / `update_todo_row` 都在未知时去掉版本 | `models::…`、`data::export_4_1_carries_lists_and_round_trips` |
| 8 存储：可空列，v31 存量为 NULL | `db/migrations.rs` v31 `ALTER TABLE todos ADD COLUMN list_updated_at TEXT`（可重跑，`has_column`） | `migrations::v31_creates_the_default_list_and_leaves_existing_todos_unknown` |
| 9 导入刷新归属的版本 | `data::import_records`：已知归属（含 3.0 / 4.0 落成的 1）的版本 = `superseding(now, 本机原来的版本)`，未知保持未知 | `data::export_4_1_carries_lists_and_round_trips`、`import_4_1_restores_lists_like_a_backup`、`importing_a_4_0_backup_leaves_existing_lists_alone` |
| 10 清单墓碑永不过期 | `sync_store::load_list_tombstones(conn)`（不再按截止过滤）、`merge_remote_lists` / `force_pull_replace_lists`（去掉 `cutoff` 参数）、`purge_list_tombstones_before` 删除（`finish` 不再清理）、`sync/mod.rs` `build_sync_doc` / `list_tombstone_set` 不按截止过滤 | `sync::tests::list_tombstones_never_expire` |
| 11 畸形记录不参与 | `merge_remote` 只用解析成功的远端记录（`listId` 类型不对 → serde 失败；`updatedAt` 无法识别 → `normalize_todo` 失败）；JSON 层的 `ListAssignment::from_value`（强制推送、补传判定）同样拒绝这两种 | `sync_store::malformed_remote_records_take_no_part_in_assignments` |
| 12 ≤ 2.4.x 不支持混用 | 文档项，见 §1.3 | — |

强制操作（契约没写，PC 自定）：`force_pull_replace` 远端已知时连同 `listUpdatedAt` 照搬（"云端覆盖本地"），远端未知时保留本地两列；
`force_push_prepare` 本地未知 → 采纳远端的归属，远端的归属更新 → 本地归属的版本推到 `superseding(now, 远端版本)`（测试
`force_push_prepare_adopts_known_remote_lists_for_unknown_local_todos`、`force_pull_keeps_a_known_list_when_the_remote_does_not_know_it`）。

### 5.2 自行做的决定，与 cloud `notes-cloud.md` §5.2 的核对

1. **"无法识别"的范围**：PC 的远端记录只要整条解析失败（任何字段类型不对、任一时间字段无法识别）就不参与归属合并——沿用 K3-6
   "无法识别的远端记录跳过"，比契约列的两种（`listId` 类型不对、`updatedAt` 无法识别）略宽。cloud §5.2-2：畸形记录记录级胜出时 cloud 存
   远端正文、归属换成其它可见版本里最新的；PC 整条跳过（本机保留旧内容），上传文档里原样带回远端那条（`merge_todo_values` 的既有规则），
   PC 不会因它补传，cloud 会补传一次合法归属，之后 PC 照常收下——两端最终一致，不会来回
2. **`listUpdatedAt` 不是字符串**（数字 / 对象）：当作缺失，不让整条记录作废；字符串但识别不了：当作"已知但没有版本"，远端胜出写库时
   去掉（与 cloud §5.2-4 一致：两端都当没有时间，不来回推送）
3. **归属未知时残留的 `listUpdatedAt`**：读、合并、上传、导出都丢掉（与 cloud §5.2-5 一致）
4. **编辑时"值变了"**：与存储比较（`list_id IS NOT ?`），未知 → 已知算变了；同值不补版本；新版本 `max(now, 旧值 + 1 秒)`
   （与 cloud §5.2-6 一致）
5. **同时间不同 `listId`**：合并时持有较小值的一端直接采纳较大的（不上传），持有较大值的一端认为远端落后、补传一次（与 cloud §5.2-7 一致）
6. **导入的"导入时刻"**：取 `superseding(now, 本机同 id 原来的 list_updated_at)`，与记录 `updated_at` 的导入规则相同（契约字面是
   "刷新为导入时刻"；本机原版本来自时钟偏快的设备时，只取 now 会输给它）
7. **不加 v32**：契约 §13-8 要求直接改 v31。开发库要重建，见 §0 的提示
8. **补传判定比较的是上传文档与远端**（不是本地库与远端），同 id 多份时各取最新；远端本来就缺的待办由记录版本比较负责
