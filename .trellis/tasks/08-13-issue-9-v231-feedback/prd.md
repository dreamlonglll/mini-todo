# issue #9 v2.3.1 第三轮反馈修复

## Goal

KieMg 在 v2.3.1 验证后提出两条体验反馈：子任务拖拽的两个视觉问题、固定模式下无法新建待办。
用户（owner）已拍板新建按钮采用 KieMg 的「方案 2」（保留悬浮大按钮 + 悬停操作按钮时半透明沉底）。

## Requirements

### R1 子任务拖拽：拖动时不再选中文字

- `EditorView.vue` 的 `.subtask-item-editor` 缺 `user-select: none`（主列表 `.todo-item` 有，所以主列表没这问题）
- 加上后，行内编辑输入框 `.subtask-inline-input` 显式恢复 `user-select: text`

### R2 子任务拖拽：拖影跟手

- 根因：`.subtask-item-editor` 的 `transition: all 0.2s ease` 被 SortableJS
  force-fallback 克隆体继承，克隆体每次 mousemove 的 transform 更新都被 0.2s
  过渡拖慢，永远追不上鼠标（拖 10cm 影子只动 1cm）
- 修复：过渡属性改为显式列表（border-color / box-shadow / background），不含 transform
- 对照：主列表 `.todo-item` 只过渡 background，所以主列表拖拽正常

### R3 新建待办按钮「方案 2」

1. 固定模式也渲染 FAB（去掉 `v-if="!appStore.isFixed"`），固定模式下用户可直接新建
   - 同步删掉 `main.scss` 里 `body.fixed-mode .main-content` 取消底部避让的规则
2. 「操作模式」状态机（MainView 事件委托监听 `.todo-actions` 悬停）：
   - 默认模式：FAB 置顶（现状，z-index 100）
   - 悬停操作按钮（完成/置顶/删除）持续 **0.75s** → 进入操作模式：FAB 加 `fab-dimmed`
     类（半透明 opacity 0.35 + z-index 4，低于 `.todo-actions` 的 z-index 5）
   - 鼠标离开操作按钮 **1.5s** 后 → 回到默认模式
   - 期间重新进入操作按钮则取消回退计时
   - 操作模式下直接悬停 FAB 本体 → 立即恢复不透明（可点击性提示）

## Acceptance Criteria

- [ ] 编辑窗口拖拽子任务不再出现划选文字
- [ ] 拖拽克隆体实时跟随鼠标
- [ ] 行内重命名输入框仍可正常选中文字
- [ ] 固定模式下 FAB 显示且可点击新建待办；底部预留占位在固定模式同样生效
- [ ] 悬停操作按钮约 0.75s 后 FAB 变半透明且沉到操作按钮之下，操作按钮可正常点击
- [ ] 离开操作按钮约 1.5s 后 FAB 恢复；1.5s 内回到操作按钮不恢复
- [ ] `npm run build`（vue-tsc）通过

## Out of Scope

- 主界面展开区子任务拖拽（上轮已说明的嵌套拖拽问题）
- 「沉浸模式」（检测前台全屏应用决定是否置顶）
- 顶部工具栏新建按钮（方案 1，用户未选）

## Technical Notes

- `pc/src/views/EditorView.vue:1180` draggable 配置、`:1831` `.subtask-item-editor` 样式
- `pc/src/views/MainView.vue:669` FAB、事件委托放 MainView 根节点
- `pc/src/styles/main.scss:718` `.fab-add`、`:547` `.todo-actions`（absolute, z-index 5）、
  `:372` `body.fixed-mode .main-content` 底部避让豁免（需删除）
- `.todo-item` 无 z-index、不构成层叠上下文，所以 `.todo-actions` 的 z-index 5 与
  FAB 的 z-index 在根层叠上下文比较 → dimmed 用 z-index 4 即可沉底
- 定时器：进入 750ms / 离开 1500ms，mouseover/mouseout + closest('.todo-actions') 判定
