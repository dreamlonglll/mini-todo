<script setup lang="ts">
import { ref, onMounted } from 'vue'
import { useSchedulerStore } from '@/stores'
import { STRATEGY_LABELS } from '@/types/scheduler'

const schedulerStore = useSchedulerStore()

const activeTab = ref('cron')
const loading = ref(false)

onMounted(async () => {
  await refreshAll()
})

async function refreshAll() {
  loading.value = true
  try {
    await Promise.all([
      schedulerStore.refreshSchedulerStatus(),
      schedulerStore.loadScheduledTodos(),
      schedulerStore.loadTriggerTodos(),
      schedulerStore.loadTemplates(),
    ])
  } finally {
    loading.value = false
  }
}

async function toggleScheduler() {
  try {
    if (schedulerStore.isRunning) {
      await schedulerStore.stopScheduler()
    } else {
      await schedulerStore.startScheduler()
    }
  } catch (e) {
    console.error('切换调度器失败:', e)
  }
}

async function toggleTask(todoId: number, enabled: boolean) {
  try {
    await schedulerStore.updateTodoScheduleConfig(todoId, undefined, undefined, enabled)
    await refreshAll()
  } catch (e) {
    console.error('切换任务失败:', e)
  }
}
</script>

<template>
  <div class="scheduler-panel">
    <div class="panel-toolbar">
      <div class="scheduler-status">
        <span class="status-label">调度引擎</span>
        <el-switch
          :model-value="schedulerStore.isRunning"
          active-text="运行中"
          inactive-text="已停止"
          @change="toggleScheduler"
        />
      </div>
      <el-button size="small" :loading="loading" @click="refreshAll">
        <el-icon><Refresh /></el-icon>
        刷新
      </el-button>
    </div>

    <el-tabs v-model="activeTab" class="scheduler-tabs">
      <el-tab-pane label="定时任务" name="cron">
        <div v-if="schedulerStore.scheduledTodos.length === 0" class="empty-state">
          暂无定时任务
        </div>
        <div v-else class="task-list">
          <div
            v-for="task in schedulerStore.scheduledTodos"
            :key="task.id"
            class="task-card"
          >
            <div class="task-header">
              <span class="task-title">{{ task.title }}</span>
              <el-switch
                :model-value="task.scheduleEnabled"
                size="small"
                @change="toggleTask(task.id, $event as boolean)"
              />
            </div>
            <div class="task-meta">
              <div class="meta-item">
                <el-icon><Timer /></el-icon>
                <span>{{ task.cronDescription || task.cronExpression }}</span>
              </div>
              <div v-if="task.nextRun" class="meta-item">
                <el-icon><Clock /></el-icon>
                <span>下次: {{ task.nextRun }}</span>
              </div>
              <div v-if="task.lastScheduledRun" class="meta-item">
                <el-icon><Finished /></el-icon>
                <span>上次: {{ task.lastScheduledRun }}</span>
              </div>
              <div class="meta-item">
                <el-icon><List /></el-icon>
                <span>{{ task.pendingSubtasks }} 个待执行子任务</span>
              </div>
            </div>
          </div>
        </div>
      </el-tab-pane>

      <el-tab-pane label="触发器" name="triggers">
        <div v-if="schedulerStore.triggerTodos.length === 0" class="empty-state">
          暂无触发器任务
        </div>
        <div v-else class="task-list">
          <div
            v-for="task in schedulerStore.triggerTodos"
            :key="task.id"
            class="task-card"
          >
            <div class="task-header">
              <div class="task-title-group">
                <span class="task-title">{{ task.title }}</span>
                <el-tag size="small" effect="light">
                  {{ STRATEGY_LABELS[task.strategy as keyof typeof STRATEGY_LABELS] || task.strategy }}
                </el-tag>
              </div>
              <el-switch
                :model-value="task.scheduleEnabled"
                size="small"
                @change="toggleTask(task.id, $event as boolean)"
              />
            </div>
            <div class="task-meta">
              <div v-if="task.projectPath" class="meta-item">
                <el-icon><Folder /></el-icon>
                <span class="path-text">{{ task.projectPath }}</span>
              </div>
              <div v-if="task.lastScheduledRun" class="meta-item">
                <el-icon><Finished /></el-icon>
                <span>上次触发: {{ task.lastScheduledRun }}</span>
              </div>
              <div class="meta-item">
                <el-icon><List /></el-icon>
                <span>{{ task.pendingSubtasks }} 个待执行子任务</span>
              </div>
            </div>
          </div>
        </div>
      </el-tab-pane>

      <el-tab-pane label="模板" name="templates">
        <div v-if="schedulerStore.templates.length === 0" class="empty-state">
          暂无 Prompt 模板
        </div>
        <div v-else class="task-list">
          <div
            v-for="tpl in schedulerStore.templates"
            :key="tpl.id"
            class="task-card"
          >
            <div class="task-header">
              <div class="task-title-group">
                <span class="task-title">{{ tpl.name }}</span>
                <el-tag v-if="tpl.category" size="small" type="info" effect="light">
                  {{ tpl.category }}
                </el-tag>
                <el-tag v-if="tpl.isBuiltin" size="small" type="warning" effect="light">
                  内置
                </el-tag>
              </div>
            </div>
            <div v-if="tpl.description" class="task-meta">
              <span class="tpl-desc">{{ tpl.description }}</span>
            </div>
          </div>
        </div>
      </el-tab-pane>
    </el-tabs>
  </div>
</template>

<style scoped>
.scheduler-panel {
  width: 100%;
}

.panel-toolbar {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-bottom: 12px;
}

.scheduler-status {
  display: flex;
  align-items: center;
  gap: 8px;
}

.status-label {
  font-size: 13px;
  color: var(--text-secondary);
}

.scheduler-tabs {
  margin-top: 4px;
}

.empty-state {
  text-align: center;
  color: var(--text-tertiary);
  padding: 24px 0;
  font-size: 13px;
}

.task-list {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.task-card {
  padding: 10px 12px;
  background: #f8fafc;
  border-radius: 6px;
  border: 1px solid #e2e8f0;
}

.task-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
}

.task-title-group {
  display: flex;
  align-items: center;
  gap: 6px;
}

.task-title {
  font-size: 13px;
  font-weight: 500;
  color: var(--text-primary);
}

.task-meta {
  margin-top: 6px;
  display: flex;
  flex-direction: column;
  gap: 3px;
}

.meta-item {
  display: flex;
  align-items: center;
  gap: 4px;
  font-size: 12px;
  color: var(--text-secondary);
}

.path-text {
  max-width: 200px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.tpl-desc {
  font-size: 12px;
  color: var(--text-secondary);
}
</style>
