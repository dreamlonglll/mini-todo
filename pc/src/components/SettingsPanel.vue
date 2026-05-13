<script setup lang="ts">
import { ref } from 'vue'
import { useAppStore, useTodoStore } from '@/stores'
import { save, open } from '@tauri-apps/plugin-dialog'
import { writeTextFile, readTextFile } from '@tauri-apps/plugin-fs'
import { ElMessage, ElMessageBox } from 'element-plus'

defineProps<{
  visible: boolean
}>()

const emit = defineEmits<{
  (e: 'update:visible', value: boolean): void
}>()

const appStore = useAppStore()
const todoStore = useTodoStore()

const exporting = ref(false)
const importing = ref(false)

// 导出数据
async function handleExport() {
  try {
    exporting.value = true
    
    // 获取导出数据
    const jsonData = await appStore.exportData()
    if (!jsonData) {
      ElMessage.error('导出失败')
      return
    }

    // 选择保存位置
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
    // 选择文件
    const filePath = await open({
      title: '导入待办数据',
      filters: [{
        name: 'JSON 文件',
        extensions: ['json']
      }]
    })

    if (!filePath) return

    // 确认覆盖
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

    // 读取文件
    const jsonData = await readTextFile(filePath as string)
    
    // 导入数据
    const success = await appStore.importData(jsonData)
    if (success) {
      // 刷新待办列表
      await todoStore.fetchTodos()
      ElMessage.success('导入成功')
      handleClose()
    } else {
      ElMessage.error('导入失败')
    }
  } catch (e) {
    if (String(e) !== 'cancel') {
      console.error('Import error:', e)
      ElMessage.error('导入失败: ' + String(e))
    }
  } finally {
    importing.value = false
  }
}

// 关闭面板
function handleClose() {
  emit('update:visible', false)
}
</script>

<template>
  <el-drawer
    :model-value="visible"
    title="设置"
    direction="rtl"
    size="300px"
    @update:model-value="handleClose"
  >
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
          <p>版本: 0.1.0</p>
          <p>一个简洁高效的桌面待办应用</p>
        </div>
      </div>
    </div>
  </el-drawer>
</template>

<style scoped>
.settings-content {
  padding: var(--space-2);
}

.settings-section {
  margin-bottom: var(--space-6);
}

.section-title {
  font-size: 14px;
  font-weight: 600;
  color: var(--text-primary);
  margin-bottom: var(--space-3);
  padding-bottom: var(--space-2);
  border-bottom: 1px solid var(--border);
}

.settings-item {
  margin-bottom: var(--space-3);
}

.settings-hint {
  font-size: 12px;
  color: var(--text-tertiary);
  margin-top: var(--space-2);
}

.about-info {
  font-size: 13px;
  color: var(--text-secondary);
  line-height: 1.8;

  strong {
    color: var(--text-primary);
  }
}
</style>
