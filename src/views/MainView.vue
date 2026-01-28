<script setup lang="ts">
import { ref, onMounted, computed } from 'vue'
import { useTodoStore, useAppStore } from '@/stores'
import { WebviewWindow } from '@tauri-apps/api/webviewWindow'
import TitleBar from '@/components/TitleBar.vue'
import TodoList from '@/components/TodoList.vue'
import type { Todo, CreateTodoRequest } from '@/types'

const todoStore = useTodoStore()
const appStore = useAppStore()

// 快速添加
const quickAddText = ref('')

// 容器类名
const containerClass = computed(() => ({
  'app-container': true,
  'fixed-mode': appStore.isFixed
}))

// 初始化
onMounted(async () => {
  await appStore.initSettings()
  await todoStore.fetchTodos()
})

// 快速添加待办
async function handleQuickAdd() {
  if (!quickAddText.value.trim()) return
  
  const data: CreateTodoRequest = {
    title: quickAddText.value.trim(),
    priority: 'medium'
  }
  
  await todoStore.addTodo(data)
  quickAddText.value = ''
}

// 打开编辑器窗口
async function openEditor(todo?: Todo) {
  const url = todo ? `#/editor?id=${todo.id}` : '#/editor'
  const label = `editor-${Date.now()}`
  
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
}

// 打开设置窗口
async function openSettings() {
  const label = `settings-${Date.now()}`
  
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
      <!-- 快速添加 -->
      <div class="quick-add">
        <el-icon class="add-icon" :size="16">
          <Plus />
        </el-icon>
        <input
          v-model="quickAddText"
          class="add-input"
          placeholder="添加新待办..."
          @keyup.enter="handleQuickAdd"
        />
      </div>

      <!-- 待办列表 -->
      <TodoList
        @edit="openEditor"
      />
    </div>
  </div>
</template>

<style scoped>
/* 组件特定样式由 main.scss 提供 */
</style>
