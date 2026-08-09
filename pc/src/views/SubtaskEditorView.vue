<script setup lang="ts">
import { ref, onMounted } from 'vue'
import { useRoute } from 'vue-router'
import { invoke } from '@tauri-apps/api/core'
import { getCurrentWindow } from '@tauri-apps/api/window'
import MarkdownEditor from '@/components/MarkdownEditor.vue'

const route = useRoute()
const subtaskId = parseInt(route.query.id as string)
const isViewMode = route.query.mode === 'view'
const appWindow = getCurrentWindow()

const title = ref('')
const markdownContent = ref('')

async function loadSubtask() {
  try {
    const result = await invoke<{ title: string; content: string | null }>('get_subtask', { id: subtaskId })
    title.value = result.title
    markdownContent.value = result.content || ''
  } catch (e) {
    console.error('Failed to load subtask:', e)
  }
}

async function handleSave() {
  if (!title.value.trim()) return
  try {
    await invoke('update_subtask', {
      id: subtaskId,
      data: {
        title: title.value.trim(),
        content: markdownContent.value,
      }
    })
    appWindow.close()
  } catch (e) {
    console.error('Failed to save subtask:', e)
  }
}

function handleClose() {
  appWindow.close()
}

async function handleMaximize() {
  const maximized = await appWindow.isMaximized()
  if (maximized) {
    await appWindow.unmaximize()
  } else {
    await appWindow.maximize()
  }
}

function onHeaderMouseDown(e: MouseEvent) {
  if (e.buttons !== 1) return
  const target = e.target as HTMLElement
  if (target.closest('[data-tauri-drag-region="false"]')) return
  if (target.closest('button, input, textarea, select, a, [role="button"]')) return
  e.preventDefault()
  appWindow.startDragging()
}

onMounted(async () => {
  await loadSubtask()
})
</script>

<template>
  <div class="subtask-editor-window">
    <div class="window-header" data-tauri-drag-region="deep" @mousedown="onHeaderMouseDown">
      <h2>{{ isViewMode ? '查看子任务' : '编辑子任务' }}</h2>
      <div class="window-controls" data-tauri-drag-region="false">
        <button class="control-btn maximize-btn" title="最大化" @click="handleMaximize">
          <el-icon :size="14"><FullScreen /></el-icon>
        </button>
        <button class="control-btn close-btn" title="关闭" @click="handleClose">
          <el-icon :size="14"><Close /></el-icon>
        </button>
      </div>
    </div>

    <div class="editor-content">
      <div class="form-field">
        <label class="field-label">标题</label>
        <el-input
          v-model="title"
          placeholder="请输入子任务标题"
          maxlength="200"
          :disabled="isViewMode"
        />
      </div>

      <div class="form-field editor-field">
        <label class="field-label">内容 (Markdown)</label>
        <MarkdownEditor
          v-model="markdownContent"
          :readonly="isViewMode"
          class="milkdown-editor-wrapper"
        />
      </div>
    </div>

    <div class="window-footer">
      <div class="footer-right">
        <el-button @click="handleClose">
          <el-icon><Close /></el-icon>
          {{ isViewMode ? '关闭' : '取消' }}
        </el-button>
        <el-button v-if="!isViewMode" type="primary" :disabled="!title.trim()" @click="handleSave">
          <el-icon><Check /></el-icon>
          保存
        </el-button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.subtask-editor-window {
  display: flex;
  flex-direction: column;
  height: 100vh;
  background: #ffffff;
}

.window-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 12px 16px;
  min-height: 44px;
  box-sizing: border-box;
  border-bottom: 1px solid var(--border, #e2e8f0);
  -webkit-app-region: drag;

  h2 {
    margin: 0;
    font-size: 16px;
    font-weight: 600;
    line-height: 1.2;
  }
}

.window-controls {
  display: flex;
  align-items: center;
  gap: 2px;
  -webkit-app-region: no-drag;
}

.control-btn {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 32px;
  height: 28px;
  border: none;
  background: transparent;
  cursor: pointer;
  border-radius: 4px;
  color: #64748b;
  transition: all 0.15s ease;

  &:hover {
    background: #f1f5f9;
    color: #334155;
  }

  &.close-btn:hover {
    background: #fee2e2;
    color: #ef4444;
  }
}

.editor-content {
  flex: 1;
  padding: 16px;
  overflow-y: auto;
  display: flex;
  flex-direction: column;
  gap: 16px;
}

.window-footer {
  display: flex;
  justify-content: flex-end;
  align-items: center;
  padding: 12px 16px;
  border-top: 1px solid var(--border, #e2e8f0);
}

.footer-right {
  display: flex;
  gap: 8px;
}

.form-field {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.field-label {
  font-size: 13px;
  font-weight: 600;
  color: #334155;
}

.editor-field {
  flex: 1;
  display: flex;
  flex-direction: column;
}

.milkdown-editor-wrapper {
  flex: 1;
  border: 1px solid #e2e8f0;
  border-radius: 8px;
  min-height: 300px;
  overflow-y: auto;
  background: #ffffff;
}

.milkdown-editor-wrapper :deep(.milkdown) {
  min-height: 290px;
}

.milkdown-editor-wrapper :deep(.ProseMirror) {
  min-height: 280px;
}
</style>
