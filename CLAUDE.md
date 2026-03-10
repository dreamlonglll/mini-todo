# Mini-Todo 项目指南

本文档用于介绍 Mini-Todo 项目，帮助 AI 助手快速了解项目结构和开发规范。

## 项目简介

Mini-Todo 是一款基于 **Tauri 2.x + Vue 3 + TypeScript** 开发的 Windows 桌面待办事项管理应用，集成了 AI Agent 调度执行、工作流编排、定时任务等高级功能。

## 技术栈

| 层级 | 技术选型 | 说明 |
|------|----------|------|
| 前端框架 | Vue 3 + TypeScript | 组合式 API，类型安全 |
| UI 组件库 | Element Plus | 包含图标库 @element-plus/icons-vue |
| 状态管理 | Pinia | Vue 官方推荐状态管理 |
| 桌面框架 | Tauri 2.x | 轻量级跨平台桌面框架 |
| 后端语言 | Rust | 高性能，内存安全 |
| 数据库 | SQLite (rusqlite) | 轻量级本地数据库 |
| 拖拽功能 | vuedraggable | Vue 拖拽排序库 |
| 异步运行时 | Tokio | Rust 异步任务调度 |
| 定时调度 | cron (Rust crate) | Cron 表达式解析 |

## 项目结构

```
mini-todo/
├── docs/                           # 文档目录
│   └── 开发文档/                    # 开发相关文档
├── src/                            # Vue 前端源码
│   ├── assets/                     # 静态资源
│   ├── components/                 # Vue 组件
│   │   ├── AgentSettings.vue       # Agent 配置组件
│   │   ├── AgentStatusBadge.vue    # Agent 状态徽章
│   │   ├── CalendarView.vue        # 日历视图
│   │   ├── CronEditor.vue          # Cron 表达式编辑器
│   │   ├── QuadrantView.vue        # 四象限视图
│   │   ├── SchedulerPanel.vue      # 调度面板
│   │   ├── SettingsPanel.vue       # 设置面板
│   │   ├── TitleBar.vue            # 标题栏
│   │   ├── TodoItem.vue            # 待办项组件
│   │   └── TodoList.vue            # 待办列表
│   ├── router/                     # 路由配置
│   ├── stores/                     # Pinia 状态管理
│   │   ├── agentStore.ts           # Agent 相关状态
│   │   ├── appStore.ts             # 应用全局状态
│   │   ├── schedulerStore.ts       # 调度器状态
│   │   └── todoStore.ts            # 待办状态
│   ├── types/                      # TypeScript 类型定义
│   │   ├── agent.ts                # Agent 类型
│   │   ├── scheduler.ts            # 调度器类型
│   │   ├── todo.ts                 # 待办类型
│   │   └── workflow.ts             # 工作流类型
│   ├── utils/                      # 工具函数
│   │   ├── logWindow.ts            # 日志窗口管理
│   │   ├── holiday.ts              # 节假日工具
│   │   └── lunar.ts                # 农历工具
│   ├── views/                      # 页面视图
│   │   ├── AgentLogView.vue        # Agent 执行日志窗口（独立 WebView）
│   │   ├── EditorView.vue          # 待办编辑主视图
│   │   ├── MainView.vue            # 主视图（待办列表）
│   │   ├── SubtaskEditorView.vue   # 子任务编辑视图
│   │   ├── WorkflowView.vue        # 工作流配置视图（独立 WebView）
│   │   ├── NotificationView.vue    # 通知视图
│   │   └── SettingsView.vue        # 设置视图
│   ├── App.vue                     # 根组件
│   └── main.ts                     # 入口文件
├── src-tauri/                      # Tauri/Rust 后端源码
│   ├── src/
│   │   ├── commands/               # Tauri 命令（前后端桥接）
│   │   │   ├── agent_cmd.rs        # Agent 管理命令
│   │   │   ├── data.rs             # 数据导入导出
│   │   │   ├── holiday.rs          # 节假日命令
│   │   │   ├── notification_cmd.rs # 通知命令
│   │   │   ├── prompt_template_cmd.rs # 提示词模板命令
│   │   │   ├── scheduler_cmd.rs    # 调度命令
│   │   │   ├── settings_cmd.rs     # 设置命令
│   │   │   ├── sync_cmd.rs         # 同步命令
│   │   │   ├── todo.rs             # 待办 CRUD 命令
│   │   │   ├── window.rs           # 窗口管理命令
│   │   │   └── workflow_cmd.rs     # 工作流命令
│   │   ├── db/                     # 数据库层
│   │   │   ├── agent_db.rs         # Agent 配置表操作
│   │   │   ├── agent_execution_db.rs # Agent 执行记录操作
│   │   │   ├── connection.rs       # 数据库连接管理
│   │   │   ├── dependency_db.rs    # 任务依赖关系
│   │   │   ├── migrations.rs       # 数据库迁移（v1~v20）
│   │   │   ├── models.rs           # 数据模型定义
│   │   │   ├── prompt_template_db.rs # 提示词模板操作
│   │   │   ├── scheduler_db.rs     # 调度状态操作
│   │   │   └── workflow_db.rs      # 工作流步骤操作
│   │   ├── services/               # 业务服务层
│   │   │   ├── agent/              # Agent 集成
│   │   │   │   ├── runner.rs       # AgentManager + AgentRunner trait
│   │   │   │   ├── claude_code.rs  # Claude Code CLI 实现
│   │   │   │   └── codex.rs        # OpenAI Codex CLI 实现
│   │   │   ├── scheduler/          # 任务调度引擎
│   │   │   │   ├── engine.rs       # 调度主引擎（tick 循环）
│   │   │   │   ├── workflow.rs     # 工作流执行逻辑
│   │   │   │   ├── state_machine.rs # 调度状态机
│   │   │   │   ├── priority_queue.rs # 优先级队列
│   │   │   │   ├── concurrency.rs  # 并发控制
│   │   │   │   └── cron_manager.rs # Cron 定时管理
│   │   │   ├── notification.rs     # 通知服务
│   │   │   └── webdav.rs           # WebDAV 同步
│   │   ├── lib.rs                  # 库入口
│   │   └── main.rs                 # 主入口
│   ├── Cargo.toml                  # Rust 依赖配置
│   └── tauri.conf.json             # Tauri 配置
├── public/                         # 公共静态资源
├── package.json                    # Node 依赖配置
└── vite.config.ts                  # Vite 构建配置
```

## 核心功能

### 待办管理
- 创建、编辑、删除待办事项
- 支持一级子任务（含内容详情、Agent 配置）
- 优先级设置（高/中/低）
- 完成状态标记
- 拖拽排序

### Agent 集成
- **支持的 Agent 类型**：Claude Code CLI、OpenAI Codex CLI
- **Agent 配置管理**：CLI 路径、启用/禁用、健康检查
- **手动执行**：子任务编辑页直接触发 Agent 执行
- **定时调度**：通过 Cron 表达式自动调度子任务
- **执行日志**：独立 WebView 窗口，支持实时流式日志和历史记录查看
- **任务终止与重执行**：运行中的任务可终止，已完成/失败/取消的任务可重新执行

### 工作流系统
- **步骤类型**：执行子任务、执行提示词
- **步骤控制**：启动、停止、继续、重置
- **上下文传递**：步骤可配置"带入上一步结果"，自动继承前一步 Agent 会话
  - Claude Code：使用 `-r session_id` 恢复会话
  - Codex CLI：使用 `resume --last` 模式
- **提示词库**：可创建和管理提示词模板，快速添加到工作流

### 任务调度
- **调度策略**：手动执行、定时执行（Cron）
- **调度状态机**：none → pending → queued → running → completed/failed/cancelled
- **优先级队列**：基于优先级和入队时间排序
- **并发控制**：可配置最大并发执行数
- **任务依赖**：子任务间可设置依赖关系
- **失败重试**：可配置重试次数和间隔

### 通知提醒
- Windows 系统通知
- 预设提醒时间（5/15/30 分钟）
- 自定义提前时间

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
- **迁移版本**：当前 v1~v20，通过 `src-tauri/src/db/migrations.rs` 管理

#### 主要数据表

| 表名 | 说明 |
|------|------|
| `todos` | 待办事项（含 Agent 配置、调度策略） |
| `subtasks` | 子任务（含调度状态、超时配置） |
| `settings` | 应用设置（键值对） |
| `screen_configs` | 屏幕配置 |
| `agent_configs` | Agent 配置（CLI 路径、类型） |
| `agent_executions` | Agent 执行记录（日志、Token、session_id） |
| `workflow_steps` | 工作流步骤（类型、顺序、carry_context） |
| `task_dependencies` | 任务依赖关系 |
| `prompt_templates` | 提示词模板库 |
| `migrations` | 迁移版本记录 |

### 数据导入导出与同步

> **重要**：当数据库结构变更（新增表/字段/设置项）时，必须同步更新导入导出和 WebDAV 同步功能！

- **导出版本号**：当前 `3.0`（位于 `src-tauri/src/commands/data.rs`）
- **关键文件**：
  - 模型定义：`src-tauri/src/db/models.rs` → `ExportData`、`AppSettings`
  - 导入导出逻辑：`src-tauri/src/commands/data.rs` → `export_data_internal`、`import_data_raw`
  - WebDAV 同步：`src-tauri/src/commands/sync_cmd.rs` → `SyncData`、`webdav_upload_sync`、`webdav_apply_remote`
  - 前端类型：`src/types/todo.ts` → `ExportData`（前端只传递 JSON 字符串，无需严格同步）

#### 导入导出架构说明

- `export_data`（手动导出 Tauri 命令）和 WebDAV 上传**共用** `export_data_internal()` 函数
- `import_data`（手动导入 Tauri 命令）和 WebDAV 下载应用**共用** `import_data_raw()` 函数
- `SyncData` 结构将 `ExportData` 的各字段以 `serde_json::Value` 形式传输，不要遗漏新字段

#### 当前同步覆盖范围

| 数据 | 是否同步 | 说明 |
|------|---------|------|
| `todos`（全字段） | 是 | 含 agent/调度/工作流全部字段 |
| `subtasks`（全字段） | 是 | 含调度状态、重试、超时等字段 |
| `settings`（部分） | 是 | 8 个应用设置项，不含 WebDAV 配置 |
| `agent_configs` | 是 | CLI 路径为设备相关，需在目标设备更新 |
| `workflow_steps` | 是 | 通过 todo_id/subtask_id 映射 |
| `task_dependencies` | 是 | 通过 subtask_id 映射 |
| `prompt_templates` | 是 | 仅用户自建模板，内置模板由迁移创建 |
| `images`（文件） | 是 | 通过 WebDAV 独立上传/下载 |
| `screen_configs` | 否 | 设备特定的屏幕配置 |
| `agent_executions` | 否 | 执行历史，数据量大且为临时数据 |
| `migrations` | 否 | 结构性表，应用启动自动管理 |

#### 导入时的 ID 映射

导入数据时，由于自增 ID 会变化，需维护三级映射：
1. `agent_id_map`：旧 agent_config ID → 新 ID（用于 todos.agent_id）
2. `todo_id_map`：旧 todo ID → 新 ID（用于 workflow_steps.todo_id）
3. `subtask_id_map`：旧 subtask ID → 新 ID（用于 workflow_steps.subtask_id、task_dependencies）

#### 维护检查清单

当新增数据库迁移时，请检查：
1. 新增的 **settings 键值** 是否已加入 `AppSettings` 结构体（含 `#[serde(default)]`）
2. `read_app_settings` 函数是否读取了新设置
3. `write_app_settings` 函数是否写入了新设置
4. 新增的 **数据表** 是否需要纳入 `ExportData` 和 `SyncData`
5. `export_data_internal` 是否查询了新表数据
6. `import_data_raw` 是否导入了新表数据（注意 ID 映射和删除顺序）
7. `SyncData` 是否新增了对应字段（含 `#[serde(default)]`）
8. `webdav_upload_sync` 是否从导出 JSON 中提取了新字段（注意 camelCase 键名）
9. `webdav_apply_remote` 和 `webdav_auto_sync` 构建的 import_json 是否包含新字段
10. `check_local_changes` 是否检测了新表的变更
11. 旧版数据导入的兼容性（新字段必须有 `#[serde(default)]`）
12. 导出版本号是否需要递增

## 核心架构概念

### Agent 执行流程

```
用户触发 → start_agent_execution (Tauri Command)
         → AgentManager.start_background_execution()
         → AgentRunner.build_command() (构建 CLI 命令)
         → 子进程执行 (tokio::process::Command)
         → parse_event_line() (解析 JSON 流事件)
         → emit AgentEvent (Tauri 事件推送前端)
         → persist_execution (保存到 agent_executions)
```

### 调度引擎流程

```
TaskScheduler.tick() (每 5 秒)
  ├── check_cron_tasks()        → Cron 到期任务推入队列
  ├── promote_pending_to_queued() → pending → queued
  └── 消费队列
       └── execute_task()       → 启动 Agent 执行
```

### 工作流执行流程

```
start_workflow()
  └── execute_current_step()
       ├── subtask 步骤 → 设为 pending，由调度器执行
       └── prompt 步骤 → 直接启动 Agent 执行
            └── carry_context → 查找前一步 session_id
                 ├── Claude Code: -r session_id
                 └── Codex CLI: resume --last
```

### 前后端通信

- **Tauri invoke**：前端调用后端 Rust 命令（请求-响应）
- **Tauri emit/listen**：事件驱动通信（实时推送）
  - `agent-event:{taskId}`：Agent 执行日志流
  - `schedule:status-changed`：调度状态变更通知

### 独立 WebView 窗口

部分功能使用独立 Tauri WebView 窗口：
- **AgentLogView**：Agent 执行日志（支持实时流式 + 历史查看）
- **WorkflowView**：工作流配置和控制

通过 `src/utils/logWindow.ts` 的 `openLogWindow()` 打开日志窗口。

## 开发命令

```bash
# 安装依赖
npm install

# 开发模式运行
npm run tauri dev

# 构建生产版本
npm run tauri build

# Rust 编译检查
cd src-tauri && cargo check
```

## 注意事项

1. **目标平台**：Windows 10/11
2. **运行环境**：需要 Node.js 和 Rust 开发环境
3. **图标使用**：仅使用 Element Plus Icons，禁止 emoji
4. **代码规范**：使用 TypeScript 类型定义，遵循 Vue 3 组合式 API
5. **Serde 命名**：Rust 模型使用 `#[serde(rename_all = "camelCase")]`，前端使用驼峰命名
6. **进程管理**：Windows 平台使用 `taskkill` 终止子进程树
7. **状态同步**：调度状态变更需同时更新数据库和通过 Tauri 事件通知前端
