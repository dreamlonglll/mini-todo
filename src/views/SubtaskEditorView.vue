<script setup lang="ts">
import { ref, onMounted, onBeforeUnmount, nextTick } from 'vue'
import { useRoute } from 'vue-router'
import { invoke } from '@tauri-apps/api/core'
import { convertFileSrc } from '@tauri-apps/api/core'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { Editor, rootCtx, defaultValueCtx, editorViewOptionsCtx } from '@milkdown/kit/core'
import { commonmark } from '@milkdown/kit/preset/commonmark'
import { listener, listenerCtx } from '@milkdown/kit/plugin/listener'
import { upload, uploadConfig } from '@milkdown/kit/plugin/upload'
import { Decoration } from '@milkdown/kit/prose/view'
import { nord } from '@milkdown/theme-nord'
import type { Node } from '@milkdown/kit/prose/model'
import type { Uploader, UploadOptions } from '@milkdown/kit/plugin/upload'
import '@milkdown/theme-nord/style.css'
import { handleFileLinkClick } from '@/utils/fileLink'
import { revealItemInDir } from '@tauri-apps/plugin-opener'

const route = useRoute()
const subtaskId = parseInt(route.query.id as string)
const isViewMode = route.query.mode === 'view'
const appWindow = getCurrentWindow()

const title = ref('')
const markdownContent = ref('')
const editorContainer = ref<HTMLDivElement | null>(null)
let editorInstance: Editor | null = null

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

async function imageUploader(files: FileList, schema: any): Promise<Node[]> {
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

  const builder = Editor.make()
    .config(nord)
    .config((ctx) => {
      ctx.set(rootCtx, editorContainer.value!)
      ctx.set(defaultValueCtx, markdownContent.value || '')

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

      if (isViewMode) {
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
            markdownContent.value = markdown
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

  if (!isViewMode) {
    builder.use(listener).use(upload)
  }

  editorInstance = await builder.create()
}

function destroyEditor() {
  if (editorInstance) {
    editorInstance.destroy()
    editorInstance = null
  }
}

async function loadSubtask() {
  try {
    const result = await invoke<{ title: string; content: string | null }>('get_subtask', { id: subtaskId })
    title.value = result.title
    markdownContent.value = result.content || ''
  } catch (e) {
    console.error('Failed to load subtask:', e)
  }
}

async function handleSave() {
  if (!title.value.trim()) return
  try {
    await invoke('update_subtask', {
      id: subtaskId,
      data: {
        title: title.value.trim(),
        content: markdownContent.value,
      }
    })
    appWindow.close()
  } catch (e) {
    console.error('Failed to save subtask:', e)
  }
}

function handleClose() {
  appWindow.close()
}

async function handleMaximize() {
  const maximized = await appWindow.isMaximized()
  if (maximized) {
    await appWindow.unmaximize()
  } else {
    await appWindow.maximize()
  }
}

function onHeaderMouseDown(e: MouseEvent) {
  if (e.buttons !== 1) return
  const target = e.target as HTMLElement
  if (target.closest('[data-tauri-drag-region="false"]')) return
  if (target.closest('button, input, textarea, select, a, [role="button"]')) return
  e.preventDefault()
  appWindow.startDragging()
}

onMounted(async () => {
  await loadSubtask()
  await nextTick()
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
  <div class="subtask-editor-window">
    <div class="window-header" data-tauri-drag-region="deep" @mousedown="onHeaderMouseDown">
      <h2>{{ isViewMode ? '查看子任务' : '编辑子任务' }}</h2>
      <div class="window-controls" data-tauri-drag-region="false">
        <button class="control-btn maximize-btn" title="最大化" @click="handleMaximize">
          <el-icon :size="14"><FullScreen /></el-icon>
        </button>
        <button class="control-btn close-btn" title="关闭" @click="handleClose">
          <el-icon :size="14"><Close /></el-icon>
        </button>
      </div>
    </div>

    <div class="editor-content">
      <div class="form-field">
        <label class="field-label">标题</label>
        <el-input
          v-model="title"
          placeholder="请输入子任务标题"
          maxlength="200"
          :disabled="isViewMode"
        />
      </div>

      <div class="form-field editor-field">
        <label class="field-label">内容 (Markdown)</label>
        <div ref="editorContainer" class="milkdown-editor-wrapper"></div>
      </div>
    </div>

    <div class="window-footer">
      <div class="footer-right">
        <el-button @click="handleClose">
          <el-icon><Close /></el-icon>
          {{ isViewMode ? '关闭' : '取消' }}
        </el-button>
        <el-button v-if="!isViewMode" type="primary" @click="handleSave" :disabled="!title.trim()">
          <el-icon><Check /></el-icon>
          保存
        </el-button>
      </div>
    </div>

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
.subtask-editor-window {
  display: flex;
  flex-direction: column;
  height: 100vh;
  background: #ffffff;
}

.window-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 12px 16px;
  min-height: 44px;
  box-sizing: border-box;
  border-bottom: 1px solid var(--border, #e2e8f0);
  -webkit-app-region: drag;

  h2 {
    margin: 0;
    font-size: 16px;
    font-weight: 600;
    line-height: 1.2;
  }
}

.window-controls {
  display: flex;
  align-items: center;
  gap: 2px;
  -webkit-app-region: no-drag;
}

.control-btn {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 32px;
  height: 28px;
  border: none;
  background: transparent;
  cursor: pointer;
  border-radius: 4px;
  color: #64748b;
  transition: all 0.15s ease;

  &:hover {
    background: #f1f5f9;
    color: #334155;
  }

  &.close-btn:hover {
    background: #fee2e2;
    color: #ef4444;
  }
}

.editor-content {
  flex: 1;
  padding: 16px;
  overflow-y: auto;
  display: flex;
  flex-direction: column;
  gap: 16px;
}

.window-footer {
  display: flex;
  justify-content: flex-end;
  align-items: center;
  padding: 12px 16px;
  border-top: 1px solid var(--border, #e2e8f0);
}

.footer-right {
  display: flex;
  gap: 8px;
}

.form-field {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.field-label {
  font-size: 13px;
  font-weight: 600;
  color: #334155;
}

.editor-field {
  flex: 1;
  display: flex;
  flex-direction: column;
}

.milkdown-editor-wrapper {
  flex: 1;
  border: 1px solid #e2e8f0;
  border-radius: 8px;
  min-height: 300px;
  overflow-y: auto;
  background: #ffffff;
}

.milkdown-editor-wrapper :deep(.milkdown) {
  padding: 12px 16px;
  min-height: 290px;
}

.milkdown-editor-wrapper :deep(.editor) {
  outline: none;
}

.milkdown-editor-wrapper :deep(.ProseMirror) {
  outline: none;
  min-height: 280px;
}

.milkdown-editor-wrapper :deep(.ProseMirror p) {
  margin: 0.4em 0;
  line-height: 1.6;
}

.milkdown-editor-wrapper :deep(.ProseMirror h1),
.milkdown-editor-wrapper :deep(.ProseMirror h2),
.milkdown-editor-wrapper :deep(.ProseMirror h3) {
  margin: 0.6em 0 0.3em;
}

.milkdown-editor-wrapper :deep(.ProseMirror img) {
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

.milkdown-editor-wrapper :deep(.ProseMirror code) {
  background: #f1f5f9;
  padding: 2px 6px;
  border-radius: 4px;
  font-size: 0.9em;
}

.milkdown-editor-wrapper :deep(.ProseMirror pre) {
  background: #1e293b;
  color: #e2e8f0;
  padding: 12px 16px;
  border-radius: 8px;
  overflow-x: auto;
}

.milkdown-editor-wrapper :deep(.ProseMirror blockquote) {
  border-left: 3px solid #3b82f6;
  padding-left: 12px;
  color: #64748b;
  margin: 0.5em 0;
}

.milkdown-editor-wrapper :deep(.ProseMirror ul),
.milkdown-editor-wrapper :deep(.ProseMirror ol) {
  padding-left: 24px;
  margin: 0.4em 0;
}

.milkdown-editor-wrapper :deep(.ProseMirror hr) {
  border: none;
  border-top: 1px solid #e2e8f0;
  margin: 1em 0;
}

</style>
