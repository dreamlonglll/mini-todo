<script setup lang="ts">
import { ref, watch } from 'vue'
import draggable from 'vuedraggable'
import { useTodoStore } from '@/stores'
import TodoItem from './TodoItem.vue'
import type { Todo } from '@/types'

const emit = defineEmits<{
  (e: 'edit', todo: Todo): void
}>()

const todoStore = useTodoStore()

// 本地待办列表 (用于拖拽)
const localPendingList = ref<Todo[]>([])

// 同步 store 中的数据到本地列表
watch(
  () => todoStore.pendingTodos,
  (newList) => {
    localPendingList.value = [...newList]
  },
  { immediate: true, deep: true }
)

// 已完成数量
const completedCount = ref(0)
watch(
  () => todoStore.todoCount.completed,
  (val) => { completedCount.value = val },
  { immediate: true }
)

// 拖拽结束处理
async function onDragEnd() {
  const ids = localPendingList.value.map(t => t.id)
  await todoStore.reorderTodos(ids)
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
        <Document />
      </el-icon>
      <p>暂无待办事项</p>
      <p class="hint">请在非锁定模式下，点击悬浮按钮添加待办项</p>
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
