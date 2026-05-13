# PRD：移除 AI Agent 功能

## 背景

Mini-Todo 自 v1.x 起逐步集成了 Agent（Claude Code CLI / Codex CLI）、任务调度（Cron + 优先级队列 + 并发控制）、工作流（多步骤 Agent 编排）、提示词模板库、子任务依赖等一整套"AI 任务执行编排"能力。这些能力膨胀了项目复杂度（前端 ~6 个组件/视图、后端 ~10 个模块、5 张额外数据表），但实际使用频率不足以支撑维护成本。

本任务目标：**让 Mini-Todo 回归"纯净待办应用"——保留待办、子任务（带 Markdown）、四象限、日历、通知、重复提醒、WebDAV 同步、导入导出；移除所有 AI/调度/工作流相关能力。**

## 范围（已通过 grill-me 会话与用户对齐）

### 保留的功能

- 待办 CRUD（标题、描述、颜色、四象限、开始/截止时间、排序、完成态）
- 子任务（标题、Markdown content、图片上传、完成态、排序）+ 独立详情窗口（SubtaskEditorView）
- 通知提醒（系统通知 + 提前提醒 `notifyAt` / `notifyBefore`）
- **重复提醒**（v22 migration 引入：`repeatEnabled` / `repeatType` / `repeatInterval` / `repeatWeekdays` / `repeatMonthDay`）
- 四象限视图、日历视图、列表视图
- WebDAV 同步、导入/导出
- 应用设置（主题、自动隐藏、固定窗口、屏幕配置等）
- 标题栏、托盘菜单（删除 agent/workflow/scheduler 相关入口）

### 移除的功能（完整清单）

#### 前端文件 —— 整体删除

- `src/components/AgentSettings.vue`
- `src/components/AgentStatusBadge.vue`
- `src/components/SchedulerPanel.vue`
- `src/components/CronEditor.vue`
- `src/views/AgentLogView.vue`
- `src/views/WorkflowView.vue`
- `src/stores/agentStore.ts`
- `src/stores/schedulerStore.ts`
- `src/types/agent.ts`
- `src/types/scheduler.ts`
- `src/types/workflow.ts`
- `src/utils/logWindow.ts`（仅供 Agent 日志窗口使用）

#### 前端文件 —— 删段保留

- `src/views/EditorView.vue`（2086 行 → 预计 ~1000 行）：删除 Agent 配置、调度配置、工作流面板及相关 ref/computed/watch/function；保留 title/description/color/quadrant/notify/start_end/repeat/subtasks 表单。**原地删改，不重构。**
- `src/views/SubtaskEditorView.vue`（892 行 → 预计 ~400 行）：删除 Agent 执行面板、调度配置、提示词模板插入；保留 Milkdown Markdown 编辑器 + 图片上传 + 标题。**原地删改。**
- `src/views/MainView.vue`（706 行）：删除调度按钮、Agent 状态徽章使用、工作流按钮入口。
- `src/views/SettingsView.vue`：删除 AgentSettings 嵌入；保留其他设置项。
- `src/components/TodoItem.vue`：删除 AgentStatusBadge 引用、调度状态显示、工作流图标；保留颜色/优先级/标题/通知图标/重复图标。
- `src/stores/todoStore.ts`：已验证不依赖 agentStore/schedulerStore，仅需清理 Todo 类型字段读写。
- `src/router/`：删除 `/agent-log` 和 `/workflow` 路由。
- `src/App.vue`：删除上述路由对应的 RouterView 出口（如有）。
- `src/types/todo.ts`：删除 Todo 接口的 `agentId` / `agentProjectPath` / `scheduleStrategy` / `cronExpression` / `scheduleEnabled` / `lastScheduledRun` / `postAction` / `workflowEnabled` / `workflowCurrentStep`；删除 SubTask 接口的 `scheduleStatus` / `priorityScore` / `maxRetries` / `retryCount` / `timeoutSecs` / `scheduledAt` / `lastScheduledRun` / `scheduleError`；删除 `PostActionType` 类型；删除 CreateTodoRequest / UpdateTodoRequest 对应字段；保留 ExportData。

#### 后端文件 —— 整体删除

- `src-tauri/src/commands/agent_cmd.rs`
- `src-tauri/src/commands/workflow_cmd.rs`
- `src-tauri/src/commands/scheduler_cmd.rs`
- `src-tauri/src/commands/prompt_template_cmd.rs`
- `src-tauri/src/db/agent_db.rs`
- `src-tauri/src/db/agent_execution_db.rs`
- `src-tauri/src/db/workflow_db.rs`
- `src-tauri/src/db/scheduler_db.rs`
- `src-tauri/src/db/prompt_template_db.rs`
- `src-tauri/src/db/dependency_db.rs`
- `src-tauri/src/services/agent/`（整个目录：runner.rs / claude_code.rs / codex.rs / mod.rs）
- `src-tauri/src/services/scheduler/`（整个目录：engine.rs / workflow.rs / state_machine.rs / priority_queue.rs / concurrency.rs / cron_manager.rs / mod.rs）

#### 后端文件 —— 删段保留

- `src-tauri/src/lib.rs`：删除 agent/scheduler/workflow/prompt 相关 Tauri command 注册、模块导入、AppState 字段（如 AgentManager / TaskScheduler）。
- `src-tauri/src/main.rs`：同上。
- `src-tauri/src/commands/mod.rs`：删除相关 pub mod 行。
- `src-tauri/src/db/mod.rs`：删除相关 pub mod / pub use 行。
- `src-tauri/src/db/models.rs`：删除 `AgentConfig` / `AgentExecution` / `WorkflowStep` / `TaskDependency` / `PromptTemplate` 等类型；删除 Todo 模型的 9 个 agent 相关字段；删除 SubTask 模型的 8 个调度字段；保留 `ExportData` 但删除 agent 子字段（导入时保留 `#[serde(default)]` 容忍旧 JSON）。
- `src-tauri/src/db/migrations.rs`：**新增 migration_v23** —— `DROP TABLE agent_configs / agent_executions / workflow_steps / task_dependencies / prompt_templates`；`ALTER TABLE todos DROP COLUMN` 各 agent 字段；`ALTER TABLE subtasks DROP COLUMN` 各调度字段。SQLite 3.35+ 支持原生 `DROP COLUMN`（rusqlite bundled 版本满足）。
- `src-tauri/src/services/mod.rs`：删除 agent/scheduler 模块。
- `src-tauri/src/services/notification.rs`：检查并删除任何 agent 相关通知逻辑。
- `src-tauri/src/services/webdav.rs`：同步覆盖范围内不再有 agent_configs / workflow_steps / prompt_templates 等表。
- `src-tauri/src/commands/data.rs`：
  - 导出版本号 `"3.0"` → `"4.0"`
  - `ExportData` 移除 agent_configs / workflow_steps / prompt_templates / task_dependencies 字段（但保留向后兼容反序列化）
  - `export_data_internal` 不再查询和写入这些数据
  - `import_data_raw` 兼容旧 v3.0 JSON：对 agent 相关字段静默忽略（serde default 跳过）
- `src-tauri/src/commands/sync_cmd.rs`：`SyncData` 同步删字段；`webdav_upload_sync` / `webdav_apply_remote` / `webdav_auto_sync` / `check_local_changes` 同步精简。

#### Cargo.toml 依赖清理

- 删除：`cron = "0.15.0"`
- 删除：`async-trait = "0.1"`（仅 AgentRunner trait 使用，待删后无引用）
- 不动：`tokio "full"`（其他功能需要）、`reqwest` / `aes-gcm` / `sha2` / `rand` / `machine-uid` / `notify`（WebDAV/通知/文件监听用）

#### 文档清理

- **整目录删除**：`docs/Agents灵感/`（27 个 .md 设计/开发文档）—— 历史可从 git 找回
- **章节修订**：`docs/开发文档/01-项目概述.md` / `02-技术架构.md` / `03-功能需求.md` / `05-数据库设计.md` —— 删除 agent/scheduler/workflow 描述，更新数据表清单
- **重写**：`CLAUDE.md` —— 删除"Agent 集成"/"工作流系统"/"任务调度"章节；从"主要数据表"清单移除 agent_configs / agent_executions / workflow_steps / task_dependencies / prompt_templates；从"核心架构概念"删除 Agent 执行流程图、调度引擎流程图、工作流执行流程图；从"事件清单"删除 `agent-event:{taskId}` 和 `schedule:status-changed`；更新数据库迁移版本号至 v23
- **快速检查**：`.trellis/spec/frontend/*.md` 和 `.trellis/spec/guides/*.md` —— 预计无大改，过一遍确认没有 agent 专用约束

## 版本号

- **应用版本**：1.6.4 → **2.0.0**（package.json + src-tauri/Cargo.toml + src-tauri/tauri.conf.json 三处同步）
- **导出版本**：3.0 → **4.0**（src-tauri/src/commands/data.rs）
- **数据库迁移版本**：v22 → **v23**

## 数据安全

- **不写自动备份代码**。
- 实施前由用户**手动**执行：`copy "%APPDATA%\mini-todo\data.db" "%APPDATA%\mini-todo\data.db.before-remove-agent.bak"`。
- 升级后旧 `agent_configs` / `agent_executions` / `workflow_steps` / `task_dependencies` / `prompt_templates` 表数据**永久丢失**——这是用户已确认的取舍。

## 向后兼容

- 导入 `import_data_raw` **兼容 v3.0 和 v4.0 JSON**：
  - 旧 JSON 里的 agent_configs / workflow_steps / prompt_templates / task_dependencies 字段静默忽略
  - 旧 JSON 里 todos / subtasks 的 agent/调度字段静默忽略
  - 通过 serde `#[serde(default)]` + 未声明字段不报错实现
- WebDAV 远端旧版数据：新版客户端拉取时同样兼容，但上传只写 v4.0 格式
- 文件中所有"agent 字段被忽略"的兼容点必须以代码可读方式呈现（例如在 ExportData 结构上方加一行注释说明）

## 实施约束

- **改造策略**：所有 .vue / .rs 文件**原地删改**，不主动重构、不重排序、不拆函数、不重命名。
- 删除后变成 unused 的 import / ref / computed / function / v-if 分支 **顺手清理**。
- **不引入新功能**、**不修改现有非 agent 功能的行为**。
- 提交流程：分批 commit 而非单 commit——例如可拆分为：
  1. 删除后端 agent/scheduler/workflow 整目录 + Cargo.toml
  2. migration v23：DROP 表 + DROP COLUMN
  3. 删除后端 commands/db 模块 + lib.rs 注册
  4. 数据 import/export/WebDAV 改造（兼容 v3.0 输入）
  5. 删除前端 store / type / 路由 / 整体文件
  6. EditorView / SubtaskEditorView / MainView / TodoItem / SettingsView 原地删段
  7. CLAUDE.md 重写 + docs/Agents灵感/ 删目录 + docs/开发文档/ 章节修订
  8. 版本号 bump 至 2.0.0

## 验收标准

- `cargo check` 通过，无 warning（与 agent 相关的 unused / dead_code 全部清理）
- `npm run build`（含 `vue-tsc --noEmit`）通过
- 应用启动后：v22 数据库自动升级到 v23，新表结构不含 agent 相关列
- 应用打开旧 v3.0 导出 JSON 仍能成功导入，todos/subtasks/settings 数据完整恢复（agent 字段静默忽略）
- 启动后 UI 验证：
  - 新建/编辑 todo 表单不含 agent 配置、调度配置、工作流按钮
  - todo 列表项不含 agent 状态徽章
  - 子任务详情窗口可正常打开 Markdown 编辑器、上传图片
  - 设置页不含 Agent 设置区
  - 重复提醒、通知、四象限、日历、WebDAV 同步功能不受影响
- `CLAUDE.md` 与代码现状一致——无任何"agent / scheduler / workflow / prompt_template / task_dependency"残留描述
- 仓库内无任何 agent/Agent/workflow/Workflow/scheduler/Scheduler/cron/Cron/prompt/Prompt 相关引用（除合法用途如 "promptly" 等英语单词、`notify_at` 中的 notify 等非 agent 语义词）

## 风险与回滚

- **风险 1**：EditorView/SubtaskEditorView 原地删改时遗漏某个隐式 watch / nextTick / emit 链，导致编辑器某个 UX 行为退化（如保存时机、表单清空时机）。**对策**：删段时保留行号对齐的 git diff，便于回归定位。
- **风险 2**：migration v23 在某些 SQLite 老版本上 DROP COLUMN 失败。**对策**：rusqlite 0.32 bundled SQLite >= 3.40，确认支持后无需 fallback。
- **风险 3**：WebDAV 远端旧数据导入时，旧 JSON 字段名 / 嵌套结构变化导致 serde 失败。**对策**：在 import_data_raw 增加 `#[serde(deny_unknown_fields = false)]`（默认行为）并对每个保留字段加 `#[serde(default)]`；新增最小 fixture 测试旧 JSON 输入。
- **回滚**：用户手动恢复 `data.db.before-remove-agent.bak` 并重新安装 v1.6.4 即可。
