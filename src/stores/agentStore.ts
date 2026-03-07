import { defineStore } from 'pinia'
import { ref, computed } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import type {
  AgentConfig,
  CreateAgentRequest,
  UpdateAgentRequest,
  AgentHealthStatus,
} from '@/types/agent'

export const useAgentStore = defineStore('agent', () => {
  const agents = ref<AgentConfig[]>([])
  const healthStatuses = ref<Map<number, AgentHealthStatus>>(new Map())
  const loading = ref(false)

  const enabledAgents = computed(() => agents.value.filter(a => a.enabled))
  const agentCount = computed(() => agents.value.length)

  async function loadAgents() {
    loading.value = true
    try {
      agents.value = await invoke<AgentConfig[]>('get_agents')
    } catch (e) {
      console.error('加载 Agent 配置失败:', e)
    } finally {
      loading.value = false
    }
  }

  async function addAgent(request: CreateAgentRequest): Promise<number> {
    const id = await invoke<number>('create_agent', { request })
    await loadAgents()
    return id
  }

  async function editAgent(id: number, request: UpdateAgentRequest) {
    await invoke('update_agent', { id, request })
    await loadAgents()
  }

  async function removeAgent(id: number) {
    await invoke('delete_agent', { id })
    await loadAgents()
  }

  async function checkHealth(id: number): Promise<AgentHealthStatus> {
    const status = await invoke<AgentHealthStatus>('check_agent_health', { id })
    healthStatuses.value.set(id, status)
    return status
  }

  async function checkAllHealth() {
    try {
      const statuses = await invoke<AgentHealthStatus[]>('check_all_agents_health')
      healthStatuses.value.clear()
      for (const s of statuses) {
        healthStatuses.value.set(s.agentId, s)
      }
    } catch (e) {
      console.error('健康检查失败:', e)
    }
  }

  function getHealthStatus(id: number): AgentHealthStatus | undefined {
    return healthStatuses.value.get(id)
  }

  return {
    agents,
    healthStatuses,
    loading,
    enabledAgents,
    agentCount,
    loadAgents,
    addAgent,
    editAgent,
    removeAgent,
    checkHealth,
    checkAllHealth,
    getHealthStatus,
  }
})
