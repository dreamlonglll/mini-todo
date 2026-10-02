<script setup lang="ts">
import { ref, watch, onMounted, onBeforeUnmount } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { ElMessage } from '@/plugins/element'
import {
  Editor,
  rootCtx,
  defaultValueCtx,
  editorViewCtx,
  editorViewOptionsCtx,
  serializerCtx,
} from '@milkdown/kit/core'
import { commonmark, linkAttr } from '@milkdown/kit/preset/commonmark'
import { gfm } from '@milkdown/kit/preset/gfm'
import { listener, listenerCtx } from '@milkdown/kit/plugin/listener'
import { upload, uploadConfig } from '@milkdown/kit/plugin/upload'
import { clipboard } from '@milkdown/kit/plugin/clipboard'
import { Decoration } from '@milkdown/kit/prose/view'
import { nord } from '@milkdown/theme-nord'
import { replaceAll } from '@milkdown/kit/utils'
import type { Mark, Node, Schema } from '@milkdown/kit/prose/model'
import type { Uploader, UploadOptions } from '@milkdown/kit/plugin/upload'
import '@milkdown/theme-nord/style.css'
import { handleLinkClick, preventLinkAuxClick } from '@/utils/fileLink'
import {
  MAX_IMAGE_BYTES,
  getImagesDir,
  resolveImageExtension,
  toAssetUrl,
  toDisplayMarkdown,
  toStorageMarkdown,
} from '@/utils/imageRef'
import { notifyError } from '@/utils/notify'

// modelValue / update:modelValue 一律是存储形式（图片引用为 minitodo-image://<name>，见 utils/imageRef）；
// 编辑器内部渲染的是本机 asset URL，进出编辑器时各转换一次
const props = defineProps<{
  modelValue: string
  readonly?: boolean
}>()

const emit = defineEmits<{
  (e: 'update:modelValue', value: string): void
}>()

const editorContainer = ref<HTMLDivElement | null>(null)
let editorInstance: Editor | null = null
// 编辑器内部当前内容（存储形式），用于区分外部赋值与用户输入，避免 watch 回环
let internalContent = ''
// 本机 images 目录：把存储形式的图片引用换成可显示的 asset URL
let imagesDir: string | null = null
// 初始化代次：create() 是异步的，readonly 切换重建/组件卸载可能与进行中的 create 竞争，
// 代次不匹配时丢弃过期实例，避免孤儿编辑器泄漏
let initSeq = 0
// 外部赋值写进编辑器（replaceAll）后，listener 会在防抖后把这份内容"规范化"后的 Markdown
// （补末尾换行、统一列表符号等）当作更新回报。那只是父组件给的内容换了个写法，不是用户输入：
// emit 回去会让内容一加载完就被当成已修改（Esc 关闭误报"未保存的修改"），导入的原文也会被悄悄改写。
// 这里记下回声的样子，markdownUpdated 收到一模一样的内容时忽略
let echoMarkdown: string | null = null

// 把存储形式的内容写进编辑器（外部赋值 / 创建期间错过的赋值），并记下它的回声
function setEditorContent(instance: Editor, storage: string) {
  instance.action((ctx) => {
    replaceAll(toDisplayMarkdown(storage, imagesDir))(ctx)
    // 只读排版不注册 listener，没有回声，也省掉一次序列化（描述弹窗的预览随输入频繁刷新）
    echoMarkdown = props.readonly ? null : ctx.get(serializerCtx)(ctx.get(editorViewCtx).state.doc)
  })
}

// 图片预览
const previewVisible = ref(false)
const previewUrls = ref<string[]>([])
const previewInitialIndex = ref(0)

function handleImageClick(e: MouseEvent) {
  const target = e.target as HTMLElement
  if (target.tagName !== 'IMG') return

  const imgSrc = (target as HTMLImageElement).src
  if (!imgSrc) return

  e.preventDefault()
  e.stopPropagation()

  const container = editorContainer.value
  if (!container) return

  const allImages = Array.from(container.querySelectorAll('.ProseMirror img'))
  const urls = allImages.map(img => (img as HTMLImageElement).src).filter(Boolean)

  if (urls.length === 0) return

  previewUrls.value = urls
  previewInitialIndex.value = Math.max(0, urls.indexOf(imgSrc))
  previewVisible.value = true
}

// 单张图片落盘：原始字节走 raw body IPC（不再逐字节拼 base64 + JSON），
// 文件名由后端生成；失败只跳过这一张，不影响同批其它图片
async function uploadImage(image: File, schema: Schema): Promise<Node | null> {
  const ext = resolveImageExtension(image)
  if (!ext) {
    ElMessage.warning(`不支持的图片格式：${image.name || image.type}（支持 png / jpg / webp / gif / bmp）`)
    return null
  }
  if (image.size > MAX_IMAGE_BYTES) {
    ElMessage.warning(`图片超过 20MB，未插入：${image.name}`)
    return null
  }

  try {
    const bytes = new Uint8Array(await image.arrayBuffer())
    const filePath = await invoke<string>('save_subtask_image', bytes, {
      headers: { 'x-image-ext': ext },
    })
    // 编辑器里放渲染形式（本机 asset URL），序列化输出时由 toStorageMarkdown 换回规范引用
    return schema.nodes.image.createAndFill({
      src: toAssetUrl(filePath),
      alt: image.name,
    })
  } catch (e) {
    notifyError(e, '图片保存失败')
    return null
  }
}

async function imageUploader(files: FileList, schema: Schema): Promise<Node[]> {
  const images = Array.from(files).filter(file => file.type.startsWith('image/'))
  const nodes = await Promise.all(images.map(image => uploadImage(image, schema)))
  return nodes.filter((node): node is Node => node !== null)
}

// 链接点击一律由协议白名单决定去向，WebView 自身永不导航（见 utils/fileLink）
function onLinkClick(event: MouseEvent): boolean {
  return handleLinkClick(event, { readonly: props.readonly === true })
}

async function initEditor() {
  const seq = ++initSeq
  // 先拿到本机 images 目录再建编辑器，首帧就能显示图片（getImagesDir 每个窗口只查询一次）。
  // 首次挂载与 readonly 切换都走这里：查询期间被重建 / 卸载时由更新的那次接手
  if (imagesDir === null) {
    imagesDir = await getImagesDir()
    if (seq !== initSeq) return
  }
  // await 期间组件可能已卸载：模板 ref 置空后直接返回
  if (!editorContainer.value) return
  // 首次挂载与 readonly 切换的初始化可能交错：同一容器里只保留一个编辑器实例
  if (editorInstance) {
    editorInstance.destroy()
    editorInstance = null
  }
  echoMarkdown = null

  // 固定 create 期间使用的初始内容，create 完成后与 internalContent 比对补同步
  const contentAtInit = internalContent

  const builder = Editor.make()
    .config(nord)
    .config((ctx) => {
      ctx.set(rootCtx, editorContainer.value!)
      ctx.set(defaultValueCtx, toDisplayMarkdown(contentAtInit, imagesDir))

      // Milkdown 渲染链接时会按自己的安全协议表清洗 href（file: 也会被清成空串），
      // 原始地址另存到 data-href，点击时由 handleLinkClick 按白名单处理
      ctx.set(linkAttr.key, (mark: Mark) => ({ 'data-href': mark.attrs.href }))

      const linkDOMHandler = {
        click: (_view: unknown, event: Event) => onLinkClick(event as MouseEvent),
      }

      if (props.readonly) {
        ctx.update(editorViewOptionsCtx, (prev) => ({
          ...prev,
          editable: () => false,
          handleDOMEvents: { ...prev.handleDOMEvents, ...linkDOMHandler },
        }))
      } else {
        ctx.update(editorViewOptionsCtx, (prev) => ({
          ...prev,
          handleDOMEvents: { ...prev.handleDOMEvents, ...linkDOMHandler },
        }))
        ctx.get(listenerCtx).markdownUpdated((_ctx, markdown, prevMarkdown) => {
          if (markdown === prevMarkdown) return
          // 外部赋值的回声（见 echoMarkdown）：内容与父组件给的相同，只是写法被规范化
          const echo = echoMarkdown
          echoMarkdown = null
          if (markdown === echo) return
          // 对外只暴露存储形式；比较也基于存储形式，外部回写同一内容时不会触发 replaceAll
          const storage = toStorageMarkdown(markdown)
          if (storage === internalContent) return
          internalContent = storage
          emit('update:modelValue', storage)
        })
        ctx.set(uploadConfig.key, {
          uploader: imageUploader as Uploader,
          enableHtmlFileUploader: true,
          uploadWidgetFactory: (pos, spec) => Decoration.widget(pos, document.createElement('span'), spec),
        } satisfies UploadOptions)
      }
    })
    .use(commonmark)
    .use(gfm)

  if (!props.readonly) {
    // clipboard：粘贴的 Markdown 源码解析为富文本，而非按字面文本插入后被转义
    builder.use(listener).use(upload).use(clipboard)
  }

  const instance = await builder.create()
  if (seq !== initSeq) {
    // create 期间已被销毁/重建（readonly 切换或卸载），丢弃过期实例
    instance.destroy()
    return
  }
  editorInstance = instance

  // create 期间外部可能已更新 modelValue（如父组件异步加载完成），
  // 此时 watch 里的 replaceAll 因 editorInstance 尚为 null 被跳过，这里补一次同步
  if (internalContent !== contentAtInit) {
    setEditorContent(instance, internalContent)
  }
}

function destroyEditor() {
  initSeq++
  if (editorInstance) {
    editorInstance.destroy()
    editorInstance = null
  }
}

// 外部赋值（如异步加载完成）时同步到编辑器；比较基于存储形式，旧数据里的
// asset URL 与规范引用视为同一内容
watch(() => props.modelValue, (value) => {
  const next = toStorageMarkdown(value ?? '')
  if (next === internalContent) return
  internalContent = next
  if (editorInstance) setEditorContent(editorInstance, next)
})

// readonly 切换需要重建编辑器（listener/upload 插件仅编辑模式注册）
watch(() => props.readonly, async () => {
  destroyEditor()
  await initEditor()
})

onMounted(async () => {
  internalContent = toStorageMarkdown(props.modelValue ?? '')
  await initEditor()
  editorContainer.value?.addEventListener('click', handleImageClick)
  editorContainer.value?.addEventListener('click', onLinkClick)
  editorContainer.value?.addEventListener('auxclick', preventLinkAuxClick)
})

onBeforeUnmount(() => {
  editorContainer.value?.removeEventListener('click', handleImageClick)
  editorContainer.value?.removeEventListener('click', onLinkClick)
  editorContainer.value?.removeEventListener('auxclick', preventLinkAuxClick)
  destroyEditor()
})
</script>

<template>
  <div class="markdown-editor" :class="{ 'is-readonly': readonly }">
    <div ref="editorContainer" class="markdown-editor-container"></div>

    <!-- 图片预览 -->
    <el-image-viewer
      v-if="previewVisible"
      :url-list="previewUrls"
      :initial-index="previewInitialIndex"
      :z-index="10000"
      @close="previewVisible = false"
    />
  </div>
</template>

<style scoped>
.markdown-editor {
  display: flex;
  flex-direction: column;
  min-height: inherit;
}

.markdown-editor-container {
  flex: 1;
  min-height: inherit;
  display: flex;
  flex-direction: column;
}

.markdown-editor-container :deep(.milkdown) {
  flex: 1;
  padding: 12px 16px;
}

.markdown-editor-container :deep(.editor) {
  outline: none;
}

.markdown-editor-container :deep(.ProseMirror) {
  outline: none;
}

.markdown-editor-container :deep(.ProseMirror p) {
  margin: 0.4em 0;
  line-height: 1.6;
}

.markdown-editor-container :deep(.ProseMirror h1),
.markdown-editor-container :deep(.ProseMirror h2),
.markdown-editor-container :deep(.ProseMirror h3) {
  margin: 0.6em 0 0.3em;
}

.markdown-editor-container :deep(.ProseMirror img) {
  max-width: 100%;
  height: auto;
  border-radius: 6px;
  margin: 8px 0;
  cursor: pointer;
  transition: opacity 0.15s ease;

  &:hover {
    opacity: 0.85;
    box-shadow: 0 2px 8px rgba(0, 0, 0, 0.15);
  }
}

.markdown-editor-container :deep(.ProseMirror code) {
  background: #f1f5f9;
  padding: 2px 6px;
  border-radius: 4px;
  font-size: 0.9em;
}

.markdown-editor-container :deep(.ProseMirror pre) {
  background: #1e293b;
  color: #e2e8f0;
  padding: 12px 16px;
  border-radius: 8px;
  overflow-x: auto;
}

.markdown-editor-container :deep(.ProseMirror pre code) {
  background: transparent;
  padding: 0;
  border-radius: 0;
  font-size: inherit;
  color: inherit;
}

.markdown-editor-container :deep(.ProseMirror blockquote) {
  border-left: 3px solid #3b82f6;
  padding-left: 12px;
  color: #64748b;
  margin: 0.5em 0;
}

.markdown-editor-container :deep(.ProseMirror ul),
.markdown-editor-container :deep(.ProseMirror ol) {
  padding-left: 24px;
  margin: 0.4em 0;
}

.markdown-editor-container :deep(.ProseMirror hr) {
  border: none;
  border-top: 1px solid #e2e8f0;
  margin: 1em 0;
}

/* GFM 表格 */
.markdown-editor-container :deep(.ProseMirror table) {
  border-collapse: collapse;
  margin: 0.6em 0;
  width: 100%;
}

.markdown-editor-container :deep(.ProseMirror th),
.markdown-editor-container :deep(.ProseMirror td) {
  border: 1px solid #e2e8f0;
  padding: 6px 10px;
  text-align: left;
}

.markdown-editor-container :deep(.ProseMirror th) {
  background: #f8fafc;
  font-weight: 600;
}

/* GFM 任务清单（gfm 渲染为 li[data-item-type="task"]，无原生 checkbox，用伪元素绘制） */
.markdown-editor-container :deep(.ProseMirror li[data-item-type='task']) {
  list-style: none;
  position: relative;
}

.markdown-editor-container :deep(.ProseMirror li[data-item-type='task'])::before {
  content: '';
  position: absolute;
  left: -20px;
  top: 0.4em;
  width: 13px;
  height: 13px;
  box-sizing: border-box;
  border: 1.5px solid #94a3b8;
  border-radius: 3px;
  background: #ffffff;
}

.markdown-editor-container :deep(.ProseMirror li[data-item-type='task'][data-checked='true'])::before {
  background: #3b82f6;
  border-color: #3b82f6;
}

.markdown-editor-container :deep(.ProseMirror li[data-item-type='task'][data-checked='true'])::after {
  content: '';
  position: absolute;
  left: -15px;
  top: 0.52em;
  width: 3px;
  height: 6px;
  border: solid #ffffff;
  border-width: 0 1.5px 1.5px 0;
  transform: rotate(45deg);
}
</style>
