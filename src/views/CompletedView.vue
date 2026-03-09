<script setup lang="ts">
import { ref, computed, onMounted, onBeforeUnmount } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { WebviewWindow } from '@tauri-apps/api/webviewWindow'
import { currentMonitor, primaryMonitor } from '@tauri-apps/api/window'
import dayjs from 'dayjs'
import type { Todo } from '@/types'

const appWindow = getCurrentWindow()
const searchQuery = ref('')
const completedTodos = ref<Todo[]>([])
let unlisteners: UnlistenFn[] = []

const filteredTodos = computed(() => {
  const q = searchQuery.value.trim().toLowerCase()
  if (!q) return completedTodos.value
  return completedTodos.value.filter(t =>
    t.title.toLowerCase().includes(q) ||
    (t.description && t.description.toLowerCase().includes(q))
  )
})

function formatTime(time: string) {
  return dayjs(time).format('MM-DD HH:mm')
}

async function loadCompletedTodos() {
  try {
    const todos = await invoke<Todo[]>('get_todos')
    completedTodos.value = todos
      .filter(t => t.completed)
      .sort((a, b) => {
        const timeA = a.updatedAt || a.createdAt
        const timeB = b.updatedAt || b.createdAt
        return timeB.localeCompare(timeA)
      })
  } catch (e) {
    console.error('Failed to load completed todos:', e)
  }
}

async function handleToggleComplete(todo: Todo) {
  try {
    await invoke('toggle_complete', { id: todo.id })
    await loadCompletedTodos()
  } catch (e) {
    console.error('Failed to toggle complete:', e)
  }
}

async function handleDelete(todo: Todo) {
  try {
    await invoke('delete_todo', { id: todo.id })
    await loadCompletedTodos()
  } catch (e) {
    console.error('Failed to delete todo:', e)
  }
}

async function openEditor(todo: Todo) {
  const url = `#/editor?id=${todo.id}`
  const label = `editor-${Date.now()}`

  try {
    const editorWidth = 1080
    const editorHeight = 600
    let x: number, y: number

    const monitor = await currentMonitor() || await primaryMonitor()
    if (monitor) {
      const s = monitor.scaleFactor
      const mx = monitor.position.x / s
      const my = monitor.position.y / s
      const mw = monitor.size.width / s
      const mh = monitor.size.height / s
      x = Math.round(mx + (mw - editorWidth) / 2)
      y = Math.round(my + (mh - editorHeight) / 2)
    } else {
      const s = await appWindow.scaleFactor()
      const pos = await appWindow.outerPosition()
      const size = await appWindow.outerSize()
      x = Math.round(pos.x / s + (size.width / s - editorWidth) / 2)
      y = Math.round(pos.y / s + (size.height / s - editorHeight) / 2)
    }

    const webview = new WebviewWindow(label, {
      url,
      title: '编辑待办',
      width: editorWidth,
      height: editorHeight,
      x,
      y,
      resizable: true,
      decorations: false,
      transparent: false,
    })

    webview.once('tauri://destroyed', async () => {
      await loadCompletedTodos()
    })
  } catch (e) {
    console.error('Failed to open editor:', e)
  }
}

function onTitleBarMouseDown(e: MouseEvent) {
  if ((e.target as HTMLElement).closest('.window-controls')) return
  appWindow.startDragging()
}

function handleClose() {
  appWindow.close()
}

onMounted(async () => {
  await loadCompletedTodos()

  const unlisten = await listen('todo-updated', async () => {
    await loadCompletedTodos()
  })
  unlisteners.push(unlisten)
})

onBeforeUnmount(() => {
  unlisteners.forEach(fn => fn())
})
</script>

<template>
  <div class="completed-window">
    <div class="title-bar" @mousedown="onTitleBarMouseDown">
      <div class="title-text">
        <span>已完成</span>
        <span class="count-badge">{{ filteredTodos.length }}</span>
      </div>
      <div class="window-controls">
        <button class="close-btn" @click="handleClose">
          <svg width="10" height="10" viewBox="0 0 10 10">
            <path d="M1 1L9 9M9 1L1 9" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"/>
          </svg>
        </button>
      </div>
    </div>

    <div class="search-bar">
      <div class="search-input-wrapper">
        <el-icon class="search-icon" :size="16"><Search /></el-icon>
        <input
          v-model="searchQuery"
          class="search-input"
          placeholder="搜索已完成的待办..."
          spellcheck="false"
        />
        <button
          v-if="searchQuery"
          class="search-clear"
          @click="searchQuery = ''"
        >
          <el-icon :size="14"><Close /></el-icon>
        </button>
      </div>
    </div>

    <div class="completed-list">
      <div
        v-for="todo in filteredTodos"
        :key="todo.id"
        class="completed-item"
        @click="openEditor(todo)"
      >
        <div class="item-color" :style="{ backgroundColor: todo.color }"></div>

        <div class="item-content">
          <div class="item-title">{{ todo.title }}</div>
          <div v-if="todo.notifyAt" class="item-time">
            <el-icon :size="12"><Clock /></el-icon>
            {{ formatTime(todo.notifyAt) }}
          </div>
        </div>

        <div class="item-actions">
          <el-tooltip content="恢复为未完成" placement="top">
            <button class="action-btn restore" @click.stop="handleToggleComplete(todo)">
              <el-icon :size="16"><RefreshLeft /></el-icon>
            </button>
          </el-tooltip>
          <el-tooltip content="删除" placement="top">
            <button class="action-btn delete" @click.stop="handleDelete(todo)">
              <el-icon :size="16"><Delete /></el-icon>
            </button>
          </el-tooltip>
        </div>
      </div>

      <div v-if="filteredTodos.length === 0 && searchQuery" class="empty-state">
        <el-icon :size="40"><Search /></el-icon>
        <span>未找到匹配的待办</span>
      </div>

      <div v-if="filteredTodos.length === 0 && !searchQuery" class="empty-state">
        <el-icon :size="40"><Check /></el-icon>
        <span>暂无已完成的待办</span>
      </div>
    </div>
  </div>
</template>

<style scoped>
.completed-window {
  display: flex;
  flex-direction: column;
  height: 100vh;
  background: #ffffff;
  overflow: hidden;
}

.title-bar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  height: 42px;
  padding: 0 12px 0 16px;
  background: #f8fafc;
  border-bottom: 1px solid #e2e8f0;
  user-select: none;
  -webkit-app-region: drag;
  flex-shrink: 0;
}

.title-text {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 14px;
  font-weight: 600;
  color: #1e293b;
}

.count-badge {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  min-width: 20px;
  height: 20px;
  padding: 0 6px;
  border-radius: 10px;
  background: #e2e8f0;
  font-size: 12px;
  font-weight: 500;
  color: #64748b;
}

.window-controls {
  display: flex;
  align-items: center;
  -webkit-app-region: no-drag;
}

.close-btn {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 28px;
  height: 28px;
  border: none;
  border-radius: 6px;
  background: transparent;
  color: #94a3b8;
  cursor: pointer;
  transition: all 0.15s ease;

  &:hover {
    background: #fee2e2;
    color: #ef4444;
  }
}

.search-bar {
  padding: 12px 16px;
  flex-shrink: 0;
}

.search-input-wrapper {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 0 12px;
  height: 36px;
  background: #f1f5f9;
  border-radius: 8px;
  border: 1px solid transparent;
  transition: all 0.2s ease;

  &:focus-within {
    background: #ffffff;
    border-color: #93c5fd;
    box-shadow: 0 0 0 3px rgba(59, 130, 246, 0.1);
  }
}

.search-icon {
  color: #94a3b8;
  flex-shrink: 0;
}

.search-input {
  flex: 1;
  border: none;
  outline: none;
  background: transparent;
  font-size: 13px;
  color: #334155;
  line-height: 1;

  &::placeholder {
    color: #94a3b8;
  }
}

.search-clear {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 20px;
  height: 20px;
  border: none;
  border-radius: 4px;
  background: transparent;
  color: #94a3b8;
  cursor: pointer;
  flex-shrink: 0;

  &:hover {
    background: #e2e8f0;
    color: #64748b;
  }
}

.completed-list {
  flex: 1;
  overflow-y: auto;
  padding: 0 16px 16px;
}

.completed-item {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 12px 16px;
  margin-bottom: 8px;
  background: #f8fafc;
  border-radius: 10px;
  cursor: pointer;
  transition: all 0.2s ease;

  &:last-child {
    margin-bottom: 0;
  }

  &:hover {
    background: #f1f5f9;

    .item-actions {
      opacity: 1;
    }
  }
}

.item-color {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  flex-shrink: 0;
}

.item-content {
  flex: 1;
  min-width: 0;
}

.item-title {
  font-size: 14px;
  color: #64748b;
  text-decoration: line-through;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.item-time {
  display: flex;
  align-items: center;
  gap: 4px;
  margin-top: 4px;
  font-size: 12px;
  color: #94a3b8;
}

.item-actions {
  display: flex;
  align-items: center;
  gap: 4px;
  opacity: 0;
  transition: opacity 0.15s ease;
}

.action-btn {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 32px;
  height: 32px;
  border: none;
  border-radius: 6px;
  cursor: pointer;
  transition: all 0.15s ease;

  &.restore {
    background: #dbeafe;
    color: #3b82f6;

    &:hover {
      background: #bfdbfe;
      color: #2563eb;
    }
  }

  &.delete {
    background: #fee2e2;
    color: #ef4444;

    &:hover {
      background: #fecaca;
      color: #dc2626;
    }
  }
}

.empty-state {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  padding: 60px 20px;
  color: #94a3b8;
  text-align: center;

  .el-icon {
    margin-bottom: 12px;
    opacity: 0.5;
  }

  span {
    font-size: 14px;
  }
}
</style>
