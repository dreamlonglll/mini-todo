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
  REST 写入的 `listId` 必须指向未删除的清单；`DELETE /lists/:id` 单事务把其下待办移到默认清单（刷新 `updatedAt`）并写清单墓碑；
  `GET /todos` 的 `listId=` / `list=` 按有效清单过滤（`listId` 缺失 / 指向已删除清单的待办算默认清单，记录不改写）；清单不分配短码
- **待办的 `listId` 缺失 = 未知**（契约 §12）：缺失或 `null` 表示写入方不认识清单（旧版本每次上传都丢掉全部 `listId`），只在显示 / 过滤时
  算默认清单，cloud 从不替它补值、上传时不写 `listId` 键、响应里也不带；合并时远端胜出但缺 `listId` → 沿用本地已知值，远端已知、本地未知 →
  不论 LWW 谁胜都采纳（不改 `updatedAt`、不标脏），远端缺本地已知的 `listId` → 补传一次；只有 `POST`（总是已知）与带 `listId` 的 `PATCH`
  让它变成已知
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
缺 `listId` 的旧记录是未知、不补值"）。

### 1.4「数据导入导出与同步」相关（与 PC 那一步合并时一起改）

- 「关键文件」的 cloud 写入校验一条：`cloud/src/model.rs`（记录字段 `TODO_FIELDS` / `SUBTASK_FIELDS`，清单字段 `LIST_FIELDS`）
- 「合并规则」小节标题已写"PC `sync_store::merge_remote` 与 cloud `sync/merge.rs` 一致"，清单规则两端同一份（cloud 在 `merge::merge_lists`）
- 「当前同步覆盖范围」表：`todo_lists`（是，顶层 `todoLists`）、清单墓碑（是，顶层 `listTombstones`，不进 `tombstones`）——cloud 两者都读写
- 「维护检查清单」第 3 / 4 条：新增需要同步的表 / 字段时 cloud 还要改 `sync/doc.rs`（顶层键、`RECORD_KEYS`）、`sync/merge.rs`，
  记录字段进 `TODO_FIELDS` / `LIST_FIELDS`

## 2. `.trellis/spec/backend/sync-protocol.md`

- §2 Signatures（cloud）：`sync::doc::{KEY_TODO_LISTS, KEY_LIST_TOMBSTONES, ListTombstone, strip_record_keys}`、
  `sync::merge::apply_remote_doc`（内部 `merge_lists`）、`model::is_pristine_default_list`；cloud meta 多一个 `lists_base_reset_v1`
- §3.1 文档结构加 `todoLists` / `listTombstones`（同契约 §1），并写明"清单墓碑不进 `tombstones`"的原因
- §3.3 合并规则加一条：清单与 todo 同构（LWW 平局保留本地、`deletedAt >= updatedAt` 压制、并集、远端没有 `todoLists` 键本地不删、
  墓碑并集取较大值 30 天、坏记录跳过计数）；`entityId = 1` 的清单墓碑两端都忽略；收到清单墓碑不改写待办
- §3.3 再加一条（契约 §12）：todo 的 `listId` 缺失 / `null` = 未知；远端胜出但未知 → 沿用本地已知值；远端已知、本地未知 → 不论谁胜都采纳
  （不改 `updatedAt`）；未知永远不覆盖已知，上传时未知不写键。cloud 的对应实现：`sync/merge.rs` 的 todo 循环（`list_ids_carried` /
  `list_ids_adopted`）、`model::ListIdState`、`repo::{todo_list_id_states, set_todo_list_id}`、`doc::build_outgoing_doc` 去掉 `listId: null`
- §3.4 Self-heal 行补充：远端缺本地清单 / 版本更旧 / 缺清单墓碑同样触发重推；**还是初始值的默认清单不算**（两端都这样，见第 4 节第 1 条）；
  远端某条待办缺 `listId`、本地已知 → 补传一次（契约 §12-8，cloud 同样做）；旧协议文档（没有 `tombstones` 键）不比较这两项（cloud）
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
- 原"旧设备编辑过的待办以缺 `listId` 的形式上传 → 落进默认清单"的退化，§12 之后改为"沿用各端已知的值"；只有从没被新版本设备
  学到归属的待办才显示在默认清单

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
    的判定一致）；补传写出的是本地记录原样（`updatedAt` 不变），之后远端不再缺，不会来回推送。旧协议文档（没有 `tombstones` 键）不做
