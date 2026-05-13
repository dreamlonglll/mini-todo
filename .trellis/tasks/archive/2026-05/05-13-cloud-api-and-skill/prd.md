# cloud API + AI skill

## Goal

在 mini-todo monorepo 新增 `cloud/` 子项目：一个 Rust HTTP API + 配套的 Claude Code skill，让 AI 能通过云端读写用户的待办数据。数据流借用 PC 端已有的 WebDAV 同步通道——云端是 WebDAV 客户端，定期拉 + 写回 `sync-data.json.gz`，避免 PC 端再起一个本机服务。

价值：让用户在 PC 之外（任意有 Claude Code 的环境，包括手机、其他电脑、CI）通过 AI 查询、添加、更新自己的待办，而不需要在每台设备装 Tauri 桌面端。

## Requirements

### 云端服务（cloud/）

- Rust + Axum HTTP 服务，rusqlite 持久化，reqwest WebDAV 客户端
- bind 127.0.0.1:8787，systemd 管理进程
- Caddy 反向代理终结 TLS（自动 Let's Encrypt），配置示例随仓库提供
- 单用户、`config.toml` 配置：`webdav_url/user/pass`、`api_key`、`bind`、`timezone`（默认 `Asia/Shanghai`）、`pull_interval`（默认 60s）
- API 鉴权：`Authorization: Bearer {api_key}`，无 token = 401

### 数据流

- `/mini-todo/sync-data.json.gz` 在 WebDAV 上仍是 source of truth
- 云端启动：拉一次 `sync-data.json.gz` + 全量拉 `/mini-todo/images/*` → 写入云端 SQLite + `/var/lib/minitodo/images/`
- 云端 worker：60s 轮询拉 WebDAV，更新本地 SQLite（按 per-record updated_at LWW 合并到本地）
- AI 写云端 → 云端 SQLite UPDATE + `meta.dirty=true` → 后台 worker 1s tick 检测 dirty
  - dirty 时：GET WebDAV（带 `If-None-Match`）→ 把本地 SQLite 当前完整状态序列化为 sync-data → PUT 回 WebDAV（带 `If-Unmodified-Since`）
  - 412 → `meta.dirty=true` 重置 → 下一轮重试
- 图片：AI POST /images → 本地写入 + dirty image 队列 → worker PUT WebDAV

### 本地存储 schema

- `todos(id TEXT PK, data_json TEXT NOT NULL, updated_at TEXT NOT NULL)`
- `subtasks(id TEXT PK, todo_id TEXT NOT NULL, data_json TEXT NOT NULL, updated_at TEXT NOT NULL)`
- `settings(key TEXT PK, value TEXT)`
- `meta(key TEXT PK, value TEXT)` — `dirty`, `last_pull_at`, `last_etag`, `last_modified`, `dirty_images`
- 查询用 SQLite JSON1：`json_extract(data_json, '$.completed')` 等
- 所有 updated_at 用 `chrono::FixedOffset(config.timezone).now().format("%Y-%m-%d %H:%M:%S")`，与 PC SQLite `datetime('now', 'localtime')` 格式完全一致

### REST API

纯 CRUD + 丰富 query string，零聚合端点。

| Method | Path | 说明 |
|---|---|---|
| GET | `/health` | 返回 `{status, sync, lastPullAt}` |
| GET | `/todos` | 列表；query: `completed`, `dueDateBefore`, `dueDateAfter`, `startDate`, `priority`, `quadrant`, `sort`, `limit`, `offset`, `q`, `withSubtasks` |
| GET | `/todos/:id` | 详情；嵌套 subtasks |
| POST | `/todos` | 创建；必填 title；其他字段透传到 data_json |
| PATCH | `/todos/:id` | merge 更新；未提及字段保留（PC v24 新字段也保留） |
| DELETE | `/todos/:id` | 删除（连带 subtasks） |
| POST | `/todos/:id/subtasks` | 创建子任务 |
| PATCH | `/subtasks/:id` | merge 更新子任务 |
| DELETE | `/subtasks/:id` | 删除子任务 |
| GET | `/images/:name` | 返回图片 bytes |
| POST | `/images` | multipart 上传，返回 `{name}` |

所有响应都带：
- `X-Sync-Status: healthy | stale | offline`
- `X-Last-Sync-At: <ISO>`
- WebDAV 失联 > 5 min 加 `Warning: 110 "sync offline"`

### Skill（cloud/skill/minitodo/）

- `SKILL.md`：frontmatter + AI 用法指南（含完整 endpoint 清单 + curl 示例 + 常见任务 → CLI 命令映射）
- `minitodo.py`：Python CLI wrapper，封装 requests，子命令 `today`、`add`、`done`、`list`、`search`、`show`、`update`，`--json` 输出
- `config.example.toml`：endpoint + api_key 模板
- `install.sh` / `install.ps1`：拷贝到 `~/.claude/skills/minitodo/` + 初始化 config

### PC 端配套改动（pc/src-tauri/）

- `services/webdav.rs::upload_bytes(...)` 加可选 `if_unmodified_since: Option<String>` 参数
- `services/webdav.rs::download_bytes(...)` 返回 `(bytes, Option<last_modified_string>)`
- `commands/sync_cmd.rs::webdav_upload_sync` PUT 时带 `If-Unmodified-Since`；412 → 触发 download + per-record merge + 重试
- `commands/sync_cmd.rs::webdav_download_sync` 记录 Last-Modified 到 `webdav_last_modified` setting
- `db/migrations.rs` 新增 v24：插入 `webdav_last_modified` setting key

## Acceptance Criteria

### 云端功能

- [ ] `cargo build --release` 在 `cloud/` 下成功；产出单 binary
- [ ] systemd unit 文件示例与 Caddyfile 示例随仓库提供，文档说明部署步骤
- [ ] `config.toml` 缺少必填字段时启动失败并提示
- [ ] 错误 `api_key` 访问 → 401
- [ ] 正确 `api_key` 访问 `/health` → 200，含 `X-Sync-Status` header
- [ ] 启动后 5 秒内首次拉到 WebDAV 数据，能 `GET /todos` 返回 PC 已有的所有待办

### CRUD

- [ ] `GET /todos?completed=false&priority=high&sort=-dueDate` 正确过滤排序
- [ ] `GET /todos/:id?withSubtasks=true` 嵌套返回子任务
- [ ] `POST /todos {title:"x"}` 创建后立即 `GET` 能看到
- [ ] `PATCH /todos/:id {completed:true}` 是 merge：原 title/priority/未知字段保留
- [ ] `POST /todos/:id/subtasks {title:"y"}` 创建子任务
- [ ] `DELETE /todos/:id` 同时删除其 subtasks

### 同步链路

- [ ] AI 通过 API 创建 todo → 1 秒内 WebDAV 上 `sync-data.json.gz` 已含此 todo
- [ ] PC 手动同步 → 能看到 AI 创建的 todo
- [ ] PC 改 todo title + AI 同时改另一个 todo completed → 两个改动最终都在 WebDAV 上（先到方 PUT 成功，后到方 412 重试 merge 后成功）
- [ ] 关停 WebDAV server → API 仍可读写，response 含 `X-Sync-Status: offline` + Warning；恢复后 dirty 数据自动回写

### 图片

- [ ] `POST /images` multipart 上传成功；后续 `GET /images/:name` 能拿到同样 bytes
- [ ] AI 上传的图片在 60s 内出现在 WebDAV `/mini-todo/images/`
- [ ] 云端启动时 WebDAV 有的图片本地没有 → 下载到 `/var/lib/minitodo/images/`

### Skill

- [ ] `install.sh` 在 Linux/macOS 一键拷贝到 `~/.claude/skills/minitodo/` 并提示编辑 config
- [ ] `install.ps1` 在 Windows 同等
- [ ] `python minitodo.py today --json` 返回今日相关 todos（dueDate=today + startDate=today + overdue）
- [ ] `python minitodo.py add "买菜" --priority high` 创建 todo
- [ ] `python minitodo.py done <id>` 标记完成
- [ ] AI 在 Claude Code 中 invoke skill 后能通过 Bash 调用上述命令

### PC 端

- [ ] PC 端 PUT 带 `If-Unmodified-Since`，能被 412 拒绝
- [ ] PC 端 412 时下载远端 + per-record merge + 重新 PUT
- [ ] 升级 v24 migration 给老用户加 `webdav_last_modified` 默认空值，不破坏现有数据
- [ ] `npm run tauri build` 和 `cargo check` 双双通过

## Definition of Done

- Rust lint (`cargo clippy --all-targets -- -D warnings`) 与 format (`cargo fmt --check`) 通过
- Python skill `minitodo.py` 通过 `python -m py_compile` 与基本 manual smoke test
- `cloud/README.md` 包含部署步骤（systemd unit + Caddyfile + WebDAV server 选型示例）
- `cloud/skill/minitodo/SKILL.md` 包含 AI 用法、endpoint 清单、常见任务示例
- 顶层 `CLAUDE.md` 与 `README.md` 增加 cloud 子项目说明，但保留 PC 端独立可用的描述
- PC 端改动通过现有 npm run build + cargo check
- 不引入新的 Rust 依赖到 `pc/`，除非 cloud 共享 crate 需要

## Technical Approach

### 仓库结构

```
mini-todo/
├── cloud/                              # 新增
│   ├── Cargo.toml                      # axum + tokio + rusqlite + reqwest + chrono + flate2 + serde
│   ├── src/
│   │   ├── main.rs                     # 启动 + tokio runtime
│   │   ├── config.rs                   # config.toml 解析
│   │   ├── api/
│   │   │   ├── mod.rs                  # axum Router
│   │   │   ├── auth.rs                 # Bearer middleware
│   │   │   ├── todos.rs                # /todos CRUD
│   │   │   ├── subtasks.rs             # /subtasks CRUD
│   │   │   ├── images.rs               # /images
│   │   │   ├── health.rs               # /health
│   │   │   └── headers.rs              # X-Sync-Status 注入
│   │   ├── db/
│   │   │   ├── mod.rs
│   │   │   ├── schema.rs               # 4 张表 CREATE
│   │   │   └── repo.rs                 # CRUD + JSON1 查询封装
│   │   ├── sync/
│   │   │   ├── mod.rs                  # worker 主循环
│   │   │   ├── webdav.rs               # reqwest WebDAV 客户端（条件 PUT、ETag、Last-Modified）
│   │   │   ├── pull.rs                 # 60s tick: GET → merge into SQLite
│   │   │   ├── push.rs                 # 1s tick: dirty → merge + conditional PUT
│   │   │   └── images.rs               # 图片镜像 worker
│   │   └── time.rs                     # FixedOffset 时间格式化
│   ├── config.example.toml
│   ├── deploy/
│   │   ├── minitodo-cloud.service      # systemd unit
│   │   └── Caddyfile.example
│   ├── skill/minitodo/
│   │   ├── SKILL.md
│   │   ├── minitodo.py
│   │   ├── config.example.toml
│   │   ├── install.sh
│   │   └── install.ps1
│   └── README.md
└── pc/                                 # 现有，仅小幅改动
    └── src-tauri/src/
        ├── services/webdav.rs          # upload_bytes 加 if_unmodified_since 参数
        ├── commands/sync_cmd.rs        # 412 重试逻辑 + 记录 Last-Modified
        └── db/migrations.rs            # v24 加 webdav_last_modified setting
```

### Push worker 关键流程

```
loop {
  sleep 1s
  if meta.dirty != true: continue
  swap dirty = false (CAS-style, 失败重试)

  let (remote_bytes, remote_lastmod) = webdav.get("sync-data.json.gz")
  let remote_data: SyncData = decompress + parse

  // 把云端 SQLite 当前完整状态合并进 remote_data（per-record LWW）
  let merged = merge(remote_data, sqlite.snapshot())
  let payload = compress(serde_json(merged))

  match webdav.put("sync-data.json.gz", payload, if_unmodified_since=remote_lastmod) {
    Ok(_) => meta.last_pull_at = now; meta.last_modified = ...,
    Err(412) => meta.dirty = true,                  // 别人刚写过，下轮重试
    Err(other) => meta.dirty = true; meta.sync_offline_since = now,
  }
}
```

### Pull worker

```
loop {
  sleep 60s
  let (remote_bytes, remote_lastmod) = webdav.get("sync-data.json.gz",
                                                  if_none_match=meta.last_etag)
  if 304: continue
  let remote_data = decompress + parse

  // per-record LWW 合并进 SQLite（云端 SQLite 内 updated_at 比 remote 大的不改）
  for todo in remote_data.todos:
    let local = sqlite.get_todo(todo.id)
    if local is None or local.updated_at < todo.updated_at:
      sqlite.upsert(todo)
  // 同上 subtasks/settings

  meta.last_pull_at = now
  meta.last_modified = remote_lastmod
  meta.last_etag = remote_etag

  // 触发图片差异下载（异步）
  spawn image_sync(remote_data.images)
}
```

### PC 端 412 重试

`webdav_upload_sync` 修改后：
1. PUT 时若 `webdav_last_modified` 非空，附 `If-Unmodified-Since`
2. 收到 412 → 调用 `webdav_download_sync` 拿最新远端
3. 把本地 todos/subtasks 与远端做 per-record LWW merge（**新逻辑**，目前是整包覆盖）
4. 用 merge 结果重新 PUT（带新的 If-Unmodified-Since）
5. 最多重试 3 次

> 注意：第 3 步的 per-record merge 是 PC 端**新增**的合并语义。原有 `webdav_apply_remote` 是 `import_data_raw` 全量替换，需要新增一个 `merge_remote_into_local` 函数。这是 PC 端最大的改动。

## Decision (ADR-lite)

### 1. 云端 vs 本地 sidecar

**Context**: AI 通过 HTTP API 访问待办数据，可以做成本机 sidecar 或真云端服务。

**Decision**: 真云端 VPS（独立部署），数据来自 WebDAV。

**Consequences**:
- 优点：跨设备/跨网络的 AI 都能用、PC 端无需在线、与 WebDAV 同步通道天然兼容
- 缺点：要付 VPS、要处理三方写并发（PC / 云 / WebDAV）

### 2. WebDAV 访问方式

**Context**: 云端可走 HTTP WebDAV，也可在同机时直接读 WebDAV server 的文件目录。

**Decision**: MVP 只走 HTTP WebDAV，同机也走 localhost。

**Consequences**:
- 优点：一套并发锁机制（If-Unmodified-Since）、代码可复用、第三方 WebDAV（坚果云等）后续也能接
- 缺点：同机部署有 5-20ms 的额外开销，未来高 QPS 时可能要重做 filesystem fast path

### 3. 写冲突策略

**Context**: PC 和云端都会写 WebDAV `sync-data.json.gz`，整包覆盖会丢数据。

**Decision**: 两端都改成条件 PUT（If-Unmodified-Since），412 重试，本地 SQLite/数据库当真理来源。PC 端原"整包覆盖"语义改为 per-record LWW merge。

**Consequences**:
- 优点：大多数 race 自愈，最终一致
- 缺点：PC 端违反"零改动"原则，要新增 merge 函数 + migration v24；同记录同字段同时改仍可能丢一方（mini-todo 单用户场景概率极低，接受）

### 4. 本地存储 schema

**Context**: 云端 SQLite 是 PC schema 的完整复刻还是简化 KV。

**Decision**: SQLite KV-style（id, data_json, updated_at）+ SQLite JSON1 查询。

**Consequences**:
- 优点：PC schema 变化（v24/v25 加字段）云端不需要改代码；data_json 原样 round-trip
- 缺点：JSON 查询性能比列式低（mini-todo 数据规模可忽略）；写复杂 join 不便

### 5. API 风格

**Context**: REST CRUD / RPC / GraphQL，是否加聚合端点。

**Decision**: 纯 REST CRUD + 丰富 query string，零聚合端点。子任务 list 不嵌套（含 subtaskCount），detail 嵌套，可 `?withSubtasks=true`。

**Consequences**:
- 优点：标准、可用 curl 调试、SKILL.md 容易写
- 缺点：AI 复杂查询要拼参数；token 占用比聚合端点略高（接受）

### 6. Skill 形态

**Context**: SKILL.md 纯文本 / SKILL.md + Python CLI / Rust CLI / MCP server。

**Decision**: SKILL.md + Python `minitodo.py` CLI wrapper。

**Consequences**:
- 优点：AI 调用 token 少、错误处理集中、Python 跨平台
- 缺点：用户机器要有 Python 3 + requests；将来想兼容非 Claude Code 客户端要重写

## Out of Scope（MVP 不做）

- ❌ Prometheus / OpenTelemetry / metrics 端点
- ❌ 多 API Key / token 轮换 / 审计日志
- ❌ CORS（默认按 Claude Code CLI 客户端，无浏览器跨域需求）
- ❌ Web 控制台 / 管理界面
- ❌ Docker 镜像 / docker-compose（仅 systemd unit + Caddyfile 示例）
- ❌ Filesystem fast path（同机绕过 HTTP）
- ❌ MCP server 形态（保留可能性，但 MVP 不做）
- ❌ 字段级时间戳 / CRDT 类细粒度合并
- ❌ PC 端"云端增强"UI 开关
- ❌ 多设备拓扑支持（仅单 PC + AI）
- ❌ 端到端加密（数据在 WebDAV 上不加密，依赖 WebDAV server 的 https + auth）
- ❌ 移动端
- ❌ 自动备份云端 SQLite（依靠 WebDAV 作 source of truth，云端 SQLite 是缓存）

## Technical Notes

### grill-me 16 轮交互的关键洞察

1. **WebDAV 是 source of truth，云端 SQLite 是缓存**——重启从 WebDAV 重建，不需要云端备份
2. **dirty flag + 后台 worker 1s tick** 比 debounce / 同步回写更鲁棒，单 PUT 在飞
3. **If-Unmodified-Since 防的是云端自己的多 PUT race，PC 无条件 PUT 防不了**——因此 PC 也要加条件 PUT
4. **per-record LWW merge** 是核心合并算法（不再是整包覆盖）
5. **时区**：PC 用 `datetime('now', 'localtime')` 不带时区后缀，云端必须用 `chrono::FixedOffset` 模拟才能保证字符串比较正确
6. **图片**：PC 现状是全量镜像（webdav_upload_sync 会上传所有图片），云端必须对齐
7. **schema 漂移**：PC 加新字段时云端不知情；用 KV-style + PATCH merge 保留未知字段
8. **PC 端 WebDAV 凭据存在 settings 表，不被 sync_data 同步**——云端必须独立 config.toml 配 WebDAV

### 关联文件清单（实现时参考）

| 文件 | 作用 |
|---|---|
| `pc/src-tauri/src/services/webdav.rs` | 现有 WebDAV 客户端，云端可借鉴；PC 端要改 upload_bytes/download_bytes 签名 |
| `pc/src-tauri/src/commands/sync_cmd.rs` | SyncData 结构 / 上传下载流程，PC 端要加 412 重试和 per-record merge |
| `pc/src-tauri/src/commands/data.rs` | export_data_internal / import_data_raw，云端 bootstrap 时可参考 SyncData 解析逻辑 |
| `pc/src-tauri/src/db/migrations.rs` | 当前 v23，PC 端要新增 v24 |
| `pc/src-tauri/src/db/models.rs` | todos/subtasks 字段定义，云端 schema 校验时参考 |

### Research References

无需 trellis-research：架构选型在 grill-me 阶段已通过 16 轮 AskUserQuestion 充分对齐，本任务进入实现阶段不需要再开放设计空间。

## Implementation Plan

### PR1 — cloud 骨架（只读可用）

**目标**：能起服、能 `GET /health`、能从 WebDAV 拉到现有数据进 SQLite。

- `cloud/Cargo.toml` 初始化（axum + tokio + rusqlite + reqwest + chrono + flate2 + serde + toml + tracing）
- `cloud/src/main.rs` + `config.rs`（解析 config.toml，缺字段 panic）
- `cloud/src/db/{schema,repo}.rs`（4 张表 CREATE + JSON1 查询封装）
- `cloud/src/sync/webdav.rs`（reqwest WebDAV 客户端：GET/PUT/PROPFIND，支持 If-None-Match/If-Unmodified-Since/Last-Modified）
- `cloud/src/sync/pull.rs`（60s tick：GET → 解压 → per-record LWW merge 进 SQLite）
- `cloud/src/sync/images.rs`（启动时拉差异图片，存 `/var/lib/minitodo/images/`）
- `cloud/src/api/{auth,health,headers}.rs`（Bearer middleware、`/health`、X-Sync-Status 注入）
- `cloud/src/time.rs`（FixedOffset 时间格式化）
- `cloud/config.example.toml`、`cloud/deploy/minitodo-cloud.service`、`cloud/deploy/Caddyfile.example`
- `cloud/README.md`（部署步骤 + WebDAV server 选型示例 nginx-dav / Caddy webdav）

**验收**：
- `cargo build --release` 通过
- 起服 5 秒后 `curl -H "Authorization: Bearer xxx" localhost:8787/health` 返回 `{status:"healthy", sync:"healthy", lastPullAt:"..."}`
- 直接 `sqlite3 /var/lib/minitodo/data.db "SELECT count(*) FROM todos"` 能看到 PC 已有的所有 todos

### PR2 — cloud REST + AI 链路完整

**目标**：AI 通过 skill 读写，PC 手动同步能看到 AI 的改动（race 可能丢字段，留给 PR3 解决）。

- `cloud/src/api/todos.rs`（GET list + filter + sort + pagination、GET detail、POST/PATCH/DELETE）
- `cloud/src/api/subtasks.rs`（POST 嵌于 todo、PATCH/DELETE 独立）
- `cloud/src/api/images.rs`（GET bytes + POST multipart）
- `cloud/src/sync/push.rs`（1s tick：dirty → GET → merge → 条件 PUT，412 重试）
- `cloud/skill/minitodo/SKILL.md`（AI 用法 + endpoint 清单 + curl 示例）
- `cloud/skill/minitodo/minitodo.py`（CLI: today/add/done/list/search/show/update，--json）
- `cloud/skill/minitodo/{config.example.toml,install.sh,install.ps1}`
- 顶层 `CLAUDE.md` 与 `README.md` 加 cloud 子项目说明

**验收**：
- AI 通过 `python minitodo.py today --json` 列出今日相关 todos
- `python minitodo.py add "买菜" --priority high` 创建后 1 秒内 WebDAV 上能看到
- PC 手动同步后能看到 AI 创建的 todo

### PR3 — PC 端 race 修复

**目标**：PC 与 AI 同时改不同 todo 的不同字段，不再丢失。

- `pc/src-tauri/src/services/webdav.rs::upload_bytes(...)` 加 `if_unmodified_since: Option<String>` 参数
- `pc/src-tauri/src/services/webdav.rs::download_bytes(...)` 改返回 `(Vec<u8>, Option<String>)`（含 Last-Modified）
- `pc/src-tauri/src/commands/sync_cmd.rs::webdav_upload_sync` PUT 时带 If-Unmodified-Since；412 → download_bytes + per-record merge + 重试（最多 3 次）
- `pc/src-tauri/src/commands/sync_cmd.rs` 新增 `merge_remote_into_local` 函数（per-record LWW，替代原 import_data_raw 全量覆盖语义，仅用于冲突恢复路径）
- `pc/src-tauri/src/commands/sync_cmd.rs::webdav_download_sync` 把 Last-Modified 存进 setting `webdav_last_modified`
- `pc/src-tauri/src/db/migrations.rs` 新增 v24：插入 `webdav_last_modified` 默认空字符串

**验收**：
- PC 与云端同时写不同 todo → 都保留
- `npm run tauri build` 与 `cargo check` 通过
- 老用户升级后 v24 migration 不破坏现有数据

## Open Questions

(none — 架构与交付节奏均已对齐，可进入实现阶段)
