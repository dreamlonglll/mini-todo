<script setup lang="ts">
import { onMounted, onUnmounted, computed, ref } from 'vue'
import { useTodoStore, useAppStore } from '@/stores'
import { WebviewWindow } from '@tauri-apps/api/webviewWindow'
import { getCurrentWindow, primaryMonitor } from '@tauri-apps/api/window'
import { listen } from '@tauri-apps/api/event'
import TitleBar from '@/components/TitleBar.vue'
import TodoList from '@/components/TodoList.vue'
import TodoItem from '@/components/TodoItem.vue'
import CalendarView from '@/components/CalendarView.vue'
import type { Todo } from '@/types'

const todoStore = useTodoStore()
const appStore = useAppStore()
const appWindow = getCurrentWindow()

// 已完成弹窗显示状态
const showCompletedDialog = ref(false)

// 是否显示日历
const showCalendar = computed(() => appStore.showCalendar)

// 所有待办（用于日历显示）
const allTodos = computed(() => todoStore.todos)

// 已完成列表
const completedList = computed(() => todoStore.completedTodos)

// 已完成数量
const completedCount = computed(() => todoStore.todoCount.completed)


// 切换完成状态
async function handleToggleComplete(todo: Todo) {
  await todoStore.toggleComplete(todo.id)
}

// 删除待办
async function handleDelete(todo: Todo) {
  await todoStore.deleteTodo(todo.id)
}

// 容器类名
const containerClass = computed(() => ({
  'app-container': true,
  'fixed-mode': appStore.isFixed
}))

// 事件监听清理函数
let unlistenClose: (() => void) | null = null
let unlistenMoved: (() => void) | null = null
let unlistenResized: (() => void) | null = null
let unlistenTrayToggle: (() => void) | null = null
let unlistenTrayReset: (() => void) | null = null
let unlistenTrayAddTodo: (() => void) | null = null
let unlistenFocus: (() => void) | null = null

// 防抖保存定时器
const saveDebounceTimer = ref<number | null>(null)

// 是否有弹窗打开（模态状态）
const isModalOpen = ref(false)
let activeModalWindow: WebviewWindow | null = null

async function bringModalToFront() {
  if (!isModalOpen.value || !activeModalWindow) return
  try {
    await activeModalWindow.setFocus()
  } catch (e) {
    console.warn('Failed to focus modal window:', e)
  }
}

// 防抖保存窗口状态（500ms 后保存）
function debouncedSaveState() {
  if (saveDebounceTimer.value) {
    clearTimeout(saveDebounceTimer.value)
  }
  saveDebounceTimer.value = window.setTimeout(async () => {
    await appStore.saveWindowState()
  }, 500)
}

// 初始化
onMounted(async () => {
  await appStore.initSettings()
  await todoStore.fetchTodos()
  
  // 异步检查版本更新（不阻塞主流程）
  appStore.checkForUpdates()
  
  // 监听窗口关闭请求，保存状态
  unlistenClose = await appWindow.onCloseRequested(async () => {
    await appStore.saveWindowState()
  })
  
  // 监听窗口移动事件，自动保存状态（防抖）
  unlistenMoved = await appWindow.onMoved(() => {
    debouncedSaveState()
  })
  
  // 监听窗口调整尺寸事件，自动保存状态（防抖）
  unlistenResized = await appWindow.onResized(() => {
    debouncedSaveState()
  })
  
  // 监听托盘菜单事件
  unlistenTrayToggle = await listen('tray-toggle-fixed', async () => {
    await appStore.toggleFixedMode()
  })
  
  unlistenTrayReset = await listen('tray-reset-window', async () => {
    // 重置后需要更新 appStore 状态并取消固定模式
    if (appStore.isFixed) {
      await appStore.toggleFixedMode()
    }
    await appStore.initSettings()
  })
  
  unlistenTrayAddTodo = await listen('tray-add-todo', () => {
    openEditor(undefined, true) // 从托盘打开时居中于屏幕
  })

  unlistenFocus = await appWindow.onFocusChanged(async ({ payload: focused }) => {
    if (focused && isModalOpen.value) {
      await bringModalToFront()
    }
  })
})

// 清理
onUnmounted(() => {
  if (unlistenClose) unlistenClose()
  if (unlistenMoved) unlistenMoved()
  if (unlistenResized) unlistenResized()
  if (unlistenTrayToggle) unlistenTrayToggle()
  if (unlistenTrayReset) unlistenTrayReset()
  if (unlistenTrayAddTodo) unlistenTrayAddTodo()
  if (unlistenFocus) unlistenFocus()
  if (saveDebounceTimer.value) {
    clearTimeout(saveDebounceTimer.value)
  }
})

// 打开编辑器窗口（模态）
async function openEditor(todo?: Todo, centerOnScreen = false) {
  // 如果已有弹窗打开，直接返回
  if (isModalOpen.value) return
  
  const url = todo ? `#/editor?id=${todo.id}` : '#/editor'
  const label = `editor-${Date.now()}`
  
  try {
    isModalOpen.value = true
    
    const editorWidth = 500
    const editorHeight = 600
    let x: number, y: number
    const scaleFactor = await appWindow.scaleFactor()
    
    if (centerOnScreen) {
      // 在主屏幕正中间打开
      const monitor = await primaryMonitor()
      if (monitor) {
        // 使用 monitor 自带的 scaleFactor 进行转换
        const monitorScale = monitor.scaleFactor
        const monitorX = monitor.position.x / monitorScale
        const monitorY = monitor.position.y / monitorScale
        const monitorW = monitor.size.width / monitorScale
        const monitorH = monitor.size.height / monitorScale
        x = Math.round(monitorX + (monitorW - editorWidth) / 2)
        y = Math.round(monitorY + (monitorH - editorHeight) / 2)
      } else {
        // fallback: 使用主窗口中心
        const mainPos = await appWindow.outerPosition()
        const mainSize = await appWindow.outerSize()
        const mainX = mainPos.x / scaleFactor
        const mainY = mainPos.y / scaleFactor
        const mainW = mainSize.width / scaleFactor
        const mainH = mainSize.height / scaleFactor
        x = Math.round(mainX + (mainW - editorWidth) / 2)
        y = Math.round(mainY + (mainH - editorHeight) / 2)
      }
    } else {
      // 在主窗口中心打开（考虑 DPI 缩放）
      const mainPos = await appWindow.outerPosition()
      const mainSize = await appWindow.outerSize()
      x = Math.round(mainPos.x / scaleFactor + (mainSize.width / scaleFactor - editorWidth) / 2)
      y = Math.round(mainPos.y / scaleFactor + (mainSize.height / scaleFactor - editorHeight) / 2)
    }
    
    const webview = new WebviewWindow(label, {
      url,
      title: todo ? '编辑待办' : '新建待办',
      width: editorWidth,
      height: editorHeight,
      x,
      y,
      resizable: true,
      decorations: false,
      transparent: false,
      parent: centerOnScreen ? undefined : appWindow
    })
    activeModalWindow = webview

    // 监听窗口关闭，刷新待办列表并清除模态状态
    webview.once('tauri://destroyed', async () => {
      isModalOpen.value = false
      activeModalWindow = null
      await todoStore.fetchTodos()
    })
    
    // 监听创建失败，清除模态状态
    webview.once('tauri://error', () => {
      isModalOpen.value = false
      activeModalWindow = null
    })
  } catch (e) {
    isModalOpen.value = false
    activeModalWindow = null
    console.error('Failed to open editor window:', e)
  }
}

// 打开设置窗口（模态）
async function openSettings() {
  // 如果已有弹窗打开，直接返回
  if (isModalOpen.value) return
  
  const label = `settings-${Date.now()}`
  
  try {
    isModalOpen.value = true
    
    // 获取主窗口位置、大小和缩放因子
    const mainPos = await appWindow.outerPosition()
    const mainSize = await appWindow.outerSize()
    const scaleFactor = await appWindow.scaleFactor()
    const settingsWidth = 350
    const settingsHeight = 400
    
    // 计算弹窗位置：主窗口正中间（考虑 DPI 缩放）
    const mainX = mainPos.x / scaleFactor
    const mainY = mainPos.y / scaleFactor
    const mainW = mainSize.width / scaleFactor
    const mainH = mainSize.height / scaleFactor
    const x = Math.round(mainX + (mainW - settingsWidth) / 2)
    const y = Math.round(mainY + (mainH - settingsHeight) / 2)
    
    const webview = new WebviewWindow(label, {
      url: '#/settings',
      title: '设置',
      width: settingsWidth,
      height: settingsHeight,
      x,
      y,
      resizable: false,
      decorations: false,
      transparent: false,
      parent: appWindow
    })
    activeModalWindow = webview
    
    // 监听窗口关闭，清除模态状态并重新加载设置
    webview.once('tauri://destroyed', async () => {
      isModalOpen.value = false
      activeModalWindow = null
      // 重新加载日历显示设置（因为设置窗口是独立的 store 实例）
      await appStore.loadShowCalendar()
    })
    
    // 监听创建失败，清除模态状态
    webview.once('tauri://error', () => {
      isModalOpen.value = false
      activeModalWindow = null
    })
  } catch (e) {
    isModalOpen.value = false
    activeModalWindow = null
    console.error('Failed to open settings window:', e)
  }
}
</script>

<template>
  <div :class="[containerClass, { 'with-calendar': showCalendar }]">
    <!-- 模态遮罩层 -->
    <div v-if="isModalOpen" class="modal-overlay" @mousedown="bringModalToFront"></div>
    
    <!-- 标题栏 -->
    <TitleBar 
      @open-settings="openSettings"
    />

    <!-- 主内容区 - 分栏布局 -->
    <div class="main-body" :class="{ 'split-layout': showCalendar }">
      <!-- 左侧：待办列表 -->
      <div class="left-panel">
        <div class="main-content">
          <!-- 待办列表 -->
          <TodoList
            @edit="openEditor"
          />
        </div>

        <!-- 已完成按钮 -->
        <div v-if="completedCount > 0" class="completed-btn-wrapper">
          <button class="completed-btn" @click="showCompletedDialog = true">
            <span>已完成 ({{ completedCount }})</span>
            <el-icon :size="14"><ArrowRight /></el-icon>
          </button>
        </div>
      </div>

      <!-- 已完成弹窗 -->
      <el-dialog
        v-model="showCompletedDialog"
        title="已完成"
        width="450"
        :modal="true"
        append-to-body
      >
        <div class="completed-dialog-list">
          <TodoItem
            v-for="todo in completedList"
            :key="todo.id"
            :todo="todo"
            @click="openEditor(todo)"
            @toggle-complete="handleToggleComplete(todo)"
            @delete="handleDelete(todo)"
          />
        </div>
      </el-dialog>

      <!-- 右侧：日历视图 -->
      <div v-if="showCalendar" class="right-panel" :class="{ 'fixed-mode': appStore.isFixed }">
        <CalendarView 
          :todos="allTodos"
          :is-fixed="appStore.isFixed"
          @select-todo="openEditor"
        />
      </div>
    </div>

    <!-- 悬浮添加按钮（固定模式下隐藏） -->
    <button 
      v-if="!appStore.isFixed" 
      class="fab-add" 
      title="新建待办" 
      @click="openEditor()"
    >
      <el-icon :size="24"><Plus /></el-icon>
    </button>
  </div>
</template>

<style scoped>
.modal-overlay {
  position: fixed;
  top: 0;
  left: 0;
  right: 0;
  bottom: 0;
  background: rgba(0, 0, 0, 0.3);
  z-index: 999;
  cursor: not-allowed;
}

/* 分栏布局 */
.main-body {
  flex: 1;
  display: flex;
  flex-direction: column;
  overflow: hidden;

  &.split-layout {
    flex-direction: row;
  }
}

.left-panel {
  display: flex;
  flex-direction: column;
  overflow: hidden;

  .split-layout & {
    width: 40%;
    min-width: 280px;
    /* 去掉分割线 */
  }
}

.right-panel {
  flex: 1;
  overflow: hidden;
  padding: 12px;
  background: transparent;

  &.fixed-mode {
    background: transparent;
    padding: 8px;
  }
}

/* 已完成按钮 */
.completed-btn-wrapper {
  padding: 8px 16px;
}

.completed-btn {
  display: flex;
  align-items: center;
  justify-content: space-between;
  width: 100%;
  padding: 8px 12px;
  background: transparent;
  border: 1px solid rgba(128, 128, 128, 0.2);
  border-radius: 6px;
  color: var(--text-secondary);
  font-size: 13px;
  cursor: pointer;
  transition: all 0.2s;

  &:hover {
    background: rgba(128, 128, 128, 0.1);
    color: var(--text-primary);
  }
}

/* 已完成弹窗列表 */
.completed-dialog-list {
  max-height: 400px;
  overflow-y: auto;
}
</style>
