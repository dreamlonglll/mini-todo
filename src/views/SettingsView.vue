<script setup lang="ts">
import { ref, computed, onMounted, reactive } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { emit } from '@tauri-apps/api/event'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { save, open } from '@tauri-apps/plugin-dialog'
import { writeTextFile, readTextFile } from '@tauri-apps/plugin-fs'
import { openUrl } from '@tauri-apps/plugin-opener'
import { enable, disable, isEnabled } from '@tauri-apps/plugin-autostart'
import { ElMessage, ElMessageBox } from 'element-plus'
import { useAppStore, APP_VERSION } from '@/stores'
import type { ScreenConfig, SyncSettings, SyncDownloadResult } from '@/types'
import AgentSettings from '@/components/AgentSettings.vue'

const appWindow = getCurrentWindow()
const appStore = useAppStore()

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
  
  // 加载通知类型设置
  try {
    const type = await invoke<string>('get_notification_type')
    notificationType.value = type === 'app' ? 'app' : 'system'
  } catch (e) {
    console.error('Failed to get notification type:', e)
  }
  
  // 加载屏幕配置列表
  await appStore.loadScreenConfigs()
  
  // 加载日历显示状态
  await appStore.loadShowCalendar()
  // 加载贴边自动隐藏状态
  await appStore.loadAutoHideEnabled()
  
  // 加载 WebDAV 同步设置
  await loadSyncSettings()
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

// 切换通知类型
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
    // 恢复原状态
    notificationType.value = oldValue
  } finally {
    notificationTypeLoading.value = false
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
    
    // 通知主窗口导入成功（在主窗口显示提示）
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

// 关闭窗口
function handleClose() {
  appWindow.close()
}

// ========== WebDAV 同步 ==========
const syncSettings = reactive<SyncSettings>({
  webdavUrl: '',
  webdavUsername: '',
  webdavPassword: '',
  autoSync: false,
  syncInterval: 15,
  lastSyncAt: null,
  deviceId: '',
})
const showPassword = ref(false)
const testingConnection = ref(false)
const syncing = ref(false)
const syncStatus = ref<'idle' | 'uploading' | 'downloading'>('idle')

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
  } catch (e) {
    console.error('Failed to load sync settings:', e)
  }
}

async function saveSyncSettings() {
  try {
    await invoke('save_sync_settings', { settings: syncSettings })
    ElMessage.success('同步设置已保存')
  } catch (e) {
    console.error('Failed to save sync settings:', e)
    ElMessage.error('保存失败: ' + String(e))
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
      password: syncSettings.webdavPassword,
    })
    ElMessage.success('连接成功')
  } catch (e) {
    ElMessage.error('连接失败: ' + String(e))
  } finally {
    testingConnection.value = false
  }
}

async function handleUploadSync() {
  if (!syncSettings.webdavUrl) {
    ElMessage.warning('请先配置 WebDAV 服务器')
    return
  }
  try {
    syncing.value = true
    syncStatus.value = 'uploading'
    const lastSyncAt = await invoke<string>('webdav_upload_sync')
    syncSettings.lastSyncAt = lastSyncAt
    ElMessage.success('数据已上传到云端')
    await emit('sync-completed')
  } catch (e) {
    ElMessage.error('上传失败: ' + String(e))
  } finally {
    syncing.value = false
    syncStatus.value = 'idle'
  }
}

async function handleDownloadSync() {
  if (!syncSettings.webdavUrl) {
    ElMessage.warning('请先配置 WebDAV 服务器')
    return
  }
  try {
    syncing.value = true
    syncStatus.value = 'downloading'
    const result = await invoke<SyncDownloadResult>('webdav_download_sync')

    if (!result.hasRemote) {
      ElMessage.info('云端暂无同步数据')
      return
    }

    if (result.hasConflict) {
      try {
        const action = await ElMessageBox.confirm(
          `本地数据（${formatTime(result.localUpdatedAt)}）和云端数据（${formatTime(result.remoteUpdatedAt)}）均有更新，请选择操作：`,
          '同步冲突',
          {
            confirmButtonText: '使用云端数据',
            cancelButtonText: '保留本地数据',
            distinguishCancelAndClose: true,
            type: 'warning',
          }
        )
        if (action === 'confirm') {
          await applyRemoteData(result)
        } else {
          await handleUploadSync()
        }
      } catch (e) {
        if (e === 'cancel') {
          await handleUploadSync()
        }
      }
    } else {
      await applyRemoteData(result)
    }
  } catch (e) {
    ElMessage.error('下载失败: ' + String(e))
  } finally {
    syncing.value = false
    syncStatus.value = 'idle'
  }
}

async function applyRemoteData(result: SyncDownloadResult) {
  if (!result.remoteData) return
  try {
    const lastSyncAt = await invoke<string>('webdav_apply_remote', {
      syncDataJson: JSON.stringify(result.remoteData),
    })
    syncSettings.lastSyncAt = lastSyncAt
    ElMessage.success('已同步云端数据到本地')
    await emit('data-imported')
  } catch (e) {
    ElMessage.error('应用远程数据失败: ' + String(e))
  }
}

function formatTime(time: string | null | undefined): string {
  if (!time) return '未知'
  try {
    return new Date(time).toLocaleString('zh-CN')
  } catch {
    return time
  }
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
      <div class="settings-card">
        <div class="card-header">
          <el-icon class="card-icon"><Setting /></el-icon>
          <h3 class="card-title">通用设置</h3>
        </div>
        
        <div class="card-body">
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
              @change="(val: boolean) => appStore.setShowCalendar(val)"
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
              @change="(val: boolean) => appStore.setAutoHideEnabled(val)"
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
              @change="() => appStore.toggleDarkTheme()"
            />
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
      </div>

      <!-- 数据管理 -->
      <div class="settings-card">
        <div class="card-header">
          <el-icon class="card-icon"><Folder /></el-icon>
          <h3 class="card-title">数据管理</h3>
        </div>
        
        <div class="card-body">
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
            导出数据可用于备份或迁移到其他设备
          </p>
        </div>
      </div>

      <!-- WebDAV 云同步 -->
      <div class="settings-card">
        <div class="card-header">
          <el-icon class="card-icon"><Connection /></el-icon>
          <h3 class="card-title">云同步 (WebDAV)</h3>
          <span v-if="syncSettings.lastSyncAt" class="last-sync-time">
            上次同步: {{ formatTime(syncSettings.lastSyncAt) }}
          </span>
        </div>
        
        <div class="card-body">
          <div class="sync-form">
            <div class="form-item">
              <label class="form-label">服务器地址</label>
              <el-input
                v-model="syncSettings.webdavUrl"
                placeholder="https://dav.example.com/dav"
                size="small"
                clearable
              />
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
                <label class="form-label">密码</label>
                <el-input
                  v-model="syncSettings.webdavPassword"
                  :type="showPassword ? 'text' : 'password'"
                  placeholder="密码"
                  size="small"
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

          <div class="sync-divider"></div>

          <div class="settings-row">
            <div class="row-left">
              <el-icon class="row-icon"><Timer /></el-icon>
              <div class="row-content">
                <span class="settings-label">自动同步</span>
                <span class="settings-desc">定时自动将数据同步到云端</span>
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
              :disabled="syncing || !syncSettings.webdavUrl"
              @click="handleUploadSync"
            >
              <el-icon><Upload /></el-icon>
              <span>{{ syncStatus === 'uploading' ? '上传中...' : '上传到云端' }}</span>
            </button>
            <button 
              class="data-btn"
              :disabled="syncing || !syncSettings.webdavUrl"
              @click="handleDownloadSync"
            >
              <el-icon><Download /></el-icon>
              <span>{{ syncStatus === 'downloading' ? '下载中...' : '从云端下载' }}</span>
            </button>
          </div>

          <p class="card-hint">
            <el-icon :size="14"><InfoFilled /></el-icon>
            通过 WebDAV 协议将待办数据和图片同步到云端存储
          </p>
        </div>
      </div>

      <!-- Agent 管理 -->
      <div class="settings-card">
        <div class="card-header">
          <el-icon class="card-icon"><MagicStick /></el-icon>
          <h3 class="card-title">Agent 管理</h3>
        </div>
        <div class="card-body">
          <AgentSettings />
        </div>
      </div>

      <!-- 屏幕配置管理 -->
      <div class="settings-card">
        <div class="card-header">
          <el-icon class="card-icon"><Monitor /></el-icon>
          <h3 class="card-title">屏幕配置</h3>
        </div>
        
        <div class="card-body">
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
      </div>

      <!-- 关于 -->
      <div class="settings-card about-card">
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
  padding: 14px 20px;
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

.settings-content {
  flex: 1;
  padding: 20px;
  overflow-y: auto;
}

/* 卡片样式 */
.settings-card {
  background: #ffffff;
  border-radius: 12px;
  margin-bottom: 16px;
  box-shadow: 0 1px 3px rgba(0, 0, 0, 0.05);
}

.card-header {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 16px 20px 0;
}

.card-icon {
  font-size: 20px;
  color: #3b82f6;
}

.card-title {
  margin: 0;
  font-size: 15px;
  font-weight: 600;
  color: #1e293b;
}

.card-body {
  padding: 16px 20px 20px;
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

/* 通知类型设置行 */
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

/* 数据操作按钮 */
.data-actions {
  display: flex;
  gap: 12px;
  margin-bottom: 16px;
}

.data-btn {
  flex: 1;
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

  &.primary {
    background: #3b82f6;
    border-color: #3b82f6;
    color: #ffffff;

    &:hover:not(:disabled) {
      background: #2563eb;
      border-color: #2563eb;
    }
  }

  .el-icon {
    font-size: 18px;
  }
}

/* 屏幕配置样式 */
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
  background: #f8fafc;
  border-radius: 8px;
  border: 1px solid transparent;
  transition: all 0.2s ease;

  &:hover {
    background: #f1f5f9;
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

/* 关于卡片 */
.about-card {
  padding: 20px;
}

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

/* WebDAV 同步样式 */
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

.form-label {
  font-size: 12px;
  color: #64748b;
  font-weight: 500;
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

.sync-divider {
  height: 1px;
  background: #f1f5f9;
  margin: 16px 0;
}

.sync-actions {
  display: flex;
  gap: 12px;
  margin-top: 16px;
  margin-bottom: 12px;
}
</style>
