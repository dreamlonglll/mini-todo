<script setup lang="ts">
import { computed } from 'vue'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { useAppStore } from '@/stores'

const emit = defineEmits<{
  (e: 'open-settings'): void
}>()

const appStore = useAppStore()
const appWindow = getCurrentWindow()

// 是否固定模式
const isFixed = computed(() => appStore.isFixed)

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
</script>

<template>
  <div class="title-bar" :class="{ 'no-drag': isFixed }">
    <div class="title-left">
      <span class="app-title">待办清单</span>
      <!-- <span class="archive-btn">归档</span> -->
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
</style>
