import { defineStore } from 'pinia'
import { ref, computed } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import type { Todo, CreateTodoRequest, UpdateTodoRequest, SubTask, CreateSubTaskRequest, UpdateSubTaskRequest } from '@/types'

export const useTodoStore = defineStore('todo', () => {
  // 状态
  const todos = ref<Todo[]>([])
  const loading = ref(false)
  const error = ref<string | null>(null)

  // 计算属性
  const pendingTodos = computed(() => 
    todos.value
      .filter(t => !t.completed)
      .sort((a, b) => a.sortOrder - b.sortOrder)
  )

  const completedTodos = computed(() => 
    todos.value
      .filter(t => t.completed)
      .sort((a, b) => b.sortOrder - a.sortOrder)
  )

  const todoCount = computed(() => ({
    total: todos.value.length,
    pending: pendingTodos.value.length,
    completed: completedTodos.value.length
  }))

  // 操作方法
  async function fetchTodos() {
    loading.value = true
    error.value = null
    try {
      todos.value = await invoke<Todo[]>('get_todos')
    } catch (e) {
      error.value = String(e)
      console.error('Failed to fetch todos:', e)
    } finally {
      loading.value = false
    }
  }

  async function addTodo(data: CreateTodoRequest): Promise<Todo | null> {
    try {
      const newTodo = await invoke<Todo>('create_todo', { data })
      todos.value.push(newTodo)
      return newTodo
    } catch (e) {
      error.value = String(e)
      console.error('Failed to add todo:', e)
      return null
    }
  }

  async function updateTodo(id: number, data: UpdateTodoRequest): Promise<boolean> {
    try {
      const updatedTodo = await invoke<Todo>('update_todo', { id, data })
      const index = todos.value.findIndex(t => t.id === id)
      if (index !== -1) {
        todos.value[index] = updatedTodo
      }
      return true
    } catch (e) {
      error.value = String(e)
      console.error('Failed to update todo:', e)
      return false
    }
  }

  async function deleteTodo(id: number): Promise<boolean> {
    try {
      await invoke('delete_todo', { id })
      todos.value = todos.value.filter(t => t.id !== id)
      return true
    } catch (e) {
      error.value = String(e)
      console.error('Failed to delete todo:', e)
      return false
    }
  }

  async function toggleComplete(id: number): Promise<boolean> {
    const todo = todos.value.find(t => t.id === id)
    if (!todo) return false
    return updateTodo(id, { completed: !todo.completed })
  }

  async function reorderTodos(orderedIds: number[]): Promise<boolean> {
    try {
      await invoke('reorder_todos', { ids: orderedIds })
      // 更新本地排序
      orderedIds.forEach((id, index) => {
        const todo = todos.value.find(t => t.id === id)
        if (todo) {
          todo.sortOrder = index
        }
      })
      return true
    } catch (e) {
      error.value = String(e)
      console.error('Failed to reorder todos:', e)
      return false
    }
  }

  // 子任务操作
  async function addSubTask(data: CreateSubTaskRequest): Promise<SubTask | null> {
    try {
      const newSubTask = await invoke<SubTask>('create_subtask', { data })
      const todo = todos.value.find(t => t.id === data.parentId)
      if (todo) {
        todo.subtasks.push(newSubTask)
      }
      return newSubTask
    } catch (e) {
      error.value = String(e)
      console.error('Failed to add subtask:', e)
      return null
    }
  }

  async function updateSubTask(id: number, data: UpdateSubTaskRequest): Promise<boolean> {
    try {
      const updatedSubTask = await invoke<SubTask>('update_subtask', { id, data })
      for (const todo of todos.value) {
        const index = todo.subtasks.findIndex(s => s.id === id)
        if (index !== -1) {
          todo.subtasks[index] = updatedSubTask
          break
        }
      }
      return true
    } catch (e) {
      error.value = String(e)
      console.error('Failed to update subtask:', e)
      return false
    }
  }

  async function deleteSubTask(id: number): Promise<boolean> {
    try {
      await invoke('delete_subtask', { id })
      for (const todo of todos.value) {
        const index = todo.subtasks.findIndex(s => s.id === id)
        if (index !== -1) {
          todo.subtasks.splice(index, 1)
          break
        }
      }
      return true
    } catch (e) {
      error.value = String(e)
      console.error('Failed to delete subtask:', e)
      return false
    }
  }

  async function toggleSubTaskComplete(id: number): Promise<boolean> {
    for (const todo of todos.value) {
      const subtask = todo.subtasks.find(s => s.id === id)
      if (subtask) {
        return updateSubTask(id, { completed: !subtask.completed })
      }
    }
    return false
  }

  return {
    // 状态
    todos,
    loading,
    error,
    // 计算属性
    pendingTodos,
    completedTodos,
    todoCount,
    // 方法
    fetchTodos,
    addTodo,
    updateTodo,
    deleteTodo,
    toggleComplete,
    reorderTodos,
    addSubTask,
    updateSubTask,
    deleteSubTask,
    toggleSubTaskComplete
  }
})
