<script setup lang="ts">
import { onMounted, onUnmounted, computed, ref, watch, nextTick } from 'vue'
import { ElMessage, ElMessageBox } from 'element-plus'
import { useTodoStore, useAppStore } from '@/stores'
import { WebviewWindow } from '@tauri-apps/api/webviewWindow'
import { getCurrentWindow, primaryMonitor, currentMonitor, LogicalSize } from '@tauri-apps/api/window'
import { listen } from '@tauri-apps/api/event'
import { invoke } from '@tauri-apps/api/core'
import TitleBar from '@/components/TitleBar.vue'
import TodoList from '@/components/TodoList.vue'
import QuadrantView from '@/components/QuadrantView.vue'
import CalendarView from '@/components/CalendarView.vue'
import type { Todo, SyncSettings, SyncDownloadResult } from '@/types'

const todoStore = useTodoStore()
const appStore = useAppStore()
const appWindow = getCurrentWindow()

// 同步状态
const isSyncing = ref(false)

// 是否显示日历
const showCalendar = computed(() => appStore.showCalendar)

// 当前视图模式
const viewMode = computed(() => todoStore.viewMode)

// 日历组件引用
const calendarRef = ref<InstanceType<typeof CalendarView> | null>(null)

// 当前月份文本（从日历组件获取）
const calendarMonthText = computed(() => calendarRef.value?.currentMonthText || '')

// 日历控制方法
function handleCalendarPrev() {
  calendarRef.value?.prevMonth()
}

function handleCalendarNext() {
  calendarRef.value?.nextMonth()
}

function handleCalendarToday() {
  calendarRef.value?.goToToday()
}

// 所有待办（用于日历显示）
const allTodos = computed(() => todoStore.todos)

// 已完成数量
const completedCount = computed(() => todoStore.todoCount.completed)

// 容器类名
const containerClass = computed(() => ({
  'app-container': true,
  'dark-theme': appStore.isDarkTheme
}))

// 事件监听清理函数
let unlistenClose: (() => void) | null = null
let unlistenMoved: (() => void) | null = null
let unlistenResized: (() => void) | null = null
let unlistenTrayToggle: (() => void) | null = null
let unlistenTrayReset: (() => void) | null = null
let unlistenTrayAddTodo: (() => void) | null = null
let unlistenTrayOpenSettings: (() => void) | null = null
let unlistenDataImported: (() => void) | null = null
let unlistenFocus: (() => void) | null = null
let unlistenSyncCompleted: (() => void) | null = null

// 自动同步定时器
let autoSyncTimer: ReturnType<typeof setInterval> | null = null

// 防抖保存定时器
const saveDebounceTimer = ref<number | null>(null)

// 是否有弹窗打开（模态状态）
const isModalOpen = ref(false)
let activeModalWindow: WebviewWindow | null = null

async function bringModalToFront() {
  if (!isModalOpen.value || !activeModalWindow) return
  try {
    await activeModalWindow.setFocus()
  } catch (e) {
    console.warn('Failed to focus modal window:', e)
  }
}

// 防抖保存窗口状态（500ms 后保存）
function debouncedSaveState() {
  if (saveDebounceTimer.value) {
    clearTimeout(saveDebounceTimer.value)
  }
  saveDebounceTimer.value = window.setTimeout(async () => {
    await appStore.saveWindowState()
  }, 500)
}

async function reportAutoHideCursorInside(inside: boolean) {
  try {
    await invoke('set_auto_hide_cursor_inside', { inside })
  } catch {
    // 忽略：该命令仅用于自动隐藏唤起辅助，不影响主流程
  }
}

function handleRootMouseEnter() {
  void reportAutoHideCursorInside(true)
}

function handleRootMouseLeave() {
  void reportAutoHideCursorInside(false)
}

const preCalendarWidth = ref<number | null>(null)
let calendarResizeReady = false

watch(showCalendar, async (show) => {
  if (!calendarResizeReady) return

  try {
    const scale = await appWindow.scaleFactor()
    const size = await appWindow.outerSize()
    const logicalW = size.width / scale
    const logicalH = size.height / scale

    if (show) {
      preCalendarWidth.value = logicalW

      document.documentElement.style.setProperty('--left-panel-width', `${logicalW}px`)

      const titleBarH = 44
      const weekdayH = 30
      const panelPadding = 24
      const gridH = logicalH - titleBarH - weekdayH - panelPadding
      const cellH = gridH / 6
      const gridW = cellH * 7
      const rightPanelW = gridW + panelPadding
      const newW = Math.round(logicalW + rightPanelW)
      await appWindow.setSize(new LogicalSize(newW, logicalH))
    } else {
      document.documentElement.style.removeProperty('--left-panel-width')
      if (preCalendarWidth.value) {
        await appWindow.setSize(new LogicalSize(preCalendarWidth.value, logicalH))
        preCalendarWidth.value = null
      }
    }
  } catch (e) {
    console.error('Failed to resize window for calendar:', e)
  }
})

// 初始化
onMounted(async () => {
  await appStore.initSettings()
  await todoStore.fetchTodos()
  await todoStore.loadViewMode()

  await nextTick()
  calendarResizeReady = true
  
  // 异步检查版本更新（不阻塞主流程）
  appStore.checkForUpdates()
  
  // 监听窗口关闭请求，保存状态
  unlistenClose = await appWindow.onCloseRequested(async () => {
    await appStore.saveWindowState()
  })
  
  // 监听窗口移动事件，自动保存状态（防抖）
  unlistenMoved = await appWindow.onMoved(() => {
    debouncedSaveState()
  })
  
  // 监听窗口调整尺寸事件，自动保存状态（防抖）
  unlistenResized = await appWindow.onResized(() => {
    debouncedSaveState()
  })
  
  // 监听托盘菜单事件
  unlistenTrayToggle = await listen('tray-toggle-fixed', async () => {
    await appStore.toggleFixedMode()
  })
  
  unlistenTrayReset = await listen('tray-reset-window', async () => {
    // 重置后需要更新 appStore 状态并取消固定模式
    if (appStore.isFixed) {
      await appStore.toggleFixedMode()
    }
    await appStore.initSettings()
  })
  
  unlistenTrayAddTodo = await listen('tray-add-todo', () => {
    openEditor(undefined, true) // 从托盘打开时居中于屏幕
  })

  unlistenTrayOpenSettings = await listen('tray-open-settings', () => {
    openSettings()
  })

  unlistenDataImported = await listen('data-imported', () => {
    ElMessage.success('数据导入成功')
  })

  unlistenFocus = await appWindow.onFocusChanged(async ({ payload: focused }) => {
    if (focused && isModalOpen.value) {
      await bringModalToFront()
    }
  })

  // 监听同步完成事件
  unlistenSyncCompleted = await listen('sync-completed', async () => {
    await todoStore.fetchTodos()
  })

  // 初始化自动同步
  startAutoSync()

  // 初始化鼠标在窗口内状态（用于 macOS 自动隐藏唤起）
  void reportAutoHideCursorInside(true)
})

// 清理
onUnmounted(() => {
  if (unlistenClose) unlistenClose()
  if (unlistenMoved) unlistenMoved()
  if (unlistenResized) unlistenResized()
  if (unlistenTrayToggle) unlistenTrayToggle()
  if (unlistenTrayReset) unlistenTrayReset()
  if (unlistenTrayAddTodo) unlistenTrayAddTodo()
  if (unlistenTrayOpenSettings) unlistenTrayOpenSettings()
  if (unlistenDataImported) unlistenDataImported()
  if (unlistenFocus) unlistenFocus()
  if (unlistenSyncCompleted) unlistenSyncCompleted()
  stopAutoSync()
  if (saveDebounceTimer.value) {
    clearTimeout(saveDebounceTimer.value)
  }
  void reportAutoHideCursorInside(false)
})

// 打开已完成列表窗口
async function openCompletedWindow() {
  const label = `completed-${Date.now()}`
  const winWidth = 460
  const winHeight = 550

  try {
    let x: number, y: number
    const monitor = await currentMonitor() || await primaryMonitor()
    if (monitor) {
      const s = monitor.scaleFactor
      const mx = monitor.position.x / s
      const my = monitor.position.y / s
      const mw = monitor.size.width / s
      const mh = monitor.size.height / s
      x = Math.round(mx + (mw - winWidth) / 2)
      y = Math.round(my + (mh - winHeight) / 2)
    } else {
      const s = await appWindow.scaleFactor()
      const pos = await appWindow.outerPosition()
      const size = await appWindow.outerSize()
      x = Math.round(pos.x / s + (size.width / s - winWidth) / 2)
      y = Math.round(pos.y / s + (size.height / s - winHeight) / 2)
    }

    const webview = new WebviewWindow(label, {
      url: '#/completed',
      title: '已完成',
      width: winWidth,
      height: winHeight,
      x,
      y,
      resizable: true,
      decorations: false,
      transparent: false,
    })

    webview.once('tauri://destroyed', async () => {
      await todoStore.fetchTodos()
    })
  } catch (e) {
    console.error('Failed to open completed window:', e)
  }
}

// 打开编辑器窗口（模态）
async function openEditor(todo?: Todo, centerOnScreen = false) {
  // 如果已有弹窗打开，直接返回
  if (isModalOpen.value) return
  
  const url = todo ? `#/editor?id=${todo.id}` : '#/editor'
  const label = `editor-${Date.now()}`
  
  try {
    isModalOpen.value = true
    
    const editorWidth = 1080 // 700 + 380（包含子任务面板）
    const editorHeight = 600
    let x: number, y: number
    
    // 始终在当前激活的屏幕居中打开
    const monitor = await currentMonitor() || await primaryMonitor()
    if (monitor) {
      const monitorScale = monitor.scaleFactor
      const monitorX = monitor.position.x / monitorScale
      const monitorY = monitor.position.y / monitorScale
      const monitorW = monitor.size.width / monitorScale
      const monitorH = monitor.size.height / monitorScale
      x = Math.round(monitorX + (monitorW - editorWidth) / 2)
      y = Math.round(monitorY + (monitorH - editorHeight) / 2)
    } else {
      // fallback: 使用主窗口中心
      const scaleFactor = await appWindow.scaleFactor()
      const mainPos = await appWindow.outerPosition()
      const mainSize = await appWindow.outerSize()
      const mainX = mainPos.x / scaleFactor
      const mainY = mainPos.y / scaleFactor
      const mainW = mainSize.width / scaleFactor
      const mainH = mainSize.height / scaleFactor
      x = Math.round(mainX + (mainW - editorWidth) / 2)
      y = Math.round(mainY + (mainH - editorHeight) / 2)
    }
    
    const webview = new WebviewWindow(label, {
      url,
      title: todo ? '编辑待办' : '新建待办',
      width: editorWidth,
      height: editorHeight,
      x,
      y,
      resizable: true,
      decorations: false,
      transparent: false,
      parent: centerOnScreen ? undefined : appWindow
    })
    activeModalWindow = webview

    // 监听窗口关闭，刷新待办列表并清除模态状态
    webview.once('tauri://destroyed', async () => {
      isModalOpen.value = false
      activeModalWindow = null
      await todoStore.fetchTodos()
    })
    
    // 监听创建失败，清除模态状态
    webview.once('tauri://error', () => {
      isModalOpen.value = false
      activeModalWindow = null
    })
  } catch (e) {
    isModalOpen.value = false
    activeModalWindow = null
    console.error('Failed to open editor window:', e)
  }
}

// 打开设置窗口（模态）
async function openSettings() {
  // 如果已有弹窗打开，直接返回
  if (isModalOpen.value) return
  
  const label = `settings-${Date.now()}`
  
  try {
    isModalOpen.value = true
    
    const settingsWidth = 480
    const settingsHeight = 720
    let x: number, y: number
    
    const monitor = await currentMonitor() || await primaryMonitor()
    if (monitor) {
      const s = monitor.scaleFactor
      const mx = monitor.position.x / s
      const my = monitor.position.y / s
      const mw = monitor.size.width / s
      const mh = monitor.size.height / s
      x = Math.round(mx + (mw - settingsWidth) / 2)
      y = Math.round(my + (mh - settingsHeight) / 2)
    } else {
      const scaleFactor = await appWindow.scaleFactor()
      const mainPos = await appWindow.outerPosition()
      const mainSize = await appWindow.outerSize()
      x = Math.round(mainPos.x / scaleFactor + (mainSize.width / scaleFactor - settingsWidth) / 2)
      y = Math.round(mainPos.y / scaleFactor + (mainSize.height / scaleFactor - settingsHeight) / 2)
    }
    
    const webview = new WebviewWindow(label, {
      url: '#/settings',
      title: '设置',
      width: settingsWidth,
      height: settingsHeight,
      x,
      y,
      resizable: false,
      decorations: false,
      transparent: false,
      parent: appWindow
    })
    activeModalWindow = webview
    
    // 监听窗口关闭，清除模态状态并重新加载设置和数据
    webview.once('tauri://destroyed', async () => {
      isModalOpen.value = false
      activeModalWindow = null
      await todoStore.fetchTodos()
      await todoStore.loadViewMode()
      await appStore.loadShowCalendar()
      startAutoSync()
    })
    
    // 监听创建失败，清除模态状态
    webview.once('tauri://error', () => {
      isModalOpen.value = false
      activeModalWindow = null
    })
  } catch (e) {
    isModalOpen.value = false
    activeModalWindow = null
    console.error('Failed to open settings window:', e)
  }
}

async function handleSync() {
  if (isSyncing.value) return
  try {
    isSyncing.value = true
    const settings = await invoke<SyncSettings>('get_sync_settings')
    if (!settings.webdavUrl) {
      ElMessage.warning('请先在设置中配置 WebDAV 服务器')
      return
    }

    const result = await invoke<SyncDownloadResult>('webdav_download_sync')

    if (result.hasRemote && result.hasConflict) {
      try {
        const action = await ElMessageBox.confirm(
          `本地和云端数据均有更新，请选择操作：`,
          '同步冲突',
          {
            confirmButtonText: '使用云端数据',
            cancelButtonText: '保留本地数据',
            distinguishCancelAndClose: true,
            type: 'warning',
          }
        )
        if (action === 'confirm' && result.remoteData) {
          await invoke<string>('webdav_apply_remote', {
            syncDataJson: JSON.stringify(result.remoteData),
          })
          await todoStore.fetchTodos()
          ElMessage.success('已同步云端数据到本地')
        } else {
          await invoke<string>('webdav_upload_sync')
          ElMessage.success('已上传本地数据到云端')
        }
      } catch (e) {
        if (e === 'cancel') {
          await invoke<string>('webdav_upload_sync')
          ElMessage.success('已上传本地数据到云端')
        }
      }
    } else if (result.hasRemote && result.remoteData) {
      const remoteIsNewer = result.remoteUpdatedAt && result.localUpdatedAt
        ? result.remoteUpdatedAt > result.localUpdatedAt
        : !!result.remoteUpdatedAt

      if (remoteIsNewer) {
        await invoke<string>('webdav_apply_remote', {
          syncDataJson: JSON.stringify(result.remoteData),
        })
        await todoStore.fetchTodos()
      }
      await invoke<string>('webdav_upload_sync')
      ElMessage.success('同步完成')
    } else {
      await invoke<string>('webdav_upload_sync')
      ElMessage.success('数据已上传到云端')
    }
  } catch (e) {
    ElMessage.error('同步失败: ' + String(e))
  } finally {
    isSyncing.value = false
  }
}

async function startAutoSync() {
  stopAutoSync()
  try {
    const settings = await invoke<SyncSettings>('get_sync_settings')
    if (settings.autoSync && settings.webdavUrl) {
      const intervalMs = (settings.syncInterval || 15) * 60 * 1000
      autoSyncTimer = setInterval(async () => {
        try {
          const result = await invoke<string>('webdav_auto_sync')
          if (result === 'conflict') {
            console.log('Auto sync: conflict detected, skipping')
          } else if (result !== 'no_changes') {
            await todoStore.fetchTodos()
          }
        } catch (e) {
          console.warn('Auto sync failed:', e)
        }
      }, intervalMs)
    }
  } catch (e) {
    console.warn('Failed to init auto sync:', e)
  }
}

function stopAutoSync() {
  if (autoSyncTimer) {
    clearInterval(autoSyncTimer)
    autoSyncTimer = null
  }
}
</script>

<template>
  <div
    :class="[containerClass, { 'with-calendar': showCalendar }]"
    @mouseenter="handleRootMouseEnter"
    @mouseleave="handleRootMouseLeave"
  >
    <!-- 模态遮罩层 -->
    <div v-if="isModalOpen" class="modal-overlay" @mousedown="bringModalToFront"></div>
    
    <!-- 标题栏 -->
    <TitleBar 
      :show-calendar-controls="showCalendar"
      :current-month-text="calendarMonthText"
      :completed-count="completedCount"
      :syncing="isSyncing"
      @open-settings="openSettings"
      @open-completed="openCompletedWindow"
      @calendar-prev="handleCalendarPrev"
      @calendar-next="handleCalendarNext"
      @calendar-today="handleCalendarToday"
      @sync="handleSync"
    />

    <!-- 主内容区 - 分栏布局 -->
    <div class="main-body" :class="{ 'split-layout': showCalendar }">
      <!-- 左侧：待办列表/四象限视图 -->
      <div class="left-panel">
        <div class="main-content">
          <!-- 列表视图 -->
          <TodoList
            v-if="viewMode === 'list'"
            @edit="openEditor"
          />
          <!-- 四象限视图 -->
          <QuadrantView
            v-else
            @edit="openEditor"
          />
        </div>
      </div>

      <!-- 已完成列表（独立窗口） -->

      <!-- 右侧：日历视图 -->
      <div v-if="showCalendar" class="right-panel" :class="{ 'dark-theme': appStore.isDarkTheme }">
        <CalendarView
          ref="calendarRef"
          :todos="allTodos"
          :is-dark-theme="appStore.isDarkTheme"
          @select-todo="openEditor"
        />
      </div>
    </div>

    <!-- 悬浮添加按钮（固定模式下隐藏） -->
    <button 
      v-if="!appStore.isFixed" 
      class="fab-add" 
      title="新建待办" 
      @click="openEditor()"
    >
      <el-icon :size="24"><Plus /></el-icon>
    </button>
  </div>
</template>

<style scoped>
.modal-overlay {
  position: fixed;
  top: 0;
  left: 0;
  right: 0;
  bottom: 0;
  background: rgba(0, 0, 0, 0.3);
  z-index: 999;
  cursor: not-allowed;
}

/* 分栏布局 */
.main-body {
  flex: 1;
  display: flex;
  flex-direction: column;
  overflow: hidden;

  &.split-layout {
    flex-direction: row;
  }
}

.left-panel {
  display: flex;
  flex-direction: column;
  overflow: hidden;

  .split-layout & {
    width: var(--left-panel-width, 40%);
    min-width: 280px;
    flex-shrink: 0;
  }
}

.right-panel {
  flex: 1;
  overflow: hidden;
  padding: 12px;
  background: transparent;

  &.dark-theme {
    background: transparent;
    padding: 8px;
  }
}

/* 已完成按钮 */
.completed-btn-wrapper {
  padding: 8px 16px;

  &.dark-theme {
    background-color: rgba(0, 0, 0, 0.15);
  }
}

.completed-btn {
  display: flex;
  align-items: center;
  justify-content: space-between;
  width: 100%;
  padding: 8px 12px;
  background: transparent;
  border: 1px solid rgba(128, 128, 128, 0.2);
  border-radius: 6px;
  color: var(--text-secondary);
  font-size: 13px;
  cursor: pointer;
  transition: all 0.2s;

  &:hover {
    background: rgba(128, 128, 128, 0.1);
    color: var(--text-primary);
  }
}

</style>
