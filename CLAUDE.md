# Mini-Todo 项目指南

本文档用于介绍 Mini-Todo 项目，帮助 AI 助手快速了解项目结构和开发规范。

## 项目简介

Mini-Todo 是一款基于 **Tauri 2.x + Vue 3 + TypeScript** 开发的 Windows 桌面待办事项管理应用，定位为简洁、聚焦的本地待办工具，支持子任务、四象限、日历、重复提醒、系统通知与 WebDAV 云同步。

仓库按平台拆子目录，`pc/` 是 Windows 桌面端，`cloud/` 是云端 HTTP API（mini-todo 在远程 VPS 上的复刻，通过同一个 WebDAV 通道与 PC 同步数据，供 AI / Claude Code Skill 通过 REST 读写）。详见 `cloud/README.md`。

## 技术栈

| 层级 | 技术选型 | 说明 |
|------|----------|------|
| 前端框架 | Vue 3 + TypeScript | 组合式 API，类型安全 |
| UI 组件库 | Element Plus | 包含图标库 @element-plus/icons-vue |
| 富文本编辑 | Milkdown | 子任务 Markdown 编辑器 |
| 状态管理 | Pinia | Vue 官方推荐状态管理 |
| 桌面框架 | Tauri 2.x | 轻量级跨平台桌面框架 |
| 后端语言 | Rust | 高性能，内存安全 |
| 数据库 | SQLite (rusqlite) | 轻量级本地数据库 |
| 拖拽功能 | vuedraggable | Vue 拖拽排序库 |
| 异步运行时 | Tokio | Rust 异步任务调度 |
| WebDAV 客户端 | reqwest | 远端备份与同步 |

## 项目结构

> 项目按平台拆分子目录。`pc/` 为 Windows 桌面端（Tauri），未来可平行加入 `mobile/`、`web/`。

```
mini-todo/
├── docs/                                   # 共享文档（跨平台）
│   └── 开发文档/                            # 开发相关文档
├── pc/                                     # PC 端（Tauri 2.x + Vue 3）
│   ├── src/                                # Vue 前端源码
│   │   ├── assets/                         # 静态资源
│   │   ├── components/                     # Vue 组件
│   │   │   ├── CalendarView.vue            # 日历视图
│   │   │   ├── QuadrantView.vue            # 四象限视图
│   │   │   ├── SettingsPanel.vue           # 设置面板
│   │   │   ├── TitleBar.vue                # 标题栏
│   │   │   ├── TodoItem.vue                # 待办项组件
│   │   │   └── TodoList.vue                # 待办列表
│   │   ├── router/                         # 路由配置
│   │   ├── stores/                         # Pinia 状态管理
│   │   │   ├── appStore.ts                 # 应用全局状态
│   │   │   └── todoStore.ts                # 待办状态
│   │   ├── types/                          # TypeScript 类型定义
│   │   │   ├── app.ts                      # 应用类型
│   │   │   └── todo.ts                     # 待办类型
│   │   ├── utils/                          # 工具函数
│   │   │   ├── fileLink.ts                 # 文件链接处理
│   │   │   ├── holiday.ts                  # 节假日工具
│   │   │   └── lunar.ts                    # 农历工具
│   │   ├── views/                          # 页面视图
│   │   │   ├── CompletedView.vue           # 已完成视图
│   │   │   ├── EditorView.vue              # 待办编辑主视图
│   │   │   ├── MainView.vue                # 主视图（待办列表）
│   │   │   ├── SubtaskEditorView.vue       # 子任务编辑视图（独立 WebView）
│   │   │   ├── NotificationView.vue        # 通知视图
│   │   │   └── SettingsView.vue            # 设置视图
│   │   ├── App.vue                         # 根组件
│   │   └── main.ts                         # 入口文件
│   ├── src-tauri/                          # Tauri/Rust 后端源码
│   │   ├── src/
│   │   │   ├── commands/                   # Tauri 命令（前后端桥接）
│   │   │   │   ├── data.rs                 # 数据导入导出
│   │   │   │   ├── holiday.rs              # 节假日命令
│   │   │   │   ├── notification_cmd.rs     # 通知命令
│   │   │   │   ├── settings_cmd.rs         # 设置命令
│   │   │   │   ├── sync_cmd.rs             # WebDAV 同步命令
│   │   │   │   ├── todo.rs                 # 待办 CRUD 命令
│   │   │   │   └── window.rs               # 窗口管理命令
│   │   │   ├── db/                         # 数据库层
│   │   │   │   ├── connection.rs           # 数据库连接管理
│   │   │   │   ├── migrations.rs           # 数据库迁移（v1~v24）
│   │   │   │   └── models.rs               # 数据模型定义
│   │   │   ├── services/                   # 业务服务层
│   │   │   │   ├── notification.rs         # 通知服务（含定时调度）
│   │   │   │   └── webdav.rs               # WebDAV 同步客户端
│   │   │   ├── lib.rs                      # 库入口
│   │   │   └── main.rs                     # 主入口
│   │   ├── Cargo.toml                      # Rust 依赖配置
│   │   └── tauri.conf.json                 # Tauri 配置
│   ├── public/                             # 公共静态资源
│   ├── index.html                          # Vite 入口 HTML
│   ├── package.json                        # Node 依赖配置
│   ├── tsconfig.json                       # TypeScript 配置
│   └── vite.config.ts                      # Vite 构建配置
├── cloud/                                  # 云端 HTTP API（独立 Rust crate）
│   ├── Cargo.toml                          # axum + tokio + rusqlite + reqwest + ...
│   ├── config.example.toml                 # 配置示例
│   ├── README.md                           # 部署 / 配置说明
│   ├── deploy/
│   │   ├── minitodo-cloud.service          # systemd unit 示例
│   │   └── Caddyfile.example               # Caddy 反代示例
│   └── src/
│       ├── main.rs                         # tokio + axum 启动
│       ├── config.rs                       # config.toml 解析
│       ├── time.rs                         # 与 PC SQLite 一致的时间格式
│       ├── db/                             # rusqlite + KV-style schema
│       ├── sync/                           # WebDAV 客户端 + pull worker + 图片镜像
│       └── api/                            # axum router + Bearer auth + /health
├── CLAUDE.md                               # 项目指南（本文档）
├── README.md
└── AGENTS.md
```

## 核心功能

### 待办管理
- 创建、编辑、删除待办事项
- 支持一级子任务（含 Markdown 详情、图片上传）
- 四象限分类（重要紧急 / 重要不紧急 / 紧急不重要 / 不紧急不重要）
- 自定义颜色标识
- 完成状态标记
- 拖拽排序
- 开始/截止时间

### 子任务
- 标题 + Markdown 内容（Milkdown 富文本编辑器）
- 支持图片粘贴/拖入上传
- 独立 WebView 详情窗口（编辑/查看双模式）
- 完成态切换、排序

### 通知提醒
- Windows 系统通知 / 应用内通知
- 预设提前提醒（5/15/30 分钟）
- 自定义提前时间

### 重复提醒
- 按天 / 周 / 周几 / 月几号循环
- 触发后自动推进到下一次提醒时间点
- 应用启动时补发错过的重复提醒

### 视图模式
- **列表视图**：按排序/优先级展示
- **四象限视图**：拖拽分类
- **日历视图**：按日期展示（含农历、节假日）

### 数据导入导出
- 导出版本：`4.0`（位于 `pc/src-tauri/src/commands/data.rs`）
- 导出为 ZIP 压缩包（内含 `data.json`）
- 导入兼容 v3.0 和 v4.0 两个版本
  - v3.0 是历史导出格式（含已移除的 AI Agent 字段），新版本反序列化时通过 `#[serde(default)]` 静默忽略多余字段
- 直接 JSON 导入也支持

### WebDAV 云同步
- 双向同步（上传/下载，含冲突检测）
- 同步范围：todos / subtasks / 部分应用设置 / 用户上传的图片
- 自动同步可选（按间隔轮询）
- 加密、压缩传输（gzip）

### 窗口特殊功能
- **普通模式**：浅色主题，可拖拽移动
- **固定模式**：
  - 透明背景
  - 固定在用户指定位置
  - 忽略 Win+D（显示桌面）
  - 禁用关闭、最小化、拖拽

## 开发规范

### UI 设计规范
- **组件库**：Element Plus
- **图标库**：Element Plus Icons（@element-plus/icons-vue）
- **禁止使用 emoji 图标**
- 设计理念：简洁现代、去除卡片边框、极简列表

#### 优先级颜色
| 级别 | 颜色代码 | 描述 |
|------|----------|------|
| 高 | #EF4444 (红色) | 紧急重要任务 |
| 中 | #F59E0B (橙色) | 一般重要任务 |
| 低 | #10B981 (绿色) | 不紧急任务 |

### 数据库设计
- **数据库类型**：SQLite
- **存储位置**：`%APPDATA%/mini-todo/data.db`
- **迁移版本**：当前 v1~v24，通过 `pc/src-tauri/src/db/migrations.rs` 管理
  - v23：移除所有 AI Agent / 任务调度 / 工作流相关表和字段（详见迁移注释）
  - v24：新增 `webdav_last_modified` settings key，用于条件 PUT

#### 主要数据表

| 表名 | 说明 |
|------|------|
| `todos` | 待办事项（含重复提醒字段） |
| `subtasks` | 子任务（标题 + Markdown 内容 + 完成态） |
| `settings` | 应用设置（键值对） |
| `screen_configs` | 屏幕配置 |
| `migrations` | 迁移版本记录 |

### 数据导入导出与同步

> **重要**：当数据库结构变更（新增表/字段/设置项）时，必须同步更新导入导出和 WebDAV 同步功能！

- **导出版本号**：当前 `4.0`（位于 `pc/src-tauri/src/commands/data.rs`）
- **关键文件**：
  - 模型定义：`pc/src-tauri/src/db/models.rs` → `ExportData`、`AppSettings`
  - 导入导出逻辑：`pc/src-tauri/src/commands/data.rs` → `export_data_internal`、`import_data_raw`
  - WebDAV 同步：`pc/src-tauri/src/commands/sync_cmd.rs` → `SyncData`、`webdav_upload_sync`、`webdav_apply_remote`
  - 前端类型：`pc/src/types/todo.ts` → `ExportData`（前端只传递 JSON 字符串，无需严格同步）

#### 导入导出架构说明

- `export_data`（手动导出 Tauri 命令）和 WebDAV 上传**共用** `export_data_internal()` 函数
- `import_data`（手动导入 Tauri 命令）和 WebDAV 下载应用**共用** `import_data_raw()` 函数
- `SyncData` 结构将 `ExportData` 的各字段以 `serde_json::Value` 形式传输，不要遗漏新字段

#### 当前同步覆盖范围

| 数据 | 是否同步 | 说明 |
|------|---------|------|
| `todos`（全字段） | 是 | 含重复提醒字段 |
| `subtasks`（全字段） | 是 | 标题 + Markdown 内容 + 完成态 |
| `settings`（部分） | 是 | 8 个应用设置项，不含 WebDAV 配置 |
| `images`（文件） | 是 | 通过 WebDAV 独立上传/下载 |
| `screen_configs` | 否 | 设备特定的屏幕配置 |
| `migrations` | 否 | 结构性表，应用启动自动管理 |

#### 向后兼容

- 旧 v3.0 备份内的 `agent_configs` / `workflow_steps` / `task_dependencies` / `prompt_templates` /
  `agent_executions` 等字段，以及 todo/subtask 上的 agent/调度/工作流字段，在 v4.0 反序列化时
  通过 serde 的"未知字段忽略"机制自动跳过，不会报错。

#### 维护检查清单

当新增数据库迁移时，请检查：
1. 新增的 **settings 键值** 是否已加入 `AppSettings` 结构体（含 `#[serde(default)]`）
2. `read_app_settings` 函数是否读取了新设置
3. `write_app_settings` 函数是否写入了新设置
4. 新增的 **数据表** 是否需要纳入 `ExportData` 和 `SyncData`
5. `export_data_internal` 是否查询了新表数据
6. `import_data_raw` 是否导入了新表数据
7. `SyncData` 是否新增了对应字段（含 `#[serde(default)]`）
8. `webdav_upload_sync` 是否从导出 JSON 中提取了新字段（注意 camelCase 键名）
9. `webdav_apply_remote` 和 `webdav_auto_sync` 构建的 import_json 是否包含新字段
10. `check_local_changes` 是否检测了新表的变更
11. 旧版数据导入的兼容性（新字段必须有 `#[serde(default)]`）
12. 导出版本号是否需要递增

## 核心架构概念

### 通知调度流程

```
NotificationService::start_scheduler() (后台线程，每分钟 tick)
  ├── 扫描 todos.notify_at <= now AND not notified
  ├── 触发系统/应用通知
  ├── 重复提醒：计算下一次提醒时间，重置 notified = 0
  └── 非重复：notified = 1
```

### 子任务编辑流程

```
TodoItem / EditorView
  └── 点击编辑/查看
       └── 打开独立 WebView：/subtask-editor?id={subtaskId}&mode={view|edit}
            └── SubtaskEditorView 使用 Milkdown 加载 Markdown
                 └── 图片粘贴 → save_subtask_image → 本地 images 目录
```

### WebDAV 同步流程

```
SyncSettings (远端配置 + device_id + last_sync_at)
  ├── webdav_upload_sync     → export_data_internal → gzip → PUT /sync-data.json.gz
  ├── webdav_download_sync   → GET → gunzip → SyncData，判断冲突
  ├── webdav_apply_remote    → SyncData → import_data_raw
  └── webdav_auto_sync       → 按 sync_interval 轮询，自动 upload 或 apply
```

## cloud/ 子项目（云端 API）

> 当前状态：**PR1 + PR2 + PR3 全部完成**——只读骨架、REST CRUD、dirty push worker、Skill、
> PC 端条件 PUT + 412 重试 + per-record merge + v24 migration 均在位。
> 详见 `cloud/README.md`、`.trellis/tasks/05-13-cloud-api-and-skill/prd.md`。

`cloud/` 是一个独立的 Rust crate（**不在** `pc/` 的 Cargo workspace 里），部署到 VPS 上为
AI / Claude Code Skill 提供 mini-todo 数据的 HTTP REST 访问能力。两端的契约：

- **共用同一个 WebDAV `sync-data.json.gz` 通道**。云端是 WebDAV 客户端，不依赖 PC 在线
- **WebDAV 是 source of truth**，云端 SQLite 是缓存，重启会重新从 WebDAV 灌满
- **时间格式与 PC SQLite 完全一致**：`YYYY-MM-DD HH:MM:SS` 无时区后缀，按 `config.toml` 的 `timezone` 取墙钟时间
- **写冲突策略**：两端都走条件 PUT（`If-Unmodified-Since`），冲突时 per-record LWW merge 后重试（PR3 把 PC 端的"整包覆盖"语义改成"merge"）
- **schema 漂移宽容**：云端 SQLite 是 KV-style（`todos(id, data_json, updated_at)`），PC 加新字段时云端不需要改代码，列表/过滤用 SQLite JSON1 `json_extract`
- **软删除墓碑**：DELETE 时写 `tombstones(entity_type, entity_id, deleted_at)`，push merge
  时拦截远端"复活"已删除的 record；7 天后清理

```
cloud/
├── src/main.rs               # 启动序列：load config → open SQLite → pull_once → spawn pull/push/image worker → axum
├── src/config.rs             # config.toml；缺必填字段直接 panic 退出
├── src/time.rs               # FixedOffset 模拟"本地时区现在"
├── src/db/                   # rusqlite + 5 张表（todos / subtasks / settings / meta / tombstones）
├── src/sync/webdav.rs        # GET（If-None-Match）/ PUT（If-Unmodified-Since）/ PROPFIND
├── src/sync/pull.rs          # 60s 轮询：GET → gunzip → per-record LWW merge
├── src/sync/push.rs          # 1s 检查 meta.dirty → merge → 条件 PUT；image push 也在此
├── src/sync/images.rs        # 启动时一次性镜像 WebDAV /mini-todo/images/ 到本地
├── src/api/                  # axum：Bearer auth + X-Sync-Status header
│   ├── health.rs             # GET /health
│   ├── todos.rs              # GET / POST / GET:id / PATCH:id / DELETE:id
│   ├── subtasks.rs           # POST /todos/:id/subtasks + PATCH/DELETE /subtasks/:id
│   └── images.rs             # GET /images/:name + POST /images（multipart）
└── skill/minitodo/           # Claude Code Skill：SKILL.md + minitodo.py + install.{sh,ps1}
```

REST API（全部需要 `Authorization: Bearer <api_key>`）：

| Method | Path | 说明 |
|---|---|---|
| GET | `/health` | `{status, sync, lastPullAt}` |
| GET | `/todos?...` | 列表；query: `completed` / `priority` / `quadrant` / `dueDateBefore` / `dueDateAfter` / `startDate` / `q` / `sort=±field` / `limit` / `offset` / `withSubtasks` |
| GET | `/todos/:id?withSubtasks=true` | 详情；默认嵌套 subtasks |
| POST | `/todos` | 创建；必填 `title`，其他字段透传到 `data_json` |
| PATCH | `/todos/:id` | merge 更新；未提及字段保留 |
| DELETE | `/todos/:id` | 删除（连带 subtasks） |
| POST | `/todos/:id/subtasks` | 创建子任务；必填 `title` |
| PATCH | `/subtasks/:id` | merge 更新子任务 |
| DELETE | `/subtasks/:id` | 删除子任务 |
| GET | `/images/:name` | 返回图片 bytes |
| POST | `/images` | multipart 上传，`file` 字段；返回 `{name}` |

排序字段白名单：`dueDate` / `startTime` / `priority` / `quadrant` / `sortOrder` /
`updatedAt` / `createdAt` / `title`，前缀 `-` 倒序、`+` 或无前缀正序。`quadrant`
接受数字 1-4 或字符串别名 `urgent_important` / `important_not_urgent` 等。

所有响应附 `X-Sync-Status: healthy | stale | offline` 与 `X-Last-Sync-At`；offline
时额外加 `Warning: 110 "sync offline"`。

PC 端 WebDAV 同步（PR3 后）：
- `services/webdav.rs::upload_bytes` 支持可选 `If-Unmodified-Since` header，返回
  `UploadOutcome::Ok(last_modified)` / `UploadOutcome::PreconditionFailed`
- `services/webdav.rs::download_bytes` 返回 `(Vec<u8>, Option<String>)`，第二个值是
  server 的 `Last-Modified`
- `commands/sync_cmd.rs::webdav_upload_sync` 走条件 PUT；412 时拉远端 →
  `merge_remote_into_local`（per-record LWW）→ 重新 PUT，最多 3 次重试
- `commands/sync_cmd.rs::webdav_download_sync` / `webdav_auto_sync` 把 server 的
  `Last-Modified` 写入 settings key `webdav_last_modified`（由 v24 migration 引入）

### 前后端通信

- **Tauri invoke**：前端调用后端 Rust 命令（请求-响应）
- **Tauri emit/listen**：事件驱动通信（实时推送）
  - `tray-toggle-fixed`、`tray-reset-window`、`tray-add-todo`、`tray-open-settings`：托盘菜单事件
  - `todo-font-changed`：字体设置变更通知

### 独立 WebView 窗口

部分功能使用独立 Tauri WebView 窗口：
- **SubtaskEditorView**：子任务详情编辑（Markdown + 图片）
- **NotificationView**：应用内通知弹窗（当通知类型为 "app" 时）

## 开发命令

> 所有命令都在 `pc/` 子目录下执行（PC 端代码所在位置）。

```bash
# 进入 PC 端工作目录
cd pc

# 安装依赖
npm install

# 开发模式运行
npm run tauri dev

# 构建生产版本
npm run tauri build

# 仅前端构建检查（含 vue-tsc）
npm run build

# Rust 编译检查
cd src-tauri && cargo check
```

## 注意事项

1. **目标平台**：Windows 10/11
2. **运行环境**：需要 Node.js 和 Rust 开发环境
3. **图标使用**：仅使用 Element Plus Icons，禁止 emoji
4. **代码规范**：使用 TypeScript 类型定义，遵循 Vue 3 组合式 API
5. **Serde 命名**：Rust 模型使用 `#[serde(rename_all = "camelCase")]`，前端使用驼峰命名
6. **进程管理**：Windows 平台需要外部子进程时使用 `taskkill` 终止子进程树
