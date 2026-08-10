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
  autoHideEnabled: boolean
  /** 贴边唤起时是否临时置顶（关闭后窗口会被全屏窗口遮挡） */
  topOnWake: boolean
  /** 文本主题：light（浅色文字，适配深色背景）或 dark（深色文字，适配浅色背景）*/
  textTheme: TextTheme
}

// 窗口模式
export type WindowMode = 'normal' | 'fixed'

/**
 * 跨窗口设置变更事件 `app-settings-changed` 的 key
 *
 * 设置窗口是独立 WebView，与主窗口不共享 Pinia 状态。
 * 改动设置后需带上对应 key 发事件，主窗口据此重新从数据库加载。
 * 新增设置项时，这里、SettingsView 的发送处、MainView 的处理分支要同步补齐。
 */
export type AppSettingKey =
  | 'showCalendar'
  | 'autoHide'
  | 'topOnWake'
  | 'theme'
  | 'sync'
  | 'update'

/** `app-settings-changed` 事件的负载 */
export interface AppSettingChangedPayload {
  key: AppSettingKey
}

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

// WebDAV 同步设置
export interface SyncSettings {
  webdavUrl: string
  webdavUsername: string
  webdavPassword: string
  autoSync: boolean
  syncInterval: number
  lastSyncAt: string | null
  deviceId: string
}

// 同步数据结构
export interface SyncData {
  version: string
  deviceId: string
  updatedAt: string
  todos: unknown[]
  settings: unknown
  images: string[]
}

// 同步下载结果
export interface SyncDownloadResult {
  hasRemote: boolean
  remoteData: SyncData | null
  localUpdatedAt: string | null
  remoteUpdatedAt: string | null
  hasConflict: boolean
}
