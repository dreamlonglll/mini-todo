<script setup lang="ts">
import { ref, watch, computed } from 'vue'
import draggable from 'vuedraggable'
import { useTodoStore, useAppStore } from '@/stores'
import TodoItem from './TodoItem.vue'
import type { Todo, QuadrantType } from '@/types'
import { QUADRANT_INFO, QUADRANTS } from '@/types'

const emit = defineEmits<{
  (e: 'edit', todo: Todo): void
}>()

const todoStore = useTodoStore()
const appStore = useAppStore()

// 是否固定模式
const isFixed = computed(() => appStore.isFixed)

// 四象限本地数据（用于拖拽）
const quadrantLists = ref<Record<QuadrantType, Todo[]>>({
  [QUADRANTS.IMPORTANT_URGENT]: [],
  [QUADRANTS.IMPORTANT_NOT_URGENT]: [],
  [QUADRANTS.URGENT_NOT_IMPORTANT]: [],
  [QUADRANTS.NOT_URGENT_NOT_IMPORTANT]: [],
})

// 同步 store 数据到本地
watch(
  () => todoStore.todosByQuadrant,
  (newData) => {
    quadrantLists.value = {
      [QUADRANTS.IMPORTANT_URGENT]: [...newData[QUADRANTS.IMPORTANT_URGENT]],
      [QUADRANTS.IMPORTANT_NOT_URGENT]: [...newData[QUADRANTS.IMPORTANT_NOT_URGENT]],
      [QUADRANTS.URGENT_NOT_IMPORTANT]: [...newData[QUADRANTS.URGENT_NOT_IMPORTANT]],
      [QUADRANTS.NOT_URGENT_NOT_IMPORTANT]: [...newData[QUADRANTS.NOT_URGENT_NOT_IMPORTANT]],
    }
  },
  { immediate: true, deep: true }
)

// 象限配置
const quadrantConfig = computed(() => [
  { ...QUADRANT_INFO[0], position: 'top-left' },
  { ...QUADRANT_INFO[1], position: 'top-right' },
  { ...QUADRANT_INFO[2], position: 'bottom-left' },
  { ...QUADRANT_INFO[3], position: 'bottom-right' },
])

// 拖拽变更处理（当元素被添加到象限时）
async function onDragChange(quadrantId: QuadrantType, evt: any) {
  // 处理添加的元素（从其他象限拖入）
  if (evt.added) {
    const todo = evt.added.element as Todo
    if (todo.quadrant !== quadrantId) {
      await todoStore.updateTodoQuadrant(todo.id, quadrantId)
      // 同时更新本地对象的 quadrant 属性
      todo.quadrant = quadrantId
    }
  }
  
  // 重新排序当前象限内的待办
  const ids = quadrantLists.value[quadrantId].map(t => t.id)
  if (ids.length > 0) {
    await todoStore.reorderTodos(ids)
  }
}

// 拖拽组配置（允许在四个象限间拖拽）
const dragGroup = {
  name: 'quadrant-todos',
  pull: true,
  put: true
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

// 获取象限样式
function getQuadrantStyle(quadrant: typeof quadrantConfig.value[0]) {
  return {
    '--quadrant-color': quadrant.color,
    '--quadrant-bg': quadrant.bgColor,
  }
}
</script>

<template>
  <div class="quadrant-view" :class="{ 'fixed-mode': isFixed }">
    <div class="quadrant-grid">
      <div 
        v-for="quadrant in quadrantConfig" 
        :key="quadrant.id"
        class="quadrant-cell"
        :class="[quadrant.position]"
        :style="getQuadrantStyle(quadrant)"
      >
        <!-- 象限标题 -->
        <div class="quadrant-header">
          <span class="quadrant-indicator" :style="{ backgroundColor: quadrant.color }"></span>
          <span class="quadrant-title">{{ quadrant.name }}</span>
          <span class="quadrant-count">{{ quadrantLists[quadrant.id].length }}</span>
        </div>

        <!-- 象限内容（可拖拽） -->
        <div class="quadrant-content">
          <draggable
            v-model="quadrantLists[quadrant.id]"
            :group="dragGroup"
            item-key="id"
            handle=".color-dot"
            ghost-class="dragging"
            :animation="200"
            :force-fallback="true"
            class="quadrant-list"
            @change="(evt: any) => onDragChange(quadrant.id, evt)"
          >
            <template #item="{ element }">
              <TodoItem
                :todo="element"
                class="quadrant-todo-item"
                @click="handleEdit(element)"
                @toggle-complete="handleToggleComplete(element)"
                @delete="handleDelete(element)"
              />
            </template>
          </draggable>

          <!-- 空状态 -->
          <div v-if="quadrantLists[quadrant.id].length === 0" class="quadrant-empty">
            <span>暂无待办</span>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.quadrant-view {
  width: 100%;
  height: 100%;
  padding: 8px;
  box-sizing: border-box;
}

.quadrant-grid {
  display: grid;
  grid-template-columns: 1fr 1fr;
  grid-template-rows: 1fr 1fr;
  gap: 8px;
  height: 100%;
}

.quadrant-cell {
  display: flex;
  flex-direction: column;
  background: var(--quadrant-bg, rgba(128, 128, 128, 0.05));
  border-radius: 8px;
  overflow: hidden;
  border: 1px solid rgba(128, 128, 128, 0.1);
  transition: all 0.2s ease;

  &:hover {
    border-color: var(--quadrant-color, rgba(128, 128, 128, 0.2));
  }
}

/* 固定模式样式 */
.quadrant-view.fixed-mode {
  .quadrant-cell {
    background: rgba(0, 0, 0, 0.15);
    border-color: rgba(255, 255, 255, 0.1);

    &:hover {
      border-color: rgba(255, 255, 255, 0.2);
    }
  }

  .quadrant-header {
    background: rgba(0, 0, 0, 0.1);
  }

  .quadrant-title {
    color: var(--text-primary);
  }

  .quadrant-count {
    background: rgba(255, 255, 255, 0.1);
    color: var(--text-secondary);
  }

  .quadrant-empty {
    color: var(--text-tertiary);
  }
}

.quadrant-header {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 8px 10px;
  background: rgba(128, 128, 128, 0.05);
  border-bottom: 1px solid rgba(128, 128, 128, 0.1);
  flex-shrink: 0;
}

.quadrant-indicator {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  flex-shrink: 0;
}

.quadrant-title {
  font-size: 12px;
  font-weight: 500;
  color: var(--text-secondary);
  flex: 1;
}

.quadrant-count {
  font-size: 11px;
  padding: 1px 6px;
  background: rgba(128, 128, 128, 0.1);
  border-radius: 10px;
  color: var(--text-tertiary);
}

.quadrant-content {
  flex: 1;
  overflow-y: auto;
  overflow-x: hidden;
  min-height: 0;
}

.quadrant-list {
  padding: 4px;
  min-height: 100%;
}

.quadrant-todo-item {
  margin-bottom: 4px;

  &:last-child {
    margin-bottom: 0;
  }
}

/* 拖拽中样式 */
:deep(.dragging) {
  opacity: 0.5;
  background: var(--quadrant-bg) !important;
}

.quadrant-empty {
  display: flex;
  align-items: center;
  justify-content: center;
  height: 100%;
  min-height: 60px;
  color: var(--text-tertiary);
  font-size: 12px;
}

/* 自定义滚动条 */
.quadrant-content {
  &::-webkit-scrollbar {
    width: 4px;
  }

  &::-webkit-scrollbar-track {
    background: transparent;
  }

  &::-webkit-scrollbar-thumb {
    background: rgba(128, 128, 128, 0.2);
    border-radius: 2px;

    &:hover {
      background: rgba(128, 128, 128, 0.3);
    }
  }
}
</style>
