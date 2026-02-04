<script setup lang="ts">
import { ref, onMounted } from 'vue'
import { useRoute } from 'vue-router'
import { invoke } from '@tauri-apps/api/core'
import { getCurrentWindow } from '@tauri-apps/api/window'

const route = useRoute()
const appWindow = getCurrentWindow()

// 从 URL 参数获取通知内容
const title = ref('')
const description = ref('')
const windowLabel = ref('')

onMounted(() => {
  // 解析 URL 参数
  title.value = decodeURIComponent(route.query.title as string || '待办提醒')
  description.value = decodeURIComponent(route.query.description as string || '')
  windowLabel.value = decodeURIComponent(route.query.label as string || '')
})

// 关闭通知窗口
async function handleClose() {
  try {
    // 先尝试通过命令关闭
    if (windowLabel.value) {
      await invoke('close_notification_window', { windowLabel: windowLabel.value })
    } else {
      // 直接关闭当前窗口
      await appWindow.close()
    }
  } catch (e) {
    console.error('Failed to close notification window:', e)
    // 备用方案：直接关闭当前窗口
    await appWindow.close()
  }
}
</script>

<template>
  <div class="notification-window" @click="handleClose">
    <div class="notification-card">
      <div class="notification-header">
        <div class="notification-icon">
          <el-icon :size="24"><Bell /></el-icon>
        </div>
        <button class="close-btn" @click.stop="handleClose">
          <el-icon :size="14"><Close /></el-icon>
        </button>
      </div>
      
      <div class="notification-content">
        <h3 class="notification-title">{{ title }}</h3>
        <p class="notification-desc" v-if="description">{{ description }}</p>
      </div>
      
      <div class="notification-footer">
        <span class="notification-time">Mini Todo</span>
      </div>
    </div>
  </div>
</template>

<style scoped>
.notification-window {
  width: 100vw;
  height: 100vh;
  display: flex;
  align-items: center;
  justify-content: center;
  background: transparent;
  cursor: pointer;
}

.notification-card {
  width: 100%;
  height: 100%;
  background: rgba(255, 255, 255, 0.98);
  border-radius: 12px;
  box-shadow: 0 8px 32px rgba(0, 0, 0, 0.15), 0 2px 8px rgba(0, 0, 0, 0.1);
  display: flex;
  flex-direction: column;
  overflow: hidden;
  border: 1px solid rgba(0, 0, 0, 0.05);
}

.notification-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 12px 14px 8px;
}

.notification-icon {
  width: 36px;
  height: 36px;
  display: flex;
  align-items: center;
  justify-content: center;
  background: linear-gradient(135deg, #3b82f6 0%, #06b6d4 100%);
  border-radius: 8px;
  color: #ffffff;
}

.close-btn {
  width: 24px;
  height: 24px;
  display: flex;
  align-items: center;
  justify-content: center;
  border: none;
  background: #f1f5f9;
  border-radius: 6px;
  color: #64748b;
  cursor: pointer;
  transition: all 0.2s ease;
}

.close-btn:hover {
  background: #e2e8f0;
  color: #334155;
}

.notification-content {
  flex: 1;
  padding: 0 14px;
  overflow: hidden;
}

.notification-title {
  margin: 0 0 4px;
  font-size: 14px;
  font-weight: 600;
  color: #1e293b;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.notification-desc {
  margin: 0;
  font-size: 12px;
  color: #64748b;
  line-height: 1.4;
  display: -webkit-box;
  -webkit-line-clamp: 2;
  -webkit-box-orient: vertical;
  overflow: hidden;
}

.notification-footer {
  padding: 8px 14px 12px;
  display: flex;
  align-items: center;
  justify-content: flex-end;
}

.notification-time {
  font-size: 11px;
  color: #94a3b8;
}
</style>
