import { QUADRANT_INFO, DEFAULT_COLOR } from '@/types'
import type { QuadrantType } from '@/types'

/** 象限对应的默认颜色 */
export function getQuadrantColor(quadrant: QuadrantType): string {
  return QUADRANT_INFO.find(q => q.id === quadrant)?.color ?? DEFAULT_COLOR
}

/** HEX 颜色比较：取色器可能写回小写，预设色是大写 */
function isSameColor(a: string, b: string): boolean {
  return a.trim().toLowerCase() === b.trim().toLowerCase()
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
