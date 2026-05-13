---
name: minitodo
description: 通过云端 HTTP API 读写用户的 mini-todo 待办；适合 today / due-soon / add / done / list / search / show / update / delete 场景，可被 openclaw cron 调度做临期提醒。
version: 1.1.0
---

# mini-todo Skill

让 Claude Code 通过云端 HTTP API 操作用户的 mini-todo 待办数据。底层数据通过
WebDAV 与 PC 端共享同一份 `sync-data.json.gz`，AI 写入 1 秒内会回写 WebDAV，PC
端下次同步即可见。

## 何时调用

用户说出下面意图时，先 `Bash` 调用本 skill 的 CLI：

- "今天有什么待办？" → `python ~/.claude/skills/minitodo/minitodo.py today --json`
- "未来 24h 有什么临期？" / 被 cron 唤起做提醒 → `due-soon`
- "帮我加一个'买菜'到待办" → `add`
- "把 ID xxx 标记完成" → `done`
- "看看所有未完成的高优先级" → `list --pending --priority high`
- "搜一下含'报告'的待办" → `search`
- "ID xxx 是什么？" → `show`
- "把 ID xxx 的截止日期改到 2026-05-20" → `update`
- "删除 ID xxx" → `delete`

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
| `due-soon` | 临期：未来 N 小时内到期 ∪ 已逾期未完成 | `python minitodo.py due-soon --hours 24 --notify-text --silent-if-empty` |
| `add <title>` | 新增待办 | `python minitodo.py add "买菜" --priority high --due 2026-05-20` |
| `done <id>` | 标记完成 | `python minitodo.py done 1734567890123456` |
| `list` | 列表，可叠加 `--pending` / `--completed` / `--priority` / `--limit` / `--sort` | `python minitodo.py list --pending --priority high --json` |
| `search <kw>` | 关键词搜索（title + notes） | `python minitodo.py search "报告" --json` |
| `show <id>` | 详情（默认含 subtasks） | `python minitodo.py show 1734567890123456 --json` |
| `update <id> <key=val>...` | 修改字段，支持多个 | `python minitodo.py update 1734567890123456 dueDate=2026-06-01 priority=medium` |
| `delete <id>` | 删除（连带 subtasks） | `python minitodo.py delete 1734567890123456` |
| `health` | 健康检查 | `python minitodo.py health --json` |

`update` 的 value 自动尝试解析为 `true` / `false` / `null` / 数字 / JSON，剩下当字符串。

## 常见任务 → 命令映射

| 用户问 | 一行命令 |
|---|---|
| 今天有什么 | `python ~/.claude/skills/minitodo/minitodo.py today --json` |
| 还有几个高优先级没完成 | `python ~/.claude/skills/minitodo/minitodo.py list --pending --priority high --json` |
| 新增"周五开会准备 PPT" | `python ~/.claude/skills/minitodo/minitodo.py add "周五开会准备 PPT" --priority medium --json` |
| 完成 ID 1234567890123 | `python ~/.claude/skills/minitodo/minitodo.py done 1234567890123 --json` |
| 把 ID xxx 的截止日改到下周 | `python ~/.claude/skills/minitodo/minitodo.py update xxx dueDate=2026-05-20 --json` |
| 删掉一周前完成的所有 | 先 `list --completed --sort -updatedAt --json`，再分别 `delete` |

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

排序字段白名单：`dueDate` / `startTime` / `priority` / `quadrant` / `sortOrder` /
`updatedAt` / `createdAt` / `title`。前缀 `-` 倒序、`+` 或省略正序。

curl 示例：

```bash
# 列表 + 排序
curl -H "Authorization: Bearer $KEY" \
  "https://minitodo.example.com/todos?completed=false&priority=high&sort=-dueDate"

# 新增
curl -H "Authorization: Bearer $KEY" -H "Content-Type: application/json" \
  -d '{"title":"买菜","priority":"high","dueDate":"2026-05-20"}' \
  https://minitodo.example.com/todos

# merge 更新
curl -X PATCH -H "Authorization: Bearer $KEY" -H "Content-Type: application/json" \
  -d '{"completed":true}' \
  https://minitodo.example.com/todos/1234567890123

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

openclaw 内置 cron 调度器，可以定时唤起一个 isolated session、让 agent 在该 session 内
调用本 skill，并把结果通过 `--announce` 推送到 default channel（即 session history 中
最近使用的 channel）。整体语义参考 openclaw 文档：
`docs/automation/cron-jobs.md` 与 `docs/automation/standing-orders.md`。

### 安装到 openclaw workspace

先把 skill 装到 openclaw 能加载的目录：

```bash
bash install.sh --target openclaw
```

最终结构：

```
~/.openclaw/workspace/skills/minitodo/
├── SKILL.md
├── minitodo.py
├── config.example.toml
└── config.toml          # 你自己填，gitignore 掉
```

### 注册 cron 任务（每天早 8 点检查临期）

```bash
openclaw cron add \
  --name minitodo-due-soon \
  --cron "0 8 * * *" \
  --tz "Asia/Shanghai" \
  --session isolated \
  --message "调用 minitodo skill 的 due-soon 子命令检查未来 24 小时内的临期和已逾期待办。命令：python ~/.openclaw/workspace/skills/minitodo/minitodo.py due-soon --hours 24 --notify-text --silent-if-empty。把命令 stdout 的内容直接作为回复发出来；如果 stdout 为空就只回复'今日无临期事项'。不要追加解释。" \
  --announce
```

要点：
- `--session isolated` 让 cron 每次起一个干净 agent turn，避免污染主 session
- 不带 `--channel` / `--to` 时，openclaw 用 session history 中的"最近 channel"作为
  default channel（参考 `cron-jobs.md` 中 `channel: "last"` 的描述）
- `--silent-if-empty` 让 skill 在没有任何临期事项时输出空字符串，prompt 再兜底成"今日无临期事项"
- 改频率：`--cron "0 8,18 * * *"`（早晚各一次）、`--cron "0 */2 * * *"`（每 2 小时）

### 显式指定 channel（可选）

要把通知固定推到某个 IM channel 而不是"最近使用"，加 `--channel` + `--to`：

```bash
# Slack
... --announce --channel slack --to "channel:C1234567890"

# Telegram
... --announce --channel telegram --to "-1001234567890"

# Discord / Mattermost
... --announce --channel discord --to "channel:<id>"
```

### 管理 / 调试

```bash
openclaw cron list                       # 看所有 cron
openclaw cron run minitodo-due-soon      # 立刻强制跑一次（测试）
openclaw cron runs --id minitodo-due-soon # 看历史执行
openclaw cron remove minitodo-due-soon   # 删除
```

### 也可作为 standing order

如果你在 `~/.openclaw/workspace/AGENTS.md` 里写了 standing order（"你拥有每日临期提醒
的执行权"），cron 的 `--message` 可以简化为 "执行 daily reminders per standing orders"，
具体步骤交给 standing order 描述，避免 cron message 太长。

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
