import { WebviewWindow } from '@tauri-apps/api/webviewWindow'
import { currentMonitor, primaryMonitor, getCurrentWindow } from '@tauri-apps/api/window'

export interface OpenLogWindowParams {
  subtaskId?: number
  todoId?: number
  stepOrder?: number
  taskId?: string
  title?: string
}

export async function openLogWindow(params: OpenLogWindowParams): Promise<WebviewWindow | null> {
  const queryParts: string[] = []
  if (params.subtaskId != null) queryParts.push(`subtaskId=${params.subtaskId}`)
  if (params.todoId != null) queryParts.push(`todoId=${params.todoId}`)
  if (params.stepOrder != null) queryParts.push(`stepOrder=${params.stepOrder}`)
  if (params.taskId) queryParts.push(`taskId=${encodeURIComponent(params.taskId)}`)
  if (params.title) queryParts.push(`title=${encodeURIComponent(params.title)}`)

  const url = `#/agent-log?${queryParts.join('&')}`
  const label = `agent-log-${Date.now()}`

  const windowWidth = 700
  const windowHeight = 550

  try {
    let x: number, y: number
    const monitor = await currentMonitor() || await primaryMonitor()
    if (monitor) {
      const s = monitor.scaleFactor
      const mx = monitor.position.x / s
      const my = monitor.position.y / s
      const mw = monitor.size.width / s
      const mh = monitor.size.height / s
      x = Math.round(mx + (mw - windowWidth) / 2)
      y = Math.round(my + (mh - windowHeight) / 2)
    } else {
      const appWindow = getCurrentWindow()
      const s = await appWindow.scaleFactor()
      const pos = await appWindow.outerPosition()
      const size = await appWindow.outerSize()
      x = Math.round(pos.x / s + (size.width / s - windowWidth) / 2)
      y = Math.round(pos.y / s + (size.height / s - windowHeight) / 2)
    }

    const webview = new WebviewWindow(label, {
      url,
      title: params.title || '执行日志',
      width: windowWidth,
      height: windowHeight,
      x,
      y,
      resizable: true,
      decorations: false,
      transparent: false,
    })

    return webview
  } catch (e) {
    console.error('Failed to open log window:', e)
    return null
  }
}
