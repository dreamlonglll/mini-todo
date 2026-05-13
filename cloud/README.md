# minitodo-cloud

mini-todo 的云端 HTTP API（Rust + Axum），与 PC 端共用同一个 WebDAV
`sync-data.json.gz` 做底层数据通道。AI 客户端（Claude Code Skill / 任意带
HTTP 客户端的工具）通过本服务读写用户待办，数据最终回写 WebDAV，PC 端下次
同步拉到。

> 当前进度：**PR1 + PR2 + PR3 全部完成**——只读骨架、REST CRUD、dirty flag push、
> PC 端条件 PUT + 412 重试 + per-record LWW merge、配套 Claude Code / openclaw
> Skill（含 openclaw cron 临期提醒）全部就位。

## 总览

```
┌────────┐  HTTPS+Bearer   ┌──────────────┐ pull 60s   ┌─────────┐
│   AI   │ ──────────────► │ minitodo-    │ ─────────► │ WebDAV  │
│ Skill  │ ◄────────────── │ cloud (本服) │ ◄───────── │ server  │
└────────┘    JSON / files └──────┬───────┘ push 1s    └────┬────┘
                                  │ rusqlite                │
                                  ▼                         ▼
                          /var/lib/minitodo/         /mini-todo/
                          data.db + images/          sync-data.json.gz
                                                     images/
```

* **WebDAV 是 source of truth**。云端 SQLite 是缓存，重启会重新从 WebDAV 灌满。
* **时间格式与 PC SQLite 完全一致**：`YYYY-MM-DD HH:MM:SS` 无时区后缀，按
  config.toml `timezone` 取墙钟时间。
* **per-record LWW**：远端 record.updated_at ≥ 本地 → upsert；本地比远端新 → 保留。

## 构建

```bash
cd cloud
cargo build --release
# 产物：target/release/minitodo-cloud
```

无需任何 C 工具链（rusqlite bundled、reqwest rustls）。

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

输出示例：

```json
{"status":"healthy","sync":"healthy","lastPullAt":"2026-05-13 12:34:56"}
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
| `api_key` | ✓ | — | Bearer Token；≥ 16 字符 |
| `bind` | × | `127.0.0.1:8787` | HTTP 监听地址 |
| `timezone` | × | `Asia/Shanghai` | IANA 时区，**必须与 PC 端一致** |
| `pull_interval` | × | `60` | Pull worker 间隔（秒） |
| `data_dir` | × | `/var/lib/minitodo` | SQLite 与 meta 数据目录 |
| `images_dir` | × | `/var/lib/minitodo/images` | 镜像图片目录 |

缺任意必填字段 → 进程启动直接退出并打印清晰错误。

## 当前能力

服务端：

- [x] `GET /health` 返回 `{status, sync, lastPullAt}` 与 `X-Sync-Status` header
- [x] Bearer token 鉴权（错/缺 → 401）
- [x] 启动同步拉一次 WebDAV `sync-data.json.gz`，per-record LWW merge 进 SQLite
- [x] 60s 后台 pull 轮询（自动用 `If-None-Match` 跳过 304）
- [x] 启动时一次性图片镜像（缺什么下什么）
- [x] `/todos` `/subtasks` `/images` REST CRUD（含 filter / sort / pagination / merge PATCH / cascade DELETE）
- [x] 1s 后台 push worker：检查 `meta.dirty` → per-record LWW merge → 条件 PUT 回 WebDAV，412 重试
- [x] 软删除墓碑：DELETE 写 `tombstones` 表，push merge 时拦截远端复活已删除的 record
- [x] 图片 push 队列：POST /images 后 ≤1s PUT 到 `/mini-todo/images/`

PC 端协同（PR3）：

- [x] WebDAV 条件 PUT（`If-Unmodified-Since`）+ 412 重试 + per-record LWW merge
- [x] v24 migration 新增 `webdav_last_modified` setting

Skill / AI 集成：

- [x] Claude Code Skill（`cloud/skill/minitodo/`，含 Python CLI、install 脚本、SKILL.md）
- [x] CLI 覆盖 today / list / add / done / search / show / update / delete / health
- [x] 同一份 skill 可装到 openclaw workspace（`install.sh --target openclaw`）
- [x] openclaw cron 临期提醒：cron 唤起 agent 后由 **agent 自己**拉 `list --pending --json` + 判断哪些该推 + 组织格式，`--announce` 推到 default channel

## REST API 速查

所有请求都需要 `Authorization: Bearer <api_key>`。

| Method | Path | 说明 |
|---|---|---|
| GET | `/health` | `{status, sync, lastPullAt}` |
| GET | `/todos` | 列表。query：`completed=true/false`, `priority=high/medium/low`, `quadrant=1..4` 或 `urgent_important` 等别名, `dueDateBefore`, `dueDateAfter`, `startDate=YYYY-MM-DD`, `q=<keyword>`, `sort=[+-]<field>`, `limit`, `offset`, `withSubtasks=true` |
| GET | `/todos/:id?withSubtasks=true` | 详情；默认嵌套 subtasks，`withSubtasks=false` 扁平化 |
| POST | `/todos` | 创建；body 必填 `title`；其他字段（priority/dueDate/quadrant/color/...）透传 |
| PATCH | `/todos/:id` | merge 更新；未提及字段保留，含 PC v24/v25 加的未知字段 |
| DELETE | `/todos/:id` | 删除并联动删除其 subtasks |
| POST | `/todos/:id/subtasks` | 创建子任务；必填 `title` |
| PATCH | `/subtasks/:id` | merge 更新子任务 |
| DELETE | `/subtasks/:id` | 删除子任务 |
| GET | `/images/:name` | 返回图片 bytes，按扩展名识别 Content-Type |
| POST | `/images` | multipart/form-data 上传（字段 `file`），返回 `{name}` |

排序字段白名单：`dueDate`/`startTime`/`priority`/`quadrant`/`sortOrder`/`updatedAt`/`createdAt`/`title`，
其他字段 fallback 到 `sortOrder asc`。

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

输出形如（由 agent 按 prompt 自己组织、每条带 `#{id}` 便于反馈）：

```
mini-todo 临期提醒｜2026-05-13 08:00
已逾期（1）：
  - #1734567890123456 [高] 写报告 (05-12 18:00，已逾期 14 小时)
未来 24h 到期（2）：
  - #1734567890123457 [中] 买菜 (05-13 10:00，2 小时后)
  - #1734567890123458 [低] 开会 (05-13 19:00，11 小时后)
```

要手动复现这一查询：
```bash
python ~/.openclaw/workspace/skills/minitodo/minitodo.py list --pending --json
```
拿到 JSON 后按"dueDate 优先、缺则 notifyAt"取时间锚，自己挑窗口内的输出即可。

## 开发

```bash
cd cloud
# 单元测试
cargo test
# Lint
cargo clippy --all-targets
# Format
cargo fmt --check
```

本子项目独立于 `pc/`，不在同一 Cargo workspace 中，互不影响。
