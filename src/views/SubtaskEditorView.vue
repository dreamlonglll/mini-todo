<script setup lang="ts">
import { ref, onMounted, onBeforeUnmount, nextTick, computed } from 'vue'
import { useRoute } from 'vue-router'
import { invoke } from '@tauri-apps/api/core'
import { convertFileSrc } from '@tauri-apps/api/core'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { ElMessage } from 'element-plus'
import { Editor, rootCtx, defaultValueCtx, editorViewOptionsCtx } from '@milkdown/kit/core'
import { commonmark } from '@milkdown/kit/preset/commonmark'
import { listener, listenerCtx } from '@milkdown/kit/plugin/listener'
import { upload, uploadConfig } from '@milkdown/kit/plugin/upload'
import { Decoration } from '@milkdown/kit/prose/view'
import { nord } from '@milkdown/theme-nord'
import type { Node } from '@milkdown/kit/prose/model'
import type { Uploader, UploadOptions } from '@milkdown/kit/plugin/upload'
import '@milkdown/theme-nord/style.css'
import { useAgentStore } from '@/stores/agentStore'
import { useSchedulerStore } from '@/stores/schedulerStore'
import { AGENT_TYPE_INFO } from '@/types/agent'
import type { PromptTemplate, TemplateVariable } from '@/types/scheduler'
import AgentLogPanel from '@/components/AgentLogPanel.vue'

const route = useRoute()
const subtaskId = parseInt(route.query.id as string)
const agentIdParam = route.query.agentId ? parseInt(route.query.agentId as string) : null
const agentProjectPath = route.query.agentProjectPath
  ? decodeURIComponent(route.query.agentProjectPath as string)
  : ''
const isViewMode = route.query.mode === 'view'
const appWindow = getCurrentWindow()

const title = ref('')
const markdownContent = ref('')
const editorContainer = ref<HTMLDivElement | null>(null)
let editorInstance: Editor | null = null

// 图片预览
const previewVisible = ref(false)
const previewUrls = ref<string[]>([])
const previewInitialIndex = ref(0)

function handleImageClick(e: MouseEvent) {
  const target = e.target as HTMLElement
  if (target.tagName !== 'IMG') return

  const imgSrc = (target as HTMLImageElement).src
  if (!imgSrc) return

  e.preventDefault()
  e.stopPropagation()

  const container = editorContainer.value
  if (!container) return

  const allImages = Array.from(container.querySelectorAll('.ProseMirror img'))
  const urls = allImages.map(img => (img as HTMLImageElement).src).filter(Boolean)

  if (urls.length === 0) return

  previewUrls.value = urls
  previewInitialIndex.value = Math.max(0, urls.indexOf(imgSrc))
  previewVisible.value = true
}

async function imageUploader(files: FileList, schema: any): Promise<Node[]> {
  const images: File[] = []
  for (let i = 0; i < files.length; i++) {
    const file = files.item(i)
    if (file && file.type.includes('image')) {
      images.push(file)
    }
  }

  const nodes: Node[] = await Promise.all(
    images.map(async (image) => {
      const arrayBuffer = await image.arrayBuffer()
      const uint8 = new Uint8Array(arrayBuffer)
      let binary = ''
      for (let i = 0; i < uint8.length; i++) {
        binary += String.fromCharCode(uint8[i])
      }
      const base64 = btoa(binary)

      const ext = image.name.split('.').pop() || 'png'
      const fileName = `${Date.now()}_${Math.random().toString(36).slice(2, 8)}.${ext}`

      const filePath = await invoke<string>('save_subtask_image', {
        imageData: base64,
        fileName,
      })

      const src = convertFileSrc(filePath)
      return schema.nodes.image.createAndFill({
        src,
        alt: image.name,
      }) as Node
    })
  )

  return nodes
}

async function initEditor() {
  if (!editorContainer.value) return

  const builder = Editor.make()
    .config(nord)
    .config((ctx) => {
      ctx.set(rootCtx, editorContainer.value!)
      ctx.set(defaultValueCtx, markdownContent.value || '')
      if (isViewMode) {
        ctx.update(editorViewOptionsCtx, (prev) => ({
          ...prev,
          editable: () => false,
        }))
      } else {
        ctx.get(listenerCtx).markdownUpdated((_ctx, markdown, prevMarkdown) => {
          if (markdown !== prevMarkdown) {
            markdownContent.value = markdown
          }
        })
        ctx.set(uploadConfig.key, {
          uploader: imageUploader as Uploader,
          enableHtmlFileUploader: true,
          uploadWidgetFactory: (pos, spec) => Decoration.widget(pos, document.createElement('span'), spec),
        } satisfies UploadOptions)
      }
    })
    .use(commonmark)

  if (!isViewMode) {
    builder.use(listener).use(upload)
  }

  editorInstance = await builder.create()
}

function destroyEditor() {
  if (editorInstance) {
    editorInstance.destroy()
    editorInstance = null
  }
}

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

// ========== Agent 执行 ==========
const agentStore = useAgentStore()
const agentDialogVisible = ref(false)

const agentForm = ref({
  agentId: agentIdParam,
  projectPath: agentProjectPath,
  prompt: '',
})

const hasAgentConfig = computed(() => !!agentForm.value.agentId)

const currentExecution = computed(() => agentStore.getExecutionForSubtask(subtaskId))
const agentExecuting = computed(() => currentExecution.value?.status === 'running')
const agentTaskId = computed(() => currentExecution.value?.taskId || '')

const currentAgentLabel = computed(() => {
  if (!agentForm.value.agentId) return ''
  const agent = agentStore.agents.find(a => a.id === agentForm.value.agentId)
  if (!agent) return ''
  const typeInfo = AGENT_TYPE_INFO[agent.agentType]
  return typeInfo?.label || agent.agentType
})

const logPanelStatus = computed<'idle' | 'running' | 'completed' | 'failed'>(() => {
  return (currentExecution.value?.status as 'idle' | 'running' | 'completed' | 'failed') || 'idle'
})

const logPanelLogs = computed(() => {
  return currentExecution.value?.logs || []
})

function buildPromptContext(): string {
  const lines: string[] = []
  const hasTitle = !!title.value.trim()
  const hasContent = !!markdownContent.value.trim()

  if (hasTitle) {
    lines.push(`【任务标题】${title.value.trim()}`)
  }
  if (hasContent) {
    if (hasTitle) lines.push('')
    lines.push(`【任务详情】`)
    lines.push(markdownContent.value.trim())
  }
  if (lines.length > 0) {
    lines.push('')
    lines.push('请根据以上任务信息执行相应操作。')
  }
  return lines.join('\n')
}

function openAgentDialog() {
  if (!hasAgentConfig.value) {
    ElMessage.warning('请先在待办编辑页配置 Agent')
    return
  }

  if (!currentExecution.value) {
    agentForm.value.prompt = buildPromptContext()
  }
  loadTemplates()
  agentDialogVisible.value = true
}

// ========== Prompt 模板 ==========
const schedulerStore = useSchedulerStore()
const selectedTemplateId = ref<number | null>(null)
const templateList = ref<PromptTemplate[]>([])
const templateVariables = ref<TemplateVariable[]>([])
const variableValues = ref<Record<string, string>>({})

async function loadTemplates() {
  try {
    await schedulerStore.loadTemplates()
    templateList.value = schedulerStore.templates
  } catch (_) {
    templateList.value = []
  }
}

function onTemplateSelect(id: number | null) {
  if (!id) {
    templateVariables.value = []
    variableValues.value = {}
    return
  }
  const tpl = templateList.value.find(t => t.id === id)
  if (!tpl) return

  try {
    const vars: TemplateVariable[] = JSON.parse(tpl.variables || '[]')
    templateVariables.value = vars
    variableValues.value = {}
    for (const v of vars) {
      variableValues.value[v.name] = v.defaultValue || ''
    }
  } catch (_) {
    templateVariables.value = []
    variableValues.value = {}
  }
}

async function applyTemplate() {
  if (!selectedTemplateId.value) return
  try {
    const rendered = await schedulerStore.renderTemplate(selectedTemplateId.value, variableValues.value)
    agentForm.value.prompt = rendered
    ElMessage.success('模板已应用')
  } catch (e) {
    ElMessage.error('应用模板失败: ' + String(e))
  }
}

async function handleAgentExecute(background: boolean = false) {
  if (!agentForm.value.agentId) {
    ElMessage.warning('未配置 Agent')
    return
  }
  if (!agentForm.value.prompt.trim()) {
    ElMessage.warning('请输入执行指令')
    return
  }
  if (!agentForm.value.projectPath.trim()) {
    ElMessage.warning('未配置项目路径，请在待办编辑页设置')
    return
  }

  const taskId = `task-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`

  try {
    await agentStore.startBackgroundExecution(
      agentForm.value.agentId,
      agentForm.value.prompt,
      agentForm.value.projectPath,
      taskId,
      subtaskId,
    )
    if (background) {
      agentDialogVisible.value = false
      ElMessage.info('Agent 已在后台执行')
    }
  } catch (e) {
    ElMessage.error('Agent 启动失败: ' + String(e))
  }
}

async function handleAgentCancel() {
  if (!agentTaskId.value) return
  try {
    await agentStore.cancelExecution(agentTaskId.value)
    ElMessage.info('已发送取消请求')
  } catch (e) {
    ElMessage.error('取消失败: ' + String(e))
  }
}

function handleClearExecution() {
  agentStore.removeExecution(subtaskId)
  agentForm.value.prompt = buildPromptContext()
}

onMounted(async () => {
  await loadSubtask()
  await nextTick()
  await initEditor()
  editorContainer.value?.addEventListener('click', handleImageClick)
  agentStore.loadAgents()
  agentStore.restoreExecutionForSubtask(subtaskId)
})

onBeforeUnmount(() => {
  editorContainer.value?.removeEventListener('click', handleImageClick)
  destroyEditor()
})
</script>

<template>
  <div class="subtask-editor-window">
    <div class="window-header">
      <h2>{{ isViewMode ? '查看子任务' : '编辑子任务' }}</h2>
      <div class="window-controls">
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
        <div ref="editorContainer" class="milkdown-editor-wrapper"></div>
      </div>
    </div>

    <div class="window-footer">
      <div class="footer-left">
        <el-button
          v-if="hasAgentConfig"
          :type="agentExecuting ? 'warning' : currentExecution?.status === 'completed' ? 'success' : currentExecution?.status === 'failed' ? 'danger' : 'info'"
          plain
          @click="openAgentDialog"
        >
          <el-icon><MagicStick /></el-icon>
          <template v-if="agentExecuting">
            执行中...
          </template>
          <template v-else-if="currentExecution?.status === 'completed'">
            执行完成
          </template>
          <template v-else-if="currentExecution?.status === 'failed'">
            执行失败
          </template>
          <template v-else>
            {{ currentAgentLabel || 'Agent' }}
          </template>
        </el-button>
      </div>
      <div class="footer-right">
        <el-button @click="handleClose">
          <el-icon><Close /></el-icon>
          {{ isViewMode ? '关闭' : '取消' }}
        </el-button>
        <el-button v-if="!isViewMode" type="primary" @click="handleSave" :disabled="!title.trim()">
          <el-icon><Check /></el-icon>
          保存
        </el-button>
      </div>
    </div>

    <!-- Agent 执行对话框 -->
    <el-dialog
      v-model="agentDialogVisible"
      title="Agent 执行"
      width="80%"
      append-to-body
      class="agent-exec-dialog"
      top="10vh"
    >
      <div class="agent-dialog-body" style="max-height: calc(80vh - 160px); overflow-y: auto;">
        <el-form label-position="top" size="default">
          <el-form-item label="Agent">
            <el-input :model-value="currentAgentLabel" disabled />
          </el-form-item>

          <el-form-item label="项目路径">
            <el-input :model-value="agentForm.projectPath" disabled />
          </el-form-item>

          <el-form-item v-if="!currentExecution && !isViewMode" label="Prompt 模板">
            <div class="template-selector">
              <el-select
                v-model="selectedTemplateId"
                placeholder="选择模板（可选）"
                clearable
                size="small"
                style="width: 100%"
                @change="onTemplateSelect"
              >
                <el-option
                  v-for="tpl in templateList"
                  :key="tpl.id"
                  :label="tpl.name"
                  :value="tpl.id"
                >
                  <div style="display: flex; justify-content: space-between; align-items: center;">
                    <span>{{ tpl.name }}</span>
                    <span style="font-size: 11px; color: #999;">{{ tpl.category }}</span>
                  </div>
                </el-option>
              </el-select>

              <div v-if="templateVariables.length > 0" class="template-vars">
                <div
                  v-for="v in templateVariables"
                  :key="v.name"
                  class="template-var-item"
                >
                  <label>{{ v.label }}{{ v.required ? ' *' : '' }}</label>
                  <el-input
                    v-if="v.type === 'textarea'"
                    v-model="variableValues[v.name]"
                    type="textarea"
                    :rows="2"
                    size="small"
                  />
                  <el-input
                    v-else
                    v-model="variableValues[v.name]"
                    size="small"
                  />
                </div>
                <el-button type="primary" size="small" @click="applyTemplate">
                  应用模板
                </el-button>
              </div>
            </div>
          </el-form-item>

          <el-form-item v-if="!currentExecution" required>
            <template #label>
              <span style="display: flex; align-items: center; gap: 6px;">
                <span>执行指令</span>
                <el-tag size="small" type="info" effect="plain">
                  基于子任务标题+内容生成
                </el-tag>
              </span>
            </template>
            <el-input
              v-model="agentForm.prompt"
              type="textarea"
              :rows="8"
              placeholder="输入要 Agent 执行的指令..."
              :disabled="agentExecuting"
            />
          </el-form-item>
        </el-form>

        <AgentLogPanel
          v-if="agentTaskId"
          :key="agentTaskId"
          :task-id="agentTaskId"
          :agent-type="currentExecution?.agentType || ''"
          :initial-status="logPanelStatus"
          :initial-logs="logPanelLogs"
          :initial-start-time="currentExecution?.startTimeMs"
        />
      </div>

      <template #footer>
        <div class="agent-dialog-footer">
          <div class="footer-left-actions">
            <el-button
              v-if="agentExecuting && !isViewMode"
              type="danger"
              size="small"
              @click="handleAgentCancel"
            >
              <el-icon><CircleClose /></el-icon>
              取消执行
            </el-button>
            <el-button
              v-if="currentExecution && !agentExecuting && !isViewMode"
              size="small"
              @click="handleClearExecution"
            >
              清除记录
            </el-button>
          </div>
          <div class="footer-right-actions">
            <el-button @click="agentDialogVisible = false">
              关闭
            </el-button>
            <template v-if="!currentExecution && !isViewMode">
              <el-button type="primary" @click="handleAgentExecute(false)">
                <el-icon><VideoPlay /></el-icon>
                开始执行
              </el-button>
              <el-button type="success" @click="handleAgentExecute(true)">
                <el-icon><Position /></el-icon>
                后台执行
              </el-button>
            </template>
          </div>
        </div>
      </template>
    </el-dialog>

    <!-- 图片预览 -->
    <el-image-viewer
      v-if="previewVisible"
      :url-list="previewUrls"
      :initial-index="previewInitialIndex"
      :z-index="10000"
      @close="previewVisible = false"
    />
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
  justify-content: space-between;
  align-items: center;
  padding: 12px 16px;
  border-top: 1px solid var(--border, #e2e8f0);
}

.footer-left {
  display: flex;
  gap: 8px;
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
  padding: 12px 16px;
  min-height: 290px;
}

.milkdown-editor-wrapper :deep(.editor) {
  outline: none;
}

.milkdown-editor-wrapper :deep(.ProseMirror) {
  outline: none;
  min-height: 280px;
}

.milkdown-editor-wrapper :deep(.ProseMirror p) {
  margin: 0.4em 0;
  line-height: 1.6;
}

.milkdown-editor-wrapper :deep(.ProseMirror h1),
.milkdown-editor-wrapper :deep(.ProseMirror h2),
.milkdown-editor-wrapper :deep(.ProseMirror h3) {
  margin: 0.6em 0 0.3em;
}

.milkdown-editor-wrapper :deep(.ProseMirror img) {
  max-width: 100%;
  height: auto;
  border-radius: 6px;
  margin: 8px 0;
  cursor: pointer;
  transition: opacity 0.15s ease;

  &:hover {
    opacity: 0.85;
    box-shadow: 0 2px 8px rgba(0, 0, 0, 0.15);
  }
}

.milkdown-editor-wrapper :deep(.ProseMirror code) {
  background: #f1f5f9;
  padding: 2px 6px;
  border-radius: 4px;
  font-size: 0.9em;
}

.milkdown-editor-wrapper :deep(.ProseMirror pre) {
  background: #1e293b;
  color: #e2e8f0;
  padding: 12px 16px;
  border-radius: 8px;
  overflow-x: auto;
}

.milkdown-editor-wrapper :deep(.ProseMirror blockquote) {
  border-left: 3px solid #3b82f6;
  padding-left: 12px;
  color: #64748b;
  margin: 0.5em 0;
}

.milkdown-editor-wrapper :deep(.ProseMirror ul),
.milkdown-editor-wrapper :deep(.ProseMirror ol) {
  padding-left: 24px;
  margin: 0.4em 0;
}

.milkdown-editor-wrapper :deep(.ProseMirror hr) {
  border: none;
  border-top: 1px solid #e2e8f0;
  margin: 1em 0;
}

.template-selector {
  width: 100%;
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.template-vars {
  display: flex;
  flex-direction: column;
  gap: 8px;
  padding: 8px;
  background: #f8fafc;
  border-radius: 6px;
  border: 1px solid #e2e8f0;
}

.template-var-item {
  display: flex;
  flex-direction: column;
  gap: 4px;

  label {
    font-size: 12px;
    color: #64748b;
    font-weight: 500;
  }
}
</style>

<style>
.agent-exec-dialog .el-dialog__body {
  padding: 12px 20px 0;
}
.agent-exec-dialog .el-dialog__footer {
  padding: 12px 20px;
}
.agent-dialog-body {
  padding-right: 4px;
  padding-bottom: 8px;
}
.agent-dialog-footer {
  display: flex;
  justify-content: space-between;
  align-items: center;
  width: 100%;
}
.agent-dialog-footer .footer-left-actions {
  display: flex;
  gap: 8px;
}
.agent-dialog-footer .footer-right-actions {
  display: flex;
  gap: 8px;
}
</style>
