<script setup lang="ts">
import { onMounted, onUnmounted, computed, ref } from 'vue'
import { useTodoStore, useAppStore } from '@/stores'
import { WebviewWindow } from '@tauri-apps/api/webviewWindow'
import { getCurrentWindow } from '@tauri-apps/api/window'
import TitleBar from '@/components/TitleBar.vue'
import TodoList from '@/components/TodoList.vue'
import type { Todo } from '@/types'

const todoStore = useTodoStore()
const appStore = useAppStore()
const appWindow = getCurrentWindow()

// 容器类名
const containerClass = computed(() => ({
  'app-container': true,
  'fixed-mode': appStore.isFixed
}))

// 事件监听清理函数
let unlistenClose: (() => void) | null = null
let unlistenMoved: (() => void) | null = null
let unlistenResized: (() => void) | null = null

// 防抖保存定时器
const saveDebounceTimer = ref<number | null>(null)

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
})

// 清理
onUnmounted(() => {
  if (unlistenClose) unlistenClose()
  if (unlistenMoved) unlistenMoved()
  if (unlistenResized) unlistenResized()
  if (saveDebounceTimer.value) {
    clearTimeout(saveDebounceTimer.value)
  }
})

// 打开编辑器窗口
async function openEditor(todo?: Todo) {
  const url = todo ? `#/editor?id=${todo.id}` : '#/editor'
  const label = `editor-${Date.now()}`
  
  try {
    const webview = new WebviewWindow(label, {
      url,
      title: todo ? '编辑待办' : '新建待办',
      width: 400,
      height: 500,
      resizable: true,
      center: true,
      decorations: false,
      transparent: false
    })

    // 监听窗口关闭，刷新待办列表
    webview.once('tauri://destroyed', async () => {
      await todoStore.fetchTodos()
    })
  } catch (e) {
    console.error('Failed to open editor window:', e)
  }
}

// 打开设置窗口
async function openSettings() {
  const label = `settings-${Date.now()}`
  
  try {
    new WebviewWindow(label, {
      url: '#/settings',
      title: '设置',
      width: 350,
      height: 400,
      resizable: false,
      center: true,
      decorations: false,
      transparent: false
    })
  } catch (e) {
    console.error('Failed to open settings window:', e)
  }
}
</script>

<template>
  <div :class="containerClass">
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

    <!-- 悬浮添加按钮 -->
    <button class="fab-add" title="新建待办" @click="openEditor()">
      <el-icon :size="24"><Plus /></el-icon>
    </button>
  </div>
</template>

<style scoped>
/* 组件特定样式由 main.scss 提供 */
</style>
