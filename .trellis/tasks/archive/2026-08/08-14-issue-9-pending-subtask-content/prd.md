# issue 9：新建待办阶段编辑子任务描述（子任务窗口内存模式）

## 背景

issue #9 用户 KieMg 在 v2.3.3 后追加反馈：新建待办窗口中添加的子任务只有一个眼睛按钮，
点开后的窗口标题为空、也无法添加内容；他希望在新建待办阶段就能给子任务写描述，
而不是必须先保存待办再回头编辑。

## 根因

- 新建模式（无 todo id）下子任务是**纯内存对象**：`EditorView.vue` 的 `pendingSubtasks`，
  用临时负数 id（`--pendingSubtaskIdCounter`），`content: null`，保存待办时才批量 `create_subtask` 落库。
- 子任务窗口 `SubtaskEditorView.vue` 完全走数据库：`get_subtask(id)` 加载、`update_subtask(id)` 保存。
  拿临时负数 id 查库查不到 → 弹出的窗口标题、内容全空（用户看到的 bug）。
- 编辑（铅笔）按钮 `v-if="isEdit && !isViewMode"` 在新建模式被刻意隐藏 → 新建阶段无法写描述；
  但眼睛按钮漏了条件，新建模式仍显示，点开就是上面的空窗口。

## 方案（B：子任务窗口支持内存模式）

用户已确认方案 B：新建模式点编辑/查看照常打开独立子任务窗口，初始数据通过 Tauri 事件传入，
保存时 emit 回主编辑窗口写进 `pendingSubtasks`，待办保存时随现有逻辑一并落库。

### EditorView.vue

- 铅笔按钮条件 `isEdit && !isViewMode` → `!isViewMode`（新建模式也显示）。
- `openSubtaskWindow`：`isEdit` 为 false 时走内存模式分支：
  - URL 追加 `&memory=1`（id 仍传临时负数 id，作为事件过滤键 pendingId）。
  - 打开窗口前注册两个一次性全局监听（窗口 `tauri://destroyed` 时 unlisten 兜底）：
    - `subtask-memory-ready`（payload 含 pendingId，过滤匹配）→ 全局 emit
      `subtask-memory-init` `{ pendingId, title, content, }` 回传初始数据；
    - `subtask-memory-save` `{ pendingId, title, content }` → 更新 `pendingSubtasks`
      对应项的 title / content。
  - 同一时刻只会有一个子任务窗口（已有 `isSubtaskEditorOpen` 守卫），全局事件按 pendingId 过滤即可。
- 现有保存逻辑已把 `subtask.content || undefined` 传给 `create_subtask`，落库路径无需改动。

### SubtaskEditorView.vue

- 解析 `memory=1` 查询参数进入内存模式：
  - 不调 `get_subtask`；先注册 `subtask-memory-init` 监听（按 pendingId 过滤），
    再 emit `subtask-memory-ready` `{ pendingId }`；收到初始数据填 title / content。
  - 保存：不调 `update_subtask`，emit `subtask-memory-save` `{ pendingId, title, content }` 后关窗。
  - 查看模式照常只读展示（同样走内存初始化）。
- DB 模式（无 memory 参数）行为不变。

### 图片上传

`save_subtask_image` 只依赖 `imageData + fileName`，与子任务 DB id 无关（已确认
`MarkdownEditor.vue:86`），内存模式下图片粘贴/拖入开箱即用。

### 事件命名

沿用项目 kebab-case 事件惯例（参照 `tray-toggle-fixed`、`todo-font-changed`）。

## 验收

- 新建待办 → 添加子任务 → 铅笔按钮打开编辑窗口，标题正确回显 → 写 Markdown 描述保存 →
  列表中眼睛按钮查看，内容回显 → 保存待办后子任务描述落库，重新打开待办详情可见。
- 新建模式反复打开/取消子任务窗口无监听器泄漏（destroyed 后 unlisten）。
- 编辑模式（已有 id）与只读详情模式行为完全不变（仍走 DB 路径）。
- `npm run build`（vue-tsc）通过；`cargo check` 通过（本次预计不改 Rust）。
