<script setup lang="ts">
import { ref, computed, onMounted, onBeforeUnmount, reactive } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { emit, listen, type UnlistenFn } from '@tauri-apps/api/event'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { save, open } from '@tauri-apps/plugin-dialog'
import { readTextFile } from '@tauri-apps/plugin-fs'
import { openUrl } from '@tauri-apps/plugin-opener'
import { enable, disable, isEnabled } from '@tauri-apps/plugin-autostart'
import { ElMessage, ElMessageBox } from 'element-plus'
import { useAppStore, APP_VERSION } from '@/stores'
import type { AppSettingKey, ScreenConfig, SyncSettings, SyncReport } from '@/types'
import { PRESET_BG_COLORS, DEFAULT_BG_COLOR } from '@/types'
import { isSameColor } from '@/utils/color'
import { formatDateKey, formatDateTime } from '@/utils/datetime'
import { notifyError } from '@/utils/notify'
import { describeSkippedRecords, describeSyncReport, isSyncBusyError } from '@/utils/syncReport'

const appWindow = getCurrentWindow()
const appStore = useAppStore()

// 当前激活的菜单
const activeMenu = ref('general')

const menuItems = [
  { key: 'general', label: '常规', icon: 'Setting' },
  { key: 'appearance', label: '外观', icon: 'Brush' },
  { key: 'data', label: '数据与同步', icon: 'Folder' },
  { key: 'screen', label: '屏幕配置', icon: 'Monitor' },
  { key: 'about', label: '关于', icon: 'InfoFilled' },
]

const exporting = ref(false)
const importing = ref(false)
const checking = ref(false)
const autoStart = ref(false)
const autoStartLoading = ref(false)

// 通知类型设置
const notificationType = ref<'system' | 'app'>('system')
const notificationTypeLoading = ref(false)

// 屏幕配置相关
const screenConfigs = computed(() => appStore.screenConfigs)
const currentConfigId = computed(() => appStore.currentScreenConfigId)

// 日历显示
const showCalendar = computed(() => appStore.showCalendar)
// 贴边自动隐藏
const autoHideEnabled = computed(() => appStore.autoHideEnabled)
// 贴边唤起时置顶
const topOnWake = computed(() => appStore.topOnWake)
// 固定模式时嵌入桌面（仅 Windows 可见：实现依赖 Win32 桌面宿主，其它平台后端直接拒绝）
const fixedEmbedDesktop = computed(() => appStore.fixedEmbedDesktop)
const isWindows = /windows/i.test(navigator.userAgent)
// 窗口底色与背景透明度
const windowBgColor = computed(() => appStore.windowBgColor)
const windowBgAlpha = computed(() => appStore.windowBgAlpha)

// 是否有更新
const hasUpdate = computed(() => appStore.hasUpdate)
const latestVersion = computed(() => appStore.latestVersion)

// 字体设置
const systemFonts = ref<string[]>([])
const fontFamily = ref('')
const fontSize = ref(14)

let unlistenSyncCompleted: UnlistenFn | null = null

// 读取本窗口展示的应用设置（打开时一次；同步应用了云端设置后再刷新一次）
async function loadAppSettingsForDisplay() {
  try {
    const type = await invoke<string>('get_notification_type')
    notificationType.value = type === 'app' ? 'app' : 'system'
  } catch (e) {
    console.error('Failed to get notification type:', e)
  }

  await appStore.loadShowCalendar()
  await appStore.loadAutoHideEnabled()
  await appStore.loadTopOnWake()
  await appStore.loadFixedEmbedDesktop()
  await appStore.loadWindowBackground()
  await appStore.loadDarkTheme()

  try {
    fontFamily.value = await invoke<string>('get_todo_font_family')
    fontSize.value = await invoke<number>('get_todo_font_size')
  } catch (e) {
    console.error('Failed to load font settings:', e)
  }
}

onMounted(async () => {
  try {
    autoStart.value = await isEnabled()
  } catch (e) {
    console.error('Failed to get autostart status:', e)
  }

  await appStore.loadScreenConfigs()
  await loadAppSettingsForDisplay()
  await loadSyncSettings()

  // 加载系统字体列表
  try {
    systemFonts.value = await invoke<string[]>('get_system_fonts')
  } catch (e) {
    console.error('Failed to load system fonts:', e)
  }

  // 任意窗口触发的同步（含主窗口自动同步）完成后：刷新"上次同步"时间；
  // 应用了云端设置时本窗口显示的设置也已过时，重新读取
  unlistenSyncCompleted = await listen<SyncReport | null>('sync-completed', async (event) => {
    const report = event.payload
    if (!report) return
    syncSettings.lastSyncAt = report.lastSyncAt
    if (report.settingsApplied) {
      await loadAppSettingsForDisplay()
    }
  })
})

onBeforeUnmount(() => {
  unlistenSyncCompleted?.()
  unlistenSyncCompleted = null
})

// 设置窗口是独立 WebView，与主窗口不共享 Pinia 状态。
// 任何改动都要发事件，让主窗口重新从数据库加载对应设置
async function notifyAppSettingChanged(key: AppSettingKey) {
  try {
    await emit('app-settings-changed', { key })
  } catch (e) {
    console.error('Failed to notify app setting change:', e)
  }
}

async function handleShowCalendarChange(val: boolean) {
  await appStore.setShowCalendar(val)
  await notifyAppSettingChanged('showCalendar')
}

async function handleAutoHideChange(val: boolean) {
  await appStore.setAutoHideEnabled(val)
  await notifyAppSettingChanged('autoHide')
}

async function handleTopOnWakeChange(val: boolean) {
  await appStore.setTopOnWake(val)
  await notifyAppSettingChanged('topOnWake')
}

async function handleFixedEmbedDesktopChange(val: boolean) {
  await appStore.setFixedEmbedDesktop(val)
  await notifyAppSettingChanged('fixedEmbedDesktop')
}

async function handleBgColorChange(color: string) {
  await appStore.setWindowBackground(color, appStore.windowBgAlpha)
  await notifyAppSettingChanged('windowBackground')
}

// 拖动过程中只更新本地显示，避免每一格都写一次库
function handleBgAlphaInput(val: number) {
  appStore.windowBgAlpha = val / 100
}

async function handleBgAlphaChange(val: number) {
  await appStore.setWindowBackground(appStore.windowBgColor, val / 100)
  await notifyAppSettingChanged('windowBackground')
}

async function handleDarkThemeChange(val: boolean) {
  await appStore.setDarkTheme(val)
  await notifyAppSettingChanged('theme')
}

// 字体变更
async function handleFontFamilyChange(val: string) {
  fontFamily.value = val
  try {
    await invoke('set_todo_font_family', { fontFamily: val })
    await emit('todo-font-changed')
  } catch (e) {
    console.error('Failed to save font family:', e)
  }
}

async function handleFontSizePersist(val: number) {
  try {
    await invoke('set_todo_font_size', { fontSize: val })
    await emit('todo-font-changed')
  } catch (e) {
    console.error('Failed to save font size:', e)
  }
}

// 删除屏幕配置
async function handleDeleteConfig(config: ScreenConfig) {
  if (config.configId === currentConfigId.value) {
    ElMessage.warning('不能删除当前正在使用的屏幕配置')
    return
  }

  try {
    await ElMessageBox.confirm(
      `确定删除屏幕配置 "${config.displayName || config.configId}" 吗？`,
      '删除确认',
      {
        confirmButtonText: '删除',
        cancelButtonText: '取消',
        type: 'warning'
      }
    )

    const success = await appStore.deleteScreenConfig(config.configId)
    if (success) {
      ElMessage.success('删除成功')
    } else {
      ElMessage.error('删除失败')
    }
  } catch (e) {
    // 用户取消
  }
}

function formatConfigInfo(configId: string): string {
  if (configId === 'legacy') return '旧版本迁移的配置'
  if (configId === 'unknown') return '未知屏幕配置'

  const parts = configId.split('_')
  if (parts.length < 2) return configId

  const count = parts[0]
  const monitors = parts.slice(1).map(p => {
    const [res, scale] = p.split('@')
    return `${res} ${scale}%`
  })

  return `${count} 个显示器: ${monitors.join(', ')}`
}

async function handleAutoStartChange(value: boolean) {
  try {
    autoStartLoading.value = true
    if (value) {
      await enable()
      ElMessage.success('已开启开机自启')
    } else {
      await disable()
      ElMessage.success('已关闭开机自启')
    }
    autoStart.value = value
    // 托盘菜单是另一个入口，不同步勾选会与这里的开关状态反转。
    // 单独捕获：走到这里注册表已改成功，托盘勾选同步失败不应把开关回滚成「设置失败」
    invoke('sync_auto_start_state', { enabled: value }).catch((e) => {
      console.error('Failed to sync tray autostart state:', e)
    })
  } catch (e) {
    console.error('Failed to toggle autostart:', e)
    ElMessage.error('设置开机自启失败')
    autoStart.value = !value
  } finally {
    autoStartLoading.value = false
  }
}

async function handleNotificationTypeChange(value: 'system' | 'app') {
  const oldValue = notificationType.value
  try {
    notificationTypeLoading.value = true
    notificationType.value = value
    await invoke('set_notification_type', { notificationType: value })
    ElMessage.success(value === 'system' ? '已切换为系统通知' : '已切换为软件通知')
  } catch (e) {
    console.error('Failed to set notification type:', e)
    ElMessage.error('设置通知类型失败')
    notificationType.value = oldValue
  } finally {
    notificationTypeLoading.value = false
  }
}

async function handleExport() {
  try {
    exporting.value = true

    const filePath = await save({
      title: '导出待办数据',
      // 用本地日期：toISOString 是 UTC，东八区凌晨导出会带上前一天的日期
      defaultPath: `mini-todo-backup-${formatDateKey(new Date())}.zip`,
      filters: [{
        name: 'ZIP 压缩包',
        extensions: ['zip']
      }]
    })

    if (filePath) {
      await invoke('export_data_to_file', { filePath })
      ElMessage.success('导出成功')
    }
  } catch (e) {
    console.error('Export error:', e)
    ElMessage.error('导出失败: ' + String(e))
  } finally {
    exporting.value = false
  }
}

async function handleImport() {
  try {
    const filePath = await open({
      title: '导入待办数据',
      filters: [
        { name: 'ZIP / JSON 文件', extensions: ['zip', 'json'] },
      ]
    })

    if (!filePath) return

    await ElMessageBox.confirm(
      '导入将覆盖现有的所有待办数据，确定继续吗？',
      '导入确认',
      {
        confirmButtonText: '确定导入',
        cancelButtonText: '取消',
        type: 'warning'
      }
    )

    importing.value = true

    const path = filePath as string
    if (path.endsWith('.zip')) {
      await invoke('import_data_from_file', { filePath: path })
    } else {
      const jsonData = await readTextFile(path)
      await invoke('import_data', { jsonData })
    }

    await emit('data-imported')
    handleClose()
  } catch (e) {
    if (String(e) !== 'cancel') {
      console.error('Import error:', e)
      ElMessage.error('导入失败: ' + String(e))
    }
  } finally {
    importing.value = false
  }
}

function handleClose() {
  appWindow.close()
}

function onHeaderMouseDown(e: MouseEvent) {
  if (e.buttons !== 1) return
  const target = e.target as HTMLElement
  if (target.closest('[data-tauri-drag-region="false"]')) return
  if (target.closest('button, input, textarea, select, a, [role="button"]')) return
  e.preventDefault()
  appWindow.startDragging()
}

// ========== WebDAV 同步 ==========
const syncSettings = reactive<SyncSettings>({
  webdavUrl: '',
  webdavUsername: '',
  webdavPassword: '',
  hasPassword: false,
  autoSync: false,
  syncInterval: 15,
  lastSyncAt: null,
  deviceId: '',
})
const showPassword = ref(false)
const testingConnection = ref(false)
const clearingPassword = ref(false)
// 当前进行中的同步动作（立即同步 / 用云端覆盖本地 / 用本地覆盖云端）
const syncAction = ref<'sync' | 'pull' | 'push' | null>(null)
const showAdvancedSync = ref(false)

// http:// 地址：账号密码与待办数据都会明文经过网络
const isInsecureWebdavUrl = computed(() => /^http:\/\//i.test(syncSettings.webdavUrl.trim()))

const passwordPlaceholder = computed(() =>
  syncSettings.hasPassword ? '已保存（留空则不修改）' : '密码'
)

const syncIntervalOptions = [
  { label: '5 分钟', value: 5 },
  { label: '10 分钟', value: 10 },
  { label: '15 分钟', value: 15 },
  { label: '30 分钟', value: 30 },
  { label: '60 分钟', value: 60 },
]

async function loadSyncSettings() {
  try {
    const settings = await invoke<SyncSettings>('get_sync_settings')
    Object.assign(syncSettings, settings)
    // 后端不回传已保存的密码；输入框只用来输入新密码
    syncSettings.webdavPassword = ''
  } catch (e) {
    console.error('Failed to load sync settings:', e)
  }
}

// 保存同步配置。密码框留空 = 保留已保存的密码；clearPassword 显式清空
async function persistSyncSettings(clearPassword = false): Promise<boolean> {
  try {
    const settings: SyncSettings = {
      ...syncSettings,
      webdavPassword: clearPassword ? '' : syncSettings.webdavPassword,
      clearPassword,
    }
    await invoke('save_sync_settings', { settings })
    // 主窗口持有自动同步定时器，需按新的开关/间隔重建
    await notifyAppSettingChanged('sync')
    // 重新读取：刷新 hasPassword，并清空密码输入框
    await loadSyncSettings()
    return true
  } catch (e) {
    notifyError(e, '保存同步设置失败')
    return false
  }
}

async function saveSyncSettings() {
  if (await persistSyncSettings()) {
    ElMessage.success('同步设置已保存')
  }
}

async function handleClearPassword() {
  try {
    await ElMessageBox.confirm(
      '确定清除已保存的 WebDAV 密码吗？清除后需要重新输入密码才能同步。',
      '清除密码',
      { confirmButtonText: '清除', cancelButtonText: '取消', type: 'warning' }
    )
  } catch {
    return
  }
  clearingPassword.value = true
  try {
    if (await persistSyncSettings(true)) {
      ElMessage.success('已清除保存的密码')
    }
  } finally {
    clearingPassword.value = false
  }
}

async function testConnection() {
  if (!syncSettings.webdavUrl) {
    ElMessage.warning('请输入 WebDAV 服务器地址')
    return
  }
  try {
    testingConnection.value = true
    await invoke<boolean>('webdav_test_connection', {
      url: syncSettings.webdavUrl,
      username: syncSettings.webdavUsername,
      // 留空时后端使用已保存的密码
      password: syncSettings.webdavPassword || null,
    })
    ElMessage.success('连接成功')
  } catch (e) {
    notifyError(e, '连接失败')
  } finally {
    testingConnection.value = false
  }
}

// 执行一次同步命令。后端成功后会向所有窗口广播 sync-completed，
// 主窗口据此刷新列表与设置，这里只负责提示与"上次同步"时间
async function runSyncCommand(
  action: 'sync' | 'pull' | 'push',
  command: 'webdav_sync' | 'webdav_force_pull' | 'webdav_force_push',
  failureText: string
) {
  if (!syncSettings.webdavUrl) {
    ElMessage.warning('请先配置 WebDAV 服务器')
    return
  }
  if (syncAction.value) return
  syncAction.value = action
  try {
    const report = await invoke<SyncReport>(command)
    syncSettings.lastSyncAt = report.lastSyncAt
    ElMessage({
      type: report.status === 'no_changes' ? 'info' : 'success',
      message: describeSyncReport(report),
    })
    const skipped = describeSkippedRecords(report)
    if (skipped) ElMessage.warning(skipped)
    if (report.settingsApplied) {
      await loadAppSettingsForDisplay()
    }
  } catch (e) {
    if (isSyncBusyError(e)) {
      ElMessage.info('已有同步正在进行，请稍后再试')
    } else {
      notifyError(e, failureText)
    }
  } finally {
    syncAction.value = null
  }
}

// 立即同步：合并本地与云端的更改（含删除），与主窗口同步按钮、自动同步同一命令
async function handleSyncNow() {
  await runSyncCommand('sync', 'webdav_sync', '同步失败')
}

// 高级：用云端覆盖本地
async function handleForcePull() {
  try {
    await ElMessageBox.confirm(
      '将让本地数据与云端完全一致：本地有而云端没有的待办和子任务会被删除，' +
        '应用设置也会被云端覆盖（窗口位置等设备相关设置除外）。此操作无法撤销，建议先导出备份。确定继续吗？',
      '用云端覆盖本地',
      { confirmButtonText: '覆盖本地', cancelButtonText: '取消', type: 'warning' }
    )
  } catch {
    return
  }
  await runSyncCommand('pull', 'webdav_force_pull', '用云端覆盖本地失败')
}

// 高级：用本地覆盖云端
async function handleForcePush() {
  try {
    await ElMessageBox.confirm(
      '将让云端数据与本地完全一致：云端有而本地没有的待办和子任务会被删除，' +
        '其它设备与云端服务下次同步时也会随之删除。此操作无法撤销。确定继续吗？',
      '用本地覆盖云端',
      { confirmButtonText: '覆盖云端', cancelButtonText: '取消', type: 'warning' }
    )
  } catch {
    return
  }
  await runSyncCommand('push', 'webdav_force_push', '用本地覆盖云端失败')
}

// "上次同步"时间（元信息，可能带时区偏移）显示为本地时间
function formatTime(time: string | null | undefined): string {
  if (!time) return '未知'
  return formatDateTime(time, 'YYYY-MM-DD HH:mm') ?? time
}

async function handleCheckUpdate() {
  try {
    checking.value = true
    await appStore.checkForUpdates()

    if (hasUpdate.value) {
      // 主窗口标题栏的更新红点由它自己的 store 驱动，需要它也刷新一次
      await notifyAppSettingChanged('update')
      await ElMessageBox.confirm(
        `发现新版本 ${latestVersion.value}，是否前往下载？`,
        '版本更新',
        {
          confirmButtonText: '前往下载',
          cancelButtonText: '稍后再说',
          type: 'info'
        }
      )
      await openUrl(appStore.getReleasesUrl())
    } else {
      ElMessage.success('当前已是最新版本')
    }
  } catch (e) {
    if (String(e) !== 'cancel') {
      ElMessage.info('检查更新失败，请稍后重试')
    }
  } finally {
    checking.value = false
  }
}
</script>

<template>
  <div class="settings-window">
    <div class="window-header" data-tauri-drag-region="deep" @mousedown="onHeaderMouseDown">
      <h2>设置</h2>
      <el-button text data-tauri-drag-region="false" @click="handleClose">
        <el-icon><Close /></el-icon>
      </el-button>
    </div>

    <div class="settings-body">
      <!-- 左侧菜单 -->
      <div class="settings-menu">
        <div
          v-for="item in menuItems"
          :key="item.key"
          class="menu-item"
          :class="{ active: activeMenu === item.key }"
          @click="activeMenu = item.key"
        >
          <el-icon :size="16">
            <component :is="item.icon" />
          </el-icon>
          <span>{{ item.label }}</span>
        </div>
      </div>

      <!-- 右侧内容区 -->
      <div class="settings-content">
        <!-- 常规 -->
        <div v-show="activeMenu === 'general'" class="panel">
          <h3 class="panel-title">常规</h3>

          <div class="settings-row">
            <div class="row-left">
              <el-icon class="row-icon"><Monitor /></el-icon>
              <span class="settings-label">开机自启</span>
            </div>
            <el-switch
              v-model="autoStart"
              :loading="autoStartLoading"
              @change="handleAutoStartChange"
            />
          </div>

          <div class="settings-row">
            <div class="row-left">
              <el-icon class="row-icon"><Calendar /></el-icon>
              <div class="row-content">
                <span class="settings-label">展示日历</span>
                <span class="settings-desc">开启后主界面将显示日历视图</span>
              </div>
            </div>
            <el-switch
              :model-value="showCalendar"
              @change="(val: boolean) => handleShowCalendarChange(val)"
            />
          </div>

          <div class="settings-row">
            <div class="row-left">
              <el-icon class="row-icon"><Monitor /></el-icon>
              <div class="row-content">
                <span class="settings-label">贴边自动隐藏</span>
                <span class="settings-desc">固定模式下，贴边后自动隐藏并在边缘唤起</span>
              </div>
            </div>
            <el-switch
              :model-value="autoHideEnabled"
              @change="(val: boolean) => handleAutoHideChange(val)"
            />
          </div>

          <div class="settings-row">
            <div class="row-left">
              <el-icon class="row-icon"><Top /></el-icon>
              <div class="row-content">
                <span class="settings-label">唤起时置顶</span>
                <span class="settings-desc">贴边唤起时显示在其它窗口之上；关闭后可能被全屏窗口遮挡</span>
              </div>
            </div>
            <el-switch
              :model-value="topOnWake"
              :disabled="!autoHideEnabled"
              @change="(val: boolean) => handleTopOnWakeChange(val)"
            />
          </div>

          <!-- 仅 Windows：依赖 Win32 桌面宿主（owner=Progman），其它平台不显示 -->
          <div v-if="isWindows" class="settings-row">
            <div class="row-left">
              <el-icon class="row-icon"><Monitor /></el-icon>
              <div class="row-content">
                <span class="settings-label">固定模式时，嵌入桌面中</span>
                <span class="settings-desc">
                  固定后窗口嵌在桌面图标之上、其它窗口之下，显示桌面（Win+D）时不会被隐藏；贴边隐藏与唤起置顶不再生效。
                  需要 Windows 11 24H2 及以上
                </span>
              </div>
            </div>
            <el-switch
              :model-value="fixedEmbedDesktop"
              @change="(val: boolean) => handleFixedEmbedDesktopChange(val)"
            />
          </div>

          <div class="settings-row">
            <div class="row-left">
              <el-icon class="row-icon"><Moon /></el-icon>
              <div class="row-content">
                <span class="settings-label">深色主题</span>
                <span class="settings-desc">启用半透明深色外观</span>
              </div>
            </div>
            <el-switch
              :model-value="appStore.isDarkTheme"
              @change="(val: boolean) => handleDarkThemeChange(val)"
            />
          </div>

          <div class="settings-row bg-color-row">
            <div class="row-left">
              <el-icon class="row-icon"><Brush /></el-icon>
              <div class="row-content">
                <span class="settings-label">界面底色</span>
                <span class="settings-desc">仅深色主题下生效</span>
              </div>
            </div>
            <div class="bg-color-picker">
              <button
                v-for="preset in PRESET_BG_COLORS"
                :key="preset.value"
                class="bg-preset-btn"
                :class="{ active: isSameColor(windowBgColor, preset.value) }"
                :style="{ backgroundColor: preset.value }"
                :title="preset.name"
                :disabled="!appStore.isDarkTheme"
                @click="handleBgColorChange(preset.value)"
              />
              <el-color-picker
                :model-value="windowBgColor"
                :disabled="!appStore.isDarkTheme"
                size="small"
                @change="(val: string | null) => handleBgColorChange(val || DEFAULT_BG_COLOR)"
              />
            </div>
          </div>

          <div class="settings-row bg-alpha-row">
            <div class="row-left">
              <el-icon class="row-icon"><Sunny /></el-icon>
              <div class="row-content">
                <span class="settings-label">背景透明度</span>
                <span class="settings-desc">数值越低越透明，仅深色主题下生效</span>
              </div>
            </div>
            <div class="bg-alpha-slider">
              <el-slider
                :model-value="Math.round(windowBgAlpha * 100)"
                :min="5"
                :max="100"
                :step="5"
                :disabled="!appStore.isDarkTheme"
                :format-tooltip="(val: number) => `${val}%`"
                @input="(val: number) => handleBgAlphaInput(val)"
                @change="(val: number) => handleBgAlphaChange(val)"
              />
              <span class="alpha-value">{{ Math.round(windowBgAlpha * 100) }}%</span>
            </div>
          </div>

          <div class="settings-row notification-type-row">
            <div class="row-left">
              <el-icon class="row-icon"><Bell /></el-icon>
              <div class="row-content">
                <span class="settings-label">通知方式</span>
                <span class="settings-desc">选择待办提醒的通知展示方式</span>
              </div>
            </div>
            <el-radio-group
              :model-value="notificationType"
              :disabled="notificationTypeLoading"
              size="small"
              @change="handleNotificationTypeChange"
            >
              <el-radio-button value="system">系统通知</el-radio-button>
              <el-radio-button value="app">软件通知</el-radio-button>
            </el-radio-group>
          </div>
        </div>

        <!-- 外观 -->
        <div v-show="activeMenu === 'appearance'" class="panel">
          <h3 class="panel-title">外观</h3>

          <div class="form-section">
            <label class="form-label">待办字体</label>
            <el-select
              :model-value="fontFamily"
              filterable
              clearable
              placeholder="跟随系统"
              style="width: 100%"
              @change="handleFontFamilyChange"
            >
              <el-option
                v-for="font in systemFonts"
                :key="font"
                :label="font"
                :value="font"
                :style="{ fontFamily: font }"
              />
            </el-select>
          </div>

          <div class="form-section">
            <label class="form-label">
              字体大小
              <span class="form-label-value">{{ fontSize }}px</span>
            </label>
            <el-slider
              v-model="fontSize"
              :min="12"
              :max="20"
              :step="1"
              :show-tooltip="false"
              @change="handleFontSizePersist"
            />
          </div>

          <div class="font-preview" :style="{ fontFamily: fontFamily || undefined, fontSize: fontSize + 'px' }">
            待办事项预览 Todo Preview 1234
          </div>
        </div>

        <!-- 数据与同步 -->
        <div v-show="activeMenu === 'data'" class="panel">
          <h3 class="panel-title">数据管理</h3>

          <div class="data-actions">
            <button
              class="data-btn primary"
              :disabled="exporting"
              @click="handleExport"
            >
              <el-icon><Download /></el-icon>
              <span>{{ exporting ? '导出中...' : '导出数据' }}</span>
            </button>

            <button
              class="data-btn"
              :disabled="importing"
              @click="handleImport"
            >
              <el-icon><Upload /></el-icon>
              <span>{{ importing ? '导入中...' : '导入数据' }}</span>
            </button>
          </div>

          <p class="card-hint">
            <el-icon :size="14"><InfoFilled /></el-icon>
            导出为 ZIP 压缩包，可用于备份或迁移到其他设备
          </p>

          <div class="section-divider"></div>

          <h3 class="panel-title">
            云同步 (WebDAV)
            <span v-if="syncSettings.lastSyncAt" class="last-sync-time">
              上次同步: {{ formatTime(syncSettings.lastSyncAt) }}
            </span>
          </h3>

          <div class="sync-form">
            <div class="form-item">
              <label class="form-label">服务器地址</label>
              <el-input
                v-model="syncSettings.webdavUrl"
                placeholder="https://dav.example.com/dav"
                size="small"
                clearable
              />
              <p v-if="isInsecureWebdavUrl" class="card-hint warning-hint">
                <el-icon :size="14"><WarningFilled /></el-icon>
                当前地址使用 http://，账号密码与待办数据将以明文传输，建议改用 https://
              </p>
            </div>

            <div class="form-row">
              <div class="form-item flex-1">
                <label class="form-label">用户名</label>
                <el-input
                  v-model="syncSettings.webdavUsername"
                  placeholder="用户名"
                  size="small"
                />
              </div>
              <div class="form-item flex-1">
                <label class="form-label">
                  <span>密码</span>
                  <button
                    v-if="syncSettings.hasPassword"
                    class="link-btn"
                    type="button"
                    :disabled="clearingPassword"
                    @click="handleClearPassword"
                  >
                    清除已保存的密码
                  </button>
                </label>
                <el-input
                  v-model="syncSettings.webdavPassword"
                  :type="showPassword ? 'text' : 'password'"
                  :placeholder="passwordPlaceholder"
                  size="small"
                  autocomplete="new-password"
                >
                  <template #suffix>
                    <el-icon class="password-toggle" @click="showPassword = !showPassword">
                      <View v-if="showPassword" />
                      <Hide v-else />
                    </el-icon>
                  </template>
                </el-input>
              </div>
            </div>

            <div class="form-actions">
              <button
                class="data-btn"
                :disabled="testingConnection"
                @click="testConnection"
              >
                <el-icon><Connection /></el-icon>
                <span>{{ testingConnection ? '测试中...' : '测试连接' }}</span>
              </button>
              <button class="data-btn primary" @click="saveSyncSettings">
                <el-icon><Check /></el-icon>
                <span>保存配置</span>
              </button>
            </div>
          </div>

          <div class="section-divider"></div>

          <div class="settings-row">
            <div class="row-left">
              <el-icon class="row-icon"><Timer /></el-icon>
              <div class="row-content">
                <span class="settings-label">自动同步</span>
                <span class="settings-desc">定时自动与云端合并同步</span>
              </div>
            </div>
            <el-switch
              v-model="syncSettings.autoSync"
              @change="saveSyncSettings"
            />
          </div>

          <div v-if="syncSettings.autoSync" class="settings-row">
            <div class="row-left">
              <el-icon class="row-icon"><Clock /></el-icon>
              <span class="settings-label">同步间隔</span>
            </div>
            <el-select
              v-model="syncSettings.syncInterval"
              size="small"
              style="width: 120px"
              @change="saveSyncSettings"
            >
              <el-option
                v-for="opt in syncIntervalOptions"
                :key="opt.value"
                :label="opt.label"
                :value="opt.value"
              />
            </el-select>
          </div>

          <div class="sync-actions">
            <button
              class="data-btn primary"
              :disabled="!!syncAction || !syncSettings.webdavUrl"
              @click="handleSyncNow"
            >
              <el-icon><Refresh /></el-icon>
              <span>{{ syncAction === 'sync' ? '同步中...' : '立即同步' }}</span>
            </button>
          </div>

          <p class="card-hint">
            <el-icon :size="14"><InfoFilled /></el-icon>
            通过 WebDAV 协议同步待办数据和图片：本地与云端的新增、修改、删除会自动合并，以较新的修改为准
          </p>

          <button
            class="advanced-toggle"
            type="button"
            @click="showAdvancedSync = !showAdvancedSync"
          >
            <el-icon :size="12">
              <ArrowDown v-if="showAdvancedSync" />
              <ArrowRight v-else />
            </el-icon>
            <span>高级</span>
          </button>

          <div v-if="showAdvancedSync" class="advanced-sync">
            <p class="card-hint">
              <el-icon :size="14"><WarningFilled /></el-icon>
              仅在两端数据出现异常时使用：会以一端为准整体覆盖另一端，被覆盖一端独有的数据将被删除
            </p>
            <div class="sync-actions">
              <button
                class="data-btn danger"
                :disabled="!!syncAction || !syncSettings.webdavUrl"
                @click="handleForcePull"
              >
                <el-icon><Download /></el-icon>
                <span>{{ syncAction === 'pull' ? '覆盖中...' : '用云端覆盖本地' }}</span>
              </button>
              <button
                class="data-btn danger"
                :disabled="!!syncAction || !syncSettings.webdavUrl"
                @click="handleForcePush"
              >
                <el-icon><Upload /></el-icon>
                <span>{{ syncAction === 'push' ? '覆盖中...' : '用本地覆盖云端' }}</span>
              </button>
            </div>
          </div>
        </div>

        <!-- 屏幕配置 -->
        <div v-show="activeMenu === 'screen'" class="panel">
          <h3 class="panel-title">屏幕配置</h3>

          <p class="card-hint" style="margin-bottom: 12px;">
            <el-icon :size="14"><InfoFilled /></el-icon>
            应用会根据不同的屏幕组合自动保存和恢复窗口位置
          </p>

          <div v-if="screenConfigs.length === 0" class="empty-configs">
            <el-icon :size="28"><Monitor /></el-icon>
            <span>暂无保存的屏幕配置</span>
          </div>

          <div v-else class="config-list">
            <div
              v-for="config in screenConfigs"
              :key="config.id"
              class="config-item"
              :class="{ active: config.configId === currentConfigId }"
            >
              <div class="config-info">
                <div class="config-name">
                  {{ config.displayName || '未命名配置' }}
                  <span v-if="config.configId === currentConfigId" class="current-badge">
                    当前
                  </span>
                </div>
                <div class="config-detail">
                  {{ formatConfigInfo(config.configId) }}
                </div>
                <div class="config-meta">
                  {{ config.isFixed ? '固定模式' : '普通模式' }} |
                  位置: ({{ config.windowX }}, {{ config.windowY }})
                </div>
              </div>
              <div class="config-actions">
                <el-button
                  type="danger"
                  text
                  size="small"
                  :disabled="config.configId === currentConfigId"
                  @click="handleDeleteConfig(config)"
                >
                  <el-icon><Delete /></el-icon>
                </el-button>
              </div>
            </div>
          </div>
        </div>

        <!-- 关于 -->
        <div v-show="activeMenu === 'about'" class="panel">
          <div class="about-content">
            <div class="app-logo">
              <el-icon :size="36"><Promotion /></el-icon>
            </div>
            <div class="app-info">
              <h3 class="app-name">Mini Todo</h3>
              <p class="app-version">
                版本 {{ APP_VERSION }}
                <span v-if="hasUpdate" class="update-badge">
                  新版本 {{ latestVersion }}
                </span>
              </p>
              <p class="app-desc">一个简洁高效的桌面待办应用</p>
            </div>
          </div>

          <button
            class="check-update-btn"
            :disabled="checking"
            @click="handleCheckUpdate"
          >
            <el-icon><Refresh /></el-icon>
            <span>{{ checking ? '检查中...' : '检查更新' }}</span>
          </button>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.settings-window {
  display: flex;
  flex-direction: column;
  height: 100vh;
  background: #f8fafc;
}

.window-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 12px 20px;
  background: #ffffff;
  border-bottom: 1px solid #e2e8f0;
  -webkit-app-region: drag;

  h2 {
    margin: 0;
    font-size: 17px;
    font-weight: 600;
    color: #1e293b;
  }

  .el-button {
    -webkit-app-region: no-drag;
  }
}

.settings-body {
  display: flex;
  flex: 1;
  overflow: hidden;
}

/* 左侧菜单 */
.settings-menu {
  width: 160px;
  flex-shrink: 0;
  background: #ffffff;
  border-right: 1px solid #e2e8f0;
  padding: 8px;
  overflow-y: auto;
}

.menu-item {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 10px 12px;
  border-radius: 8px;
  font-size: 13px;
  font-weight: 500;
  color: #64748b;
  cursor: pointer;
  transition: all 0.15s ease;
  user-select: none;

  &:hover {
    background: #f1f5f9;
    color: #334155;
  }

  &.active {
    background: #eff6ff;
    color: #3b82f6;
  }
}

/* 右侧内容区 */
.settings-content {
  flex: 1;
  padding: 20px 24px;
  overflow-y: auto;
}

.panel-title {
  display: flex;
  align-items: center;
  gap: 8px;
  margin: 0 0 16px;
  font-size: 15px;
  font-weight: 600;
  color: #1e293b;
}

/* 设置行 */
.settings-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 12px 0;
  border-bottom: 1px solid #f1f5f9;

  &:last-child {
    border-bottom: none;
    padding-bottom: 0;
  }
}

.row-left {
  display: flex;
  align-items: center;
  gap: 12px;
}

.row-icon {
  font-size: 18px;
  color: #64748b;
}

.row-content {
  display: flex;
  flex-direction: column;
}

.settings-label {
  font-size: 14px;
  color: #334155;
  font-weight: 500;
}

.settings-desc {
  font-size: 12px;
  color: #94a3b8;
  margin-top: 2px;
}

.notification-type-row {
  flex-wrap: wrap;
  gap: 8px;

  .row-left {
    flex: 1;
    min-width: 150px;
  }

  :deep(.el-radio-group) {
    flex-shrink: 0;
  }

  :deep(.el-radio-button__inner) {
    padding: 6px 12px;
    font-size: 12px;
  }
}

.bg-color-row,
.bg-alpha-row {
  flex-wrap: wrap;
  gap: 8px;

  .row-left {
    flex: 1;
    min-width: 150px;
  }
}

.bg-color-picker {
  display: flex;
  align-items: center;
  gap: 6px;
  flex-shrink: 0;
}

.bg-preset-btn {
  width: 20px;
  height: 20px;
  border-radius: 50%;
  border: 2px solid transparent;
  cursor: pointer;
  padding: 0;
  transition: transform 0.15s ease, border-color 0.15s ease;
  box-shadow: 0 0 0 1px #e2e8f0;

  &:hover:not(:disabled) {
    transform: scale(1.15);
  }

  &.active {
    border-color: var(--primary);
  }

  &:disabled {
    cursor: not-allowed;
    opacity: 0.4;
  }
}

.bg-alpha-slider {
  display: flex;
  align-items: center;
  gap: 12px;
  flex-shrink: 0;
  width: 180px;

  :deep(.el-slider) {
    flex: 1;
  }

  .alpha-value {
    font-size: 12px;
    color: #64748b;
    min-width: 34px;
    text-align: right;
  }
}

/* 表单区域 */
.form-section {
  margin-bottom: 20px;
}

.form-label {
  display: flex;
  align-items: center;
  justify-content: space-between;
  font-size: 13px;
  color: #64748b;
  font-weight: 500;
  margin-bottom: 8px;
}

.form-label-value {
  font-size: 12px;
  color: #3b82f6;
  font-weight: 600;
}

/* 字体预览 */
.font-preview {
  padding: 16px;
  background: #ffffff;
  border: 1px solid #e2e8f0;
  border-radius: 8px;
  color: #334155;
  line-height: 1.5;
}

.card-hint {
  display: flex;
  align-items: flex-start;
  gap: 6px;
  font-size: 12px;
  color: #64748b;
  margin: 0;

  .el-icon {
    margin-top: 1px;
    color: #94a3b8;
  }
}

/* 分隔线 */
.section-divider {
  height: 1px;
  background: #e2e8f0;
  margin: 20px 0;
}

/* 数据操作按钮 */
.data-actions {
  display: flex;
  gap: 12px;
  margin-bottom: 12px;
}

.data-btn {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 8px;
  padding: 10px 16px;
  border: 1px solid #e2e8f0;
  border-radius: 8px;
  background: #ffffff;
  font-size: 13px;
  font-weight: 500;
  color: #334155;
  cursor: pointer;
  transition: all 0.2s ease;

  &:hover:not(:disabled) {
    background: #f8fafc;
    border-color: #cbd5e1;
  }

  &:disabled {
    opacity: 0.6;
    cursor: not-allowed;
  }

  &.primary {
    background: #3b82f6;
    border-color: #3b82f6;
    color: #ffffff;

    &:hover:not(:disabled) {
      background: #2563eb;
      border-color: #2563eb;
    }
  }

  &.danger {
    color: #dc2626;
    border-color: #fecaca;

    &:hover:not(:disabled) {
      background: #fef2f2;
      border-color: #fca5a5;
    }
  }

  .el-icon {
    font-size: 16px;
  }
}

/* 明文传输等警示 */
.card-hint.warning-hint {
  margin-top: 4px;
  color: #b45309;

  .el-icon {
    color: #f59e0b;
  }
}

/* 表单标签右侧的文字按钮（清除已保存的密码） */
.link-btn {
  padding: 0;
  border: none;
  background: transparent;
  font-size: 12px;
  font-weight: 400;
  color: #3b82f6;
  cursor: pointer;

  &:hover:not(:disabled) {
    color: #2563eb;
    text-decoration: underline;
  }

  &:disabled {
    opacity: 0.6;
    cursor: not-allowed;
  }
}

/* 高级同步选项 */
.advanced-toggle {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  margin-top: 16px;
  padding: 0;
  border: none;
  background: transparent;
  font-size: 12px;
  color: #64748b;
  cursor: pointer;

  &:hover {
    color: #334155;
  }
}

.advanced-sync {
  margin-top: 8px;
  padding: 12px;
  border: 1px solid #fde68a;
  border-radius: 8px;
  background: #fffbeb;

  .sync-actions {
    margin-top: 12px;
    margin-bottom: 0;
  }
}

/* WebDAV */
.last-sync-time {
  margin-left: auto;
  font-size: 11px;
  color: #94a3b8;
  font-weight: 400;
}

.sync-form {
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.form-item {
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.form-row {
  display: flex;
  gap: 12px;
}

.flex-1 {
  flex: 1;
}

.form-actions {
  display: flex;
  gap: 8px;
  margin-top: 4px;
}

.password-toggle {
  cursor: pointer;
  color: #94a3b8;
  transition: color 0.2s;

  &:hover {
    color: #64748b;
  }
}

.sync-actions {
  display: flex;
  gap: 12px;
  margin-top: 16px;
  margin-bottom: 12px;
}

/* 屏幕配置 */
.empty-configs {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  padding: 24px;
  color: #94a3b8;
  text-align: center;

  .el-icon {
    margin-bottom: 8px;
    opacity: 0.5;
  }

  span {
    font-size: 13px;
  }
}

.config-list {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.config-item {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 12px 14px;
  background: #ffffff;
  border-radius: 8px;
  border: 1px solid #e2e8f0;
  transition: all 0.2s ease;

  &:hover {
    background: #f8fafc;
  }

  &.active {
    border-color: #3b82f6;
    background: #eff6ff;
  }
}

.config-info {
  flex: 1;
  min-width: 0;
}

.config-name {
  font-size: 13px;
  font-weight: 500;
  color: #334155;
  display: flex;
  align-items: center;
  gap: 8px;
}

.current-badge {
  font-size: 10px;
  padding: 2px 8px;
  background: linear-gradient(135deg, #3b82f6 0%, #06b6d4 100%);
  color: white;
  border-radius: 10px;
  font-weight: 500;
}

.config-detail {
  font-size: 11px;
  color: #64748b;
  margin-top: 4px;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.config-meta {
  font-size: 11px;
  color: #94a3b8;
  margin-top: 2px;
}

.config-actions {
  flex-shrink: 0;
  margin-left: 8px;
}

/* 关于 */
.about-content {
  display: flex;
  align-items: center;
  gap: 16px;
  margin-bottom: 20px;
}

.app-logo {
  width: 60px;
  height: 60px;
  display: flex;
  align-items: center;
  justify-content: center;
  background: linear-gradient(135deg, #3b82f6 0%, #06b6d4 100%);
  border-radius: 14px;
  color: #ffffff;
}

.app-info {
  flex: 1;
}

.app-name {
  margin: 0 0 4px;
  font-size: 18px;
  font-weight: 600;
  color: #1e293b;
}

.app-version {
  margin: 0 0 4px;
  font-size: 13px;
  color: #64748b;
  display: flex;
  align-items: center;
  gap: 8px;
}

.update-badge {
  font-size: 11px;
  padding: 2px 8px;
  background: #fee2e2;
  color: #ef4444;
  border-radius: 10px;
  font-weight: 500;
}

.app-desc {
  margin: 0;
  font-size: 12px;
  color: #94a3b8;
}

.check-update-btn {
  width: 100%;
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 8px;
  padding: 12px 16px;
  border: 1px solid #e2e8f0;
  border-radius: 8px;
  background: #ffffff;
  font-size: 14px;
  font-weight: 500;
  color: #334155;
  cursor: pointer;
  transition: all 0.2s ease;

  &:hover:not(:disabled) {
    background: #f8fafc;
    border-color: #cbd5e1;
  }

  &:disabled {
    opacity: 0.6;
    cursor: not-allowed;
  }

  .el-icon {
    font-size: 16px;
  }
}
</style>
