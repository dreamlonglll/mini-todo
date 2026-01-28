<script setup lang="ts">
import { ref, computed } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { save, open } from '@tauri-apps/plugin-dialog'
import { writeTextFile, readTextFile } from '@tauri-apps/plugin-fs'
import { openUrl } from '@tauri-apps/plugin-opener'
import { ElMessage, ElMessageBox } from 'element-plus'
import { useAppStore, APP_VERSION } from '@/stores'

const appWindow = getCurrentWindow()
const appStore = useAppStore()

const exporting = ref(false)
const importing = ref(false)
const checking = ref(false)

// 是否有更新
const hasUpdate = computed(() => appStore.hasUpdate)
const latestVersion = computed(() => appStore.latestVersion)

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
</style>
