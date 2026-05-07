# feat: 子任务标题展示

## Goal

当待办事项包含子任务时，用户目前只能看到子任务数量（如 "2/5"），无法快速预览子任务标题。本任务在 TodoItem 中增加可展开/收起的子任务标题列表，让用户无需进入编辑页即可了解子任务详情。

来源：GitHub Issue #5 建议 2。

## Requirements

- 点击子任务计数区域（`2/5`）展开/收起子任务标题列表
- 计数区域旁显示 Element Plus `ArrowRight`（收起）/ `ArrowDown`（展开）图标
- 计数区域 hover 时变色，暗示可点击
- 点击计数区域使用 `@click.stop` 阻止冒泡到 todo 项的编辑跳转
- 展开后显示每个子任务：完成状态图标 + 标题文本
  - 已完成子任务：标题加删除线
- 标题单行截断 + 省略号（`text-overflow: ellipsis`）
- 鼠标悬停截断标题时显示 tooltip（完整标题）
- 展开/收起使用 Vue `<Transition>` 高度渐变动画
- 子任务标题纯展示，不可点击交互
- 展开状态为组件内 `ref`，不持久化，刷新/切换后重置为收起

## Acceptance Criteria

- [ ] 有子任务的 todo 项显示计数 + 箭头图标
- [ ] 点击计数区域展开子任务标题列表，再次点击收起
- [ ] 展开时箭头从 ArrowRight 变为 ArrowDown
- [ ] 已完成子任务显示勾选图标 + 删除线
- [ ] 未完成子任务显示未勾选图标
- [ ] 长标题单行截断并显示省略号
- [ ] 悬停截断标题时显示 tooltip
- [ ] 展开/收起有平滑高度过渡动画
- [ ] 点击计数区域不触发 todo 编辑跳转
- [ ] 点击子任务标题无任何交互
- [ ] 浅色和深色主题下样式正常
- [ ] 固定模式（透明背景）下样式正常

## Definition of Done

- Lint / typecheck 通过
- 浅色/深色主题均视觉正常
- 固定模式下无样式异常

## Out of Scope

- 子任务标题列表中不支持点击交互（不跳转编辑页、不切换完成状态）
- 不持久化展开/收起状态
- 不修改后端或数据库
- 不新增 store 逻辑

## Technical Approach

### 改动范围

仅 `src/components/TodoItem.vue`，无后端改动。

### 实现要点

1. **数据**：`props.todo.subtasks` 已包含完整 SubTask 数据（含 title、completed），无需额外请求
2. **状态**：组件内 `ref<boolean>` 控制展开/收起
3. **模板**：
   - 修改现有 `.subtask-count` span，添加 `@click.stop` + 箭头图标
   - 在 `.todo-meta` 下方新增子任务列表区域，用 `<Transition>` 包裹
4. **样式**：
   - 子任务列表缩进、紧凑排列
   - 已完成项删除线 + 降低透明度
   - 标题 `text-overflow: ellipsis` + `el-tooltip`
   - 高度过渡动画 CSS

### 关键文件

- `src/components/TodoItem.vue`（L96-99 子任务计数区域）
- `src/types/todo.ts`（SubTask 接口，title 字段已存在）

## Decision (ADR-lite)

**Context**: Issue #5 要求展示子任务标题，当前只显示数量  
**Decision**: 在 TodoItem 内增加可展开的子任务列表，点击计数切换，纯展示无交互  
**Consequences**: 改动集中在单个组件，无跨模块影响；未来如需点击交互可在此基础上扩展
