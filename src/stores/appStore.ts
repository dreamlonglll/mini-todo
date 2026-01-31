import { defineStore } from 'pinia'
import { ref } from 'vue'
import { getVersion } from '@tauri-apps/api/app'
import { invoke } from '@tauri-apps/api/core'
import { getCurrentWindow, PhysicalPosition, PhysicalSize, availableMonitors, primaryMonitor } from '@tauri-apps/api/window'
import type { WindowPosition, WindowSize, WindowMode, ScreenConfig, SaveScreenConfigRequest, MonitorInfo } from '@/types'

// 当前应用版本（从系统读取）
export const APP_VERSION = ref<string>('')
// GitHub 仓库信息
const GITHUB_OWNER = 'dreamlonglll'
const GITHUB_REPO = 'mini-todo'

export const useAppStore = defineStore('app', () => {
  async function loadAppVersion() {
    if (APP_VERSION.value) return
    try {
      APP_VERSION.value = await getVersion()
    } catch (e) {
      console.error('Failed to load app version:', e)
    }
  }

  // 状态
  const isFixed = ref(false)
  const windowPosition = ref<WindowPosition | null>(null)
  const windowSize = ref<WindowSize | null>(null)
  const windowMode = ref<WindowMode>('normal')
  
  // 屏幕配置相关状态
  const currentScreenConfigId = ref<string>('')
  const screenConfigs = ref<ScreenConfig[]>([])
  
  // 版本更新相关状态
  const hasUpdate = ref(false)
  const latestVersion = ref<string | null>(null)
  const releaseUrl = ref<string | null>(null)

  // 获取当前窗口
  const appWindow = getCurrentWindow()

  /**
   * 生成当前屏幕配置的唯一标识
   * 格式：{显示器数量}_{分辨率1}@{缩放1}_{分辨率2}@{缩放2}...
   * 按分辨率排序确保一致性
   */
  async function generateScreenConfigId(): Promise<string> {
    try {
      const monitors = await availableMonitors()
      if (monitors.length === 0) {
        return 'unknown'
      }

      // 收集所有显示器信息
      const monitorInfos: MonitorInfo[] = monitors.map(m => ({
        width: m.size.width,
        height: m.size.height,
        scaleFactor: Math.round(m.scaleFactor * 100) // 转换为百分比整数
      }))

      // 按分辨率排序（降序，大屏在前）
      monitorInfos.sort((a, b) => {
        const aPixels = a.width * a.height
        const bPixels = b.width * b.height
        return bPixels - aPixels
      })

      // 生成标识字符串
      const parts = monitorInfos.map(m => `${m.width}x${m.height}@${m.scaleFactor}`)
      return `${monitors.length}_${parts.join('_')}`
    } catch (e) {
      console.error('Failed to generate screen config id:', e)
      return 'unknown'
    }
  }

  /**
   * 生成人类可读的屏幕配置描述
   */
  function generateScreenConfigDisplayName(configId: string): string {
    if (configId === 'unknown' || configId === 'legacy') {
      return configId === 'legacy' ? '旧版配置' : '未知配置'
    }
    
    const parts = configId.split('_')
    const count = parts[0]
    const monitors = parts.slice(1).map(p => {
      const [res, scale] = p.split('@')
      return `${res} (${scale}%)`
    })
    
    return `${count}屏: ${monitors.join(' + ')}`
  }

  // 初始化应用设置
  async function initSettings() {
    try {
      await loadAppVersion()
      
      // 生成当前屏幕配置标识
      currentScreenConfigId.value = await generateScreenConfigId()
      console.log('Current screen config ID:', currentScreenConfigId.value)
      
      // 尝试获取当前屏幕配置的保存记录
      const savedConfig = await invoke<ScreenConfig | null>('get_screen_config', {
        configId: currentScreenConfigId.value
      })
      
      if (savedConfig) {
        // 有保存的配置，恢复窗口状态
        console.log('Restoring saved screen config:', savedConfig)
        
        isFixed.value = savedConfig.isFixed
        windowPosition.value = { x: savedConfig.windowX, y: savedConfig.windowY }
        windowSize.value = { width: savedConfig.windowWidth, height: savedConfig.windowHeight }
        windowMode.value = savedConfig.isFixed ? 'fixed' : 'normal'
        
        // 恢复窗口位置
        try {
          await appWindow.setPosition(
            new PhysicalPosition(savedConfig.windowX, savedConfig.windowY)
          )
        } catch (e) {
          console.error('Failed to restore window position:', e)
        }
        
        // 恢复窗口尺寸
        try {
          await appWindow.setSize(
            new PhysicalSize(savedConfig.windowWidth, savedConfig.windowHeight)
          )
        } catch (e) {
          console.error('Failed to restore window size:', e)
        }
        
        // 如果是固定模式，应用固定模式设置
        if (savedConfig.isFixed) {
          await applyFixedMode()
        }
      } else {
        // 没有保存的配置，使用主屏幕中心位置
        console.log('No saved config found, using primary monitor center')
        
        isFixed.value = false
        windowMode.value = 'normal'
        
        try {
          const monitor = await primaryMonitor()
          if (monitor) {
            const defaultWidth = 380
            const defaultHeight = 600
            const scale = monitor.scaleFactor
            
            // 计算主屏幕中心位置（逻辑坐标转物理坐标）
            const centerX = Math.round(
              monitor.position.x + (monitor.size.width - defaultWidth * scale) / 2
            )
            const centerY = Math.round(
              monitor.position.y + (monitor.size.height - defaultHeight * scale) / 2
            )
            
            await appWindow.setPosition(new PhysicalPosition(centerX, centerY))
            await appWindow.setSize(new PhysicalSize(defaultWidth * scale, defaultHeight * scale))
            
            windowPosition.value = { x: centerX, y: centerY }
            windowSize.value = { width: Math.round(defaultWidth * scale), height: Math.round(defaultHeight * scale) }
          }
        } catch (e) {
          console.error('Failed to center window:', e)
        }
        
        // 为当前配置创建初始记录
        await saveWindowState()
      }
      
      // 加载所有屏幕配置列表
      await loadScreenConfigs()
    } catch (e) {
      console.error('Failed to load settings:', e)
    }
  }

  // 切换固定模式
  async function toggleFixedMode() {
    try {
      isFixed.value = !isFixed.value
      windowMode.value = isFixed.value ? 'fixed' : 'normal'

      // 应用窗口模式
      if (isFixed.value) {
        await applyFixedMode()
      } else {
        await applyNormalMode()
      }

      // 保存窗口状态到屏幕配置表和 settings 表
      await saveWindowState()
    } catch (e) {
      console.error('Failed to toggle fixed mode:', e)
    }
  }

  // 应用固定模式
  async function applyFixedMode() {
    try {
      // 设置窗口属性（不再置顶，支持正常窗体堆叠）
      await appWindow.setResizable(false)
      
      // 设置 body 的 class 以启用透明背景
      document.body.classList.add('fixed-mode')
      
      // 调用 Rust 端设置窗口为工具窗口样式（忽略 Win+D）
      await invoke('set_window_fixed_mode', { fixed: true })
    } catch (e) {
      console.error('Failed to apply fixed mode:', e)
    }
  }

  // 应用普通模式
  async function applyNormalMode() {
    try {
      await appWindow.setResizable(true)
      
      // 移除 body 的固定模式 class
      document.body.classList.remove('fixed-mode')
      
      // 调用 Rust 端恢复窗口样式
      await invoke('set_window_fixed_mode', { fixed: false })
    } catch (e) {
      console.error('Failed to apply normal mode:', e)
    }
  }

  // 保存窗口状态（位置和尺寸）到当前屏幕配置
  async function saveWindowState() {
    try {
      const position = await appWindow.outerPosition()
      const size = await appWindow.outerSize()
      windowPosition.value = { x: position.x, y: position.y }
      windowSize.value = { width: size.width, height: size.height }
      
      // 确保有当前屏幕配置 ID
      if (!currentScreenConfigId.value) {
        currentScreenConfigId.value = await generateScreenConfigId()
      }
      
      // 保存到屏幕配置表
      const configRequest: SaveScreenConfigRequest = {
        configId: currentScreenConfigId.value,
        displayName: generateScreenConfigDisplayName(currentScreenConfigId.value),
        windowX: position.x,
        windowY: position.y,
        windowWidth: size.width,
        windowHeight: size.height,
        isFixed: isFixed.value
      }
      
      await invoke('save_screen_config', { config: configRequest })
      
      // 同时保存到旧的 settings 表（保持向后兼容）
      await invoke('save_settings', {
        settings: {
          isFixed: isFixed.value,
          windowPosition: windowPosition.value,
          windowSize: windowSize.value,
          textTheme: 'light'
        }
      })
    } catch (e) {
      console.error('Failed to save window state:', e)
    }
  }

  // 加载所有屏幕配置列表
  async function loadScreenConfigs() {
    try {
      screenConfigs.value = await invoke<ScreenConfig[]>('list_screen_configs')
    } catch (e) {
      console.error('Failed to load screen configs:', e)
      screenConfigs.value = []
    }
  }

  // 删除屏幕配置
  async function deleteScreenConfig(configId: string): Promise<boolean> {
    try {
      await invoke('delete_screen_config', { configId })
      await loadScreenConfigs()
      return true
    } catch (e) {
      console.error('Failed to delete screen config:', e)
      return false
    }
  }

  // 更新屏幕配置名称
  async function updateScreenConfigName(configId: string, displayName: string): Promise<boolean> {
    try {
      await invoke('update_screen_config_name', { configId, displayName })
      await loadScreenConfigs()
      return true
    } catch (e) {
      console.error('Failed to update screen config name:', e)
      return false
    }
  }

  // 导出数据
  async function exportData(): Promise<string | null> {
    try {
      return await invoke<string>('export_data')
    } catch (e) {
      console.error('Failed to export data:', e)
      return null
    }
  }

  // 导入数据
  async function importData(jsonData: string): Promise<boolean> {
    try {
      await invoke('import_data', { jsonData })
      return true
    } catch (e) {
      console.error('Failed to import data:', e)
      return false
    }
  }

  // 比较版本号 (返回: 1 表示 v1 > v2, -1 表示 v1 < v2, 0 表示相等)
  function compareVersions(v1: string, v2: string): number {
    const parts1 = v1.replace(/^v/, '').split('.').map(Number)
    const parts2 = v2.replace(/^v/, '').split('.').map(Number)
    
    for (let i = 0; i < Math.max(parts1.length, parts2.length); i++) {
      const p1 = parts1[i] || 0
      const p2 = parts2[i] || 0
      if (p1 > p2) return 1
      if (p1 < p2) return -1
    }
    return 0
  }

  // 检查版本更新
  async function checkForUpdates(): Promise<void> {
    try {
      await loadAppVersion()
      if (!APP_VERSION.value) {
        console.log('App version unavailable; skip update check')
        return
      }
      const response = await fetch(
        `https://api.github.com/repos/${GITHUB_OWNER}/${GITHUB_REPO}/releases/latest`,
        {
          headers: {
            'Accept': 'application/vnd.github.v3+json'
          }
        }
      )
      
      if (!response.ok) {
        console.log('No releases found or API error')
        return
      }
      
      const release = await response.json()
      const tagName = release.tag_name as string
      
      // 比较版本号
      if (compareVersions(tagName, APP_VERSION.value) > 0) {
        hasUpdate.value = true
        latestVersion.value = tagName
        releaseUrl.value = release.html_url
      }
    } catch (e) {
      console.error('Failed to check for updates:', e)
    }
  }

  // 获取 GitHub Release 页面 URL
  function getReleasesUrl(): string {
    return releaseUrl.value || `https://github.com/${GITHUB_OWNER}/${GITHUB_REPO}/releases`
  }

  return {
    // 状态
    isFixed,
    windowPosition,
    windowSize,
    windowMode,
    hasUpdate,
    latestVersion,
    // 屏幕配置状态
    currentScreenConfigId,
    screenConfigs,
    // 方法
    initSettings,
    toggleFixedMode,
    saveWindowState,
    exportData,
    importData,
    checkForUpdates,
    getReleasesUrl,
    // 屏幕配置方法
    generateScreenConfigId,
    generateScreenConfigDisplayName,
    loadScreenConfigs,
    deleteScreenConfig,
    updateScreenConfigName
  }
})
