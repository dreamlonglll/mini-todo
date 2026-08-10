<script setup lang="ts">
import { ref, onMounted, onBeforeUnmount, computed, watch, nextTick } from 'vue'
import { useRoute } from 'vue-router'
import { invoke } from '@tauri-apps/api/core'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { WebviewWindow } from '@tauri-apps/api/webviewWindow'
import { currentMonitor, primaryMonitor } from '@tauri-apps/api/window'
import { ElMessage, ElMessageBox } from 'element-plus'
import { open as openDialog } from '@tauri-apps/plugin-dialog'
import type { Todo, CreateTodoRequest, UpdateTodoRequest, CreateSubTaskRequest, QuadrantType } from '@/types'
import { DEFAULT_COLOR, PRESET_COLORS, QUADRANT_INFO, DEFAULT_QUADRANT } from '@/types'
import { resolveQuadrantColor } from '@/utils/quadrant'
import draggable from 'vuedraggable'
import MarkdownEditor from '@/components/MarkdownEditor.vue'

const route = useRoute()
const todoId = computed(() => route.query.id ? parseInt(route.query.id as string) : null)
const appWindow = getCurrentWindow()

// 只读模式（仅编辑已有待办时有效，点击 [编辑] 原地切换到编辑排版）
const isViewMode = ref(route.query.mode === 'view' && !!route.query.id)

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

// 当前显示的子任务列表（根据编辑模式决定）
//
// 纯手工顺序：不按完成状态重排，勾选完成后条目留在原位，拖拽结果即最终顺序。
// 返回的是原数组引用而非拷贝，vuedraggable 的 :list 才能原地重排。
const currentSubtaskList = computed(() =>
  isEdit.value ? subtasks.value : pendingSubtasks.value
)

// 拖拽结束后落库新顺序（新建模式下顺序就是数组顺序，保存时按序创建）
async function onSubtaskDragEnd() {
  if (!isEdit.value) return
  const ids = currentSubtaskList.value.map(s => s.id)
  if (ids.length === 0) return
  try {
    await invoke('reorder_subtasks', { ids })
  } catch (e) {
    console.error('Failed to reorder subtasks:', e)
    // 落库失败时本地顺序已经变了，重新拉一次让两边一致
    await loadTodo()
  }
}

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

// 重复提醒
const repeatEnabled = ref(false)
const repeatType = ref<'daily' | 'weekly' | 'monthly'>('daily')
const repeatInterval = ref(1)
const repeatWeekdays = ref<number[]>([])
const repeatMonthDay = ref(1)

const weekdayOptions = [
  { label: '周一', value: 1 },
  { label: '周二', value: 2 },
  { label: '周三', value: 3 },
  { label: '周四', value: 4 },
  { label: '周五', value: 5 },
  { label: '周六', value: 6 },
  { label: '周日', value: 7 },
]

// 原始的通知时间（用于判断是否需要清除）
const originalNotifyAt = ref<string | null>(null)
// 原始的开始和截止时间（用于判断是否需要清除）
const originalStartTime = ref<string | null>(null)
const originalEndTime = ref<string | null>(null)

// 只读模式：当前象限（优先级）信息
const quadrantInfo = computed(() => QUADRANT_INFO.find(q => q.id === form.value.quadrant))

// 只读模式：格式化 "YYYY-MM-DDTHH:MM:SS" 为 "YYYY-MM-DD HH:MM"
function formatDateTime(value: string | null): string {
  if (!value) return ''
  return value.replace('T', ' ').substring(0, 16)
}

// 只读模式：通知状态描述
const notifyStatusText = computed(() => {
  if (repeatEnabled.value) {
    const unit = { daily: '天', weekly: '周', monthly: '月' }[repeatType.value] || '天'
    let text = `每 ${repeatInterval.value} ${unit}重复`
    if (repeatType.value === 'weekly' && repeatWeekdays.value.length > 0) {
      const names = weekdayOptions
        .filter(o => repeatWeekdays.value.includes(o.value))
        .map(o => o.label)
        .join('、')
      text += `（${names}）`
    } else if (repeatType.value === 'monthly') {
      text += `（每月 ${repeatMonthDay.value} 号）`
    }
    if (form.value.notifyAt) {
      text += `，下次 ${formatDateTime(form.value.notifyAt)}`
    }
    return text
  }
  if (form.value.notifyAt) {
    let text = `提醒 ${formatDateTime(form.value.notifyAt)}`
    if (form.value.notifyBefore > 0) {
      text += `（提前 ${form.value.notifyBefore} 分钟）`
    }
    return text
  }
  return ''
})

// 选择象限时自动同步颜色：仅当颜色还是旧象限的默认色（用户没手动改过）才跟随
function handleQuadrantSelect(quadrantId: QuadrantType) {
  form.value.color = resolveQuadrantColor(form.value.color, form.value.quadrant, quadrantId)
  form.value.quadrant = quadrantId
}

// ===== 描述放大编辑弹窗（源码 / 预览分栏）=====
const descDialogVisible = ref(false)
const descDialogMaximized = ref(false)
const descDraft = ref('')
const descPreview = ref('')
// 进入弹窗最大化前，编辑窗口是否本就最大化（还原时避免误还原）
let wasWindowMaximized = false
let descPreviewTimer: ReturnType<typeof setTimeout> | null = null

function openDescDialog() {
  descDraft.value = form.value.description
  descPreview.value = descDraft.value
  descDialogVisible.value = true
}

// 源码变更 → 300ms 防抖同步到预览（readonly MarkdownEditor 内部 watch 会 replaceAll 更新渲染）
watch(descDraft, (value) => {
  if (descPreviewTimer) clearTimeout(descPreviewTimer)
  descPreviewTimer = setTimeout(() => {
    descPreviewTimer = null
    descPreview.value = value
  }, 300)
})

// 弹窗最大化 ↔ 还原（联动编辑窗口本体的最大化状态）
async function toggleDescDialogMaximize() {
  try {
    if (!descDialogMaximized.value) {
      wasWindowMaximized = await appWindow.isMaximized()
      if (!wasWindowMaximized) {
        await appWindow.maximize()
      }
      descDialogMaximized.value = true
    } else {
      await restoreDescDialogMaximize()
    }
  } catch (e) {
    console.error('Failed to toggle window maximize:', e)
  }
}

async function restoreDescDialogMaximize() {
  descDialogMaximized.value = false
  if (!wasWindowMaximized) {
    await appWindow.unmaximize()
  }
}

function confirmDescDialog() {
  // 写回后，内联 MarkdownEditor 经 modelValue watch 自动同步渲染
  form.value.description = descDraft.value
  descDialogVisible.value = false
}

// 弹窗关闭统一收口（确定 / 取消 / X 都会走到）：清防抖 + 还原窗口最大化状态
async function onDescDialogClosed() {
  if (descPreviewTimer) {
    clearTimeout(descPreviewTimer)
    descPreviewTimer = null
  }
  if (descDialogMaximized.value) {
    try {
      await restoreDescDialogMaximize()
    } catch (e) {
      console.error('Failed to restore window state:', e)
    }
  }
}

onBeforeUnmount(() => {
  if (descPreviewTimer) {
    clearTimeout(descPreviewTimer)
    descPreviewTimer = null
  }
})

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

      // 加载重复提醒设置
      repeatEnabled.value = !!todo.value.repeatEnabled
      if (todo.value.repeatType) {
        repeatType.value = todo.value.repeatType as 'daily' | 'weekly' | 'monthly'
      }
      repeatInterval.value = todo.value.repeatInterval || 1
      repeatWeekdays.value = todo.value.repeatWeekdays
        ? todo.value.repeatWeekdays.split(',').map(Number).filter(n => n >= 1 && n <= 7)
        : []
      repeatMonthDay.value = todo.value.repeatMonthDay || 1
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
      
      const wasRepeatEnabled = !!todo.value?.repeatEnabled
      const shouldClearRepeat = wasRepeatEnabled && !repeatEnabled.value

      const data: UpdateTodoRequest = {
        title: form.value.title,
        description: form.value.description || null,
        color: form.value.color,
        quadrant: form.value.quadrant,
        notifyAt: form.value.notifyAt || undefined,
        notifyBefore: repeatEnabled.value ? 0 : form.value.notifyBefore,
        clearNotifyAt: shouldClearNotifyAt,
        startTime: form.value.startTime || undefined,
        endTime: form.value.endTime || undefined,
        clearStartTime: shouldClearStartTime,
        clearEndTime: shouldClearEndTime,
        clearRepeat: shouldClearRepeat,
        repeatEnabled: repeatEnabled.value || undefined,
        repeatType: repeatEnabled.value ? repeatType.value : undefined,
        repeatInterval: repeatEnabled.value ? repeatInterval.value : undefined,
        repeatWeekdays: repeatEnabled.value && repeatType.value === 'weekly' && repeatWeekdays.value.length > 0
          ? repeatWeekdays.value.sort((a, b) => a - b).join(',') : undefined,
        repeatMonthDay: repeatEnabled.value && repeatType.value === 'monthly'
          ? repeatMonthDay.value : undefined,
      }
      await invoke('update_todo', { id: todoId.value, data })
      ElMessage.success('待办已保存')
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
      }
      const newTodo = await invoke<Todo>('create_todo', { data })
      
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
      ElMessage.success('待办已创建')
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

function handleImportCommand(command: string) {
  if (command === 'files') importSubtasks()
  else if (command === 'folder') importSubtasksFromFolder()
}

async function importSubtasks() {
  if (!isEdit.value || !todoId.value) {
    ElMessage.warning('请先保存待办后再导入子任务')
    return
  }

  try {
    const selected = await openDialog({
      title: '导入子任务（选择 .md/.txt 文件或文件夹）',
      multiple: true,
      directory: false,
      filters: [{ name: '文本文件', extensions: ['md', 'txt'] }],
    })

    if (!selected) return

    const paths = Array.isArray(selected) ? selected : [selected]
    if (paths.length === 0) return

    const created = await invoke<any[]>('import_subtasks_from_paths', {
      parentId: todoId.value,
      paths,
    })

    await loadTodo()
    ElMessage.success(`成功导入 ${created.length} 个子任务`)
  } catch (e) {
    ElMessage.error('导入失败: ' + String(e))
  }
}

async function importSubtasksFromFolder() {
  if (!isEdit.value || !todoId.value) {
    ElMessage.warning('请先保存待办后再导入子任务')
    return
  }

  try {
    const selected = await openDialog({
      title: '选择文件夹（递归导入 .md/.txt 文件）',
      directory: true,
    })

    if (!selected) return

    const paths = [selected as string]

    const created = await invoke<any[]>('import_subtasks_from_paths', {
      parentId: todoId.value,
      paths,
    })

    await loadTodo()
    ElMessage.success(`成功导入 ${created.length} 个子任务`)
  } catch (e) {
    ElMessage.error('导入失败: ' + String(e))
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
  let subtaskTitle: string
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

  const modeParam = mode === 'view' ? '&mode=view' : ''
  const url = `#/subtask-editor?id=${subtaskId}${modeParam}`
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

// 关闭窗口
function handleClose() {
  appWindow.close()
}

function onHeaderMouseDown(e: MouseEvent) {
  if (e.buttons !== 1) return
  const target = e.target as HTMLElement
  if (target.closest('[data-tauri-drag-region="false"]')) return
  if (target.closest('button, input, textarea, select, a, [role="button"]')) return
  e.preventDefault()
  appWindow.startDragging()
}
</script>

<template>
  <div class="editor-window">
    <!-- 主内容区域 -->
    <div class="main-area">
      <div class="window-header" data-tauri-drag-region="deep" @mousedown="onHeaderMouseDown">
        <h2>{{ isViewMode ? '待办详情' : (isEdit ? '编辑待办' : '新建待办') }}</h2>
        <div class="header-actions" data-tauri-drag-region="false">
          <el-button v-if="isViewMode" text type="primary" @click="isViewMode = false">
            <el-icon><Edit /></el-icon>
            <span>编辑</span>
          </el-button>
          <el-button text @click="handleClose">
            <el-icon><Close /></el-icon>
          </el-button>
        </div>
      </div>

      <!-- 只读模式：简化排版，突出内容 -->
      <div v-if="isViewMode" class="view-content">
        <h1 class="view-title" :class="{ completed: todo?.completed }">{{ form.title }}</h1>

        <div class="view-meta">
          <span
            v-if="quadrantInfo"
            class="meta-badge"
            :style="{ color: quadrantInfo.color, backgroundColor: quadrantInfo.bgColor }"
          >
            <span class="badge-dot" :style="{ backgroundColor: quadrantInfo.color }"></span>
            {{ quadrantInfo.name }}
          </span>
          <span v-if="notifyStatusText" class="meta-badge notify-badge">
            <el-icon :size="13"><Bell /></el-icon>
            {{ notifyStatusText }}
          </span>
        </div>

        <MarkdownEditor
          v-if="form.description"
          :model-value="form.description"
          readonly
          class="view-markdown"
        />
        <div v-else class="view-empty-desc">暂无描述</div>
      </div>

      <div v-else class="editor-content">
        <el-form label-position="top" :model="form">
          <!-- 标题 -->
          <el-form-item label="标题" required>
            <el-input 
              v-model="form.title" 
              placeholder="请输入待办标题"
              maxlength="100"
            />
          </el-form-item>

          <!-- 描述（Markdown） -->
          <el-form-item>
            <template #label>
              <div class="desc-label">
                <span>描述</span>
                <button
                  class="desc-expand-btn"
                  type="button"
                  title="放大编辑"
                  @click="openDescDialog"
                >
                  <el-icon :size="14"><FullScreen /></el-icon>
                </button>
              </div>
            </template>
            <MarkdownEditor v-model="form.description" class="description-editor" />
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
          <el-form-item :label="repeatEnabled ? '首次提醒' : '提醒时间'">
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

          <!-- 提前通知（重复模式下隐藏） -->
          <el-form-item v-if="form.notifyAt && !repeatEnabled" label="提前提醒">
            <el-select
              :model-value="isCustomNotifyBefore ? -1 : form.notifyBefore"
              style="width: 100%"
              @change="handleNotifyBeforeChange"
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

          <!-- 重复提醒 -->
          <el-form-item label="重复">
            <el-switch v-model="repeatEnabled" />
          </el-form-item>

          <template v-if="repeatEnabled">
            <el-form-item label="重复方式">
              <el-select v-model="repeatType" style="width: 100%">
                <el-option label="每天" value="daily" />
                <el-option label="每周" value="weekly" />
                <el-option label="每月" value="monthly" />
              </el-select>
            </el-form-item>

            <el-form-item :label="'每 ' + repeatInterval + ' ' + ({ daily: '天', weekly: '周', monthly: '月' }[repeatType] || '天')">
              <el-input-number
                v-model="repeatInterval"
                :min="1"
                :max="99"
                style="width: 100%"
              />
            </el-form-item>

            <!-- 周模式：选择星期几 -->
            <el-form-item v-if="repeatType === 'weekly'" label="星期">
              <el-checkbox-group v-model="repeatWeekdays" class="weekday-checkbox-group">
                <el-checkbox
                  v-for="opt in weekdayOptions"
                  :key="opt.value"
                  :label="opt.label"
                  :value="opt.value"
                />
              </el-checkbox-group>
            </el-form-item>

            <!-- 月模式：选择日期 -->
            <el-form-item v-if="repeatType === 'monthly'" label="每月几号">
              <el-input-number
                v-model="repeatMonthDay"
                :min="1"
                :max="31"
                style="width: 100%"
              />
            </el-form-item>
          </template>

        </el-form>
      </div>

      <div v-if="!isViewMode" class="window-footer">
        <div class="footer-right">
          <el-button
            v-if="isEdit && todo && !todo.completed"
            type="success"
            plain
            size="small"
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
            size="small"
            :loading="isUpdatingCompleteState"
            @click="handleReopenTodo"
          >
            <el-icon><RefreshLeft /></el-icon>
            重新打开
          </el-button>
          <el-button size="small" @click="handleClose">
            <el-icon><Close /></el-icon>
            取消
          </el-button>
          <el-button type="primary" size="small" @click="handleSave">
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
      <div class="panel-header" data-tauri-drag-region="deep" @mousedown="onHeaderMouseDown">
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

          <!-- 添加子任务（只读模式隐藏，它是写入口） -->
          <div v-if="!isViewMode" class="add-subtask">
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
              <el-dropdown v-if="isEdit" trigger="click" @command="handleImportCommand">
                <button class="import-btn" title="导入子任务">
                  <el-icon :size="14"><Upload /></el-icon>
                </button>
                <template #dropdown>
                  <el-dropdown-menu>
                    <el-dropdown-item command="files">选择文件 (.md/.txt)</el-dropdown-item>
                    <el-dropdown-item command="folder">选择文件夹（递归导入）</el-dropdown-item>
                  </el-dropdown-menu>
                </template>
              </el-dropdown>
            </div>
          </div>

          <!-- 子任务列表 -->
          <div v-if="currentSubtaskList.length > 0" class="subtask-list-editor">
            <draggable
              :list="currentSubtaskList"
              item-key="id"
              handle=".subtask-drag-handle"
              ghost-class="dragging"
              :animation="200"
              :force-fallback="true"
              :disabled="isViewMode"
              @end="onSubtaskDragEnd"
            >
              <template #item="{ element: subtask }">
              <div
                class="subtask-item-editor"
                :class="{ completed: subtask.completed, readonly: isViewMode }"
              >
                <el-icon v-if="!isViewMode" class="subtask-drag-handle" :size="14" title="拖拽排序">
                  <Rank />
                </el-icon>
                <div
                  class="custom-checkbox"
                  :class="{ checked: subtask.completed }"
                  @click="isViewMode ? null : (isEdit ? toggleSubtask(subtask.id) : togglePendingSubtask(subtask.id))"
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
                  @dblclick="isEdit && !isViewMode && startInlineEdit(subtask)"
                >
                  {{ subtask.title }}
                </span>
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
                    title="查看子任务"
                    @click="openSubtaskWindow(subtask.id, 'view')"
                  >
                    <el-icon><View /></el-icon>
                  </button>
                  <button
                    v-if="isEdit && !isViewMode"
                    class="action-btn edit-btn"
                    title="编辑子任务"
                    @click="openSubtaskWindow(subtask.id, 'edit')"
                  >
                    <el-icon><Edit /></el-icon>
                  </button>
                  <button
                    v-if="!isViewMode"
                    class="action-btn delete-btn"
                    title="删除子任务"
                    @click="deleteSubtask(subtask.id)"
                  >
                    <el-icon><Delete /></el-icon>
                  </button>
                </div>
              </div>
              </template>
            </draggable>
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

    <!-- 描述放大编辑弹窗：左侧 Markdown 源码，右侧实时预览 -->
    <el-dialog
      v-model="descDialogVisible"
      :fullscreen="descDialogMaximized"
      width="88%"
      :close-on-click-modal="false"
      append-to-body
      @closed="onDescDialogClosed"
    >
      <template #header>
        <div class="desc-dialog-header">
          <span class="desc-dialog-title">编辑描述</span>
          <button
            class="desc-dialog-max-btn"
            type="button"
            :title="descDialogMaximized ? '还原' : '最大化'"
            @click="toggleDescDialogMaximize"
          >
            <el-icon :size="14"><FullScreen /></el-icon>
          </button>
        </div>
      </template>

      <div class="desc-dialog-body" :class="{ 'is-fullscreen': descDialogMaximized }">
        <textarea
          v-model="descDraft"
          class="desc-source"
          placeholder="输入 Markdown 源码..."
          spellcheck="false"
        ></textarea>
        <div class="desc-divider"></div>
        <MarkdownEditor :model-value="descPreview" readonly class="desc-preview" />
      </div>

      <template #footer>
        <el-button size="small" @click="descDialogVisible = false">取消</el-button>
        <el-button type="primary" size="small" @click="confirmDescDialog">确定</el-button>
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

.header-actions {
  display: flex;
  align-items: center;
  gap: 2px;
  -webkit-app-region: no-drag;

  .el-button + .el-button {
    margin-left: 0;
  }
}

.editor-content {
  flex: 1;
  padding: 16px;
  overflow-y: auto;
}

/* 只读模式：简化排版 */
.view-content {
  flex: 1;
  padding: 24px 28px;
  overflow-y: auto;
}

.view-title {
  margin: 0 0 14px;
  font-size: 22px;
  font-weight: 650;
  line-height: 1.35;
  color: #1e293b;
  word-break: break-word;

  &.completed {
    color: #94a3b8;
    text-decoration: line-through;
  }
}

.view-meta {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 8px;
  margin-bottom: 18px;
}

.meta-badge {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 3px 10px;
  border-radius: 999px;
  font-size: 12px;
  font-weight: 500;
  line-height: 1.6;

  .badge-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    flex-shrink: 0;
  }

  &.notify-badge {
    color: #64748b;
    background: #f1f5f9;
  }
}

.view-markdown :deep(.milkdown) {
  padding: 0;
}

.view-empty-desc {
  color: #94a3b8;
  font-size: 13px;
}

/* 描述 Markdown 编辑器 */
.description-editor {
  width: 100%;
  border: 1px solid #e2e8f0;
  border-radius: 8px;
  min-height: 120px;
  max-height: 280px;
  overflow-y: auto;
  background: #ffffff;
  transition: border-color 0.15s ease;

  &:focus-within {
    border-color: #3b82f6;
  }
}

.description-editor :deep(.milkdown) {
  padding: 8px 12px;
  min-height: 110px;
}

.description-editor :deep(.ProseMirror) {
  min-height: 100px;
}

/* 描述 label：文本 + 放大编辑按钮 */
.desc-label {
  display: flex;
  align-items: center;
  gap: 6px;
}

.desc-expand-btn {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 22px;
  height: 22px;
  padding: 0;
  color: #94a3b8;
  background: transparent;
  border: none;
  border-radius: 4px;
  cursor: pointer;
  transition: all 0.15s ease;

  &:hover {
    color: #3b82f6;
    background: #eff6ff;
  }
}

/* 描述放大编辑弹窗 */
.desc-dialog-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  /* 给 el-dialog 自带的关闭 X 留出位置 */
  padding-right: 32px;
}

.desc-dialog-title {
  font-size: 15px;
  font-weight: 600;
  color: #1e293b;
}

.desc-dialog-max-btn {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 24px;
  height: 24px;
  padding: 0;
  color: #94a3b8;
  background: transparent;
  border: none;
  border-radius: 4px;
  cursor: pointer;
  transition: all 0.15s ease;

  &:hover {
    color: #3b82f6;
    background: #eff6ff;
  }
}

.desc-dialog-body {
  display: flex;
  height: 60vh;
  border: 1px solid #e2e8f0;
  border-radius: 8px;
  overflow: hidden;

  &.is-fullscreen {
    /* fullscreen 态撑满：约减去 dialog 的 header / footer / 内边距 */
    height: calc(100vh - 150px);
  }
}

.desc-source {
  flex: 1;
  min-width: 0;
  height: 100%;
  padding: 12px 14px;
  box-sizing: border-box;
  border: none;
  outline: none;
  resize: none;
  font-family: Consolas, 'Courier New', monospace;
  font-size: 13px;
  line-height: 1.6;
  color: #334155;
  background: #f8fafc;
}

.desc-divider {
  width: 1px;
  background: #e2e8f0;
  flex-shrink: 0;
}

.desc-preview {
  flex: 1;
  min-width: 0;
  overflow-y: auto;
  background: #ffffff;
}

.window-footer {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: 10px 16px;
  border-top: 1px solid var(--border);
  gap: 12px;
  flex-wrap: wrap;
}

.footer-right {
  display: flex;
  gap: 6px;
  flex-shrink: 0;
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

    .import-btn {
      display: flex;
      align-items: center;
      justify-content: center;
      width: 28px;
      height: 28px;
      padding: 0;
      color: #64748b;
      background: transparent;
      border: 1px solid #cbd5e1;
      border-radius: 6px;
      cursor: pointer;
      flex-shrink: 0;
      transition: all 0.15s ease;

      &:hover {
        color: #3b82f6;
        border-color: #3b82f6;
        background: #eff6ff;
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

    .subtask-drag-handle {
      opacity: 1;
    }
  }

  &.dragging {
    opacity: 0.5;
  }
}

/* 拖拽手柄：平时淡出，hover 行时显现 */
.subtask-drag-handle {
  flex-shrink: 0;
  color: #94a3b8;
  cursor: grab;
  opacity: 0;
  transition: opacity 0.15s ease;

  &:active {
    cursor: grabbing;
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

  /* 只读模式：复选框只表示状态，不该显得可点 */
  &.readonly .custom-checkbox {
    cursor: default;

    &:hover {
      border-color: #cbd5e1;
    }

    &.checked:hover {
      border-color: transparent;
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

      &.log-btn:hover {
        background: #fef3c7;
        color: #d97706;
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

.weekday-checkbox-group {
  display: flex;
  flex-wrap: wrap;
  gap: 4px 0;
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
