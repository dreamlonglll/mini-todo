# 多清单：PC core ↔ cloud 数据契约（实现时以本文为准）

> 配套 `prd.md`。PC（`pc-gpui/crates/core`）与 cloud（`cloud/`）是同一份同步文档的两个写入方，下面每一条两边都必须一致。
> 有疑问时**不要各自发挥**，按本文最保守的理解实现，并在交付说明里列出。

## 1. 同步文档（`sync-data.json.gz`）新增内容

```jsonc
{
  "todos": [ { /* … */ "listId": 1 } ],          // 新字段：整数；缺失 = 1
  "todoLists": [                                  // 新顶层键：全部未删除的清单（含默认清单 id 1）
    { "id": 1, "name": "待办清单", "sortOrder": 0,
      "createdAt": "2000-01-01 00:00:00", "updatedAt": "2000-01-01 00:00:00" }
  ],
  "listTombstones": [                             // 新顶层键：清单删除墓碑
    { "entityId": 1731000000000123, "deletedAt": "YYYY-MM-DD HH:MM:SS" }
  ]
  // tombstones / settings / images / 其它顶层键：规则不变
}
```

* 时间一律规范格式 `YYYY-MM-DD HH:MM:SS`（PC `db::time`、cloud `time.rs`），解析宽松规则与 todo 相同
* 写入方每次上传都写出**全部**未删除的清单（含 id 1）与保留期内的全部清单墓碑
* **清单墓碑不进共用的 `tombstones` 数组**：v2.5.0 / 3.0.x 的 `parse_tombstone` 丢弃未知 `entityType`，上传时又按本地表重建
  `tombstones`，放进去会被旧设备抹掉。`todoLists` / `listTombstones` 作为新顶层键，旧版本经 `#[serde(flatten)] extra` 原样带回
* 远端文档**没有** `todoLists` 键（旧格式 / 还没有新版本写过）：视为"远端没有清单信息"，本地清单一条都不删（并集规则自然成立）
* 清单记录里的未知字段：cloud 原样保留（KV 存储）；PC 强类型可丢弃

## 2. 默认清单

* id 固定 `1`；初始 `name = "待办清单"`、`sortOrder = 0`、`createdAt = updatedAt = "2000-01-01 00:00:00"`（PC 在迁移 v31 建；
  cloud 在本地库没有 id 1 时按同样的值惰性补建）。固定的早时间保证两端各自建出的 id 1 不互相"抢"，任何一次改名都胜出
* **不可删除**：PC API 拒绝；cloud `DELETE` 返回 400；收到 `entityId = 1` 的清单墓碑一律忽略（不存、不传播）
* 可改名（改名照常刷新 `updatedAt`，参与 LWW）

## 3. 有效清单（读侧兜底）

`effective_list_id(todo) = todo.listId`，当且仅当存在一个**未删除**的清单 `id == listId`；否则为 `1`。

* 用于：PC 界面归属 / 计数 / 过滤；cloud 的 `listId=` / `list=` 过滤
* 存储与同步的仍是原始 `listId`（不因清单不存在而改写记录）
* 收到清单墓碑时**不**改写本地待办（靠读侧兜底显示在默认清单）

## 4. 合并规则（与 todo 完全同构）

1. 清单记录级 LWW：比较规范化后的 `updatedAt`，**平局保留本地**；无法解析的时间视为最旧
2. 墓碑压制：同 `entityId` 且 `deletedAt >= updatedAt` → 删除 / 压制该清单；删除后又被编辑（`updatedAt > deletedAt`）的保留
3. 只在一侧存在的清单保留（并集），不做"缺席即删除"
4. 清单墓碑取并集，同 `entityId` 取较大 `deletedAt`，保留 30 天（与 todo 墓碑同一个截止口径）
5. 远端清单记录反序列化失败只跳过并计数（PC `recordsSkipped`），不当作删除
6. 墓碑 `deletedAt = max(now, 被删清单的 updatedAt)`；改名等写操作的 `updatedAt = max(now, 旧值 + 1 秒)`（PC `db::time::superseding`）

## 5. 删除清单（由执行删除的一方完成，单事务）

1. 所有 `listId == X` 的待办改为 `listId = 1`，`updatedAt` 按 superseding 规则刷新（随同步传播）
2. 删除清单 X，写清单墓碑
3. `X == 1` → 拒绝（PC 返回中文错误；cloud 400）

强制同步（PC）：`ForcePull` 让本地清单 = 远端（删掉本地独有的清单，不写墓碑，待办靠读侧兜底）；`ForcePush` 为远端独有的清单写墓碑。

## 6. 名称

* 去首尾空白；空 → 错误 `清单名称不能为空`；长度上限 50 个字符（按 `char` 计）→ 超出报 `清单名称不能超过 50 个字`
* 唯一性：在未删除的清单里**不区分大小写**比较（去首尾空白后），重名 → PC 错误 `已有同名清单「<名称>」`；cloud 409
* 同步造成的重名不处理（容忍）

## 7. 新清单 id（两端同一算法，cloud 已有 `cloud/src/api/ids.rs`）

* `id = UNIX 毫秒 × 1000 + 随机 0..=999`；超出 JS 安全整数 `2^53 − 1` 时退回 `[1_000_000_000_000_000, 2^53 − 1]` 内的随机值
* 插入遇主键冲突换一个 id 重试，最多 8 次
* PC 在 core 里实现同样的算法（cloud 不依赖 core，各自一份，常量逐字相同）

## 8. 排序

* 新清单 `sortOrder = 现有最大值 + 1`；列表一律按 `(sortOrder, id)` 升序
* 第一版没有调整顺序的界面；`sortOrder` 可经 cloud `PATCH` 修改

## 9. 导出 / 导入（PC，`EXPORT_VERSION = "4.1"`）

* 导出：顶层 `todoLists`（同 §1 结构），每条 todo 带 `listId`；不导出清单墓碑、不导出窗口属性
* 导入有 `todoLists` 键（4.1）：清单按"恢复备份"语义——保留 id upsert；本地有、备份里没有的清单（id 1 除外）写清单墓碑并删除；
  默认清单取备份里的名字；被导入清单的 `updatedAt` 与 todo 一样刷新为当前时间（保证恢复结果在同步中胜出）
* 导入没有 `todoLists` 键（3.0 / 4.0）：**现有清单不动**；todo 缺 `listId` → 1
* 导入前照常 `VACUUM INTO` 备份

## 10. cloud REST

* `TODO_FIELDS` 加 `listId`（整数 ≥ 1，且必须是未删除的清单，否则 400 `清单不存在`）；`POST /todos` 不带时为 1
* `GET /todos`：新增 `listId=<n>`（按有效清单）与 `list=<名称>`（去首尾空白、不区分大小写精确匹配；重名 409；不存在 404）
* `GET /lists` → `[{ id, name, sortOrder, createdAt, updatedAt }]`，按 `(sortOrder, id)`
* `GET /lists/:id`、`POST /lists`（必填 `name`，可选 `sortOrder`）→ 201、`PATCH /lists/:id`（`name` / `sortOrder`）、`DELETE /lists/:id`（§5）
* `:id` 接受数字 id 或 URL 编码的清单名（按 §6 的比较规则）；名称重名 409；不存在 404
* 清单不分配 `C{seq}` 短码；未知字段 / 类型错误 → 400（与 todo 一致）
* 写操作照常标 dirty、走去抖推送

## 11. PC 设备本地窗口属性（仅 PC；不同步、不导出、不接 `local_seq`）

窗口 key：清单窗口 `list:<清单 id>`、日历窗口 `calendar`。

* 每窗口：是否隐藏、深色、视图模式（`list` / `quadrant`，仅清单）、展开全部子任务（仅清单）、日历清单筛选（仅日历：`null` = 全部，或清单 id 数组）
* 每 (窗口 key, 屏幕组合 ID)：客户区物理像素 `x / y / width / height`、是否锁定（`is_fixed`）
* 迁移 v31 种子数据：
  * `list:1` ← 设置 `text_theme`（`"light"` 表示深色，语义反转，见 `AppState.is_dark_theme`）、`view_mode`；
    各屏幕组合的布局 ← `screen_configs` 的位置 / 尺寸 / `is_fixed`
  * `calendar` ← 隐藏 = `!show_calendar`、深色同 `list:1`；`show_calendar = true` 时把每条 `screen_configs` 的矩形按
    左 40%（取整）/ 右 60% 切开：左给 `list:1`，右给 `calendar`，两者 `is_fixed` 相同；`show_calendar = false` 时 `calendar` 没有布局
    （第一次显示时由界面决定位置）
  * 展开全部子任务在 `ui-state.json`（app 层），由 app 在首次启动时迁移，不在 SQL 迁移里
* 删除屏幕配置时连带删除该组合下所有窗口布局（与 `fence_layouts` 同）

## 12. 修订 1：`listId` 缺失 = 未知（防跨版本分叉，**取代** §1 / §3 里"缺失 = 1"的说法）

**问题**：旧版本（≤ 3.0.x、v2.5.0）每次上传都按本地库重新序列化**全部**待办，全部丢掉 `listId`。一台先跑旧版本、后升级的设备，
迁移后本地全是"默认清单"且 `updatedAt` 与远端相同 → 平局保留本地 → 永远学不到真实归属，之后上传又把文档里的 `listId` 全改成 1，
与其它设备分叉。原先"旧设备改过的待办回到默认清单"的退化也是同一个根源。

**规则**（PC 与 cloud 一致）：

1. todo 记录里 `listId` **缺失或为 `null` = 未知**（写入方不认识清单），**不等于** 1；类型错误（如字符串）仍按"无法识别的记录"跳过
2. 合并时远端记录 LWW **胜出**但没有 `listId`：沿用本地记录的 `listId`（本地已知 → 带过去；本地也未知 / 本地没有这条 → 未知）
3. 合并时远端记录**有** `listId`、本地这条是未知：**无论 LWW 谁胜（含平局、本地更新）都采纳远端的 `listId`**，不改 `updatedAt`；
   PC 上这次采纳不应触发一次额外上传（远端本来就有）
4. 有效清单（§3）：未知 → 1（只影响显示 / 过滤，不写回）
5. 上传 / 导出时未知 → **不写 `listId` 键**（与旧写入方相同，未知永远不会覆盖别人的已知值）；已知（含 1）→ 照常写
6. 只有"显式设置清单"的写操作让未知变已知：新建（总是已知）、移动到清单、编辑时改了清单、cloud `POST` / `PATCH` 带了 `listId`。
   只改其它字段时保持未知（PC `UpdateTodoRequest.list_id = None`、cloud `PATCH` 不带 `listId` → 不变）
7. PC 存储：`todos.list_id` 可为 NULL，NULL = 未知；迁移 v31 把**存量**待办设为 NULL（不是 1）；新建一律写具体值
8. PC 补传：远端某条缺 `listId`、本机同一条已知（任何值）→ 补传一次，修复文档（替代原先"本机是非默认值才补传"的条件）
9. 导入 3.0 / 4.0 备份（显式恢复）：`listId` 缺失 → **具体的 1**（§9 不变：导入旧备份 = 全部待办进默认清单，并随同步传播）
10. 升级后第一次同步必须是无条件 GET（PC 迁移 v31 时作废基准 ETag；cloud 已有 `lists_base_reset_v1`），让规则 3 尽快生效
11. cloud：本地 KV 里已有的、缺 `listId` 的记录就是"未知"；cloud 不替它补值；响应里未知 → 不带 `listId`（或按现状）
