// 优先级枚举
export type Priority = 'high' | 'medium' | 'low'

// 子任务接口
export interface SubTask {
  id: number
  parentId: number
  title: string
  completed: boolean
  sortOrder: number
  createdAt: string
  updatedAt: string
}

// 待办事项接口
export interface Todo {
  id: number
  title: string
  description: string | null
  priority: Priority
  notifyAt: string | null
  notifyBefore: number
  notified: boolean
  completed: boolean
  sortOrder: number
  /** 开始时间（可为空，空则使用 createdAt） */
  startTime: string | null
  /** 截止时间（可为空） */
  endTime: string | null
  createdAt: string
  updatedAt: string
  subtasks: SubTask[]
}

// 创建待办请求
export interface CreateTodoRequest {
  title: string
  description?: string
  priority: Priority
  notifyAt?: string
  notifyBefore?: number
  /** 开始时间 */
  startTime?: string
  /** 截止时间 */
  endTime?: string
}

// 更新待办请求
export interface UpdateTodoRequest {
  title?: string
  description?: string | null
  priority?: Priority
  notifyAt?: string | null
  notifyBefore?: number
  completed?: boolean
  sortOrder?: number
  /** 是否明确清除通知时间 */
  clearNotifyAt?: boolean
  /** 开始时间 */
  startTime?: string | null
  /** 截止时间 */
  endTime?: string | null
  /** 是否明确清除开始时间 */
  clearStartTime?: boolean
  /** 是否明确清除截止时间 */
  clearEndTime?: boolean
}

// 创建子任务请求
export interface CreateSubTaskRequest {
  parentId: number
  title: string
}

// 更新子任务请求
export interface UpdateSubTaskRequest {
  title?: string
  completed?: boolean
  sortOrder?: number
}

// 导出数据格式
export interface ExportData {
  version: string
  exportedAt: string
  todos: Todo[]
  settings: Record<string, unknown>
}
