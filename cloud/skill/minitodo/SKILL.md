---
name: minitodo
description: 通过云端 HTTP API 读写用户的 mini-todo 待办；适合 today / add / done / list / search / show / update / delete 场景，可被 openclaw cron 调度（agent 自行拉数据 + 判断是否推送）做临期提醒。
version: 2.0.0
---

# mini-todo Skill

让 Claude Code 通过云端 HTTP API 操作用户的 mini-todo 待办数据。底层数据通过
WebDAV 与 PC 端共享同一份 `sync-data.json.gz`，AI 写入 1 秒内会回写 WebDAV，PC
端下次同步即可见。

## 何时调用

用户说出下面意图时，先 `Bash` 调用本 skill 的 CLI：

- "今天有什么待办？" → `python ~/.claude/skills/minitodo/minitodo.py today --json`
- "未来 24h 有什么临期？" / 被 cron 唤起做提醒 → `list --pending --json` 后**你自己**按时间判断该推哪些
- "帮我加一个'买菜'到待办" → `add`
- "把 C3 / ID xxx 标记完成" → `done C3`（短码）或 `done <i64-id>`
- "看看所有未完成的高优先级" → `list --pending --priority high`
- "搜一下含'报告'的待办" → `search`
- "C3 / ID xxx 是什么？" → `show C3`
- "把 C3 / ID xxx 的截止日期改到 2026-05-20" → `update C3 dueDate=2026-05-20`
- "删除 C3 / ID xxx" → `delete C3`

> **`C{seq}` 短码**是 cloud 给每个 todo 分配的递增编号（响应 JSON 里 `seq` 字段），
> 比 16 位完整 i64 id 更适合用户口语反馈。所有 `done` / `show` / `update` / `delete`
> 的 id 位置都同时接受 `C{seq}` / `c{seq}`（大小写不敏感）和完整 i64 id。

非待办相关问题不要调用本 skill；它只覆盖 mini-todo 这一个领域。

## 安装

默认安装到 Claude Code 的 skill 目录；通过 `--target` 切到 openclaw 或两者都装。

```bash
bash install.sh                   # ~/.claude/skills/minitodo/（默认）
bash install.sh --target openclaw # ~/.openclaw/workspace/skills/minitodo/
bash install.sh --target both     # 两者都装
```

Windows 用户在 Git Bash / WSL / MSYS2 里执行同样的命令即可。脚本做三件事：

1. 把 `SKILL.md` / `minitodo.py` / `config.example.toml` 复制到目标 skill 目录
2. 如果 `config.toml` 不存在，把 `config.example.toml` 复制为 `config.toml`
3. 提示编辑 `config.toml` 填入 `endpoint` + `api_key`

完成后必须确认本机有 Python 3 + `requests`：

```bash
python --version             # 3.10+ 推荐；3.10 需要 pip install tomli
pip install requests         # 必装
```

## CLI 参考

### 全局 flag

- `--config <path>` 自定义 config.toml 路径
- `--json` 输出原始 JSON（AI 解析时务必使用）

### 子命令

| 子命令 | 说明 | 示例 |
|---|---|---|
| `today` | 今日相关（今天到期 ∪ 今天开始 ∪ 已逾期未完成） | `python minitodo.py today --json` |
| `add <title>` | 新增待办 | `python minitodo.py add "买菜" --priority high --due 2026-05-20` |
| `done <ref>` | 标记完成 | `python minitodo.py done C3` 或 `done 1734567890123456` |
| `list` | 列表，可叠加 `--pending` / `--completed` / `--priority` / `--limit` / `--sort` | `python minitodo.py list --pending --priority high --json` |
| `search <kw>` | 关键词搜索（title + notes） | `python minitodo.py search "报告" --json` |
| `show <ref>` | 详情（默认含 subtasks） | `python minitodo.py show C3 --json` |
| `update <ref> <key=val>...` | 修改字段，支持多个 | `python minitodo.py update C3 dueDate=2026-06-01 priority=medium` |
| `delete <ref>` | 删除（连带 subtasks） | `python minitodo.py delete C3` |
| `health` | 健康检查 | `python minitodo.py health --json` |
| `sync` | 手动触发 WebDAV 同步（pull + push） | `python minitodo.py sync --json` |
| `sync pull` | 仅从 WebDAV 拉取最新数据 | `python minitodo.py sync pull --json` |
| `sync push` | 仅推送本地变更到 WebDAV | `python minitodo.py sync push --json` |

`<ref>` = `C{seq}` 短码（cloud 端 1, 2, 3... 自增）或完整 i64 id。`C` 大小写不敏感。

`update` 的 value 自动尝试解析为 `true` / `false` / `null` / 数字 / JSON，剩下当字符串。

## 常见任务 → 命令映射

| 用户问 | 一行命令 |
|---|---|
| 今天有什么 | `python ~/.claude/skills/minitodo/minitodo.py today --json` |
| 还有几个高优先级没完成 | `python ~/.claude/skills/minitodo/minitodo.py list --pending --priority high --json` |
| 新增"周五开会准备 PPT" | `python ~/.claude/skills/minitodo/minitodo.py add "周五开会准备 PPT" --priority medium --json` |
| 完成 C3 | `python ~/.claude/skills/minitodo/minitodo.py done C3 --json` |
| 把 C3 的截止日改到下周 | `python ~/.claude/skills/minitodo/minitodo.py update C3 dueDate=2026-05-20 --json` |
| 删掉一周前完成的所有 | 先 `list --completed --sort -updatedAt --json`，再分别 `delete C{seq}` |
| 确保数据是最新的 | `python ~/.claude/skills/minitodo/minitodo.py sync --json` |

## 直接调用 HTTP API（fallback）

CLI 不可用时，AI 可直接用 curl。所有请求都要 `Authorization: Bearer <api_key>`。

| Method | Path | 用途 |
|---|---|---|
| `GET /health` | 服务健康（含 sync 状态） |
| `GET /todos?...` | 列表；query: `completed=true/false`, `priority=high/medium/low`, `quadrant=1-4` 或 `urgent_important` 等别名, `dueDateBefore`, `dueDateAfter`, `startDate=YYYY-MM-DD`, `q=<keyword>`, `sort=[+-]<field>`, `limit`, `offset`, `withSubtasks=true` |
| `GET /todos/:id?withSubtasks=true` | 详情 |
| `POST /todos` | 创建；body 必填 `title` |
| `PATCH /todos/:id` | merge 更新；未提及字段保留 |
| `DELETE /todos/:id` | 删除（连带 subtasks） |
| `POST /todos/:id/subtasks` | 创建子任务；body 必填 `title` |
| `PATCH /subtasks/:id` | merge 更新子任务 |
| `DELETE /subtasks/:id` | 删除子任务 |
| `GET /images/:name` | 取图片 bytes |
| `POST /images` | multipart 上传，`file` 字段；返回 `{name}` |
| `POST /sync` | 手动触发 pull + push；返回 `{pull, push, pullError?, pushError?}` |
| `POST /sync/pull` | 仅 pull；返回 `{status: "ok"}` |
| `POST /sync/push` | 仅 push；返回 `{status: "ok"}` |

排序字段白名单：`dueDate` / `startTime` / `priority` / `quadrant` / `sortOrder` /
`updatedAt` / `createdAt` / `title`。前缀 `-` 倒序、`+` 或省略正序。

### todo JSON 字段速查

`GET /todos` / `GET /todos/:id` 返回的对象是 PC 端 todo 原样透传（KV-style，
PC 加新字段云端自动透传，不需要发版）。下面是临期提醒最常用到的一组字段：

| 字段 | 类型 | 说明 |
|---|---|---|
| `seq` | int（≥ 1） | **cloud-only 短码**。用户/LLM 反馈用 `#C{seq}` 引用，简短易读 |
| `id` | int（i64 时间戳变体） | 跨设备唯一稳定的内部 id；推送时**优先用 `#C{seq}`**，缺时降级到 `#{id}` |
| `title` | 字符串 | 任务名称 |
| `priority` | `"high"` / `"medium"` / `"low"` / null | 优先级 |
| `completed` | bool | 完成态；`list --pending` 已筛 false |
| `startTime` | `YYYY-MM-DD HH:MM:SS` / 空 | 开始时间 |
| `endTime` 或 `dueDate` | `YYYY-MM-DD HH:MM:SS` / 空 | 结束 / 截止时间。新版用 `endTime`，老数据可能用 `dueDate` |
| `notifyAt` | `YYYY-MM-DD HH:MM:SS` / 空 | 下一次提醒时间。**重复任务每次触发后只更新这个字段**，`endTime` 不变 |
| `repeatEnabled` | bool | 是否开启重复 |
| `repeatType` | `"daily"` / `"weekly"` / `"monthly"` / null | 重复类型 |
| `repeatInterval` | int，缺省 1 | 间隔；`weekly` + `interval=2` = 每两周 |
| `repeatWeekdays` | `"1,3,5"` / null | 仅 `weekly`：星期几（1=周一 … 7=周日） |
| `repeatMonthDay` | int / null | 仅 `monthly`：每月几号 |
| `description` | 字符串 | 备注 / 富文本说明 |
| `quadrant` | 1-4 / null | 四象限分类 |
| `subtaskCount` | int | 子任务数量（`withSubtasks=false` 时附带） |

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
> 非重复任务按 `endTime` → `dueDate` → `notifyAt` 顺序取第一个非空值。

curl 示例：

```bash
# 列表 + 排序
curl -H "Authorization: Bearer $KEY" \
  "https://minitodo.example.com/todos?completed=false&priority=high&sort=-dueDate"

# 新增
curl -H "Authorization: Bearer $KEY" -H "Content-Type: application/json" \
  -d '{"title":"买菜","priority":"high","dueDate":"2026-05-20"}' \
  https://minitodo.example.com/todos

# merge 更新（path 支持 C{seq} 短码或完整 i64 id）
curl -X PATCH -H "Authorization: Bearer $KEY" -H "Content-Type: application/json" \
  -d '{"completed":true}' \
  https://minitodo.example.com/todos/C3

# 上传图片
curl -H "Authorization: Bearer $KEY" \
  -F "file=@/tmp/screenshot.png" \
  https://minitodo.example.com/images
```

响应头永远包含：

- `X-Sync-Status: healthy | stale | offline` —— 与 WebDAV 的同步状态
- `X-Last-Sync-At: <时间字符串>` —— 最近一次成功 pull/push
- `Warning: 110 "sync offline"` —— offline 时附带

`X-Sync-Status=offline` 时仍可读写，但写入暂不会反映到 PC（恢复后自动回写）。

## 在 openclaw 中启用临期提醒

整体范式：cron 定时唤起一个 isolated session，session prompt 让你 **agent 自己**
拉所有未完成 todo、根据用户偏好的临期窗口判断哪些该推、组织好格式后通过
`--announce` 推到 default channel（session history 中最近用过的 channel）。

**为什么不让 skill 内置一个 due-soon 子命令？** mini-todo 的重复提醒只更新
`notifyAt`、不动 `dueDate`；而服务端 query 又只能按 `dueDate` 筛——客户端硬
编码规则永远会有漏推或误推。让 agent 看到原始 JSON、自己判断，更稳。

详细的安装 / 配置 / cron message 模板见仓库根 `cloud/openclaw.md`。

最简核心步骤（具体 prompt 见 `cloud/openclaw.md`）：

```bash
# 1. 装 skill 到 openclaw workspace
bash install.sh --target openclaw

# 2. 编辑 ~/.openclaw/workspace/skills/minitodo/config.toml 填 endpoint + api_key

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

2. **对每条 todo，按以下优先级取"时间锚"**：

   a. 若 `repeatEnabled == true` 且 `notifyAt` 非空 → 用 `notifyAt`

   b. 若 `repeatEnabled == true` 且 `notifyAt` 为空 → 根据 `repeatType` 推算本次应触发时间：
      - **monthly**：取当月 `repeatMonthDay` 日的 00:00。若该日已过 → 算逾期。
        逾期天数 = 当月当前日号 − repeatMonthDay。
      - **weekly**：如果 `repeatWeekdays` 包含今天 → 时间锚 = 今天 00:00。
        若不包含今天，找 `repeatWeekdays` 中下一个最近的星期几，算出天数差。
      - **daily**：时间锚 = 今天 00:00。

   c. 否则按 `endTime` → `dueDate` → `notifyAt` 顺序取第一个非空值

   d. 全空则**跳过**该 todo（纯备忘类不推送）

3. **判断临期**——用当前墙钟时间 `now`（Asia/Shanghai）和窗口 **H = 12 小时**：

   - 时间锚 < `now`                → "已逾期"（显示逾期多久）
   - `now` ≤ 时间锚 ≤ `now + 12h`  → "未来 12h 到期"（显示还有多久）
   - 时间锚 > `now + 12h`          → 跳过

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
   未来 12h 到期（M）：
   - #C{seq} [优先级] 标题 (MM-DD HH:MM，X 小时后｜重复描述)
   ```

   - 优先级映射：high → 高、medium → 中、low → 低，没有就省略 `[...]`
   - **前缀优先用 `#C{seq}`**（短码，用户回复"完成 C3"更顺手）；若 JSON 缺 `seq`
     字段则降级用 `#{id}` 完整 i64 id
   - 非重复任务省略 `｜重复描述`
   - 时间差显示规则：< 60 分钟用 "X 分钟后"，< 24h 用 "X 小时后"，≥ 24h 用 "X 天后"；逾期同理
   - 示例：`- #C3 [中] 月度回顾 (05-14 09:00，已逾期 3 小时｜每月 14 号)`

6. 如果两组都为空，只输出一行 `12h 内无临期事项`，**不要再加任何字**。

## 已知限制

- 单用户、单 API key（云端 `config.toml` 配）
- 写入 ~1s 延迟才能反映到 WebDAV，PC 端要在自己的同步周期内才能拉到
- 删除会留下软删除墓碑（7 天内会拦截远端"复活"），跨 7 天的离线设备复活该 ID 仍可能恢复
- 不支持移动端
- 同字段同时改可能丢一方（mini-todo 单用户极少触发；写后立刻 `show` 校验最稳）

## 安装目录约定

最终运行时结构：

```
~/.claude/skills/minitodo/
├── SKILL.md
├── minitodo.py
├── config.example.toml
└── config.toml          # 你自己填，gitignore 掉
```
