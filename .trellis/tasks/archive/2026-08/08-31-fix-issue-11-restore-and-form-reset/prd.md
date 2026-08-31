# 修复 issue 11：已完成列表恢复按钮无效与编辑窗口修改被子任务操作重置

来源：GitHub issue #11（用户 KieMg 反馈，两个功能 bug）。

## Bug 1：已完成列表「恢复为未完成」按钮无反应

- **根因**：`pc/src/views/CompletedView.vue` 的 `handleToggleComplete` 调用
  `invoke('toggle_complete', { id })`，但后端 `commands/todo.rs` 从未注册过
  `toggle_complete` 命令（全前端仅此一处引用）。invoke reject 被 catch 吞掉，
  界面无任何反馈。
- **修复**：改走既有命令 `update_todo`，即
  `invoke('update_todo', { id, data: { completed: false } })`，与主列表
  `todoStore.toggleComplete` 及编辑窗口「重新打开」按钮同一路径。
  该按钮语义固定是"恢复为未完成"（列表里只有已完成项），直接传 `false`。

## Bug 2：编辑待办窗口的未保存修改被子任务操作重置

- **根因**：`pc/src/views/EditorView.vue` 的 `loadTodo()` 除了刷新
  `todo.value`（子任务列表来源）外，还会用数据库旧值整体重建 `form.value`
  （标题/描述/颜色/四象限/时间/重复设置）。子任务编辑窗口关闭
  （`tauri://destroyed` 回调）、添加/勾选/删除/重命名/导入子任务、拖拽排序失败
  回滚等路径都会触发 `loadTodo()`，把左侧表单未保存的修改冲掉。
- **修复**：拆分「初始加载（重建表单）」与「子任务刷新（不动表单）」：
  `loadTodo(refreshForm = true)`，仅 onMounted 初始加载用默认值 true，
  所有子任务操作后的刷新传 false。`todo.value` 照常整体更新（子任务列表、
  completed 状态取自它），只跳过 form/repeat/notify 等表单态的重建。

## 验收

- 已完成列表点「恢复为未完成」，待办立即回到主列表（`todo-updated` 事件已有
  监听，列表自动刷新）。
- 编辑待办窗口：改标题/描述/颜色/象限后，添加、勾选、删除、重命名、
  编辑子任务（含独立子任务窗口保存关闭），左侧表单修改保持不丢；
  最后点「保存」能把修改落库。
- `npm run build`（vue-tsc）通过；`cargo check` 通过（后端无改动，仅版本号）。

## 发版

修复后按固定发版流程发 v2.3.8：三处版本号同步 → 提交 → 推 tag → 等
GitHub 构建成功 → 改写中文 release notes。
