<script setup lang="ts">
import { computed } from 'vue'
import type { AgentHealthState } from '@/types/agent'

const props = defineProps<{
  status: AgentHealthState
}>()

const statusMap: Record<AgentHealthState, { text: string; color: string; type: string }> = {
  healthy: { text: '在线', color: '#10B981', type: 'success' },
  outdated: { text: '版本过低', color: '#F59E0B', type: 'warning' },
  unavailable: { text: '不可用', color: '#EF4444', type: 'danger' },
  no_key: { text: '未配置Key', color: '#F59E0B', type: 'warning' },
  error: { text: '异常', color: '#EF4444', type: 'danger' },
  disabled: { text: '已禁用', color: '#9CA3AF', type: 'info' },
}

const info = computed(() => statusMap[props.status] || statusMap.error)
</script>

<template>
  <el-tag :type="info.type as any" size="small" round>
    <span class="status-dot" :style="{ backgroundColor: info.color }" />
    {{ info.text }}
  </el-tag>
</template>

<style scoped>
.status-dot {
  display: inline-block;
  width: 6px;
  height: 6px;
  border-radius: 50%;
  margin-right: 4px;
  vertical-align: middle;
}
</style>
