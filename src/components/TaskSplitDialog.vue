<script setup lang="ts">
import { ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { ElMessage } from 'element-plus'
import type { SplitTask, SplitResult } from '@/types/intelligence'
import { COMPLEXITY_MAP, AGENT_LABELS } from '@/types/intelligence'

const props = defineProps<{
  todoId: number
  projectPath: string
}>()

const emit = defineEmits<{
  (e: 'completed'): void
  (e: 'close'): void
}>()

const visible = ref(true)
const step = ref<'input' | 'loading' | 'result' | 'applying' | 'done'>('input')
const requirement = ref('')
const editableTasks = ref<SplitTask[]>([])
const summary = ref('')
const createdIds = ref<number[]>([])
const errorMessage = ref('')
const expandedRow = ref<number | null>(null)

function toggleExpand(index: number) {
  expandedRow.value = expandedRow.value === index ? null : index
}

async function handleSplit() {
  if (!requirement.value.trim()) {
    ElMessage.warning('请输入需求描述')
    return
  }

  step.value = 'loading'
  errorMessage.value = ''

  try {
    const result = await invoke<SplitResult>('split_task', {
      requirement: requirement.value,
      projectPath: props.projectPath,
    })

    editableTasks.value = result.tasks.map(t => ({ ...t }))
    summary.value = result.summary
    step.value = 'result'
  } catch (e) {
    errorMessage.value = String(e)
    step.value = 'input'
    ElMessage.error('拆分失败: ' + String(e))
  }
}

function removeTask(index: number) {
  editableTasks.value.splice(index, 1)
  for (const task of editableTasks.value) {
    task.dependencies = task.dependencies
      .filter(d => d !== index)
      .map(d => (d > index ? d - 1 : d))
  }
  if (expandedRow.value === index) {
    expandedRow.value = null
  } else if (expandedRow.value !== null && expandedRow.value > index) {
    expandedRow.value--
  }
}

function getDependencyOptions(currentIndex: number) {
  return editableTasks.value
    .map((t, i) => ({ label: `#${i + 1} ${t.title}`, value: i }))
    .filter(o => o.value < currentIndex)
}

function getDependencyLabel(deps: number[]) {
  if (deps.length === 0) return '-'
  return deps.map(d => `#${d + 1}`).join(', ')
}

async function handleApply() {
  if (editableTasks.value.length === 0) {
    ElMessage.warning('没有可创建的子任务')
    return
  }

  step.value = 'applying'

  try {
    const ids = await invoke<number[]>('apply_split_result', {
      todoId: props.todoId,
      tasks: editableTasks.value,
    })

    createdIds.value = ids
    step.value = 'done'
  } catch (e) {
    step.value = 'result'
    ElMessage.error('创建失败: ' + String(e))
  }
}

function handleDone() {
  emit('completed')
  handleClose()
}

function handleClose() {
  visible.value = false
  emit('close')
}

function handleReSplit() {
  editableTasks.value = []
  summary.value = ''
  step.value = 'input'
}
</script>

<template>
  <el-dialog
    v-model="visible"
    width="680px"
    top="5vh"
    append-to-body
    class="split-dialog"
    :close-on-click-modal="false"
    @close="handleClose"
  >
    <template #header>
      <span v-if="step === 'done'">智能拆分 - 完成</span>
      <span v-else-if="step === 'result'">智能拆分 - 拆分结果</span>
      <span v-else>智能拆分</span>
    </template>

    <div class="split-body" style="max-height: calc(85vh - 160px); overflow-y: auto; padding-right: 4px;">
      <!-- 步骤 1：输入 -->
      <template v-if="step === 'input'">
        <div class="step-input">
          <p class="step-label">请描述您想要实现的功能需求：</p>
          <el-input
            v-model="requirement"
            type="textarea"
            :rows="5"
            placeholder="例如：给项目添加用户认证系统，包含登录、注册、Token 管理功能"
            maxlength="2000"
            show-word-limit
          />
          <p class="step-tip">
            描述越详细，拆分结果越准确。AI 将根据项目上下文自动分析并拆分为可执行的子任务。
          </p>
          <div v-if="errorMessage" class="error-box">
            {{ errorMessage }}
          </div>
        </div>
      </template>

      <!-- 步骤 2：加载中 -->
      <template v-if="step === 'loading'">
        <div class="step-loading">
          <el-icon class="loading-icon is-loading" :size="40"><Loading /></el-icon>
          <p class="loading-text">AI 正在分析需求并拆分任务...</p>
          <p class="loading-tip">这可能需要 30-60 秒</p>
        </div>
      </template>

      <!-- 步骤 3：结果 -->
      <template v-if="step === 'result'">
        <div class="step-result">
          <div v-if="summary" class="result-summary">
            <strong>执行策略：</strong>{{ summary }}
          </div>

          <div class="task-list">
            <div
              v-for="(task, index) in editableTasks"
              :key="index"
              class="task-item"
            >
              <div class="task-row" @click="toggleExpand(index)">
                <span class="task-num">#{{ index + 1 }}</span>
                <span class="task-title">{{ task.title }}</span>
                <el-tag
                  :color="COMPLEXITY_MAP[task.complexity]?.color || '#999'"
                  effect="dark"
                  size="small"
                  style="border: none; color: #fff;"
                >
                  {{ COMPLEXITY_MAP[task.complexity]?.label || task.complexity }}
                </el-tag>
                <el-tag size="small" type="info">
                  {{ AGENT_LABELS[task.recommendedAgent] || task.recommendedAgent }}
                </el-tag>
                <span class="task-deps">{{ getDependencyLabel(task.dependencies) }}</span>
                <el-icon class="expand-icon">
                  <ArrowDown v-if="expandedRow !== index" />
                  <ArrowUp v-else />
                </el-icon>
              </div>

              <div v-if="expandedRow === index" class="task-detail">
                <el-form label-position="top" size="small">
                  <el-form-item label="标题">
                    <el-input v-model="task.title" />
                  </el-form-item>
                  <el-form-item label="描述">
                    <el-input v-model="task.description" type="textarea" :rows="3" />
                  </el-form-item>
                  <el-form-item label="Agent 指令">
                    <el-input v-model="task.prompt" type="textarea" :rows="3" />
                  </el-form-item>
                  <div class="detail-row">
                    <el-form-item label="复杂度">
                      <el-select v-model="task.complexity" style="width: 100px">
                        <el-option label="低" value="low" />
                        <el-option label="中" value="medium" />
                        <el-option label="高" value="high" />
                      </el-select>
                    </el-form-item>
                    <el-form-item label="推荐 Agent">
                      <el-select v-model="task.recommendedAgent" style="width: 140px">
                        <el-option label="Claude Code" value="claude_code" />
                        <el-option label="Codex" value="codex" />
                      </el-select>
                    </el-form-item>
                    <el-form-item label="依赖">
                      <el-select
                        v-model="task.dependencies"
                        multiple
                        style="width: 200px"
                        placeholder="无依赖"
                      >
                        <el-option
                          v-for="opt in getDependencyOptions(index)"
                          :key="opt.value"
                          :label="opt.label"
                          :value="opt.value"
                        />
                      </el-select>
                    </el-form-item>
                  </div>
                  <div class="detail-actions">
                    <el-button type="danger" size="small" text @click.stop="removeTask(index)">
                      <el-icon><Delete /></el-icon>
                      删除此任务
                    </el-button>
                  </div>
                </el-form>
              </div>
            </div>
          </div>

          <p class="result-tip">点击任务行可展开编辑详情</p>
        </div>
      </template>

      <!-- 步骤 4：应用中 -->
      <template v-if="step === 'applying'">
        <div class="step-loading">
          <el-icon class="loading-icon is-loading" :size="40"><Loading /></el-icon>
          <p class="loading-text">正在创建子任务...</p>
        </div>
      </template>

      <!-- 步骤 5：完成 -->
      <template v-if="step === 'done'">
        <div class="step-done">
          <el-icon :size="48" color="#10B981"><CircleCheck /></el-icon>
          <p class="done-text">已成功创建 {{ createdIds.length }} 个子任务</p>
          <ul class="done-list">
            <li v-for="(task, i) in editableTasks" :key="i">{{ task.title }}</li>
          </ul>
          <p class="done-tip">依赖关系已自动建立。</p>
        </div>
      </template>
    </div>

    <template #footer>
      <template v-if="step === 'input'">
        <el-button @click="handleClose">取消</el-button>
        <el-button type="primary" @click="handleSplit" :disabled="!requirement.trim()">
          <el-icon><MagicStick /></el-icon>
          开始拆分
        </el-button>
      </template>

      <template v-else-if="step === 'loading'">
        <el-button @click="handleClose">取消拆分</el-button>
      </template>

      <template v-else-if="step === 'result'">
        <div class="footer-spread">
          <el-button @click="handleReSplit">
            <el-icon><RefreshRight /></el-icon>
            重新拆分
          </el-button>
          <div>
            <el-button @click="handleClose">取消</el-button>
            <el-button type="primary" @click="handleApply">
              创建 {{ editableTasks.length }} 个子任务
            </el-button>
          </div>
        </div>
      </template>

      <template v-else-if="step === 'done'">
        <el-button type="primary" @click="handleDone">完成</el-button>
      </template>
    </template>
  </el-dialog>
</template>

<style scoped>
.step-input {
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.step-label {
  margin: 0;
  font-size: 14px;
  font-weight: 500;
  color: var(--text-primary);
}

.step-tip {
  margin: 0;
  font-size: 12px;
  color: var(--text-secondary, #999);
}

.error-box {
  background: #FEF2F2;
  border: 1px solid #FECACA;
  border-radius: 4px;
  padding: 8px 12px;
  font-size: 12px;
  color: #DC2626;
  white-space: pre-wrap;
  max-height: 120px;
  overflow-y: auto;
}

.step-loading {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  padding: 48px 0;
  gap: 16px;
}

.loading-icon {
  color: var(--el-color-primary);
}

.loading-text {
  margin: 0;
  font-size: 16px;
  color: var(--text-primary);
}

.loading-tip {
  margin: 0;
  font-size: 12px;
  color: var(--text-secondary, #999);
}

.step-result {
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.result-summary {
  background: #F0F9FF;
  border-radius: 6px;
  padding: 10px 14px;
  font-size: 13px;
  color: #1E40AF;
}

.task-list {
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.task-item {
  border: 1px solid var(--border, #E5E7EB);
  border-radius: 6px;
  overflow: hidden;
}

.task-row {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 10px 12px;
  cursor: pointer;
  transition: background 0.15s;
}

.task-row:hover {
  background: #F9FAFB;
}

.task-num {
  font-size: 12px;
  font-weight: 600;
  color: var(--el-color-primary);
  min-width: 24px;
}

.task-title {
  flex: 1;
  font-size: 13px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.task-deps {
  font-size: 11px;
  color: var(--text-secondary, #999);
  min-width: 40px;
  text-align: right;
}

.expand-icon {
  color: var(--text-secondary, #999);
  font-size: 12px;
}

.task-detail {
  border-top: 1px solid var(--border, #E5E7EB);
  padding: 12px;
  background: #FAFAFA;
}

.detail-row {
  display: flex;
  gap: 12px;
  flex-wrap: wrap;
}

.detail-actions {
  display: flex;
  justify-content: flex-end;
  margin-top: 4px;
}

.result-tip {
  margin: 0;
  font-size: 11px;
  color: var(--text-secondary, #999);
  text-align: center;
}

.step-done {
  display: flex;
  flex-direction: column;
  align-items: center;
  padding: 32px 0;
  gap: 12px;
}

.done-text {
  margin: 0;
  font-size: 16px;
  font-weight: 500;
  color: var(--text-primary);
}

.done-list {
  list-style: disc;
  padding-left: 24px;
  margin: 8px 0;
}

.done-list li {
  font-size: 13px;
  color: var(--text-primary);
  line-height: 1.8;
}

.done-tip {
  margin: 0;
  font-size: 12px;
  color: var(--text-secondary, #999);
}

.footer-spread {
  display: flex;
  justify-content: space-between;
  align-items: center;
  width: 100%;
}
</style>
