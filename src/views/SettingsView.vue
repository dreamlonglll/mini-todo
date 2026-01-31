<script setup lang="ts">
import { ref, computed, onMounted } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { save, open } from '@tauri-apps/plugin-dialog'
import { writeTextFile, readTextFile } from '@tauri-apps/plugin-fs'
import { openUrl } from '@tauri-apps/plugin-opener'
import { enable, disable, isEnabled } from '@tauri-apps/plugin-autostart'
import { ElMessage, ElMessageBox } from 'element-plus'
import { useAppStore, APP_VERSION } from '@/stores'
import type { ScreenConfig } from '@/types'

const appWindow = getCurrentWindow()
const appStore = useAppStore()

const exporting = ref(false)
const importing = ref(false)
const checking = ref(false)
const autoStart = ref(false)
const autoStartLoading = ref(false)

// 屏幕配置相关
const screenConfigs = computed(() => appStore.screenConfigs)
const currentConfigId = computed(() => appStore.currentScreenConfigId)

// 日历显示
const showCalendar = computed(() => appStore.showCalendar)

// 是否有更新
const hasUpdate = computed(() => appStore.hasUpdate)
const latestVersion = computed(() => appStore.latestVersion)

// 初始化时获取开机自启状态和屏幕配置
onMounted(async () => {
  try {
    autoStart.value = await isEnabled()
  } catch (e) {
    console.error('Failed to get autostart status:', e)
  }
  
  // 加载屏幕配置列表
  await appStore.loadScreenConfigs()
  
  // 加载日历显示状态
  await appStore.loadShowCalendar()
})

// 删除屏幕配置
async function handleDeleteConfig(config: ScreenConfig) {
  // 不允许删除当前正在使用的配置
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

// 格式化屏幕配置信息
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

// 切换开机自启
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
  } catch (e) {
    console.error('Failed to toggle autostart:', e)
    ElMessage.error('设置开机自启失败')
    // 恢复原状态
    autoStart.value = !value
  } finally {
    autoStartLoading.value = false
  }
}

// 导出数据
async function handleExport() {
  try {
    exporting.value = true
    
    const jsonData = await invoke<string>('export_data')
    if (!jsonData) {
      ElMessage.error('导出失败')
      return
    }

    const filePath = await save({
      title: '导出待办数据',
      defaultPath: `mini-todo-backup-${new Date().toISOString().slice(0, 10)}.json`,
      filters: [{
        name: 'JSON 文件',
        extensions: ['json']
      }]
    })

    if (filePath) {
      await writeTextFile(filePath, jsonData)
      ElMessage.success('导出成功')
    }
  } catch (e) {
    console.error('Export error:', e)
    ElMessage.error('导出失败: ' + String(e))
  } finally {
    exporting.value = false
  }
}

// 导入数据
async function handleImport() {
  try {
    const filePath = await open({
      title: '导入待办数据',
      filters: [{
        name: 'JSON 文件',
        extensions: ['json']
      }]
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

    const jsonData = await readTextFile(filePath as string)
    await invoke('import_data', { jsonData })
    
    ElMessage.success('导入成功')
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

// 关闭窗口
function handleClose() {
  appWindow.close()
}

// 检查更新
async function handleCheckUpdate() {
  try {
    checking.value = true
    await appStore.checkForUpdates()
    
    if (hasUpdate.value) {
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
    <div class="window-header">
      <h2>设置</h2>
      <el-button text @click="handleClose">
        <el-icon><Close /></el-icon>
      </el-button>
    </div>

    <div class="settings-content">
      <!-- 通用设置 -->
      <div class="settings-section">
        <h3 class="section-title">通用设置</h3>
        
        <div class="settings-row">
          <span class="settings-label">开机自启</span>
          <el-switch 
            v-model="autoStart"
            :loading="autoStartLoading"
            @change="handleAutoStartChange"
          />
        </div>
        
        <div class="settings-row">
          <span class="settings-label">展示日历</span>
          <el-switch 
            :model-value="showCalendar"
            @change="(val: boolean) => appStore.setShowCalendar(val)"
          />
        </div>
        <p class="settings-hint" style="margin-top: 4px;">
          开启后主界面将显示日历视图
        </p>
      </div>

      <!-- 数据管理 -->
      <div class="settings-section">
        <h3 class="section-title">数据管理</h3>
        
        <div class="settings-item">
          <el-button 
            type="primary" 
            :loading="exporting"
            style="width: 100%"
            @click="handleExport"
          >
            <el-icon><Download /></el-icon>
            <span>导出数据</span>
          </el-button>
        </div>

        <div class="settings-item">
          <el-button 
            :loading="importing"
            style="width: 100%"
            @click="handleImport"
          >
            <el-icon><Upload /></el-icon>
            <span>导入数据</span>
          </el-button>
        </div>

        <p class="settings-hint">
          导出数据可用于备份或迁移到其他设备
        </p>
      </div>

      <!-- 屏幕配置管理 -->
      <div class="settings-section">
        <h3 class="section-title">屏幕配置</h3>
        
        <p class="settings-hint" style="margin-bottom: 12px;">
          应用会根据不同的屏幕组合自动保存和恢复窗口位置
        </p>
        
        <div v-if="screenConfigs.length === 0" class="empty-configs">
          暂无保存的屏幕配置
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
      <div class="settings-section">
        <h3 class="section-title">关于</h3>
        <div class="about-info">
          <p><strong>Mini Todo</strong></p>
          <p>
            版本: {{ APP_VERSION }}
            <span v-if="hasUpdate" class="update-available">
              (有新版本 {{ latestVersion }})
            </span>
          </p>
          <p>一个简洁高效的桌面待办应用</p>
        </div>
        
        <div class="settings-item" style="margin-top: 12px;">
          <el-button 
            :loading="checking"
            style="width: 100%"
            @click="handleCheckUpdate"
          >
            <el-icon><Refresh /></el-icon>
            <span>检查更新</span>
          </el-button>
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
  background: #FFFFFF;
}

.window-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 12px 16px;
  border-bottom: 1px solid var(--border);
  -webkit-app-region: drag;

  h2 {
    margin: 0;
    font-size: 16px;
    font-weight: 600;
  }

  .el-button {
    -webkit-app-region: no-drag;
  }
}

.settings-content {
  flex: 1;
  padding: 16px;
  overflow-y: auto;
}

.settings-section {
  margin-bottom: 24px;
}

.section-title {
  font-size: 14px;
  font-weight: 600;
  color: var(--text-primary);
  margin-bottom: 12px;
  padding-bottom: 8px;
  border-bottom: 1px solid var(--border);
}

.settings-item {
  margin-bottom: 12px;
}

.settings-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 8px 0;
}

.settings-label {
  font-size: 14px;
  color: var(--text-primary);
}

.settings-hint {
  font-size: 12px;
  color: var(--text-tertiary);
  margin-top: 8px;
}

.about-info {
  font-size: 13px;
  color: var(--text-secondary);
  line-height: 1.8;

  strong {
    color: var(--text-primary);
  }
}

.update-available {
  color: #EF4444;
  font-size: 12px;
}

/* 屏幕配置样式 */
.empty-configs {
  text-align: center;
  color: var(--text-tertiary);
  font-size: 13px;
  padding: 16px;
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
  padding: 10px 12px;
  background: var(--bg-secondary);
  border-radius: 6px;
  border: 1px solid transparent;
  transition: border-color 0.2s;

  &.active {
    border-color: var(--primary);
    background: rgba(64, 158, 255, 0.05);
  }
}

.config-info {
  flex: 1;
  min-width: 0;
}

.config-name {
  font-size: 13px;
  font-weight: 500;
  color: var(--text-primary);
  display: flex;
  align-items: center;
  gap: 6px;
}

.current-badge {
  font-size: 11px;
  padding: 1px 6px;
  background: var(--primary);
  color: white;
  border-radius: 4px;
  font-weight: normal;
}

.config-detail {
  font-size: 11px;
  color: var(--text-secondary);
  margin-top: 2px;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.config-meta {
  font-size: 11px;
  color: var(--text-tertiary);
  margin-top: 2px;
}

.config-actions {
  flex-shrink: 0;
  margin-left: 8px;
}
</style>
