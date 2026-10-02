import { describe, expect, it } from 'vitest'
import {
  composeDateTime,
  formatDateKey,
  formatDateTime,
  formatStorageDateTime,
  normalizeDateTime,
  parseDateTime,
  splitDateTime,
  toDateKey,
} from './datetime'

// vitest.config.ts 固定 TZ=Asia/Shanghai（UTC+8，无夏令时）

/** 本地墙钟各分量，便于断言 */
function wall(date: Date | null): number[] | null {
  if (!date) return null
  return [
    date.getFullYear(),
    date.getMonth() + 1,
    date.getDate(),
    date.getHours(),
    date.getMinutes(),
    date.getSeconds(),
  ]
}

describe('test environment', () => {
  it('runs in a fixed UTC+8 timezone', () => {
    expect(new Date(2026, 0, 1).getTimezoneOffset()).toBe(-480)
    expect(new Date(2026, 6, 1).getTimezoneOffset()).toBe(-480)
  })
})

describe('parseDateTime', () => {
  it('parses the canonical storage format as local wall-clock time', () => {
    expect(wall(parseDateTime('2026-10-02 09:30:15'))).toEqual([2026, 10, 2, 9, 30, 15])
  })

  it('accepts T / t separators, missing seconds and fractional seconds', () => {
    expect(wall(parseDateTime('2026-10-02T09:30:15'))).toEqual([2026, 10, 2, 9, 30, 15])
    expect(wall(parseDateTime('2026-10-02t09:30'))).toEqual([2026, 10, 2, 9, 30, 0])
    expect(wall(parseDateTime('2026-10-02 09:30'))).toEqual([2026, 10, 2, 9, 30, 0])
    expect(wall(parseDateTime('2026-10-02T09:30:15.123'))).toEqual([2026, 10, 2, 9, 30, 15])
  })

  it('trims surrounding whitespace', () => {
    expect(wall(parseDateTime('  2026-10-02 09:30:00\n'))).toEqual([2026, 10, 2, 9, 30, 0])
  })

  it('converts timezone suffixes to local wall-clock time', () => {
    expect(wall(parseDateTime('2026-10-02T01:00:00Z'))).toEqual([2026, 10, 2, 9, 0, 0])
    expect(wall(parseDateTime('2026-10-02T01:00:00.500z'))).toEqual([2026, 10, 2, 9, 0, 0])
    expect(wall(parseDateTime('2026-10-02T09:00:00+08:00'))).toEqual([2026, 10, 2, 9, 0, 0])
    expect(wall(parseDateTime('2026-10-02T09:00:00+0800'))).toEqual([2026, 10, 2, 9, 0, 0])
    expect(wall(parseDateTime('2026-10-02T09:00:00+08'))).toEqual([2026, 10, 2, 9, 0, 0])
    // 跨日：纽约晚上 = 北京次日上午
    expect(wall(parseDateTime('2026-10-01T21:00:00-05:00'))).toEqual([2026, 10, 2, 10, 0, 0])
    expect(wall(parseDateTime('2026-10-02 09:00 +05:30'))).toEqual([2026, 10, 2, 11, 30, 0])
  })

  it('fills the default time of day for date-only input by field kind', () => {
    expect(wall(parseDateTime('2026-10-02', 'start'))).toEqual([2026, 10, 2, 0, 0, 0])
    expect(wall(parseDateTime('2026-10-02', 'end'))).toEqual([2026, 10, 2, 23, 59, 0])
    expect(wall(parseDateTime('2026-10-02', 'notify'))).toEqual([2026, 10, 2, 9, 0, 0])
    expect(wall(parseDateTime('2026-10-02'))).toEqual([2026, 10, 2, 0, 0, 0])
  })

  it('keeps an explicit time even when a kind is given', () => {
    expect(wall(parseDateTime('2026-10-02 18:45:00', 'notify'))).toEqual([2026, 10, 2, 18, 45, 0])
  })

  it('validates calendar dates (no rollover)', () => {
    expect(parseDateTime('2026-02-29')).toBeNull()
    expect(wall(parseDateTime('2028-02-29'))).toEqual([2028, 2, 29, 0, 0, 0])
    expect(parseDateTime('2026-04-31 10:00:00')).toBeNull()
    expect(parseDateTime('2026-13-01')).toBeNull()
    expect(parseDateTime('2026-00-10')).toBeNull()
    expect(parseDateTime('2026-10-00')).toBeNull()
  })

  it('rejects out-of-range or malformed times and offsets', () => {
    expect(parseDateTime('2026-10-02 24:00:00')).toBeNull()
    expect(parseDateTime('2026-10-02 23:60')).toBeNull()
    expect(parseDateTime('2026-10-02 23:59:60')).toBeNull()
    expect(parseDateTime('2026-10-02 9:00')).toBeNull()
    expect(parseDateTime('2026-10-02T09:00:00+24:00')).toBeNull()
    expect(parseDateTime('2026-10-02T09:00:00+08:60')).toBeNull()
    // 时区只能跟在时间后面
    expect(parseDateTime('2026-10-02Z')).toBeNull()
  })

  it('returns null for empty or unrecognised values', () => {
    expect(parseDateTime(null)).toBeNull()
    expect(parseDateTime(undefined)).toBeNull()
    expect(parseDateTime('')).toBeNull()
    expect(parseDateTime('tomorrow')).toBeNull()
    expect(parseDateTime('2026/10/02 09:00')).toBeNull()
    expect(parseDateTime('20261002')).toBeNull()
    expect(parseDateTime('2026-10-02 09:00:00T09:00:00')).toBeNull()
  })
})

describe('normalizeDateTime', () => {
  it('outputs YYYY-MM-DD HH:MM:SS byte-for-byte', () => {
    const out = normalizeDateTime('2026-10-02T09:05')
    expect(out).toBe('2026-10-02 09:05:00')
    expect(out).toMatch(/^\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}$/)
  })

  it('normalises every accepted input shape', () => {
    expect(normalizeDateTime('2026-10-02 09:05:07')).toBe('2026-10-02 09:05:07')
    expect(normalizeDateTime('2026-10-02T09:05:07.999')).toBe('2026-10-02 09:05:07')
    expect(normalizeDateTime('2026-10-02T01:05:07Z')).toBe('2026-10-02 09:05:07')
    expect(normalizeDateTime('2026-10-02', 'end')).toBe('2026-10-02 23:59:00')
    expect(normalizeDateTime('2026-10-02', 'notify')).toBe('2026-10-02 09:00:00')
  })

  it('is idempotent', () => {
    const once = normalizeDateTime('2026-10-01T22:30:00-02:00')
    expect(once).toBe('2026-10-02 08:30:00')
    expect(normalizeDateTime(once)).toBe(once)
  })

  it('returns null for invalid input', () => {
    expect(normalizeDateTime('not a date')).toBeNull()
    expect(normalizeDateTime(null)).toBeNull()
  })
})

describe('composeDateTime', () => {
  it('combines picker values into the storage format', () => {
    expect(composeDateTime('2026-10-02', '14:30', 'notify')).toBe('2026-10-02 14:30:00')
    expect(composeDateTime('2026-10-02', '14:30:15', 'start')).toBe('2026-10-02 14:30:15')
  })

  it('uses the default time of day when only a date is picked', () => {
    expect(composeDateTime('2026-10-02', null, 'start')).toBe('2026-10-02 00:00:00')
    expect(composeDateTime('2026-10-02', '', 'end')).toBe('2026-10-02 23:59:00')
    expect(composeDateTime('2026-10-02', undefined, 'notify')).toBe('2026-10-02 09:00:00')
  })

  it('returns null without a date', () => {
    expect(composeDateTime(null, '14:30', 'notify')).toBeNull()
    expect(composeDateTime('', null, 'start')).toBeNull()
  })
})

describe('splitDateTime', () => {
  it('splits stored values into picker values', () => {
    expect(splitDateTime('2026-10-02 14:30:59')).toEqual({ date: '2026-10-02', time: '14:30' })
    expect(splitDateTime('2026-10-02T14:30')).toEqual({ date: '2026-10-02', time: '14:30' })
  })

  it('never produces the doubled "…T09:00:00" shape from space-separated values', () => {
    const parts = splitDateTime('2026-10-02 09:00:00', 'notify')
    expect(parts).toEqual({ date: '2026-10-02', time: '09:00' })
    expect(composeDateTime(parts?.date, parts?.time, 'notify')).toBe('2026-10-02 09:00:00')
  })

  it('applies the field default for date-only values', () => {
    expect(splitDateTime('2026-10-02', 'end')).toEqual({ date: '2026-10-02', time: '23:59' })
  })

  it('converts offsets to local time before splitting', () => {
    expect(splitDateTime('2026-10-01T20:00:00Z')).toEqual({ date: '2026-10-02', time: '04:00' })
  })

  it('returns null for invalid input', () => {
    expect(splitDateTime('garbage')).toBeNull()
    expect(splitDateTime(null)).toBeNull()
  })
})

describe('date keys and display formatting', () => {
  it('formats local date keys', () => {
    expect(formatDateKey(new Date(2026, 0, 5, 23, 59))).toBe('2026-01-05')
    expect(toDateKey('2026-10-02 23:59:59')).toBe('2026-10-02')
    expect(toDateKey('2026-10-01T16:30:00Z')).toBe('2026-10-02')
    expect(toDateKey('2026-10-02', 'end')).toBe('2026-10-02')
    expect(toDateKey('nope')).toBeNull()
  })

  it('formats for display with a dayjs pattern', () => {
    expect(formatDateTime('2026-10-02 09:05:00')).toBe('2026-10-02 09:05')
    expect(formatDateTime('2026-10-02T09:05:00', 'MM-DD HH:mm')).toBe('10-02 09:05')
    expect(formatDateTime('2026-10-02', 'MM-DD HH:mm', 'notify')).toBe('10-02 09:00')
    expect(formatDateTime('invalid')).toBeNull()
    expect(formatDateTime(null)).toBeNull()
  })

  it('formats a Date in the storage format', () => {
    expect(formatStorageDateTime(new Date(2026, 9, 2, 7, 8, 9))).toBe('2026-10-02 07:08:09')
  })
})
