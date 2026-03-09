<script setup lang="ts">
import { ref, computed, onMounted, onBeforeUnmount, nextTick } from 'vue'
import { useRoute } from 'vue-router'
import { invoke } from '@tauri-apps/api/core'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { marked } from 'marked'
import type { AgentEvent } from '@/types/agent'
import { SCHEDULE_STATUS_MAP } from '@/types/scheduler'
import { STEP_STATUS_MAP } from '@/types/workflow'
import type { WorkflowStepStatus } from '@/types/workflow'

interface CachedLog {
  content: string
  level: string
  timestampMs: number
}

interface ExecutionState {
  taskId: string
  subtaskId: number | null
  agentType: string
  status: string
  logs: CachedLog[]
  error: string | null
  startTimeMs: number
  durationMs: number | null
  inputTokens: number
  outputTokens: number
}

interface WorkflowExecutionInfo {
  taskId: string
  stepOrder: number
  agentType: string
  status: string
  logs: CachedLog[]
  error: string | null
  inputTokens: number
  outputTokens: number
  startTimeMs: number
  durationMs: number
}

interface LogLine {
  content: string
  level: string
  timestamp: number
}

const route = useRoute()
const appWindow = getCurrentWindow()

const subtaskId = route.query.subtaskId ? parseInt(route.query.subtaskId as string) : null
const todoId = route.query.todoId ? parseInt(route.query.todoId as string) : null
const stepOrder = route.query.stepOrder != null ? parseInt(route.query.stepOrder as string) : null
const taskIdParam = route.query.taskId ? decodeURIComponent(route.query.taskId as string) : null
const titleParam = route.query.title ? decodeURIComponent(route.query.title as string) : '执行日志'

const loading = ref(true)
const status = ref<string>('idle')
const agentType = ref('')
const error = ref<string | null>(null)
const startTimeMs = ref(0)
const durationMs = ref(0)
const inputTokens = ref(0)
const outputTokens = ref(0)
const logLines = ref<LogLine[]>([])
const taskId = ref(taskIdParam || '')

const containerRef = ref<HTMLElement | null>(null)
const autoScroll = ref(true)
const elapsed = ref(0)
let timer: ReturnType<typeof setInterval> | null = null
let unlisten: UnlistenFn | null = null

const isRunning = computed(() => status.value === 'running')

function getStatusInfo(s: string) {
  const scheduleInfo = SCHEDULE_STATUS_MAP[s as keyof typeof SCHEDULE_STATUS_MAP]
  if (scheduleInfo) return scheduleInfo
  const stepInfo = STEP_STATUS_MAP[s as WorkflowStepStatus]
  if (stepInfo) return stepInfo
  return { label: s, type: 'info' }
}

function appendLog(content: string, level: string = 'stdout') {
  logLines.value.push({ content, level, timestamp: Date.now() })
  if (autoScroll.value) {
    nextTick(() => {
      if (containerRef.value) {
        containerRef.value.scrollTop = containerRef.value.scrollHeight
      }
    })
  }
}

function handleEvent(event: AgentEvent) {
  switch (event.kind) {
    case 'Log':
      appendLog(event.content || '', event.level || 'stdout')
      break
    case 'Progress':
      appendLog(event.message || '', 'info')
      break
    case 'TokenUsage':
      inputTokens.value += event.inputTokens || 0
      outputTokens.value += event.outputTokens || 0
      break
    case 'Completed':
      status.value = 'completed'
      stopTimer()
      appendLog(`[完成] exit_code=${event.exitCode}`, 'success')
      if (subtaskId) {
        invoke('update_subtask_schedule_status', { subtaskId, targetStatus: 'completed' }).catch(() => {})
      }
      break
    case 'Failed':
      status.value = 'failed'
      stopTimer()
      appendLog(`[失败] ${event.error}`, 'stderr')
      if (subtaskId) {
        invoke('update_subtask_schedule_status', { subtaskId, targetStatus: 'failed' }).catch(() => {})
      }
      break
  }
}

function startTimer(fromTime: number) {
  startTimeMs.value = fromTime
  elapsed.value = Math.floor((Date.now() - fromTime) / 1000)
  stopTimer()
  timer = setInterval(() => {
    elapsed.value = Math.floor((Date.now() - startTimeMs.value) / 1000)
  }, 1000)
}

function stopTimer() {
  if (timer) {
    clearInterval(timer)
    timer = null
  }
}

const renderedMarkdown = computed(() => {
  if (logLines.value.length === 0) return ''
  const deduped = deduplicateLogs(logLines.value)
  const parts = deduped.map(line => {
    if (line.level === 'stderr') return `<span class="log-stderr">${escapeHtml(line.content)}</span>`
    if (line.level === 'success') return `<span class="log-success">${escapeHtml(line.content)}</span>`
    return formatCommandLines(line.content)
  })
  const md = mergeAdjacentCodeBlocks(parts.join('\n\n'))
  return marked.parse(md, { breaks: true, async: false }) as string
})

function deduplicateLogs(lines: LogLine[]): LogLine[] {
  const result: LogLine[] = []
  for (let i = 0; i < lines.length; i++) {
    if (i > 0
      && lines[i].content === lines[i - 1].content
      && lines[i].level === lines[i - 1].level
    ) {
      continue
    }
    result.push(lines[i])
  }
  return result
}

function mergeAdjacentCodeBlocks(text: string): string {
  return text.replace(/```\s*```(\w*\n)?/g, '\n')
}

function formatCommandLines(text: string): string {
  if (text.includes('```')) return text

  const lines = text.split('\n')
  const result: string[] = []
  let cmdBlock: string[] = []
  let patchBlock: string[] = []
  let inPatch = false

  for (const line of lines) {
    const trimmed = line.trim()

    if (trimmed.startsWith('*** Begin Patch') || trimmed.startsWith('*** begin patch')) {
      if (cmdBlock.length > 0) {
        result.push('```\n' + cmdBlock.join('\n') + '\n```')
        cmdBlock = []
      }
      inPatch = true
      patchBlock = [line]
      continue
    }

    if (inPatch) {
      patchBlock.push(line)
      if (trimmed.startsWith('*** End Patch') || trimmed.startsWith('*** end patch')) {
        inPatch = false
        continue
      }
      continue
    }

    if (patchBlock.length > 0) {
      if (trimmed === "'@\"" || trimmed === "'@" || trimmed === '"@' || trimmed === "@'" || trimmed === '') {
        patchBlock.push(line)
        if (trimmed !== '') {
          result.push('```\n' + patchBlock.join('\n') + '\n```')
          patchBlock = []
        }
        continue
      } else {
        result.push('```\n' + patchBlock.join('\n') + '\n```')
        patchBlock = []
      }
    }

    if (trimmed.startsWith('$ ')) {
      cmdBlock.push(trimmed)
    } else {
      if (cmdBlock.length > 0) {
        result.push('```\n' + cmdBlock.join('\n') + '\n```')
        cmdBlock = []
      }
      result.push(line)
    }
  }

  if (cmdBlock.length > 0) {
    result.push('```\n' + cmdBlock.join('\n') + '\n```')
  }
  if (patchBlock.length > 0) {
    result.push('```diff\n' + patchBlock.join('\n') + '\n```')
  }

  return result.join('\n')
}

function escapeHtml(str: string): string {
  return str
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
}

function handleScroll() {
  if (!containerRef.value) return
  const { scrollTop, scrollHeight, clientHeight } = containerRef.value
  autoScroll.value = scrollHeight - scrollTop - clientHeight < 30
}

function formatElapsed(secs: number): string {
  const m = Math.floor(secs / 60)
  const s = secs % 60
  return m > 0 ? `${m}m ${s}s` : `${s}s`
}

function formatDuration(ms: number): string {
  if (ms < 1000) return `${ms}ms`
  const seconds = Math.round(ms / 1000)
  if (seconds < 60) return `${seconds}s`
  const minutes = Math.floor(seconds / 60)
  const secs = seconds % 60
  return `${minutes}m ${secs}s`
}

function formatTime(timestampMs: number): string {
  if (!timestampMs) return '-'
  return new Date(timestampMs).toLocaleString('zh-CN')
}

async function loadData() {
  loading.value = true
  try {
    if (subtaskId != null) {
      await loadSubtaskLog()
    } else if (todoId != null && stepOrder != null) {
      await loadWorkflowStepLog()
    }
  } catch (e) {
    console.error('加载日志失败:', e)
  } finally {
    loading.value = false
  }
}

async function loadSubtaskLog() {
  if (taskIdParam) {
    const state = await invoke<ExecutionState | null>('get_agent_execution_state', {
      taskId: taskIdParam,
    })
    if (state) {
      applyExecutionState(state)
      return
    }
  }

  const state = await invoke<ExecutionState | null>('get_agent_execution_by_subtask', {
    subtaskId: subtaskId!,
  })
  if (state) {
    applyExecutionState(state)
  }
}

async function loadWorkflowStepLog() {
  const executions = await invoke<WorkflowExecutionInfo[]>('get_workflow_executions', {
    todoId: todoId!,
  })
  const match = executions.find(e => e.stepOrder === stepOrder)
  if (match) {
    status.value = match.status
    agentType.value = match.agentType
    error.value = match.error
    startTimeMs.value = match.startTimeMs
    durationMs.value = match.durationMs
    inputTokens.value = match.inputTokens
    outputTokens.value = match.outputTokens
    for (const log of match.logs) {
      logLines.value.push({ content: log.content, level: log.level, timestamp: log.timestampMs })
    }
  }
}

function applyExecutionState(state: ExecutionState) {
  taskId.value = state.taskId
  status.value = state.status
  agentType.value = state.agentType
  error.value = state.error
  startTimeMs.value = state.startTimeMs
  durationMs.value = state.durationMs ?? 0
  inputTokens.value = state.inputTokens ?? 0
  outputTokens.value = state.outputTokens ?? 0
  for (const log of state.logs) {
    logLines.value.push({ content: log.content, level: log.level, timestamp: log.timestampMs })
  }
}

async function setupRealtime() {
  if (!taskId.value) return
  if (status.value !== 'running') return

  startTimer(startTimeMs.value || Date.now())
  unlisten = await listen<AgentEvent>(`agent:log:${taskId.value}`, (event) => {
    handleEvent(event.payload)
  })
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

const cancelling = ref(false)

async function cancelExecution() {
  if (!taskId.value || cancelling.value) return
  cancelling.value = true
  try {
    await invoke('cancel_agent_execution', { taskId: taskId.value })
    status.value = 'cancelled'
    stopTimer()
    appendLog('[已终止] 用户手动终止了任务', 'stderr')
    if (subtaskId) {
      invoke('update_subtask_schedule_status', { subtaskId, targetStatus: 'cancelled' }).catch(() => {})
    }
  } catch (e) {
    console.error('终止任务失败:', e)
  } finally {
    cancelling.value = false
  }
}

const reExecuting = ref(false)

async function reExecute() {
  if (!subtaskId || reExecuting.value) return
  reExecuting.value = true
  try {
    const subtask = await invoke<any>('get_subtask', { id: subtaskId })
    if (!subtask) throw new Error('子任务不存在')

    const todos = await invoke<any[]>('get_todos')
    const todo = todos.find((t: any) => t.id === subtask.parentId)
    if (!todo || !todo.agentId) throw new Error('未配置 Agent')

    const newTaskId = `subtask-${subtaskId}-${Date.now()}`

    if (unlisten) {
      unlisten()
      unlisten = null
    }
    stopTimer()
    logLines.value = []
    status.value = 'running'
    error.value = null
    inputTokens.value = 0
    outputTokens.value = 0
    taskId.value = newTaskId

    await invoke('update_subtask_schedule_status', {
      subtaskId,
      targetStatus: 'running',
    }).catch(() => {})

    const prompt = subtask.content || subtask.title
    await invoke('start_agent_execution', {
      agentId: todo.agentId,
      prompt,
      projectPath: todo.agentProjectPath || '',
      taskId: newTaskId,
      subtaskId,
    })

    startTimer(Date.now())
    unlisten = await listen<AgentEvent>(`agent:log:${newTaskId}`, (event) => {
      handleEvent(event.payload)
    })
  } catch (e: any) {
    status.value = 'failed'
    error.value = String(e)
    appendLog(`[重新执行失败] ${e}`, 'stderr')
  } finally {
    reExecuting.value = false
  }
}

onMounted(async () => {
  await loadData()
  await setupRealtime()
  nextTick(() => {
    if (containerRef.value) {
      containerRef.value.scrollTop = containerRef.value.scrollHeight
    }
  })
})

onBeforeUnmount(() => {
  stopTimer()
  if (unlisten) {
    unlisten()
    unlisten = null
  }
})
</script>

<template>
  <div class="log-window">
    <div class="window-header">
      <h2>{{ titleParam }}</h2>
      <div class="window-controls">
        <button class="control-btn maximize-btn" title="最大化" @click="handleMaximize">
          <el-icon :size="14"><FullScreen /></el-icon>
        </button>
        <button class="control-btn close-btn" title="关闭" @click="handleClose">
          <el-icon :size="14"><Close /></el-icon>
        </button>
      </div>
    </div>

    <div class="log-meta">
      <div class="meta-items">
        <div class="meta-item">
          <span class="meta-label">状态</span>
          <el-tag
            :type="getStatusInfo(status).type as any"
            size="small"
            effect="light"
          >
            {{ getStatusInfo(status).label }}
          </el-tag>
        </div>
        <div v-if="agentType" class="meta-item">
          <span class="meta-label">Agent</span>
          <span class="meta-value">{{ agentType }}</span>
        </div>
        <div v-if="startTimeMs" class="meta-item">
          <span class="meta-label">开始</span>
          <span class="meta-value">{{ formatTime(startTimeMs) }}</span>
        </div>
        <div class="meta-item">
          <span class="meta-label">耗时</span>
          <span class="meta-value">
            <template v-if="isRunning">{{ formatElapsed(elapsed) }}</template>
            <template v-else>{{ formatDuration(durationMs) }}</template>
          </span>
        </div>
        <div v-if="inputTokens > 0 || outputTokens > 0" class="meta-item">
          <span class="meta-label">Tokens</span>
          <span class="meta-value">{{ inputTokens }} in / {{ outputTokens }} out</span>
        </div>
      </div>
      <div v-if="error" class="meta-error">
        <span class="meta-label">错误</span>
        <span class="error-text">{{ error }}</span>
      </div>
    </div>

    <div class="log-body" v-loading="loading">
      <div
        ref="containerRef"
        class="log-container"
        @scroll="handleScroll"
      >
        <div v-if="logLines.length === 0 && !loading" class="log-empty">
          暂无日志输出
        </div>
        <div
          v-else
          class="log-markdown"
          v-html="renderedMarkdown"
        ></div>
      </div>
    </div>

    <div class="status-bar">
      <div class="status-left">
        <span v-if="isRunning" class="status-running">
          运行中 {{ formatElapsed(elapsed) }}
        </span>
        <span v-else-if="status === 'completed'" class="status-success">
          已完成
        </span>
        <span v-else-if="status === 'failed'" class="status-error">
          已失败
        </span>
        <span v-else-if="status === 'cancelled'" class="status-error">
          已终止
        </span>
        <span v-else>{{ logLines.length }} 条日志</span>
      </div>
      <div class="status-right">
        <span v-if="inputTokens > 0 || outputTokens > 0" class="token-info">
          Token: {{ inputTokens }} in / {{ outputTokens }} out
        </span>
        <button
          v-if="isRunning"
          class="action-btn cancel-btn"
          :disabled="cancelling"
          @click="cancelExecution"
        >
          {{ cancelling ? '终止中...' : '终止任务' }}
        </button>
        <button
          v-if="subtaskId && !isRunning && (status === 'completed' || status === 'failed' || status === 'cancelled')"
          class="action-btn rerun-btn"
          :disabled="reExecuting"
          @click="reExecute"
        >
          {{ reExecuting ? '启动中...' : '重新执行' }}
        </button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.log-window {
  display: flex;
  flex-direction: column;
  height: 100vh;
  background: #ffffff;
}

.window-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 10px 16px;
  min-height: 40px;
  box-sizing: border-box;
  border-bottom: 1px solid #e2e8f0;
  -webkit-app-region: drag;
  flex-shrink: 0;

  h2 {
    margin: 0;
    font-size: 14px;
    font-weight: 600;
    line-height: 1.2;
    color: #334155;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: calc(100% - 80px);
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

.log-meta {
  padding: 10px 16px;
  border-bottom: 1px solid #e2e8f0;
  flex-shrink: 0;
  background: #f8fafc;
}

.meta-items {
  display: flex;
  flex-wrap: wrap;
  gap: 16px;
  align-items: center;
}

.meta-item {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 12px;
}

.meta-label {
  color: #94a3b8;
  flex-shrink: 0;
}

.meta-value {
  color: #334155;
}

.meta-error {
  display: flex;
  gap: 6px;
  margin-top: 8px;
  font-size: 12px;
  align-items: flex-start;
}

.error-text {
  color: #ef4444;
  word-break: break-all;
}

.log-body {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
}

.log-container {
  flex: 1;
  background-color: #1e1e1e;
  color: #d4d4d4;
  font-family: 'Consolas', 'Source Code Pro', 'Courier New', monospace;
  font-size: 13px;
  line-height: 1.5;
  padding: 12px;
  overflow-y: auto;
}

.log-empty {
  color: #858585;
  font-style: italic;
  text-align: center;
  padding: 40px 0;
}

.log-markdown {
  word-break: break-word;
}

.log-markdown :deep(p) {
  margin: 4px 0;
  line-height: 1.6;
}

.log-markdown :deep(pre) {
  background: #2d2d2d;
  border: 1px solid #404040;
  border-radius: 4px;
  padding: 8px 12px;
  margin: 6px 0;
  font-size: 12px;
  white-space: pre-wrap;
  word-break: break-all;
}

.log-markdown :deep(code) {
  background: #2d2d2d;
  padding: 1px 5px;
  border-radius: 3px;
  font-size: 12px;
  color: #e6db74;
}

.log-markdown :deep(pre code) {
  padding: 0;
  background: transparent;
  color: #d4d4d4;
}

.log-markdown :deep(strong) {
  color: #6796e6;
}

.log-markdown :deep(.log-stderr) {
  color: #f48771;
}

.log-markdown :deep(.log-success) {
  color: #89d185;
}

.log-markdown :deep(hr) {
  border: none;
  border-top: 1px solid #404040;
  margin: 8px 0;
}

.log-markdown :deep(h1),
.log-markdown :deep(h2),
.log-markdown :deep(h3) {
  color: #6796e6;
  margin: 8px 0 4px;
  font-size: 14px;
}

.status-bar {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: 6px 16px;
  background: #252526;
  font-size: 12px;
  color: #858585;
  font-family: 'Consolas', 'Source Code Pro', 'Courier New', monospace;
  flex-shrink: 0;
}

.status-left {
  display: flex;
  align-items: center;
}

.status-right {
  display: flex;
  align-items: center;
  gap: 10px;
}

.status-running {
  color: #dcdcaa;
}

.status-success {
  color: #89d185;
}

.status-error {
  color: #f48771;
}

.token-info {
  color: #858585;
}

.action-btn {
  padding: 2px 10px;
  font-size: 11px;
  border: 1px solid;
  border-radius: 3px;
  cursor: pointer;
  font-family: inherit;
  transition: all 0.15s ease;
}

.action-btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.cancel-btn {
  color: #f48771;
  border-color: #f4877166;
  background: transparent;
}

.cancel-btn:hover:not(:disabled) {
  background: #f4877122;
}

.rerun-btn {
  color: #6796e6;
  border-color: #6796e666;
  background: transparent;
}

.rerun-btn:hover:not(:disabled) {
  background: #6796e622;
}
</style>
