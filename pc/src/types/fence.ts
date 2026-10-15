/**
 * 桌面收纳箱（v29 `fences`，后端 `commands/fence.rs`）
 *
 * 几何字段是物理像素：x / y 为虚拟屏幕坐标（折叠态即标题栏位置），width / height 为展开态尺寸，
 * 只在用户拖动 / 缩放结束后经 `commit_fence_geometry` 落库。设备相关数据，不进导出 / 同步。
 */
export interface Fence {
  id: number
  title: string
  /** 收纳根目录下的子文件夹名 */
  folderName: string
  x: number
  y: number
  width: number
  height: number
  collapsed: boolean
  /** 锁定（「固定」）：不可移动 / 缩放，仍可拖入、折叠、改名 */
  locked: boolean
  bgColor: string
  /** 背景透明度 0~1 */
  bgAlpha: number
  /** 图标大小（逻辑像素） */
  iconSize: number
  sortOrder: number
  createdAt: string
  updatedAt: string
}

/** `update_fence` 的 patch：只更新给出的字段 */
export interface UpdateFenceRequest {
  collapsed?: boolean
  locked?: boolean
  bgColor?: string
  bgAlpha?: number
  iconSize?: number
}

/** 标题栏高度（CSS 像素），与后端 `FENCE_TITLE_BAR_DIP` 一致：折叠态窗口高度 = 它 × 缩放比例 */
export const FENCE_TITLE_BAR_HEIGHT = 32

/** 最小宽度 / 展开态最小高度（CSS 像素），与后端 `MIN_WIDTH_DIP` / `MIN_EXPANDED_HEIGHT_DIP` 一致 */
export const FENCE_MIN_WIDTH = 160
export const FENCE_MIN_EXPANDED_HEIGHT = 160

/** 标题最大长度（字符数），与后端 `MAX_TITLE_CHARS` 一致 */
export const FENCE_TITLE_MAX_LENGTH = 40

/** 外观面板的背景透明度滑块范围（百分比） */
export const FENCE_BG_ALPHA_MIN_PERCENT = 5
export const FENCE_BG_ALPHA_MAX_PERCENT = 100

/** 图标大小上下限 / Ctrl+滚轮每格的步长（CSS 像素），上下限与后端 `MIN_ICON_SIZE` / `MAX_ICON_SIZE` 一致 */
export const FENCE_ICON_SIZE_MIN = 24
export const FENCE_ICON_SIZE_MAX = 128
export const FENCE_ICON_SIZE_STEP = 8

/** 图标缓存的尺寸档位（物理像素），与后端 `fence_fs::ICON_SIZE_BUCKETS` 一致：请求尺寸向上取整到档位 */
export const FENCE_ICON_BUCKETS = [16, 24, 32, 48, 64, 96, 128, 256] as const

/** 收纳箱内容变化事件（后端对账后发给 `fence-{id}` 窗口，payload 为 `FenceEntry[]`） */
export const FENCE_ITEMS_CHANGED_EVENT = 'fence-items-changed'

/** 收纳箱里的一项（后端 `fence_fs::FenceEntry`），文件夹是内容的 source of truth */
export interface FenceEntry {
  /** 收纳文件夹内的实际文件名 */
  fileName: string
  /** 显示名：`.lnk` / `.url` 去掉后缀 */
  displayName: string
  /** 完整路径 */
  path: string
  isDir: boolean
  isShortcut: boolean
  /** 修改时间（Unix 毫秒） */
  modifiedMs: number | null
}

/** 移入 / 移回 / 跨箱的逐项状态 */
export type FenceItemOpStatus = 'moved' | 'cancelled' | 'failed' | 'skipped'

/** 移入 / 移回 / 跨箱的逐项结果（后端 `fence_fs::ItemOpResult`） */
export interface FenceItemOpResult {
  /** 源完整路径 */
  source: string
  status: FenceItemOpStatus
  /** moved 时：移动后的实际文件名 */
  fileName: string | null
  /** moved 时：移动后的完整路径 */
  destPath: string | null
  /** 失败 / 取消 / 跳过的原因；moved 时偶尔带提示 */
  error: string | null
}

/** `fence_move_in` 的结果 */
export interface FenceMoveInResult {
  /** 与传入的 paths 一一对应 */
  results: FenceItemOpResult[]
  /** 需要显示一次性提示：桌面被 OneDrive 重定向，移出桌面会同步到其他设备 */
  onedriveNotice: boolean
}

/** `rename_fence` 的结果 */
export interface RenameFenceResult {
  fence: Fence
  /** 收纳文件夹没能跟着改名的原因（标题照改、保留旧文件夹名） */
  folderError: string | null
}

/** `delete_fence` 的结果 */
export interface DeleteFenceResult {
  /** 已删除（全部移回、文件夹已删、窗口已由后端关闭）；false = 有项目没能移回或收纳文件夹没能删除，收纳箱整个保留 */
  deleted: boolean
  /** 已移回的项目数 */
  restored: number
  /** 没能移走、仍留在收纳文件夹里的文件名 */
  remaining: string[]
  /** 移回失败 / 被取消的项 */
  failures: FenceItemOpResult[]
}
