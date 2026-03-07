<script setup lang="ts">
import { ref, nextTick, onBeforeUnmount, watch } from 'vue'
import type { AgentEvent } from '@/types/agent'

const props = defineProps<{
  taskId?: string
}>()

interface LogLine {
  content: string
  level: string
  timestamp: number
}

const logLines = ref<LogLine[]>([])
const containerRef = ref<HTMLElement | null>(null)
const autoScroll = ref(true)
const inputTokens = ref(0)
const outputTokens = ref(0)
const status = ref<'idle' | 'running' | 'completed' | 'failed'>('idle')
const startTime = ref(0)
const elapsed = ref(0)
let timer: ReturnType<typeof setInterval> | null = null

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
      break
    case 'Failed':
      status.value = 'failed'
      stopTimer()
      appendLog(`[失败] ${event.error}`, 'stderr')
      break
  }
}

function startExecution() {
  status.value = 'running'
  startTime.value = Date.now()
  timer = setInterval(() => {
    elapsed.value = Math.floor((Date.now() - startTime.value) / 1000)
  }, 1000)
}

function stopTimer() {
  if (timer) {
    clearInterval(timer)
    timer = null
  }
}

function clearLog() {
  logLines.value = []
  inputTokens.value = 0
  outputTokens.value = 0
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

watch(() => props.taskId, () => {
  clearLog()
  status.value = 'idle'
  elapsed.value = 0
})

onBeforeUnmount(() => {
  stopTimer()
})

defineExpose({ handleEvent, appendLog, startExecution, clearLog })
</script>

<template>
  <div class="agent-log-panel">
    <div
      ref="containerRef"
      class="log-container"
      @scroll="handleScroll"
    >
      <div v-if="logLines.length === 0" class="log-empty">
        等待执行...
      </div>
      <div
        v-for="(line, i) in logLines"
        :key="i"
        class="log-line"
        :class="`log-line--${line.level}`"
      >{{ line.content }}</div>
    </div>
    <div class="status-bar">
      <span v-if="status === 'running'">
        运行中 {{ formatElapsed(elapsed) }}
      </span>
      <span v-else-if="status === 'completed'" class="status-success">
        已完成 {{ formatElapsed(elapsed) }}
      </span>
      <span v-else-if="status === 'failed'" class="status-error">
        已失败
      </span>
      <span v-else>就绪</span>
      <span v-if="inputTokens > 0 || outputTokens > 0">
        Token: {{ inputTokens }} in / {{ outputTokens }} out
      </span>
    </div>
  </div>
</template>

<style scoped>
.agent-log-panel {
  border-radius: var(--radius-base);
  overflow: hidden;
  border: 1px solid var(--border);
}

.log-container {
  background-color: #1e1e1e;
  color: #d4d4d4;
  font-family: 'Consolas', 'Source Code Pro', 'Courier New', monospace;
  font-size: 13px;
  line-height: 1.5;
  padding: 12px;
  overflow-y: auto;
  max-height: 300px;
  min-height: 120px;
}

.log-empty {
  color: #858585;
  font-style: italic;
}

.log-line {
  white-space: pre-wrap;
  word-break: break-all;
}

.log-line--stderr {
  color: #f48771;
}

.log-line--success {
  color: #89d185;
}

.log-line--info {
  color: #6796e6;
}

.status-bar {
  display: flex;
  justify-content: space-between;
  padding: 6px 12px;
  background: #252526;
  font-size: 12px;
  color: #858585;
  font-family: 'Consolas', 'Source Code Pro', 'Courier New', monospace;
}

.status-success {
  color: #89d185;
}

.status-error {
  color: #f48771;
}
</style>
