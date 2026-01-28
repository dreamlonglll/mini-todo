import { defineStore } from 'pinia'
import { ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { getCurrentWindow } from '@tauri-apps/api/window'
import type { WindowPosition, WindowMode } from '@/types'

export const useAppStore = defineStore('app', () => {
  // 状态
  const isFixed = ref(false)
  const windowPosition = ref<WindowPosition | null>(null)
  const windowMode = ref<WindowMode>('normal')

  // 获取当前窗口
  const appWindow = getCurrentWindow()

  // 初始化应用设置
  async function initSettings() {
    try {
      const settings = await invoke<{ isFixed: boolean; windowPosition: WindowPosition | null }>('get_settings')
      isFixed.value = settings.isFixed
      windowPosition.value = settings.windowPosition
      windowMode.value = settings.isFixed ? 'fixed' : 'normal'
      
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
      // 如果要切换到固定模式，先保存当前位置
      if (!isFixed.value) {
        const position = await appWindow.outerPosition()
        windowPosition.value = { x: position.x, y: position.y }
      }

      isFixed.value = !isFixed.value
      windowMode.value = isFixed.value ? 'fixed' : 'normal'

      // 保存设置到数据库
      await invoke('save_settings', {
        settings: {
          isFixed: isFixed.value,
          windowPosition: windowPosition.value
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

  // 保存窗口位置
  async function saveWindowPosition() {
    try {
      const position = await appWindow.outerPosition()
      windowPosition.value = { x: position.x, y: position.y }
      
      await invoke('save_settings', {
        settings: {
          isFixed: isFixed.value,
          windowPosition: windowPosition.value
        }
      })
    } catch (e) {
      console.error('Failed to save window position:', e)
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

  return {
    // 状态
    isFixed,
    windowPosition,
    windowMode,
    // 方法
    initSettings,
    toggleFixedMode,
    saveWindowPosition,
    exportData,
    importData
  }
})
