<script setup lang="ts">
import { onMounted, onUnmounted, computed, ref, watch, nextTick, defineAsyncComponent } from 'vue'
import { Plus } from '@element-plus/icons-vue'
import { ElMessage } from '@/plugins/element'
import { useTodoStore, useAppStore } from '@/stores'
import { WebviewWindow } from '@tauri-apps/api/webviewWindow'
import { getCurrentWindow, primaryMonitor, currentMonitor, LogicalSize } from '@tauri-apps/api/window'
import { listen } from '@tauri-apps/api/event'
import { invoke } from '@tauri-apps/api/core'
import TitleBar from '@/components/TitleBar.vue'
import TodoList from '@/components/TodoList.vue'
import QuadrantView from '@/components/QuadrantView.vue'
import type CalendarViewComponent from '@/components/CalendarView.vue'
import type { AppSettingChangedPayload, Todo, SyncSettings, SyncReport } from '@/types'
import { errorMessage, notifyError } from '@/utils/notify'
import {
  describeSkippedRecords,
  describeSyncReport,
  hasLocalDataChanges,
  isSyncBusyError,
} from '@/utils/syncReport'

// 日历默认隐藏：按需加载，农历库 lunar-javascript（约 285KB）不进主窗口首屏的包
const CalendarView = defineAsyncComponent(() => import('@/components/CalendarView.vue'))

const todoStore = useTodoStore()
const appStore = useAppStore()
const appWindow = getCurrentWindow()

// 同步状态
const isSyncing = ref(false)

// 是否显示日历
const showCalendar = computed(() => appStore.showCalendar)

// 当前视图模式
const viewMode = computed(() => todoStore.viewMode)

// 日历组件引用（异步组件的模板 ref 会转发到加载完成的内部组件实例）
const calendarRef = ref<InstanceType<typeof CalendarViewComponent> | null>(null)

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

// 固定模式挂 body 类，供全局样式做描边/底部避让的差异化（仅主窗口有 MainView，天然不会污染子窗口）。
// 嵌入桌面的固定模式例外：它要保留普通模式的圆角与描边观感，不挂这个类
watch(() => appStore.isFixed && !appStore.isEmbeddedInDesktop, (fixedLook) => {
  document.body.classList.toggle('fixed-mode', fixedLook)
}, { immediate: true })

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
let unlistenFontChanged: (() => void) | null = null
let unlistenAppSettings: (() => void) | null = null

// 自动同步定时器
let autoSyncTimer: ReturnType<typeof setInterval> | null = null
// startAutoSync 代次：设置窗口连续改开关 / 间隔会重叠调用，只让最后一次调用装定时器
let autoSyncGeneration = 0
// 自动同步是否处于连续失败中：同一段失败只提示一次，成功后复位
let autoSyncFailing = false
// 最近一次已处理的同步报告：手动同步的返回值与后端 sync-completed 事件携带的是同一份报告，
// 两条路径都会走到 applySyncReport，用它去重避免重复刷新
let lastAppliedSyncReport = ''

// 变更轮询定时器：每 5s 读一次本地变更计数（get_change_seq，极轻量），
// 其它窗口 / 提醒调度 / 同步 / 外部脚本写过库时才全量拉取
let changePollTimer: ReturnType<typeof setInterval> | null = null
const CHANGE_POLL_INTERVAL = 5_000

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

// 新建按钮「操作模式」（issue #9 方案 2）：
// 悬停待办操作按钮持续 0.75s → FAB 半透明沉到操作按钮之下；离开 1.5s 后恢复
const FAB_DIM_ENTER_DELAY = 750
const FAB_DIM_LEAVE_DELAY = 1500
const fabDimmed = ref(false)
let fabEnterTimer: number | null = null
let fabLeaveTimer: number | null = null

function clearFabTimers() {
  if (fabEnterTimer !== null) {
    clearTimeout(fabEnterTimer)
    fabEnterTimer = null
  }
  if (fabLeaveTimer !== null) {
    clearTimeout(fabLeaveTimer)
    fabLeaveTimer = null
  }
}

function cancelFabEnter() {
  if (fabEnterTimer !== null) {
    clearTimeout(fabEnterTimer)
    fabEnterTimer = null
  }
}

function startFabRecovery() {
  if (fabDimmed.value && fabLeaveTimer === null) {
    fabLeaveTimer = window.setTimeout(() => {
      fabLeaveTimer = null
      fabDimmed.value = false
    }, FAB_DIM_LEAVE_DELAY)
  }
}

function handleActionsMouseOver(e: MouseEvent) {
  const target = e.target as Element | null
  if (!target?.closest('.todo-actions')) {
    // 删除/完成会把操作条连同所在行直接移出 DOM，此时不会触发 mouseout，
    // 靠「悬停到非操作按钮区域」兜底：取消进入计时、补启恢复计时
    cancelFabEnter()
    startFabRecovery()
    return
  }
  const from = e.relatedTarget as Element | null
  // 在操作按钮内部移动不重置计时
  if (from?.closest?.('.todo-actions')) return
  if (fabLeaveTimer !== null) {
    clearTimeout(fabLeaveTimer)
    fabLeaveTimer = null
  }
  if (!fabDimmed.value && fabEnterTimer === null) {
    fabEnterTimer = window.setTimeout(() => {
      fabEnterTimer = null
      fabDimmed.value = true
    }, FAB_DIM_ENTER_DELAY)
  }
}

function handleActionsMouseOut(e: MouseEvent) {
  const target = e.target as Element | null
  if (!target?.closest('.todo-actions')) return
  const to = e.relatedTarget as Element | null
  if (to?.closest?.('.todo-actions')) return
  cancelFabEnter()
  startFabRecovery()
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

// 重新加载会被外部改动（设置窗口 / 数据导入 / 云同步）影响的应用设置
async function reloadAppSettings() {
  await todoStore.loadViewMode()
  await appStore.loadShowCalendar()
  await appStore.loadAutoHideEnabled()
  await appStore.loadFixedEmbedDesktop()
  await appStore.loadTopOnWake()
  await appStore.loadWindowBackground()
  await appStore.loadDarkTheme()
  await appStore.loadTodoFontSettings()
}

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
    // 后端已把窗口挪回默认位置与尺寸。重置后需要更新 appStore 状态并退回普通模式
    // （固定模式不允许挪窗口，嵌入桌面同样）
    if (appStore.isFixed) {
      // toggleFixedMode 内部会 saveWindowState，把重置后的几何写进当前屏幕配置
      await appStore.toggleFixedMode()
    } else {
      // 普通模式下 onMoved 的 500ms 防抖还没落库：先把重置后的几何写进当前屏幕配置，
      // 否则下面 initSettings 读到的是旧记录，会把窗口又挪回原处
      if (saveDebounceTimer.value) {
        clearTimeout(saveDebounceTimer.value)
        saveDebounceTimer.value = null
      }
      await appStore.saveWindowState()
    }
    await appStore.initSettings()
  })
  
  unlistenTrayAddTodo = await listen('tray-add-todo', () => {
    openEditor(undefined, true) // 从托盘打开时居中于屏幕
  })

  unlistenTrayOpenSettings = await listen('tray-open-settings', () => {
    openSettings()
  })

  unlistenDataImported = await listen('data-imported', async () => {
    // 导入会覆盖 settings 表，待办和应用设置都要重载（云同步走 sync-completed）
    await todoStore.fetchTodos()
    await reloadAppSettings()
    ElMessage.success('数据导入成功')
  })

  unlistenFontChanged = await listen('todo-font-changed', async () => {
    await appStore.loadTodoFontSettings()
  })

  // 设置窗口改动了应用设置：它有独立的 store 副本，主窗口需要自己重新加载
  unlistenAppSettings = await listen<AppSettingChangedPayload>('app-settings-changed', async (event) => {
    switch (event.payload?.key) {
      case 'showCalendar':
        await appStore.loadShowCalendar()
        break
      case 'autoHide':
        await appStore.loadAutoHideEnabled()
        break
      case 'topOnWake':
        await appStore.loadTopOnWake()
        break
      case 'fixedEmbedDesktop':
        // 窗口本身的切换由后端 set_fixed_embed_desktop 直接完成，这里只同步 store（body 类、保存时的值）
        await appStore.loadFixedEmbedDesktop()
        break
      case 'windowBackground':
        await appStore.loadWindowBackground()
        break
      case 'theme':
        await appStore.loadDarkTheme()
        break
      case 'sync':
        startAutoSync()
        break
      case 'update':
        await appStore.checkForUpdates()
        break
    }
  })

  unlistenFocus = await appWindow.onFocusChanged(async ({ payload: focused }) => {
    if (focused) {
      if (isModalOpen.value) {
        await bringModalToFront()
      } else {
        await todoStore.fetchTodos()
      }
    }
  })

  // 后端同步（手动 / 自动 / 设置窗口里的立即同步与强制覆盖）改动了本地数据或设置时广播，
  // 负载是这次的同步报告
  unlistenSyncCompleted = await listen<SyncReport | null>('sync-completed', async (event) => {
    if (event.payload) {
      await applySyncReport(event.payload)
    } else {
      await todoStore.fetchTodos()
    }
  })

  // 初始化自动同步
  startAutoSync()

  // 启动变更轮询（子窗口打开期间不刷新列表，关窗时会统一刷新）
  changePollTimer = setInterval(() => {
    if (!isModalOpen.value) {
      void todoStore.refreshIfChanged()
    }
  }, CHANGE_POLL_INTERVAL)

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
  if (unlistenFontChanged) unlistenFontChanged()
  if (unlistenAppSettings) unlistenAppSettings()
  if (unlistenFocus) unlistenFocus()
  if (unlistenSyncCompleted) unlistenSyncCompleted()
  // 作废进行中的 startAutoSync（它 await 回来后不会再装定时器）
  autoSyncGeneration++
  stopAutoSync()
  clearFabTimers()
  if (changePollTimer) {
    clearInterval(changePollTimer)
    changePollTimer = null
  }
  if (saveDebounceTimer.value) {
    clearTimeout(saveDebounceTimer.value)
  }
  void reportAutoHideCursorInside(false)
})

// 打开已完成列表窗口（模态）
async function openCompletedWindow() {
  if (isModalOpen.value) return

  const label = `completed-${Date.now()}`
  const winWidth = 460
  const winHeight = 550

  try {
    isModalOpen.value = true

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
      parent: appWindow,
    })
    activeModalWindow = webview

    webview.once('tauri://destroyed', async () => {
      isModalOpen.value = false
      activeModalWindow = null
      await todoStore.fetchTodos()
    })

    webview.once('tauri://error', () => {
      isModalOpen.value = false
      activeModalWindow = null
    })
  } catch (e) {
    isModalOpen.value = false
    activeModalWindow = null
    console.error('Failed to open completed window:', e)
  }
}

// 打开编辑器窗口（模态）
async function openEditor(todo?: Todo, centerOnScreen = false) {
  // 如果已有弹窗打开，直接返回
  if (isModalOpen.value) return
  
  // 已有待办默认进入只读详情，新建直接进入编辑
  const url = todo ? `#/editor?id=${todo.id}&mode=view` : '#/editor'
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
      title: todo ? '待办详情' : '新建待办',
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
    
    const settingsWidth = 680
    const settingsHeight = 560
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
      await reloadAppSettings()
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

// 按同步报告刷新本地视图：待办有增删改才重新拉列表，应用了云端设置才重载设置。
// 同一份报告（手动同步返回值 + 后端 sync-completed 事件）只处理一次
async function applySyncReport(report: SyncReport) {
  const key = JSON.stringify(report)
  if (key === lastAppliedSyncReport) return
  lastAppliedSyncReport = key

  // 与焦点 / 轮询刷新同一约定：子窗口打开期间不动列表，关窗时会统一刷新
  if (hasLocalDataChanges(report) && !isModalOpen.value) {
    await todoStore.fetchTodos()
  }
  if (report.settingsApplied) {
    await reloadAppSettings()
  }
}

// 手动同步（标题栏按钮）：与设置页「立即同步」、自动同步走同一个智能同步命令
async function handleSync() {
  if (isSyncing.value) return
  try {
    isSyncing.value = true
    const settings = await invoke<SyncSettings>('get_sync_settings')
    if (!settings.webdavUrl) {
      ElMessage.warning('请先在设置中配置 WebDAV 服务器')
      return
    }

    // 手动同步总是无条件 GET：nginx 的 ETag 只到秒，同秒等长的远端改写会被条件 GET 的 304 挡住
    const report = await invoke<SyncReport>('webdav_sync', { full: true })
    autoSyncFailing = false
    await applySyncReport(report)

    ElMessage({
      type: report.status === 'no_changes' ? 'info' : 'success',
      message: describeSyncReport(report),
    })
    const skipped = describeSkippedRecords(report)
    if (skipped) ElMessage.warning(skipped)
  } catch (e) {
    if (isSyncBusyError(e)) {
      ElMessage.info('已有同步正在进行，请稍后再试')
    } else {
      notifyError(e, '同步失败')
    }
  } finally {
    isSyncing.value = false
  }
}

// 自动同步的一次执行：失败时同一段连续失败只提示一次，成功后复位
async function runAutoSync() {
  // 手动同步进行中（后端也会以"同步正在进行中"拒绝），跳过这一轮
  if (isSyncing.value) return
  isSyncing.value = true
  try {
    const report = await invoke<SyncReport>('webdav_sync')
    autoSyncFailing = false
    await applySyncReport(report)
    const skipped = describeSkippedRecords(report)
    if (skipped) console.warn('Auto sync:', skipped)
  } catch (e) {
    // 设置窗口里正好在同步：不算失败
    if (isSyncBusyError(e)) return
    console.warn('Auto sync failed:', e)
    if (!autoSyncFailing) {
      autoSyncFailing = true
      const detail = errorMessage(e)
      ElMessage.error(detail ? `自动同步失败：${detail}` : '自动同步失败')
    }
  } finally {
    isSyncing.value = false
  }
}

// 按当前同步设置（重新）装自动同步定时器。
// 读取设置是异步的，重叠调用时只让最后一次生效；清旧定时器与装新定时器都放在 await 之后，
// 避免前一次调用在 await 期间装上的定时器被遗漏（定时器泄漏 → 同步频率翻倍）
async function startAutoSync() {
  const generation = ++autoSyncGeneration
  let settings: SyncSettings
  try {
    settings = await invoke<SyncSettings>('get_sync_settings')
  } catch (e) {
    console.warn('Failed to init auto sync:', e)
    return
  }
  if (generation !== autoSyncGeneration) return

  stopAutoSync()
  if (settings.autoSync && settings.webdavUrl) {
    const intervalMs = Math.max(1, settings.syncInterval || 15) * 60 * 1000
    autoSyncTimer = setInterval(() => {
      void runAutoSync()
    }, intervalMs)
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
    @mouseover="handleActionsMouseOver"
    @mouseout="handleActionsMouseOut"
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

    <!-- 悬浮添加按钮（固定模式同样显示；悬停操作按钮时半透明沉底，见 fab-dimmed） -->
    <button
      class="fab-add"
      :class="{ 'fab-dimmed': fabDimmed }"
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
