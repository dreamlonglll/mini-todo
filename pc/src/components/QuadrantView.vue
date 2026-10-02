<script setup lang="ts">
import { ref, watch, computed } from 'vue'
import draggable from 'vuedraggable'
import { useTodoStore, useAppStore } from '@/stores'
import TodoItem from './TodoItem.vue'
import type { Todo, QuadrantType } from '@/types'
import { QUADRANT_INFO, QUADRANTS } from '@/types'
import { mergeQuadrantOrder, resolveQuadrantColor } from '@/utils/quadrant'

const emit = defineEmits<{
  (e: 'edit', todo: Todo): void
}>()

const todoStore = useTodoStore()
const appStore = useAppStore()

// 是否深色主题
const isDarkTheme = computed(() => appStore.isDarkTheme)

// 四象限本地数据（用于拖拽）
const quadrantLists = ref<Record<QuadrantType, Todo[]>>({
  [QUADRANTS.IMPORTANT_URGENT]: [],
  [QUADRANTS.IMPORTANT_NOT_URGENT]: [],
  [QUADRANTS.URGENT_NOT_IMPORTANT]: [],
  [QUADRANTS.NOT_URGENT_NOT_IMPORTANT]: [],
})

// 拖拽进行中（SortableJS 拖动中，或松手后的落库中）：期间 store 会多次回流（象限更新、排序，
// 以及轮询 / 聚焦 / 同步触发的刷新），先不按 store 重建本地列表——拖动途中重建会让 Vue 移动
// SortableJS 正在操作的 DOM 节点，落库途中重建会让待办闪回原象限。都结束后再统一按 store 重建一次
let dragging = false
let dragPersisting = 0

// 按 store 重建本地四象限列表
function syncFromStore() {
  const data = todoStore.todosByQuadrant
  quadrantLists.value = {
    [QUADRANTS.IMPORTANT_URGENT]: [...data[QUADRANTS.IMPORTANT_URGENT]],
    [QUADRANTS.IMPORTANT_NOT_URGENT]: [...data[QUADRANTS.IMPORTANT_NOT_URGENT]],
    [QUADRANTS.URGENT_NOT_IMPORTANT]: [...data[QUADRANTS.URGENT_NOT_IMPORTANT]],
    [QUADRANTS.NOT_URGENT_NOT_IMPORTANT]: [...data[QUADRANTS.NOT_URGENT_NOT_IMPORTANT]],
  }
}

function syncIfIdle() {
  if (!dragging && dragPersisting === 0) syncFromStore()
}

// 同步 store 数据到本地。
// 不需要 deep：todosByQuadrant 是 computed，成员、象限、排序变化都会让它产出新对象；
// 列表项内部字段由 TodoItem 直接读同一个待办对象，自然响应
watch(() => todoStore.todosByQuadrant, syncIfIdle, { immediate: true })

function onDragStart() {
  dragging = true
}

// SortableJS 先派发 add / remove / update（对应 change，落库从这里开始），最后才是 end；
// 落库还没结束时由 onDragChange 的 finally 负责重建
function onDragEnd() {
  dragging = false
  syncIfIdle()
}

// 象限配置
const quadrantConfig = computed(() => [
  { ...QUADRANT_INFO[0], position: 'top-left' },
  { ...QUADRANT_INFO[1], position: 'top-right' },
  { ...QUADRANT_INFO[2], position: 'bottom-left' },
  { ...QUADRANT_INFO[3], position: 'bottom-right' },
])

// vuedraggable 的 change 事件负载
interface DragChangeEvent {
  added?: { element: Todo; newIndex: number }
  removed?: { element: Todo; oldIndex: number }
  moved?: { element: Todo; oldIndex: number; newIndex: number }
}

// 拖拽变更处理
//
// vuedraggable 的 change：象限内移动 → moved；跨象限 → 目标象限 added + 源象限 removed。
// 源象限移走一项不改变其余待办的相对顺序，只需处理 moved / added。
async function onDragChange(quadrantId: QuadrantType, evt: DragChangeEvent) {
  if (!evt.added && !evt.moved) return

  // 拖放结果在第一个 await 之前取好：之后 store 回流会改动这些列表
  const quadrantIds = quadrantLists.value[quadrantId].map(t => t.id)
  const globalIds = todoStore.pendingTodos.map(t => t.id)

  dragPersisting++
  try {
    // 处理添加的元素（从其他象限拖入）
    if (evt.added) {
      const todo = evt.added.element
      if (todo.quadrant !== quadrantId) {
        // 颜色跟随象限，除非用户手动挑过颜色（与编辑窗口同一套策略）
        const nextColor = resolveQuadrantColor(todo.color, todo.quadrant, quadrantId)
        const ok = await todoStore.updateTodoQuadrant(todo.id, quadrantId, nextColor)
        // 失败时 store 未变：跳过排序，finally 里按 store 重建，待办退回原象限
        if (!ok) return
      }
    }

    // 只调整本象限内部的先后，合并回全局顺序后整体落库，
    // 列表视图里其它象限待办的相对位置保持不变
    await todoStore.reorderTodos(mergeQuadrantOrder(globalIds, quadrantIds))
  } finally {
    dragPersisting--
    syncIfIdle()
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
  <div class="quadrant-view" :class="{ 'dark-theme': isDarkTheme }">
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
            @start="onDragStart"
            @end="onDragEnd"
            @change="(evt: DragChangeEvent) => onDragChange(quadrant.id, evt)"
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

/* 深色主题样式 */
.quadrant-view.dark-theme {
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
