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

// 窗口底色默认值
//
// 与 Rust 侧 db/models.rs 的 DEFAULT_WINDOW_BG_COLOR / DEFAULT_WINDOW_BG_ALPHA
// 以及 main.scss 的 :root 回落值保持一致。alpha 取 0.45 是因为抽成可配置之前，
// 深色主题是多层半透明黑叠加，合成不透明度约为此值。
export const DEFAULT_BG_COLOR = '#000000'
export const DEFAULT_BG_ALPHA = 0.45

// 底色预设：深色系为主，浅色底在深色主题下会让白字不可读
export const PRESET_BG_COLORS = [
  { name: '黑色', value: '#000000' },
  { name: '深灰', value: '#1E293B' },
  { name: '深蓝', value: '#0F2942' },
  { name: '深紫', value: '#2E1065' },
  { name: '深绿', value: '#052E24' },
  { name: '深棕', value: '#2B1A0F' },
] as const

// 应用设置接口
export interface AppSettings {
  windowPosition: WindowPosition | null
  windowSize: WindowSize | null
  isFixed: boolean
  /**
   * 固定模式时是否嵌入桌面（嵌在桌面图标之上、所有应用窗口之下，Win+D 不最小化）。
   * 仅 Windows 生效（Windows 11 24H2 及以上保证 Win+D 期间可见）
   */
  fixedEmbedDesktop: boolean
  autoHideEnabled: boolean
  /** 贴边唤起时是否临时置顶（关闭后窗口会被全屏窗口遮挡） */
  topOnWake: boolean
  /** 窗口底色（HEX），仅深色主题下生效 */
  windowBgColor: string
  /** 窗口背景透明度 0~1，仅深色主题下生效 */
  windowBgAlpha: number
  /** 文本主题：light（浅色文字，适配深色背景）或 dark（深色文字，适配浅色背景）*/
  textTheme: TextTheme
}

// 窗口模式：普通 / 固定（可贴边隐藏、置顶唤起；开启"嵌入桌面"后固定即嵌入桌面图标层之上）
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
  | 'fixedEmbedDesktop'
  | 'theme'
  | 'windowBackground'
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

// WebDAV 同步设置（get_sync_settings 返回 / save_sync_settings 入参）
export interface SyncSettings {
  webdavUrl: string
  webdavUsername: string
  /**
   * 读取时恒为空串：后端不再把已保存的密码回传给 WebView（见 hasPassword）。
   * 保存时为空串表示保留已保存的密码，非空则替换
   */
  webdavPassword: string
  /** 后端是否已保存密码（只读，保存时忽略） */
  hasPassword: boolean
  /** 保存时显式清空已保存的密码（只写，可缺省） */
  clearPassword?: boolean
  autoSync: boolean
  syncInterval: number
  /** 上次同步时间（元信息，可能是带时区偏移的 ISO 字符串） */
  lastSyncAt: string | null
  deviceId: string
}

/**
 * 同步结果状态
 * - no_changes：两端都没有需要同步的更改
 * - pulled：只把云端更改合并到了本地
 * - pushed：只把本地更改推送到了云端
 * - merged：两端都有更改，已合并并推送
 */
export type SyncStatus = 'no_changes' | 'pulled' | 'pushed' | 'merged'

/**
 * 同步报告（webdav_sync / webdav_force_pull / webdav_force_push 的返回值，
 * 也是后端 `sync-completed` 事件的负载）
 */
export interface SyncReport {
  status: SyncStatus
  lastSyncAt: string
  todosInserted: number
  todosUpdated: number
  todosDeleted: number
  subtasksInserted: number
  subtasksUpdated: number
  subtasksDeleted: number
  /** 云端无法解析而被跳过的记录数（不会被当作删除） */
  recordsSkipped: number
  imagesUploaded: number
  imagesDownloaded: number
  /** 是否应用了云端的应用设置（主窗口需要重载设置） */
  settingsApplied: boolean
}
