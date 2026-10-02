import { QUADRANT_INFO, DEFAULT_COLOR } from '@/types'
import type { QuadrantType } from '@/types'
import { isSameColor } from './color'

/** 象限对应的默认颜色 */
export function getQuadrantColor(quadrant: QuadrantType): string {
  return QUADRANT_INFO.find(q => q.id === quadrant)?.color ?? DEFAULT_COLOR
}

/**
 * 切换象限时解析待办应有的颜色。
 *
 * 当前颜色仍是旧象限的默认色 → 视为用户没自定义过，跟随新象限；
 * 用户手动挑过颜色则保留，不被象限覆盖。
 */
export function resolveQuadrantColor(
  currentColor: string,
  prevQuadrant: QuadrantType,
  nextQuadrant: QuadrantType
): string {
  return isSameColor(currentColor, getQuadrantColor(prevQuadrant))
    ? getQuadrantColor(nextQuadrant)
    : currentColor
}

/**
 * 把某个象限内的新相对顺序合并回全局（列表视图）顺序。
 *
 * 全局顺序里属于该象限的那些"槽位"按新顺序依次填入，其它象限的待办原地不动；
 * 这样四象限里拖拽只改变本象限内部的先后，不会把列表视图的整体顺序打乱。
 * 象限顺序里出现、全局顺序里却没有的 id 追加在末尾（理论上不会发生，兜底防丢）。
 */
export function mergeQuadrantOrder(globalIds: number[], quadrantIds: number[]): number[] {
  const inQuadrant = new Set(quadrantIds)
  const queue = [...quadrantIds]
  const merged = globalIds.map(id => (inQuadrant.has(id) ? (queue.shift() as number) : id))
  return queue.length > 0 ? [...merged, ...queue] : merged
}
