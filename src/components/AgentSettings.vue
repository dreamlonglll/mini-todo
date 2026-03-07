<script setup lang="ts">
import { ref, onMounted, computed } from 'vue'
import { ElMessage, ElMessageBox } from 'element-plus'
import { Plus, Delete, Edit, Connection } from '@element-plus/icons-vue'
import AgentStatusBadge from './AgentStatusBadge.vue'
import { useAgentStore } from '@/stores/agentStore'
import type {
  AgentConfig,
  AgentType,
  CreateAgentRequest,
  UpdateAgentRequest,
  AgentCapabilities,
  SandboxConfig,
  AgentHealthState,
} from '@/types/agent'
import {
  AGENT_TYPE_INFO,
  DEFAULT_CAPABILITIES,
  DEFAULT_SANDBOX_CONFIG,
} from '@/types/agent'

const agentStore = useAgentStore()

const dialogVisible = ref(false)
const dialogMode = ref<'add' | 'edit'>('add')
const editingId = ref<number | null>(null)
const checkingHealthId = ref<number | null>(null)

interface EnvVar {
  key: string
  value: string
}

const form = ref({
  name: '',
  agentType: 'claude_code' as AgentType,
  cliPath: '',
  apiKey: '',
  defaultModel: '',
  maxConcurrent: 1,
  timeoutSeconds: 300,
  capabilities: { codeGen: 5, codeFix: 5, testReview: 5, speed: 5, costEfficiency: 5 } as AgentCapabilities,
  sandbox: { ...DEFAULT_SANDBOX_CONFIG } as SandboxConfig,
  envVars: [] as EnvVar[],
  enabled: true,
})

const showAdvanced = ref(false)

const agentTypeOptions = computed(() =>
  Object.entries(AGENT_TYPE_INFO).map(([value, info]) => ({
    value,
    label: info.label,
    description: info.description,
  }))
)

function getHealthState(agent: AgentConfig): AgentHealthState {
  if (!agent.enabled) return 'disabled'
  const status = agentStore.getHealthStatus(agent.id)
  return (status?.status as AgentHealthState) || 'unavailable'
}

function envVarsToArray(jsonStr: string): EnvVar[] {
  try {
    const obj = JSON.parse(jsonStr)
    return Object.entries(obj).map(([key, value]) => ({ key, value: String(value) }))
  } catch {
    return []
  }
}

function envVarsToJson(vars: EnvVar[]): string {
  const obj: Record<string, string> = {}
  for (const v of vars) {
    if (v.key.trim()) obj[v.key.trim()] = v.value
  }
  return JSON.stringify(obj)
}

function addEnvVar() {
  form.value.envVars.push({ key: '', value: '' })
}

function removeEnvVar(index: number) {
  form.value.envVars.splice(index, 1)
}

function resetForm() {
  form.value = {
    name: '',
    agentType: 'claude_code',
    cliPath: '',
    apiKey: '',
    defaultModel: '',
    maxConcurrent: 1,
    timeoutSeconds: 300,
    capabilities: { ...DEFAULT_CAPABILITIES['claude_code'] },
    sandbox: { ...DEFAULT_SANDBOX_CONFIG },
    envVars: [],
    enabled: true,
  }
  showAdvanced.value = false
}

function openAddDialog() {
  dialogMode.value = 'add'
  editingId.value = null
  resetForm()
  dialogVisible.value = true
}

function openEditDialog(agent: AgentConfig) {
  dialogMode.value = 'edit'
  editingId.value = agent.id

  let caps: AgentCapabilities
  try {
    caps = JSON.parse(agent.capabilities)
  } catch {
    caps = { ...DEFAULT_CAPABILITIES[agent.agentType] || DEFAULT_CAPABILITIES['custom'] }
  }

  let sandbox: SandboxConfig
  try {
    sandbox = { ...DEFAULT_SANDBOX_CONFIG, ...JSON.parse(agent.sandboxConfig) }
  } catch {
    sandbox = { ...DEFAULT_SANDBOX_CONFIG }
  }

  form.value = {
    name: agent.name,
    agentType: agent.agentType,
    cliPath: agent.cliPath,
    apiKey: '',
    defaultModel: agent.defaultModel,
    maxConcurrent: agent.maxConcurrent,
    timeoutSeconds: agent.timeoutSeconds,
    capabilities: caps,
    sandbox,
    envVars: envVarsToArray(agent.envVars),
    enabled: agent.enabled,
  }
  showAdvanced.value = false
  dialogVisible.value = true
}

function onAgentTypeChange(type: AgentType) {
  const defaults = DEFAULT_CAPABILITIES[type] || DEFAULT_CAPABILITIES['custom']
  form.value.capabilities = { ...defaults }

  if (type === 'claude_code') {
    form.value.cliPath = form.value.cliPath || 'claude'
  } else if (type === 'codex') {
    form.value.cliPath = form.value.cliPath || 'codex'
  }
}

async function handleSubmit() {
  if (!form.value.name.trim()) {
    ElMessage.warning('请输入 Agent 名称')
    return
  }
  if (!form.value.cliPath.trim()) {
    ElMessage.warning('请输入 CLI 路径')
    return
  }

  try {
    if (dialogMode.value === 'add') {
      const request: CreateAgentRequest = {
        name: form.value.name,
        agentType: form.value.agentType,
        cliPath: form.value.cliPath,
        apiKey: form.value.apiKey || undefined,
        defaultModel: form.value.defaultModel || undefined,
        maxConcurrent: form.value.maxConcurrent,
        timeoutSeconds: form.value.timeoutSeconds,
        capabilities: JSON.stringify(form.value.capabilities),
        envVars: envVarsToJson(form.value.envVars),
        sandboxConfig: JSON.stringify(form.value.sandbox),
      }
      await agentStore.addAgent(request)
      ElMessage.success('Agent 已添加')
    } else if (editingId.value !== null) {
      const request: UpdateAgentRequest = {
        name: form.value.name,
        agentType: form.value.agentType,
        cliPath: form.value.cliPath,
        apiKey: form.value.apiKey || undefined,
        defaultModel: form.value.defaultModel || undefined,
        maxConcurrent: form.value.maxConcurrent,
        timeoutSeconds: form.value.timeoutSeconds,
        capabilities: JSON.stringify(form.value.capabilities),
        envVars: envVarsToJson(form.value.envVars),
        sandboxConfig: JSON.stringify(form.value.sandbox),
        enabled: form.value.enabled,
      }
      await agentStore.editAgent(editingId.value, request)
      ElMessage.success('Agent 已更新')
    }
    dialogVisible.value = false
  } catch (e) {
    ElMessage.error('操作失败: ' + String(e))
  }
}

async function handleDelete(agent: AgentConfig) {
  try {
    await ElMessageBox.confirm(
      `确定删除 Agent「${agent.name}」吗？`,
      '删除确认',
      { confirmButtonText: '删除', cancelButtonText: '取消', type: 'warning' }
    )
    await agentStore.removeAgent(agent.id)
    ElMessage.success('已删除')
  } catch {
    // cancelled
  }
}

async function handleToggleEnabled(agent: AgentConfig) {
  try {
    await agentStore.editAgent(agent.id, { enabled: !agent.enabled })
  } catch (e) {
    ElMessage.error('切换失败: ' + String(e))
  }
}

async function handleCheckHealth(agent: AgentConfig) {
  checkingHealthId.value = agent.id
  try {
    const result = await agentStore.checkHealth(agent.id)
    ElMessage.info(result.message || '检测完成')
  } catch (e) {
    ElMessage.error('检测失败: ' + String(e))
  } finally {
    checkingHealthId.value = null
  }
}

onMounted(() => {
  agentStore.loadAgents()
})
</script>

<template>
  <div class="agent-settings">
    <!-- Agent 列表 -->
    <div class="agent-list">
      <div
        v-for="agent in agentStore.agents"
        :key="agent.id"
        class="agent-card"
      >
        <div class="agent-header">
          <div class="agent-name">
            <span>{{ agent.name }}</span>
            <el-tag size="small" type="info" class="agent-type-tag">
              {{ AGENT_TYPE_INFO[agent.agentType]?.label || agent.agentType }}
            </el-tag>
          </div>
          <AgentStatusBadge :status="getHealthState(agent)" />
        </div>

        <div class="agent-info">
          <span v-if="agent.defaultModel" class="info-item">
            {{ agent.defaultModel }}
          </span>
          <span class="info-item">
            {{ agent.cliPath }}
          </span>
        </div>

        <div class="agent-actions">
          <el-switch
            :model-value="agent.enabled"
            size="small"
            @change="handleToggleEnabled(agent)"
          />
          <el-button
            size="small"
            :icon="Connection"
            :loading="checkingHealthId === agent.id"
            @click="handleCheckHealth(agent)"
          >
            检测
          </el-button>
          <el-button
            size="small"
            :icon="Edit"
            @click="openEditDialog(agent)"
          >
            编辑
          </el-button>
          <el-button
            size="small"
            type="danger"
            :icon="Delete"
            @click="handleDelete(agent)"
          />
        </div>
      </div>

      <div v-if="agentStore.agents.length === 0" class="empty-state">
        <p>尚未配置 Agent</p>
        <p class="empty-hint">添加 Claude Code 或 Codex 来执行代码任务</p>
      </div>
    </div>

    <el-button
      type="primary"
      :icon="Plus"
      style="width: 100%; margin-top: 12px"
      @click="openAddDialog"
    >
      添加 Agent
    </el-button>

    <!-- 添加/编辑弹窗 -->
    <el-dialog
      v-model="dialogVisible"
      :title="dialogMode === 'add' ? '添加 Agent' : '编辑 Agent'"
      width="460px"
      :close-on-click-modal="false"
      append-to-body
    >
      <el-form label-position="top" size="default">
        <el-form-item label="名称" required>
          <el-input v-model="form.name" placeholder="例如 My Claude" />
        </el-form-item>

        <el-form-item label="类型" required>
          <el-select
            v-model="form.agentType"
            style="width: 100%"
            @change="onAgentTypeChange"
          >
            <el-option
              v-for="opt in agentTypeOptions"
              :key="opt.value"
              :label="opt.label"
              :value="opt.value"
            >
              <div>
                <span>{{ opt.label }}</span>
                <span style="color: var(--text-tertiary); font-size: 12px; margin-left: 8px">
                  {{ opt.description }}
                </span>
              </div>
            </el-option>
          </el-select>
        </el-form-item>

        <el-form-item label="CLI 路径" required>
          <el-input v-model="form.cliPath" placeholder="例如 claude 或完整路径" />
        </el-form-item>

        <el-form-item label="API Key">
          <el-input
            v-model="form.apiKey"
            type="password"
            show-password
            :placeholder="dialogMode === 'edit' ? '留空保持不变' : '输入 API Key'"
          />
        </el-form-item>

        <el-form-item label="默认模型">
          <el-input v-model="form.defaultModel" placeholder="留空使用默认" />
        </el-form-item>

        <!-- 高级设置 -->
        <el-collapse-transition>
          <div v-show="showAdvanced">
            <el-divider content-position="left">高级设置</el-divider>

            <el-form-item label="超时时间（秒）">
              <el-input-number
                v-model="form.timeoutSeconds"
                :min="30"
                :max="3600"
                :step="30"
                style="width: 100%"
              />
            </el-form-item>

            <el-form-item label="最大并发数">
              <el-input-number
                v-model="form.maxConcurrent"
                :min="1"
                :max="5"
                style="width: 100%"
              />
            </el-form-item>

            <el-divider content-position="left">环境变量</el-divider>

            <div class="env-vars-section">
              <div
                v-for="(envVar, index) in form.envVars"
                :key="index"
                class="env-var-row"
              >
                <el-input
                  v-model="envVar.key"
                  placeholder="变量名"
                  size="small"
                  style="width: 40%"
                />
                <el-input
                  v-model="envVar.value"
                  placeholder="值"
                  size="small"
                  style="width: 48%"
                />
                <el-button
                  :icon="Delete"
                  size="small"
                  type="danger"
                  link
                  @click="removeEnvVar(index)"
                />
              </div>
              <el-button
                size="small"
                :icon="Plus"
                @click="addEnvVar"
              >
                添加变量
              </el-button>
              <p class="env-hint">
                用于配置 API 端点等，如 ANTHROPIC_BASE_URL、OPENAI_BASE_URL
              </p>
            </div>

            <el-divider content-position="left">能力画像</el-divider>

            <el-form-item label="代码生成">
              <el-slider v-model="form.capabilities.codeGen" :min="1" :max="10" show-stops />
            </el-form-item>
            <el-form-item label="代码修复">
              <el-slider v-model="form.capabilities.codeFix" :min="1" :max="10" show-stops />
            </el-form-item>
            <el-form-item label="测试与审查">
              <el-slider v-model="form.capabilities.testReview" :min="1" :max="10" show-stops />
            </el-form-item>
            <el-form-item label="速度">
              <el-slider v-model="form.capabilities.speed" :min="1" :max="10" show-stops />
            </el-form-item>
            <el-form-item label="成本效率">
              <el-slider v-model="form.capabilities.costEfficiency" :min="1" :max="10" show-stops />
            </el-form-item>

            <el-divider content-position="left">沙盒配置</el-divider>

            <el-form-item label="Worktree 隔离">
              <el-switch v-model="form.sandbox.enableWorktreeIsolation" />
            </el-form-item>
          </div>
        </el-collapse-transition>

        <el-button
          link
          type="primary"
          style="margin-top: 4px"
          @click="showAdvanced = !showAdvanced"
        >
          {{ showAdvanced ? '收起高级设置' : '展开高级设置' }}
        </el-button>
      </el-form>

      <template #footer>
        <el-button @click="dialogVisible = false">取消</el-button>
        <el-button type="primary" @click="handleSubmit">
          {{ dialogMode === 'add' ? '添加' : '保存' }}
        </el-button>
      </template>
    </el-dialog>
  </div>
</template>

<style scoped>
.agent-settings {
  padding: 0;
}

.agent-list {
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.agent-card {
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  padding: 12px;
  transition: border-color var(--transition-fast);
}

.agent-card:hover {
  border-color: var(--primary-light);
}

.agent-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-bottom: 8px;
}

.agent-name {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 14px;
  font-weight: 600;
  color: var(--text-primary);
}

.agent-type-tag {
  font-weight: 400;
}

.agent-info {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
  margin-bottom: 10px;
}

.info-item {
  font-size: 12px;
  color: var(--text-tertiary);
}

.agent-actions {
  display: flex;
  align-items: center;
  gap: 8px;
}

.empty-state {
  text-align: center;
  padding: 24px 0;
  color: var(--text-secondary);
  font-size: 14px;
}

.empty-hint {
  font-size: 12px;
  color: var(--text-tertiary);
  margin-top: 4px;
}

.env-vars-section {
  margin-bottom: 12px;
}

.env-var-row {
  display: flex;
  align-items: center;
  gap: 6px;
  margin-bottom: 6px;
}

.env-hint {
  font-size: 12px;
  color: var(--text-tertiary);
  margin-top: 6px;
}
</style>
