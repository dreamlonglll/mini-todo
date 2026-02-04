// 预设颜色列表
export const PRESET_COLORS = [
  { name: '红色', value: '#EF4444' },
  { name: '橙色', value: '#F59E0B' },
  { name: '黄色', value: '#EAB308' },
  { name: '绿色', value: '#10B981' },
  { name: '青色', value: '#06B6D4' },
  { name: '蓝色', value: '#3B82F6' },
  { name: '紫色', value: '#8B5CF6' },
  { name: '粉色', value: '#EC4899' },
] as const

// 默认颜色（绿色）
export const DEFAULT_COLOR = '#10B981'

// 四象限定义
export const QUADRANTS = {
  IMPORTANT_URGENT: 1,      // 重要且紧急
  IMPORTANT_NOT_URGENT: 2,  // 重要不紧急
  URGENT_NOT_IMPORTANT: 3,  // 紧急不重要
  NOT_URGENT_NOT_IMPORTANT: 4, // 不紧急不重要
} as const

export type QuadrantType = typeof QUADRANTS[keyof typeof QUADRANTS]

// 四象限信息
export const QUADRANT_INFO = [
  { id: QUADRANTS.IMPORTANT_URGENT, name: '重要且紧急', color: '#EF4444', bgColor: 'rgba(239, 68, 68, 0.1)' },
  { id: QUADRANTS.IMPORTANT_NOT_URGENT, name: '重要不紧急', color: '#F59E0B', bgColor: 'rgba(245, 158, 11, 0.1)' },
  { id: QUADRANTS.URGENT_NOT_IMPORTANT, name: '紧急不重要', color: '#3B82F6', bgColor: 'rgba(59, 130, 246, 0.1)' },
  { id: QUADRANTS.NOT_URGENT_NOT_IMPORTANT, name: '不紧急不重要', color: '#10B981', bgColor: 'rgba(16, 185, 129, 0.1)' },
] as const

// 默认象限（不紧急不重要）
export const DEFAULT_QUADRANT = QUADRANTS.NOT_URGENT_NOT_IMPORTANT

// 视图模式
export type ViewMode = 'list' | 'quadrant'

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
  /** 颜色（HEX 格式，如 #EF4444） */
  color: string
  /** 四象限：1=重要紧急, 2=重要不紧急, 3=紧急不重要, 4=不紧急不重要 */
  quadrant: QuadrantType
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
  /** 颜色（HEX 格式） */
  color: string
  /** 四象限：1=重要紧急, 2=重要不紧急, 3=紧急不重要, 4=不紧急不重要 */
  quadrant?: QuadrantType
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
  /** 颜色（HEX 格式） */
  color?: string
  /** 四象限：1=重要紧急, 2=重要不紧急, 3=紧急不重要, 4=不紧急不重要 */
  quadrant?: QuadrantType
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
