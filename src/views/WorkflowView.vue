<script setup lang="ts">
import { ref, computed, onMounted } from 'vue'
import { useRoute } from 'vue-router'
import { invoke } from '@tauri-apps/api/core'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { ElMessage, ElMessageBox } from 'element-plus'
import { useSchedulerStore } from '@/stores/schedulerStore'
import type { PromptTemplate } from '@/types/scheduler'
import type { WorkflowStep, WorkflowStepStatus } from '@/types/workflow'
import { STEP_STATUS_MAP } from '@/types/workflow'
import { openLogWindow } from '@/utils/logWindow'

interface SubtaskOption {
  id: number
  title: string
  content: string
  completed: boolean
}

const route = useRoute()
const todoId = parseInt(route.query.todoId as string)
const appWindow = getCurrentWindow()

const schedulerStore = useSchedulerStore()
const loading = ref(true)

const todoTitle = ref('')
const workflowEnabled = ref(false)
const workflowCurrentStep = ref(-1)
const subtasks = ref<SubtaskOption[]>([])
const workflowSteps = ref<Array<{ stepType: string; subtaskId?: number; promptText?: string; carryContext?: boolean }>>([])
const workflowProgress = ref<WorkflowStep[]>([])

const activeSection = ref<'workflow' | 'prompts'>('workflow')

// ========== Prompt Library ==========
const promptLibrary = ref<PromptTemplate[]>([])
const editingPrompt = ref<Partial<PromptTemplate> | null>(null)
const isCreatingPrompt = ref(false)
const promptForm = ref({
  name: '',
  category: '',
  description: '',
  templateContent: '',
})

const promptCategories = computed(() => {
  const cats = new Set(promptLibrary.value.map(p => p.category).filter(Boolean))
  return Array.from(cats)
})

onMounted(async () => {
  await loadData()
})

async function loadData() {
  loading.value = true
  try {
    const todos = await invoke<any[]>('get_todos')
    const todo = todos.find(t => t.id === todoId)
    if (todo) {
      todoTitle.value = todo.title
      workflowEnabled.value = !!todo.workflowEnabled
      workflowCurrentStep.value = typeof todo.workflowCurrentStep === 'number' ? todo.workflowCurrentStep : -1
      subtasks.value = (todo.subtasks || []).map((s: any) => ({
        id: s.id,
        title: s.title,
        content: s.content || '',
        completed: s.completed,
      }))
    }

    workflowProgress.value = await invoke<WorkflowStep[]>('get_workflow_steps', { todoId })
    workflowSteps.value = workflowProgress.value.map(s => {
      const step: { stepType: string; subtaskId?: number; promptText?: string; carryContext?: boolean } = {
        stepType: s.stepType,
        subtaskId: s.subtaskId,
        promptText: s.promptText,
        carryContext: s.carryContext ?? false,
      }
      if (s.stepType === 'subtask' && s.subtaskId && !s.promptText) {
        step.promptText = buildSubtaskPrompt(s.subtaskId)
      }
      return step
    })

    await loadPromptLibrary()
  } catch (e) {
    console.error('Load data failed:', e)
  } finally {
    loading.value = false
  }
}

async function loadPromptLibrary() {
  try {
    await schedulerStore.loadTemplates()
    promptLibrary.value = [...schedulerStore.templates]
  } catch { /* ignore */ }
}

// ========== Workflow Steps ==========
function addWorkflowStep() {
  workflowSteps.value.push({ stepType: 'subtask' })
}

function moveWorkflowStep(idx: number, dir: 'up' | 'down') {
  const target = dir === 'up' ? idx - 1 : idx + 1
  if (target < 0 || target >= workflowSteps.value.length) return
  const tmp = workflowSteps.value[idx]
  workflowSteps.value[idx] = workflowSteps.value[target]
  workflowSteps.value[target] = tmp
  if (workflowSteps.value[0]?.carryContext) {
    workflowSteps.value[0].carryContext = false
  }
}

function removeWorkflowStep(idx: number) {
  workflowSteps.value.splice(idx, 1)
}

function availableSubtasksForStep(currentIdx: number) {
  const currentSubtaskId = workflowSteps.value[currentIdx]?.subtaskId
  const usedIds = workflowSteps.value
    .filter((s, i) => i !== currentIdx && s.stepType === 'subtask' && s.subtaskId)
    .map(s => s.subtaskId)
  return subtasks.value.filter(st =>
    (!st.completed || st.id === currentSubtaskId) && !usedIds.includes(st.id)
  )
}

function selectPromptForStep(idx: number, template: PromptTemplate) {
  workflowSteps.value[idx].promptText = template.templateContent
}

function buildSubtaskPrompt(subtaskId: number): string {
  const st = subtasks.value.find(s => s.id === subtaskId)
  if (!st) return ''
  const lines: string[] = []
  if (st.title.trim()) {
    lines.push(`【任务标题】${st.title.trim()}`)
  }
  if (st.content.trim()) {
    if (lines.length > 0) lines.push('')
    lines.push('【任务详情】')
    lines.push(st.content.trim())
  }
  if (lines.length > 0) {
    lines.push('')
    lines.push('请根据以上任务信息执行相应操作。')
  }
  return lines.join('\n')
}

function onSubtaskSelected(idx: number, subtaskId: number | undefined) {
  if (subtaskId) {
    workflowSteps.value[idx].promptText = buildSubtaskPrompt(subtaskId)
  } else {
    workflowSteps.value[idx].promptText = undefined
  }
}

function handleClearWorkflow() {
  workflowSteps.value = []
}

async function handleSave() {
  try {
    await invoke('set_workflow_steps', {
      todoId,
      steps: workflowSteps.value.map(s => ({
        stepType: s.stepType,
        subtaskId: s.subtaskId ?? null,
        promptText: s.promptText ?? null,
        carryContext: s.carryContext ?? false,
      })),
    })
    await loadData()
    ElMessage.success('工作流配置已保存')
  } catch (e) {
    ElMessage.error('保存失败: ' + String(e))
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

// ========== Workflow Control ==========
async function doStartWorkflow() {
  try {
    await invoke('set_workflow_steps', {
      todoId,
      steps: workflowSteps.value.map(s => ({
        stepType: s.stepType,
        subtaskId: s.subtaskId ?? null,
        promptText: s.promptText ?? null,
        carryContext: s.carryContext ?? false,
      })),
    })
    await invoke('start_workflow', { todoId })
    ElMessage.success('工作流已启动')
    await loadData()
  } catch (e) {
    ElMessage.error('启动失败: ' + String(e))
  }
}

async function doStopWorkflow() {
  try {
    await invoke('stop_workflow', { todoId })
    ElMessage.info('工作流已停止')
    await loadData()
  } catch (e) {
    ElMessage.error('停止失败: ' + String(e))
  }
}

async function doContinueWorkflow() {
  try {
    await invoke('continue_workflow', { todoId })
    ElMessage.success('工作流已继续')
    await loadData()
  } catch (e) {
    ElMessage.error('继续失败: ' + String(e))
  }
}

async function doResetWorkflow() {
  try {
    await invoke('reset_workflow', { todoId })
    ElMessage.success('工作流已重置')
    await loadData()
  } catch (e) {
    ElMessage.error('重置失败: ' + String(e))
  }
}

const workflowCompletedCount = computed(() =>
  workflowProgress.value.filter(s => s.status === 'completed' || s.status === 'skipped').length
)

const canContinueWorkflow = computed(() =>
  !workflowEnabled.value &&
  workflowCurrentStep.value >= 0 &&
  workflowCurrentStep.value < workflowProgress.value.length
)

function getStepStatus(idx: number): string | null {
  const progress = workflowProgress.value[idx]
  return progress ? progress.status : null
}

function getStepStatusInfo(status: string) {
  return STEP_STATUS_MAP[status as WorkflowStepStatus] || { label: status, type: 'info' }
}

function canViewLog(idx: number): boolean {
  const status = getStepStatus(idx)
  return !!status && status !== 'pending'
}

function isStepLocked(idx: number): boolean {
  void idx
  return workflowEnabled.value || canContinueWorkflow.value
}

function viewStepLog(idx: number) {
  const step = workflowSteps.value[idx]
  const progress = workflowProgress.value[idx]
  if (!progress) return

  if (step.stepType === 'subtask' && step.subtaskId) {
    openLogWindow({
      subtaskId: step.subtaskId,
      title: `步骤 ${idx + 1} 执行日志`,
    })
  } else if (step.stepType === 'prompt') {
    openLogWindow({
      todoId,
      stepOrder: idx,
      title: `步骤 ${idx + 1} 执行日志`,
    })
  }
}

// ========== Prompt CRUD ==========
function startCreatePrompt() {
  isCreatingPrompt.value = true
  editingPrompt.value = null
  promptForm.value = { name: '', category: '', description: '', templateContent: '' }
}

function startEditPrompt(tpl: PromptTemplate) {
  isCreatingPrompt.value = false
  editingPrompt.value = tpl
  promptForm.value = {
    name: tpl.name,
    category: tpl.category,
    description: tpl.description,
    templateContent: tpl.templateContent,
  }
}

function cancelPromptEdit() {
  editingPrompt.value = null
  isCreatingPrompt.value = false
}

async function savePrompt() {
  if (!promptForm.value.name.trim()) {
    ElMessage.warning('请输入提示词名称')
    return
  }
  if (!promptForm.value.templateContent.trim()) {
    ElMessage.warning('请输入提示词内容')
    return
  }

  try {
    if (isCreatingPrompt.value) {
      await schedulerStore.createTemplate({
        name: promptForm.value.name,
        category: promptForm.value.category,
        description: promptForm.value.description,
        templateContent: promptForm.value.templateContent,
      })
      ElMessage.success('提示词已创建')
    } else if (editingPrompt.value) {
      await schedulerStore.updateTemplate(editingPrompt.value.id!, {
        name: promptForm.value.name,
        category: promptForm.value.category,
        description: promptForm.value.description,
        templateContent: promptForm.value.templateContent,
      })
      ElMessage.success('提示词已更新')
    }
    await loadPromptLibrary()
    cancelPromptEdit()
  } catch (e) {
    ElMessage.error('保存失败: ' + String(e))
  }
}

async function deletePrompt(tpl: PromptTemplate) {
  try {
    await ElMessageBox.confirm(`确定删除提示词"${tpl.name}"吗？`, '删除确认', {
      confirmButtonText: '删除',
      cancelButtonText: '取消',
      type: 'warning',
    })
    await schedulerStore.deleteTemplate(tpl.id)
    await loadPromptLibrary()
    ElMessage.success('已删除')
  } catch { /* cancelled */ }
}

function quickInsertPrompt(tpl: PromptTemplate) {
  if (workflowEnabled.value || canContinueWorkflow.value) return
  workflowSteps.value.push({
    stepType: 'prompt',
    promptText: tpl.templateContent,
  })
  activeSection.value = 'workflow'
  ElMessage.success(`已添加提示词"${tpl.name}"到工作流`)
}
</script>

<template>
  <div class="workflow-window">
    <!-- 窗口标题栏 -->
    <div class="window-header">
      <div class="header-left">
        <h2>工作流配置</h2>
        <span v-if="todoTitle" class="todo-title-tag">{{ todoTitle }}</span>
      </div>
      <div class="window-controls">
        <button class="control-btn maximize-btn" title="最大化" @click="handleMaximize">
          <el-icon :size="14"><FullScreen /></el-icon>
        </button>
        <button class="control-btn close-btn" title="关闭" @click="handleClose">
          <el-icon :size="14"><Close /></el-icon>
        </button>
      </div>
    </div>

    <!-- 主内容 -->
    <div class="workflow-body" v-loading="loading">
      <!-- Tab 切换 -->
      <div class="section-tabs">
        <button
          class="section-tab"
          :class="{ active: activeSection === 'workflow' }"
          @click="activeSection = 'workflow'"
        >
          <el-icon><Setting /></el-icon>
          工作流步骤
          <span v-if="workflowSteps.length > 0" class="tab-badge">{{ workflowSteps.length }}</span>
        </button>
        <button
          class="section-tab"
          :class="{ active: activeSection === 'prompts' }"
          @click="activeSection = 'prompts'"
        >
          <el-icon><Collection /></el-icon>
          提示词库
          <span v-if="promptLibrary.length > 0" class="tab-badge">{{ promptLibrary.length }}</span>
        </button>
      </div>

      <!-- 工作流步骤区域 -->
      <div v-show="activeSection === 'workflow'" class="section-content">
        <!-- 工作流进度 -->
        <div v-if="workflowProgress.length > 0" class="workflow-progress-card">
          <div class="progress-header">
            <span class="progress-title">执行进度 {{ workflowCompletedCount }}/{{ workflowProgress.length }}</span>
            <div class="progress-controls">
              <el-button
                v-if="!workflowEnabled"
                size="small"
                type="primary"
                :disabled="workflowProgress.length === 0"
                @click="canContinueWorkflow ? doContinueWorkflow() : doStartWorkflow()"
              >
                {{ canContinueWorkflow ? '继续' : '启动' }}
              </el-button>
              <el-button v-else size="small" @click="doStopWorkflow">停止</el-button>
              <el-button size="small" type="danger" plain @click="doResetWorkflow">重置</el-button>
            </div>
          </div>
          <el-progress
            :percentage="workflowProgress.length > 0 ? Math.round(workflowCompletedCount / workflowProgress.length * 100) : 0"
            :stroke-width="6"
            style="margin: 6px 0 0"
          />
        </div>

        <!-- 步骤编辑列表 -->
        <div class="steps-header">
          <span class="steps-title">步骤列表</span>
          <div class="steps-actions">
            <el-button size="small" type="danger" plain @click="handleClearWorkflow" :disabled="workflowSteps.length === 0 || workflowEnabled || canContinueWorkflow">
              <el-icon><Delete /></el-icon>
              清空
            </el-button>
            <el-button size="small" type="primary" @click="addWorkflowStep" :disabled="workflowEnabled || canContinueWorkflow">
              <el-icon><Plus /></el-icon>
              添加步骤
            </el-button>
          </div>
        </div>

        <div v-if="workflowSteps.length === 0" class="empty-steps">
          <el-icon class="empty-icon"><List /></el-icon>
          <p>暂无步骤，点击"添加步骤"开始配置工作流</p>
          <p class="empty-hint">也可以切换到"提示词库"快速添加提示词步骤</p>
        </div>

        <div v-else class="steps-list">
          <div v-for="(step, idx) in workflowSteps" :key="idx" class="step-card">
            <div class="step-number-area">
              <span class="step-number">{{ idx + 1 }}</span>
              <el-tag
                v-if="getStepStatus(idx) && getStepStatus(idx) !== 'pending'"
                :type="getStepStatusInfo(getStepStatus(idx)!).type"
                size="small"
                effect="light"
                class="step-status-tag"
              >
                {{ getStepStatusInfo(getStepStatus(idx)!).label }}
              </el-tag>
            </div>
            <div class="step-content">
              <div class="step-top-row">
                <el-select
                  v-model="step.stepType"
                  style="width: 120px"
                  size="small"
                  :disabled="isStepLocked(idx)"
                  @change="step.subtaskId = undefined; step.promptText = undefined"
                >
                  <el-option label="执行子任务" value="subtask" />
                  <el-option label="执行提示词" value="prompt" />
                </el-select>
                <el-select
                  v-if="step.stepType === 'subtask'"
                  v-model="step.subtaskId"
                  style="flex: 1; min-width: 0"
                  size="small"
                  placeholder="选择子任务"
                  clearable
                  :disabled="isStepLocked(idx)"
                  popper-class="subtask-select-popper"
                  @change="(val: number) => onSubtaskSelected(idx, val)"
                >
                  <el-option
                    v-for="st in availableSubtasksForStep(idx)"
                    :key="st.id"
                    :label="st.title"
                    :value="st.id"
                    class="subtask-option-item"
                  >
                    <span class="subtask-option-text">{{ st.title }}</span>
                  </el-option>
                </el-select>
                <el-select
                  v-else
                  :model-value="undefined"
                  style="flex: 1; min-width: 0"
                  size="small"
                  placeholder="从提示词库选择..."
                  clearable
                  :disabled="isStepLocked(idx)"
                  @change="(val: string) => { const t = promptLibrary.find(p => p.id === val); if (t) selectPromptForStep(idx, t) }"
                >
                  <el-option
                    v-for="tpl in promptLibrary"
                    :key="tpl.id"
                    :label="tpl.name"
                    :value="tpl.id"
                  />
                </el-select>
                <div class="step-actions-inline">
                  <button
                    v-if="canViewLog(idx)"
                    class="step-action-btn log"
                    @click="viewStepLog(idx)"
                    title="查看日志"
                  >
                    <el-icon :size="14"><Document /></el-icon>
                  </button>
                  <button class="step-action-btn" :disabled="idx === 0 || isStepLocked(idx)" @click="moveWorkflowStep(idx, 'up')" title="上移">
                    <el-icon :size="14"><Top /></el-icon>
                  </button>
                  <button class="step-action-btn" :disabled="idx === workflowSteps.length - 1 || isStepLocked(idx)" @click="moveWorkflowStep(idx, 'down')" title="下移">
                    <el-icon :size="14"><Bottom /></el-icon>
                  </button>
                  <button class="step-action-btn delete" :disabled="isStepLocked(idx)" @click="removeWorkflowStep(idx)" title="删除">
                    <el-icon :size="14"><Delete /></el-icon>
                  </button>
                </div>
              </div>
              <div v-if="idx > 0" class="step-carry-context-row">
                <el-switch
                  v-model="step.carryContext"
                  size="small"
                  :disabled="isStepLocked(idx)"
                  active-text="带入上一步结果"
                  inactive-text=""
                />
              </div>
              <div v-if="step.stepType === 'prompt'" class="step-prompt-row">
                <el-input
                  v-model="step.promptText"
                  type="textarea"
                  :autosize="{ minRows: 1, maxRows: 4 }"
                  placeholder="输入提示词内容..."
                  resize="vertical"
                  :disabled="isStepLocked(idx)"
                />
              </div>
              <div v-if="step.stepType === 'subtask' && step.subtaskId" class="step-prompt-row">
                <div class="step-prompt-label">
                  <span>执行指令</span>
                  <el-tag size="small" type="info" effect="plain">基于子任务标题+内容生成</el-tag>
                </div>
                <el-input
                  v-model="step.promptText"
                  type="textarea"
                  :autosize="{ minRows: 1, maxRows: 4 }"
                  placeholder="执行指令（自动生成）"
                  resize="vertical"
                  :disabled="isStepLocked(idx)"
                />
              </div>
            </div>
          </div>
        </div>
      </div>

      <!-- 提示词库区域 -->
      <div v-show="activeSection === 'prompts'" class="section-content">
        <!-- 提示词编辑表单 -->
        <div v-if="isCreatingPrompt || editingPrompt" class="prompt-edit-form">
          <div class="prompt-form-header">
            <span>{{ isCreatingPrompt ? '新建提示词' : '编辑提示词' }}</span>
            <el-button size="small" text @click="cancelPromptEdit">
              <el-icon><Close /></el-icon>
            </el-button>
          </div>
          <el-form label-position="top" size="default">
            <div class="prompt-form-row">
              <el-form-item label="名称" style="flex: 1" required>
                <el-input v-model="promptForm.name" placeholder="提示词名称" />
              </el-form-item>
              <el-form-item label="分类" style="width: 180px">
                <el-select
                  v-model="promptForm.category"
                  filterable
                  allow-create
                  placeholder="选择或新建"
                  clearable
                >
                  <el-option v-for="cat in promptCategories" :key="cat" :label="cat" :value="cat" />
                </el-select>
              </el-form-item>
            </div>
            <el-form-item label="说明">
              <el-input v-model="promptForm.description" placeholder="简短描述（可选）" />
            </el-form-item>
            <el-form-item label="内容" required>
              <el-input
                v-model="promptForm.templateContent"
                type="textarea"
                :rows="6"
                placeholder="提示词内容..."
              />
            </el-form-item>
          </el-form>
          <div class="prompt-form-footer">
            <el-button @click="cancelPromptEdit">取消</el-button>
            <el-button type="primary" @click="savePrompt">
              <el-icon><Check /></el-icon>
              保存
            </el-button>
          </div>
        </div>

        <!-- 提示词列表 -->
        <template v-else>
          <div class="prompts-toolbar">
            <span class="prompts-count">共 {{ promptLibrary.length }} 条提示词</span>
            <el-button size="small" type="primary" @click="startCreatePrompt">
              <el-icon><Plus /></el-icon>
              新建提示词
            </el-button>
          </div>

          <div v-if="promptLibrary.length === 0" class="empty-steps">
            <el-icon class="empty-icon"><Collection /></el-icon>
            <p>暂无提示词，点击"新建提示词"创建</p>
          </div>

          <div v-else class="prompt-list">
            <div v-for="tpl in promptLibrary" :key="tpl.id" class="prompt-card">
              <div class="prompt-card-header">
                <div class="prompt-card-title">
                  <span class="prompt-name">{{ tpl.name }}</span>
                  <el-tag v-if="tpl.category" size="small" type="info" effect="light">{{ tpl.category }}</el-tag>
                </div>
                <div class="prompt-card-actions">
                  <el-button
                    size="small"
                    type="primary"
                    plain
                    :disabled="workflowEnabled || canContinueWorkflow"
                    @click="quickInsertPrompt(tpl)"
                    title="添加到工作流"
                  >
                    <el-icon><Plus /></el-icon>
                    使用
                  </el-button>
                  <el-button size="small" text @click="startEditPrompt(tpl)">
                    <el-icon><Edit /></el-icon>
                  </el-button>
                  <el-button size="small" text type="danger" @click="deletePrompt(tpl)">
                    <el-icon><Delete /></el-icon>
                  </el-button>
                </div>
              </div>
              <div v-if="tpl.description" class="prompt-desc">{{ tpl.description }}</div>
              <div class="prompt-preview">{{ tpl.templateContent }}</div>
            </div>
          </div>
        </template>
      </div>
    </div>

    <!-- 底部操作栏 -->
    <div class="window-footer">
      <div class="footer-left">
        <span class="step-count">{{ workflowSteps.length }} 个步骤</span>
      </div>
      <div class="footer-right">
        <el-button size="small" @click="handleClose">
          <el-icon><Close /></el-icon>
          取消
        </el-button>
        <el-button type="primary" size="small" :disabled="workflowEnabled || canContinueWorkflow" @click="handleSave">
          <el-icon><Check /></el-icon>
          保存工作流
        </el-button>
      </div>
    </div>

  </div>
</template>

<style scoped>
.workflow-window {
  display: flex;
  flex-direction: column;
  height: 100vh;
  background: #ffffff;
  border: 1px solid #e2e8f0;
  box-sizing: border-box;
}

/* ========== Header ========== */
.window-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 12px 16px;
  min-height: 44px;
  box-sizing: border-box;
  border-bottom: 1px solid var(--border, #e2e8f0);
  -webkit-app-region: drag;
}

.header-left {
  display: flex;
  align-items: center;
  gap: 10px;

  h2 {
    margin: 0;
    font-size: 16px;
    font-weight: 600;
    line-height: 1.2;
    color: #1e293b;
  }
}

.todo-title-tag {
  font-size: 12px;
  color: #64748b;
  background: #f1f5f9;
  padding: 2px 8px;
  border-radius: 4px;
  max-width: 300px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
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

/* ========== Body ========== */
.workflow-body {
  flex: 1;
  display: flex;
  flex-direction: column;
  padding: 14px 24px;
  overflow: hidden;
  min-height: 0;
  background: #fafbfc;
}

.section-tabs {
  display: flex;
  gap: 0;
  border-bottom: 1px solid #e2e8f0;
  margin-bottom: 16px;
  flex-shrink: 0;
}

.section-tab {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 10px 20px;
  background: none;
  border: none;
  border-bottom: 2px solid transparent;
  cursor: pointer;
  font-size: 14px;
  color: #64748b;
  transition: all 0.2s;

  &:hover {
    color: #3b82f6;
    background: #f8fafc;
  }

  &.active {
    color: #3b82f6;
    border-bottom-color: #3b82f6;
    font-weight: 500;
  }
}

.tab-badge {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  min-width: 18px;
  height: 18px;
  padding: 0 5px;
  font-size: 11px;
  font-weight: 600;
  background: #e2e8f0;
  color: #64748b;
  border-radius: 9px;
}

.section-tab.active .tab-badge {
  background: #dbeafe;
  color: #3b82f6;
}

.section-content {
  flex: 1;
  overflow-y: auto;
  min-height: 0;
  padding-right: 6px;
  margin-right: -6px;

  &::-webkit-scrollbar {
    width: 5px;
  }

  &::-webkit-scrollbar-track {
    background: transparent;
  }

  &::-webkit-scrollbar-thumb {
    background: #cbd5e1;
    border-radius: 3px;
  }
}

/* ========== Workflow Progress ========== */
.workflow-progress-card {
  padding: 14px 16px;
  background: linear-gradient(135deg, #f0f9ff 0%, #eff6ff 100%);
  border-radius: 10px;
  border: 1px solid rgba(59, 130, 246, 0.12);
  margin-bottom: 16px;
  overflow: hidden;
}

.progress-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
}

.progress-title {
  font-size: 14px;
  font-weight: 500;
  color: #334155;
}

.progress-controls {
  display: flex;
  gap: 6px;
}


/* ========== Steps Editor ========== */
.steps-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-bottom: 14px;
}

.steps-title {
  font-size: 15px;
  font-weight: 600;
  color: #1e293b;
}

.steps-actions {
  display: flex;
  gap: 8px;
}

.empty-steps {
  text-align: center;
  padding: 40px 24px;
  color: #94a3b8;
  background: #ffffff;
  border: 1px dashed #e2e8f0;
  border-radius: 10px;

  .empty-icon {
    font-size: 36px;
    margin-bottom: 12px;
    opacity: 0.35;
  }

  p {
    margin: 0;
    font-size: 13px;
    line-height: 1.7;
  }

  .empty-hint {
    font-size: 12px;
    color: #cbd5e1;
    margin-top: 4px;
  }
}

.steps-list {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.step-card {
  display: flex;
  align-items: flex-start;
  gap: 10px;
  padding: 10px 12px;
  background: #ffffff;
  border: 1px solid #e2e8f0;
  border-radius: 8px;
  transition: all 0.2s;

  &:hover {
    border-color: #93c5fd;
    box-shadow: 0 1px 3px rgba(59, 130, 246, 0.08);
  }
}

.step-number-area {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 4px;
  flex-shrink: 0;
  margin-top: 4px;
}

.step-number {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 24px;
  height: 24px;
  font-size: 12px;
  font-weight: 700;
  color: #3b82f6;
  background: #eff6ff;
  border-radius: 6px;
}

.step-status-tag {
  font-size: 10px;
  transform: scale(0.85);
  white-space: nowrap;
}

.step-content {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.step-top-row {
  display: flex;
  align-items: center;
  gap: 8px;

  :deep(.el-select .el-input .el-input__inner) {
    text-overflow: ellipsis;
    overflow: hidden;
    white-space: nowrap;
  }
}

.step-carry-context-row {
  display: flex;
  align-items: center;
  padding: 2px 0;
}

.step-prompt-row {
  padding-left: 0;
}

.step-prompt-label {
  display: flex;
  align-items: center;
  gap: 6px;
  margin-bottom: 4px;
  font-size: 12px;
  color: #64748b;
}

.step-actions-inline {
  display: flex;
  align-items: center;
  gap: 2px;
  flex-shrink: 0;
}

.step-action-btn {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 26px;
  height: 26px;
  border: none;
  background: transparent;
  cursor: pointer;
  border-radius: 4px;
  color: #94a3b8;
  transition: all 0.15s;
  padding: 0;

  &:hover:not(:disabled) {
    background: #f1f5f9;
    color: #475569;
  }

  &:disabled {
    opacity: 0.3;
    cursor: not-allowed;
  }

  &.delete:hover:not(:disabled) {
    background: #fef2f2;
    color: #ef4444;
  }

  &.log {
    color: #3b82f6;
  }

  &.log:hover:not(:disabled) {
    background: #eff6ff;
    color: #2563eb;
  }
}

/* ========== Prompt Library ========== */
.prompts-toolbar {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-bottom: 14px;
}

.prompts-count {
  font-size: 13px;
  color: #64748b;
}

.prompt-list {
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.prompt-card {
  padding: 12px 14px;
  background: #ffffff;
  border: 1px solid #e2e8f0;
  border-radius: 8px;
  transition: all 0.2s;

  &:hover {
    border-color: #93c5fd;
    box-shadow: 0 1px 4px rgba(59, 130, 246, 0.08);
  }
}

.prompt-card-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-bottom: 6px;
}

.prompt-card-title {
  display: flex;
  align-items: center;
  gap: 8px;
}

.prompt-name {
  font-size: 14px;
  font-weight: 600;
  color: #1e293b;
}

.prompt-card-actions {
  display: flex;
  align-items: center;
  gap: 4px;
}

.prompt-desc {
  font-size: 12px;
  color: #94a3b8;
  margin-bottom: 8px;
}

.prompt-preview {
  font-size: 12px;
  color: #64748b;
  background: #f8fafc;
  border: 1px solid #f1f5f9;
  border-radius: 6px;
  padding: 8px 10px;
  max-height: 64px;
  overflow: hidden;
  white-space: pre-wrap;
  word-break: break-word;
  line-height: 1.5;
}

/* ========== Prompt Edit Form ========== */
.prompt-edit-form {
  border: 1px solid #dbeafe;
  border-radius: 8px;
  padding: 16px 18px;
  background: #ffffff;
  box-shadow: 0 1px 3px rgba(59, 130, 246, 0.06);
}

.prompt-form-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-bottom: 14px;
  font-size: 16px;
  font-weight: 600;
  color: #1e293b;
}

.prompt-form-row {
  display: flex;
  gap: 14px;
}

.prompt-form-footer {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
  margin-top: 8px;
}

/* ========== Footer ========== */
.window-footer {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: 10px 20px;
  border-top: 1px solid #e2e8f0;
  flex-shrink: 0;
  background: #ffffff;
}

.footer-left {
  display: flex;
  align-items: center;
}

.step-count {
  font-size: 12px;
  color: #94a3b8;
  font-weight: 500;
}

.footer-right {
  display: flex;
  gap: 8px;
}

</style>

<style>
.subtask-select-popper .el-select-dropdown__item {
  height: auto !important;
  padding: 8px 12px !important;
  line-height: 1.5 !important;
  white-space: normal !important;
  word-break: break-all !important;
  border-bottom: 1px solid #f0f0f0;
}

.subtask-select-popper .el-select-dropdown__item:last-child {
  border-bottom: none;
}

.subtask-select-popper .subtask-option-text {
  display: block;
  white-space: normal;
  word-break: break-all;
  line-height: 1.5;
}

.subtask-select-popper .el-select-dropdown__list {
  max-width: 480px;
}
</style>
