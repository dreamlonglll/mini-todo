<script setup lang="ts">
import { onMounted, onUnmounted, computed, ref } from 'vue'
import { useTodoStore, useAppStore } from '@/stores'
import { WebviewWindow } from '@tauri-apps/api/webviewWindow'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { listen } from '@tauri-apps/api/event'
import TitleBar from '@/components/TitleBar.vue'
import TodoList from '@/components/TodoList.vue'
import TodoItem from '@/components/TodoItem.vue'
import type { Todo } from '@/types'

const todoStore = useTodoStore()
const appStore = useAppStore()
const appWindow = getCurrentWindow()

// 已完成区域展开状态
const showCompleted = ref(false)

// 已完成列表
const completedList = computed(() => todoStore.completedTodos)

// 已完成数量
const completedCount = computed(() => todoStore.todoCount.completed)

// 切换已完成区域
function toggleCompleted() {
  showCompleted.value = !showCompleted.value
}

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

// 防抖保存定时器
const saveDebounceTimer = ref<number | null>(null)

// 是否有弹窗打开（模态状态）
const isModalOpen = ref(false)

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
    openEditor(undefined)
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
  if (saveDebounceTimer.value) {
    clearTimeout(saveDebounceTimer.value)
  }
})

// 打开编辑器窗口（模态）
async function openEditor(todo?: Todo) {
  // 如果已有弹窗打开，直接返回
  if (isModalOpen.value) return
  
  const url = todo ? `#/editor?id=${todo.id}` : '#/editor'
  const label = `editor-${Date.now()}`
  
  try {
    isModalOpen.value = true
    
    // 获取主窗口位置，计算弹窗位置
    const mainPos = await appWindow.outerPosition()
    const mainSize = await appWindow.outerSize()
    const editorWidth = 400
    const editorHeight = 500
    
    // 计算弹窗位置：在主窗口中心偏移
    const x = mainPos.x + Math.round((mainSize.width - editorWidth) / 2)
    const y = mainPos.y + Math.round((mainSize.height - editorHeight) / 2)
    
    const webview = new WebviewWindow(label, {
      url,
      title: todo ? '编辑待办' : '新建待办',
      width: editorWidth,
      height: editorHeight,
      x,
      y,
      resizable: true,
      decorations: false,
      transparent: false
    })

    // 监听窗口关闭，刷新待办列表并清除模态状态
    webview.once('tauri://destroyed', async () => {
      isModalOpen.value = false
      await todoStore.fetchTodos()
    })
    
    // 监听创建失败，清除模态状态
    webview.once('tauri://error', () => {
      isModalOpen.value = false
    })
  } catch (e) {
    isModalOpen.value = false
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
    
    // 获取主窗口位置，计算弹窗位置
    const mainPos = await appWindow.outerPosition()
    const mainSize = await appWindow.outerSize()
    const settingsWidth = 350
    const settingsHeight = 400
    
    // 计算弹窗位置：在主窗口中心偏移
    const x = mainPos.x + Math.round((mainSize.width - settingsWidth) / 2)
    const y = mainPos.y + Math.round((mainSize.height - settingsHeight) / 2)
    
    const webview = new WebviewWindow(label, {
      url: '#/settings',
      title: '设置',
      width: settingsWidth,
      height: settingsHeight,
      x,
      y,
      resizable: false,
      decorations: false,
      transparent: false
    })
    
    // 监听窗口关闭，清除模态状态
    webview.once('tauri://destroyed', () => {
      isModalOpen.value = false
    })
    
    // 监听创建失败，清除模态状态
    webview.once('tauri://error', () => {
      isModalOpen.value = false
    })
  } catch (e) {
    isModalOpen.value = false
    console.error('Failed to open settings window:', e)
  }
}
</script>

<template>
  <div :class="containerClass">
    <!-- 模态遮罩层 -->
    <div v-if="isModalOpen" class="modal-overlay"></div>
    
    <!-- 标题栏 -->
    <TitleBar 
      @open-settings="openSettings"
    />

    <!-- 主内容区 -->
    <div class="main-content">
      <!-- 待办列表 -->
      <TodoList
        @edit="openEditor"
      />
    </div>

    <!-- 已完成区域（放在 main-content 外面，固定在底部） -->
    <div v-if="completedCount > 0" class="completed-section">
      <div class="section-header" @click="toggleCompleted">
        <span>已完成 ({{ completedCount }})</span>
        <el-icon class="collapse-icon" :class="{ expanded: showCompleted }" :size="14">
          <ArrowDown />
        </el-icon>
      </div>

      <div v-show="showCompleted" class="completed-list">
        <TodoItem
          v-for="todo in completedList"
          :key="todo.id"
          :todo="todo"
          @click="openEditor(todo)"
          @toggle-complete="handleToggleComplete(todo)"
          @delete="handleDelete(todo)"
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
</style>
