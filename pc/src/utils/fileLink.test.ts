import { beforeEach, describe, expect, it, vi } from 'vitest'

const { openUrlMock, revealMock } = vi.hoisted(() => ({
  openUrlMock: vi.fn((_url: string) => Promise.resolve()),
  revealMock: vi.fn((_path: string) => Promise.resolve()),
}))
vi.mock('@tauri-apps/plugin-opener', () => ({
  openUrl: openUrlMock,
  revealItemInDir: revealMock,
}))

import { extractFilePath, handleLinkClick, resolveLinkTarget } from './fileLink'

describe('extractFilePath', () => {
  it('maps file:/// URLs to Windows drive paths', () => {
    expect(extractFilePath('file:///C:/Users/bob/notes.txt', true)).toBe('C:\\Users\\bob\\notes.txt')
    expect(extractFilePath('FILE:///d:/a%20b/c.md', true)).toBe('d:\\a b\\c.md')
  })

  it('strips query and fragment before decoding', () => {
    expect(extractFilePath('file:///C:/a/b.txt?x=1#frag', true)).toBe('C:\\a\\b.txt')
    // 编码过的 # / ? 属于文件名
    expect(extractFilePath('file:///C:/a/%23b%3F.txt', true)).toBe('C:\\a\\#b?.txt')
  })

  it('rejects UNC and non-absolute Windows paths', () => {
    expect(extractFilePath('file://server/share/a.txt', true)).toBeNull()
    expect(extractFilePath('file://///server/share/a.txt', true)).toBeNull()
    expect(extractFilePath('file:////server/share/a.txt', true)).toBeNull()
    expect(extractFilePath('file:///relative/a.txt', true)).toBeNull()
    expect(extractFilePath('file:///%5C%5C?%5CC:%5Ca.txt', true)).toBeNull()
  })

  it('maps file:/// URLs to POSIX paths and rejects network paths', () => {
    expect(extractFilePath('file:///home/bob/a%20b.txt', false)).toBe('/home/bob/a b.txt')
    expect(extractFilePath('file:////server/share', false)).toBeNull()
  })

  it('rejects malformed percent-encoding and other schemes', () => {
    expect(extractFilePath('file:///C:/a%E0%A4%A.txt', true)).toBeNull()
    expect(extractFilePath('https://example.com/a', true)).toBeNull()
    expect(extractFilePath('C:\\a.txt', true)).toBeNull()
  })
})

describe('resolveLinkTarget', () => {
  it('sends http / https / mailto to the system handler', () => {
    expect(resolveLinkTarget('https://example.com/a?b=1')).toEqual({
      kind: 'external',
      url: 'https://example.com/a?b=1',
    })
    expect(resolveLinkTarget('HTTP://Example.com')).toEqual({ kind: 'external', url: 'http://example.com/' })
    expect(resolveLinkTarget('mailto:bob@example.com')).toEqual({
      kind: 'external',
      url: 'mailto:bob@example.com',
    })
    expect(resolveLinkTarget('  https://example.com/a b  ')).toEqual({
      kind: 'external',
      url: 'https://example.com/a%20b',
    })
  })

  it('reveals local files only for file:/// paths', () => {
    expect(resolveLinkTarget('file:///C:/a/b.txt', true)).toEqual({ kind: 'file', path: 'C:\\a\\b.txt' })
    expect(resolveLinkTarget('file:///home/bob/b.txt', false)).toEqual({ kind: 'file', path: '/home/bob/b.txt' })
    expect(resolveLinkTarget('file://server/share/x', true)).toEqual({ kind: 'blocked' })
  })

  it('blocks script-capable and unknown schemes, including obfuscated spellings', () => {
    for (const href of [
      'javascript:alert(1)',
      'JaVaScRiPt:alert(1)',
      'java\tscript:alert(1)',
      ' javascript:alert(1)',
      'java\nscript:alert(1)',
      'data:text/html,<script>alert(1)</script>',
      'vbscript:msgbox(1)',
      'blob:https://example.com/uuid',
      'ftp://example.com/a',
      'tauri://localhost',
      'asset://localhost/C%3A%5Ca.txt',
    ]) {
      expect(resolveLinkTarget(href), JSON.stringify(href)).toEqual({ kind: 'blocked' })
    }
  })

  it('blocks relative and empty hrefs', () => {
    for (const href of ['', '   ', '#anchor', './a.md', '/abs/path', 'example.com']) {
      expect(resolveLinkTarget(href), JSON.stringify(href)).toEqual({ kind: 'blocked' })
    }
  })
})

/** 最小化的点击事件替身：只实现 handleLinkClick 用到的成员 */
function linkClick(
  attrs: Record<string, string>,
  modifiers: { ctrlKey?: boolean; metaKey?: boolean } = {}
) {
  const anchor = { getAttribute: (name: string) => attrs[name] ?? null }
  return {
    target: { closest: (selector: string) => (selector === 'a' ? anchor : null) },
    ctrlKey: modifiers.ctrlKey ?? false,
    metaKey: modifiers.metaKey ?? false,
    preventDefault: vi.fn(),
    stopPropagation: vi.fn(),
  }
}

function click(event: ReturnType<typeof linkClick>, options?: { readonly?: boolean }): boolean {
  return handleLinkClick(event as unknown as MouseEvent, options)
}

describe('handleLinkClick', () => {
  beforeEach(() => {
    openUrlMock.mockClear()
    revealMock.mockClear()
  })

  it('ignores clicks outside links', () => {
    const event = {
      target: { closest: () => null },
      preventDefault: vi.fn(),
      stopPropagation: vi.fn(),
    }
    expect(handleLinkClick(event as unknown as MouseEvent)).toBe(false)
    expect(event.preventDefault).not.toHaveBeenCalled()
  })

  it('never lets the WebView navigate to a blocked link', () => {
    const event = linkClick({ href: 'javascript:alert(1)' })
    expect(click(event, { readonly: true })).toBe(true)
    expect(event.preventDefault).toHaveBeenCalled()
    expect(openUrlMock).not.toHaveBeenCalled()
    expect(revealMock).not.toHaveBeenCalled()
  })

  it('opens external links on plain click in read-only mode', () => {
    const event = linkClick({ href: 'https://example.com' })
    expect(click(event, { readonly: true })).toBe(true)
    expect(openUrlMock).toHaveBeenCalledWith('https://example.com/')
  })

  it('requires Ctrl/Cmd+click to open external links while editing', () => {
    const plain = linkClick({ href: 'https://example.com' })
    expect(click(plain)).toBe(false)
    expect(plain.preventDefault).toHaveBeenCalled()
    expect(openUrlMock).not.toHaveBeenCalled()

    expect(click(linkClick({ href: 'https://example.com' }, { ctrlKey: true }))).toBe(true)
    expect(click(linkClick({ href: 'https://example.com' }, { metaKey: true }))).toBe(true)
    expect(openUrlMock).toHaveBeenCalledTimes(2)
  })

  it('prefers the original data-href over the sanitised href', () => {
    const event = linkClick({ 'data-href': 'file:///home/bob/a.txt', href: '' })
    expect(click(event)).toBe(true)
    expect(revealMock).toHaveBeenCalledWith('/home/bob/a.txt')
  })

  it('handles the same event only once (editor and container listeners both see it)', () => {
    const event = linkClick({ href: 'https://example.com' })
    expect(click(event, { readonly: true })).toBe(true)
    expect(click(event, { readonly: true })).toBe(true)
    expect(openUrlMock).toHaveBeenCalledTimes(1)
  })
})
