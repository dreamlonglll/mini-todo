import { defineStore } from 'pinia'
import { ref, computed } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import type { Todo, UpdateTodoRequest, ViewMode, QuadrantType } from '@/types'
import { QUADRANTS } from '@/types'
import { notifyError } from '@/utils/notify'

export const useTodoStore = defineStore('todo', () => {
  // 状态
  const todos = ref<Todo[]>([])
  const viewMode = ref<ViewMode>('list')

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

  // 按象限分组的待办（仅未完成）
  const todosByQuadrant = computed(() => {
    const result: Record<QuadrantType, Todo[]> = {
      [QUADRANTS.IMPORTANT_URGENT]: [],
      [QUADRANTS.IMPORTANT_NOT_URGENT]: [],
      [QUADRANTS.URGENT_NOT_IMPORTANT]: [],
      [QUADRANTS.NOT_URGENT_NOT_IMPORTANT]: [],
    }

    pendingTodos.value.forEach(todo => {
      const quadrant = todo.quadrant as QuadrantType
      if (result[quadrant]) {
        result[quadrant].push(todo)
      } else {
        // 默认放入第一象限
        result[QUADRANTS.IMPORTANT_URGENT].push(todo)
      }
    })

    return result
  })

  // ===== 全量拉取（get_todos）=====
  //
  // 触发源很多（启动、窗口聚焦、子窗口关闭、同步完成、变更计数轮询），可能接连发生：
  // - 单飞：同一时刻只有一个拉取在途。在途期间再请求只记一笔，结束后补拉一次——在途那次
  //   可能读在了触发新请求的那次改动之前；所有调用方都等到补拉结束
  // - 序号：本窗口的写操作（完成、删除、排序……）会就地更新 todos。拉取期间发生过写操作时，
  //   这次结果可能早于该写操作，直接丢弃并补拉，避免列表闪回旧状态
  let fetching = false
  let fetchQueued = false
  let fetchWaiters: Array<(ok: boolean) => void> = []
  let mutationSeq = 0
  // 连续失败只在第一次弹提示，成功后复位（后台轮询 / 聚焦也会触发拉取）
  let fetchFailing = false

  // 当前 todos 对应的本地变更计数（get_change_seq）；null = 未知（尚未拉取或后端不支持）
  let loadedChangeSeq: number | null = null
  let changeSeqUnsupported = false

  /** 本地 todos / subtasks 的变更计数，任意增删改都会增大；读取失败返回 null */
  async function readChangeSeq(): Promise<number | null> {
    try {
      return await invoke<number>('get_change_seq')
    } catch (e) {
      if (!changeSeqUnsupported) {
        changeSeqUnsupported = true
        console.warn('Failed to read change seq:', e)
      }
      return null
    }
  }

  async function runFetchLoop() {
    let ok = false
    try {
      do {
        fetchQueued = false
        const mutationSeqAtStart = mutationSeq
        try {
          // 先记变更计数再读数据：读数据期间发生的改动会让下一次轮询看到新计数，不会漏
          const changeSeq = await readChangeSeq()
          const result = await invoke<Todo[]>('get_todos')
          if (mutationSeqAtStart === mutationSeq) {
            todos.value = result
            loadedChangeSeq = changeSeq
          } else {
            fetchQueued = true
          }
          ok = true
          fetchFailing = false
        } catch (e) {
          ok = false
          if (fetchFailing) {
            console.error('Failed to fetch todos:', e)
          } else {
            fetchFailing = true
            notifyError(e, '加载待办失败')
          }
        }
      } while (fetchQueued)
    } finally {
      fetching = false
      const waiters = fetchWaiters
      fetchWaiters = []
      waiters.forEach(resolve => resolve(ok))
    }
  }

  /** 全量拉取待办（单飞）；返回最终是否拉取成功 */
  function fetchTodos(): Promise<boolean> {
    return new Promise<boolean>(resolve => {
      fetchWaiters.push(resolve)
      if (fetching) {
        fetchQueued = true
        return
      }
      fetching = true
      void runFetchLoop()
    })
  }

  /**
   * 变更计数与当前列表对应的计数不同（其它窗口、同步、提醒调度写过库）时才全量拉取。
   * 用于主窗口的高频轮询；拉取在途时跳过（在途那次结束后计数自然对齐，下一轮再比较）
   */
  async function refreshIfChanged(): Promise<void> {
    if (fetching) return
    const seq = await readChangeSeq()
    if (seq === null || seq === loadedChangeSeq || fetching) return
    await fetchTodos()
  }

  async function updateTodo(id: number, data: UpdateTodoRequest): Promise<boolean> {
    mutationSeq++
    try {
      const updatedTodo = await invoke<Todo>('update_todo', { id, data })
      const index = todos.value.findIndex(t => t.id === id)
      if (index !== -1) {
        todos.value[index] = updatedTodo
      }
      return true
    } catch (e) {
      notifyError(e, '更新待办失败')
      return false
    }
  }

  async function deleteTodo(id: number): Promise<boolean> {
    mutationSeq++
    try {
      await invoke('delete_todo', { id })
      todos.value = todos.value.filter(t => t.id !== id)
      return true
    } catch (e) {
      notifyError(e, '删除待办失败')
      return false
    }
  }

  async function toggleComplete(id: number): Promise<boolean> {
    const todo = todos.value.find(t => t.id === id)
    if (!todo) return false
    return updateTodo(id, { completed: !todo.completed })
  }

  async function reorderTodos(orderedIds: number[]): Promise<boolean> {
    mutationSeq++
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
      notifyError(e, '保存排序失败')
      return false
    }
  }

  // 更新待办的象限（color 传入时一并更新，用于颜色跟随象限）
  async function updateTodoQuadrant(id: number, quadrant: QuadrantType, color?: string): Promise<boolean> {
    return updateTodo(id, color ? { quadrant, color } : { quadrant })
  }

  // 设置视图模式
  function setViewMode(mode: ViewMode) {
    viewMode.value = mode
  }

  // 加载视图模式设置
  async function loadViewMode() {
    try {
      const savedMode = await invoke<string | null>('get_setting', { key: 'view_mode' })
      if (savedMode === 'list' || savedMode === 'quadrant') {
        viewMode.value = savedMode
      }
    } catch (e) {
      console.error('Failed to load view mode:', e)
    }
  }

  // 保存视图模式设置
  async function saveViewMode() {
    try {
      await invoke('set_setting', { key: 'view_mode', value: viewMode.value })
    } catch (e) {
      console.error('Failed to save view mode:', e)
    }
  }

  return {
    // 状态
    todos,
    viewMode,
    // 计算属性
    pendingTodos,
    todoCount,
    todosByQuadrant,
    // 方法
    fetchTodos,
    refreshIfChanged,
    deleteTodo,
    toggleComplete,
    reorderTodos,
    updateTodoQuadrant,
    setViewMode,
    loadViewMode,
    saveViewMode,
  }
})
