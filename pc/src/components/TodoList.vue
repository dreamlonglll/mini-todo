<script setup lang="ts">
import { ref, watch } from 'vue'
import draggable from 'vuedraggable'
// 别名：Document 会遮蔽 DOM 全局类型
import { Document as DocumentIcon } from '@element-plus/icons-vue'
import { useTodoStore } from '@/stores'
import TodoItem from './TodoItem.vue'
import type { Todo } from '@/types'

const emit = defineEmits<{
  (e: 'edit', todo: Todo): void
}>()

const todoStore = useTodoStore()

// 本地待办列表 (用于拖拽)
const localPendingList = ref<Todo[]>([])

// 拖拽进行中：期间 store 的刷新（轮询 / 聚焦 / 同步）不重建本地列表，免得 Vue 在
// SortableJS 拖动途中移动 DOM 节点导致落点错乱；拖拽结束后统一按 store 重建
let dragging = false

function syncFromStore() {
  localPendingList.value = [...todoStore.pendingTodos]
}

// 同步 store 中的数据到本地列表。
// 不需要 deep：pendingTodos 是 computed，成员、完成状态、排序变化都会让它产出新数组；
// 列表项内部字段（标题、子任务……）由 TodoItem 直接读同一个对象，自然响应
watch(
  () => todoStore.pendingTodos,
  () => {
    if (dragging) return
    syncFromStore()
  },
  { immediate: true }
)

// 已完成数量
const completedCount = ref(0)
watch(
  () => todoStore.todoCount.completed,
  (val) => { completedCount.value = val },
  { immediate: true }
)

function onDragStart() {
  dragging = true
}

// 拖拽结束处理：落库新顺序；无论成败都按 store 重建一次
// （失败时 store 顺序未变，列表回到拖拽前的样子；拖拽期间被跳过的刷新也在这里补上）
async function onDragEnd() {
  dragging = false
  const ids = localPendingList.value.map(t => t.id)
  try {
    await todoStore.reorderTodos(ids)
  } finally {
    // 落库期间又开始了新一轮拖拽：留给那一轮结束时重建
    if (!dragging) syncFromStore()
  }
}

// 编辑待办
function handleEdit(todo: Todo) {
  emit('edit', todo)
}

// 切换完成状态
async function handleToggleComplete(todo: Todo) {
  await todoStore.toggleComplete(todo.id)
}

// 删除待办
async function handleDelete(todo: Todo) {
  await todoStore.deleteTodo(todo.id)
}
</script>

<template>
  <div class="todo-list">
    <!-- 未完成待办列表 (可拖拽) -->
    <draggable
      v-model="localPendingList"
      item-key="id"
      handle=".color-dot"
      ghost-class="dragging"
      :animation="200"
      :force-fallback="true"
      @start="onDragStart"
      @end="onDragEnd"
    >
      <template #item="{ element }">
        <TodoItem
          :todo="element"
          @click="handleEdit(element)"
          @toggle-complete="handleToggleComplete(element)"
          @delete="handleDelete(element)"
        />
      </template>
    </draggable>

    <!-- 空状态 -->
    <div v-if="localPendingList.length === 0 && completedCount === 0" class="empty-state">
      <el-icon :size="48" color="var(--text-tertiary)">
        <DocumentIcon />
      </el-icon>
      <p>暂无待办事项</p>
      <p class="hint">点击右下角悬浮按钮添加待办项</p>
    </div>
  </div>
</template>

<style scoped>
.todo-list {
  /* min-height: 200px; */
}

.empty-state {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  padding: var(--space-6);
  color: var(--text-tertiary);
  text-align: center;

  p {
    margin-top: var(--space-2);
    font-size: 14px;
  }

  .hint {
    font-size: 12px;
    margin-top: var(--space-1);
  }
}
</style>
