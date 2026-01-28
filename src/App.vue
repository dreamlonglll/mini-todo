<script setup lang="ts">
import { ref, onMounted, computed } from 'vue'
import { useTodoStore, useAppStore } from '@/stores'
import TitleBar from '@/components/TitleBar.vue'
import TodoList from '@/components/TodoList.vue'
import TodoEditor from '@/components/TodoEditor.vue'
import SettingsPanel from '@/components/SettingsPanel.vue'
import type { Todo, CreateTodoRequest } from '@/types'

const todoStore = useTodoStore()
const appStore = useAppStore()

// 编辑器状态
const showEditor = ref(false)
const editingTodo = ref<Todo | null>(null)

// 设置面板状态
const showSettings = ref(false)

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

// 打开编辑器
function openEditor(todo?: Todo) {
  editingTodo.value = todo || null
  showEditor.value = true
}

// 关闭编辑器
function closeEditor() {
  showEditor.value = false
  editingTodo.value = null
}

// 打开设置
function openSettings() {
  showSettings.value = true
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

    <!-- 编辑弹窗 -->
    <TodoEditor
      v-model:visible="showEditor"
      :todo="editingTodo"
      @close="closeEditor"
    />

    <!-- 设置面板 -->
    <SettingsPanel
      v-model:visible="showSettings"
    />
  </div>
</template>

<style scoped>
/* 组件特定样式由 main.scss 提供 */
</style>
