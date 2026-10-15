<script setup lang="ts">
/**
 * 收纳箱项目网格（FenceView 的内容区）
 *
 * - 图标：按可见项懒加载（IntersectionObserver），按 `iconSize × devicePixelRatio` 取整到缓存档位请求
 *   物理像素的 PNG（后端 `fence_item_icon`），`convertFileSrc` 加载；失败显示占位图标
 * - 单击选中、悬停高亮、双击打开：打开期间显示"正在打开"（目标失效的快捷方式约 3 秒后才返回错误）
 * - 箱内拖动排序：vuedraggable + force-fallback（Windows 上原生文件拖放与 HTML5 DnD 互斥），松手后落库
 * - 自绘右键菜单：打开 / 打开所在位置 / 移回桌面 / 移动到…（列出其他收纳箱）
 * - Ctrl+滚轮调整图标大小：只发 `update:iconSize`，防抖落库由 FenceView 负责
 */
import { computed, nextTick, onMounted, onUnmounted, reactive, ref, watch, watchEffect } from 'vue'
import draggable from 'vuedraggable'
import { convertFileSrc, invoke } from '@tauri-apps/api/core'
import {
  ArrowLeft,
  Back,
  Document,
  Folder,
  FolderOpened,
  Loading,
  Rank,
  Right,
  TopRight,
} from '@element-plus/icons-vue'
import { ElMessage } from '@/plugins/element'
import type { Fence, FenceEntry, FenceItemOpResult } from '@/types'
import { iconBucket, iconRequestPx, nextIconSize, parentDir } from '@/utils/fence'
import { notifyError } from '@/utils/notify'

const props = defineProps<{
  fenceId: number
  items: FenceEntry[]
  /** 图标大小（CSS 像素） */
  iconSize: number
}>()

const emit = defineEmits<{
  /** 拖动排序后的新顺序（已落库） */
  (e: 'update:items', items: FenceEntry[]): void
  /** Ctrl+滚轮调整后的图标大小 */
  (e: 'update:iconSize', size: number): void
  /** 内容可能变了（移回 / 移走 / 排序保存失败），请父组件重新列举 */
  (e: 'changed'): void
  /** 右键菜单打开 / 关闭：折叠态下打开菜单要保持临时展开 */
  (e: 'menu-visible', visible: boolean): void
}>()

/** 同时在途的图标请求数：后端图标在单个 STA 线程上排队，多发也不会更快 */
const MAX_ICON_REQUESTS = 3
/** 可见区域外多算一点，滚动时图标提前加载 */
const ICON_PRELOAD_MARGIN = '64px'
/** Ctrl+滚轮的最小间隔：触控板一次手势会连发很多事件 */
const WHEEL_STEP_INTERVAL = 40

type IconState = { status: 'loading' } | { status: 'ok'; url: string } | { status: 'failed' }

interface MenuState {
  x: number
  y: number
  item: FenceEntry
  page: 'main' | 'move'
  /** 「移动到…」的目标：其他收纳箱 */
  targets: Fence[]
}

/** 本地列表：拖动期间不跟随 props（避免 SortableJS 拖动途中 Vue 移动 DOM 节点） */
const list = ref<FenceEntry[]>([])
let dragging = false
const selected = ref<string | null>(null)
const launching = reactive(new Set<string>())
const menu = ref<MenuState | null>(null)
const menuEl = ref<HTMLElement | null>(null)
const scroller = ref<HTMLElement | null>(null)
const dpr = ref(window.devicePixelRatio || 1)

/** 图标状态，键见 iconKey：路径、修改时间、尺寸档位任一变化都会重新请求 */
const icons = reactive(new Map<string, IconState>())
/** 已发出过请求的键（非响应式：在 watchEffect 里查重不会把自己再触发一遍） */
const requested = new Set<string>()
const pendingIcons: { key: string; fileName: string; sizePx: number }[] = []
let activeIconRequests = 0
/** 当前在可见区域内的项（文件名） */
const visible = reactive(new Set<string>())
let observer: IntersectionObserver | null = null
let lastWheelAt = 0

const bucket = computed(() => iconBucket(iconRequestPx(props.iconSize, dpr.value)))
const gridStyle = computed(() => ({
  '--fence-icon-size': `${props.iconSize}px`,
  '--fence-cell-width': `${props.iconSize + 40}px`,
}))

watch(
  () => props.items,
  (items) => {
    if (dragging) return
    list.value = [...items]
    if (selected.value && !items.some((item) => item.fileName === selected.value)) selected.value = null
    if (menu.value && !items.some((item) => item.fileName === menu.value?.item.fileName)) closeMenu()
  },
  { immediate: true }
)

// ---------- 图标 ----------

function iconKey(item: FenceEntry): string {
  return `${item.path}|${item.modifiedMs ?? ''}|${bucket.value}`
}

function iconOf(item: FenceEntry): IconState | undefined {
  return icons.get(iconKey(item))
}

function iconUrl(item: FenceEntry): string | undefined {
  const state = iconOf(item)
  return state?.status === 'ok' ? state.url : undefined
}

function requestIcon(item: FenceEntry) {
  const key = iconKey(item)
  if (requested.has(key)) return
  requested.add(key)
  icons.set(key, { status: 'loading' })
  pendingIcons.push({ key, fileName: item.fileName, sizePx: bucket.value })
  pumpIcons()
}

function pumpIcons() {
  while (activeIconRequests < MAX_ICON_REQUESTS && pendingIcons.length > 0) {
    const job = pendingIcons.shift()
    if (!job) break
    activeIconRequests += 1
    invoke<string>('fence_item_icon', { id: props.fenceId, fileName: job.fileName, sizePx: job.sizePx })
      .then((path) => icons.set(job.key, { status: 'ok', url: convertFileSrc(path) }))
      .catch((e) => {
        console.warn('Failed to load fence icon:', job.fileName, e)
        icons.set(job.key, { status: 'failed' })
      })
      .finally(() => {
        activeIconRequests -= 1
        pumpIcons()
      })
  }
}

// PNG 已在缓存里但加载失败（例如刚被清理）：显示占位图标
function onIconError(item: FenceEntry) {
  icons.set(iconKey(item), { status: 'failed' })
}

// 可见项缺图标就请求（列表、可见范围、尺寸档位变化都会重新跑）
watchEffect(() => {
  for (const item of list.value) {
    if (visible.has(item.fileName)) requestIcon(item)
  }
})

function observeItems() {
  if (!observer || !scroller.value) return
  observer.disconnect()
  visible.clear()
  for (const el of scroller.value.querySelectorAll<HTMLElement>('[data-file]')) observer.observe(el)
}

watch(list, () => void nextTick(observeItems), { flush: 'post' })

// 跨显示器移动后缩放比例变了：按新的 devicePixelRatio 取图标
function onResize() {
  dpr.value = window.devicePixelRatio || 1
}

// ---------- 选择 / 打开 ----------

function select(item: FenceEntry) {
  closeMenu()
  selected.value = item.fileName
}

async function openItem(item: FenceEntry) {
  closeMenu()
  selected.value = item.fileName
  if (launching.has(item.fileName)) return
  launching.add(item.fileName)
  try {
    await invoke('fence_open_item', { id: props.fenceId, fileName: item.fileName })
  } catch (e) {
    notifyError(e, `打开「${item.displayName}」失败`)
  } finally {
    launching.delete(item.fileName)
  }
}

async function revealItem(item: FenceEntry) {
  closeMenu()
  try {
    await invoke('fence_reveal_item', { id: props.fenceId, fileName: item.fileName })
  } catch (e) {
    notifyError(e, '打开所在位置失败')
  }
}

/** 移回 / 移走后先从本地列表去掉（不等监听事件），再请父组件按文件夹重新列举 */
function dropLocally(item: FenceEntry) {
  list.value = list.value.filter((entry) => entry.fileName !== item.fileName)
  emit('update:items', [...list.value])
  emit('changed')
}

/** 移回 / 移动的逐项结果提示：取消 → 提示已取消；失败 → 原因；跳过 → 不提示 */
function reportResult(result: FenceItemOpResult, action: string): boolean {
  if (result.status === 'moved') {
    if (result.error) ElMessage.warning(result.error)
    return true
  }
  if (result.status === 'cancelled') ElMessage.info(`已取消${action}`)
  else if (result.status === 'failed') notifyError(result.error ?? '', `${action}失败`)
  return false
}

async function restoreItem(item: FenceEntry) {
  closeMenu()
  try {
    const result = await invoke<FenceItemOpResult>('fence_restore_item', {
      id: props.fenceId,
      fileName: item.fileName,
    })
    if (reportResult(result, '移回桌面')) {
      if (!result.error && result.destPath) ElMessage.success(`已移回 ${parentDir(result.destPath)}`)
      dropLocally(item)
    }
  } catch (e) {
    notifyError(e, '移回桌面失败')
  }
}

async function moveItemTo(item: FenceEntry, target: Fence) {
  closeMenu()
  try {
    const result = await invoke<FenceItemOpResult>('fence_move_item', {
      id: props.fenceId,
      fileName: item.fileName,
      targetId: target.id,
    })
    if (reportResult(result, `移动到「${target.title}」`)) dropLocally(item)
  } catch (e) {
    notifyError(e, '移动失败')
  }
}

// ---------- 右键菜单 ----------

async function openMenu(e: MouseEvent, item: FenceEntry) {
  selected.value = item.fileName
  let targets: Fence[] = []
  try {
    targets = (await invoke<Fence[]>('list_fences')).filter((fence) => fence.id !== props.fenceId)
  } catch (err) {
    console.error('Failed to list fences:', err)
  }
  const wasOpen = menu.value !== null
  menu.value = { x: e.clientX, y: e.clientY, item, page: 'main', targets }
  if (!wasOpen) emit('menu-visible', true)
  await nextTick()
  clampMenu()
}

/** 菜单整体放进窗口可见范围（收纳箱窗口很小，靠右 / 靠下点开时要往回挪） */
function clampMenu() {
  const el = menuEl.value
  if (!el || !menu.value) return
  const rect = el.getBoundingClientRect()
  menu.value.x = Math.max(4, Math.min(menu.value.x, window.innerWidth - rect.width - 4))
  menu.value.y = Math.max(4, Math.min(menu.value.y, window.innerHeight - rect.height - 4))
}

async function showMovePage() {
  if (!menu.value || menu.value.targets.length === 0) return
  menu.value.page = 'move'
  await nextTick()
  clampMenu()
}

async function showMainPage() {
  if (!menu.value) return
  menu.value.page = 'main'
  await nextTick()
  clampMenu()
}

function closeMenu() {
  if (!menu.value) return
  menu.value = null
  emit('menu-visible', false)
}

function onDocumentPointerDown(e: PointerEvent) {
  if (menu.value && menuEl.value && !menuEl.value.contains(e.target as Node)) closeMenu()
}

function onKeydown(e: KeyboardEvent) {
  if (e.key === 'Escape' && menu.value) {
    e.stopPropagation()
    closeMenu()
  }
}

// ---------- 排序 / Ctrl+滚轮 ----------

function onDragStart() {
  dragging = true
  closeMenu()
}

async function onDragEnd() {
  dragging = false
  const names = list.value.map((item) => item.fileName)
  if (names.join('\n') === props.items.map((item) => item.fileName).join('\n')) return
  emit('update:items', [...list.value])
  try {
    await invoke('reorder_fence_items', { id: props.fenceId, fileNames: names })
  } catch (e) {
    notifyError(e, '保存排序失败')
  }
  // 拖动期间被跳过的刷新（监听事件）在这里补上；排序已落库，列举结果按新顺序返回
  emit('changed')
}

function onWheel(e: WheelEvent) {
  if (!e.ctrlKey) {
    closeMenu()
    return
  }
  e.preventDefault()
  const now = performance.now()
  if (now - lastWheelAt < WHEEL_STEP_INTERVAL) return
  lastWheelAt = now
  const next = nextIconSize(props.iconSize, e.deltaY)
  if (next !== props.iconSize) emit('update:iconSize', next)
}

onMounted(() => {
  observer = new IntersectionObserver(
    (entries) => {
      for (const entry of entries) {
        const name = (entry.target as HTMLElement).dataset.file
        if (name === undefined) continue
        if (entry.isIntersecting) visible.add(name)
        else visible.delete(name)
      }
    },
    { root: scroller.value, rootMargin: ICON_PRELOAD_MARGIN }
  )
  observeItems()
  window.addEventListener('resize', onResize)
  window.addEventListener('blur', closeMenu)
  window.addEventListener('keydown', onKeydown, true)
  document.addEventListener('pointerdown', onDocumentPointerDown, true)
})

onUnmounted(() => {
  observer?.disconnect()
  observer = null
  window.removeEventListener('resize', onResize)
  window.removeEventListener('blur', closeMenu)
  window.removeEventListener('keydown', onKeydown, true)
  document.removeEventListener('pointerdown', onDocumentPointerDown, true)
  closeMenu()
})
</script>

<template>
  <div ref="scroller" class="fence-grid-scroll" :style="gridStyle" @click="selected = null" @wheel="onWheel">
    <draggable
      v-model="list"
      item-key="fileName"
      class="fence-grid"
      ghost-class="is-ghost"
      :animation="150"
      :force-fallback="true"
      :fallback-tolerance="4"
      @start="onDragStart"
      @end="onDragEnd"
    >
      <template #item="{ element }">
        <div
          class="fence-item"
          :class="{
            'is-selected': selected === element.fileName,
            'is-launching': launching.has(element.fileName),
          }"
          :data-file="element.fileName"
          :title="launching.has(element.fileName) ? `正在打开「${element.displayName}」…` : element.displayName"
          @click.stop="select(element)"
          @dblclick.stop="openItem(element)"
          @contextmenu.prevent.stop="openMenu($event, element)"
        >
          <div class="fence-item-icon">
            <img
              v-if="iconUrl(element)"
              :src="iconUrl(element)"
              alt=""
              draggable="false"
              @error="onIconError(element)"
            />
            <el-icon v-else-if="iconOf(element)?.status === 'failed'" class="fence-item-placeholder">
              <Folder v-if="element.isDir" />
              <Document v-else />
            </el-icon>
            <div v-if="launching.has(element.fileName)" class="fence-item-busy">
              <el-icon class="is-loading"><Loading /></el-icon>
            </div>
          </div>
          <span class="fence-item-name">{{ element.displayName }}</span>
        </div>
      </template>
    </draggable>

    <div
      v-if="menu"
      ref="menuEl"
      class="fence-item-menu"
      :style="{ left: `${menu.x}px`, top: `${menu.y}px` }"
      @click.stop
      @contextmenu.prevent.stop
    >
      <template v-if="menu.page === 'main'">
        <button class="menu-item" @click="openItem(menu.item)">
          <el-icon><TopRight /></el-icon>
          <span>打开</span>
        </button>
        <button class="menu-item" @click="revealItem(menu.item)">
          <el-icon><FolderOpened /></el-icon>
          <span>打开所在位置</span>
        </button>
        <button class="menu-item" @click="restoreItem(menu.item)">
          <el-icon><Back /></el-icon>
          <span>移回桌面</span>
        </button>
        <button
          class="menu-item"
          :disabled="menu.targets.length === 0"
          :title="menu.targets.length === 0 ? '没有其他收纳箱' : ''"
          @click="showMovePage"
        >
          <el-icon><Rank /></el-icon>
          <span>移动到…</span>
          <span v-if="menu.targets.length === 0" class="menu-hint">没有其他收纳箱</span>
          <el-icon v-else class="menu-arrow"><Right /></el-icon>
        </button>
      </template>
      <template v-else>
        <button class="menu-item" @click="showMainPage">
          <el-icon><ArrowLeft /></el-icon>
          <span>移动到</span>
        </button>
        <div class="menu-divider" />
        <button
          v-for="target in menu.targets"
          :key="target.id"
          class="menu-item"
          :title="target.title"
          @click="moveItemTo(menu.item, target)"
        >
          <span class="menu-target">{{ target.title }}</span>
        </button>
      </template>
    </div>
  </div>
</template>

<style scoped>
.fence-grid-scroll {
  height: 100%;
  overflow-x: hidden;
  overflow-y: auto;
  scrollbar-width: thin;
  scrollbar-color: rgba(255, 255, 255, 0.25) transparent;
}

.fence-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, var(--fence-cell-width));
  /* 列间距为 0：默认尺寸刚好放下 4 列（后端按 88 DIP 一格 × 4 列 + 内边距建窗） */
  gap: 2px 0;
  align-content: start;
  justify-content: start;
  min-height: 100%;
}

/* 可拖动列表项：过渡属性显式列出、不含 transform（fallback 克隆体会被 transition 拖慢，见 component-guidelines） */
.fence-item {
  position: relative;
  display: flex;
  flex-direction: column;
  gap: 4px;
  align-items: center;
  padding: 6px 4px 4px;
  cursor: default;
  user-select: none;
  border-radius: 6px;
  transition:
    background-color 0.15s ease,
    box-shadow 0.15s ease;
}

.fence-item:hover {
  background: rgba(255, 255, 255, 0.12);
}

.fence-item.is-selected {
  background: rgba(96, 165, 250, 0.32);
  box-shadow: inset 0 0 0 1px rgba(147, 197, 253, 0.65);
}

.fence-item.is-ghost {
  opacity: 0.35;
}

.fence-item-icon {
  position: relative;
  display: flex;
  flex-shrink: 0;
  align-items: center;
  justify-content: center;
  width: var(--fence-icon-size);
  height: var(--fence-icon-size);
}

.fence-item-icon img {
  width: 100%;
  height: 100%;
  object-fit: contain;
  filter: drop-shadow(0 1px 2px rgba(0, 0, 0, 0.35));
  -webkit-user-drag: none;
}

.fence-item-placeholder {
  font-size: calc(var(--fence-icon-size) * 0.72);
  color: rgba(255, 255, 255, 0.75);
}

.fence-item.is-launching .fence-item-icon img,
.fence-item.is-launching .fence-item-placeholder {
  opacity: 0.45;
}

.fence-item-busy {
  position: absolute;
  inset: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: calc(var(--fence-icon-size) * 0.45);
  color: #fff;
}

.fence-item-name {
  display: -webkit-box;
  width: 100%;
  overflow: hidden;
  font-size: 12px;
  line-height: 16px;
  text-align: center;
  text-shadow: 0 1px 2px rgba(0, 0, 0, 0.65);
  word-break: break-all;
  -webkit-line-clamp: 2;
  -webkit-box-orient: vertical;
}

.fence-item-menu {
  position: fixed;
  z-index: 100;
  display: flex;
  flex-direction: column;
  min-width: 150px;
  max-width: calc(100vw - 8px);
  max-height: calc(100vh - 8px);
  padding: 4px;
  overflow-y: auto;
  background: rgba(15, 23, 42, 0.96);
  border: 1px solid rgba(255, 255, 255, 0.12);
  border-radius: 6px;
  box-shadow: 0 6px 18px rgba(0, 0, 0, 0.45);
}

.menu-item {
  display: flex;
  flex-shrink: 0;
  gap: 8px;
  align-items: center;
  height: 28px;
  padding: 0 8px;
  font-size: 12px;
  color: rgba(255, 255, 255, 0.9);
  text-align: left;
  white-space: nowrap;
  cursor: pointer;
  background: transparent;
  border: none;
  border-radius: 4px;
}

.menu-item:hover:not(:disabled) {
  background: rgba(255, 255, 255, 0.12);
}

.menu-item:disabled {
  cursor: default;
  opacity: 0.5;
}

.menu-hint {
  margin-left: auto;
  font-size: 11px;
  color: rgba(255, 255, 255, 0.6);
}

.menu-arrow {
  margin-left: auto;
}

.menu-target {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
}

.menu-divider {
  flex-shrink: 0;
  height: 1px;
  margin: 3px 4px;
  background: rgba(255, 255, 255, 0.1);
}
</style>
