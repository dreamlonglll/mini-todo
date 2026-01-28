<script setup lang="ts">
import { computed } from 'vue'
import { useTodoStore } from '@/stores'
import dayjs from 'dayjs'
import type { Todo } from '@/types'

const props = defineProps<{
  todo: Todo
}>()

const emit = defineEmits<{
  (e: 'click'): void
  (e: 'toggle-complete'): void
  (e: 'delete'): void
}>()

const todoStore = useTodoStore()

// 是否已完成
const isCompleted = computed(() => props.todo.completed)

// 优先级类
const priorityClass = computed(() => props.todo.priority)

// 子任务统计
const subtaskStats = computed(() => {
  const total = props.todo.subtasks.length
  const completed = props.todo.subtasks.filter(s => s.completed).length
  return { total, completed }
})

// 格式化通知时间
const formattedNotifyTime = computed(() => {
  if (!props.todo.notifyAt) return null
  return dayjs(props.todo.notifyAt).format('MM-DD HH:mm')
})

// 切换完成状态
function toggleComplete(e: Event) {
  e.stopPropagation()
  emit('toggle-complete')
}

// 删除待办
function deleteTodo(e: Event) {
  e.stopPropagation()
  emit('delete')
}

// 置顶待办
async function topTodo(e: Event) {
  e.stopPropagation()
  const ids = todoStore.pendingTodos
    .filter(t => t.id !== props.todo.id)
    .map(t => t.id)
  await todoStore.reorderTodos([props.todo.id, ...ids])
}

// 点击待办
function handleClick() {
  emit('click')
}
</script>

<template>
  <div 
    class="todo-item" 
    :class="{ completed: isCompleted }"
    @click="handleClick"
  >
    <!-- 拖拽手柄 + 优先级圆点 -->
    <div class="drag-handle priority-dot" :class="priorityClass"></div>

    <!-- 内容区域 -->
    <div class="todo-content">
      <div class="todo-title">{{ todo.title }}</div>
      
      <div v-if="subtaskStats.total > 0 || formattedNotifyTime" class="todo-meta">
        <!-- 子任务统计 -->
        <span v-if="subtaskStats.total > 0" class="subtask-count">
          <el-icon :size="12"><Finished /></el-icon>
          {{ subtaskStats.completed }}/{{ subtaskStats.total }}
        </span>

        <!-- 通知时间 -->
        <span v-if="formattedNotifyTime" class="notify-time">
          <el-icon :size="12"><Bell /></el-icon>
          {{ formattedNotifyTime }}
        </span>
      </div>
    </div>

    <!-- 操作按钮 -->
    <div class="todo-actions">
      <button 
        class="action-btn complete-btn" 
        :title="isCompleted ? '取消完成' : '完成'"
        @click="toggleComplete"
      >
        <el-icon :size="16">
          <Select v-if="!isCompleted" />
          <RefreshLeft v-else />
        </el-icon>
      </button>

      <button 
        v-if="!isCompleted"
        class="action-btn top-btn" 
        title="置顶"
        @click="topTodo"
      >
        <el-icon :size="16"><Top /></el-icon>
      </button>

      <button 
        class="action-btn delete-btn" 
        title="删除"
        @click="deleteTodo"
      >
        <el-icon :size="16"><Delete /></el-icon>
      </button>
    </div>
  </div>
</template>

<style scoped>
/* 使用 main.scss 中的样式 */
</style>
