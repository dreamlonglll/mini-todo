<script setup lang="ts">
import { ref, watch, onMounted, onBeforeUnmount } from 'vue'
import { invoke, convertFileSrc } from '@tauri-apps/api/core'
import { Editor, rootCtx, defaultValueCtx, editorViewOptionsCtx } from '@milkdown/kit/core'
import { commonmark } from '@milkdown/kit/preset/commonmark'
import { gfm } from '@milkdown/kit/preset/gfm'
import { listener, listenerCtx } from '@milkdown/kit/plugin/listener'
import { upload, uploadConfig } from '@milkdown/kit/plugin/upload'
import { clipboard } from '@milkdown/kit/plugin/clipboard'
import { Decoration } from '@milkdown/kit/prose/view'
import { nord } from '@milkdown/theme-nord'
import { replaceAll } from '@milkdown/kit/utils'
import type { Node, Schema } from '@milkdown/kit/prose/model'
import type { Uploader, UploadOptions } from '@milkdown/kit/plugin/upload'
import '@milkdown/theme-nord/style.css'
import { handleFileLinkClick } from '@/utils/fileLink'
import { revealItemInDir } from '@tauri-apps/plugin-opener'

const props = defineProps<{
  modelValue: string
  readonly?: boolean
}>()

const emit = defineEmits<{
  (e: 'update:modelValue', value: string): void
}>()

const editorContainer = ref<HTMLDivElement | null>(null)
let editorInstance: Editor | null = null
// 编辑器内部当前内容，用于区分外部赋值与用户输入，避免 watch 回环
let internalContent = ''
// 初始化代次：create() 是异步的，readonly 切换重建/组件卸载可能与进行中的 create 竞争，
// 代次不匹配时丢弃过期实例，避免孤儿编辑器泄漏
let initSeq = 0

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

async function imageUploader(files: FileList, schema: Schema): Promise<Node[]> {
  const images: File[] = []
  for (let i = 0; i < files.length; i++) {
    const file = files.item(i)
    if (file && file.type.includes('image')) {
      images.push(file)
    }
  }

  const nodes: Node[] = await Promise.all(
    images.map(async (image) => {
      const arrayBuffer = await image.arrayBuffer()
      const uint8 = new Uint8Array(arrayBuffer)
      let binary = ''
      for (let i = 0; i < uint8.length; i++) {
        binary += String.fromCharCode(uint8[i])
      }
      const base64 = btoa(binary)

      const ext = image.name.split('.').pop() || 'png'
      const fileName = `${Date.now()}_${Math.random().toString(36).slice(2, 8)}.${ext}`

      const filePath = await invoke<string>('save_subtask_image', {
        imageData: base64,
        fileName,
      })

      const src = convertFileSrc(filePath)
      return schema.nodes.image.createAndFill({
        src,
        alt: image.name,
      }) as Node
    })
  )

  return nodes
}

async function initEditor() {
  if (!editorContainer.value) return

  const seq = ++initSeq
  // 固定 create 期间使用的初始内容，create 完成后与 internalContent 比对补同步
  const contentAtInit = internalContent

  const builder = Editor.make()
    .config(nord)
    .config((ctx) => {
      ctx.set(rootCtx, editorContainer.value!)
      ctx.set(defaultValueCtx, contentAtInit)

      const fileLinkDOMHandler = {
        click: (_view: unknown, event: Event) => {
          const target = (event.target as HTMLElement)?.closest('a') as HTMLAnchorElement | null
          if (!target) return false
          const href = target.getAttribute('href') || ''
          if (!href.startsWith('file:///')) return false
          event.preventDefault()
          let path = decodeURIComponent(href.slice(8)).split('#')[0].replace(/\//g, '\\')
          if (path) revealItemInDir(path).catch(console.error)
          return true
        },
      }

      if (props.readonly) {
        ctx.update(editorViewOptionsCtx, (prev) => ({
          ...prev,
          editable: () => false,
          handleDOMEvents: { ...prev.handleDOMEvents, ...fileLinkDOMHandler },
        }))
      } else {
        ctx.update(editorViewOptionsCtx, (prev) => ({
          ...prev,
          handleDOMEvents: { ...prev.handleDOMEvents, ...fileLinkDOMHandler },
        }))
        ctx.get(listenerCtx).markdownUpdated((_ctx, markdown, prevMarkdown) => {
          if (markdown !== prevMarkdown) {
            internalContent = markdown
            emit('update:modelValue', markdown)
          }
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
    instance.action(replaceAll(internalContent))
  }
}

function destroyEditor() {
  initSeq++
  if (editorInstance) {
    editorInstance.destroy()
    editorInstance = null
  }
}

// 外部赋值（如异步加载完成）时同步到编辑器
watch(() => props.modelValue, (value) => {
  const next = value ?? ''
  if (next === internalContent) return
  internalContent = next
  editorInstance?.action(replaceAll(next))
})

// readonly 切换需要重建编辑器（listener/upload 插件仅编辑模式注册）
watch(() => props.readonly, async () => {
  destroyEditor()
  await initEditor()
})

onMounted(async () => {
  internalContent = props.modelValue ?? ''
  await initEditor()
  editorContainer.value?.addEventListener('click', handleImageClick)
  editorContainer.value?.addEventListener('click', handleFileLinkClick)
})

onBeforeUnmount(() => {
  editorContainer.value?.removeEventListener('click', handleImageClick)
  editorContainer.value?.removeEventListener('click', handleFileLinkClick)
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
