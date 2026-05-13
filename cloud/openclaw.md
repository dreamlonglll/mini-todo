# 在 openclaw 中安装 mini-todo Skill

> 本文档面向 **openclaw agent**（如 Claude / 其他 LLM 在 openclaw 工作区中执行）。
> 按顺序执行下面的步骤，把 mini-todo skill 装到 openclaw workspace、配置好、
> 并注册一个定时临期提醒。每一步都给出验证命令，请确认通过后再进入下一步。
>
> 与本文档配套：
> - 仓库内的 [`skill/minitodo/SKILL.md`](skill/minitodo/SKILL.md) — skill 的运行时使用规则（你被 cron / 用户唤起时按这里调用 CLI）
> - 上级 [`README.md`](README.md) — minitodo-cloud 服务端部署文档

## 0. 目标与产出

完成本文档后，openclaw workspace 中将拥有：

1. `~/.openclaw/workspace/skills/minitodo/` 下的 skill 文件（SKILL.md + Python CLI + config）
2. 一个名为 `minitodo-due-soon` 的 cron job：每天定时检查临期待办，自动通过
   `--announce` 推送到 default channel（session history 中最近用过的 channel；
   也可显式指定 Slack / Telegram / Discord 等）

## 1. 前置条件检查

执行以下命令，**全部通过**再继续：

```bash
# 1.1 openclaw 已安装且可用
openclaw --version

# 1.2 cron 子命令可用
openclaw cron --help | head -5

# 1.3 Python 3 在 PATH 中（>= 3.10 推荐；3.10 需要 tomli）
python3 --version || python --version

# 1.4 用户告诉你 minitodo-cloud 已经部署，并且你已经拿到两件东西：
#       - endpoint  形如 https://minitodo.example.com
#       - api_key   长字符串（≥ 16 字符），与服务端 config.toml 一致
#     如果用户没给，停下来问；不要自己捏造。
```

**任何一条不通过时的处理：**

| 失败项 | 处理 |
|---|---|
| 1.1 openclaw 不在 PATH | 引导用户 `npm install -g openclaw` 或参考其官方安装文档 |
| 1.2 cron 子命令缺失 | openclaw 版本太老，请用户升级 |
| 1.3 Python 缺失 | 引导用户安装 Python 3.10+ |
| 1.4 缺 endpoint/api_key | 引导用户先部署 minitodo-cloud（见 `cloud/README.md`） |

## 2. 获取 skill 源文件

skill 源文件在 mini-todo 仓库的 `cloud/skill/minitodo/` 目录下。**有三种情况**：

### 2.A 用户已 clone 仓库到本机

直接进入 skill 目录：

```bash
cd <用户告诉你的 mini-todo 仓库路径>/cloud/skill/minitodo
ls SKILL.md minitodo.py install.sh config.example.toml   # 验证 4 个文件都在
```

### 2.B 用户没 clone，但允许你 clone

```bash
git clone https://github.com/<owner>/mini-todo.git ~/mini-todo
cd ~/mini-todo/cloud/skill/minitodo
```

> 替换 `<owner>` 为用户告诉你的仓库 owner。**不要自己猜**。

### 2.C 用户没 clone 且不希望 clone 整个仓库

仅下载需要的 4 个文件到临时目录（替换 `<owner>` / `<branch>`，缺省 `main`）：

```bash
mkdir -p /tmp/minitodo-skill && cd /tmp/minitodo-skill
RAW="https://raw.githubusercontent.com/<owner>/mini-todo/<branch>/cloud/skill/minitodo"
for f in SKILL.md minitodo.py install.sh config.example.toml; do
  curl -fsSL "$RAW/$f" -o "$f"
done
ls SKILL.md minitodo.py install.sh config.example.toml
```

## 3. 安装到 openclaw workspace

在 skill 源文件目录下运行：

```bash
bash install.sh --target openclaw
```

**预期输出**包含：

```
>> installing minitodo skill into <home>/.openclaw/workspace/skills/minitodo
!! <home>/.openclaw/workspace/skills/minitodo/config.toml created from example. ...
```

**验证安装结果：**

```bash
ls ~/.openclaw/workspace/skills/minitodo
# 应该列出：SKILL.md  config.example.toml  config.toml  minitodo.py
```

> 如果 `install.sh` 报 `requests` 缺失或 `tomli` 缺失（Python < 3.11），
> 按提示执行 `pip install requests`（必装）/ `pip install tomli`（仅 3.10）。

## 4. 配置 endpoint + api_key

编辑 `~/.openclaw/workspace/skills/minitodo/config.toml`，把用户给的两个值填进去：

```toml
endpoint = "https://minitodo.example.com"          # ← 用户给的真实地址
api_key  = "REPLACE_WITH_REAL_BEARER_FROM_CLOUD"   # ← 用户给的真实 token
timeout  = 10
```

**推荐做法：让用户自己填**，避免你看到明文 api_key。给用户这条命令：

```bash
${EDITOR:-nano} ~/.openclaw/workspace/skills/minitodo/config.toml
```

并提醒：`api_key` 必须与 minitodo-cloud 服务端 `config.toml` 中的 `api_key` **完全一致**。

## 5. 验证 skill 能正常调用云端

```bash
python ~/.openclaw/workspace/skills/minitodo/minitodo.py health --json
```

**预期输出**：

```json
{
  "status": "healthy",
  "sync": "healthy",
  "lastPullAt": "2026-05-13 12:34:56"
}
```

**常见失败：**

| 错误信息 | 原因 | 处理 |
|---|---|---|
| `HTTP 401: unauthorized` | api_key 与服务端不一致 | 让用户核对两端 config.toml |
| `HTTP error: ...connection refused` / `timeout` | endpoint 不可达 / 服务未启动 | 让用户检查 `systemctl status minitodo-cloud` 或反代 |
| `ERROR: 缺少 requests 库` | Python 环境缺 requests | `pip install requests` |
| `sync: offline` | 服务能连上但 WebDAV 拉不到 | 服务端 logs 检查 WebDAV 凭据，临期提醒仍可工作（读本地缓存） |

**通过后跑一次 list 看看：**

```bash
python ~/.openclaw/workspace/skills/minitodo/minitodo.py list --pending --json
```

预期：一个 JSON 数组，每条 todo 至少有 `id` / `title` / `completed`，可能还带
`dueDate` / `notifyAt` / `priority` / `endTime` / `description` 等。**所有
"是否该推送给用户"的判断都交给 agent 在 §7 的 cron prompt 里做**，不要在
skill 端硬编码筛选逻辑。

> **为什么不内置 due-soon 子命令？** mini-todo 的重复提醒只更新 `notifyAt`、
> 不动 `dueDate`；服务端 query 又只能按 `dueDate` 筛——客户端硬编码规则永远
> 有漏推/误推。让你（agent）看到原始 JSON、自己判断更稳。

## 6. 询问用户提醒偏好（不要替用户做主）

在执行 §7 注册 cron **之前**，必须先用日常语言问用户三件事，然后用回答填到
§7 的 `<USER_CRON>` / `<USER_TZ>` / `<USER_HOURS>` 三个占位符。**不要直接套默认值发出去**。

按下面这套问题问用户（一次性问完，或者根据用户语境拆开问，都行）：

| 要问的 | 默认值（仅当用户说"随便/默认"时用） | 取值示例 |
|---|---|---|
| **时区**（IANA） | `Asia/Shanghai` | `Asia/Shanghai` / `America/Los_Angeles` / `Europe/London` |
| **触发时间**（每天几点 / 工作日还是天天 / 几次） | 每天 1 次，08:00 | "每天早 8 点" / "工作日早 9 晚 6" / "每 2 小时" |
| **临期窗口**（往前看多少小时 H） | 24 小时 | "24" / "12" / "48" |

> 触发频率与窗口 H 要"对齐"：每 2 小时跑一次就应该把 H 设成 2，否则同一条
> 临期会在 24 小时内被推 12 遍。问完用户后顺手提醒一句。

把用户回答翻译成 cron 表达式。常见映射：

| 用户说的话 | `<USER_CRON>` | `<USER_HOURS>` 推荐 |
|---|---|---|
| 每天早 8 点 | `"0 8 * * *"` | 24 |
| 每天早 9 点 + 晚 6 点 | `"0 9,18 * * *"` | 12 |
| 工作日早 9 点 | `"0 9 * * 1-5"` | 24（周末顺延） |
| 每 2 小时一次 | `"0 */2 * * *"` | 2 |
| 每小时一次 | `"0 * * * *"` | 1 |

如果用户给的描述模糊（比如"早上"），追问具体时分；不要自己猜测。

## 7. 注册 cron

cron 唤起的 agent（每天那一刻的"你"）**自己**拉数据、自己判断、自己组织格式。
skill 这边只提供数据获取通道。

把 §6 得到的三个值（`<USER_CRON>` / `<USER_TZ>` / `<USER_HOURS>`）填到下面的
命令里再执行：

```bash
openclaw cron add \
  --name minitodo-due-soon \
  --cron "<USER_CRON>" \
  --tz "<USER_TZ>" \
  --session isolated \
  --message $'你被 cron 唤起做 mini-todo 临期提醒。严格按下面流程：\n\n1) 跑命令拿原始 JSON：\n   python ~/.openclaw/workspace/skills/minitodo/minitodo.py list --pending --json\n\n2) 对每条 todo，取时间锚（优先级 dueDate > endTime > notifyAt）。完全没有任何时间字段的 → 跳过。\n\n3) 用当前墙钟时间（cron --tz 已是 <USER_TZ>）和窗口 H=<USER_HOURS> 小时判断：\n   - 时间锚 < now              → "已逾期"\n   - now ≤ 时间锚 ≤ now + H    → "未来 H 小时到期"\n   - 时间锚 > now + H          → 跳过\n\n4) 严格按下面格式输出，不要任何解释/翻译/建议/总结：\n\n   mini-todo 临期提醒｜YYYY-MM-DD HH:MM\n   已逾期（N）：\n     - #{id} [优先级中文] 标题 (MM-DD HH:MM，已逾期 X 小时)\n   未来 <USER_HOURS>h 到期（M）：\n     - #{id} [优先级中文] 标题 (MM-DD HH:MM，X 小时后)\n\n   - 优先级映射：high→高、medium→中、low→低；没有就省略 [..]\n   - #{id} 必须是完整 ID，不要截断\n   - X 小时后：差 < 60 分钟用 "X 分钟后"，> 24 小时用 "X 天后"\n\n5) 如果两组都为空，只输出一行 "<USER_HOURS>h 内无临期事项"，不要再加任何字。' \
  --announce
```

例：用户选了「每天早 8 点 / Asia/Shanghai / 24 小时」，最终命令是把上面三个占位符
替换为 `"0 8 * * *"` / `"Asia/Shanghai"` / `24`。

参数解读（不要随意改）：

| 参数 | 作用 | 改它的代价 |
|---|---|---|
| `--name minitodo-due-soon` | 唯一标识，后续管理用 | 改了的话下面所有 `cron run/remove` 命令也要跟着改 |
| `--cron "<USER_CRON>"` | 触发时机 | 见 §9.A 的事后调整方法 |
| `--tz "<USER_TZ>"` | 时区 | 必须与用户实际生活时区一致，否则会在凌晨推 |
| `--session isolated` | 干净 agent turn | 改 `main` 会污染主对话历史；改 `current` 会跑在当前 session（不推荐） |
| `--message ...` | agent 在 cron session 里的工作清单 | 改格式 / 跳过规则前先想清楚；message 是"未来的你"唯一的指令源 |
| `--announce` | 把回复推到 channel | 缺这个就只在 cron history 里能看到 |

**不带 `--channel` / `--to`** 时，openclaw 用 session history 中最近用过的 channel
作为 default channel（参考 openclaw 文档 `docs/automation/cron-jobs.md` 中
`channel: "last"`）。如果用户希望固定推到某个 IM channel，跳到 §9.B。

> **shell 转义提示**：上面 message 用了 `$'...'` 让 `\n` 真正成为换行；如果你
> 在 PowerShell 里跑，要换成多行 here-string 或一行长字符串拼接。也可以把
> message 写进 `~/.openclaw/workspace/cron-message-minitodo.txt`，再用 `--message-file` 引用（参考 openclaw cron 文档）。

## 8. 验证 cron

### 8.1 列出 cron 看注册成功

```bash
openclaw cron list
# 应包含一行 minitodo-due-soon
```

### 8.2 立刻强制跑一次（不等下一次触发）

```bash
openclaw cron run minitodo-due-soon
```

预期：default channel（或 cron 输出 / 当前对话窗口）出现 mini-todo 临期摘要，
由 agent 按 §7 prompt 自己组织、形如：

```
mini-todo 临期提醒｜2026-05-13 08:00
已逾期（1）：
  - #1734567890123456 [高] 写报告 (05-12 18:00，已逾期 14 小时)
未来 24h 到期（2）：
  - #1734567890123457 [中] 买菜 (05-13 10:00，2 小时后)
  - #1734567890123458 [低] 开会 (05-13 19:00，11 小时后)
```

每条任务前的 `#{id}` 是 mini-todo 的内部 ID，用户反馈 / 二次操作时可直接用它定位。

或者，如果没有任何临期事项：`<USER_HOURS>h 内无临期事项`。

> agent 在 §7 流程的第 2 步会自行跳过没有任何时间字段（`notifyAt` / `dueDate` /
> `endTime` 全空）的 todo；纯文本备忘类不会出现在 channel 推送里。

### 8.3 看执行历史

```bash
openclaw cron runs --id minitodo-due-soon
```

成功的 run 会标记 `status: ok`；失败的会有 error 信息，按里面的提示回到 §5 排查。

## 9. 事后调整

### 9.A 改时间 / 频率 / 临期窗口

用 `openclaw cron edit`，新 cron 表达式参考 §6 的映射表：

```bash
# 改时间
openclaw cron edit minitodo-due-soon --cron "0 9,18 * * *" --tz "Asia/Shanghai"

# 改临期窗口（H）：重新生成 §7 的 message，把所有 <USER_HOURS> 出现处替换成新 H
openclaw cron edit minitodo-due-soon \
  --message "<§7 模板，把 <USER_HOURS> 全部换成新 H>"
```

> 频率与 H 要对齐：每 2 小时跑就把 H 设成 2，否则同一临期会被反复推送。

### 9.B 显式指定 channel（推荐生产场景）

不依赖 "session last channel"，固定推到一个 IM channel：

```bash
openclaw cron edit minitodo-due-soon \
  --channel slack --to "channel:C1234567890"
```

其他 channel 的 `--to` 格式：

| Channel | --to |
|---|---|
| Slack | `channel:<C... id>` |
| Telegram | `<-100xxx 群 id>` |
| Discord / Mattermost | `channel:<id>` 或 `user:<id>` |
| Matrix | `room:!room:server` |

具体 id 怎么拿，参考 openclaw `docs/channels/<provider>.md`。

### 9.C 用 standing order 简化 cron message

如果用户在 `~/.openclaw/workspace/AGENTS.md` 里写了 standing order：

```markdown
## Program: mini-todo 临期提醒

**Authority:** 检查 mini-todo 临期待办，整理后通过 default channel 推送
**Trigger:** 由 cron job `minitodo-due-soon` 触发
**Approval gate:** 无
**Escalation:** skill `health` 返回非 healthy 时附加一行警告

### Execution steps
1. 跑 `python ~/.openclaw/workspace/skills/minitodo/minitodo.py list --pending --json`
2. 按字段优先级 dueDate > endTime > notifyAt 取时间锚，无时间字段则跳过
3. 把 [now, now+24h] 内的归入"未来 24h 到期"，时间锚 < now 的归入"已逾期"
4. 按 §7 prompt 第 4 步给出的格式输出（含 #{id} 前缀），两组均空时回复"24h 内无临期事项"

### What NOT to do
- 不要补充推理 / 翻译 / 解释
- 不要并行调用其他 skill
- 不要截断或省略 #{id}
```

写好后 cron message 可简化为：

```bash
--message "执行 mini-todo 临期提醒 per standing orders"
```

## 10. 卸载

```bash
openclaw cron remove minitodo-due-soon
rm -rf ~/.openclaw/workspace/skills/minitodo
```

（如果用 `--target both` 同时装了 Claude Code，那边的 `~/.claude/skills/minitodo`
按需另外清理。）

## 故障排查速查

| 症状 | 排查 |
|---|---|
| `openclaw cron run` 跑完 channel 没收到 | 1) `--announce` 是否带了？`openclaw cron get minitodo-due-soon` 看 JSON；2) 该 session 之前是否用过 channel？没用过就用 §9.B 显式指定 |
| 推送的是空（既没列表也没"Xh 内无临期事项"） | agent 没按 §7 第 5 步执行；用 `openclaw cron get minitodo-due-soon` 看 message 是否完整 |
| 推送内容里少了 `#{id}` 前缀 | agent 偷懒把 ID 去掉了；在 message 末尾追加"严禁省略 #{id}" |
| 推送格式不像 §7 模板 | agent 自作主张改写；增强 message 措辞，强调"严格按格式、不要总结/翻译" |
| 推送内容里有多余的解释文字 | message 末尾追加更强的"只输出 stdout，不要任何补充"指令 |
| 时间显示比实际晚 / 早 | `--tz` 与服务端 `timezone` 不一致，统一改成用户所在时区 |
| skill `health` 返回 `sync: stale` | minitodo-cloud 已经超过 5×pull_interval 没成功拉过 WebDAV；服务端日志查 WebDAV 401/403 |
| skill `health` 返回 `sync: offline` | 同上，但仍能读写本地缓存；提醒仍能正常发出，只是数据可能过时 |

## 设计要点（供 agent 理解，不必复述给用户）

- **skill 只是数据通道**：CLI 只暴露 CRUD/list/show 等通用命令，不内置"哪些算临期"
  的硬编码规则；判断逻辑放在 cron message 里、由 agent 在 session 中执行——好处是
  当用户改临期定义或 mini-todo 字段语义变化时，只需改 prompt，不必改代码或重装 skill
- **default channel = session last channel**：openclaw cron 的 `--announce` 不带 channel 时走 `channel: "last"`，这是 openclaw cron 文档显式记录的行为
- **isolated session**：每次 cron 起一个独立 turn，避免污染主对话；意味着每次 cron 调用 agent 都"忘记上次"，所以 message 必须自包含、不依赖历史上下文
- **skill 位置兼容**：`install.sh` 既支持 `--target claude` 装到 `~/.claude/skills/`（Claude Code），也支持 `--target openclaw` 装到 `~/.openclaw/workspace/skills/`，源文件完全一致
