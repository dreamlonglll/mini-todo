export interface SplitTask {
  title: string
  description: string
  prompt: string
  complexity: 'low' | 'medium' | 'high'
  recommendedAgent: 'claude_code' | 'codex'
  dependencies: number[]
}

export interface SplitResult {
  tasks: SplitTask[]
  summary: string
}

export interface ProjectContext {
  name: string
  path: string
  techStack: Array<{ category: string; name: string; version?: string }>
  directoryTree: string
  documentation: Array<{ filename: string; content: string }>
  recentCommits: Array<{ hash: string; message: string; date: string }>
  activeBranch: string
}

export const COMPLEXITY_MAP: Record<string, { label: string; color: string }> = {
  low: { label: '低', color: '#10B981' },
  medium: { label: '中', color: '#F59E0B' },
  high: { label: '高', color: '#EF4444' },
}

export const AGENT_LABELS: Record<string, string> = {
  claude_code: 'Claude Code',
  codex: 'Codex',
}
