<script setup lang="ts">
import { ref, onMounted, computed, watch, nextTick } from 'vue'
import { useRoute } from 'vue-router'
import { invoke } from '@tauri-apps/api/core'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { WebviewWindow } from '@tauri-apps/api/webviewWindow'
import { currentMonitor, primaryMonitor } from '@tauri-apps/api/window'
import { ElMessage, ElMessageBox } from 'element-plus'
import type { Todo, CreateTodoRequest, UpdateTodoRequest, CreateSubTaskRequest, QuadrantType } from '@/types'
import { DEFAULT_COLOR, PRESET_COLORS, QUADRANT_INFO, DEFAULT_QUADRANT } from '@/types'
import { useAgentStore } from '@/stores/agentStore'
import { useSchedulerStore } from '@/stores/schedulerStore'
import { AGENT_TYPE_INFO } from '@/types/agent'
import { SCHEDULE_STATUS_MAP } from '@/types/scheduler'
import type { ScheduleStrategy } from '@/types/scheduler'
import CronEditor from '@/components/CronEditor.vue'

const route = useRoute()
const todoId = computed(() => route.query.id ? parseInt(route.query.id as string) : null)
const appWindow = getCurrentWindow()

// 表单数据
const form = ref({
  title: '',
  description: '',
  color: DEFAULT_COLOR,
  quadrant: DEFAULT_QUADRANT as QuadrantType,
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

// 当前显示的子任务列表（根据编辑模式决定，未完成的置顶）
const currentSubtaskList = computed(() => {
  const list = isEdit.value ? subtasks.value : pendingSubtasks.value
  // 未完成的排在前面，已完成的排在后面
  return [...list].sort((a, b) => {
    if (a.completed === b.completed) return 0
    return a.completed ? 1 : -1
  })
})

// 已完成的子任务数量
const completedSubtaskCount = computed(() => {
  return currentSubtaskList.value.filter(s => s.completed).length
})

// 子任务完成进度百分比
const subtaskProgressPercent = computed(() => {
  if (currentSubtaskList.value.length === 0) return 0
  return Math.round((completedSubtaskCount.value / currentSubtaskList.value.length) * 100)
})

// 新建模式下待创建的子任务列表
const pendingSubtasks = ref<Array<{ id: number; title: string; content: string | null; completed: boolean }>>([])
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
const isUpdatingCompleteState = ref(false)

// 原始的通知时间（用于判断是否需要清除）
const originalNotifyAt = ref<string | null>(null)
// 原始的开始和截止时间（用于判断是否需要清除）
const originalStartTime = ref<string | null>(null)
const originalEndTime = ref<string | null>(null)

// 根据象限ID获取对应颜色
function getQuadrantColor(quadrantId: QuadrantType): string {
  const quadrant = QUADRANT_INFO.find(q => q.id === quadrantId)
  return quadrant ? quadrant.color : DEFAULT_COLOR
}

// 选择象限时自动同步颜色（仅新建模式）
function handleQuadrantSelect(quadrantId: QuadrantType) {
  form.value.quadrant = quadrantId
  if (!isEdit.value) {
    form.value.color = getQuadrantColor(quadrantId)
  }
}

// 初始化
onMounted(async () => {
  if (todoId.value) {
    await loadTodo()
  }
  agentStore.loadAgents()
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
        quadrant: todo.value.quadrant,
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

      // 恢复 Agent 配置
      agentForm.value = {
        agentId: todo.value.agentId ?? null,
        projectPath: todo.value.agentProjectPath ?? '',
      }

      // 恢复调度配置
      initScheduleForm()
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
      
      const shouldClearAgent = (todo.value?.agentId !== null) && !agentForm.value.agentId

      const data: UpdateTodoRequest = {
        title: form.value.title,
        description: form.value.description || null,
        color: form.value.color,
        quadrant: form.value.quadrant,
        notifyAt: form.value.notifyAt || undefined,
        notifyBefore: form.value.notifyBefore,
        clearNotifyAt: shouldClearNotifyAt,
        startTime: form.value.startTime || undefined,
        endTime: form.value.endTime || undefined,
        clearStartTime: shouldClearStartTime,
        clearEndTime: shouldClearEndTime,
        agentId: agentForm.value.agentId || undefined,
        agentProjectPath: agentForm.value.projectPath || undefined,
        clearAgent: shouldClearAgent,
      }
      await invoke('update_todo', { id: todoId.value, data })
    } else {
      const data: CreateTodoRequest = {
        title: form.value.title,
        description: form.value.description || undefined,
        color: form.value.color,
        quadrant: form.value.quadrant,
        notifyAt: form.value.notifyAt || undefined,
        notifyBefore: form.value.notifyBefore,
        startTime: form.value.startTime || undefined,
        endTime: form.value.endTime || undefined,
        agentId: agentForm.value.agentId || undefined,
        agentProjectPath: agentForm.value.projectPath || undefined,
      }
      const newTodo = await invoke<Todo>('create_todo', { data })
      
      // 如果有待创建的子任务，批量创建
      if (pendingSubtasks.value.length > 0) {
        for (const subtask of pendingSubtasks.value) {
          const subtaskData: CreateSubTaskRequest = {
            parentId: newTodo.id,
            title: subtask.title,
            content: subtask.content || undefined
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

// 更新待办完成状态
async function updateTodoCompleted(completed: boolean) {
  if (!isEdit.value || !todoId.value || isUpdatingCompleteState.value) return
  if (todo.value?.completed === completed) return

  isUpdatingCompleteState.value = true
  try {
    const data: UpdateTodoRequest = { completed }
    await invoke('update_todo', { id: todoId.value, data })
    handleClose()
  } catch (e) {
    const action = completed ? 'complete' : 'reopen'
    console.error(`Failed to ${action} todo:`, e)
  } finally {
    isUpdatingCompleteState.value = false
  }
}

// 标记当前待办为已完成
async function handleCompleteTodo() {
  await updateTodoCompleted(true)
}

// 重新打开已完成待办
async function handleReopenTodo() {
  await updateTodoCompleted(false)
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
      id: --pendingSubtaskIdCounter,
      title: newSubtaskTitle.value.trim(),
      content: null,
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

// 子任务编辑窗口是否已打开
const isSubtaskEditorOpen = ref(false)

// 子任务内联编辑
const inlineEditingSubtaskId = ref<number | null>(null)
const inlineEditingTitle = ref('')

function startInlineEdit(subtask: { id: number; title: string }) {
  inlineEditingSubtaskId.value = subtask.id
  inlineEditingTitle.value = subtask.title
  nextTick(() => {
    const input = document.querySelector('.subtask-inline-input') as HTMLInputElement
    if (input) {
      input.focus()
      input.select()
    }
  })
}

async function saveInlineEdit(subtaskId: number) {
  const newTitle = inlineEditingTitle.value.trim()
  if (!newTitle) {
    cancelInlineEdit()
    return
  }

  try {
    await invoke('update_subtask', {
      id: subtaskId,
      data: { title: newTitle },
    })
    await loadTodo()
  } catch (e) {
    console.error('Failed to update subtask title:', e)
  }
  inlineEditingSubtaskId.value = null
}

function cancelInlineEdit() {
  inlineEditingSubtaskId.value = null
  inlineEditingTitle.value = ''
}

function handleInlineEditKeydown(e: KeyboardEvent, subtaskId: number) {
  if (e.key === 'Enter') {
    e.preventDefault()
    saveInlineEdit(subtaskId)
  } else if (e.key === 'Escape') {
    cancelInlineEdit()
  }
}

async function openSubtaskWindow(subtaskId: number, mode: 'edit' | 'view') {
  if (isSubtaskEditorOpen.value) return

  const agentId = agentForm.value.agentId ?? todo.value?.agentId ?? ''
  const agentPath = encodeURIComponent(agentForm.value.projectPath || todo.value?.agentProjectPath || '')
  const modeParam = mode === 'view' ? '&mode=view' : ''
  const url = `#/subtask-editor?id=${subtaskId}&agentId=${agentId}&agentProjectPath=${agentPath}${modeParam}`
  const label = `subtask-${mode}-${Date.now()}`
  const isEditMode = mode === 'edit'

  try {
    isSubtaskEditorOpen.value = true

    const windowWidth = 800
    const windowHeight = 750
    let x: number, y: number

    const monitor = await currentMonitor() || await primaryMonitor()
    if (monitor) {
      const s = monitor.scaleFactor
      const mx = monitor.position.x / s
      const my = monitor.position.y / s
      const mw = monitor.size.width / s
      const mh = monitor.size.height / s
      x = Math.round(mx + (mw - windowWidth) / 2)
      y = Math.round(my + (mh - windowHeight) / 2)
    } else {
      const s = await appWindow.scaleFactor()
      const pos = await appWindow.outerPosition()
      const size = await appWindow.outerSize()
      x = Math.round(pos.x / s + (size.width / s - windowWidth) / 2)
      y = Math.round(pos.y / s + (size.height / s - windowHeight) / 2)
    }

    const webview = new WebviewWindow(label, {
      url,
      title: isEditMode ? '编辑子任务' : '查看子任务',
      width: windowWidth,
      height: windowHeight,
      x,
      y,
      resizable: true,
      decorations: false,
      transparent: false,
      parent: appWindow,
    })

    webview.once('tauri://destroyed', async () => {
      isSubtaskEditorOpen.value = false
      if (isEditMode) await loadTodo()
    })

    webview.once('tauri://error', () => {
      isSubtaskEditorOpen.value = false
    })
  } catch (e) {
    isSubtaskEditorOpen.value = false
    console.error(`Failed to open subtask ${mode}:`, e)
  }
}

// ========== Agent 配置 ==========
const agentStore = useAgentStore()
const agentConfigVisible = ref(false)

const agentForm = ref({
  agentId: null as number | null,
  projectPath: '',
})

function openAgentConfig() {
  if (agentStore.enabledAgents.length === 0) {
    ElMessage.warning('请先在设置中配置并启用 Agent')
    return
  }

  agentForm.value = {
    agentId: todo.value?.agentId ?? agentStore.enabledAgents[0]?.id ?? null,
    projectPath: todo.value?.agentProjectPath ?? '',
  }
  agentConfigVisible.value = true
}

function saveAgentConfig() {
  if (!agentForm.value.agentId) {
    ElMessage.warning('请选择 Agent')
    return
  }

  agentConfigVisible.value = false
  ElMessage.success('Agent 配置已暂存，保存待办时生效')
}

function clearAgentConfig() {
  agentForm.value = { agentId: null, projectPath: '' }
  agentConfigVisible.value = false
  ElMessage.info('已清除 Agent 配置')
}

const currentAgentLabel = computed(() => {
  const id = agentForm.value.agentId ?? todo.value?.agentId
  if (!id) return ''
  const agent = agentStore.agents.find(a => a.id === id)
  if (!agent) return ''
  const typeInfo = AGENT_TYPE_INFO[agent.agentType]
  return typeInfo?.label || agent.agentType
})

const hasAgentConfig = computed(() => {
  return !!(agentForm.value.agentId || todo.value?.agentId)
})

// ========== 调度配置 ==========
const schedulerStore = useSchedulerStore()

const scheduleForm = ref({
  strategy: 'manual' as ScheduleStrategy,
  cronExpression: '',
  enabled: false,
})

function initScheduleForm() {
  if (todo.value) {
    scheduleForm.value = {
      strategy: (todo.value.scheduleStrategy as ScheduleStrategy) || 'manual',
      cronExpression: todo.value.cronExpression || '',
      enabled: !!todo.value.scheduleEnabled,
    }
  }
}

async function saveScheduleConfig() {
  if (!todoId.value) return
  try {
    await schedulerStore.updateTodoScheduleConfig(
      todoId.value,
      scheduleForm.value.strategy,
      scheduleForm.value.strategy === 'cron' ? scheduleForm.value.cronExpression : undefined,
      scheduleForm.value.enabled,
    )
    ElMessage.success('调度配置已保存')
  } catch (e) {
    ElMessage.error('保存调度配置失败: ' + String(e))
  }
}

function getScheduleStatusInfo(status: string) {
  return SCHEDULE_STATUS_MAP[status as keyof typeof SCHEDULE_STATUS_MAP] || SCHEDULE_STATUS_MAP.none
}

// 关闭窗口
function handleClose() {
  appWindow.close()
}
</script>

<template>
  <div class="editor-window">
    <!-- 主内容区域 -->
    <div class="main-area">
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

          <!-- 四象限 -->
          <el-form-item label="四象限">
            <div class="quadrant-picker">
              <button
                v-for="quadrant in QUADRANT_INFO"
                :key="quadrant.id"
                class="quadrant-btn"
                :class="{ active: form.quadrant === quadrant.id }"
                :style="{ 
                  '--quadrant-color': quadrant.color,
                  '--quadrant-bg': quadrant.bgColor 
                }"
                type="button"
                @click="handleQuadrantSelect(quadrant.id)"
              >
                <span class="quadrant-indicator" :style="{ backgroundColor: quadrant.color }"></span>
                <span class="quadrant-name">{{ quadrant.name }}</span>
              </button>
            </div>
          </el-form-item>

          <!-- 时间范围 -->
          <el-form-item label="时间范围">
            <div class="time-range-picker">
              <div class="time-range-row">
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

        </el-form>
      </div>

      <div class="window-footer">
        <div class="footer-left">
          <el-button
            :type="hasAgentConfig ? 'primary' : 'info'"
            plain
            @click="openAgentConfig"
          >
            <el-icon><MagicStick /></el-icon>
            {{ hasAgentConfig ? currentAgentLabel : 'Agent' }}
          </el-button>
        </div>
        <div class="footer-right">
          <el-button
            v-if="isEdit && todo && !todo.completed"
            type="success"
            plain
            :loading="isUpdatingCompleteState"
            @click="handleCompleteTodo"
          >
            <el-icon><CircleCheck /></el-icon>
            完成任务
          </el-button>
          <el-button
            v-if="isEdit && todo && todo.completed"
            type="warning"
            plain
            :loading="isUpdatingCompleteState"
            @click="handleReopenTodo"
          >
            <el-icon><RefreshLeft /></el-icon>
            重新打开
          </el-button>
          <el-button @click="handleClose">
            <el-icon><Close /></el-icon>
            取消
          </el-button>
          <el-button type="primary" @click="handleSave">
            <el-icon>
              <Check v-if="isEdit" />
              <Plus v-else />
            </el-icon>
            {{ isEdit ? '保存' : '创建' }}
          </el-button>
        </div>
      </div>
    </div>

    <!-- 子任务面板（始终显示） -->
    <div class="subtask-panel">
      <div class="panel-header">
        <h3>子任务</h3>
      </div>

        <div class="panel-content">
          <!-- 进度条 -->
          <div v-if="currentSubtaskList.length > 0" class="subtask-progress">
            <div class="progress-info">
              <span class="progress-text">{{ completedSubtaskCount }} / {{ currentSubtaskList.length }}</span>
              <span class="progress-label">已完成</span>
            </div>
            <div class="progress-bar">
              <div 
                class="progress-fill" 
                :style="{ width: subtaskProgressPercent + '%' }"
              ></div>
            </div>
          </div>

          <!-- 添加子任务 -->
          <div class="add-subtask">
            <div class="add-subtask-input">
              <el-icon class="input-icon"><Plus /></el-icon>
              <input
                v-model="newSubtaskTitle"
                type="text"
                placeholder="添加子任务..."
                @keyup.enter="addSubtask"
              />
              <transition name="fade">
                <button 
                  v-if="newSubtaskTitle.trim()"
                  class="add-btn"
                  @click="addSubtask"
                >
                  <el-icon><Plus /></el-icon>
                  <span>添加</span>
                </button>
              </transition>
            </div>
          </div>

          <!-- 子任务列表 -->
          <div v-if="currentSubtaskList.length > 0" class="subtask-list-editor">
            <transition-group name="subtask-list" tag="div">
              <div 
                v-for="subtask in currentSubtaskList" 
                :key="subtask.id" 
                class="subtask-item-editor"
                :class="{ completed: subtask.completed }"
              >
                <div 
                  class="custom-checkbox"
                  :class="{ checked: subtask.completed }"
                  @click="isEdit ? toggleSubtask(subtask.id) : togglePendingSubtask(subtask.id)"
                >
                  <el-icon v-if="subtask.completed" class="check-icon"><Check /></el-icon>
                </div>
                <input
                  v-if="inlineEditingSubtaskId === subtask.id"
                  v-model="inlineEditingTitle"
                  class="subtask-inline-input"
                  @blur="saveInlineEdit(subtask.id)"
                  @keydown="handleInlineEditKeydown($event, subtask.id)"
                />
                <span 
                  v-else
                  class="subtask-title"
                  @dblclick="isEdit && startInlineEdit(subtask)"
                >
                  {{ subtask.title }}
                </span>
                <el-tag
                  v-if="'scheduleStatus' in subtask && subtask.scheduleStatus && subtask.scheduleStatus !== 'none'"
                  :type="getScheduleStatusInfo(subtask.scheduleStatus as string).type as any"
                  size="small"
                  effect="light"
                  class="schedule-tag"
                >
                  {{ getScheduleStatusInfo(subtask.scheduleStatus as string).label }}
                </el-tag>
                <el-icon
                  v-if="subtask.content"
                  class="content-indicator"
                  :size="12"
                  title="包含详细内容"
                >
                  <Document />
                </el-icon>
                <div v-if="inlineEditingSubtaskId !== subtask.id" class="subtask-actions">
                  <button
                    class="action-btn view-btn"
                    @click="openSubtaskWindow(subtask.id, 'view')"
                    title="查看子任务"
                  >
                    <el-icon><View /></el-icon>
                  </button>
                  <button 
                    v-if="isEdit"
                    class="action-btn edit-btn"
                    @click="openSubtaskWindow(subtask.id, 'edit')"
                    title="编辑子任务"
                  >
                    <el-icon><Edit /></el-icon>
                  </button>
                  <button 
                    class="action-btn delete-btn"
                    @click="deleteSubtask(subtask.id)"
                    title="删除子任务"
                  >
                    <el-icon><Delete /></el-icon>
                  </button>
                </div>
              </div>
            </transition-group>
          </div>

          <!-- 空状态 -->
          <div v-else class="subtask-empty">
            <el-icon class="empty-icon"><List /></el-icon>
            <span>暂无子任务</span>
          </div>
        </div>
    </div>

    <!-- 模态遮罩：子任务编辑窗口打开时阻止操作 -->
    <div v-if="isSubtaskEditorOpen" class="modal-overlay"></div>

    <!-- Agent 配置对话框 -->
    <el-dialog
      v-model="agentConfigVisible"
      title="Agent 配置"
      width="460px"
      append-to-body
      class="agent-config-dialog"
      top="10vh"
    >
      <div style="max-height: calc(80vh - 160px); overflow-y: auto; padding-right: 4px;">
      <el-form label-position="top" size="default">
        <el-form-item label="选择 Agent" required>
          <el-select
            v-model="agentForm.agentId"
            style="width: 100%"
            placeholder="选择要绑定的 Agent"
            clearable
          >
            <el-option
              v-for="agent in agentStore.enabledAgents"
              :key="agent.id"
              :label="AGENT_TYPE_INFO[agent.agentType]?.label || agent.agentType"
              :value="agent.id"
            />
          </el-select>
        </el-form-item>

        <el-form-item label="项目路径">
          <el-input
            v-model="agentForm.projectPath"
            placeholder="Agent 工作的项目目录，如 D:\Git\my-project"
            clearable
          />
          <div class="form-tip">子任务执行 Agent 时使用此目录</div>
        </el-form-item>

        <el-divider v-if="isEdit" />

        <template v-if="isEdit">
          <el-form-item label="调度策略">
            <el-select
              v-model="scheduleForm.strategy"
              style="width: 100%"
              placeholder="选择调度策略"
            >
              <el-option label="手动执行" value="manual" />
              <el-option label="自动调度" value="auto" />
              <el-option label="定时执行 (Cron)" value="cron" />
              <el-option label="Git Push 触发" value="git_push" />
              <el-option label="文件变更触发" value="file_watch" />
            </el-select>
          </el-form-item>

          <el-form-item v-if="scheduleForm.strategy === 'cron'" label="Cron 表达式">
            <CronEditor v-model="scheduleForm.cronExpression" />
          </el-form-item>

          <el-form-item label="启用调度">
            <el-switch v-model="scheduleForm.enabled" />
            <span class="form-tip" style="margin-left: 8px">
              {{ scheduleForm.enabled ? '调度已启用' : '调度已暂停' }}
            </span>
          </el-form-item>
        </template>
      </el-form>
      </div>

      <template #footer>
        <el-button type="danger" plain @click="clearAgentConfig">
          清除配置
        </el-button>
        <el-button @click="agentConfigVisible = false">取消</el-button>
        <el-button type="primary" @click="() => { saveAgentConfig(); if (isEdit) saveScheduleConfig() }">
          确定
        </el-button>
      </template>
    </el-dialog>
  </div>
</template>

<style scoped>
.editor-window {
  display: flex;
  height: 100vh;
  background: #FFFFFF;
}

.main-area {
  flex: 1;
  display: flex;
  flex-direction: column;
  min-width: 0;
}

.window-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 12px 16px;
  min-height: 44px;
  box-sizing: border-box;
  border-bottom: 1px solid var(--border);
  -webkit-app-region: drag;

  h2 {
    margin: 0;
    font-size: 16px;
    font-weight: 600;
    line-height: 1.2;
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
  justify-content: space-between;
  align-items: center;
  padding: 12px 16px;
  border-top: 1px solid var(--border);
}

.footer-left {
  display: flex;
  gap: 8px;
}

.footer-right {
  display: flex;
  gap: 8px;
}

/* 子任务面板 */
.subtask-panel {
  width: 380px;
  flex-shrink: 0;
  display: flex;
  flex-direction: column;
  background: #fafbfc;
  border-left: 1px solid #e2e8f0;
}

.panel-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 12px 16px;
  min-height: 57px;
  box-sizing: border-box;
  border-bottom: 1px solid var(--border);
  background: #ffffff;
  -webkit-app-region: drag;

  h3 {
    margin: 0;
    font-size: 16px;
    font-weight: 600;
    line-height: 1.2;
    color: #334155;
  }
}

.panel-content {
  flex: 1;
  padding: 16px;
  overflow-y: auto;
  display: flex;
  flex-direction: column;
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

/* 四象限选择器 */
.quadrant-picker {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 8px;
  width: 100%;
}

.quadrant-btn {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 10px 12px;
  background: var(--quadrant-bg);
  border: 2px solid transparent;
  border-radius: 8px;
  cursor: pointer;
  transition: all 0.2s ease;

  &:hover {
    border-color: var(--quadrant-color);
  }

  &.active {
    border-color: var(--quadrant-color);
    box-shadow: 0 0 0 2px color-mix(in srgb, var(--quadrant-color) 30%, transparent);
  }

  .quadrant-indicator {
    width: 10px;
    height: 10px;
    border-radius: 50%;
    flex-shrink: 0;
  }

  .quadrant-name {
    font-size: 12px;
    color: #334155;
    font-weight: 500;
  }
}

/* 进度条样式 */
.subtask-progress {
  display: flex;
  align-items: center;
  gap: 12px;
  margin-bottom: 16px;
  padding: 12px 14px;
  background: linear-gradient(135deg, #f0f9ff 0%, #e0f2fe 100%);
  border-radius: 10px;

  .progress-info {
    display: flex;
    flex-direction: column;
    min-width: 50px;

    .progress-text {
      font-size: 16px;
      font-weight: 600;
      color: #0369a1;
    }

    .progress-label {
      font-size: 11px;
      color: #64748b;
    }
  }

  .progress-bar {
    flex: 1;
    height: 6px;
    background: #e2e8f0;
    border-radius: 3px;
    overflow: hidden;

    .progress-fill {
      height: 100%;
      background: linear-gradient(90deg, #3b82f6 0%, #06b6d4 100%);
      border-radius: 3px;
      transition: width 0.3s ease;
    }
  }
}

/* 添加子任务输入框 */
.add-subtask {
  margin-bottom: 12px;

  .add-subtask-input {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 12px;
    background: #f8fafc;
    border: 1px dashed #cbd5e1;
    border-radius: 8px;
    transition: all 0.2s ease;

    &:hover {
      border-color: #94a3b8;
      background: #f1f5f9;
    }

    &:focus-within {
      border-color: #3b82f6;
      border-style: solid;
      background: #ffffff;
      box-shadow: 0 0 0 3px rgba(59, 130, 246, 0.1);
    }

    .input-icon {
      color: #94a3b8;
      font-size: 16px;
      flex-shrink: 0;
    }

    input {
      flex: 1;
      border: none;
      outline: none;
      background: transparent;
      font-size: 13px;
      color: #334155;

      &::placeholder {
        color: #94a3b8;
      }
    }

    .add-btn {
      display: inline-flex;
      align-items: center;
      gap: 6px;
      padding: 4px 12px;
      font-size: 12px;
      font-weight: 500;
      color: #ffffff;
      background: #3b82f6;
      border: none;
      border-radius: 6px;
      cursor: pointer;
      transition: all 0.15s ease;

      &:hover {
        background: #2563eb;
      }

      &:active {
        transform: scale(0.96);
      }
    }
  }
}

/* 子任务列表 */
.subtask-list-editor {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  padding-right: 4px;

  &::-webkit-scrollbar {
    width: 4px;
  }

  &::-webkit-scrollbar-track {
    background: transparent;
  }

  &::-webkit-scrollbar-thumb {
    background: #cbd5e1;
    border-radius: 2px;
  }
}

/* 子任务列表项 */
.subtask-item-editor {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 10px 12px;
  margin-bottom: 6px;
  background: #ffffff;
  border: 1px solid #e2e8f0;
  border-radius: 8px;
  transition: all 0.2s ease;
  position: relative;

  &:last-child {
    margin-bottom: 0;
  }

  &:hover {
    border-color: #cbd5e1;
    box-shadow: 0 2px 4px rgba(0, 0, 0, 0.04);

    .subtask-actions {
      display: flex;
    }
  }

  &.completed {
    background: #f8fafc;
    border-color: #e2e8f0;

    .subtask-title {
      text-decoration: line-through;
      color: #94a3b8;
    }
  }

  /* 自定义复选框 */
  .custom-checkbox {
    width: 20px;
    height: 20px;
    border: 2px solid #cbd5e1;
    border-radius: 50%;
    cursor: pointer;
    display: flex;
    align-items: center;
    justify-content: center;
    transition: all 0.2s ease;
    flex-shrink: 0;

    &:hover {
      border-color: #3b82f6;
    }

    &.checked {
      background: linear-gradient(135deg, #3b82f6 0%, #06b6d4 100%);
      border-color: transparent;

      .check-icon {
        color: #ffffff;
        font-size: 12px;
      }
    }
  }

  .subtask-title {
    flex: 1;
    font-size: 13px;
    color: #334155;
    line-height: 1.4;
    word-break: break-word;
    cursor: default;
    padding: 2px 4px;
    border-radius: 4px;
    transition: background 0.15s ease;

    &:hover {
      background: #f1f5f9;
    }
  }

  .subtask-inline-input {
    flex: 1;
    font-size: 13px;
    color: #334155;
    line-height: 1.4;
    padding: 2px 4px;
    border: 1px solid #3b82f6;
    border-radius: 4px;
    outline: none;
    background: #ffffff;
    font-family: inherit;
  }

  .schedule-tag {
    flex-shrink: 0;
    font-size: 10px;
    padding: 0 4px;
    height: 18px;
    line-height: 18px;
  }

  .content-indicator {
    color: #3b82f6;
    flex-shrink: 0;
    opacity: 0.7;
  }

  .subtask-actions {
    display: none;
    align-items: center;
    gap: 2px;
    position: absolute;
    right: 6px;
    top: 50%;
    transform: translateY(-50%);
    background: rgba(255, 255, 255, 0.95);
    padding: 2px 4px;
    border-radius: 4px;
    box-shadow: 0 1px 4px rgba(0, 0, 0, 0.1);
    z-index: 5;

    .action-btn {
      display: flex;
      align-items: center;
      justify-content: center;
      width: 24px;
      height: 24px;
      padding: 0;
      background: transparent;
      border: none;
      border-radius: 4px;
      cursor: pointer;
      color: #94a3b8;
      transition: all 0.15s ease;

      &.view-btn:hover {
        background: #e0e7ff;
        color: #6366f1;
      }

      &.edit-btn:hover {
        background: #dbeafe;
        color: #3b82f6;
      }

      &.delete-btn:hover {
        background: #fee2e2;
        color: #ef4444;
      }
    }
  }
}

/* 空状态 */
.subtask-empty {
  flex: 1;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  padding: 24px 16px;
  color: #94a3b8;
  text-align: center;

  .empty-icon {
    font-size: 32px;
    margin-bottom: 8px;
    opacity: 0.5;
  }

  span {
    font-size: 13px;
  }
}

/* 动画 */
.fade-enter-active,
.fade-leave-active {
  transition: opacity 0.15s ease;
}

.fade-enter-from,
.fade-leave-to {
  opacity: 0;
}

.subtask-list-enter-active,
.subtask-list-leave-active {
  transition: all 0.25s ease;
}

.subtask-list-enter-from {
  opacity: 0;
  transform: translateX(-10px);
}

.subtask-list-leave-to {
  opacity: 0;
  transform: translateX(10px);
}

.subtask-list-move {
  transition: transform 0.25s ease;
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

  .date-picker-sm {
    flex: 1;
  }

  .time-picker-sm {
    width: 90px;
    flex-shrink: 0;
  }
}

.modal-overlay {
  position: fixed;
  top: 0;
  left: 0;
  right: 0;
  bottom: 0;
  background: rgba(0, 0, 0, 0.15);
  z-index: 9999;
  cursor: not-allowed;
}

.form-tip {
  font-size: 12px;
  color: #94a3b8;
  margin-top: 4px;
  line-height: 1.4;
}
</style>

<style>
.agent-config-dialog .el-dialog__body {
  padding-top: 12px;
}
</style>
