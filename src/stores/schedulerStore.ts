import { defineStore } from 'pinia'
import { ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import type {
  ScheduledTodoInfo,
  TriggerTodoInfo,
  TaskDependency,
  PromptTemplate,
  ScheduleStrategy,
} from '@/types/scheduler'

export const useSchedulerStore = defineStore('scheduler', () => {
  const isRunning = ref(false)
  const scheduledTodos = ref<ScheduledTodoInfo[]>([])
  const triggerTodos = ref<TriggerTodoInfo[]>([])
  const templates = ref<PromptTemplate[]>([])

  // ========== 调度器控制 ==========

  async function startScheduler() {
    await invoke('start_scheduler')
    isRunning.value = true
  }

  async function stopScheduler() {
    await invoke('stop_scheduler')
    isRunning.value = false
  }

  async function refreshSchedulerStatus() {
    isRunning.value = await invoke<boolean>('get_scheduler_status')
  }

  // ========== 子任务调度操作 ==========

  async function updateSubtaskScheduleStatus(subtaskId: number, targetStatus: string) {
    return invoke<string>('update_subtask_schedule_status', { subtaskId, targetStatus })
  }

  async function updateSubtaskPriority(subtaskId: number, priorityScore: number) {
    await invoke('update_subtask_priority', { subtaskId, priorityScore })
  }

  async function updateSubtaskTimeout(subtaskId: number, timeoutSecs: number) {
    await invoke('update_subtask_timeout', { subtaskId, timeoutSecs })
  }

  async function updateSubtaskMaxRetries(subtaskId: number, maxRetries: number) {
    await invoke('update_subtask_max_retries', { subtaskId, maxRetries })
  }

  async function submitTask(subtaskId: number) {
    await invoke('submit_task_to_scheduler', { subtaskId })
  }

  // ========== 依赖管理 ==========

  async function addDependency(subtaskId: number, dependsOnId: number, dependencyType: string = 'finish-to-start') {
    return invoke<number>('add_task_dependency', { subtaskId, dependsOnId, dependencyType })
  }

  async function removeDependency(dependencyId: number) {
    await invoke('remove_task_dependency', { dependencyId })
  }

  async function getDependencies(subtaskId: number) {
    return invoke<TaskDependency[]>('get_task_dependencies', { subtaskId })
  }

  async function checkDependenciesMet(subtaskId: number) {
    return invoke<boolean>('check_dependencies_met', { subtaskId })
  }

  // ========== Todo 调度配置 ==========

  async function updateTodoScheduleConfig(
    todoId: number,
    strategy?: ScheduleStrategy,
    cronExpression?: string,
    enabled?: boolean,
  ) {
    await invoke('update_todo_schedule_config', {
      todoId,
      strategy: strategy ?? null,
      cronExpression: cronExpression ?? null,
      enabled: enabled ?? null,
    })
  }

  // ========== 定时任务 ==========

  async function loadScheduledTodos() {
    scheduledTodos.value = await invoke<ScheduledTodoInfo[]>('get_scheduled_todos')
  }

  async function validateCron(expression: string) {
    return invoke<string>('validate_cron_expression', { expression })
  }

  async function getNextCronExecution(expression: string) {
    return invoke<string>('get_next_cron_execution', { expression })
  }

  // ========== 触发器 ==========

  async function loadTriggerTodos() {
    triggerTodos.value = await invoke<TriggerTodoInfo[]>('get_trigger_todos')
  }

  async function initGitTrigger(projectPath: string) {
    await invoke('init_git_trigger', { projectPath })
  }

  async function getLastCommitInfo(projectPath: string) {
    return invoke<string | null>('get_last_commit_info', { projectPath })
  }

  // ========== Prompt 模板 ==========

  async function loadTemplates() {
    templates.value = await invoke<PromptTemplate[]>('get_prompt_templates')
  }

  async function getTemplatesByCategory(category: string) {
    return invoke<PromptTemplate[]>('get_prompt_templates_by_category', { category })
  }

  async function createTemplate(template: Partial<PromptTemplate>) {
    return invoke<number>('create_prompt_template', {
      name: template.name,
      category: template.category ?? '',
      description: template.description ?? '',
      templateContent: template.templateContent ?? '',
      variables: template.variables ?? '[]',
      isBuiltin: false,
    })
  }

  async function updateTemplate(id: number, template: Partial<PromptTemplate>) {
    await invoke('update_prompt_template', {
      id,
      name: template.name ?? null,
      category: template.category ?? null,
      description: template.description ?? null,
      templateContent: template.templateContent ?? null,
      variables: template.variables ?? null,
    })
  }

  async function deleteTemplate(id: number) {
    await invoke('delete_prompt_template', { id })
  }

  async function renderTemplate(templateId: number, variables: Record<string, string>) {
    return invoke<string>('render_prompt_template', {
      templateId,
      variables: JSON.stringify(variables),
    })
  }

  return {
    isRunning,
    scheduledTodos,
    triggerTodos,
    templates,
    startScheduler,
    stopScheduler,
    refreshSchedulerStatus,
    updateSubtaskScheduleStatus,
    updateSubtaskPriority,
    updateSubtaskTimeout,
    updateSubtaskMaxRetries,
    submitTask,
    addDependency,
    removeDependency,
    getDependencies,
    checkDependenciesMet,
    updateTodoScheduleConfig,
    loadScheduledTodos,
    validateCron,
    getNextCronExecution,
    loadTriggerTodos,
    initGitTrigger,
    getLastCommitInfo,
    loadTemplates,
    getTemplatesByCategory,
    createTemplate,
    updateTemplate,
    deleteTemplate,
    renderTemplate,
  }
})
