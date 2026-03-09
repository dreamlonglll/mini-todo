export type ScheduleStrategy = 'manual' | 'cron'

export type ScheduleStatusType =
  | 'none'
  | 'pending'
  | 'queued'
  | 'running'
  | 'reviewing'
  | 'completed'
  | 'failed'
  | 'cancelled'

export interface TaskDependency {
  id: number
  subtaskId: number
  dependsOnId: number
  dependencyType: 'finish-to-start' | 'finish-to-finish' | 'start-to-start'
  createdAt: string
}

export interface PromptTemplate {
  id: number
  name: string
  category: string
  description: string
  templateContent: string
  variables: string
  isBuiltin: boolean
  createdAt: string
  updatedAt: string
}

export interface TemplateVariable {
  name: string
  label: string
  type: 'text' | 'textarea' | 'select'
  required: boolean
  defaultValue?: string
  options?: string[]
}

export interface ScheduledTodoInfo {
  id: number
  title: string
  cronExpression: string
  cronDescription: string
  scheduleEnabled: boolean
  lastScheduledRun: string | null
  nextRun: string
  pendingSubtasks: number
}

export const SCHEDULE_STATUS_MAP: Record<ScheduleStatusType, { label: string; type: string }> = {
  none: { label: '未调度', type: 'info' },
  pending: { label: '待调度', type: 'warning' },
  queued: { label: '排队中', type: '' },
  running: { label: '执行中', type: 'primary' },
  reviewing: { label: '待审核', type: 'warning' },
  completed: { label: '已完成', type: 'success' },
  failed: { label: '失败', type: 'danger' },
  cancelled: { label: '已取消', type: 'info' },
}

export const CRON_PRESETS = [
  { label: '每小时', value: '0 * * * *' },
  { label: '每天 9:00', value: '0 9 * * *' },
  { label: '每天 18:00', value: '0 18 * * *' },
  { label: '每周一 9:00', value: '0 9 * * 1' },
  { label: '每月 1 日 9:00', value: '0 9 1 * *' },
  { label: '每 30 分钟', value: '*/30 * * * *' },
] as const
