import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'
import type { Todo } from '@/types'

const { invokeMock, notifyErrorMock } = vi.hoisted(() => ({
  invokeMock: vi.fn(),
  notifyErrorMock: vi.fn(),
}))
vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }))
// 真实实现会弹 ElMessage（依赖 DOM 与 Element Plus），这里只关心是否被调用
vi.mock('@/utils/notify', () => ({ notifyError: notifyErrorMock }))

import { useTodoStore } from './todoStore'

function todo(id: number, extra: Partial<Todo> = {}): Todo {
  return {
    id,
    title: `t${id}`,
    description: null,
    color: '#10B981',
    quadrant: 4,
    notifyAt: null,
    notifyBefore: 0,
    notified: false,
    completed: false,
    sortOrder: id,
    startTime: null,
    endTime: null,
    createdAt: '2026-10-02 09:00:00',
    updatedAt: '2026-10-02 09:00:00',
    subtasks: [],
    ...extra,
  }
}

/** 手动控制何时返回的 Promise */
function deferred<T>() {
  let resolve!: (value: T) => void
  let reject!: (reason: unknown) => void
  const promise = new Promise<T>((res, rej) => {
    resolve = res
    reject = rej
  })
  return { promise, resolve, reject }
}

/**
 * 模拟后端：get_todos 挂起到测试手动 resolve；get_change_seq 返回当前计数；
 * update_todo 立即返回更新后的待办
 */
function fakeBackend() {
  const pendingFetches: Array<ReturnType<typeof deferred<Todo[]>>> = []
  const state = { changeSeq: 1 as number | Error }
  invokeMock.mockImplementation((cmd: string, args?: { id: number; data: Partial<Todo> }) => {
    switch (cmd) {
      case 'get_change_seq':
        return state.changeSeq instanceof Error
          ? Promise.reject(state.changeSeq)
          : Promise.resolve(state.changeSeq)
      case 'get_todos': {
        const d = deferred<Todo[]>()
        pendingFetches.push(d)
        return d.promise
      }
      case 'update_todo':
        return Promise.resolve(todo(args!.id, args!.data))
      default:
        return Promise.reject(new Error(`unexpected command ${cmd}`))
    }
  })
  const fetchCount = () => invokeMock.mock.calls.filter(([cmd]) => cmd === 'get_todos').length
  return { pendingFetches, state, fetchCount }
}

beforeEach(() => {
  setActivePinia(createPinia())
  invokeMock.mockReset()
  notifyErrorMock.mockReset()
})

describe('fetchTodos', () => {
  it('coalesces concurrent calls into one in-flight fetch plus one trailing fetch', async () => {
    const backend = fakeBackend()
    const store = useTodoStore()

    const calls = [store.fetchTodos(), store.fetchTodos(), store.fetchTodos()]
    await vi.waitFor(() => expect(backend.fetchCount()).toBe(1))

    // 在途期间又来的请求：在途那次结束后补拉一次（它可能读在了那些改动之前）
    backend.pendingFetches[0].resolve([todo(1)])
    await vi.waitFor(() => expect(backend.fetchCount()).toBe(2))
    backend.pendingFetches[1].resolve([todo(1), todo(2)])

    await expect(Promise.all(calls)).resolves.toEqual([true, true, true])
    expect(backend.fetchCount()).toBe(2)
    expect(store.todos.map(t => t.id)).toEqual([1, 2])
  })

  it('runs a fresh fetch for calls made after the previous one finished', async () => {
    const backend = fakeBackend()
    const store = useTodoStore()

    const first = store.fetchTodos()
    await vi.waitFor(() => expect(backend.fetchCount()).toBe(1))
    backend.pendingFetches[0].resolve([todo(1)])
    await first

    const second = store.fetchTodos()
    await vi.waitFor(() => expect(backend.fetchCount()).toBe(2))
    backend.pendingFetches[1].resolve([todo(3)])
    await expect(second).resolves.toBe(true)
    expect(store.todos.map(t => t.id)).toEqual([3])
  })

  it('drops a response that may predate a local write and fetches again', async () => {
    const backend = fakeBackend()
    const store = useTodoStore()

    const initial = store.fetchTodos()
    await vi.waitFor(() => expect(backend.fetchCount()).toBe(1))
    backend.pendingFetches[0].resolve([todo(1)])
    await initial

    // 拉取在途时本窗口完成了一个待办
    const inFlight = store.fetchTodos()
    await vi.waitFor(() => expect(backend.fetchCount()).toBe(2))
    await store.toggleComplete(1)
    expect(store.todos[0].completed).toBe(true)

    // 在途那次读的是写之前的数据：不能把列表闪回未完成
    backend.pendingFetches[1].resolve([todo(1, { completed: false })])
    await vi.waitFor(() => expect(backend.fetchCount()).toBe(3))
    expect(store.todos[0].completed).toBe(true)

    backend.pendingFetches[2].resolve([todo(1, { completed: true })])
    await expect(inFlight).resolves.toBe(true)
    expect(store.todos[0].completed).toBe(true)
  })

  it('notifies only the first of consecutive failures and resets after a success', async () => {
    const backend = fakeBackend()
    const store = useTodoStore()
    const consoleError = vi.spyOn(console, 'error').mockImplementation(() => {})

    const run = async (outcome: Todo[] | Error) => {
      const done = store.fetchTodos()
      await vi.waitFor(() => expect(backend.pendingFetches.length).toBeGreaterThan(0))
      const d = backend.pendingFetches.shift()!
      if (outcome instanceof Error) d.reject(outcome)
      else d.resolve(outcome)
      return done
    }

    await expect(run(new Error('db locked'))).resolves.toBe(false)
    await expect(run(new Error('db locked'))).resolves.toBe(false)
    expect(notifyErrorMock).toHaveBeenCalledTimes(1)
    expect(consoleError).toHaveBeenCalledTimes(1)

    await expect(run([todo(1)])).resolves.toBe(true)
    await expect(run(new Error('again'))).resolves.toBe(false)
    expect(notifyErrorMock).toHaveBeenCalledTimes(2)
    consoleError.mockRestore()
  })
})

describe('refreshIfChanged', () => {
  async function loadedStore(backend: ReturnType<typeof fakeBackend>) {
    const store = useTodoStore()
    const done = store.fetchTodos()
    await vi.waitFor(() => expect(backend.fetchCount()).toBe(1))
    backend.pendingFetches[0].resolve([todo(1)])
    await done
    return store
  }

  it('skips the full fetch while the change counter is unchanged', async () => {
    const backend = fakeBackend()
    backend.state.changeSeq = 5
    const store = await loadedStore(backend)

    await store.refreshIfChanged()
    expect(backend.fetchCount()).toBe(1)
  })

  it('fetches when another window, the scheduler or a sync wrote to the database', async () => {
    const backend = fakeBackend()
    backend.state.changeSeq = 5
    const store = await loadedStore(backend)

    backend.state.changeSeq = 6
    const refresh = store.refreshIfChanged()
    await vi.waitFor(() => expect(backend.fetchCount()).toBe(2))
    backend.pendingFetches[1].resolve([todo(1), todo(2)])
    await refresh
    expect(store.todos).toHaveLength(2)

    // 计数已对齐：下一轮不再拉取
    await store.refreshIfChanged()
    expect(backend.fetchCount()).toBe(2)
  })

  it('does nothing when the change counter cannot be read', async () => {
    const backend = fakeBackend()
    const consoleWarn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    const store = await loadedStore(backend)

    backend.state.changeSeq = new Error('command not found')
    await expect(store.refreshIfChanged()).resolves.toBeUndefined()
    expect(backend.fetchCount()).toBe(1)
    consoleWarn.mockRestore()
  })
})
