<script setup lang="ts">
import { ref, watch, computed } from 'vue'
import { useTodoStore } from '@/stores'
import type { Todo, Priority, CreateTodoRequest, UpdateTodoRequest } from '@/types'

const props = defineProps<{
  visible: boolean
  todo: Todo | null
}>()

const emit = defineEmits<{
  (e: 'update:visible', value: boolean): void
  (e: 'close'): void
}>()

const todoStore = useTodoStore()

// 表单数据
const form = ref({
  title: '',
  description: '',
  priority: 'medium' as Priority,
  notifyAt: null as string | null,
  notifyBefore: 15
})

// 新子任务输入
const newSubtaskTitle = ref('')

// 是否编辑模式
const isEdit = computed(() => props.todo !== null)

// 当前待办的子任务列表
const subtasks = computed(() => props.todo?.subtasks || [])

// 提前通知选项
const notifyBeforeOptions = [
  { label: '准时', value: 0 },
  { label: '5 分钟前', value: 5 },
  { label: '15 分钟前', value: 15 },
  { label: '30 分钟前', value: 30 },
  { label: '1 小时前', value: 60 },
  { label: '自定义', value: -1 }
]

// 自定义提前时间
const customNotifyBefore = ref(15)
const isCustomNotifyBefore = ref(false)

// 监听弹窗显示，初始化表单
watch(() => props.visible, (visible) => {
  if (visible && props.todo) {
    form.value = {
      title: props.todo.title,
      description: props.todo.description || '',
      priority: props.todo.priority as Priority,
      notifyAt: props.todo.notifyAt,
      notifyBefore: props.todo.notifyBefore
    }
    
    // 检查是否是自定义时间
    const presetValues = [0, 5, 15, 30, 60]
    isCustomNotifyBefore.value = !presetValues.includes(props.todo.notifyBefore)
    if (isCustomNotifyBefore.value) {
      customNotifyBefore.value = props.todo.notifyBefore
    }
  } else if (visible) {
    // 新建模式，重置表单
    form.value = {
      title: '',
      description: '',
      priority: 'medium',
      notifyAt: null,
      notifyBefore: 15
    }
    isCustomNotifyBefore.value = false
  }
})

// 处理提前通知选择
function handleNotifyBeforeChange(value: number) {
  if (value === -1) {
    isCustomNotifyBefore.value = true
    form.value.notifyBefore = customNotifyBefore.value
  } else {
    isCustomNotifyBefore.value = false
    form.value.notifyBefore = value
  }
}

// 保存待办
async function handleSave() {
  if (!form.value.title.trim()) {
    return
  }

  if (isCustomNotifyBefore.value) {
    form.value.notifyBefore = customNotifyBefore.value
  }

  if (isEdit.value && props.todo) {
    // 更新
    const data: UpdateTodoRequest = {
      title: form.value.title,
      description: form.value.description || null,
      priority: form.value.priority,
      notifyAt: form.value.notifyAt,
      notifyBefore: form.value.notifyBefore
    }
    await todoStore.updateTodo(props.todo.id, data)
  } else {
    // 新建
    const data: CreateTodoRequest = {
      title: form.value.title,
      description: form.value.description || undefined,
      priority: form.value.priority,
      notifyAt: form.value.notifyAt || undefined,
      notifyBefore: form.value.notifyBefore
    }
    await todoStore.addTodo(data)
  }

  handleClose()
}

// 添加子任务
async function addSubtask() {
  if (!newSubtaskTitle.value.trim() || !props.todo) return
  
  await todoStore.addSubTask({
    parentId: props.todo.id,
    title: newSubtaskTitle.value.trim()
  })
  
  newSubtaskTitle.value = ''
}

// 切换子任务完成状态
async function toggleSubtask(subtaskId: number) {
  await todoStore.toggleSubTaskComplete(subtaskId)
}

// 删除子任务
async function deleteSubtask(subtaskId: number) {
  await todoStore.deleteSubTask(subtaskId)
}

// 关闭弹窗
function handleClose() {
  emit('update:visible', false)
  emit('close')
}
</script>

<template>
  <el-dialog
    :model-value="visible"
    :title="isEdit ? '编辑待办' : '新建待办'"
    width="360px"
    :close-on-click-modal="false"
    @update:model-value="handleClose"
  >
    <el-form label-position="top" :model="form">
      <!-- 标题 -->
      <el-form-item label="标题" required>
        <el-input 
          v-model="form.title" 
          placeholder="请输入待办标题"
          maxlength="100"
        />
      </el-form-item>

      <!-- 描述 -->
      <el-form-item label="描述">
        <el-input 
          v-model="form.description" 
          type="textarea"
          :rows="3"
          placeholder="添加详细描述..."
          maxlength="500"
        />
      </el-form-item>

      <!-- 优先级 -->
      <el-form-item label="优先级">
        <el-radio-group v-model="form.priority">
          <el-radio value="high">
            <span class="priority-label high">高</span>
          </el-radio>
          <el-radio value="medium">
            <span class="priority-label medium">中</span>
          </el-radio>
          <el-radio value="low">
            <span class="priority-label low">低</span>
          </el-radio>
        </el-radio-group>
      </el-form-item>

      <!-- 通知时间 -->
      <el-form-item label="提醒时间">
        <el-date-picker
          v-model="form.notifyAt"
          type="datetime"
          placeholder="选择提醒时间"
          format="YYYY-MM-DD HH:mm"
          value-format="YYYY-MM-DDTHH:mm:ss"
          style="width: 100%"
        />
      </el-form-item>

      <!-- 提前通知 -->
      <el-form-item v-if="form.notifyAt" label="提前提醒">
        <el-select 
          :model-value="isCustomNotifyBefore ? -1 : form.notifyBefore"
          @change="handleNotifyBeforeChange"
          style="width: 100%"
        >
          <el-option 
            v-for="opt in notifyBeforeOptions"
            :key="opt.value"
            :label="opt.label"
            :value="opt.value"
          />
        </el-select>
        
        <el-input-number
          v-if="isCustomNotifyBefore"
          v-model="customNotifyBefore"
          :min="1"
          :max="1440"
          style="width: 100%; margin-top: 8px"
        >
          <template #suffix>分钟前</template>
        </el-input-number>
      </el-form-item>

      <!-- 子任务 (仅编辑模式) -->
      <el-form-item v-if="isEdit" label="子任务">
        <div class="subtask-editor">
          <!-- 添加子任务 -->
          <div class="add-subtask">
            <el-input
              v-model="newSubtaskTitle"
              placeholder="添加子任务..."
              @keyup.enter="addSubtask"
            >
              <template #append>
                <el-button @click="addSubtask">
                  <el-icon><Plus /></el-icon>
                </el-button>
              </template>
            </el-input>
          </div>

          <!-- 子任务列表 -->
          <div class="subtask-list-editor">
            <div 
              v-for="subtask in subtasks" 
              :key="subtask.id" 
              class="subtask-item-editor"
            >
              <el-checkbox 
                :model-value="subtask.completed"
                @change="toggleSubtask(subtask.id)"
              />
              <span 
                class="subtask-title"
                :class="{ completed: subtask.completed }"
              >
                {{ subtask.title }}
              </span>
              <el-button 
                type="danger" 
                text 
                size="small"
                @click="deleteSubtask(subtask.id)"
              >
                <el-icon><Delete /></el-icon>
              </el-button>
            </div>
          </div>
        </div>
      </el-form-item>
    </el-form>

    <template #footer>
      <el-button @click="handleClose">取消</el-button>
      <el-button type="primary" @click="handleSave">
        {{ isEdit ? '保存' : '创建' }}
      </el-button>
    </template>
  </el-dialog>
</template>

<style scoped>
.priority-label {
  padding: 2px 8px;
  border-radius: 4px;
  font-size: 12px;

  &.high {
    background: var(--priority-high-bg);
    color: var(--priority-high);
  }

  &.medium {
    background: var(--priority-medium-bg);
    color: var(--priority-medium);
  }

  &.low {
    background: var(--priority-low-bg);
    color: var(--priority-low);
  }
}

.subtask-editor {
  width: 100%;
}

.add-subtask {
  margin-bottom: 12px;
}

.subtask-list-editor {
  max-height: 200px;
  overflow-y: auto;
}

.subtask-item-editor {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 6px 0;
  border-bottom: 1px solid var(--border-light);

  &:last-child {
    border-bottom: none;
  }

  .subtask-title {
    flex: 1;
    font-size: 13px;

    &.completed {
      text-decoration: line-through;
      color: var(--text-tertiary);
    }
  }
}
</style>
