export type AgentType = 'claude_code' | 'codex' | 'custom'

export type AgentHealthState = 'healthy' | 'outdated' | 'unavailable' | 'no_key' | 'error' | 'disabled'

export interface AgentCapabilities {
  codeGen: number
  codeFix: number
  testReview: number
  speed: number
  costEfficiency: number
}

export interface SandboxConfig {
  enableWorktreeIsolation: boolean
  protectedFiles: string[]
  allowedTools: string[]
  maxFilesChanged: number
  maxLinesChanged: number
  worktreeBaseDir: string
}

export interface AgentConfig {
  id: number
  name: string
  agentType: AgentType
  cliPath: string
  cliVersion: string
  minCliVersion: string
  defaultModel: string
  maxConcurrent: number
  timeoutSeconds: number
  capabilities: string
  envVars: string
  sandboxConfig: string
  enabled: boolean
  hasApiKey: boolean
  createdAt: string
  updatedAt: string
}

export interface CreateAgentRequest {
  name: string
  agentType: AgentType
  cliPath: string
  apiKey?: string
  defaultModel?: string
  maxConcurrent?: number
  timeoutSeconds?: number
  capabilities?: string
  envVars?: string
  sandboxConfig?: string
}

export interface UpdateAgentRequest {
  name?: string
  agentType?: string
  cliPath?: string
  apiKey?: string
  defaultModel?: string
  maxConcurrent?: number
  timeoutSeconds?: number
  capabilities?: string
  envVars?: string
  sandboxConfig?: string
  enabled?: boolean
}

export interface AgentHealthStatus {
  agentId: number
  status: AgentHealthState
  cliFound: boolean
  detectedVersion: string | null
  versionCompatible: boolean
  message: string | null
}

export interface AgentEvent {
  kind: 'Log' | 'Progress' | 'TokenUsage' | 'Completed' | 'Failed'
  content?: string
  level?: string
  message?: string
  inputTokens?: number
  outputTokens?: number
  exitCode?: number
  result?: string
  error?: string
}

export const AGENT_TYPE_INFO: Record<AgentType, { label: string; description: string }> = {
  claude_code: {
    label: 'Claude Code',
    description: '复杂架构设计、多文件重构、深度代码理解',
  },
  codex: {
    label: 'Codex',
    description: '快速代码生成、小范围修改',
  },
  custom: {
    label: '自定义',
    description: '通过 CLI 适配器接入的其他工具',
  },
}

export const DEFAULT_CAPABILITIES: Record<string, AgentCapabilities> = {
  claude_code: { codeGen: 8, codeFix: 8, testReview: 7, speed: 7, costEfficiency: 7 },
  codex: { codeGen: 6, codeFix: 6, testReview: 5, speed: 8, costEfficiency: 8 },
  custom: { codeGen: 5, codeFix: 5, testReview: 5, speed: 5, costEfficiency: 5 },
}

export const DEFAULT_SANDBOX_CONFIG: SandboxConfig = {
  enableWorktreeIsolation: true,
  protectedFiles: ['.env', '.env.local', '.gitignore'],
  allowedTools: ['Edit', 'Write', 'Read', 'Glob', 'Grep'],
  maxFilesChanged: 50,
  maxLinesChanged: 5000,
  worktreeBaseDir: '.agent-worktrees',
}
