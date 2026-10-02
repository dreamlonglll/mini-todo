---
name: minitodo
description: 通过云端 HTTP API 读写用户的 mini-todo 待办；适合 today / add / done / list / search / show / update / delete 场景，可被 openclaw cron 调度（agent 自行拉数据 + 判断是否推送）做临期提醒。
version: 2.1.0
---

# mini-todo Skill

让 Claude Code 通过云端 HTTP API 操作用户的 mini-todo 待办数据。底层数据通过
WebDAV 与 PC 端共享同一份 `sync-data.json.gz`：AI 的写入约 1.5 秒后（连续写入最多
攒 10 秒）推送到 WebDAV，PC 端下次同步即可见。

## 何时调用

用户说出下面意图时，先 `Bash` 调用本 skill 的 CLI：

- "今天有什么待办？" → `python ~/.claude/skills/minitodo/minitodo.py today --json`
- "未来 24h 有什么临期？" / 被 cron 唤起做提醒 → `list --pending --json` 后**你自己**按时间判断该推哪些
- "帮我加一个'买菜'到待办" → `add`
- "把 C3 / ID xxx 标记完成" → `done C3`（短码）或 `done <i64-id>`
- "看看所有未完成的高优先级" → `list --pending --priority high`
- "搜一下含'报告'的待办" → `search`
- "C3 / ID xxx 是什么？" → `show C3`
- "把 C3 / ID xxx 的截止时间改到 2026-05-20" → `update C3 endTime=2026-05-20`
- "删除 C3 / ID xxx" → `delete C3`

> **`C{seq}` 短码**是 cloud 给每个 todo 分配的递增编号（响应 JSON 里 `seq` 字段），
> 比 16 位完整 i64 id 更适合用户口语反馈；删除后短码不会被复用。所有 `done` / `show` /
> `update` / `delete` 的 id 位置都同时接受 `C{seq}` / `c{seq}`（大小写不敏感）和完整 i64 id。

非待办相关问题不要调用本 skill；它只覆盖 mini-todo 这一个领域。

## 安装

默认安装到 Claude Code 的 skill 目录；通过 `--target` 切到 openclaw 或两者都装。

```bash
bash install.sh                   # ~/.claude/skills/minitodo/（默认）
bash install.sh --target openclaw # ~/.openclaw/workspace/skills/minitodo/
bash install.sh --target both     # 两者都装
```

Windows 用户用 PowerShell 执行 `.\install.ps1`（参数相同：`-Target claude/openclaw/both`），
或在 Git Bash / WSL / MSYS2 里执行上面的 bash 命令。脚本做三件事：

1. 把 `SKILL.md` / `minitodo.py` / `config.example.toml` 复制到目标 skill 目录
2. 如果 `config.toml` 不存在，把 `config.example.toml` 复制为 `config.toml`，并把权限收紧到
   只有当前用户可读写（Linux/macOS `chmod 600`，Windows 去掉继承的访问权限、只留当前用户）
3. 提示编辑 `config.toml` 填入 `endpoint` + `api_key`（可选 `timezone`，见下）

完成后必须确认本机有 Python 3 + `requests`：

```bash
python --version             # 3.10+ 推荐；3.10 需要 pip install tomli
pip install requests         # 必装
pip install tzdata           # 仅 Windows 且在 config.toml 里配置了 timezone 时需要
```

`config.toml` 字段：`endpoint`、`api_key`（必填）；`timeout`（秒，默认 10；`sync` 至少 90）；
`timezone`（IANA 时区名，例如 `Asia/Shanghai`，`today` 用它计算"今天"；不填用本机时区，
建议与云端 / PC 一致）。

## CLI 参考

### 全局 flag

- `--config <path>` 自定义 config.toml 路径
- `--json` 输出原始 JSON（AI 解析时务必使用）

退出码：`0` 成功（包括 `health` 报告同步降级）；`2` 配置 / 参数错误；`3` 网络错误（GET 已自动
重试 2 次）；`4` 服务端返回错误（stderr 是 `HTTP <状态码>: <detail>`）。

### 子命令

| 子命令 | 说明 | 示例 |
|---|---|---|
| `today` | 今日相关（今天到期 ∪ 今天开始 ∪ 已逾期未完成）；"今天"按 config 的 `timezone` | `python minitodo.py today --json` |
| `add <title>` | 新增待办；`--due` 写入 `endTime` | `python minitodo.py add "买菜" --priority high --due 2026-05-20` |
| `done <ref>` | 标记完成 | `python minitodo.py done C3` 或 `done 1734567890123456` |
| `list` | 列表，可叠加 `--pending` / `--completed` / `--priority` / `--quadrant` / `--limit` / `--sort` | `python minitodo.py list --pending --priority high --json` |
| `search <kw>` | 关键词搜索（标题 + 描述） | `python minitodo.py search "报告" --json` |
| `show <ref>` | 详情（默认含 subtasks） | `python minitodo.py show C3 --json` |
| `update <ref> <key=val>...` | 修改字段，支持多个 | `python minitodo.py update C3 endTime="2026-06-01 18:00" priority=medium` |
| `delete <ref>` | 删除（连带 subtasks） | `python minitodo.py delete C3` |
| `health` | 健康检查（同步降级时照常输出 JSON，stderr 附一行提示） | `python minitodo.py health --json` |
| `sync` | 手动触发 WebDAV 同步（pull + push） | `python minitodo.py sync --json` |
| `sync pull` | 仅从 WebDAV 拉取最新数据 | `python minitodo.py sync pull --json` |
| `sync push` | 仅推送本地变更到 WebDAV | `python minitodo.py sync push --json` |

`<ref>` = `C{seq}` 短码（cloud 端 1, 2, 3... 自增）或完整 i64 id。`C` 大小写不敏感。

- `--due`：规范格式 `YYYY-MM-DD HH:MM:SS`；也可写 `YYYY-MM-DD`（= 当天 `23:59:00`）或
  `YYYY-MM-DD HH:MM`；带 `Z` / `±HH:MM` 时区后缀的值由服务端换算成它配置的时区。
- `--priority high|medium|low`：服务端映射成 PC 的颜色（见下方字段表）；同时给 `--color` 时以颜色为准。
- `update` 的 value 自动尝试解析为 `true` / `false` / `null` / 数字 / JSON，剩下当字符串；
  `quadrant=` 接受别名（`urgent_important` 等）；字段名必须是下方字段表里的（或别名），
  否则服务端返回 400 并列出允许的字段。

## 常见任务 → 命令映射

| 用户问 | 一行命令 |
|---|---|
| 今天有什么 | `python ~/.claude/skills/minitodo/minitodo.py today --json` |
| 还有几个高优先级没完成 | `python ~/.claude/skills/minitodo/minitodo.py list --pending --priority high --json` |
| 新增"周五开会准备 PPT" | `python ~/.claude/skills/minitodo/minitodo.py add "周五开会准备 PPT" --priority medium --json` |
| 完成 C3 | `python ~/.claude/skills/minitodo/minitodo.py done C3 --json` |
| 把 C3 的截止时间改到下周一 18 点 | `python ~/.claude/skills/minitodo/minitodo.py update C3 endTime="2026-05-25 18:00" --json` |
| 清空 C3 的提醒时间 | `python ~/.claude/skills/minitodo/minitodo.py update C3 notifyAt=null --json` |
| 删掉一周前完成的所有 | 先 `list --completed --sort -updatedAt --json`，再分别 `delete C{seq}` |
| 确保数据是最新的 | `python ~/.claude/skills/minitodo/minitodo.py sync --json` |

## 直接调用 HTTP API（fallback）

CLI 不可用时，AI 可直接用 curl。所有请求都要 `Authorization: Bearer <api_key>`；写请求
body 是 JSON（`Content-Type: application/json`）。

| Method | Path | 用途 |
|---|---|---|
| `GET /health` | 服务健康与同步状态（降级时 **503**，见下文） |
| `GET /todos?...` | 列表；query: `completed=true/false`, `priority=high/medium/low`（按颜色派生）, `quadrant=1-4` 或 `urgent_important` 等别名, `dueDateBefore` / `dueDateAfter`（按 `endTime`；见下）, `startDate=YYYY-MM-DD`, `q=<keyword>`（标题 + 描述）, `sort=[+-]<field>`, `limit`, `offset`, `withSubtasks=true` |
| `GET /todos/:id?withSubtasks=true` | 详情（默认嵌套 subtasks） |
| `POST /todos` | 创建；body 必填 `title`，其它字段见字段表 |
| `PATCH /todos/:id` | 部分更新；未提及字段保留；内容没变化时不刷新 `updatedAt` |
| `DELETE /todos/:id` | 删除（连带 subtasks） |
| `POST /todos/:id/subtasks` | 创建子任务；body 必填 `title` |
| `PATCH /subtasks/:id` | 部分更新子任务 |
| `DELETE /subtasks/:id` | 删除子任务 |
| `GET /images/:name` | 取图片 bytes |
| `POST /images` | multipart 上传，`file` 字段；返回 `{name}` |
| `POST /sync` | 手动触发 pull + push；全部成功 200，部分失败 207；返回 `{pull, push, pullError?, pushError?}` |
| `POST /sync/pull` | 仅 pull；返回 `{status, changed, repushScheduled, todosUpserted, ...}` |
| `POST /sync/push` | 仅 push；返回 `{status, pushed, attempts, dirtyCleared, ...}` |

排序字段白名单：`dueDate`（= `endTime`）/ `startTime` / `priority`（high > medium > low >
自定义颜色）/ `quadrant` / `sortOrder` / `updatedAt` / `createdAt` / `title`。前缀 `-` 倒序、
`+` 或省略正序；不在白名单的字段按 `sortOrder` 排序。

`dueDateBefore` / `dueDateAfter` / `startDate` 接受与写入相同的时间格式；仅日期时
`dueDateBefore=D` 表示"D 当天 23:59:59 及以前"（含当天），`dueDateAfter=D` 表示"D 当天
00:00:00 及以后"。格式不对 → 400。

### 时间格式

所有时间字段（`notifyAt` / `startTime` / `endTime` / `createdAt` / `updatedAt`）都是
**`YYYY-MM-DD HH:MM:SS`**：服务端（与 PC）配置时区的墙钟时间，无时区后缀，例如
`"2026-05-20 18:30:00"`。写入时也接受下面这些形式，服务端统一换成规范格式再保存：

| 输入 | 保存为 |
|---|---|
| `2026-05-20 18:30` / `2026-05-20T18:30` / `2026-05-20T18:30:00.123` | `2026-05-20 18:30:00` |
| `2026-05-20T10:30:00Z` / `2026-05-20T18:30:00+08:00` | 换算成服务端时区的墙钟 |
| `2026-05-20`（仅日期） | `startTime` → `00:00:00`；`endTime` / `dueDate` → `23:59:00`；`notifyAt` → `09:00:00` |
| `null` 或 `""` | 清空该字段 |

### todo 字段（写入契约，与 PC 端模型一致）

| 字段 | 类型 | 说明 |
|---|---|---|
| `title` | 非空字符串 | 任务名称（创建时必填） |
| `description` | 字符串 / null | 备注（Markdown）。别名 `notes`（只在没给 `description` 时生效） |
| `color` | `"#RRGGBB"` | 颜色，PC 用它表示优先级：`#EF4444` 高、`#F59E0B` 中、`#10B981` 低（新建默认 `#10B981`）；其它颜色是用户自定义色 |
| `priority` | `"high"` / `"medium"` / `"low"` | **写入时是 `color` 的别名**（high→#EF4444、medium→#F59E0B、low→#10B981；同时给了 `color` 以 `color` 为准；写 `null` 被忽略）。**读取时是派生字段**：由 `color` 映射，自定义颜色为 `null`；不入库 |
| `quadrant` | 1-4 | 四象限：1 重要紧急、2 重要不紧急、3 紧急不重要、4 不紧急不重要（默认）。也接受别名 `urgent_important` / `important_not_urgent` / `urgent_not_important` / `not_urgent_not_important`，保存为整数 |
| `completed` | bool | 完成态 |
| `startTime` | 时间 / null | 开始时间 |
| `endTime` | 时间 / null | 截止时间。别名 `dueDate`（旧版字段名，只在没给 `endTime` 时生效） |
| `notifyAt` | 时间 / null | 下一次提醒时间。**重复任务每次触发后只更新这个字段**，`endTime` 不变。改了它而没显式给 `notified` 时，`notified` 自动重置为 `false`（与 PC 一致） |
| `notifyBefore` | 整数 ≥ 0 | 提前多少分钟提醒 |
| `notified` | bool | 本次提醒是否已触发 |
| `sortOrder` | 整数 | 列表排序 |
| `repeatEnabled` | bool | 是否开启重复（开启且没给 `notifyBefore` 时，`notifyBefore` 置 0，与 PC 一致） |
| `repeatType` | `"daily"` / `"weekly"` / `"monthly"` / null | 重复类型 |
| `repeatInterval` | 整数 ≥ 1 | 间隔；`weekly` + `interval=2` = 每两周 |
| `repeatWeekdays` | `"1,3,5"` / null | 仅 `weekly`：星期几（1=周一 … 7=周日）；也接受数组 `[1,3,5]` |
| `repeatMonthDay` | 1-31 / null | 仅 `monthly`：每月几号 |

只读 / 服务端维护的字段：`id`、`createdAt`、`updatedAt`、`seq`（cloud 短码）、`subtasks`、
`subtaskCount`（列表不嵌套时附带）。写入时这些字段会被**忽略**，所以把读到的对象改几个字段
后整包写回是安全的；对象里原样带回的 `color` / `endTime` / `description` 不会盖掉你对
`priority` / `dueDate` / `notes` 的修改（规范字段只有真的改了才优先于别名）。

**其它任何字段一律 400**（PC 端是强类型模型，不认识的字段会被静默丢掉）；类型不对同样 400。
错误体是 JSON：

```json
{
  "error": "bad_request",
  "detail": "unknown field(s): tags; allowed: title, description, color, ...; quadrant: must be 1-4 or one of urgent_important, ...",
  "unknownFields": ["tags"],
  "invalidFields": {"quadrant": "must be 1-4 or one of ..."},
  "allowedFields": ["title", "description", "color", "quadrant", "..."]
}
```

其它错误码：`unauthorized`（401）、`not_found`（404）、`unsupported_media_type`（415，
缺 `Content-Type: application/json`）、`internal`（500，详情只在服务端日志）、`sync_failed`
（500，`/sync/pull` / `/sync/push` 失败；detail 给出 WebDAV 层面的原因）。

### subtask 字段

| 字段 | 类型 | 说明 |
|---|---|---|
| `title` | 非空字符串 | 创建时必填 |
| `content` | 字符串 / null | Markdown 内容 |
| `completed` | bool | 完成态 |
| `sortOrder` | 整数 | 排序 |
| `parentId` | 只读 | 所属 todo 的 id。**不能修改**（要移动就删掉再在另一个 todo 下新建）；写入时给了不同的值 → 400，相同的值忽略 |

`id` / `createdAt` / `updatedAt` 写入时忽略，其它字段 400。

### 图片

描述 / 子任务内容里的图片引用有两种形式，都用 `GET /images/<name>` 取图（需要 Bearer）：

- 规范形式：`![](minitodo-image://<name>)` → 文件名就是 `<name>`
- 旧数据：`http://asset.localhost/<URL 编码的本机绝对路径>` 或 `asset://localhost/<...>`
  （PC 端本机路径）→ 先 URL 解码，再按 `/` 或 `\` 切分取**最后一段**作为文件名，例如
  `http://asset.localhost/C%3A%5CUsers%5Cme%5CAppData%5CLocal%5Cmini-todo%5Cimages%5C1715000000000_ab12cd.png`
  → `1715000000000_ab12cd.png`

文件名只含字母、数字、`.`、`_`、`-`（以字母或数字开头、不含 `..`、最长 128 字符），其它名字返回 400。
上传：`POST /images`（multipart `file` 字段）返回 `{name}`，在 Markdown 里写
`![](minitodo-image://<name>)` 引用。

### 健康状态 `/health`

```json
{
  "status": "healthy",
  "sync": "healthy",
  "pull": "healthy",
  "push": "healthy",
  "lastPullAt": "2026-05-13 12:34:56",
  "lastPullError": null,
  "lastPushOkAt": "2026-05-13 12:30:01",
  "lastPushError": null,
  "dirty": false,
  "dirtySince": null,
  "imageQueueLength": 0
}
```

- `sync` = `pull` 与 `push` 中较差者：`healthy` | `stale` | `offline`。`pull` 看最近一次成功拉取距今；
  `push` 看本地待推送的写入（`dirty` / 图片队列）积压了多久
- 不是 healthy 时 `status = "degraded"`，HTTP 状态码 **503**，`lastPullError` / `lastPushError` /
  `dirtySince` / `imageQueueLength` 说明原因。**降级只是信息**：API 仍可读写，写入会在
  WebDAV 恢复后自动推送；读到的数据可能不是 PC 端的最新状态。CLI 的 `health` 照常输出 JSON、
  退出码 0，并在 stderr 打一行提示
- 每个鉴权通过的响应都带 `X-Sync-Status: healthy | stale | offline` 与
  `X-Last-Sync-At: <最近一次成功拉取>`；offline 时还带 `Warning: 110 - "sync offline"`

curl 示例：

```bash
# 列表 + 排序
curl -H "Authorization: Bearer $KEY" \
  "https://minitodo.example.com/todos?completed=false&priority=high&sort=-dueDate"

# 新增（priority / dueDate 是别名，服务端存成 color / endTime）
curl -H "Authorization: Bearer $KEY" -H "Content-Type: application/json" \
  -d '{"title":"买菜","priority":"high","endTime":"2026-05-20 18:00:00"}' \
  https://minitodo.example.com/todos

# 部分更新（path 支持 C{seq} 短码或完整 i64 id）
curl -X PATCH -H "Authorization: Bearer $KEY" -H "Content-Type: application/json" \
  -d '{"completed":true}' \
  https://minitodo.example.com/todos/C3

# 上传图片
curl -H "Authorization: Bearer $KEY" \
  -F "file=@/tmp/screenshot.png" \
  https://minitodo.example.com/images
```

**重复任务的人话描述**（用于 channel 推送）：

| 字段组合 | 描述 |
|---|---|
| `repeatType=daily, interval=1` | 每天 |
| `repeatType=daily, interval=N` | 每 N 天 |
| `repeatType=weekly, interval=1, weekdays="1,3,5"` | 每周的周一、周三、周五 |
| `repeatType=weekly, interval=2` | 每 2 周 |
| `repeatType=monthly, interval=1, monthDay=14` | 每月 14 号 |
| `repeatType=monthly, interval=3, monthDay=1` | 每 3 月 1 号 |

> 当 `repeatEnabled=true` 且 `notifyAt` 非空时，直接用 `notifyAt` 作为时间锚。
> 若 `notifyAt` 为空，则根据 `repeatType` 推算本期应触发时间（见 cron 流程步骤 2b）。
> 非重复任务按 `endTime` → `notifyAt` 顺序取第一个非空值。

## 在 openclaw 中启用临期提醒

整体范式：cron 定时唤起一个 isolated session，session prompt 让你 **agent 自己**
拉所有未完成 todo、根据用户偏好的临期窗口判断哪些该推、组织好格式后通过
`--announce` 推到 default channel（session history 中最近用过的 channel）。

**为什么不让 skill 内置一个 due-soon 子命令？** mini-todo 的重复提醒只更新
`notifyAt`、不动 `endTime`；而服务端 query 只能按 `endTime` 筛——客户端硬
编码规则永远会有漏推或误推。让 agent 看到原始 JSON、自己判断，更稳。

详细的安装 / 配置 / cron message 模板见仓库根 `cloud/openclaw.md`。

最简核心步骤（具体 prompt 见 `cloud/openclaw.md`）：

```bash
# 1. 装 skill 到 openclaw workspace
bash install.sh --target openclaw

# 2. 编辑 ~/.openclaw/workspace/skills/minitodo/config.toml 填 endpoint + api_key（建议填 timezone）

# 3. 注册 cron（msg 里要求 agent 自己拉 list --pending --json + 判断）
openclaw cron add \
  --name minitodo-due-soon \
  --cron "0 8 * * *" \
  --tz "Asia/Shanghai" \
  --session isolated \
  --message "<见 cloud/openclaw.md §7>" \
  --announce
```

### agent 在 cron session 里要做的事

你被 cron 唤起做 mini-todo 临期提醒。严格按下面流程，不要做任何步骤外的事：

1. **获取数据**：

   ```bash
   python ~/.openclaw/workspace/skills/minitodo/minitodo.py --json list --pending
   ```

2. **对每条 todo，按以下优先级取"时间锚"**（所有时间都是 `YYYY-MM-DD HH:MM:SS`，服务端时区的墙钟）：

   a. 若 `repeatEnabled == true` 且 `notifyAt` 非空 → 用 `notifyAt`

   b. 若 `repeatEnabled == true` 且 `notifyAt` 为空 → 根据 `repeatType` 推算本次应触发时间：
      - **monthly**：取当月 `repeatMonthDay` 日的 00:00。若该日已过 → 算逾期。
        逾期天数 = 当月当前日号 − repeatMonthDay。
      - **weekly**：如果 `repeatWeekdays` 包含今天 → 时间锚 = 今天 00:00。
        若不包含今天，找 `repeatWeekdays` 中下一个最近的星期几，算出天数差。
      - **daily**：时间锚 = 今天 00:00。

   c. 否则按 `endTime` → `notifyAt` 顺序取第一个非空值

   d. 全空则**跳过**该 todo（纯备忘类不推送）

3. **判断临期**——用当前墙钟时间 `now`（Asia/Shanghai）和窗口 **H = 24 小时**
   （与 `openclaw.md` 的默认窗口一致；用户在 cron message 里另有指定时以其为准）：

   - 时间锚 < `now`                → "已逾期"（显示逾期多久）
   - `now` ≤ 时间锚 ≤ `now + 24h`  → "未来 24h 到期"（显示还有多久）
   - 时间锚 > `now + 24h`          → 跳过

4. **仅当 `repeatEnabled == true` 时**，把 `repeat*` 字段翻译成中文：

   | 条件 | 描述 |
   |---|---|
   | `monthly, interval=1, monthDay=D` | 每月 D 号 |
   | `weekly, interval=1, weekdays="1,3,5"` | 每周的周一、周三、周五 |
   | `daily, interval=1` | 每天 |
   | `interval=N`（N > 1） | 每 N 天 / 周 / 月 |

   weekdays 映射：1=周一、2=周二、3=周三、4=周四、5=周五、6=周六、7=周日。

5. **严格按下面格式输出**，不要加任何解释 / 翻译 / 总结：

   ```
   mini-todo 临期提醒｜YYYY-MM-DD HH:MM
   已逾期（N）：
   - #C{seq} [优先级] 标题 (MM-DD HH:MM，已逾期 X 小时｜重复描述)
   未来 24h 到期（M）：
   - #C{seq} [优先级] 标题 (MM-DD HH:MM，X 小时后｜重复描述)
   ```

   - 优先级映射：`priority` 字段 high → 高、medium → 中、low → 低；`priority` 为 null（自定义颜色）就省略 `[...]`
   - **前缀优先用 `#C{seq}`**（短码，用户回复"完成 C3"更顺手）；若 JSON 缺 `seq`
     字段则降级用 `#{id}` 完整 i64 id
   - 非重复任务省略 `｜重复描述`
   - 时间差显示规则：< 60 分钟用 "X 分钟后"，< 24h 用 "X 小时后"，≥ 24h 用 "X 天后"；逾期同理
   - 示例：`- #C3 [中] 月度回顾 (05-14 09:00，已逾期 3 小时｜每月 14 号)`

6. 如果两组都为空，只输出一行 `24h 内无临期事项`，**不要再加任何字**。

## 已知限制

- 单用户、单 API key（云端 `config.toml` 配）
- 写入约 1.5 秒后推送到 WebDAV（连续写入最多攒 10 秒），PC 端要在自己的同步周期内才能拉到
- 删除会留下墓碑并双向同步，30 天内能拦截其它端把已删除记录"复活"；离线超过 30 天的设备
  再上线时，它手里的旧记录仍可能被恢复
- 同一条记录两端同时修改时按 `updatedAt` 整条取较新的一方（不做字段级合并）；写后立刻 `show` 校验最稳
- 不支持移动端

## 安装目录约定

最终运行时结构：

```
~/.claude/skills/minitodo/
├── SKILL.md
├── minitodo.py
├── config.example.toml
└── config.toml          # 你自己填，仅当前用户可读，gitignore 掉
```
