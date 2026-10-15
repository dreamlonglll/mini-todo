import { describe, expect, it } from 'vitest'
import { FENCE_ICON_SIZE_MAX, FENCE_ICON_SIZE_MIN, FENCE_ICON_SIZE_STEP, FENCE_TITLE_MAX_LENGTH } from '@/types'
import type { FenceItemOpResult } from '@/types'
import {
  baseName,
  clampIconSize,
  describeDeleteKept,
  describeMoveIn,
  exceedsDragThreshold,
  GeometryCommitGate,
  iconBucket,
  iconRequestPx,
  nextIconSize,
  normalizeFenceTitle,
  parentDir,
  parseFenceId,
  resizeRect,
  summarizeOpResults,
} from './fence'

describe('parseFenceId', () => {
  it('accepts positive integers only', () => {
    expect(parseFenceId('12')).toBe(12)
    expect(parseFenceId(['7', '8'])).toBe(7)
    for (const bad of [undefined, null, '', '0', '-1', '1.5', 'abc', '12abc', ' 3', 3, '99999999999999999999']) {
      expect(parseFenceId(bad)).toBeNull()
    }
  })
})

describe('normalizeFenceTitle', () => {
  it('trims and replaces control characters like the backend', () => {
    expect(normalizeFenceTitle('  工作  ')).toBe('工作')
    expect(normalizeFenceTitle('a\nb\tc')).toBe('a b c')
  })

  it('rejects empty and over-long titles', () => {
    expect(normalizeFenceTitle('   ')).toBeNull()
    expect(normalizeFenceTitle('\n\t')).toBeNull()
    expect(normalizeFenceTitle('字'.repeat(FENCE_TITLE_MAX_LENGTH))).toBe('字'.repeat(FENCE_TITLE_MAX_LENGTH))
    expect(normalizeFenceTitle('字'.repeat(FENCE_TITLE_MAX_LENGTH + 1))).toBeNull()
  })
})

describe('exceedsDragThreshold', () => {
  it('treats tiny movements as clicks', () => {
    expect(exceedsDragThreshold(0, 0)).toBe(false)
    expect(exceedsDragThreshold(3, -3)).toBe(false)
    expect(exceedsDragThreshold(4, 0)).toBe(true)
    expect(exceedsDragThreshold(0, -4)).toBe(true)
  })
})

describe('GeometryCommitGate', () => {
  it('ignores moves that the user did not start', () => {
    const gate = new GeometryCommitGate(1000)
    expect(gate.accept(0)).toBe(false)
    expect(gate.isArmed(0)).toBe(false)
  })

  it('accepts and extends while the user keeps dragging', () => {
    const gate = new GeometryCommitGate(1000)
    gate.arm(0)
    expect(gate.accept(900)).toBe(true)
    // 每个事件续期：拖得再久也不会中途失效
    expect(gate.accept(1800)).toBe(true)
    expect(gate.isArmed(2700)).toBe(true)
    expect(gate.accept(2900)).toBe(false)
  })

  it('expires when a press never turned into a drag and closes on disarm', () => {
    const gate = new GeometryCommitGate(1000)
    gate.arm(0)
    expect(gate.accept(1001)).toBe(false)
    gate.arm(2000)
    gate.disarm()
    expect(gate.accept(2001)).toBe(false)
  })
})

describe('resizeRect', () => {
  const start = { x: 100, y: 200, width: 552, height: 504 }

  it('moves only the dragged edges', () => {
    expect(resizeRect(start, 'se', 30, 40, 240, 240)).toEqual({ x: 100, y: 200, width: 582, height: 544 })
    expect(resizeRect(start, 'e', 30, 40, 240, 240)).toEqual({ x: 100, y: 200, width: 582, height: 504 })
    expect(resizeRect(start, 's', 30, 40, 240, 240)).toEqual({ x: 100, y: 200, width: 552, height: 544 })
    // 左 / 上边：右下角不动
    expect(resizeRect(start, 'nw', -20, -10, 240, 240)).toEqual({ x: 80, y: 190, width: 572, height: 514 })
    expect(resizeRect(start, 'ne', 20, 10, 240, 240)).toEqual({ x: 100, y: 210, width: 572, height: 494 })
    expect(resizeRect(start, 'sw', 20, 10, 240, 240)).toEqual({ x: 120, y: 200, width: 532, height: 514 })
  })

  it('stops at the minimum size without pushing the opposite edge', () => {
    expect(resizeRect(start, 'se', -1000, -1000, 240, 240)).toEqual({ x: 100, y: 200, width: 240, height: 240 })
    // 拖左 / 上边缩到最小：右下角仍在原处
    const r = resizeRect(start, 'nw', 1000, 1000, 240, 240)
    expect(r).toEqual({ x: 100 + 552 - 240, y: 200 + 504 - 240, width: 240, height: 240 })
  })

  it('rounds fractional deltas from high-DPI pointers', () => {
    expect(resizeRect(start, 'e', 10.4, 0, 240, 240).width).toBe(562)
  })
})

describe('icon sizes', () => {
  it('rounds the requested physical size up to a cache bucket like the backend', () => {
    expect(iconBucket(0)).toBe(16)
    expect(iconBucket(48)).toBe(48)
    expect(iconBucket(49)).toBe(64)
    expect(iconBucket(72)).toBe(96)
    expect(iconBucket(1000)).toBe(256)
  })

  it('requests icon size times devicePixelRatio, rounded up', () => {
    expect(iconRequestPx(48, 1.5)).toBe(72)
    expect(iconRequestPx(48, 1.25)).toBe(60)
    expect(iconRequestPx(40, 1.75)).toBe(70)
    expect(iconRequestPx(48, 0)).toBe(48)
  })

  it('steps with Ctrl+wheel and stays within the limits', () => {
    expect(nextIconSize(48, -100)).toBe(48 + FENCE_ICON_SIZE_STEP)
    expect(nextIconSize(48, 100)).toBe(48 - FENCE_ICON_SIZE_STEP)
    expect(nextIconSize(48, 0)).toBe(48)
    expect(nextIconSize(FENCE_ICON_SIZE_MAX, -100)).toBe(FENCE_ICON_SIZE_MAX)
    expect(nextIconSize(FENCE_ICON_SIZE_MIN, 100)).toBe(FENCE_ICON_SIZE_MIN)
    expect(clampIconSize(1000)).toBe(FENCE_ICON_SIZE_MAX)
    expect(clampIconSize(-5)).toBe(FENCE_ICON_SIZE_MIN)
  })
})

describe('paths', () => {
  it('splits Windows and POSIX paths', () => {
    expect(baseName('C:\\Users\\me\\Desktop\\Chrome.lnk')).toBe('Chrome.lnk')
    expect(baseName('C:\\Users\\me\\Desktop\\文件夹\\')).toBe('文件夹')
    expect(baseName('/home/me/a.txt')).toBe('a.txt')
    expect(baseName('a.txt')).toBe('a.txt')
    expect(parentDir('C:\\Users\\me\\Desktop\\Chrome.lnk')).toBe('C:\\Users\\me\\Desktop')
    expect(parentDir('/home/me/a.txt')).toBe('/home/me')
  })
})

describe('delete kept', () => {
  const S = String.fromCharCode(92)
  const failure = (name: string, error: string): FenceItemOpResult => ({
    source: ['C:', 'root', '箱', name].join(S),
    status: 'failed',
    fileName: null,
    destPath: null,
    error,
  })

  it('lists what stayed behind with reasons', () => {
    expect(
      describeDeleteKept({
        deleted: false,
        restored: 1,
        remaining: ['b.txt', 'C.lnk', 'hidden.tmp'],
        failures: [failure('b.txt', '正被其他程序占用'), { ...failure('c.lnk', '用户取消了操作'), status: 'cancelled' }],
      })
    ).toBe('3 个项目未能移回桌面，收纳箱已保留：b.txt（正被其他程序占用）、C.lnk（用户取消了操作） 等')
  })

  it('falls back to failures when nothing is listed as remaining', () => {
    expect(
      describeDeleteKept({ deleted: false, restored: 0, remaining: [], failures: [failure('a.lnk', '没有权限')] })
    ).toBe('1 个项目未能移回桌面，收纳箱已保留：a.lnk（没有权限）')
  })

  it('explains when every item moved back but the folder itself could not be removed', () => {
    expect(describeDeleteKept({ deleted: false, restored: 3, remaining: [], failures: [] })).toBe(
      '3 个项目已移回桌面，但收纳文件夹未能删除（可能被占用），收纳箱已保留，可稍后再删除'
    )
    // 空收纳箱（没有项目要移回）
    expect(describeDeleteKept({ deleted: false, restored: 0, remaining: [], failures: [] })).toBe(
      '收纳文件夹未能删除（可能被占用），收纳箱已保留，可稍后再删除'
    )
  })
})

describe('move-in results', () => {
  const result = (name: string, status: FenceItemOpResult['status'], error: string | null = null): FenceItemOpResult => ({
    source: `C:\\Users\\me\\Desktop\\${name}`,
    status,
    fileName: status === 'moved' ? name : null,
    destPath: null,
    error,
  })

  it('stays silent when everything moved or was skipped', () => {
    const summary = summarizeOpResults([result('a.lnk', 'moved'), result('desktop.ini', 'skipped', '系统文件')])
    expect(summary).toEqual({ moved: 1, cancelled: 0, failed: [], notes: [] })
    expect(describeMoveIn(summary)).toBeNull()
  })

  it('reports cancellations as info and failures with reasons', () => {
    expect(describeMoveIn(summarizeOpResults([result('a.lnk', 'cancelled', '用户取消了操作')]))).toEqual({
      type: 'info',
      message: '已取消 1 项',
    })
    const failed = describeMoveIn(
      summarizeOpResults([
        result('a.lnk', 'failed', '正被其他程序占用'),
        result('b.lnk', 'failed', '不存在'),
        result('c.lnk', 'failed', null),
      ])
    )
    expect(failed?.type).toBe('error')
    expect(failed?.message).toBe('3 项未能收纳：a.lnk（正被其他程序占用）、b.lnk（不存在） 等')
  })

  it('mentions what did move when the batch is mixed', () => {
    const mixed = describeMoveIn(
      summarizeOpResults([
        result('a.lnk', 'moved'),
        result('b.lnk', 'cancelled', '用户取消了操作'),
        result('c.lnk', 'failed', '没有权限'),
      ])
    )
    expect(mixed).toEqual({ type: 'warning', message: '已收纳 1 项；1 项未能收纳：c.lnk（没有权限）；已取消 1 项' })
    const noted = describeMoveIn(summarizeOpResults([result('a.lnk', 'moved', '源文件未能删除')]))
    expect(noted).toEqual({ type: 'warning', message: '已收纳 1 项；a.lnk：源文件未能删除' })
  })
})
