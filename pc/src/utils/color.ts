/**
 * HEX 转 RGB 分量字符串：`#3B82F6` → `59, 130, 246`
 *
 * 供 `rgba(var(--app-bg-rgb), var(--app-bg-alpha))` 这类写法使用——
 * CSS 变量存分量而非完整颜色，alpha 才能独立调节。
 * 非法输入回落到黑色，避免写出 `rgba(, 0.5)` 这种无效值。
 */
/** HEX 颜色比较：取色器可能写回小写，预设色是大写 */
export function isSameColor(a: string, b: string): boolean {
  return a.trim().toLowerCase() === b.trim().toLowerCase()
}

export function hexToRgbChannels(hex: string): string {
  const normalized = hex.trim().replace(/^#/, '')
  const full =
    normalized.length === 3
      ? normalized.split('').map(c => c + c).join('')
      : normalized

  if (!/^[0-9a-fA-F]{6}$/.test(full)) return '0, 0, 0'

  const int = parseInt(full, 16)
  return `${(int >> 16) & 255}, ${(int >> 8) & 255}, ${int & 255}`
}
