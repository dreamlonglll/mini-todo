import { beforeEach, describe, expect, it, vi } from 'vitest'

// convertFileSrc 依赖 Tauri 注入的 window.__TAURI_INTERNALS__：按 Windows 上的真实输出模拟
// （http://asset.localhost/ + encodeURIComponent(绝对路径)）。invoke 用 hoisted 的同一个 mock，
// resetModules 后重新求值工厂也拿到同一个实例
const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }))
vi.mock('@tauri-apps/api/core', () => ({
  convertFileSrc: (path: string) => `http://asset.localhost/${encodeURIComponent(path)}`,
  invoke: invokeMock,
}))

import {
  IMAGE_REF_SCHEME,
  isSafeImageName,
  resolveImageExtension,
  toDisplayMarkdown,
  toImageRef,
  toStorageMarkdown,
} from './imageRef'

const WIN_DIR = 'C:\\Users\\bob\\AppData\\Local\\mini-todo\\images'
const MAC_DIR = '/Users/bob/Library/Application Support/mini-todo/images'

/** 旧版本存进 Markdown 的图片地址：convertFileSrc(本机绝对路径) */
function legacyWindowsUrl(dir: string, name: string, origin = 'http://asset.localhost'): string {
  return `${origin}/${encodeURIComponent(`${dir}\\${name}`)}`
}

describe('isSafeImageName', () => {
  it('accepts plain single path segments', () => {
    for (const name of ['a', 'img_1.png', '1700000000000_ab12cd.png', 'img_1727-x.JPEG', 'A'.repeat(128)]) {
      expect(isSafeImageName(name), name).toBe(true)
    }
  })

  it('rejects traversal, separators, odd characters and overlong names', () => {
    for (const name of [
      '',
      '.hidden.png',
      '-x.png',
      '../x.png',
      'a..b.png',
      'a/b.png',
      'a\\b.png',
      'a b.png',
      'a%2Fb.png',
      '图片.png',
      'A'.repeat(129),
    ]) {
      expect(isSafeImageName(name), name).toBe(false)
    }
  })
})

describe('toStorageMarkdown', () => {
  it('keeps canonical references unchanged (idempotent)', () => {
    const md = `![a](${IMAGE_REF_SCHEME}1700_a.png)\n\ntext ![b](minitodo-image://b.webp)`
    expect(toStorageMarkdown(md)).toBe(md)
    expect(toStorageMarkdown(toStorageMarkdown(md))).toBe(md)
  })

  it('rewrites legacy Windows asset URLs whose parent directory is images', () => {
    const md = `before ![shot](${legacyWindowsUrl(WIN_DIR, '1700_abc.png')}) after`
    expect(toStorageMarkdown(md)).toBe('before ![shot](minitodo-image://1700_abc.png) after')
  })

  it('handles https asset origins and asset://localhost (macOS / Linux)', () => {
    expect(toStorageMarkdown(`![x](${legacyWindowsUrl(WIN_DIR, 'x.png', 'https://asset.localhost')})`)).toBe(
      '![x](minitodo-image://x.png)'
    )
    const mac = `asset://localhost/${encodeURIComponent(`${MAC_DIR}/y.gif`)}`
    expect(toStorageMarkdown(`![y](${mac})`)).toBe('![y](minitodo-image://y.gif)')
    // 未编码的 / 分隔符同样识别
    expect(toStorageMarkdown('![z](asset://localhost/home/bob/.local/share/mini-todo/images/z.png)')).toBe(
      '![z](minitodo-image://z.png)'
    )
  })

  it('rewrites every image in a document, including other users / devices', () => {
    const other = legacyWindowsUrl('D:\\Data\\alice\\mini-todo\\images', '2.png')
    const md = `![1](${legacyWindowsUrl(WIN_DIR, '1.png')})\n![2](${other})\n![3](minitodo-image://3.png)`
    expect(toStorageMarkdown(md)).toBe(
      '![1](minitodo-image://1.png)\n![2](minitodo-image://2.png)\n![3](minitodo-image://3.png)'
    )
  })

  it('leaves unrelated or unsafe URLs untouched', () => {
    const cases = [
      // 父目录不是 images
      `![p](${legacyWindowsUrl('C:\\Users\\bob\\Pictures', 'p.png')})`,
      // 不是 asset 协议
      '![w](https://example.com/images/w.png)',
      // 文件名不安全
      `![u](http://asset.localhost/${encodeURIComponent(`${WIN_DIR}\\..\\secret.png`)})`,
      '![u](minitodo-image://../secret.png)',
      // 普通链接与文本
      '[link](https://example.com) and `minitodo-image` in code',
    ]
    for (const md of cases) expect(toStorageMarkdown(md), md).toBe(md)
  })

  it('returns empty input as-is', () => {
    expect(toStorageMarkdown('')).toBe('')
  })
})

describe('toDisplayMarkdown', () => {
  it('maps canonical references to the local images directory', () => {
    expect(toDisplayMarkdown('![a](minitodo-image://a.png)', WIN_DIR)).toBe(
      `![a](http://asset.localhost/${encodeURIComponent(`${WIN_DIR}\\a.png`)})`
    )
    expect(toDisplayMarkdown('![a](minitodo-image://a.png)', MAC_DIR)).toBe(
      `![a](http://asset.localhost/${encodeURIComponent(`${MAC_DIR}/a.png`)})`
    )
  })

  it('does not double a trailing separator', () => {
    expect(toDisplayMarkdown('![a](minitodo-image://a.png)', `${WIN_DIR}\\`)).toBe(
      `![a](http://asset.localhost/${encodeURIComponent(`${WIN_DIR}\\a.png`)})`
    )
  })

  it("remaps legacy URLs from another device's path to this device", () => {
    const foreign = legacyWindowsUrl('D:\\Data\\alice\\mini-todo\\images', 'f.png')
    expect(toDisplayMarkdown(`![f](${foreign})`, WIN_DIR)).toBe(
      `![f](http://asset.localhost/${encodeURIComponent(`${WIN_DIR}\\f.png`)})`
    )
  })

  it('is reversed by toStorageMarkdown', () => {
    const md = '# t\n\n![a](minitodo-image://a.png) text ![b](minitodo-image://b.jpg)'
    expect(toStorageMarkdown(toDisplayMarkdown(md, WIN_DIR))).toBe(md)
    expect(toStorageMarkdown(toDisplayMarkdown(md, MAC_DIR))).toBe(md)
  })

  it('leaves content untouched without an images directory', () => {
    const md = '![a](minitodo-image://a.png)'
    expect(toDisplayMarkdown(md, null)).toBe(md)
  })
})

describe('toImageRef', () => {
  it('builds the canonical reference', () => {
    expect(toImageRef('a.png')).toBe('minitodo-image://a.png')
  })
})

describe('resolveImageExtension', () => {
  it('prefers the MIME type (pasted screenshots have generic names)', () => {
    expect(resolveImageExtension({ name: 'image.png', type: 'image/jpeg' })).toBe('jpg')
    expect(resolveImageExtension({ name: 'clip', type: 'IMAGE/PNG' })).toBe('png')
    expect(resolveImageExtension({ name: 'x', type: 'image/x-ms-bmp' })).toBe('bmp')
  })

  it('falls back to the lower-cased file extension', () => {
    expect(resolveImageExtension({ name: 'Photo.JPEG', type: '' })).toBe('jpeg')
    expect(resolveImageExtension({ name: 'a.b.webp', type: 'application/octet-stream' })).toBe('webp')
  })

  it('rejects types outside the whitelist', () => {
    expect(resolveImageExtension({ name: 'vector.svg', type: 'image/svg+xml' })).toBeNull()
    expect(resolveImageExtension({ name: 'noext', type: '' })).toBeNull()
    expect(resolveImageExtension({ name: 'doc.tiff', type: 'image/tiff' })).toBeNull()
  })
})

describe('getImagesDir', () => {
  // 模块级缓存：每个用例重新加载模块，从干净状态开始
  beforeEach(() => {
    vi.resetModules()
    invokeMock.mockReset()
  })

  it('queries the backend once per window', async () => {
    invokeMock.mockResolvedValue(WIN_DIR)
    const { getImagesDir } = await import('./imageRef')
    expect(await getImagesDir()).toBe(WIN_DIR)
    expect(await getImagesDir()).toBe(WIN_DIR)
    expect(invokeMock).toHaveBeenCalledTimes(1)
    expect(invokeMock).toHaveBeenCalledWith('get_images_dir')
  })

  it('returns null on failure and retries on the next call', async () => {
    const consoleError = vi.spyOn(console, 'error').mockImplementation(() => {})
    invokeMock.mockRejectedValueOnce(new Error('boom')).mockResolvedValueOnce(WIN_DIR)
    const { getImagesDir } = await import('./imageRef')
    expect(await getImagesDir()).toBeNull()
    expect(await getImagesDir()).toBe(WIN_DIR)
    expect(invokeMock).toHaveBeenCalledTimes(2)
    consoleError.mockRestore()
  })
})
