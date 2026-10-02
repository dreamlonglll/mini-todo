/**
 * 编辑类子窗口（待办编辑 / 子任务编辑）的键盘快捷键：Esc 关闭、Ctrl/Cmd+Enter 保存
 *
 * 监听挂在 window 的冒泡阶段，晚于输入框、编辑器（ProseMirror）、Element Plus 组件自己的处理：
 * - 已被处理（defaultPrevented）的按键不再响应，如代码块里的 Ctrl+Enter（ProseMirror 用来跳出代码块）
 * - 输入法组合输入中的按键属于输入法（回车上屏、Esc 取消候选）
 * - 有弹出层打开时（下拉、日期 / 颜色选择器、对话框、确认框、图片预览、消息提示），Esc 先交给弹出层关闭
 *
 * 与 main.ts 生产环境的快捷键拦截不冲突：那里只拦 F5 / F12 与 Ctrl+R/U/S/P… 等浏览器组合键，
 * 不含 Esc 与 Ctrl+Enter
 */

export type EditorShortcut = 'close' | 'submit'

type ShortcutKeyEvent = Pick<
  KeyboardEvent,
  'key' | 'ctrlKey' | 'metaKey' | 'altKey' | 'shiftKey' | 'isComposing' | 'keyCode'
>

/**
 * 输入法组合输入中（如中文拼音用回车上屏）。部分 WebView 只给出 keyCode 229 而不置 isComposing
 */
export function isImeComposing(e: Pick<KeyboardEvent, 'isComposing' | 'keyCode'>): boolean {
  return e.isComposing || e.keyCode === 229
}

/** 按键 → 快捷键（纯函数）；不是快捷键返回 null */
export function resolveEditorShortcut(e: ShortcutKeyEvent): EditorShortcut | null {
  if (isImeComposing(e) || e.altKey || e.shiftKey) return null
  const mod = e.ctrlKey || e.metaKey
  if (e.key === 'Escape' && !mod) return 'close'
  if (e.key === 'Enter' && mod) return 'submit'
  return null
}

function isRendered(el: Element): boolean {
  return el.getClientRects().length > 0
}

/**
 * 页面上是否有打开的弹出层
 *
 * Element Plus 的下拉 / 选择器弹层在打开时 aria-hidden="false"；对话框、确认框的遮罩
 * 关闭后是 display:none 或被移除。弹出层在同一次 keydown 里自己处理 Esc 时 DOM 还没更新，
 * 这里看到的仍是打开状态，正好让窗口级的快捷键让路。
 *
 * @param includeMessages 是否把 ElMessage 提示也算进来：它同样响应 Esc（先关提示），
 *   但不该挡住 Ctrl+Enter（保存失败的提示还在时用户正要重试）
 */
export function hasOpenOverlay(includeMessages = false): boolean {
  if (document.querySelector('.el-popper[aria-hidden="false"]')) return true
  const selector = includeMessages
    ? '.el-overlay, .el-image-viewer__wrapper, .el-message'
    : '.el-overlay, .el-image-viewer__wrapper'
  return Array.from(document.querySelectorAll(selector)).some(isRendered)
}

export interface EditorShortcutHandlers {
  /** Esc：关闭窗口（有未保存的修改时可先确认） */
  onClose: () => void
  /** Ctrl/Cmd+Enter：保存 */
  onSubmit: () => void
  /** 返回 false 时忽略快捷键，如本窗口打开的子窗口仍在编辑 */
  enabled?: () => boolean
}

/** 绑定快捷键，返回解绑函数（在 onBeforeUnmount 里调用） */
export function bindEditorShortcuts(handlers: EditorShortcutHandlers): () => void {
  const onKeydown = (e: KeyboardEvent) => {
    if (e.defaultPrevented || e.repeat) return
    const shortcut = resolveEditorShortcut(e)
    if (!shortcut) return
    if (handlers.enabled && !handlers.enabled()) return
    if (hasOpenOverlay(shortcut === 'close')) return
    e.preventDefault()
    if (shortcut === 'close') {
      handlers.onClose()
    } else {
      handlers.onSubmit()
    }
  }
  window.addEventListener('keydown', onKeydown)
  return () => window.removeEventListener('keydown', onKeydown)
}
