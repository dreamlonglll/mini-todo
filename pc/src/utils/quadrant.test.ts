import { describe, expect, it } from 'vitest'
import { QUADRANTS } from '@/types'
import { getQuadrantColor, mergeQuadrantOrder, resolveQuadrantColor } from './quadrant'

describe('mergeQuadrantOrder', () => {
  it('reorders only the slots that belong to the quadrant', () => {
    // 象限成员 2、4 互换，其它象限的 1、3、5 原地不动
    expect(mergeQuadrantOrder([1, 2, 3, 4, 5], [4, 2])).toEqual([1, 4, 3, 2, 5])
  })

  it('keeps the global order when the quadrant order is unchanged', () => {
    expect(mergeQuadrantOrder([1, 2, 3, 4, 5], [2, 4])).toEqual([1, 2, 3, 4, 5])
  })

  it('places an item dragged in from another quadrant into the quadrant slots', () => {
    // 5 从别的象限拖进来，落在 2 与 4 之间：象限成员 {2,4,5} 原本占第 2、4、5 个位置
    expect(mergeQuadrantOrder([1, 2, 3, 4, 5], [2, 5, 4])).toEqual([1, 2, 3, 5, 4])
    // 拖到最前
    expect(mergeQuadrantOrder([1, 2, 3, 4, 5], [5, 2, 4])).toEqual([1, 5, 3, 2, 4])
  })

  it('appends ids missing from the global order instead of dropping them', () => {
    expect(mergeQuadrantOrder([1, 2, 3], [3, 9])).toEqual([1, 2, 3, 9])
  })

  it('leaves the global order alone for an empty quadrant', () => {
    expect(mergeQuadrantOrder([1, 2, 3], [])).toEqual([1, 2, 3])
    expect(mergeQuadrantOrder([], [])).toEqual([])
  })

  it('preserves every id exactly once', () => {
    const global = [10, 20, 30, 40, 50, 60]
    const merged = mergeQuadrantOrder(global, [60, 20, 40])
    expect([...merged].sort((a, b) => a - b)).toEqual(global)
    expect(merged).toEqual([10, 60, 30, 20, 50, 40])
  })
})

describe('quadrant colours', () => {
  it('maps each quadrant to its default colour', () => {
    expect(getQuadrantColor(QUADRANTS.IMPORTANT_URGENT)).toBe('#EF4444')
    expect(getQuadrantColor(QUADRANTS.NOT_URGENT_NOT_IMPORTANT)).toBe('#10B981')
  })

  it('follows the new quadrant while the colour is still the old default', () => {
    expect(
      resolveQuadrantColor('#EF4444', QUADRANTS.IMPORTANT_URGENT, QUADRANTS.IMPORTANT_NOT_URGENT)
    ).toBe('#F59E0B')
    // 取色器写回的小写同样视为默认色
    expect(
      resolveQuadrantColor('#ef4444', QUADRANTS.IMPORTANT_URGENT, QUADRANTS.URGENT_NOT_IMPORTANT)
    ).toBe('#3B82F6')
  })

  it('keeps a colour the user picked by hand', () => {
    expect(
      resolveQuadrantColor('#8B5CF6', QUADRANTS.IMPORTANT_URGENT, QUADRANTS.IMPORTANT_NOT_URGENT)
    ).toBe('#8B5CF6')
  })
})
