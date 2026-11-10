# minitodo-cloud

mini-todo 的云端 HTTP API（Rust + Axum），与 PC 端共用同一个 WebDAV
`sync-data.json.gz` 做底层数据通道。AI 客户端（Claude Code Skill / 任意带
HTTP 客户端的工具）通过本服务读写用户待办，数据最终回写 WebDAV，PC 端下次
同步拉到。

> 同步协议（两端共用的文档结构、合并规则、WebDAV 请求规则）见
> [`.trellis/spec/backend/sync-protocol.md`](../.trellis/spec/backend/sync-protocol.md)。

## 总览

```
┌────────┐  HTTPS+Bearer   ┌──────────────┐ pull 60s   ┌─────────┐
│   AI   │ ──────────────► │ minitodo-    │ ─────────► │ WebDAV  │
│ Skill  │ ◄────────────── │ cloud (本服) │ ◄───────── │ server  │
└────────┘    JSON / files └──────┬───────┘ push 去抖   └────┬────┘
                                  │ rusqlite                │
                                  ▼                         ▼
                          /var/lib/minitodo/         /mini-todo/
                          data.db + images/          sync-data.json.gz
                                                     images/
```

* **WebDAV 是 source of truth**。云端 SQLite 是持久化缓存（重启后保留，启动时先拉取合并一次）；
  API 写入先落本地并标脏，由 push worker 回写 WebDAV，推送成功前不会被拉取覆盖。
* **时间格式与 PC SQLite 完全一致**：`YYYY-MM-DD HH:MM:SS` 无时区后缀，按
  config.toml `timezone`（IANA）每次重新换算墙钟时间（夏令时安全）。
* **记录级合并**（与 PC 同一套规则）：按 `updatedAt` 逐条 LWW，平局保留本地；只在一侧存在的记录保留；
  删除靠墓碑（`tombstones`，随 sync-data 双向传播，保留 30 天）。
* **待办清单**（PC 3.1 起的多清单）：清单在 sync-data 顶层的 `todoLists`，清单删除墓碑在独立的
  `listTombstones`（不进 `tombstones`：旧版本会丢掉不认识的墓碑类型），合并规则与 todo 相同，只是
  **清单墓碑永不过期**（清单很少；这样长期离线的设备带不回已删的清单）；默认清单 id 1（"待办清单"）可改名、
  不可删除，它的墓碑一律忽略。远端没有 `todoLists`（旧格式）时本地清单一条都不删。待办的 `listId` 指向已删除的
  清单时按默认清单显示 / 过滤（读侧兜底，记录不改写）。契约见 `.trellis/tasks/11-09-multi-list-windows/contract.md`。
* **`listId` 缺失 = 未知**（契约 §12）：旧版本 PC 每次上传都会丢掉全部待办的 `listId`，所以缺失（或 `null`）
  表示"写入方不认识清单"，**不等于**默认清单——只在显示 / 过滤时算默认清单，从不写回成 1。上传时未知不写
  `listId` / `listUpdatedAt`，cloud 也不替缺 `listId` 的记录补值。
* **清单归属字段级合并**（契约 §13）：`listId` 有自己的版本 `listUpdatedAt`，归属 `(listId, listUpdatedAt)`
  不跟着整条记录的 `updatedAt` 走。新旧：未知 < 已知但没有时间 < 已知有时间（按时间），一样新时取较大的 `listId`。
  记录本身照旧按 `updatedAt` LWW 与墓碑决定；结果记录存在时，归属取所有看得到的版本（本地合并前的版本、远端版本，
  含被墓碑压制的）里最新的，采纳不改 `updatedAt`、不标脏；合并后远端那一版的归属比本地旧 → 补传一次。
  畸形记录（`listId` 类型不对、`updatedAt` 无法解析）不参与归属合并。`listUpdatedAt` 只在显式设置清单时刷新
  （新建、`PATCH` 改了 `listId`、删除清单把待办移进默认清单），由服务端维护。
* **不支持与 ≤ 2.4.x 混用**：那些版本的写入方不带回不认识的顶层键、也没有墓碑，上传一次就会抹掉
  `todoLists` / `listTombstones` 与全部归属。
* **时间戳单调**：改写已有记录时 `updatedAt = max(now, 旧值 + 1 秒)`，同一秒内两次修改也不会丢；
  删除的墓碑 `deletedAt = max(now, 记录的 updatedAt)`，保证压得住被删的版本。
* **WebDAV 请求**：写前总是先 GET 合并；条件 PUT 只用 `If-Match`（去掉 `W/`），从不用
  `If-Unmodified-Since` / `If-Modified-Since`（Apache 与 nginx 的实现互不兼容，见上面的协议文档）。

## 构建

```bash
cd cloud
cargo build --release
# 产物：target/release/minitodo-cloud
```

无需任何 C 工具链（rusqlite bundled、reqwest rustls）。

也可以直接用预编译版：仓库每次发版（推 `v*` tag，`.github/workflows/release.yml`）都会在 GitHub Release 附带
`minitodo-cloud-<cloud 版本>-linux-<x86_64|aarch64>.tar.gz` 与对应的 `.sha256`（cloud 版本取自 `cloud/Cargo.toml`，与 PC 版本号无关）。

## 配置

复制 `config.example.toml` 为 `/etc/minitodo/config.toml`（路径自定），按注释填
WebDAV 凭据、`api_key`、`timezone` 等。

`api_key` 建议至少 32 字符随机串：

```bash
openssl rand -hex 32
```

## 部署（systemd + Caddy）

```bash
# 1. 创建 service 账户与数据目录
sudo useradd --system --no-create-home minitodo
sudo mkdir -p /opt/minitodo-cloud /var/lib/minitodo /etc/minitodo
sudo chown -R minitodo:minitodo /var/lib/minitodo

# 2. 拷贝二进制与配置
sudo cp target/release/minitodo-cloud /opt/minitodo-cloud/
sudo cp config.example.toml /etc/minitodo/config.toml
sudo $EDITOR /etc/minitodo/config.toml      # 填实际值
sudo chown root:minitodo /etc/minitodo/config.toml
sudo chmod 640 /etc/minitodo/config.toml

# 3. 安装 systemd unit
sudo cp deploy/minitodo-cloud.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now minitodo-cloud

# 4. 反向代理（任选其一）
#    Caddy：将 deploy/Caddyfile.example 中的 server block 并入 /etc/caddy/Caddyfile
#           然后 `sudo systemctl reload caddy`，自动签 Let's Encrypt 证书
#    Nginx：自行配置反代到 127.0.0.1:8787

# 5. 健康检查
curl -H "Authorization: Bearer $API_KEY" https://minitodo.example.com/health
```

输出示例（同步正常时 200；拉取或推送异常时 `status: "degraded"` 并返回 **503**，
方便外部监控直接按状态码告警）：

```json
{"status":"healthy","sync":"healthy","pull":"healthy","push":"healthy",
 "lastPullAt":"2026-05-13 12:34:56","lastPullError":null,
 "lastPushOkAt":"2026-05-13 12:30:02","lastPushError":null,
 "dirty":false,"dirtySince":null,"imageQueueLength":0}
```

并附 HTTP 响应头：

```
X-Sync-Status: healthy
X-Last-Sync-At: 2026-05-13 12:34:56
```

## WebDAV server 选型

| 选项 | 说明 |
|------|------|
| **Caddy + caddy-webdav 插件** | 单二进制，与反代共用 TLS；适合自托管个人 |
| **nginx-dav-ext-module** | 通过 nginx `dav_methods PUT DELETE MKCOL COPY MOVE` 开放；需要重新编译 nginx 或用 openresty |
| **NextCloud** | 重量级，但自带 Web UI；本服务通过 `/remote.php/dav/files/USER` 接入 |
| **坚果云** | 国内体验稳定；WebDAV 限速明显，仅适合个人 |

e2e 套件（`pc-gpui/scripts/e2e-linux/`，旧 Tauri 版时期也有一套）对 Apache mod_dav 与 nginx dav（+ dav_ext）都完整验证过同步。
nginx 必须加载 `dav_ext` 模块（PROPFIND），否则图片清单拿不到。

最简单的自托管：

```caddy
:8443 {
    tls /etc/ssl/cert.pem /etc/ssl/key.pem
    basicauth /mini-todo/* {
        webdavuser <bcrypt-hash>
    }
    webdav /mini-todo/* {
        root /srv/webdav
    }
}
```

PC 端与 cloud 同时填写 `https://yourhost:8443`、`webdavuser`、原始密码。

## 配置字段速查

| 字段 | 必填 | 默认 | 说明 |
|---|---|---|---|
| `webdav_url` | ✓ | — | WebDAV 服务器根 URL，不带尾部 `/mini-todo` |
| `webdav_username` | ✓ | — | WebDAV 账号 |
| `webdav_password` | ✓ | — | WebDAV 密码 |
| `webdav_ca_file` | × | — | 额外信任的 CA 证书（PEM），用于自签证书的 WebDAV |
| `api_key` | ✓ | — | Bearer Token；< 16 字符拒绝启动，< 32 字符启动时 warn |
| `bind` | × | `127.0.0.1:8787` | HTTP 监听地址 |
| `timezone` | × | `Asia/Shanghai` | IANA 时区，**必须与 PC 端一致** |
| `pull_interval` | × | `60` | Pull worker 间隔（秒） |
| `data_dir` | × | `/var/lib/minitodo` | SQLite 与 meta 数据目录 |
| `images_dir` | × | `/var/lib/minitodo/images` | 镜像图片目录 |

缺任意必填字段或值不合法 → 进程启动直接退出并打印清晰错误；非本机的明文 `http://` WebDAV 启动时打 warn。

## 当前能力

服务端：

- [x] `GET /health`：同步状态、最近错误与本地积压；异常时 503
- [x] Bearer token 鉴权（错/缺 → 401）；5xx 不向客户端泄露内部错误细节
- [x] 启动时拉取一次（`spawn_blocking`，失败不阻断启动）；之后每 `pull_interval` 秒条件拉取（`If-None-Match`），
      每 10 轮做一次无条件全量拉取（nginx 的 ETag 只有秒级精度）；失败指数退避 + 抖动（上限 5 分钟）
- [x] push worker：每 500ms 检查 `meta.dirty`，连续写入去抖 1.5s（最长攒 10s）→ 写前 GET 合并 → `If-Match` 条件 PUT；
      412 等 1.1s 后重新拉取合并再试（≤3 次）；推送期间的新写入保持 dirty，不会被吞
- [x] 墓碑：DELETE 写 `tombstones`，随 sync-data 双向传播；远端缺本地记录或墓碑时自动补推（自愈）
- [x] 图片：`POST /images` 进入推送队列；每次拉到新文档后补下缺失图片（失败退避重试）；文件名白名单校验
- [x] 写入校验：只接受 PC 模型里的字段与类型（未知字段 / 类型错误 → 400），`dueDate` / `priority` / `notes` 等别名自动映射；
      启动后一次性把旧版 API 写入的别名字段、非规范时间归一化
- [x] 优雅停机：停止接收新请求、等待在途请求，有待推送内容时限时补推一次
- [x] `/todos` `/subtasks` `/images` REST CRUD（filter / sort / pagination / merge PATCH / cascade DELETE）
- [x] `/lists` 待办清单 CRUD（按 id 或名称寻址、重名 409、删除时其下待办移到默认清单）；
      todo 的 `listId` 写入校验与 `listId=` / `list=` 过滤；`todoLists` / `listTombstones` 随 sync-data 双向同步
- [x] 待办的清单归属按 `listUpdatedAt` 做字段级合并（契约 §13）；远端那一版的归属比本地旧时补推；清单墓碑永不过期

Skill / AI 集成：

- [x] Claude Code Skill（`cloud/skill/minitodo/`，含 Python CLI、单测、install 脚本、SKILL.md）
- [x] CLI 覆盖 today / list / add / done / search / show / update / delete / health / sync / lists；
      `update` 按字段类型转换取值（字符串字段不会被误转成数字）；部分失败退出码 4；
      待办相关命令带 `--list <清单名称或 id>`（先经 `GET /lists/<清单>` 解析成 id）
- [x] cloud 端 `C{seq}` 短码：每个 todo 分配自增短码（`todo_seq` 表），CLI 与 API 均可用 `C3` 代替完整 id
- [x] 同一份 skill 可装到 openclaw workspace（`install.sh --target openclaw`）
- [x] openclaw cron 临期提醒：cron 唤起 agent 后由 **agent 自己**拉 `list --pending --json` + 判断哪些该推 + 组织格式，`--announce` 推到 default channel

## REST API 速查

所有请求都需要 `Authorization: Bearer <api_key>`。

| Method | Path | 说明 |
|---|---|---|
| GET | `/health` | `{status, sync, lastPullAt}` |
| GET | `/todos` | 列表。query：`completed=true/false`, `priority=high/medium/low`, `quadrant=1..4` 或 `urgent_important` 等别名, `dueDateBefore`, `dueDateAfter`, `startDate=YYYY-MM-DD`, `q=<keyword>`, `listId=<清单 id>` / `list=<清单名>`（按有效清单，见下）, `sort=[+-]<field>`, `limit`, `offset`, `withSubtasks=true` |
| GET | `/todos/:id?withSubtasks=true` | 详情；默认嵌套 subtasks，`withSubtasks=false` 扁平化 |
| POST | `/todos` | 创建；body 必填 `title`；其余字段按下方字段契约校验 |
| PATCH | `/todos/:id` | merge 更新；未提及字段保留（含更新版 PC 加的、本服务还不认识的字段）；内容没变时不刷新 `updatedAt` |
| DELETE | `/todos/:id` | 删除并联动删除其 subtasks |
| POST | `/todos/:id/subtasks` | 创建子任务；必填 `title` |
| PATCH | `/subtasks/:id` | merge 更新子任务（`title` / `content` / `completed` / `sortOrder`；改 `parentId` → 400） |
| DELETE | `/subtasks/:id` | 删除子任务 |
| GET | `/lists` | 全部清单 `[{id, name, sortOrder, createdAt, updatedAt}]`，按 `(sortOrder, id)`；默认清单 id 1 总在 |
| GET | `/lists/:id` | 单个清单；`:id` 是数字 id 或 URL 编码的清单名 |
| POST | `/lists` | 新建；body 必填 `name`，可选 `sortOrder`（缺省排最后）→ 201 |
| PATCH | `/lists/:id` | 改 `name` / `sortOrder`；内容没变时不刷新 `updatedAt` |
| DELETE | `/lists/:id` | 删除清单：其下待办移到默认清单（刷新 `updatedAt`）并写 `listTombstones`；返回 200 `{deleted, name, movedTodos, movedTo}`；默认清单 → 400 |
| GET | `/images/:name` | 返回图片 bytes，按扩展名识别 Content-Type |
| POST | `/images` | multipart/form-data 上传（字段 `file`），返回 `{name}`；body 上限 32 MiB |
| POST | `/sync` | 手动触发 pull + push；全部成功 200，部分失败 207，返回 `{pull, push, pullError?, pushError?}` |
| POST | `/sync/pull` | 仅从 WebDAV 拉取（手动拉取总是全量下载，不受 ETag 秒级精度影响） |
| POST | `/sync/push` | 仅推送到 WebDAV（`meta.dirty` 未置位时为 no-op） |

todo 相关路径中的 `:id` 既接受完整 i64 id，也接受 `C{seq}` 短码（如 `/todos/C3`），
短码大小写不敏感；todo 响应会附 `seq` 字段。

todo 字段契约（以 `src/model.rs` 的 `TODO_FIELDS` 为准，与 PC 端模型一致）：

| 字段 | 类型 |
|---|---|
| `title` | 非空字符串（必填） |
| `description` | 字符串 \| null（Markdown；图片写 `minitodo-image://<文件名>`） |
| `color` | `#RRGGBB` |
| `quadrant` | 1–4，或 `urgent_important` / `important_not_urgent` / `urgent_not_important` / `not_urgent_not_important` |
| `notifyAt` / `startTime` / `endTime` | `YYYY-MM-DD HH:MM:SS` \| null；也接受 `T`、无秒、时区后缀、仅日期 |
| `notifyBefore` | ≥ 0 的整数（分钟） |
| `notified` / `completed` | bool |
| `sortOrder` | 整数 |
| `repeatEnabled` / `repeatType` / `repeatInterval` / `repeatWeekdays` / `repeatMonthDay` | bool / `daily`\|`weekly`\|`monthly` / ≥ 1 / 如 `"1,3,5"` / 1–31 |
| `listId` | 所属清单的 id（整数 ≥ 1，必须是未删除的清单，否则 400 `清单不存在`）；`POST` 不带时为 1（默认清单）。PATCH 原样写回当前值时不校验，不带时清单不变。旧版本设备写的记录没有 `listId`（未知），响应里也不带 |

别名：`dueDate` → `endTime`（仅日期补 23:59:00）；`priority`（`high`/`medium`/`low`）→ `color`
（#EF4444 / #F59E0B / #10B981，同时给了 `color` 时以 `color` 为准）；`notes` → `description`。
`id` / `createdAt` / `updatedAt` / `listUpdatedAt` / `seq` / `subtasks` 写入时忽略；其它未知字段或类型错误返回 400，错误信息列出允许的字段。

`listUpdatedAt`（契约 §13）是 `listId` 自己的版本，服务端维护、只读，响应里照常返回（清单未知时不带）：`POST` 时 = 新建时刻；
`PATCH` 带了 `listId` 且值变了（含未知 → 已知）时 = `max(now, 旧值 + 1 秒)`；`DELETE /lists/:id` 把待办移进默认清单时同样刷新。
只改其它字段、原样写回 `listId` 时不动；存量"已知但没有 `listUpdatedAt`"的记录不补时间。同步合并时用它比较归属的新旧（见上文）。
响应附派生字段 `priority`（自定义颜色时为 null），派生字段不入库。

排序字段白名单：`dueDate`/`startTime`/`priority`/`quadrant`/`sortOrder`/`updatedAt`/`createdAt`/`title`，
其他字段 fallback 到 `sortOrder asc`。

### 待办清单

清单字段（`todoLists` 的元素，与 PC 模型一致）：

| 字段 | 类型 |
|---|---|
| `name` | 字符串（必填）：去首尾空白，不能为空、不超过 50 个字；与其它清单不区分大小写重名 → 409 `已有同名清单「<已有清单的名称>」` |
| `sortOrder` | 整数；新建不给时 = 现有最大值 + 1 |

`id`（新建时服务端按 `UNIX 毫秒 × 1000 + 随机 0..999` 分配，与 PC 同一算法）/ `createdAt` / `updatedAt` 写入时忽略，
其它字段 400。清单不分配 `C{seq}` 短码。

* `/lists/:id` 的 `:id`：先当数字 id 找；没有这个 id 再按名称（去首尾空白、不区分大小写）精确匹配。
  同名的有多个（两台设备各建了一个，同步不处理重名）→ 409，错误体带 `candidates: [{id, name}]`，改用 id 即可；找不到 → 404
* `GET /todos?listId=<n>` / `?list=<名称>` 按**有效清单**过滤：待办的 `listId` 指向一个未删除的清单时就是它，
  否则（未知 = 缺失或 null、类型不对、清单已被删除）算在默认清单 1 里；响应里的 `listId` 是存储的原值。清单不存在 → 404，
  名称重名 → 409，两个参数都给了却指向不同的清单 → 400
* `DELETE /lists/:id`：单事务里把 `listId` 等于它的待办改到默认清单（刷新 `updatedAt` 与 `listUpdatedAt`，随同步传播），
  删清单，写清单墓碑 `deletedAt = max(now, 清单的 updatedAt)`（永不过期）；默认清单 → 400 `默认清单不能删除`
* 收到别的设备的清单墓碑时不改写待办（读侧兜底显示在默认清单）

```bash
curl -H "Authorization: Bearer $KEY" -H "Content-Type: application/json" \
  -d '{"name":"工作"}' https://minitodo.example.com/lists
# → 201 {"id":1762650000000123,"name":"工作","sortOrder":1,"createdAt":"…","updatedAt":"…"}
curl -H "Authorization: Bearer $KEY" "https://minitodo.example.com/todos?list=%E5%B7%A5%E4%BD%9C&completed=false"
curl -X DELETE -H "Authorization: Bearer $KEY" https://minitodo.example.com/lists/%E5%B7%A5%E4%BD%9C
# → 200 {"deleted":1762650000000123,"name":"工作","movedTodos":3,"movedTo":1}
```

所有响应附 `X-Sync-Status: healthy | stale | offline` 与 `X-Last-Sync-At`；
offline 时还会带 `Warning: 110 "sync offline"`。offline 状态下 API 仍可读写，
push worker 会在 WebDAV 恢复后自动回写。

## AI 自动提醒（openclaw cron）

[`skill/minitodo/`](skill/minitodo/) 既可作为 Claude Code Skill，也可装到
[openclaw](https://github.com/openclaw/openclaw) 的 workspace 让 cron 调度器
定时唤起 agent、由 **agent 自己**判断临期待办并推到 default channel。

> 设计理由：mini-todo 的重复提醒只更新 `notifyAt`、不动 `dueDate`，服务端
> query 又只能按 `dueDate` 筛——客户端硬编码"临期规则"永远会有漏推/误推。
> 把判断交给 agent 看原始 JSON、按 cron prompt 给的窗口自己判断更稳。

> **给 openclaw agent**：完整的安装/配置/cron message 模板/故障排查指南见
> [`openclaw.md`](openclaw.md)（带前置检查、§6 询问用户偏好、§7 cron prompt 模板、卸载步骤）。

输出形如（由 agent 按 prompt 自己组织、每条带 `#C{seq}` 短码便于反馈，
缺 seq 时降级用完整 `#{id}`）：

```
mini-todo 临期提醒｜2026-05-13 08:00
已逾期（1）：
  - #C5 [高] 写报告 (05-12 18:00，已逾期 14 小时)
未来 24h 到期（2）：
  - #C7 [中] 买菜 (05-13 10:00，2 小时后)
  - #C2 [低] 开会 (05-13 19:00，11 小时后)
```

要手动复现这一查询：
```bash
python ~/.openclaw/workspace/skills/minitodo/minitodo.py list --pending --json
```
拿到 JSON 后按"dueDate 优先、缺则 notifyAt"取时间锚，自己挑窗口内的输出即可。

## 开发

```bash
cd cloud
# 单元 / 集成测试（含 mock WebDAV：按 Apache / nginx 实测语义模拟的场景测试）
cargo test
# Lint
cargo clippy --all-targets -- -D warnings
# Format
cargo fmt --check
# Skill CLI 单测（需要 requests）
python3 -m unittest discover -s skill/minitodo -p 'test_*.py'
```

与真实 PC 应用、真实 WebDAV 的端到端验证见 `pc-gpui/scripts/e2e-linux/README.md`。

本子项目独立于 PC 端 `pc-gpui/`，不在同一 Cargo workspace 中，互不影响。
