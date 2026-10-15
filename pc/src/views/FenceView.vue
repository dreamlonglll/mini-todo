<script setup lang="ts">
/**
 * 桌面收纳箱窗口（label `fence-{id}`，路由 `#/fence?id={id}`）
 *
 * 窗口由后端 `commands/fence.rs` 建立并始终嵌入桌面层。这里负责外壳：标题栏（移动 / 折叠 / 固定 /
 * 改名 / 菜单）、缩放手柄、悬停与拖文件经过时临时展开、外观面板、删除确认；以及收纳箱内容：
 * - 项目列表来自 `get_fence_items`（后端按收纳文件夹对账），文件夹变化时后端推 `fence-items-changed`
 * - 外部拖入（`onDragDropEvent` 的 drop）→ `fence_move_in`，逐项结果汇总提示；锁定时照样可以拖入
 * - 项目网格（图标、打开、右键菜单、箱内排序、Ctrl+滚轮）在 `components/FenceItemGrid.vue`
 * - 图标大小防抖落库（只写最后一次）；删除收纳箱时项目移回桌面，有没能移回的项目时删除中止、收纳箱整个保留并提示原因
 *
 * 几何约定：
 * - 移动：标题栏按下后移动超过阈值才 `startDragging`，之后的 onMoved 防抖落库（`commit_fence_geometry`）
 * - 缩放：窗口是 `resizable(false)`（系统缩放循环不生效），缩放手柄捕获指针、逐帧调 `set_fence_bounds`，
 *   松手后落库。系统挪窗（显示器变化等）一律不落库
 * - 折叠 / 展开（含悬停临时展开）只调 `set_fence_view`：后端按库里的几何算位置尺寸，
 *   下方空间不够时向上展开
 * - 「固定」只锁定移动和缩放：仍可拖入、折叠、改名
 */
import { computed, nextTick, onMounted, onUnmounted, reactive, ref, watch } from 'vue'
import { useRoute } from 'vue-router'
import { invoke } from '@tauri-apps/api/core'
import type { UnlistenFn } from '@tauri-apps/api/event'
import { getCurrentWebview } from '@tauri-apps/api/webview'
import { getCurrentWindow } from '@tauri-apps/api/window'
import type { InputInstance } from 'element-plus'
import {
  ArrowDown,
  ArrowUp,
  Brush,
  Close,
  Delete,
  EditPen,
  FolderOpened,
  InfoFilled,
  Lock,
  MoreFilled,
  Unlock,
} from '@element-plus/icons-vue'
import FenceItemGrid from '@/components/FenceItemGrid.vue'
import { ElMessage } from '@/plugins/element'
import type {
  DeleteFenceResult,
  Fence,
  FenceEntry,
  FenceMoveInResult,
  RenameFenceResult,
  UpdateFenceRequest,
} from '@/types'
import {
  FENCE_ITEMS_CHANGED_EVENT,
  FENCE_BG_ALPHA_MAX_PERCENT,
  FENCE_BG_ALPHA_MIN_PERCENT,
  FENCE_MIN_EXPANDED_HEIGHT,
  FENCE_MIN_WIDTH,
  FENCE_TITLE_BAR_HEIGHT,
  FENCE_TITLE_MAX_LENGTH,
  PRESET_BG_COLORS,
} from '@/types'
import { hexToRgbChannels, isSameColor } from '@/utils/color'
import { isImeComposing } from '@/utils/editorShortcuts'
import {
  describeDeleteKept,
  describeMoveIn,
  exceedsDragThreshold,
  GeometryCommitGate,
  normalizeFenceTitle,
  parseFenceId,
  resizeRect,
  summarizeOpResults,
} from '@/utils/fence'
import type { FenceResizeDirection, PxRect } from '@/utils/fence'
import { errorMessage, notifyError } from '@/utils/notify'

/**
 * 折叠态下临时展开的原因（任一存在即展开，全部消失即收回；都不改持久化的折叠态）：
 * 鼠标悬停 / 拖着文件经过 / 标题栏菜单打开（菜单要有地方弹出）/ 面板打开 / 项目右键菜单打开
 */
type ExpandReason = 'hover' | 'drag' | 'menu' | 'panel' | 'itemMenu'
/** 面板：外观 / 删除确认 / 首次收纳的 OneDrive 提示 */
type Panel = 'none' | 'appearance' | 'delete' | 'onedrive'

/** 悬停多久才展开（避免鼠标划过就弹开）、移开多久才收回 */
const HOVER_EXPAND_DELAY = 150
const HOVER_COLLAPSE_DELAY = 400
/** 拖着文件离开 / 放下后多久收回 */
const DRAG_COLLAPSE_DELAY = 300
/** 拖动 / 缩放闸门的续期窗口与落库防抖（见 GeometryCommitGate） */
const GEOMETRY_ARM_MS = 3000
const GEOMETRY_COMMIT_DEBOUNCE = 300
/** Ctrl+滚轮调整图标大小后多久落库（连续滚动只写最后一次） */
const ICON_SIZE_SAVE_DELAY = 400
/** 删除中止提示的显示时长（要列出文件名与原因，比普通提示长） */
const DELETE_KEPT_MESSAGE_DURATION = 8000

const RESIZE_HANDLES: FenceResizeDirection[] = ['n', 's', 'w', 'e', 'nw', 'ne', 'sw', 'se']

const route = useRoute()
const appWindow = getCurrentWindow()
const fenceId = parseFenceId(route.query.id)

const fence = ref<Fence | null>(null)
const loadError = ref('')
const expandReasons = reactive(new Set<ExpandReason>())
const panel = ref<Panel>('none')
const editingTitle = ref(false)
const titleDraft = ref('')
const titleInput = ref<InputInstance | null>(null)
const dragHover = ref(false)
/** 拖动透明度滑块时的本地预览值，松手后落库 */
const alphaPreview = ref<number | null>(null)
/** 收纳箱内容（按 fence_items.sort_order 排好序） */
const items = ref<FenceEntry[]>([])
/** 删除确认框里的项目数：打开确认框时重新列举（null = 正在统计） */
const deleteCount = ref<number | null>(null)
const deleting = ref(false)

const geometryGate = new GeometryCommitGate(GEOMETRY_ARM_MS)
let commitTimer: ReturnType<typeof setTimeout> | undefined
let hoverTimer: ReturnType<typeof setTimeout> | undefined
let dragTimer: ReturnType<typeof setTimeout> | undefined
let iconSizeTimer: ReturnType<typeof setTimeout> | undefined
let pressPoint: { x: number; y: number } | null = null
let viewQueue: Promise<void> = Promise.resolve()
/** 列表刷新序号：只采用最后一次发起的列举 / 最新的推送，乱序返回的旧结果丢掉 */
let itemsSeq = 0
/** 缩放中的状态：起点指针位置（屏幕 CSS 像素）与窗口矩形（物理像素） */
let resizing: {
  direction: FenceResizeDirection
  pointerId: number
  startX: number
  startY: number
  start: PxRect
  dpr: number
} | null = null
/** 按下缩放手柄、还没松开（按下后要先异步取窗口矩形，期间松开就不再进入缩放） */
let resizePressed = false
let resizeFrame = 0
let pendingBounds: PxRect | null = null
let boundsInFlight: Promise<void> = Promise.resolve()
const unlisteners: UnlistenFn[] = []

const collapsed = computed(() => fence.value?.collapsed ?? false)
const locked = computed(() => fence.value?.locked ?? false)
/** 窗口当前是否显示完整高度（展开，或折叠态下的临时展开） */
const viewExpanded = computed(() => !!fence.value && (!fence.value.collapsed || expandReasons.size > 0))
/** 只有固定展开、未锁定时可以缩放（折叠态与临时展开都不给缩放手柄） */
const canResize = computed(
  () => !!fence.value && !fence.value.locked && !fence.value.collapsed && !editingTitle.value
)
const bgAlpha = computed(() => alphaPreview.value ?? fence.value?.bgAlpha ?? 0.45)
const rootStyle = computed(() => ({
  '--fence-bg-rgb': hexToRgbChannels(fence.value?.bgColor ?? '#000000'),
  '--fence-bg-alpha': String(bgAlpha.value),
  '--fence-title-height': `${FENCE_TITLE_BAR_HEIGHT}px`,
}))
const deleteHint = computed(() => {
  if (deleteCount.value === null) return '正在统计其中的项目…'
  if (deleteCount.value === 0) return '收纳箱是空的，收纳文件夹会一并删除'
  return `其中的 ${deleteCount.value} 个项目会移回桌面`
})

function setReason(reason: ExpandReason, on: boolean) {
  if (on) expandReasons.add(reason)
  else expandReasons.delete(reason)
}

// ---------- 折叠 / 展开 ----------

// 视图变化串行发给后端：快速进出时保证最后一次生效
watch(viewExpanded, (expanded) => {
  viewQueue = viewQueue.then(async () => {
    // 刚拖完还没落库的位置先落库：后端按库里的几何算折叠 / 展开后的位置
    await flushGeometry()
    try {
      await invoke('set_fence_view', { id: fenceId, expanded })
    } catch (e) {
      console.error('Failed to apply fence view:', e)
    }
  })
})

watch(panel, (value) => {
  setReason('panel', value !== 'none')
  // 滑块拖到一半就关面板（没触发 change）时，不留下没落库的预览值
  if (value !== 'appearance') alphaPreview.value = null
})

/** 采用后端返回的收纳箱；图标大小还没落库时保留本地值（否则 Ctrl+滚轮后立刻折叠会跳回旧大小） */
function applyFence(next: Fence) {
  const pendingIconSize = iconSizeTimer !== undefined && fence.value ? fence.value.iconSize : null
  fence.value = pendingIconSize === null ? next : { ...next, iconSize: pendingIconSize }
}

async function patchFence(patch: UpdateFenceRequest, failMessage: string) {
  if (fenceId === null) return
  try {
    await flushGeometry()
    applyFence(await invoke<Fence>('update_fence', { id: fenceId, patch }))
  } catch (e) {
    notifyError(e, failMessage)
  }
}

async function toggleCollapsed() {
  if (!fence.value) return
  await patchFence({ collapsed: !fence.value.collapsed }, '折叠收纳箱失败')
  // 手动折叠 / 展开后不再保留悬停展开：刚点了折叠的收纳箱不该因为鼠标还在上面又弹开
  setReason('hover', false)
}

async function toggleLocked() {
  if (!fence.value) return
  await patchFence({ locked: !fence.value.locked }, '固定收纳箱失败')
}

function onPointerEnter() {
  clearTimeout(hoverTimer)
  if (!collapsed.value) return
  hoverTimer = setTimeout(() => {
    // 按下标题栏就拖走了（还没到展开延时）：拖动中不展开，否则窗口会在系统拖动循环里被挪动 / 撑开
    if (geometryGate.isArmed(Date.now())) return
    setReason('hover', true)
  }, HOVER_EXPAND_DELAY)
}

function onPointerLeave() {
  clearTimeout(hoverTimer)
  if (!expandReasons.has('hover')) return
  hoverTimer = setTimeout(releaseHover, HOVER_COLLAPSE_DELAY)
}

function releaseHover() {
  // 拖动期间 WebView 可能报出 mouseleave（系统拖动循环接管了鼠标），拖完再判断
  if (geometryGate.isArmed(Date.now())) {
    hoverTimer = setTimeout(releaseHover, HOVER_COLLAPSE_DELAY)
    return
  }
  setReason('hover', false)
}

// ---------- 移动 / 缩放与几何落库 ----------

function onTitleMouseDown(e: MouseEvent) {
  if (e.button !== 0) return
  const target = e.target as HTMLElement
  if (target.closest('button, input, textarea, .el-input, .el-dropdown')) return
  pressPoint = { x: e.screenX, y: e.screenY }
}

// 按下后真的移动了才交给系统拖动：直接在 mousedown 里 startDragging 会吞掉 mouseup，双击改名就收不到了
function onTitleMouseMove(e: MouseEvent) {
  if (!pressPoint) return
  if (e.buttons !== 1) {
    pressPoint = null
    return
  }
  if (!exceedsDragThreshold(e.screenX - pressPoint.x, e.screenY - pressPoint.y)) return
  pressPoint = null
  if (locked.value || editingTitle.value) return
  geometryGate.arm(Date.now())
  appWindow.startDragging().catch((err) => console.error('Failed to start dragging fence:', err))
}

async function onResizePointerDown(direction: FenceResizeDirection, e: PointerEvent) {
  if (e.button !== 0 || !canResize.value || resizing) return
  e.preventDefault()
  const handle = e.currentTarget as HTMLElement
  // 指针捕获：拖出窗口外仍能收到 pointermove / pointerup
  handle.setPointerCapture(e.pointerId)
  const pointer = { id: e.pointerId, x: e.screenX, y: e.screenY }
  resizePressed = true
  await flushGeometry()
  const [position, size] = await Promise.all([appWindow.outerPosition(), appWindow.outerSize()])
  if (!resizePressed) return
  resizing = {
    direction,
    pointerId: pointer.id,
    startX: pointer.x,
    startY: pointer.y,
    start: { x: position.x, y: position.y, width: size.width, height: size.height },
    dpr: window.devicePixelRatio || 1,
  }
}

function onResizePointerMove(e: PointerEvent) {
  if (!resizing || e.pointerId !== resizing.pointerId || (e.buttons & 1) === 0) return
  const { direction, start, dpr } = resizing
  pendingBounds = resizeRect(
    start,
    direction,
    (e.screenX - resizing.startX) * dpr,
    (e.screenY - resizing.startY) * dpr,
    Math.round(FENCE_MIN_WIDTH * dpr),
    Math.round(FENCE_MIN_EXPANDED_HEIGHT * dpr)
  )
  // 每帧最多发一次；上一次还没处理完就只留最新的
  if (!resizeFrame) resizeFrame = requestAnimationFrame(flushBounds)
}

function flushBounds() {
  resizeFrame = 0
  const bounds = pendingBounds
  pendingBounds = null
  if (!bounds || fenceId === null) return
  boundsInFlight = boundsInFlight.then(() =>
    invoke<void>('set_fence_bounds', { id: fenceId, ...bounds }).catch((err) =>
      console.error('Failed to resize fence:', err)
    )
  )
}

async function onResizePointerUp(e: PointerEvent) {
  resizePressed = false
  if (!resizing || e.pointerId !== resizing.pointerId) return
  resizing = null
  ;(e.currentTarget as HTMLElement).releasePointerCapture(e.pointerId)
  if (resizeFrame) {
    cancelAnimationFrame(resizeFrame)
    flushBounds()
  }
  await boundsInFlight
  await commitGeometry()
}

function onGeometryEvent() {
  if (!geometryGate.accept(Date.now())) return
  clearTimeout(commitTimer)
  commitTimer = setTimeout(() => void commitGeometry(), GEOMETRY_COMMIT_DEBOUNCE)
}

// 落库后不关闸门：用户按住不放、停顿超过防抖时间再接着拖时，后续移动仍要算数。闸门在最后一次
// 移动后 GEOMETRY_ARM_MS 自然失效；折叠 / 展开等程序调整前由 flushGeometry 显式关掉
async function commitGeometry() {
  commitTimer = undefined
  if (fenceId === null) return
  try {
    applyFence(
      await invoke<Fence>('commit_fence_geometry', {
        id: fenceId,
        expandedView: viewExpanded.value,
      })
    )
  } catch (e) {
    notifyError(e, '保存收纳箱位置失败')
  }
}

/** 有待落库的拖动结果就立即落库，然后关掉闸门：之后的程序调整不会被当成用户拖动 */
async function flushGeometry() {
  if (commitTimer !== undefined) {
    clearTimeout(commitTimer)
    await commitGeometry()
  }
  geometryGate.disarm()
}

// ---------- 改名 ----------

async function startRename() {
  if (!fence.value || editingTitle.value) return
  titleDraft.value = fence.value.title
  editingTitle.value = true
  await nextTick()
  titleInput.value?.focus()
  titleInput.value?.select()
}

async function finishRename(commit: boolean) {
  if (!editingTitle.value || !fence.value || fenceId === null) return
  editingTitle.value = false
  if (!commit) return
  const title = normalizeFenceTitle(titleDraft.value)
  if (!title || title === fence.value.title) return
  try {
    const result = await invoke<RenameFenceResult>('rename_fence', { id: fenceId, title })
    applyFence(result.fence)
    if (result.folderError) {
      ElMessage.warning(`标题已修改，但收纳文件夹没能跟着改名：${result.folderError}`)
    }
  } catch (e) {
    notifyError(e, '重命名收纳箱失败')
  }
}

function onTitleKeydown(e: Event | KeyboardEvent) {
  if (!(e instanceof KeyboardEvent) || isImeComposing(e)) return
  if (e.key === 'Enter') {
    e.preventDefault()
    void finishRename(true)
  } else if (e.key === 'Escape') {
    e.preventDefault()
    void finishRename(false)
  }
}

// ---------- 菜单 / 外观 / 删除 ----------

function onMenuVisibleChange(visible: boolean) {
  if (visible) {
    setReason('menu', true)
  } else {
    // 菜单项打开的面板先接管展开态，再撤掉菜单的原因，避免收回又展开
    void nextTick(() => setReason('menu', false))
  }
}

function onMenuCommand(command: string) {
  if (command === 'rename') void startRename()
  else if (command === 'appearance') panel.value = 'appearance'
  else if (command === 'delete') void openDeletePanel()
}

async function setBgColor(color: string) {
  if (!fence.value || isSameColor(color, fence.value.bgColor)) return
  await patchFence({ bgColor: color }, '设置收纳箱背景色失败')
}

function onCustomColorChange(e: Event) {
  void setBgColor((e.target as HTMLInputElement).value)
}

// el-slider 事件值的类型是 number | number[]（range 模式才是数组），这里不是 range
function sliderValue(value: number | number[]): number {
  return Array.isArray(value) ? value[0] : value
}

function onAlphaInput(value: number | number[]) {
  alphaPreview.value = sliderValue(value) / 100
}

async function onAlphaChange(value: number | number[]) {
  await patchFence({ bgAlpha: sliderValue(value) / 100 }, '设置收纳箱透明度失败')
  alphaPreview.value = null
}

// 确认框里的项目数用实际列举结果（文件夹为准；隐藏文件不计入，但删除时会一并移回）
async function openDeletePanel() {
  panel.value = 'delete'
  deleteCount.value = null
  await refreshItems()
  deleteCount.value = items.value.length
}

async function confirmDelete() {
  if (fenceId === null || deleting.value) return
  deleting.value = true
  try {
    // 后端先把项目全部移回桌面；全部移回才删记录并销毁本窗口
    const result = await invoke<DeleteFenceResult>('delete_fence', { id: fenceId })
    if (!result.deleted) {
      // 有项目没能移回（或收纳文件夹没能删除）：删除中止，收纳箱整个保留，继续显示剩余项，用户处理后可以再删一次
      panel.value = 'none'
      ElMessage({
        type: 'warning',
        message: describeDeleteKept(result),
        duration: DELETE_KEPT_MESSAGE_DURATION,
        showClose: true,
      })
      await refreshItems()
    }
  } catch (e) {
    notifyError(e, '删除收纳箱失败')
  } finally {
    deleting.value = false
  }
}

// ---------- 内容：列举 / 拖入 / 图标大小 ----------

async function refreshItems() {
  if (fenceId === null) return
  const seq = ++itemsSeq
  try {
    const list = await invoke<FenceEntry[]>('get_fence_items', { id: fenceId })
    if (seq === itemsSeq) items.value = list
  } catch (e) {
    // 后台刷新不弹提示（收纳箱正在删除时也会走到这里）
    console.error('Failed to load fence items:', e)
  }
}

function setItems(list: FenceEntry[]) {
  itemsSeq += 1
  items.value = list
}

// 外部拖入：锁定（固定）状态下照样可以拖入
async function moveIn(paths: string[]) {
  if (fenceId === null || paths.length === 0) return
  try {
    const result = await invoke<FenceMoveInResult>('fence_move_in', { id: fenceId, paths })
    const notice = describeMoveIn(summarizeOpResults(result.results))
    if (notice) ElMessage({ type: notice.type, message: notice.message, duration: 5000 })
    if (result.onedriveNotice) panel.value = 'onedrive'
  } catch (e) {
    notifyError(e, '收纳失败')
  }
  await refreshItems()
}

function onIconSizeChange(size: number) {
  if (!fence.value) return
  fence.value.iconSize = size
  clearTimeout(iconSizeTimer)
  iconSizeTimer = setTimeout(() => void saveIconSize(), ICON_SIZE_SAVE_DELAY)
}

async function saveIconSize() {
  iconSizeTimer = undefined
  if (!fence.value || fenceId === null) return
  try {
    // 不采用返回值：落库期间用户可能又滚了几格
    await invoke('update_fence', { id: fenceId, patch: { iconSize: fence.value.iconSize } })
  } catch (e) {
    notifyError(e, '保存图标大小失败')
  }
}

function onKeydown(e: KeyboardEvent) {
  // 删除进行中（可能在等管理员权限对话框）不关确认框
  if (e.key !== 'Escape' || isImeComposing(e) || deleting.value) return
  if (panel.value !== 'none') panel.value = 'none'
}

// 改名时点到桌面 / 别的窗口：窗口失去焦点也算失焦提交（输入框自身的 blur 在页面没有系统焦点时不一定触发）
function onWindowBlur() {
  if (editingTitle.value) void finishRename(true)
}

// ---------- 生命周期 ----------

onMounted(async () => {
  if (fenceId === null) {
    loadError.value = '收纳箱参数无效'
    return
  }
  try {
    fence.value = await invoke<Fence>('get_fence', { id: fenceId })
  } catch (e) {
    loadError.value = errorMessage(e) || '读取收纳箱失败'
    return
  }
  void refreshItems()

  window.addEventListener('keydown', onKeydown)
  window.addEventListener('blur', onWindowBlur)
  // 后端对账后推给本窗口（emit_to 本窗口 label）的项目列表
  unlisteners.push(
    await appWindow.listen<FenceEntry[]>(FENCE_ITEMS_CHANGED_EVENT, (event) => setItems(event.payload))
  )
  unlisteners.push(await appWindow.onMoved(onGeometryEvent))
  unlisteners.push(await appWindow.onResized(onGeometryEvent))
  unlisteners.push(
    await getCurrentWebview().onDragDropEvent((event) => {
      const { type } = event.payload
      if (type === 'enter' || type === 'over') {
        clearTimeout(dragTimer)
        dragHover.value = true
        setReason('drag', true)
        return
      }
      dragHover.value = false
      if (type === 'drop') void moveIn(event.payload.paths)
      dragTimer = setTimeout(() => setReason('drag', false), DRAG_COLLAPSE_DELAY)
    })
  )
})

onUnmounted(() => {
  window.removeEventListener('keydown', onKeydown)
  window.removeEventListener('blur', onWindowBlur)
  for (const unlisten of unlisteners) unlisten()
  clearTimeout(commitTimer)
  clearTimeout(hoverTimer)
  clearTimeout(dragTimer)
  if (iconSizeTimer !== undefined) {
    clearTimeout(iconSizeTimer)
    void saveIconSize()
  }
  if (resizeFrame) cancelAnimationFrame(resizeFrame)
})
</script>

<template>
  <div
    class="fence-window"
    :class="{ 'is-collapsed': !viewExpanded, 'is-locked': locked, 'is-drag-hover': dragHover }"
    :style="rootStyle"
    @mouseenter="onPointerEnter"
    @mouseleave="onPointerLeave"
  >
    <div
      class="fence-titlebar"
      @mousedown="onTitleMouseDown"
      @mousemove="onTitleMouseMove"
      @mouseup="pressPoint = null"
      @dblclick="startRename"
    >
      <el-input
        v-if="editingTitle"
        ref="titleInput"
        v-model="titleDraft"
        class="fence-title-input"
        size="small"
        :maxlength="FENCE_TITLE_MAX_LENGTH"
        @keydown="onTitleKeydown"
        @blur="finishRename(true)"
      />
      <span v-else class="fence-title" :title="fence?.title">{{ fence?.title ?? loadError }}</span>

      <div v-if="fence" class="fence-actions" @mousedown.stop @dblclick.stop>
        <button class="fence-btn" :title="collapsed ? '展开' : '折叠'" @click="toggleCollapsed">
          <el-icon :size="14">
            <ArrowDown v-if="collapsed" />
            <ArrowUp v-else />
          </el-icon>
        </button>
        <button
          class="fence-btn"
          :class="{ active: locked }"
          :title="locked ? '取消固定' : '固定位置和大小'"
          @click="toggleLocked"
        >
          <el-icon :size="14">
            <Lock v-if="locked" />
            <Unlock v-else />
          </el-icon>
        </button>
        <el-dropdown
          trigger="click"
          placement="bottom-end"
          :max-height="200"
          @command="onMenuCommand"
          @visible-change="onMenuVisibleChange"
        >
          <button class="fence-btn" title="更多">
            <el-icon :size="14"><MoreFilled /></el-icon>
          </button>
          <template #dropdown>
            <el-dropdown-menu>
              <el-dropdown-item command="rename" :icon="EditPen">重命名</el-dropdown-item>
              <el-dropdown-item command="appearance" :icon="Brush">外观</el-dropdown-item>
              <el-dropdown-item command="delete" :icon="Delete" divided>删除收纳箱</el-dropdown-item>
            </el-dropdown-menu>
          </template>
        </el-dropdown>
      </div>
    </div>

    <div v-show="viewExpanded" class="fence-body">
      <FenceItemGrid
        v-if="fence && items.length > 0"
        :fence-id="fence.id"
        :items="items"
        :icon-size="fence.iconSize"
        @update:items="setItems"
        @update:icon-size="onIconSizeChange"
        @changed="refreshItems"
        @menu-visible="(visible: boolean) => setReason('itemMenu', visible)"
      />
      <div v-else-if="fence" class="fence-empty">
        <el-icon :size="28"><FolderOpened /></el-icon>
        <span>把桌面图标拖到这里</span>
      </div>

      <div v-if="fence && panel === 'appearance'" class="fence-panel">
        <div class="panel-header">
          <span>外观</span>
          <button class="fence-btn" title="关闭" @click="panel = 'none'">
            <el-icon :size="14"><Close /></el-icon>
          </button>
        </div>
        <div class="panel-row">
          <span class="panel-label">背景色</span>
          <div class="color-swatches">
            <button
              v-for="preset in PRESET_BG_COLORS"
              :key="preset.value"
              class="color-swatch"
              :class="{ active: isSameColor(fence.bgColor, preset.value) }"
              :style="{ backgroundColor: preset.value }"
              :title="preset.name"
              @click="setBgColor(preset.value)"
            />
            <label class="color-swatch custom" title="自定义颜色" :style="{ backgroundColor: fence.bgColor }">
              <input type="color" :value="fence.bgColor" @change="onCustomColorChange" />
            </label>
          </div>
        </div>
        <div class="panel-row">
          <span class="panel-label">透明度</span>
          <div class="alpha-slider">
            <el-slider
              :model-value="Math.round(bgAlpha * 100)"
              :min="FENCE_BG_ALPHA_MIN_PERCENT"
              :max="FENCE_BG_ALPHA_MAX_PERCENT"
              :step="5"
              :show-tooltip="false"
              size="small"
              @input="onAlphaInput"
              @change="onAlphaChange"
            />
            <span class="alpha-value">{{ Math.round(bgAlpha * 100) }}%</span>
          </div>
        </div>
      </div>

      <div v-if="fence && panel === 'delete'" class="fence-panel">
        <div class="delete-text">
          <p class="delete-title">删除收纳箱「{{ fence.title }}」？</p>
          <p class="delete-desc">{{ deleteHint }}</p>
        </div>
        <div class="panel-actions">
          <el-button size="small" :disabled="deleting" @click="panel = 'none'">取消</el-button>
          <el-button
            size="small"
            type="danger"
            :loading="deleting"
            :disabled="deleteCount === null"
            @click="confirmDelete"
          >
            删除
          </el-button>
        </div>
      </div>

      <div v-if="panel === 'onedrive'" class="fence-panel">
        <div class="delete-text">
          <p class="delete-title notice-title">
            <el-icon :size="16"><InfoFilled /></el-icon>
            <span>桌面由 OneDrive 同步</span>
          </p>
          <p class="delete-desc">
            收进收纳箱的项目会从桌面移走，移出桌面会同步到你的其他设备（它们的桌面上也会少掉这些图标）。
            项目仍保存在收纳文件夹里，移回桌面即可恢复。
          </p>
        </div>
        <div class="panel-actions">
          <el-button size="small" type="primary" @click="panel = 'none'">知道了</el-button>
        </div>
      </div>

    </div>

    <template v-if="canResize">
      <div
        v-for="direction in RESIZE_HANDLES"
        :key="direction"
        class="resize-handle"
        :class="direction"
        @pointerdown="onResizePointerDown(direction, $event)"
        @pointermove="onResizePointerMove"
        @pointerup="onResizePointerUp"
        @pointercancel="onResizePointerUp"
      />
    </template>
  </div>
</template>

<style scoped>
.fence-window {
  position: relative;
  display: flex;
  flex-direction: column;
  width: 100%;
  height: 100%;
  overflow: hidden;
  color: #fff;
  font-family: -apple-system, 'Segoe UI', 'Microsoft YaHei', sans-serif;
  font-size: 13px;
  background: rgba(var(--fence-bg-rgb), var(--fence-bg-alpha));
  border: 1px solid rgba(255, 255, 255, 0.14);
  border-radius: 8px;
  user-select: none;
}

.fence-titlebar {
  display: flex;
  flex-shrink: 0;
  align-items: center;
  gap: 6px;
  height: var(--fence-title-height);
  padding: 0 4px 0 10px;
  cursor: default;
}

.fence-window:not(.is-collapsed) .fence-titlebar {
  border-bottom: 1px solid rgba(255, 255, 255, 0.08);
}

.fence-title {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  font-weight: 600;
  white-space: nowrap;
  text-overflow: ellipsis;
  text-shadow: 0 1px 2px rgba(0, 0, 0, 0.4);
}

.fence-title-input {
  flex: 1;
  min-width: 0;
}

.fence-title-input :deep(.el-input__inner) {
  user-select: text;
}

.fence-actions {
  display: flex;
  flex-shrink: 0;
  align-items: center;
  gap: 2px;
  opacity: 0.55;
  transition: opacity var(--transition-fast, 150ms);
}

.fence-window:hover .fence-actions,
.fence-actions:focus-within {
  opacity: 1;
}

.fence-btn {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 24px;
  height: 24px;
  padding: 0;
  color: rgba(255, 255, 255, 0.85);
  cursor: pointer;
  background: transparent;
  border: none;
  border-radius: 4px;
  transition: background-color var(--transition-fast, 150ms);
}

.fence-btn:hover {
  background: rgba(255, 255, 255, 0.14);
}

.fence-btn.active {
  color: #60a5fa;
}

.fence-body {
  position: relative;
  flex: 1;
  min-height: 0;
  padding: 8px;
  overflow: hidden;
  border-radius: 0 0 8px 8px;
}

.fence-window.is-drag-hover .fence-body {
  outline: 2px dashed rgba(255, 255, 255, 0.45);
  outline-offset: -6px;
}

.fence-empty {
  display: flex;
  flex-direction: column;
  gap: 8px;
  align-items: center;
  justify-content: center;
  height: 100%;
  color: rgba(255, 255, 255, 0.55);
  text-align: center;
}

.fence-panel {
  position: absolute;
  inset: 0;
  display: flex;
  flex-direction: column;
  gap: 10px;
  padding: 10px 12px;
  overflow: auto;
  background: rgba(15, 23, 42, 0.92);
}

.panel-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  font-weight: 600;
}

.panel-row {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.panel-label {
  font-size: 12px;
  color: rgba(255, 255, 255, 0.7);
}

.color-swatches {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
}

.color-swatch {
  position: relative;
  width: 22px;
  height: 22px;
  padding: 0;
  cursor: pointer;
  border: 2px solid rgba(255, 255, 255, 0.25);
  border-radius: 50%;
}

.color-swatch.active {
  border-color: #fff;
}

.color-swatch.custom {
  overflow: hidden;
  background-image: conic-gradient(#ef4444, #f59e0b, #10b981, #3b82f6, #8b5cf6, #ef4444);
  background-blend-mode: soft-light;
}

.color-swatch.custom input {
  position: absolute;
  inset: 0;
  width: 100%;
  height: 100%;
  cursor: pointer;
  opacity: 0;
}

.alpha-slider {
  display: flex;
  gap: 10px;
  align-items: center;
  padding: 0 4px;
}

.alpha-slider :deep(.el-slider) {
  flex: 1;
}

.alpha-value {
  width: 36px;
  font-size: 12px;
  color: rgba(255, 255, 255, 0.8);
  text-align: right;
}

.delete-text {
  flex: 1;
}

.delete-title {
  margin-bottom: 6px;
  font-weight: 600;
  word-break: break-all;
}

.delete-desc {
  font-size: 12px;
  color: rgba(255, 255, 255, 0.7);
}

.notice-title {
  display: flex;
  gap: 6px;
  align-items: center;
}

.panel-actions {
  display: flex;
  gap: 8px;
  justify-content: flex-end;
}

.resize-handle {
  position: absolute;
  z-index: 10;
}

.resize-handle.n,
.resize-handle.s {
  right: 10px;
  left: 10px;
  height: 5px;
  cursor: ns-resize;
}

.resize-handle.n {
  top: 0;
}

.resize-handle.s {
  bottom: 0;
}

.resize-handle.w,
.resize-handle.e {
  top: 10px;
  bottom: 10px;
  width: 5px;
  cursor: ew-resize;
}

.resize-handle.w {
  left: 0;
}

.resize-handle.e {
  right: 0;
}

.resize-handle.nw,
.resize-handle.ne,
.resize-handle.sw,
.resize-handle.se {
  width: 10px;
  height: 10px;
}

.resize-handle.nw {
  top: 0;
  left: 0;
  cursor: nwse-resize;
}

.resize-handle.se {
  right: 0;
  bottom: 0;
  cursor: nwse-resize;
}

.resize-handle.ne {
  top: 0;
  right: 0;
  cursor: nesw-resize;
}

.resize-handle.sw {
  bottom: 0;
  left: 0;
  cursor: nesw-resize;
}
</style>
