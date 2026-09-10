import { revealItemInDir, openUrl } from '@tauri-apps/plugin-opener'
import type { MarkedExtension } from 'marked'

// ProseMirror 的 handleDOMEvents 与容器上的 click 监听器会先后收到同一个事件，
// 打标记保证一次点击只打开一次（否则外部链接会开出两个浏览器标签页）
type LinkClickEvent = MouseEvent & { __linkHandled?: boolean }

function extractFilePath(href: string): string | null {
  if (!href.startsWith('file:///')) return null

  let path = decodeURIComponent(href.slice(8)) // strip "file:///"
  path = path.split('#')[0]
  path = path.replace(/\//g, '\\')
  return path || null
}

function consume(event: LinkClickEvent) {
  event.__linkHandled = true
  event.preventDefault()
  event.stopPropagation()
}

/**
 * Handles click events on container elements: file:/// links are revealed in
 * the system file explorer, http(s) links open in the default browser.
 *
 * `readonly` 表示当前处于详情预览排版（contenteditable=false）——此时点击链接
 * 唯一合理的语义就是打开它，且不拦截的话 WebView 会被整个导航到外部站点。
 * 编辑排版下普通点击要留给编辑器定位光标，只有 Ctrl/Cmd+点击（浏览器里真正
 * 会触发跳转的操作）才接管。
 *
 * 返回 true 表示这次点击已被消费。
 */
export function handleLinkClick(event: MouseEvent, options?: { readonly?: boolean }): boolean {
  const evt = event as LinkClickEvent
  if (evt.__linkHandled) return true

  const target = event.target as HTMLElement
  if (!target) return false

  const anchor = target.closest('a[data-file-link]') as HTMLAnchorElement | null
    ?? target.closest('a') as HTMLAnchorElement | null
  if (!anchor) return false

  const href = anchor.getAttribute('data-file-path')
    || anchor.getAttribute('href')
    || ''

  const filePath = extractFilePath(href)
  if (filePath) {
    consume(evt)
    revealItemInDir(filePath).catch((e) => {
      console.error('Failed to reveal file:', e)
    })
    return true
  }

  if (/^https?:\/\//i.test(href)) {
    if (!options?.readonly && !event.ctrlKey && !event.metaKey) return false

    consume(evt)
    openUrl(href).catch((e) => {
      console.error('Failed to open url:', e)
    })
    return true
  }

  return false
}

/**
 * Marked extension that auto-links bare file:/// URLs in text.
 */
export const fileLinkExtension: MarkedExtension = {
  extensions: [{
    name: 'fileLink',
    level: 'inline',
    start(src: string) {
      return src.indexOf('file:///')
    },
    tokenizer(src: string) {
      const match = src.match(/^file:\/\/\/[^\s<>)"']+/)
      if (match) {
        return {
          type: 'fileLink',
          raw: match[0],
          href: match[0],
        }
      }
      return undefined
    },
    renderer(token) {
      const href = (token as Record<string, string>).href || ''
      const display = decodeURIComponent(href.replace('file:///', ''))
      return `<a href="${href}" data-file-link="true" data-file-path="${href}" class="file-link" title="在资源管理器中打开">${display}</a>`
    },
  }],
}
