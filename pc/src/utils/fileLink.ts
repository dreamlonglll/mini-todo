import { revealItemInDir, openUrl } from '@tauri-apps/plugin-opener'

// ProseMirror 的 handleDOMEvents 与容器上的 click 监听器会先后收到同一个事件，
// 打标记保证一次点击只处理一次（否则外部链接会开出两个浏览器标签页）
type LinkClickEvent = MouseEvent & { __linkHandled?: boolean }

const IS_WINDOWS = /windows/i.test(navigator.userAgent)

/** 链接的去向：按协议白名单判定，白名单外一律不处理 */
export type LinkTarget =
  /** http / https / mailto：交给系统默认程序 */
  | { kind: 'external'; url: string }
  /** file:///：在资源管理器中定位 */
  | { kind: 'file'; path: string }
  /** javascript: / data: / vbscript: / 相对地址 / UNC 等：什么都不做 */
  | { kind: 'blocked' }

/**
 * file:/// 链接 → 本机绝对路径；不是本机绝对路径时返回 null
 *
 * 只接受三斜杠形式。`file://host/share`、`file://///host/share` 这类 UNC 路径一律拒绝：
 * 交给 revealItemInDir 会让系统去连 SMB 共享（可能泄露 NTLM 凭据）
 */
export function extractFilePath(href: string, windows = IS_WINDOWS): string | null {
  if (!/^file:\/\/\//i.test(href)) return null

  // 先去掉查询串与片段再解码，避免把编码过的 %23 / %3F 误当分隔符
  const raw = href.slice('file:///'.length).split(/[?#]/)[0]
  let rest: string
  try {
    rest = decodeURIComponent(raw)
  } catch {
    return null
  }

  if (windows) {
    const path = rest.replace(/\//g, '\\')
    // 必须是盘符开头的绝对路径（顺带排除 \\server\share 与 \\?\ 前缀）
    return /^[A-Za-z]:\\/.test(path) ? path : null
  }
  const path = `/${rest}`
  return path.startsWith('//') ? null : path
}

/** 按协议白名单解析链接（纯函数） */
export function resolveLinkTarget(href: string, windows = IS_WINDOWS): LinkTarget {
  const raw = href.trim()
  if (!raw) return { kind: 'blocked' }

  let url: URL
  try {
    // 与浏览器导航同一套解析规则：大小写、内嵌的制表符 / 换行都会被规范化，
    // 不会被 "JaVa\tScript:" 之类的写法绕过；相对地址没有 base 会直接解析失败
    url = new URL(raw)
  } catch {
    return { kind: 'blocked' }
  }

  switch (url.protocol) {
    case 'http:':
    case 'https:':
    case 'mailto:':
      return { kind: 'external', url: url.href }
    case 'file:': {
      const path = extractFilePath(raw, windows)
      return path ? { kind: 'file', path } : { kind: 'blocked' }
    }
    default:
      return { kind: 'blocked' }
  }
}

/**
 * 处理 Markdown 容器内的链接点击：任何 <a> 都不允许让 WebView 自己导航，
 * 去向由协议白名单决定（http/https/mailto 走系统默认程序，file:/// 在资源管理器中定位，
 * 其余如 javascript:、data: 一律忽略）。
 *
 * `readonly` 表示当前处于详情预览排版（contenteditable=false）——此时点击链接
 * 唯一合理的语义就是打开它。编辑排版下普通点击要留给编辑器定位光标，只有
 * Ctrl/Cmd+点击才打开外部链接（file:/// 链接沿用原行为，单击即定位）。
 *
 * 链接 DOM 的 href 会被 Milkdown 按其安全协议表清洗（file: 也会被清空），
 * 原始地址由 MarkdownEditor 另存在 data-href 上，这里优先读取它。
 *
 * 返回 true 表示这次点击已被消费。
 */
export function handleLinkClick(event: MouseEvent, options?: { readonly?: boolean }): boolean {
  const evt = event as LinkClickEvent
  if (evt.__linkHandled) return true

  const target = event.target as Element | null
  const anchor = target?.closest?.('a')
  if (!anchor) return false

  // 先兜底：无论下面怎么判定，都不让 WebView 跟随这个链接导航
  event.preventDefault()
  evt.__linkHandled = true

  const href = anchor.getAttribute('data-href') ?? anchor.getAttribute('href') ?? ''
  const link = resolveLinkTarget(href)

  if (link.kind === 'file') {
    event.stopPropagation()
    revealItemInDir(link.path).catch((e) => {
      console.error('Failed to reveal file:', e)
    })
    return true
  }

  if (link.kind === 'external') {
    // 编辑排版：普通点击只定位光标（光标在 mousedown 时已放好，这里不必额外处理）
    if (!options?.readonly && !event.ctrlKey && !event.metaKey) return false

    event.stopPropagation()
    openUrl(link.url).catch((e) => {
      console.error('Failed to open url:', e)
    })
    return true
  }

  return true
}

/** 中键点击链接默认会让 WebView 新开窗口导航，一并拦掉 */
export function preventLinkAuxClick(event: MouseEvent): void {
  const target = event.target as Element | null
  if (target?.closest?.('a')) event.preventDefault()
}
