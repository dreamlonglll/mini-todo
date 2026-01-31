<script setup lang="ts">
import { ref, onMounted, computed, watch } from 'vue'
import { useRoute } from 'vue-router'
import { invoke } from '@tauri-apps/api/core'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { ElMessageBox } from 'element-plus'
import type { Todo, CreateTodoRequest, UpdateTodoRequest, CreateSubTaskRequest } from '@/types'
import { DEFAULT_COLOR, PRESET_COLORS } from '@/types'

const route = useRoute()
const todoId = computed(() => route.query.id ? parseInt(route.query.id as string) : null)
const appWindow = getCurrentWindow()

// 表单数据
const form = ref({
  title: '',
  description: '',
  color: DEFAULT_COLOR,
  notifyAt: null as string | null,
  notifyBefore: 15,
  startTime: null as string | null,
  endTime: null as string | null
})


// 开始和截止时间的日期时间组件值
const startDate = ref<string | null>(null)
const startTimeValue = ref<string | null>(null)
const endDate = ref<string | null>(null)
const endTimeValue = ref<string | null>(null)

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

// 组合开始日期和时间
function updateStartTime() {
  if (startDate.value && startTimeValue.value) {
    form.value.startTime = `${startDate.value}T${startTimeValue.value}:00`
  } else if (startDate.value) {
    form.value.startTime = `${startDate.value}T00:00:00`
  } else {
    form.value.startTime = null
  }
}

// 组合截止日期和时间
function updateEndTime() {
  if (endDate.value && endTimeValue.value) {
    form.value.endTime = `${endDate.value}T${endTimeValue.value}:00`
  } else if (endDate.value) {
    form.value.endTime = `${endDate.value}T23:59:00`
  } else {
    form.value.endTime = null
  }
}

// 解析开始时间
function parseStartTime(startTimeStr: string | null) {
  if (startTimeStr) {
    const [datePart, timePart] = startTimeStr.split('T')
    startDate.value = datePart
    startTimeValue.value = timePart ? timePart.substring(0, 5) : '00:00'
  } else {
    startDate.value = null
    startTimeValue.value = null
  }
}

// 解析截止时间
function parseEndTime(endTimeStr: string | null) {
  if (endTimeStr) {
    const [datePart, timePart] = endTimeStr.split('T')
    endDate.value = datePart
    endTimeValue.value = timePart ? timePart.substring(0, 5) : '23:59'
  } else {
    endDate.value = null
    endTimeValue.value = null
  }
}

// 监听开始时间变化
watch([startDate, startTimeValue], () => {
  updateStartTime()
})

// 监听截止时间变化
watch([endDate, endTimeValue], () => {
  updateEndTime()
})

// 待办数据
const todo = ref<Todo | null>(null)

// 新子任务输入
const newSubtaskTitle = ref('')

// 是否编辑模式
const isEdit = computed(() => todoId.value !== null)

// 当前待办的子任务列表（编辑模式从服务器加载）
const subtasks = computed(() => todo.value?.subtasks || [])

// 新建模式下待创建的子任务列表
const pendingSubtasks = ref<Array<{ id: number; title: string; completed: boolean }>>([])
let pendingSubtaskIdCounter = 0

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
// 原始的开始和截止时间（用于判断是否需要清除）
const originalStartTime = ref<string | null>(null)
const originalEndTime = ref<string | null>(null)

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
        color: todo.value.color,
        notifyAt: todo.value.notifyAt,
        notifyBefore: todo.value.notifyBefore,
        startTime: todo.value.startTime,
        endTime: todo.value.endTime
      }
      
      // 保存原始的通知时间
      originalNotifyAt.value = todo.value.notifyAt
      
      // 保存原始的开始和截止时间
      originalStartTime.value = todo.value.startTime
      originalEndTime.value = todo.value.endTime
      
      // 解析日期和时间
      parseNotifyAt(todo.value.notifyAt)
      parseStartTime(todo.value.startTime)
      parseEndTime(todo.value.endTime)
      
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
      // 判断是否需要清除时间字段
      const shouldClearNotifyAt = originalNotifyAt.value !== null && !form.value.notifyAt
      const shouldClearStartTime = originalStartTime.value !== null && !form.value.startTime
      const shouldClearEndTime = originalEndTime.value !== null && !form.value.endTime
      
      const data: UpdateTodoRequest = {
        title: form.value.title,
        description: form.value.description || null,
        color: form.value.color,
        notifyAt: form.value.notifyAt || undefined,
        notifyBefore: form.value.notifyBefore,
        clearNotifyAt: shouldClearNotifyAt,
        startTime: form.value.startTime || undefined,
        endTime: form.value.endTime || undefined,
        clearStartTime: shouldClearStartTime,
        clearEndTime: shouldClearEndTime
      }
      await invoke('update_todo', { id: todoId.value, data })
    } else {
      const data: CreateTodoRequest = {
        title: form.value.title,
        description: form.value.description || undefined,
        color: form.value.color,
        notifyAt: form.value.notifyAt || undefined,
        notifyBefore: form.value.notifyBefore,
        startTime: form.value.startTime || undefined,
        endTime: form.value.endTime || undefined
      }
      const newTodo = await invoke<Todo>('create_todo', { data })
      
      // 如果有待创建的子任务，批量创建
      if (pendingSubtasks.value.length > 0) {
        for (const subtask of pendingSubtasks.value) {
          const subtaskData: CreateSubTaskRequest = {
            parentId: newTodo.id,
            title: subtask.title
          }
          await invoke('create_subtask', { data: subtaskData })
        }
      }
    }

    handleClose()
  } catch (e) {
    console.error('Failed to save:', e)
  }
}

// 添加子任务
async function addSubtask() {
  if (!newSubtaskTitle.value.trim()) return
  
  if (isEdit.value && todoId.value) {
    // 编辑模式：调用 API 创建子任务
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
  } else {
    // 新建模式：添加到本地列表
    pendingSubtasks.value.push({
      id: --pendingSubtaskIdCounter, // 使用负数作为临时 ID
      title: newSubtaskTitle.value.trim(),
      completed: false
    })
    newSubtaskTitle.value = ''
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
  // 获取子任务标题用于确认
  let subtaskTitle = ''
  if (isEdit.value) {
    const subtask = subtasks.value.find(s => s.id === subtaskId)
    subtaskTitle = subtask?.title || ''
  } else {
    const subtask = pendingSubtasks.value.find(s => s.id === subtaskId)
    subtaskTitle = subtask?.title || ''
  }
  
  // 二次确认
  try {
    await ElMessageBox.confirm(
      `确定删除子任务"${subtaskTitle}"吗？`,
      '删除确认',
      {
        confirmButtonText: '删除',
        cancelButtonText: '取消',
        type: 'warning'
      }
    )
  } catch {
    // 用户取消
    return
  }
  
  if (isEdit.value) {
    // 编辑模式：调用 API 删除子任务
    try {
      await invoke('delete_subtask', { id: subtaskId })
      await loadTodo()
    } catch (e) {
      console.error('Failed to delete subtask:', e)
    }
  } else {
    // 新建模式：从本地列表删除
    const index = pendingSubtasks.value.findIndex(s => s.id === subtaskId)
    if (index !== -1) {
      pendingSubtasks.value.splice(index, 1)
    }
  }
}

// 切换本地子任务完成状态（新建模式）
function togglePendingSubtask(subtaskId: number) {
  const subtask = pendingSubtasks.value.find(s => s.id === subtaskId)
  if (subtask) {
    subtask.completed = !subtask.completed
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

        <!-- 颜色 -->
        <el-form-item label="颜色">
          <div class="color-picker-row">
            <button
              v-for="color in PRESET_COLORS"
              :key="color.value"
              class="color-preset-btn"
              :class="{ active: form.color === color.value }"
              :style="{ backgroundColor: color.value }"
              :title="color.name"
              type="button"
              @click="form.color = color.value"
            ></button>
            <el-color-picker
              v-model="form.color"
              :predefine="PRESET_COLORS.map(c => c.value)"
              size="small"
            />
          </div>
        </el-form-item>

        <!-- 时间范围 -->
        <el-form-item label="时间范围">
          <div class="time-range-picker">
            <div class="time-range-row">
              <span class="time-label">开始</span>
              <el-date-picker
                v-model="startDate"
                type="date"
                placeholder="开始日期"
                format="YYYY-MM-DD"
                value-format="YYYY-MM-DD"
                :teleported="true"
                :popper-options="{
                  placement: 'top-start',
                  modifiers: [{ name: 'flip', enabled: false }]
                }"
                class="date-picker-sm"
              />
              <el-time-picker
                v-model="startTimeValue"
                placeholder="时间"
                format="HH:mm"
                value-format="HH:mm"
                :teleported="true"
                :popper-options="{
                  placement: 'top-start',
                  modifiers: [{ name: 'flip', enabled: false }]
                }"
                class="time-picker-sm"
                :disabled="!startDate"
              />
            </div>
            <div class="time-range-row">
              <span class="time-label">截止</span>
              <el-date-picker
                v-model="endDate"
                type="date"
                placeholder="截止日期"
                format="YYYY-MM-DD"
                value-format="YYYY-MM-DD"
                :teleported="true"
                :popper-options="{
                  placement: 'top-start',
                  modifiers: [{ name: 'flip', enabled: false }]
                }"
                class="date-picker-sm"
              />
              <el-time-picker
                v-model="endTimeValue"
                placeholder="时间"
                format="HH:mm"
                value-format="HH:mm"
                :teleported="true"
                :popper-options="{
                  placement: 'top-start',
                  modifiers: [{ name: 'flip', enabled: false }]
                }"
                class="time-picker-sm"
                :disabled="!endDate"
              />
            </div>
          </div>
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

        <!-- 子任务 -->
        <el-form-item label="子任务">
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

            <!-- 编辑模式：显示已保存的子任务 -->
            <div v-if="isEdit" class="subtask-list-editor">
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

            <!-- 新建模式：显示待创建的子任务 -->
            <div v-else class="subtask-list-editor">
              <div 
                v-for="subtask in pendingSubtasks" 
                :key="subtask.id" 
                class="subtask-item-editor"
              >
                <el-checkbox 
                  :model-value="subtask.completed"
                  @change="togglePendingSubtask(subtask.id)"
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

.color-picker-row {
  display: flex;
  align-items: center;
  gap: 8px;
}

.color-preset-btn {
  width: 24px;
  height: 24px;
  border-radius: 4px;
  border: 2px solid transparent;
  cursor: pointer;
  transition: all 0.15s;
  padding: 0;

  &:hover {
    transform: scale(1.1);
  }

  &.active {
    border-color: var(--primary);
    box-shadow: 0 0 0 2px rgba(59, 130, 246, 0.3);
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

.time-range-picker {
  width: 100%;
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.time-range-row {
  display: flex;
  align-items: center;
  gap: 8px;

  .time-label {
    width: 32px;
    font-size: 12px;
    color: var(--text-secondary);
    flex-shrink: 0;
  }

  .date-picker-sm {
    flex: 1;
  }

  .time-picker-sm {
    width: 90px;
    flex-shrink: 0;
  }
}
</style>
