/**
 * 日期时间工具（跨端契约 K1）
 *
 * 规范存储格式：`YYYY-MM-DD HH:MM:SS`（本地墙钟、无时区后缀），与 PC SQLite
 * `datetime('now','localtime')`、云端 `time.rs` 的输出逐字节一致。前端写入一律用它。
 *
 * 读取兼容（云端 / AI / 旧版本可能写入的形态）：
 * - `YYYY-MM-DD HH:MM:SS`、`YYYY-MM-DDTHH:MM:SS`
 * - `YYYY-MM-DD HH:MM`、`YYYY-MM-DDTHH:MM`
 * - 可选小数秒（`.123`）
 * - 可选时区后缀 `Z` / `±HH:MM` / `±HHMM` / `±HH`（换算为本机墙钟）
 * - 仅日期 `YYYY-MM-DD`：按字段类型补默认时刻（见 DEFAULT_TIME）
 *
 * 不要再用 `split('T')` 之类的字符串切分解析时间：空格格式会被拼成
 * `…09:00:00T09:00:00` 再写回数据库。
 */
import dayjs from 'dayjs'

/** 时间字段类型，决定仅日期输入时补的默认时刻 */
export type DateTimeKind = 'start' | 'end' | 'notify' | 'generic'

/** 仅日期时的默认时刻（与编辑器、后端一致：开始 00:00，截止 23:59，提醒 09:00） */
const DEFAULT_TIME: Record<DateTimeKind, [number, number, number]> = {
  start: [0, 0, 0],
  end: [23, 59, 0],
  notify: [9, 0, 0],
  generic: [0, 0, 0],
}

export const STORAGE_FORMAT = 'YYYY-MM-DD HH:mm:ss'

// 日期 + 可选时间（T 或空格分隔，秒与小数秒可选）+ 可选时区；时区只允许跟在时间后面
const DATE_TIME_RE =
  /^(\d{4})-(\d{2})-(\d{2})(?:[Tt ](\d{2}):(\d{2})(?::(\d{2})(?:\.\d+)?)?\s*([Zz]|[+-]\d{2}(?::?\d{2})?)?)?$/

function daysInMonth(year: number, month: number): number {
  return new Date(year, month, 0).getDate()
}

/** 时区后缀 → 相对 UTC 的分钟数；非法时返回 null */
function parseOffsetMinutes(offset: string): number | null {
  if (offset === 'Z' || offset === 'z') return 0
  const sign = offset[0] === '-' ? -1 : 1
  const digits = offset.slice(1).replace(':', '')
  const hours = Number(digits.slice(0, 2))
  const minutes = digits.length > 2 ? Number(digits.slice(2, 4)) : 0
  if (hours > 23 || minutes > 59) return null
  return sign * (hours * 60 + minutes)
}

/**
 * 解析任意 K1 形态为本地时间的 Date；无法识别时返回 null
 *
 * 不带时区的输入按本机墙钟解释；带时区的先换算成绝对时间再落到本机墙钟
 */
export function parseDateTime(
  value: string | null | undefined,
  kind: DateTimeKind = 'generic'
): Date | null {
  if (!value) return null
  const match = DATE_TIME_RE.exec(value.trim())
  if (!match) return null

  const [, y, mo, d, h, mi, s, offset] = match
  const year = Number(y)
  const month = Number(mo)
  const day = Number(d)
  if (month < 1 || month > 12 || day < 1 || day > daysInMonth(year, month)) return null

  let hour: number
  let minute: number
  let second: number
  if (h === undefined) {
    ;[hour, minute, second] = DEFAULT_TIME[kind]
  } else {
    hour = Number(h)
    minute = Number(mi)
    second = s === undefined ? 0 : Number(s)
    if (hour > 23 || minute > 59 || second > 59) return null
  }

  if (offset) {
    const offsetMinutes = parseOffsetMinutes(offset)
    if (offsetMinutes === null) return null
    return new Date(Date.UTC(year, month - 1, day, hour, minute, second) - offsetMinutes * 60_000)
  }
  return new Date(year, month - 1, day, hour, minute, second)
}

/** Date → 规范存储格式 `YYYY-MM-DD HH:MM:SS` */
export function formatStorageDateTime(date: Date): string {
  return dayjs(date).format(STORAGE_FORMAT)
}

/** 任意 K1 形态 → 规范存储格式；无法识别时返回 null */
export function normalizeDateTime(
  value: string | null | undefined,
  kind: DateTimeKind = 'generic'
): string | null {
  const date = parseDateTime(value, kind)
  return date ? formatStorageDateTime(date) : null
}

/**
 * 把日期 / 时间选择器的值合成为规范存储格式
 *
 * @param date `YYYY-MM-DD`（el-date-picker value-format），为空时返回 null
 * @param time `HH:mm` 或 `HH:mm:ss`（el-time-picker value-format），为空时按 kind 补默认时刻
 */
export function composeDateTime(
  date: string | null | undefined,
  time: string | null | undefined,
  kind: DateTimeKind
): string | null {
  if (!date) return null
  return normalizeDateTime(time ? `${date} ${time}` : date, kind)
}

/** 拆成日期 / 时间选择器的值（`YYYY-MM-DD` / `HH:mm`）；无法识别时返回 null */
export function splitDateTime(
  value: string | null | undefined,
  kind: DateTimeKind = 'generic'
): { date: string; time: string } | null {
  const date = parseDateTime(value, kind)
  if (!date) return null
  const d = dayjs(date)
  return { date: d.format('YYYY-MM-DD'), time: d.format('HH:mm') }
}

/**
 * 两个时间是否落在同一分钟（任意 K1 形态）。都为空视为相同；只有一边为空或无法识别视为不同。
 *
 * 编辑器的时间选择器只精确到分钟，用它判断"用户有没有改过这个时间"：
 * 已存的 `09:00:30`（云端 / AI 写入）回填选择器后是 `09:00`，不应被当成修改
 */
export function isSameMinute(
  a: string | null | undefined,
  b: string | null | undefined,
  kind: DateTimeKind = 'generic'
): boolean {
  if (!a && !b) return true
  const da = parseDateTime(a, kind)
  const db = parseDateTime(b, kind)
  if (!da || !db) return false
  return Math.floor(da.getTime() / 60_000) === Math.floor(db.getTime() / 60_000)
}

/** Date → 本地日期键 `YYYY-MM-DD`（日历格子、按天比较用） */
export function formatDateKey(date: Date): string {
  return dayjs(date).format('YYYY-MM-DD')
}

/** 任意 K1 形态 → 本地日期键 `YYYY-MM-DD`；无法识别时返回 null */
export function toDateKey(
  value: string | null | undefined,
  kind: DateTimeKind = 'generic'
): string | null {
  const date = parseDateTime(value, kind)
  return date ? formatDateKey(date) : null
}

/**
 * 展示用格式化（dayjs 格式串，默认 `YYYY-MM-DD HH:mm`）；无法识别时返回 null，
 * 由调用方决定是隐藏还是回落到原文
 */
export function formatDateTime(
  value: string | null | undefined,
  pattern = 'YYYY-MM-DD HH:mm',
  kind: DateTimeKind = 'generic'
): string | null {
  const date = parseDateTime(value, kind)
  return date ? dayjs(date).format(pattern) : null
}
