export interface WorkflowStep {
  id: number
  todoId: number
  stepOrder: number
  stepType: 'subtask' | 'prompt'
  subtaskId?: number
  promptText?: string
  status: WorkflowStepStatus
  createdAt: string
}

export type WorkflowStepStatus = 'pending' | 'running' | 'completed' | 'failed' | 'skipped'

export interface WorkflowStepInput {
  stepType: 'subtask' | 'prompt'
  subtaskId?: number
  promptText?: string
}

export const STEP_TYPE_LABELS: Record<string, string> = {
  subtask: '执行子任务',
  prompt: '执行提示词',
}

export const STEP_STATUS_MAP: Record<WorkflowStepStatus, { label: string; type: string }> = {
  pending: { label: '待执行', type: 'info' },
  running: { label: '执行中', type: 'primary' },
  completed: { label: '已完成', type: 'success' },
  failed: { label: '失败', type: 'danger' },
  skipped: { label: '已跳过', type: 'info' },
}
