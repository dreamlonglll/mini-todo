// 窗口位置接口
export interface WindowPosition {
  x: number
  y: number
}

// 窗口尺寸接口
export interface WindowSize {
  width: number
  height: number
}

// 文本主题类型
export type TextTheme = 'light' | 'dark'

// 应用设置接口
export interface AppSettings {
  windowPosition: WindowPosition | null
  windowSize: WindowSize | null
  isFixed: boolean
  /** 文本主题：light（浅色文字，适配深色背景）或 dark（深色文字，适配浅色背景）*/
  textTheme: TextTheme
}

// 窗口模式
export type WindowMode = 'normal' | 'fixed'
