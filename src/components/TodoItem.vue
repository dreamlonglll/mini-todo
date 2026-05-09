<script setup lang="ts">
import { computed, ref } from 'vue'
import { useTodoStore } from '@/stores'
import { ElMessageBox } from 'element-plus'
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

// 颜色样式
const colorStyle = computed(() => ({
  backgroundColor: props.todo.color
}))

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

// 是否重复提醒
const isRepeat = computed(() => !!props.todo.repeatEnabled)

// 重复提醒的 tooltip 文字
const repeatTooltip = computed(() => {
  if (!isRepeat.value || !props.todo.repeatType) return ''
  const interval = props.todo.repeatInterval || 1
  const weekdayNames = ['一', '二', '三', '四', '五', '六', '日']
  switch (props.todo.repeatType) {
    case 'daily':
      return interval === 1 ? '每天重复' : `每 ${interval} 天重复`
    case 'weekly': {
      const days = (props.todo.repeatWeekdays || '')
        .split(',')
        .filter(Boolean)
        .map(d => weekdayNames[parseInt(d) - 1] || d)
        .join('、')
      const prefix = interval === 1 ? '每周' : `每 ${interval} 周`
      return days ? `${prefix} 周${days}` : prefix
    }
    case 'monthly': {
      const day = props.todo.repeatMonthDay || 1
      return interval === 1 ? `每月 ${day} 号` : `每 ${interval} 月 ${day} 号`
    }
    default:
      return '重复提醒'
  }
})

// 切换完成状态
function toggleComplete(e: Event) {
  e.stopPropagation()
  emit('toggle-complete')
}

// 删除待办
async function deleteTodo(e: Event) {
  e.stopPropagation()
  try {
    await ElMessageBox.confirm(
      `确定要删除待办"${props.todo.title}"吗？`,
      '删除确认',
      {
        confirmButtonText: '删除',
        cancelButtonText: '取消',
        type: 'warning'
      }
    )
    emit('delete')
  } catch {
    // 用户取消，不做任何操作
  }
}

// 置顶待办
async function topTodo(e: Event) {
  e.stopPropagation()
  const ids = todoStore.pendingTodos
    .filter(t => t.id !== props.todo.id)
    .map(t => t.id)
  await todoStore.reorderTodos([props.todo.id, ...ids])
}

// 子任务按完成状态排序：未完成在前
const sortedSubtasks = computed(() =>
  [...props.todo.subtasks].sort((a, b) => Number(a.completed) - Number(b.completed))
)

// 子任务列表展开状态
const subtaskExpanded = ref(false)

function toggleSubtaskExpand(e: Event) {
  e.stopPropagation()
  subtaskExpanded.value = !subtaskExpanded.value
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
    <!-- 拖拽手柄 + 颜色圆点 -->
    <div class="drag-handle color-dot" :style="colorStyle"></div>

    <!-- 内容区域 -->
    <div class="todo-content">
      <div class="todo-title">{{ todo.title }}</div>
      
      <div v-if="subtaskStats.total > 0 || formattedNotifyTime || isRepeat" class="todo-meta">
        <!-- 子任务统计 -->
        <span v-if="subtaskStats.total > 0" class="subtask-count" :class="{ expanded: subtaskExpanded }" @click="toggleSubtaskExpand">
          <el-icon :size="12"><Finished /></el-icon>
          {{ subtaskStats.completed }}/{{ subtaskStats.total }}
          <el-icon :size="10" class="expand-arrow"><ArrowDown v-if="subtaskExpanded" /><ArrowRight v-else /></el-icon>
        </span>

        <!-- 重复提醒标识 -->
        <el-tooltip v-if="isRepeat" :content="repeatTooltip" placement="top" :show-after="300">
          <span class="repeat-badge">
            <el-icon :size="12"><RefreshRight /></el-icon>
          </span>
        </el-tooltip>

        <!-- 通知时间 -->
        <span v-if="formattedNotifyTime" class="notify-time">
          <el-icon :size="12"><Bell /></el-icon>
          {{ formattedNotifyTime }}
        </span>
      </div>

      <!-- 子任务标题列表 -->
      <Transition name="subtask-list">
        <div v-if="subtaskExpanded && subtaskStats.total > 0" class="subtask-list">
          <div
            v-for="subtask in sortedSubtasks"
            :key="subtask.id"
            class="subtask-item"
            :class="{ 'is-completed': subtask.completed }"
          >
            <el-icon :size="12" class="subtask-status-icon">
              <SuccessFilled v-if="subtask.completed" />
              <CirclePlus v-else />
            </el-icon>
            <el-tooltip :content="subtask.title" placement="top" :show-after="500" :disabled="!subtask.title || subtask.title.length < 20">
              <span class="subtask-title">{{ subtask.title }}</span>
            </el-tooltip>
          </div>
        </div>
      </Transition>
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
