<script setup lang="ts">
import { ref, onMounted, computed, watch } from 'vue'
import { useRoute } from 'vue-router'
import { invoke } from '@tauri-apps/api/core'
import { getCurrentWindow } from '@tauri-apps/api/window'
import type { Todo, Priority, CreateTodoRequest, UpdateTodoRequest, CreateSubTaskRequest } from '@/types'

const route = useRoute()
const todoId = computed(() => route.query.id ? parseInt(route.query.id as string) : null)
const appWindow = getCurrentWindow()

// 表单数据
const form = ref({
  title: '',
  description: '',
  priority: 'medium' as Priority,
  notifyAt: null as string | null,
  notifyBefore: 15
})

// 拆分的日期和时间
const notifyDate = ref<string | null>(null)
const notifyTime = ref<string | null>(null)

// 组合日期和时间生成 notifyAt
function updateNotifyAt() {
  if (notifyDate.value && notifyTime.value) {
    form.value.notifyAt = `${notifyDate.value}T${notifyTime.value}:00`
  } else if (notifyDate.value) {
    form.value.notifyAt = `${notifyDate.value}T09:00:00`
  } else {
    form.value.notifyAt = null
  }
}

// 解析 notifyAt 为日期和时间
function parseNotifyAt(notifyAtValue: string | null) {
  if (notifyAtValue) {
    const [datePart, timePart] = notifyAtValue.split('T')
    notifyDate.value = datePart
    notifyTime.value = timePart ? timePart.substring(0, 5) : '09:00'
  } else {
    notifyDate.value = null
    notifyTime.value = null
  }
}

// 监听日期和时间变化
watch([notifyDate, notifyTime], () => {
  updateNotifyAt()
})

// 待办数据
const todo = ref<Todo | null>(null)

// 新子任务输入
const newSubtaskTitle = ref('')

// 是否编辑模式
const isEdit = computed(() => todoId.value !== null)

// 当前待办的子任务列表
const subtasks = computed(() => todo.value?.subtasks || [])

// 提前通知选项
const notifyBeforeOptions = [
  { label: '准时', value: 0 },
  { label: '5 分钟前', value: 5 },
  { label: '15 分钟前', value: 15 },
  { label: '30 分钟前', value: 30 },
  { label: '1 小时前', value: 60 },
  { label: '自定义', value: -1 }
]

// 自定义提前时间
const customNotifyBefore = ref(15)
const isCustomNotifyBefore = ref(false)

// 原始的通知时间（用于判断是否需要清除）
const originalNotifyAt = ref<string | null>(null)

// 初始化
onMounted(async () => {
  if (todoId.value) {
    await loadTodo()
  }
})

// 加载待办数据
async function loadTodo() {
  if (!todoId.value) return
  
  try {
    const todos = await invoke<Todo[]>('get_todos')
    todo.value = todos.find(t => t.id === todoId.value) || null
    
    if (todo.value) {
      form.value = {
        title: todo.value.title,
        description: todo.value.description || '',
        priority: todo.value.priority as Priority,
        notifyAt: todo.value.notifyAt,
        notifyBefore: todo.value.notifyBefore
      }
      
      // 保存原始的通知时间
      originalNotifyAt.value = todo.value.notifyAt
      
      // 解析日期和时间
      parseNotifyAt(todo.value.notifyAt)
      
      // 检查是否是自定义时间
      const presetValues = [0, 5, 15, 30, 60]
      isCustomNotifyBefore.value = !presetValues.includes(todo.value.notifyBefore)
      if (isCustomNotifyBefore.value) {
        customNotifyBefore.value = todo.value.notifyBefore
      }
    }
  } catch (e) {
    console.error('Failed to load todo:', e)
  }
}

// 处理提前通知选择
function handleNotifyBeforeChange(value: number) {
  if (value === -1) {
    isCustomNotifyBefore.value = true
    form.value.notifyBefore = customNotifyBefore.value
  } else {
    isCustomNotifyBefore.value = false
    form.value.notifyBefore = value
  }
}

// 保存待办
async function handleSave() {
  if (!form.value.title.trim()) return

  if (isCustomNotifyBefore.value) {
    form.value.notifyBefore = customNotifyBefore.value
  }

  try {
    if (isEdit.value && todoId.value) {
      // 判断是否需要清除通知时间（原来有值，现在为空）
      const shouldClearNotifyAt = originalNotifyAt.value !== null && !form.value.notifyAt
      
      const data: UpdateTodoRequest = {
        title: form.value.title,
        description: form.value.description || null,
        priority: form.value.priority,
        notifyAt: form.value.notifyAt || undefined,
        notifyBefore: form.value.notifyBefore,
        clearNotifyAt: shouldClearNotifyAt
      }
      await invoke('update_todo', { id: todoId.value, data })
    } else {
      const data: CreateTodoRequest = {
        title: form.value.title,
        description: form.value.description || undefined,
        priority: form.value.priority,
        notifyAt: form.value.notifyAt || undefined,
        notifyBefore: form.value.notifyBefore
      }
      await invoke('create_todo', { data })
    }

    handleClose()
  } catch (e) {
    console.error('Failed to save:', e)
  }
}

// 添加子任务
async function addSubtask() {
  if (!newSubtaskTitle.value.trim() || !todoId.value) return
  
  try {
    const data: CreateSubTaskRequest = {
      parentId: todoId.value,
      title: newSubtaskTitle.value.trim()
    }
    await invoke('create_subtask', { data })
    await loadTodo()
    newSubtaskTitle.value = ''
  } catch (e) {
    console.error('Failed to add subtask:', e)
  }
}

// 切换子任务完成状态
async function toggleSubtask(subtaskId: number) {
  const subtask = subtasks.value.find(s => s.id === subtaskId)
  if (!subtask) return

  try {
    await invoke('update_subtask', { 
      id: subtaskId, 
      data: { completed: !subtask.completed } 
    })
    await loadTodo()
  } catch (e) {
    console.error('Failed to toggle subtask:', e)
  }
}

// 删除子任务
async function deleteSubtask(subtaskId: number) {
  try {
    await invoke('delete_subtask', { id: subtaskId })
    await loadTodo()
  } catch (e) {
    console.error('Failed to delete subtask:', e)
  }
}

// 关闭窗口
function handleClose() {
  appWindow.close()
}
</script>

<template>
  <div class="editor-window">
    <div class="window-header">
      <h2>{{ isEdit ? '编辑待办' : '新建待办' }}</h2>
      <el-button text @click="handleClose">
        <el-icon><Close /></el-icon>
      </el-button>
    </div>

    <div class="editor-content">
      <el-form label-position="top" :model="form">
        <!-- 标题 -->
        <el-form-item label="标题" required>
          <el-input 
            v-model="form.title" 
            placeholder="请输入待办标题"
            maxlength="100"
          />
        </el-form-item>

        <!-- 描述 -->
        <el-form-item label="描述">
          <el-input 
            v-model="form.description" 
            type="textarea"
            :rows="3"
            placeholder="添加详细描述..."
            maxlength="500"
          />
        </el-form-item>

        <!-- 优先级 -->
        <el-form-item label="优先级">
          <el-radio-group v-model="form.priority">
            <el-radio value="high">
              <span class="priority-label high">高</span>
            </el-radio>
            <el-radio value="medium">
              <span class="priority-label medium">中</span>
            </el-radio>
            <el-radio value="low">
              <span class="priority-label low">低</span>
            </el-radio>
          </el-radio-group>
        </el-form-item>

        <!-- 提醒时间 - 拆分为日期和时间 -->
        <el-form-item label="提醒时间">
          <div class="notify-datetime-picker">
            <el-date-picker
              v-model="notifyDate"
              type="date"
              placeholder="选择日期"
              format="YYYY-MM-DD"
              value-format="YYYY-MM-DD"
              :teleported="true"
              :popper-options="{
                placement: 'top-start',
                modifiers: [{ name: 'flip', enabled: false }]
              }"
              class="date-picker"
            />
            <el-time-picker
              v-model="notifyTime"
              placeholder="时间"
              format="HH:mm"
              value-format="HH:mm"
              :teleported="true"
              :popper-options="{
                placement: 'top-start',
                modifiers: [{ name: 'flip', enabled: false }]
              }"
              class="time-picker"
              :disabled="!notifyDate"
            />
          </div>
        </el-form-item>

        <!-- 提前通知 -->
        <el-form-item v-if="form.notifyAt" label="提前提醒">
          <el-select 
            :model-value="isCustomNotifyBefore ? -1 : form.notifyBefore"
            @change="handleNotifyBeforeChange"
            style="width: 100%"
          >
            <el-option 
              v-for="opt in notifyBeforeOptions"
              :key="opt.value"
              :label="opt.label"
              :value="opt.value"
            />
          </el-select>
          
          <el-input-number
            v-if="isCustomNotifyBefore"
            v-model="customNotifyBefore"
            :min="1"
            :max="1440"
            style="width: 100%; margin-top: 8px"
          >
            <template #suffix>分钟前</template>
          </el-input-number>
        </el-form-item>

        <!-- 子任务 (仅编辑模式) -->
        <el-form-item v-if="isEdit" label="子任务">
          <div class="subtask-editor">
            <div class="add-subtask">
              <el-input
                v-model="newSubtaskTitle"
                placeholder="添加子任务..."
                @keyup.enter="addSubtask"
              >
                <template #append>
                  <el-button @click="addSubtask">
                    <el-icon><Plus /></el-icon>
                  </el-button>
                </template>
              </el-input>
            </div>

            <div class="subtask-list-editor">
              <div 
                v-for="subtask in subtasks" 
                :key="subtask.id" 
                class="subtask-item-editor"
              >
                <el-checkbox 
                  :model-value="subtask.completed"
                  @change="toggleSubtask(subtask.id)"
                />
                <span 
                  class="subtask-title"
                  :class="{ completed: subtask.completed }"
                >
                  {{ subtask.title }}
                </span>
                <el-button 
                  type="danger" 
                  text 
                  size="small"
                  @click="deleteSubtask(subtask.id)"
                >
                  <el-icon><Delete /></el-icon>
                </el-button>
              </div>
            </div>
          </div>
        </el-form-item>
      </el-form>
    </div>

    <div class="window-footer">
      <el-button @click="handleClose">取消</el-button>
      <el-button type="primary" @click="handleSave">
        {{ isEdit ? '保存' : '创建' }}
      </el-button>
    </div>
  </div>
</template>

<style scoped>
.editor-window {
  display: flex;
  flex-direction: column;
  height: 100vh;
  background: #FFFFFF;
}

.window-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 12px 16px;
  border-bottom: 1px solid var(--border);
  -webkit-app-region: drag;

  h2 {
    margin: 0;
    font-size: 16px;
    font-weight: 600;
  }

  .el-button {
    -webkit-app-region: no-drag;
  }
}

.editor-content {
  flex: 1;
  padding: 16px;
  overflow-y: auto;
}

.window-footer {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
  padding: 12px 16px;
  border-top: 1px solid var(--border);
}

.priority-label {
  padding: 2px 8px;
  border-radius: 4px;
  font-size: 12px;

  &.high {
    background: var(--priority-high-bg);
    color: var(--priority-high);
  }

  &.medium {
    background: var(--priority-medium-bg);
    color: var(--priority-medium);
  }

  &.low {
    background: var(--priority-low-bg);
    color: var(--priority-low);
  }
}

.subtask-editor {
  width: 100%;
}

.add-subtask {
  margin-bottom: 12px;
}

.subtask-list-editor {
  max-height: 200px;
  overflow-y: auto;
}

.subtask-item-editor {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 6px 0;
  border-bottom: 1px solid var(--border-light);

  &:last-child {
    border-bottom: none;
  }

  .subtask-title {
    flex: 1;
    font-size: 13px;

    &.completed {
      text-decoration: line-through;
      color: var(--text-tertiary);
    }
  }
}

.notify-datetime-picker {
  display: flex;
  gap: 8px;
  width: 100%;

  .date-picker {
    flex: 1;
  }

  .time-picker {
    width: 100px;
    flex-shrink: 0;
  }
}
</style>
