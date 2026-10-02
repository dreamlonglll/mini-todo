import { ElMessage } from 'element-plus'

// 提示里展示的错误详情上限：后端偶尔会带回很长的链式错误，整段塞进 toast 不可读
const MAX_DETAIL_LENGTH = 200

/**
 * 把 invoke / 运行时抛出的任意错误转成可读文本
 *
 * Tauri 命令的 `Err(String)` 到前端是字符串，JS 运行时错误是 Error，其余兜底 String()
 */
export function errorMessage(e: unknown): string {
  let text: string
  if (typeof e === 'string') {
    text = e
  } else if (e instanceof Error) {
    text = e.message
  } else if (e && typeof e === 'object' && 'message' in e && typeof e.message === 'string') {
    text = e.message
  } else {
    text = String(e ?? '')
  }
  text = text.trim()
  return text.length > MAX_DETAIL_LENGTH ? `${text.slice(0, MAX_DETAIL_LENGTH)}…` : text
}

/**
 * 统一的失败提示：控制台保留完整错误，界面弹出「操作描述：错误详情」
 *
 * 用于用户触发的操作（保存、删除、同步……）的失败路径；后台静默刷新之类的
 * 非用户操作不要每次都调用，避免刷屏
 */
export function notifyError(e: unknown, msg: string): void {
  console.error(msg, e)
  const detail = errorMessage(e)
  ElMessage.error(detail ? `${msg}：${detail}` : msg)
}
