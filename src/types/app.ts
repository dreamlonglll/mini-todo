// 窗口位置接口
export interface WindowPosition {
  x: number
  y: number
}

// 应用设置接口
export interface AppSettings {
  windowPosition: WindowPosition | null
  isFixed: boolean
}

// 窗口模式
export type WindowMode = 'normal' | 'fixed'
