<script setup lang="ts">
import { computed } from 'vue'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { useAppStore, APP_VERSION } from '@/stores'
import { openUrl } from '@tauri-apps/plugin-opener'
import { ElMessageBox } from 'element-plus'

defineProps<{
  showCalendarControls?: boolean
  currentMonthText?: string
}>()

const emit = defineEmits<{
  (e: 'open-settings'): void
  (e: 'calendar-prev'): void
  (e: 'calendar-next'): void
  (e: 'calendar-today'): void
}>()

const appStore = useAppStore()
const appWindow = getCurrentWindow()

// 是否固定模式
const isFixed = computed(() => appStore.isFixed)

// 是否有更新
const hasUpdate = computed(() => appStore.hasUpdate)

// 最新版本
const latestVersion = computed(() => appStore.latestVersion)

// 切换固定模式
async function toggleFixed() {
  await appStore.toggleFixedMode()
}

// 关闭窗口
async function closeWindow() {
  await appWindow.close()
}

// 打开设置
function openSettings() {
  emit('open-settings')
}

// 点击版本号
async function handleVersionClick() {
  if (hasUpdate.value) {
    try {
      await ElMessageBox.confirm(
        `发现新版本 ${latestVersion.value}，是否前往下载？`,
        '版本更新',
        {
          confirmButtonText: '前往下载',
          cancelButtonText: '稍后再说',
          type: 'info'
        }
      )
      // 用户点击确认，打开 GitHub Release 页面
      await openUrl(appStore.getReleasesUrl())
    } catch {
      // 用户点击取消，不做任何操作
    }
  }
}
</script>

<template>
  <div class="title-bar" :class="{ 'no-drag': isFixed }">
    <div class="title-left">
      <span class="app-title-wrapper">
        <span class="app-title">待办清单</span>
        <span 
          class="version-tag" 
          :class="{ 'has-update': hasUpdate }"
          @click="handleVersionClick"
        >
          v{{ APP_VERSION }}
          <span v-if="hasUpdate" class="update-dot"></span>
        </span>
      </span>
    </div>

    <!-- 日历控制区域（居中显示） -->
    <div v-if="showCalendarControls" class="title-center">
      <div class="calendar-nav">
        <el-button text size="small" class="nav-btn" @click="emit('calendar-prev')">
          <el-icon><ArrowLeft /></el-icon>
        </el-button>
        <span class="current-month">{{ currentMonthText }}</span>
        <el-button text size="small" class="nav-btn" @click="emit('calendar-next')">
          <el-icon><ArrowRight /></el-icon>
        </el-button>
      </div>
      <el-button size="small" class="today-btn" @click="emit('calendar-today')">今天</el-button>
    </div>
    
    <div class="title-right">
      <!-- 固定按钮 -->
      <button 
        class="title-btn" 
        :class="{ active: isFixed }"
        :title="isFixed ? '取消固定' : '固定窗口'"
        @click="toggleFixed"
      >
        <el-icon :size="16">
          <Lock v-if="isFixed" />
          <Unlock v-else />
        </el-icon>
      </button>

      <!-- 设置按钮 -->
      <button 
        class="title-btn"
        title="设置"
        @click="openSettings"
      >
        <el-icon :size="16">
          <Setting />
        </el-icon>
      </button>

      <!-- 关闭按钮 (非固定模式) -->
      <button 
        v-if="!isFixed"
        class="title-btn btn-close"
        title="关闭"
        @click="closeWindow"
      >
        <el-icon :size="16">
          <Close />
        </el-icon>
      </button>
    </div>
  </div>
</template>

<style scoped>
/* 固定模式下禁用拖拽 */
.title-bar.no-drag {
  -webkit-app-region: no-drag !important;
}

/* 标题包装器 */
.app-title-wrapper {
  display: flex;
  align-items: baseline;
}

/* 当有日历控制时，标题左侧固定 40% 宽度（与左侧面板对应） */
.title-bar:has(.title-center) {
  .title-left {
    width: 40%;
    min-width: 280px;
    flex-shrink: 0;
  }
}

/* 日历控制区域 - 左对齐（紧跟标题区域） */
.title-center {
  display: flex;
  align-items: center;
  gap: 12px;
  flex: 1;
  -webkit-app-region: no-drag;
}

.calendar-nav {
  display: flex;
  align-items: center;
  gap: 4px;
}

.current-month {
  font-size: 15px;
  font-weight: 600;
  min-width: 90px;
  text-align: center;
  color: var(--text-primary);
}

/* 非固定模式下的按钮样式 */
.nav-btn {
  color: var(--text-secondary) !important;

  &:hover {
    color: var(--primary) !important;
    background: var(--bg-tertiary) !important;
  }

  :deep(.el-icon) {
    color: inherit !important;
  }
}

.today-btn {
  color: var(--text-secondary) !important;
  background: var(--bg-secondary) !important;
  border-color: var(--border) !important;

  &:hover {
    color: var(--primary) !important;
    background: var(--bg-tertiary) !important;
    border-color: var(--primary) !important;
  }
}

/* 固定模式下的按钮样式 */
.title-bar.no-drag {
  .nav-btn {
    color: var(--text-primary) !important;

    &:hover {
      color: var(--primary-light) !important;
      background: rgba(255, 255, 255, 0.1) !important;
    }
  }

  .today-btn {
    color: var(--text-primary) !important;
    background: transparent !important;
    border-color: rgba(255, 255, 255, 0.4) !important;

    &:hover {
      color: white !important;
      background: rgba(255, 255, 255, 0.15) !important;
      border-color: rgba(255, 255, 255, 0.5) !important;
    }
  }
}

/* 版本号标签 */
.version-tag {
  margin-left: 4px;
  font-size: 11px;
  color: var(--text-tertiary, rgba(255, 255, 255, 0.45));
  cursor: default;
  user-select: none;
  -webkit-app-region: no-drag;
  white-space: nowrap;
}

.version-tag.has-update {
  cursor: pointer;
}

.version-tag.has-update:hover {
  color: var(--text-secondary, rgba(255, 255, 255, 0.6));
}

/* 更新红点 */
.update-dot {
  display: inline-block;
  width: 6px;
  height: 6px;
  margin-left: 2px;
  background: #EF4444;
  border-radius: 50%;
  vertical-align: middle;
  animation: pulse 2s infinite;
}

@keyframes pulse {
  0% {
    box-shadow: 0 0 0 0 rgba(239, 68, 68, 0.7);
  }
  70% {
    box-shadow: 0 0 0 4px rgba(239, 68, 68, 0);
  }
  100% {
    box-shadow: 0 0 0 0 rgba(239, 68, 68, 0);
  }
}
</style>
