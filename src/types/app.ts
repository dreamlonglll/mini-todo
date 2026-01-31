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

// 屏幕配置记录，用于存储不同屏幕组合下的窗口状态
export interface ScreenConfig {
  id: number
  /** 屏幕配置唯一标识（如 "2_2560x1440@125_1920x1080@100"） */
  configId: string
  /** 显示名称（用户可编辑） */
  displayName: string | null
  /** 窗口 X 坐标 */
  windowX: number
  /** 窗口 Y 坐标 */
  windowY: number
  /** 窗口宽度 */
  windowWidth: number
  /** 窗口高度 */
  windowHeight: number
  /** 是否固定模式 */
  isFixed: boolean
  /** 创建时间 */
  createdAt: string
  /** 更新时间 */
  updatedAt: string
}

// 保存屏幕配置的请求
export interface SaveScreenConfigRequest {
  configId: string
  displayName?: string | null
  windowX: number
  windowY: number
  windowWidth: number
  windowHeight: number
  isFixed: boolean
}

// 显示器信息（用于生成屏幕配置标识）
export interface MonitorInfo {
  width: number
  height: number
  scaleFactor: number
}
