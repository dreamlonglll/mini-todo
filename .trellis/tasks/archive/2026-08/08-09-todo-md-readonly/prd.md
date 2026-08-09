# 编辑窗口支持 Markdown + todo 只读模式

## Goal

优化待办编辑体验：1) 待办编辑窗口的"描述"字段支持 Markdown（参考子任务的 Milkdown 实现，含图片粘贴上传）；
2) EditorView 新增只读（view）模式，简化排版、重点突出内容；所有查看入口点击待办默认进入只读模式，
可原地切换到编辑模式。

## Requirements

### R1: 描述字段 Markdown 化
- 编辑模式下，"描述"由 `el-input textarea` 替换为 Milkdown 编辑器（commonmark）
- 支持图片粘贴/拖入上传：复用 `save_subtask_image` 命令（images 目录本就是通用的），图片自动纳入现有 WebDAV 图片同步
- 放开原 maxlength=500 限制（DB 为 TEXT，无需迁移）
- 数据仍存 `todo.description` 字段（Markdown 源文本），同步层零改动

### R2: EditorView 只读（view）模式
- URL 增加 `mode=view` 参数；无 id（新建）时始终为编辑模式
- 只读排版：保持左右结构
  - 左侧简化展示：标题、内容（Markdown 渲染）、通知状态、优先级——隐藏其余表单控件
  - 右侧子任务面板**完全照常**（可勾选完成、新增、删除、编辑、查看、排序），"只读"仅限左侧待办本体字段
- 顶部提供 [编辑] 按钮，原地切换到现有编辑排版（不换窗口）
- 只读时 Milkdown 用 `editable: () => false` 且不注册 listener/upload（同子任务 view 模式）

### R3: 入口统一
- 四个查看入口统一默认进只读：TodoList（列表）、QuadrantView（四象限）、CalendarView（日历）、CompletedView（已完成，注意它有独立的 openEditor 实现）
- 新建待办（托盘/悬浮按钮）直接进编辑模式
- 不在待办项上新增悬停编辑图标（保持简洁）

## Acceptance Criteria

- [x] 编辑模式下描述字段为 Milkdown 编辑器，支持 Markdown 语法与图片粘贴上传
- [x] 从列表/四象限/日历/已完成 四个入口单击待办，进入只读模式
- [x] 只读模式左侧仅展示标题、Markdown 渲染内容、通知状态、优先级，排版简洁
- [x] 只读模式右侧子任务面板功能与编辑模式一致
- [x] 只读模式点 [编辑] 原地切换到编辑排版，保存/取消行为不变
- [x] 新建待办直接进入编辑模式
- [x] 旧纯文本描述在 Markdown 渲染下正常显示（向后兼容；已知：纯文本单换行按 commonmark 规则渲染为软换行，多行可能合并，见 Technical Notes）
- [x] 描述长度不再受 500 字符限制

> 以上为代码级完成 + build/lint/cargo check 通过；应用内的人工运行验证待用户执行 `npm run tauri dev` 确认。

## Implementation Summary

- 新增 `pc/src/components/MarkdownEditor.vue`：可复用 Milkdown 组件（v-model + readonly，图片上传复用
  save_subtask_image，内置图片预览与 file:/// 链接处理；含 initSeq 代次守卫 + create 后补同步，
  修复异步 create 与异步数据加载的竞态）
- `SubtaskEditorView.vue` 改用该组件（删除约 150 行内联 Milkdown 逻辑）
- `EditorView.vue`：描述字段 Markdown 化（去掉 maxlength=500）；新增 isViewMode 只读排版
  （标题 + 象限徽标 + 通知状态 + MD 渲染），[编辑] 原地切换，footer 只读时隐藏
- `MainView.vue` / `CompletedView.vue`：openEditor 对已有待办追加 `&mode=view`，窗口标题改"待办详情"
- 版本号 2.1.3 → 2.2.0（package.json / tauri.conf.json / Cargo.toml / Cargo.lock）
- README.md / CLAUDE.md 功能与架构描述更新
- 质检（trellis-check agent）：发现并修复 1 个 P1 竞态；入口残留、抽取等价性、样式、版本一致性全部核对通过

## Definition of Done

- 前端 `npm run build`（含 vue-tsc）通过、ESLint 通过
- `cargo check` 通过（预计无后端改动，复用 save_subtask_image）
- README / CLAUDE.md 行为描述更新
- 版本号递增（2.1.3 → 2.2.0，功能级变更）

## Technical Approach

1. **抽取可复用 Markdown 组件**：从 `SubtaskEditorView.vue` 提取 Milkdown 初始化/销毁、图片上传、
   只读切换逻辑为 `components/MarkdownEditor.vue`（props: `modelValue` / `readonly` / 图片上传开关），
   SubtaskEditorView 改用该组件（顺带回归验证）
2. **EditorView 编辑模式**：描述字段替换为 MarkdownEditor，去掉 maxlength，调整布局高度
3. **EditorView view 模式**：`route.query.mode === 'view'` 进入只读排版；`isViewMode` 响应式状态支持
   [编辑] 原地切换；只读左侧 = 标题 + MarkdownEditor(readonly) + 通知状态 + 优先级
4. **入口改造**：`MainView.openEditor` 与 `CompletedView.openEditor` 增加 mode 参数，四个查看入口传
   `mode=view`，新建不传

## Round 2 增量需求（用户验证后反馈）——已实现，trellis-check 通过（0 缺陷，build/lint 全绿）

### R4: 修复"只读模式显示 MD 源码"（bug）
- **诊断**（已取证，DB id 869）：用户把 MD 源码粘贴进编辑模式的 Milkdown WYSIWYG，粘贴内容被当作
  字面文本插入，序列化时 `#`/`*`/`` ` `` 被反斜杠转义存库（`\# 标题`、`\*\*加粗\*\*`）；
  只读模式忠实渲染了转义后的字面文本，视觉上等于"显示源码"。渲染链路本身无 bug，是输入路径问题
- 修复：MarkdownEditor 编辑模式注册 `@milkdown/kit/plugin/clipboard`（粘贴 MD 源码解析为富文本）
- 增强：注册 `@milkdown/kit/preset/gfm`（编辑+只读都要），表格/任务清单/删除线可正常渲染
- 存量已转义数据不做自动迁移（有误伤风险），用户可重新粘贴或单独决定是否跑数据修复

### R5: 描述专属编辑弹窗（源码/预览分栏）
- 编辑模式下"描述"label 右侧新增放大编辑 icon（FullScreen），点击弹出 el-dialog
- 弹窗左右分栏：左侧 Markdown 源码 textarea（等宽字体），右侧实时渲染预览（readonly MarkdownEditor，
  300ms 防抖）
- 弹窗支持最大化：切换时 dialog fullscreen 并联动 `appWindow.maximize()` 把编辑窗口最大化到屏幕，
  还原/关闭时恢复窗口原状（记录进入时窗口是否本就最大化，避免误还原）
- [确定] 把源码写回 form.description（内联编辑器经 modelValue watch → replaceAll 同步渲染），
  [取消]/关闭丢弃草稿

## Decision (ADR-lite)

**Context**: 只读模式可做成独立 Viewer 窗口或 EditorView 内嵌模式；子任务面板交互边界需明确。
**Decision**: EditorView 加 view 模式原地切换（无开窗开销、复用加载逻辑）；只读仅约束左侧待办本体，
子任务面板完全照常；四个入口统一默认只读；不加待办项悬停编辑图标。
**Consequences**: EditorView 单文件承载双排版，模板复杂度上升，需用清晰的 `isViewMode` 分支组织；
好处是状态共享、切换即时、与子任务 mode=view|edit 心智一致。

## Out of Scope

- 待办项上的悬停"直达编辑"图标
- todo 级独立图片命令（直接复用 save_subtask_image）
- 描述字段的 DB/同步层改动（沿用 description TEXT）
- 移动端/云端（cloud API 透传 data_json，无需改动）

## Technical Notes

- 关键文件：`pc/src/views/EditorView.vue`（:697-706 描述字段、:601-658 子任务窗口）、
  `pc/src/views/SubtaskEditorView.vue`（Milkdown 参考实现全量）、`pc/src/views/MainView.vue:329`（openEditor）、
  `pc/src/views/CompletedView.vue:62`（独立 openEditor）、`pc/src/components/{TodoList,QuadrantView,CalendarView}.vue`
- 图片渲染依赖 assetProtocol scope `**`（tauri.conf.json:30-38），已满足
- 已知风险：旧纯文本描述的单换行在 commonmark 中是软换行，多行文本可能被合并展示；实现时验证
  Milkdown 行为，必要时加载时做换行兼容处理
- 依赖已齐：`@milkdown/kit ^7.19.0`、`@milkdown/theme-nord`，无需新增依赖
