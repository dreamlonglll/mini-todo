/**
 * 桌面收纳箱窗口（FenceView）用到的纯函数
 */
import {
  FENCE_ICON_BUCKETS,
  FENCE_ICON_SIZE_MAX,
  FENCE_ICON_SIZE_MIN,
  FENCE_ICON_SIZE_STEP,
  FENCE_TITLE_MAX_LENGTH,
} from '@/types'
import type { DeleteFenceResult, FenceItemOpResult } from '@/types'

/** 路由 query 里的收纳箱 id：只接受正整数，其它一律 null */
export function parseFenceId(value: unknown): number | null {
  const raw = Array.isArray(value) ? value[0] : value
  if (typeof raw !== 'string' || !/^\d+$/.test(raw)) return null
  const id = Number(raw)
  return Number.isSafeInteger(id) && id > 0 ? id : null
}

/**
 * 规范化收纳箱标题，规则与后端 `normalize_title` 一致：控制字符换成空格、去首尾空白；
 * 为空或超过 {@link FENCE_TITLE_MAX_LENGTH} 个字符时返回 null
 */
export function normalizeFenceTitle(raw: string): string | null {
  // eslint-disable-next-line no-control-regex
  const title = raw.replace(/[\u0000-\u001f\u007f-\u009f]/g, ' ').trim()
  if (!title || Array.from(title).length > FENCE_TITLE_MAX_LENGTH) return null
  return title
}

/** 按下后移动超过阈值（CSS 像素）才算拖动：没动就松开的是单击 / 双击，留给 DOM 处理 */
export function exceedsDragThreshold(dx: number, dy: number, threshold = 3): boolean {
  return Math.abs(dx) > threshold || Math.abs(dy) > threshold
}

/**
 * 判定"用户拖动 / 缩放结束"的闸门
 *
 * tauri 的 `startDragging` / `startResizeDragging` 发出去就返回，拖动结束没有事件。做法：
 * 用户按下标题栏 / 缩放手柄开始拖动时 {@link arm}，之后的 `onMoved` / `onResized` 事件
 * 在闸门打开期间才算数（每个事件续期一次），停下来一段时间后由调用方防抖落库。闸门在最后一次事件后
 * `windowMs` 自然失效（按住不放停顿一会儿再接着拖仍然算数）；调用方做程序调整前先 {@link disarm}。
 * 闸门之外的移动 / 缩放（显示器变化导致的系统挪窗、折叠展开时的程序调整）一律忽略，不落库。
 */
export class GeometryCommitGate {
  private armedUntil = Number.NEGATIVE_INFINITY

  constructor(private readonly windowMs: number) {}

  /** 用户开始拖动 / 缩放 */
  arm(now: number): void {
    this.armedUntil = now + this.windowMs
  }

  /** 收到一次移动 / 缩放事件：闸门开着就续期并返回 true（应当安排落库），否则返回 false */
  accept(now: number): boolean {
    if (now > this.armedUntil) return false
    this.armedUntil = now + this.windowMs
    return true
  }

  isArmed(now: number): boolean {
    return now <= this.armedUntil
  }

  disarm(): void {
    this.armedUntil = Number.NEGATIVE_INFINITY
  }
}

/** 缩放方向（与缩放手柄一一对应） */
export type FenceResizeDirection = 'n' | 's' | 'w' | 'e' | 'nw' | 'ne' | 'sw' | 'se'

/** 物理像素矩形 */
export interface PxRect {
  x: number
  y: number
  width: number
  height: number
}

/**
 * 按拖动缩放手柄的位移（物理像素）算出新矩形：拖哪条边改哪条边，对边不动；
 * 不小于最小尺寸（拖左 / 上边缩到最小时就停住，右下角不会被推着走）
 */
export function resizeRect(
  start: PxRect,
  direction: FenceResizeDirection,
  dx: number,
  dy: number,
  minWidth: number,
  minHeight: number
): PxRect {
  let { x, y, width, height } = start
  if (direction.includes('e')) width = Math.max(minWidth, start.width + dx)
  if (direction.includes('s')) height = Math.max(minHeight, start.height + dy)
  if (direction.includes('w')) {
    width = Math.max(minWidth, start.width - dx)
    x = start.x + start.width - width
  }
  if (direction.includes('n')) {
    height = Math.max(minHeight, start.height - dy)
    y = start.y + start.height - height
  }
  return { x: Math.round(x), y: Math.round(y), width: Math.round(width), height: Math.round(height) }
}

/** 物理像素 → 图标缓存档位：向上取整到档位，超过最大档按最大档（与后端 `icon_bucket` 一致） */
export function iconBucket(sizePx: number): number {
  return FENCE_ICON_BUCKETS.find((bucket) => bucket >= sizePx) ?? FENCE_ICON_BUCKETS[FENCE_ICON_BUCKETS.length - 1]
}

/** 请求图标用的物理像素：逻辑尺寸 × devicePixelRatio，向上取整 */
export function iconRequestPx(iconSize: number, dpr: number): number {
  return Math.ceil(iconSize * (dpr > 0 ? dpr : 1))
}

/** 把图标大小夹在上下限内并取整 */
export function clampIconSize(size: number): number {
  return Math.min(FENCE_ICON_SIZE_MAX, Math.max(FENCE_ICON_SIZE_MIN, Math.round(size)))
}

/** Ctrl+滚轮后的图标大小：向上滚（deltaY < 0）放大一档、向下滚缩小一档，夹在上下限内 */
export function nextIconSize(current: number, deltaY: number): number {
  if (deltaY === 0) return clampIconSize(current)
  return clampIconSize(current + (deltaY < 0 ? FENCE_ICON_SIZE_STEP : -FENCE_ICON_SIZE_STEP))
}

/** 去掉末尾分隔符后，最后一个分隔符（`\` 或 `/`）的位置 */
function splitLastSeparator(path: string): [string, number] {
  const trimmed = path.replace(/[\\/]+$/, '')
  return [trimmed, Math.max(trimmed.lastIndexOf('\\'), trimmed.lastIndexOf('/'))]
}

/** 路径的最后一段（Windows / POSIX 分隔符都认） */
export function baseName(path: string): string {
  const [trimmed, index] = splitLastSeparator(path)
  return index >= 0 ? trimmed.slice(index + 1) : trimmed
}

/** 路径的上级目录 */
export function parentDir(path: string): string {
  const [trimmed, index] = splitLastSeparator(path)
  return index > 0 ? trimmed.slice(0, index) : trimmed
}

/** 逐项结果的汇总（跳过的项不计：已在该收纳箱、desktop.ini、隐藏文件等，一般无需提示） */
export interface FenceOpSummary {
  moved: number
  cancelled: number
  failed: { name: string; reason: string }[]
  /** 已移动但带提示的项（例如源文件未能删除） */
  notes: { name: string; note: string }[]
}

export function summarizeOpResults(results: FenceItemOpResult[]): FenceOpSummary {
  const summary: FenceOpSummary = { moved: 0, cancelled: 0, failed: [], notes: [] }
  for (const result of results) {
    const name = baseName(result.source)
    if (result.status === 'moved') {
      summary.moved += 1
      if (result.error) summary.notes.push({ name, note: result.error })
    } else if (result.status === 'cancelled') {
      summary.cancelled += 1
    } else if (result.status === 'failed') {
      summary.failed.push({ name, reason: result.error || '未知原因' })
    }
  }
  return summary
}

/** 提示里最多列出几项的名字 */
const MAX_LISTED_ITEMS = 2

/**
 * 删除中止（收纳箱已保留）的提示文案：列出没能移回的文件名，附上失败 / 取消原因；
 * 项目都已移回、只是收纳文件夹本身没能删除（被占用、`desktop.ini` 删不掉等）时单独说明
 */
export function describeDeleteKept(result: DeleteFenceResult): string {
  const reasons = new Map(result.failures.map((f) => [baseName(f.source).toLowerCase(), f.error ?? '']))
  const names = result.remaining.length > 0 ? result.remaining : result.failures.map((f) => baseName(f.source))
  if (names.length === 0) {
    const restored = result.restored > 0 ? `${result.restored} 个项目已移回桌面，但` : ''
    return `${restored}收纳文件夹未能删除（可能被占用），收纳箱已保留，可稍后再删除`
  }
  const listed = names
    .slice(0, MAX_LISTED_ITEMS)
    .map((name) => {
      const reason = reasons.get(name.toLowerCase())
      return reason ? `${name}（${reason}）` : name
    })
    .join('、')
  const more = names.length > MAX_LISTED_ITEMS ? ' 等' : ''
  const head = `${names.length} 个项目未能移回桌面，收纳箱已保留`
  return listed ? `${head}：${listed}${more}` : head
}

/**
 * 拖入结果的提示文案：全部成功（或只有跳过的项）时返回 null；
 * 有失败 → error（部分成功时 warning）；只有取消 → info；只有"已移动但带提示" → warning
 */
export function describeMoveIn(
  summary: FenceOpSummary
): { type: 'error' | 'warning' | 'info'; message: string } | null {
  const parts: string[] = []
  if (summary.failed.length > 0) {
    const listed = summary.failed
      .slice(0, MAX_LISTED_ITEMS)
      .map((f) => `${f.name}（${f.reason}）`)
      .join('、')
    const more = summary.failed.length > MAX_LISTED_ITEMS ? ' 等' : ''
    parts.push(`${summary.failed.length} 项未能收纳：${listed}${more}`)
  }
  if (summary.cancelled > 0) parts.push(`已取消 ${summary.cancelled} 项`)
  if (summary.notes.length > 0) parts.push(`${summary.notes[0].name}：${summary.notes[0].note}`)
  if (parts.length === 0) return null
  if (summary.moved > 0) parts.unshift(`已收纳 ${summary.moved} 项`)
  let type: 'error' | 'warning' | 'info' = 'warning'
  if (summary.failed.length > 0) type = summary.moved > 0 ? 'warning' : 'error'
  else if (summary.notes.length === 0) type = 'info'
  return { type, message: parts.join('；') }
}
