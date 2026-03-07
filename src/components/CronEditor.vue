<script setup lang="ts">
import { ref, watch } from 'vue'
import { useSchedulerStore } from '@/stores'
import { CRON_PRESETS } from '@/types/scheduler'

const props = defineProps<{
  modelValue: string
}>()

const emit = defineEmits<{
  (e: 'update:modelValue', value: string): void
}>()

const schedulerStore = useSchedulerStore()

const preset = ref('')
const customCron = ref(props.modelValue || '')
const cronDescription = ref('')
const nextRunTime = ref('')
const validationError = ref('')

watch(() => props.modelValue, (val) => {
  const found = CRON_PRESETS.find(p => p.value === val)
  if (found) {
    preset.value = val
    customCron.value = val
  } else if (val) {
    preset.value = 'custom'
    customCron.value = val
  }
  validateAndPreview(val)
}, { immediate: true })

function onPresetChange(val: string) {
  if (val === 'custom') {
    customCron.value = ''
    cronDescription.value = ''
    nextRunTime.value = ''
    return
  }
  customCron.value = val
  emit('update:modelValue', val)
  validateAndPreview(val)
}

function onCustomChange(val: string) {
  emit('update:modelValue', val)
  validateAndPreview(val)
}

async function validateAndPreview(expression: string) {
  if (!expression) {
    cronDescription.value = ''
    nextRunTime.value = ''
    validationError.value = ''
    return
  }

  try {
    cronDescription.value = await schedulerStore.validateCron(expression)
    nextRunTime.value = await schedulerStore.getNextCronExecution(expression)
    validationError.value = ''
  } catch (e) {
    validationError.value = String(e)
    cronDescription.value = ''
    nextRunTime.value = ''
  }
}
</script>

<template>
  <div class="cron-editor">
    <el-select
      v-model="preset"
      placeholder="选择预设"
      size="small"
      @change="onPresetChange"
      style="width: 100%"
    >
      <el-option
        v-for="p in CRON_PRESETS"
        :key="p.value"
        :label="p.label"
        :value="p.value"
      />
      <el-option label="自定义" value="custom" />
    </el-select>

    <el-input
      v-if="preset === 'custom'"
      v-model="customCron"
      placeholder="Cron 表达式（分 时 日 月 周）"
      size="small"
      class="custom-input"
      @change="onCustomChange"
    />

    <div v-if="cronDescription" class="cron-preview">
      {{ cronDescription }}
    </div>
    <div v-if="nextRunTime" class="cron-next">
      下次执行: {{ nextRunTime }}
    </div>
    <div v-if="validationError" class="cron-error">
      {{ validationError }}
    </div>
  </div>
</template>

<style scoped>
.cron-editor {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.custom-input {
  margin-top: 4px;
}

.cron-preview {
  font-size: 12px;
  color: var(--text-secondary);
}

.cron-next {
  font-size: 11px;
  color: var(--el-color-primary);
}

.cron-error {
  font-size: 11px;
  color: var(--el-color-danger);
}
</style>
