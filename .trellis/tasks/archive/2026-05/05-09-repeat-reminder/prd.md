# feat: 重复提醒功能（闹钟式循环提醒）

## Goal

为 Todo 增加闹钟式的重复提醒功能，支持按天/周/月固定间隔循环提醒，触发后自动推进到下一次，直到用户手动关闭或 Todo 完成。

## What I already know

* 当前提醒系统仅支持单次提醒：`notify_at`（目标时间）+ `notify_before`（提前分钟数）+ `notified`（是否已发送）
* 通知调度器在 `notification.rs` 中，每分钟轮询一次
* 支持两种通知类型：系统通知 + 应用内 WebView 窗口
* 通知窗口 `NotificationView.vue` 目前只有关闭按钮
* 编辑器 `EditorView.vue` 中有日期+时间选择器和提前提醒下拉
* 项目已有 cron 调度能力（用于 Agent 任务），但与通知系统独立
* 数据导入导出和 WebDAV 同步需同步覆盖新字段

## Requirements

### 重复模式
* 支持三种重复类型：每 N 天（daily）、每 N 周（weekly）、每 N 月（monthly）
* 间隔数 N ≥ 1，默认为 1
* 周模式：支持选择星期几（可多选，如周一/三/五），使用 1=周一 ~ 7=周日
* 月模式：指定每月固定日期（1~31），若当月无此日则取当月最后一天

### 触发行为
* 提醒触发后，自动将 `notify_at` 推进到下一个时间点，重置 `notified = 0`
* 推进逻辑在后端 `notification.rs` 的调度循环中完成
* Todo 标记完成时停止重复提醒（不再推进）
* 用户手动关闭重复开关时停止

### 与现有字段的关系
* 重复模式下禁用 `notify_before`（提前提醒），设为 0
* `notify_at` 在重复模式下依然作为「下次提醒时间」使用
* 首次设置时，日期+时间作为「首次提醒」时间点

### 错过提醒处理
* 应用启动时检测已过期的重复提醒
* 补发一次通知，然后推进到下一个未来时间点
* 不补发所有错过的通知，只补发最近一次

### 数据库
* 在 todos 表新增 5 个字段（独立字段，不用 JSON）：
  - `repeat_enabled` INTEGER DEFAULT 0
  - `repeat_type` TEXT（'daily' / 'weekly' / 'monthly'）
  - `repeat_type_interval` INTEGER DEFAULT 1
  - `repeat_weekdays` TEXT（逗号分隔，如 '1,3,5'）
  - `repeat_month_day` INTEGER（1~31）

### UI - EditorView
* 在现有日期/时间选择器下方添加「重复」开关
* 开启后展开重复模式选项（类型选择 + 间隔设置 + 周/月特定选项）
* 开启重复后，日期+时间标签改为「首次提醒」
* 开启重复后，隐藏「提前提醒」选项

### UI - 主列表
* 带重复提醒的 Todo 显示循环小图标（Element Plus 的 RefreshRight 或类似图标）
* 悬停显示下次提醒时间的 tooltip

### 通知窗口
* 不改动，保持现有的关闭按钮交互

### 数据同步
* 新增字段纳入 ExportData 和 SyncData
* 导入导出逻辑同步更新
* 新字段使用 `#[serde(default)]` 确保旧版兼容

## Acceptance Criteria

* [ ] 可在 EditorView 中开启/关闭重复提醒，并设置类型、间隔
* [ ] 每天模式：设置每 N 天重复，到时触发通知并自动推进
* [ ] 每周模式：选择星期几，到时触发通知并自动推进到下一个匹配日
* [ ] 每月模式：选择日期，到时触发通知并自动推进到下月对应日
* [ ] 每月 31 号在短月（如 2 月）正确降级到当月最后一天
* [ ] Todo 标记完成后不再触发重复提醒
* [ ] 手动关闭重复开关后不再推进
* [ ] 应用关闭后重新启动，错过的重复提醒补发一次并推进
* [ ] 主列表中重复提醒的 Todo 显示循环图标
* [ ] 数据导出包含新字段，导入能正确恢复
* [ ] WebDAV 同步覆盖新字段

## Definition of Done

* Lint / typecheck / cargo check 通过
* 导入导出和 WebDAV 同步覆盖新字段
* 数据库迁移版本递增

## Out of Scope

* Subtask 级别的重复提醒
* Cron 表达式模式
* 分钟/小时级别的重复间隔
* 通知窗口的「稍后提醒」/「停止重复」按钮
* 结束日期或最大重复次数
* 通知历史记录

## Technical Notes

### 涉及文件

| 文件 | 改动 |
|------|------|
| `src-tauri/src/db/migrations.rs` | 新增迁移版本，添加 5 个字段 |
| `src-tauri/src/db/models.rs` | Todo 结构体新增字段 |
| `src-tauri/src/services/notification.rs` | 推进逻辑 + 启动补发 |
| `src-tauri/src/commands/todo.rs` | 完成 Todo 时处理重复提醒 |
| `src-tauri/src/commands/data.rs` | 导入导出覆盖新字段 |
| `src-tauri/src/commands/sync_cmd.rs` | WebDAV 同步覆盖新字段 |
| `src/views/EditorView.vue` | 重复提醒 UI 控件 |
| `src/components/TodoItem.vue` | 列表循环图标 |
| `src/types/todo.ts` | 前端类型定义 |

### 推进算法要点

* **daily**：`notify_at + N days`，保持原时间
* **weekly**：从 `repeat_weekdays` 中找到当前之后的下一个匹配星期，跳过 N 周
* **monthly**：`notify_at + N months`，日期取 `min(repeat_month_day, 当月最后一天)`
* 错过多天时：循环推进直到 `notify_at > now`

### 约束

* Windows 平台专用（系统通知使用 Tauri notification plugin）
* 通知调度器每分钟轮询，精度为分钟级
* Serde 使用 camelCase（`#[serde(rename_all = "camelCase")]`）
