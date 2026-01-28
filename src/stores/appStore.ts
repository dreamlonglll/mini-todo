import { defineStore } from 'pinia'
import { ref } from 'vue'
import { getVersion } from '@tauri-apps/api/app'
import { invoke } from '@tauri-apps/api/core'
import { getCurrentWindow, PhysicalPosition, PhysicalSize } from '@tauri-apps/api/window'
import type { WindowPosition, WindowSize, WindowMode } from '@/types'

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
  
  // 版本更新相关状态
  const hasUpdate = ref(false)
  const latestVersion = ref<string | null>(null)
  const releaseUrl = ref<string | null>(null)

  // 获取当前窗口
  const appWindow = getCurrentWindow()

  // 初始化应用设置
  async function initSettings() {
    try {
      await loadAppVersion()
      const settings = await invoke<{ 
        isFixed: boolean
        windowPosition: WindowPosition | null
        windowSize: WindowSize | null
      }>('get_settings')
      
      isFixed.value = settings.isFixed
      windowPosition.value = settings.windowPosition
      windowSize.value = settings.windowSize
      windowMode.value = settings.isFixed ? 'fixed' : 'normal'
      
      // 恢复窗口位置
      if (settings.windowPosition) {
        try {
          await appWindow.setPosition(
            new PhysicalPosition(settings.windowPosition.x, settings.windowPosition.y)
          )
        } catch (e) {
          console.error('Failed to restore window position:', e)
        }
      }
      
      // 恢复窗口尺寸
      if (settings.windowSize) {
        try {
          await appWindow.setSize(
            new PhysicalSize(settings.windowSize.width, settings.windowSize.height)
          )
        } catch (e) {
          console.error('Failed to restore window size:', e)
        }
      }
      
      // 如果是固定模式，应用固定模式设置
      if (isFixed.value) {
        await applyFixedMode()
      }
    } catch (e) {
      console.error('Failed to load settings:', e)
    }
  }

  // 切换固定模式
  async function toggleFixedMode() {
    try {
      // 保存当前位置和尺寸
      const position = await appWindow.outerPosition()
      const size = await appWindow.outerSize()
      windowPosition.value = { x: position.x, y: position.y }
      windowSize.value = { width: size.width, height: size.height }

      isFixed.value = !isFixed.value
      windowMode.value = isFixed.value ? 'fixed' : 'normal'

      // 保存设置到数据库
      await invoke('save_settings', {
        settings: {
          isFixed: isFixed.value,
          windowPosition: windowPosition.value,
          windowSize: windowSize.value,
          textTheme: 'light'
        }
      })

      // 应用窗口模式
      if (isFixed.value) {
        await applyFixedMode()
      } else {
        await applyNormalMode()
      }
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

  // 保存窗口状态（位置和尺寸）
  async function saveWindowState() {
    try {
      const position = await appWindow.outerPosition()
      const size = await appWindow.outerSize()
      windowPosition.value = { x: position.x, y: position.y }
      windowSize.value = { width: size.width, height: size.height }
      
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
    // 方法
    initSettings,
    toggleFixedMode,
    saveWindowState,
    exportData,
    importData,
    checkForUpdates,
    getReleasesUrl
  }
})
