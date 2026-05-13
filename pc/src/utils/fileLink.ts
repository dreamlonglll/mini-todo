import { revealItemInDir } from '@tauri-apps/plugin-opener'
import type { MarkedExtension } from 'marked'

function extractFilePath(href: string): string | null {
  if (!href.startsWith('file:///')) return null

  let path = decodeURIComponent(href.slice(8)) // strip "file:///"
  path = path.split('#')[0]
  path = path.replace(/\//g, '\\')
  return path || null
}

/**
 * Handles click events on container elements, intercepting file:/// links
 * and opening them in the system file explorer.
 */
export function handleFileLinkClick(event: MouseEvent) {
  const target = event.target as HTMLElement
  if (!target) return

  const anchor = target.closest('a[data-file-link]') as HTMLAnchorElement | null
    ?? target.closest('a') as HTMLAnchorElement | null

  if (anchor) {
    const href = anchor.getAttribute('data-file-path')
      || anchor.getAttribute('href')
      || ''
    const filePath = extractFilePath(href)
    if (!filePath) return

    event.preventDefault()
    event.stopPropagation()
    revealItemInDir(filePath).catch((e) => {
      console.error('Failed to reveal file:', e)
    })
  }
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
