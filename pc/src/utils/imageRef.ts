/**
 * Markdown 图片引用（跨端契约 K5）
 *
 * 规范形式：`minitodo-image://<name>`，<name> 是本机 images 目录下的安全文件名。
 * 数据库、导出文件与 WebDAV 同步数据里只存规范形式，云端 / AI 通过 `GET /images/<name>` 取图。
 *
 * - 渲染（toDisplayMarkdown）：换成本机 `convertFileSrc(<images 目录>/<name>)`（括号另行编码，见 toAssetUrl）
 * - 保存（toStorageMarkdown）：父目录名为 `images` 的 asset URL 一律换回规范形式
 *
 * 兼容旧数据：旧版本直接存 `convertFileSrc(本机绝对路径)`，即
 * `http(s)://asset.localhost/<url 编码的绝对路径>`（Windows）或 `asset://localhost/<…>`（macOS / Linux）。
 * 绝对路径里含用户名，换台设备就裂图；这里按文件名映射到本机 images 目录，保存时顺带写回规范形式。
 */
import { convertFileSrc, invoke } from '@tauri-apps/api/core'

export const IMAGE_REF_SCHEME = 'minitodo-image://'

/** 编辑器允许上传的图片扩展名（大小写不敏感，存小写） */
export const IMAGE_EXTENSIONS = ['png', 'jpg', 'jpeg', 'webp', 'gif', 'bmp'] as const

/** 单张图片上限 20MB（后端同样校验） */
export const MAX_IMAGE_BYTES = 20 * 1024 * 1024

// 安全文件名：单个普通路径段，不含 ".."（同步上传 / 下载 / 列举时不满足的一律跳过）
const SAFE_NAME = '[A-Za-z0-9][A-Za-z0-9._-]{0,127}'
const SAFE_NAME_RE = new RegExp(`^${SAFE_NAME}$`)

// 名字后面紧跟的字符不能再是名字字符或百分号编码（否则说明文件名还没结束，或含有不安全字符）
const NAME_END = '(?![A-Za-z0-9._%-])'
// convertFileSrc 用 encodeURIComponent 编码路径，分隔符是 %5C（\）或 %2F（/），也容忍未编码的 /
const SEP = '(?:%5[Cc]|%2[Ff]|/)'
// 编码后路径里可能出现的字符（encodeURIComponent 不编码 !~*'()），外加 Markdown 序列化时
// 可能出现的 \( \) 转义；不含空白、引号、方括号与冒号，匹配不会越过当前 URL
const PATH_CHAR = `(?:[A-Za-z0-9\\-_.!~*'()/]|\\\\[()]|%[0-9A-Fa-f]{2})`

// 规范形式 或 父目录为 images 的 asset URL（第 1 组 / 第 2 组为文件名）
const IMAGE_REF_RE = new RegExp(
  `minitodo-image://(${SAFE_NAME})${NAME_END}` +
    `|(?:https?://asset\\.localhost|asset://localhost)/${PATH_CHAR}*?${SEP}images${SEP}(${SAFE_NAME})${NAME_END}`,
  'g'
)

/** 是否为允许落盘 / 同步的安全图片文件名 */
export function isSafeImageName(name: string): boolean {
  return SAFE_NAME_RE.test(name) && !name.includes('..')
}

/** 规范引用 `minitodo-image://<name>` */
export function toImageRef(name: string): string {
  return `${IMAGE_REF_SCHEME}${name}`
}

function joinPath(dir: string, name: string): string {
  const sep = dir.includes('\\') && !dir.includes('/') ? '\\' : '/'
  return dir.endsWith('\\') || dir.endsWith('/') ? `${dir}${name}` : `${dir}${sep}${name}`
}

/**
 * 本机文件 → 可放进 Markdown 的 asset URL
 *
 * convertFileSrc 用 encodeURIComponent 编码路径，而 ( ) 不在它的编码范围内：路径里有
 * 不成对的括号（如用户名 "bob("、"a)b"）时，原样拼进 `![](...)` 会让链接目标被截断或整个
 * 图片语法失效，用户一编辑，序列化结果就把图片引用转义成普通文本。asset 协议按百分号解码
 * 路径，这里把括号也编码掉，拼进 Markdown 的一定是合法的链接目标
 */
export function toAssetUrl(path: string): string {
  return convertFileSrc(path).replace(/[()]/g, (c) => (c === '(' ? '%28' : '%29'))
}

function replaceImageRefs(markdown: string, replacer: (name: string) => string): string {
  return markdown.replace(
    IMAGE_REF_RE,
    (whole: string, canonicalName?: string, assetName?: string) => {
      const name = canonicalName ?? assetName
      return name && isSafeImageName(name) ? replacer(name) : whole
    }
  )
}

/**
 * 存储形式 → 渲染形式：规范引用与旧 asset URL 都换成本机 images 目录的 asset URL
 *
 * @param imagesDir `get_images_dir` 返回的本机目录；拿不到时原样返回（图片不显示，但不改数据）
 */
export function toDisplayMarkdown(markdown: string, imagesDir: string | null): string {
  if (!markdown || !imagesDir) return markdown
  return replaceImageRefs(markdown, (name) => toAssetUrl(joinPath(imagesDir, name)))
}

/** 渲染形式 / 旧数据 → 存储形式：父目录为 images 的 asset URL 换回规范引用（幂等） */
export function toStorageMarkdown(markdown: string): string {
  if (!markdown) return markdown
  return replaceImageRefs(markdown, toImageRef)
}

let imagesDirPromise: Promise<string | null> | null = null

/** 本机 images 目录（每个窗口只查询一次；失败时下次调用重试） */
export function getImagesDir(): Promise<string | null> {
  if (!imagesDirPromise) {
    imagesDirPromise = invoke<string>('get_images_dir').catch((e) => {
      console.error('Failed to get images dir:', e)
      imagesDirPromise = null
      return null
    })
  }
  return imagesDirPromise
}

const MIME_EXTENSIONS: Record<string, string> = {
  'image/png': 'png',
  'image/jpeg': 'jpg',
  'image/jpg': 'jpg',
  'image/pjpeg': 'jpg',
  'image/webp': 'webp',
  'image/gif': 'gif',
  'image/bmp': 'bmp',
  'image/x-ms-bmp': 'bmp',
}

/**
 * 推断上传图片的扩展名（小写）：优先 MIME（粘贴的截图文件名往往是通用的 image.png），
 * 其次文件名；不在白名单内返回 null
 */
export function resolveImageExtension(file: { name: string; type: string }): string | null {
  const fromMime = MIME_EXTENSIONS[file.type.toLowerCase()]
  if (fromMime) return fromMime
  const dot = file.name.lastIndexOf('.')
  if (dot < 0) return null
  const ext = file.name.slice(dot + 1).toLowerCase()
  return (IMAGE_EXTENSIONS as readonly string[]).includes(ext) ? ext : null
}
