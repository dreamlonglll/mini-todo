# 多清单第 2 步（cloud）：要同步到根 CLAUDE.md / spec 的内容

> cloud 代理不改 `CLAUDE.md` 与 `.trellis/spec/**`，这里列出合并时要改的地方（建议文字可直接用）。
> 实现见本分支改 `cloud/` 的提交；cloud 自己的文档（`cloud/README.md`、`cloud/skill/minitodo/SKILL.md`）已随提交更新。
> 第 4 节的细节已与并行中的 PC core 实现（`pc-gpui/crates/core/src/lists.rs`、`db/sync_store.rs`、`db/models.rs`）逐条对照过。

## 1. 根 CLAUDE.md

### 1.1「cloud/ 子项目（云端 API）」的契约列表

在"墓碑"一条后面加两条：

```markdown
- **清单**：`todo_lists(id, data_json, updated_at)` + `list_tombstones(entity_id, deleted_at)`（KV，清单记录的未知字段原样保留）；
  同步文档顶层 `todoLists` / `listTombstones`，合并规则与 todo 同构、与 PC `sync_store` 一致（远端没有 `todoLists` 键时本地清单一条都不删）；
  默认清单 id 1 在建库与每次合并时按固定初始值补建（"待办清单"、`sortOrder` 0、`2000-01-01 00:00:00`），不可删除，`entityId = 1` 的清单墓碑一律忽略；
  **清单墓碑永不过期**（契约 §13-10；todo 墓碑仍 30 天）；
  REST 写入的 `listId` 必须指向未删除的清单；`DELETE /lists/:id` 单事务把其下待办移到默认清单（刷新 `updatedAt` 与 `listUpdatedAt`）并写清单墓碑；
  `GET /todos` 的 `listId=` / `list=` 按有效清单过滤（`listId` 缺失 / 指向已删除清单的待办算默认清单，记录不改写）；清单不分配短码
- **待办的清单归属**（契约 §12 + §13）：`listId` 缺失或 `null` = 未知（写入方不认识清单），只在显示 / 过滤时算默认清单，cloud 从不替它补值；
  `listId` 有自己的版本 `listUpdatedAt`（服务端维护：客户端传入的忽略，只在 `POST`、`PATCH` 改了 `listId`、删除清单移进默认清单时刷新），
  合并时 `(listId, listUpdatedAt)` 做字段级 LWW（未知 < 已知没有时间 < 已知有时间，一样新取较大的 `listId`；记录级结果照旧，归属取本地
  合并前与远端版本——含被墓碑压制的——里最新的，采纳不改 `updatedAt`、不标脏；畸形记录不参与）；合并后远端那一版的归属比本地旧 → 补传一次；
  上传时未知不写两个键
- **升级**：从不认识清单的旧版 cloud 升级后第一次启动作废一次基准 ETag（旧版把 `todoLists` / `listTombstones` 当未知键存进了信封），
  保证第一次 GET 是全量 GET 并把清单合并进来、§12 的采纳规则尽快生效（meta 标记 `lists_base_reset_v1`）
```

"字段契约"一条里的 `todos(id, data_json, updated_at)` 可改成 `todos` / `todo_lists` 等表都是 `(id, data_json, updated_at)`。

### 1.2 cloud 目录树

```
├── src/model.rs              # 记录字段契约（todo / subtask / 清单）：写入校验、别名映射、派生 priority、清单名称规则
├── src/db/                   # todos / subtasks / todo_lists / settings / meta / tombstones / list_tombstones / todo_seq；normalize.rs 修存量数据
├── src/sync/doc.rs           # sync-data 文档读写（todoLists / listTombstones；保留未知顶层键）
├── src/sync/merge.rs         # 与 PC 一致的记录 LWW + 墓碑 + 清单 + 设置合并
├── src/api/                  # axum：Bearer 鉴权、X-Sync-Status 头、5xx 不外泄内部细节；lists.rs 是清单 CRUD
└── skill/minitodo/           # Claude Code Skill：SKILL.md + minitodo.py（含单测，lists 子命令与 --list）+ install.{sh,ps1}
```

### 1.3 REST API 表

`GET /todos?...` 一行的 query 加 `listId` / `list`（按有效清单）；在 subtasks 几行后面加：

```markdown
| GET | `/lists` | 全部清单 `[{id, name, sortOrder, createdAt, updatedAt}]`，按 `(sortOrder, id)`；默认清单 id 1 总在 |
| GET | `/lists/:id` | 单个清单；`:id` 是数字 id 或 URL 编码的清单名（先按 id、再按名称不区分大小写匹配；重名 409、找不到 404） |
| POST | `/lists` | 新建；必填 `name`，可选 `sortOrder`（缺省排最后）→ 201；名称空 / 超过 50 个字 400，重名 409 |
| PATCH | `/lists/:id` | 改 `name` / `sortOrder`；内容没变时不刷新 `updatedAt` |
| DELETE | `/lists/:id` | 其下待办移到默认清单 + 写清单墓碑；200 `{deleted, name, movedTodos, movedTo}`；默认清单 400 |
```

表后的 todo 可写字段列表加 `listId`（"整数 ≥ 1，必须是未删除的清单，否则 400 `清单不存在`；`POST` 不带时为 1；`PATCH` 不带时不变，
缺 `listId` 的旧记录是未知、不补值"）；"服务端字段写入时忽略"一句加 `listUpdatedAt`（`listId` 自己的版本，服务端维护，
只在显式设置清单时刷新，契约 §13-2）。

### 1.4「数据导入导出与同步」相关（与 PC 那一步合并时一起改）

- 「关键文件」的 cloud 写入校验一条：`cloud/src/model.rs`（记录字段 `TODO_FIELDS` / `SUBTASK_FIELDS`，清单字段 `LIST_FIELDS`）
- 「合并规则」小节标题已写"PC `sync_store::merge_remote` 与 cloud `sync/merge.rs` 一致"，清单规则两端同一份（cloud 在 `merge::merge_lists`）
- 「当前同步覆盖范围」表：`todo_lists`（是，顶层 `todoLists`）、清单墓碑（是，顶层 `listTombstones`，不进 `tombstones`）——cloud 两者都读写
- 「维护检查清单」第 3 / 4 条：新增需要同步的表 / 字段时 cloud 还要改 `sync/doc.rs`（顶层键、`RECORD_KEYS`）、`sync/merge.rs`，
  记录字段进 `TODO_FIELDS` / `LIST_FIELDS`

## 2. `.trellis/spec/backend/sync-protocol.md`

- §2 Signatures（cloud）：`sync::doc::{KEY_TODO_LISTS, KEY_LIST_TOMBSTONES, ListTombstone, strip_record_keys}`、
  `sync::merge::apply_remote_doc`（内部 `merge_lists`）、`model::is_pristine_default_list`、
  `model::{Ownership, ownership_of, newest_ownership, apply_ownership}`（契约 §13）；cloud meta 多一个 `lists_base_reset_v1`
- §3.1 文档结构加 `todoLists` / `listTombstones`（同契约 §1），并写明"清单墓碑不进 `tombstones`"的原因
- §3.3 合并规则加一条：清单与 todo 同构（LWW 平局保留本地、`deletedAt >= updatedAt` 压制、并集、远端没有 `todoLists` 键本地不删、
  墓碑并集取较大值、坏记录跳过计数），只是**清单墓碑永不过期**（契约 §13-10）；`entityId = 1` 的清单墓碑两端都忽略；收到清单墓碑不改写待办
- §3.3 再加一条（契约 §12 + §13）：todo 的 `listId` 缺失 / `null` = 未知；归属 `(listId, listUpdatedAt)` 做字段级 LWW（顺序与采纳规则见
  第 5 节），上传时未知不写两个键。cloud 的对应实现见第 5 节
- §3.4 Self-heal 行补充：远端缺本地清单 / 版本更旧 / 缺清单墓碑同样触发重推；**还是初始值的默认清单不算**（两端都这样，见第 4 节第 1 条）；
  合并后远端那一版的归属比本地旧 → 补传一次（契约 §13-6，cloud 同样做）；旧协议文档（没有 `tombstones` 键）不比较这两项（cloud）
- §4 错误矩阵：`todoLists` / `listTombstones` 不是数组（也不是 null）→ 整份文档读不懂，不合并、不上传（cloud 与 `todos` 同一处理）
- §6 Tests Required（cloud）：`sync/merge.rs` 清单与 `listId` 单测、`sync/scenario_tests.rs` 的 7 个清单场景（并发改名 LWW、删除双向传播不复活、
  远端缺 `todoLists`、旧格式 todo 无 `listId`、默认清单墓碑忽略、412 重试、升级路径）与 3 个 §12 场景（旧写入方上传缺 `listId` 的较新记录后清单
  不变并补传一次、新写入方的 `listId` 补进 cloud 本地未知的记录且不推送、本地较新时同样采纳；都验证两轮往返后不再推送）、
  `api/integration_tests.rs` 的 `/lists`、未知 `listId` 不补值、PC 模型反序列化

## 3. `.trellis/spec/pc-gpui/data-compat.md`

- §2 读写方表 cloud 一行：写入字段契约加"清单字段见 `LIST_FIELDS`"；清单与清单墓碑 cloud 也是写入方（REST 增删改 + 删除时移动待办）
- §3 规则表"`todos` / `subtasks` 字段"一行：cloud 侧同步加 `TODO_FIELDS`（`listId` 已加）
- 旧版本 cloud（不认识清单）与新版 PC 混跑：旧 cloud 把 `todoLists` / `listTombstones` 当未知顶层键原样带回（信封），不会抹掉；
  它写出的待办没有 `listId`（未知，§12）：已知归属的设备会沿用本地值，不会被挤回默认清单；它新建的待办在各端都是未知、显示在默认清单
- 原"旧设备编辑过的待办以缺 `listId` 的形式上传 → 落进默认清单"的退化，§12 之后改为"沿用各端已知的值"，§13 之后进一步改为
  "按 `listUpdatedAt` 取各端看到的最新归属"（陈旧副本不再把旧归属带进新版本）；只有从没被新版本设备学到归属的待办才显示在默认清单
- ≤ 2.4.x 的写入方（没有 `extra`、没有 `tombstones`）上传一次就会抹掉 `todoLists` / `listTombstones` 与全部归属：不支持与多清单版本混用（契约 §13-12）

## 4. cloud 实现里需要 PC 侧知道的决定（与契约的细节差异 / 契约没写清的地方）

1. **自愈与远端 404 时不把"初始值的默认清单"算作待同步内容**：id 1 且 `updatedAt <= 2000-01-01 00:00:00`。否则每份还没有清单信息的
   远端文档都会让 cloud 多上传一次。PC core 也是这样（`sync_uploads_lists_and_only_adds_list_info_when_it_carries_something`）；
   即使不一致也只会多一次上传，不影响合并结果
2. **远端清单记录**：只要求是对象且 `id` 是 JSON 整数（字符串 id 跳过计数）；其余字段不校验、原样存（KV）。PC 强类型若因 `name` 等类型不对
   跳过某条记录，cloud 仍会保留并继续上传它（只影响畸形记录）
3. **清单墓碑 `entityId`** 接受整数或数字字符串（与 todo 墓碑同样宽松），写出一律整数；`entityId = 1` 忽略且不计入 `recordsSkipped`
4. **名称比较**：Rust `str::trim()`（Unicode 空白，含全角空格）+ `to_lowercase()`（Unicode 小写）；长度按 `chars().count()`；
   重名的错误文案用已有清单的名称（`已有同名清单「<已有清单的名称>」`）。与 PC core 的 `lists::normalize_name` / `names_conflict` /
   `conflict_error` 一致（只影响 409 / 报错，不影响同步合并）
5. **`sortOrder`**：i64（与 PC `TodoList.sort_order` 一致），REST 可改；新清单 = 现有最大值 + 1
6. 远端 `todoLists` 不是数组（且不是 null）→ cloud 整份文档报错不上传（与 `todos` 一致，PC core 也是这样）
7. **待办的 `listId` 缺失 / `null`**（§12）：未知。cloud 的有效清单过滤算默认清单；存量归一化把 `null` 统一成缺失（仅格式，不改
   `updatedAt`）；响应与上传都不带；REST 写入 `listId: null` 返回 400（写入只能把清单设成已知）
8. **合并顺序**：PC 先把墓碑应用到本地、再用远端版本自身的 `updatedAt` 判断压制；cloud 先按 LWW 胜出的版本判断压制、再清扫本地。
   两种顺序对任意 (本地, 远端, 墓碑) 组合结果相同
9. **远端清单记录的 `updatedAt` 缺失 / 无法解析**：PC 当作无法识别的记录跳过；cloud 与它处理 todo 的方式一样当作最旧的版本保存
   （本地没有时插入）。只影响畸形记录
10. **§12 规则 1 的"类型错误"**（如 `listId: "5"`）：PC 跳过整条记录；cloud 合并时照常原样存（KV，与它对待其它字段一致），有效清单算默认清单、
    不采纳也不沿用；PATCH 时存量归一化把能读出整数的改成整数、读不出的去掉（变成未知，不猜成 1）
11. **§12-8 补传在 cloud 也做**：远端某条待办缺 `listId`、cloud 本地已知 → pull 后标脏补传一次（cloud 的自愈判定本来就与 PC "需要上传"
    的判定一致）；补传写出的是本地记录原样（`updatedAt` 不变），之后远端不再缺，不会来回推送。旧协议文档（没有 `tombstones` 键）不做。
    （§13 起由第 5 节的"远端那一版的归属更旧"取代，第 7 / 10 条的 §12 细节同样以第 5 节为准）

## 5. 契约 §13（修订 2：`listId` 字段级 LWW）的落点与决定

### 5.1 各条落点（`cloud/src/`）

| 契约 | 落点 |
|---|---|
| §13-1 归属与新旧顺序（-3） | `model::Ownership`（`rank` / `newer_than`：未知 < 已知没有时间 < 已知有时间，按规范时间串比较；一样新取较大 `listId`）、`model::ownership_of`、`model::newest_ownership` |
| §13-2 写入 | `model::new_todo_record`（`listUpdatedAt = now`）；`api/todos.rs::patch_todo`（带 `listId` 且与存储的 `ListIdState` 不同 → `bump_updated_at(now, [旧 listUpdatedAt])`）；`api/lists.rs::delete_list_tx`（移进默认清单时同样刷新）；`model::TODO_IGNORED` 加 `listUpdatedAt`（客户端传入的忽略）；`model::todo_view`（未知时两个键都不带出） |
| §13-4 / -5 合并 | `sync/merge.rs::apply_remote_doc` 的 todo 循环：`local_ownerships` 在应用任何墓碑之前读出本地全部归属；结果记录存在（LWW 胜者没被墓碑压制）时 `newest_ownership(本地合并前, 远端)`，远端版本被压制也算；远端胜出 → `apply_ownership` 写进远端正文（`list_ids_carried`）；本地保留 → `rewrite_ownership` 只改 `listId` / `listUpdatedAt`，`updated_at` 列与 `updatedAt` 不动、不标脏（`list_ids_adopted`） |
| §13-6 补传 | `sync/merge.rs::remote_lacks_local_ownership`（由 `remote_lacks_local_state` 调用；本地归属比远端那一版新，或远端畸形 / 缺失而本地已知 → pull 标脏） |
| §13-7 上传 | `sync/doc.rs::build_outgoing_doc`（未知 → 两个键都去掉；已知且 `listUpdatedAt` 为 null → 去掉它） |
| §13-8 存储 | KV 的 `data_json`；存量归一化 `model::Fixer::list_updated_at`（可解析的统一成规范格式、null / 未知归属上残留的去掉，都是仅格式；数字等非字符串去掉算语义修复；无法解析的字符串不动；不补时间） |
| §13-10 清单墓碑永不过期 | 读：`merge::merge_lists` 不再丢弃早于保留期的远端清单墓碑；写 / 上传：`doc::outgoing_lists` 全部写出；清理：删掉 `repo::purge_list_tombstones_before`，`merge` 与 `push` 只清 todo / subtask 墓碑；自愈：`remote_lacks_local_lists` 不再留"快过期"的余量 |
| §13-11 畸形记录 | `model::ownership_of` 返回 `None`（`listId` 类型不对、记录 `updatedAt` 缺失或无法解析） |
| §12-1 / -4 / -5 / -6 / -9 / -10 / -11 | 不变（`model::ListIdState`、有效清单过滤、`lists_base_reset_v1`） |

### 5.2 契约没写清、cloud 自己定的（PC 核对用）

1. **结果记录存在的判定**：沿用 cloud 原有顺序——LWW 胜出的版本没被墓碑压制即存在（与 PC "先应用墓碑再合并"结果相同，第 4 节第 8 条）。
   胜出版本被压制时结果不存在，归属不处理
2. **畸形记录在记录级胜出时**（例如远端 `listId: "5"` 但 `updatedAt` 更新；PC 会整条跳过）：cloud 照旧存远端正文，但归属换成其它
   可见版本里最新的那个（本地合法的归属；都没有就是未知、去掉 `listId`），不保留畸形的 `listId`；本地已知时远端算落后、补传一次
3. **本地记录是畸形的**（只有 cloud 会有：KV 里旧版 API 写进来的 `listId: "5"`、缺 `updatedAt` 的行）：它的归属不参与；远端合法版本的归属
   会写进这条本地记录（即使记录级保留本地）。PC 不会有这种本地记录
4. **`listUpdatedAt` 无法解析**（如 `"garbage"`）：按契约算"已知但没有时间"；远端胜出时 `apply_ownership` 把它去掉（存成已知没有时间），
   本地保留时原样不动、原样上传（两端都把它当没有时间，不会来回推送）
5. **未知归属上残留的 `listUpdatedAt`**：没有意义，合并 / 上传 / 响应 / 存量归一化都去掉
6. **`PATCH` 的"值变了"**：与存储里（归一化之后）的 `ListIdState` 比较；未知 → 已知算变了；存量"已知但没有时间"原样写回同一个 `listId`
   不补时间。新时间 = `max(now, 旧 listUpdatedAt + 1 秒)`（旧值缺失 / 无法解析时就是 now）
7. **补传判定里的同时间不同 `listId`**：持有较大值的一端认为远端落后、补传一次，持有较小值的一端直接采纳——两端结论一致，一轮收敛
8. **不另加一次基准 ETag 作废**：§12 / §13 的 cloud 都没发布过，`lists_base_reset_v1` 已保证升级后第一次是全量 GET；开发期跑过 §12 版的
   cloud 即使 304 后上传了缺 `listUpdatedAt` 的记录，PC 会按 §13-6 再补传，每 10 轮一次的全量拉取也会合并回来

### 5.3 测试（`cloud/src/`）

- `model.rs`：`ownership_order_and_malformed_records`、`list_updated_at_is_repaired_without_adding_times`、`list_id_state_and_view`（两个键一起不带出）
- `sync/merge.rs`：`remote_winner_with_an_older_ownership_keeps_the_local_one`、`newer_remote_ownership_is_adopted_regardless_of_record_lww`、
  `remote_with_an_older_ownership_counts_as_behind`、`same_time_ownerships_resolve_to_the_larger_list_id`、
  `stale_ownership_never_beats_a_newer_move_in_either_order`（F1 两种顺序）、`ownership_of_a_local_version_deleted_then_rescued_is_kept`（F2）、
  `ownership_from_a_suppressed_remote_version_is_adopted`（F4）、`malformed_records_take_no_part_in_ownership`（F6）、
  `list_tombstones_take_max_and_never_expire`（F5）
- `sync/scenario_tests.rs`（mock WebDAV，都验证两轮往返不再推送）：`stale_cloud_ownership_yields_to_a_newer_move_after_its_repush`（F1 情形 1）、
  `newer_move_arriving_during_the_repush_is_adopted_on_412_retry`（F1 情形 2，If-Match）、`newer_move_overwritten_by_a_stale_repush_still_converges`
  （F1 情形 2，nginx 覆盖）、`same_time_ownerships_converge_on_the_larger_list_id`、`malformed_remote_records_never_lend_their_list_id`（F6）、
  `list_tombstones_older_than_the_retention_are_kept_and_propagated`（F5）
- `api/integration_tests.rs`：`list_updated_at_is_server_maintained`；`pushed_records_deserialize_into_pc_model` 的 PC 模型副本加 `list_updated_at`
- PC 侧需要对齐的：`Todo.list_updated_at: Option<String>`（`#[serde(default)]`，未知时不序列化）；同一组 F1 / F2 / F4 / F6 / 同时间 / 永不过期的用例
