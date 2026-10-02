import type { SyncReport } from '@/types'

// 防御：后端字段缺失时按 0 处理，避免 NaN 让"是否有变化"判断静默失效
function count(value: unknown): number {
  return typeof value === 'number' && Number.isFinite(value) ? value : 0
}

/** 本地待办 / 子任务是否被这次同步改动过（决定是否需要重新拉取列表） */
export function hasLocalDataChanges(report: SyncReport): boolean {
  return (
    count(report.todosInserted) +
      count(report.todosUpdated) +
      count(report.todosDeleted) +
      count(report.subtasksInserted) +
      count(report.subtasksUpdated) +
      count(report.subtasksDeleted) >
    0
  )
}

const STATUS_TEXT: Record<SyncReport['status'], string> = {
  no_changes: '已是最新，没有需要同步的更改',
  pulled: '已从云端拉取更新',
  pushed: '已将本地更改推送到云端',
  merged: '已合并本地与云端的更改',
}

/** 同步结果的一句话描述（含非零计数），用于成功提示 */
export function describeSyncReport(report: SyncReport): string {
  const base = STATUS_TEXT[report.status] ?? '同步完成'
  const counters: Array<[unknown, string]> = [
    [report.todosInserted, '新增待办'],
    [report.todosUpdated, '更新待办'],
    [report.todosDeleted, '删除待办'],
    [report.subtasksInserted, '新增子任务'],
    [report.subtasksUpdated, '更新子任务'],
    [report.subtasksDeleted, '删除子任务'],
    [report.imagesDownloaded, '下载图片'],
    [report.imagesUploaded, '上传图片'],
  ]
  const parts = counters.filter(([n]) => count(n) > 0).map(([n, label]) => `${label} ${count(n)}`)
  if (report.settingsApplied) parts.push('已应用云端设置')
  return parts.length > 0 ? `${base}（${parts.join('，')}）` : base
}

/** 有记录因无法解析被跳过时的提醒文案；没有则返回 null */
export function describeSkippedRecords(report: SyncReport): string | null {
  const skipped = count(report.recordsSkipped)
  return skipped > 0 ? `有 ${skipped} 条云端记录无法识别，已跳过（未删除任何本地数据）` : null
}

/** 后端同步互斥锁返回的错误：另一处正在同步，不算失败 */
export function isSyncBusyError(e: unknown): boolean {
  return String(e).includes('同步正在进行中')
}
