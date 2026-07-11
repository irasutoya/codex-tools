export type Page =
  "dashboard" | "providers" | "sessions" | "repair" | "settings"

export type Protocol = "responses" | "chat_completions"

export type Provider = {
  id: string
  name: string
  protocol: Protocol
  baseUrl: string
  defaultModel: string
  models: string[]
  headers: Record<string, string>
  timeoutSecs: number
  contextWindow?: number
  autoCompactThreshold?: number
  enabled: boolean
  active: boolean
  activeAccountId?: string
  accountCount: number
}

export type Account = {
  id: string
  providerId?: string
  name: string
  authKind: "api_key" | "official_oauth"
  apiKey?: string
  authJson?: unknown
  headers: Record<string, string>
  active: boolean
  email?: string
  createdAt: number
  updatedAt: number
}

export type Dashboard = {
  providerCount: number
  activeProvider?: string
  codexHome: string
  databaseCount: number
  sessionCount: number
  databaseHealth: string
}

export type Session = {
  id: string
  title: string
  provider: string
  cwd: string
  archived: boolean
  updatedAt: number
  sourceDb: string
}

export type RepairScan = {
  operationId: string
  databases: {
    path: string
    health: string
    knownSchema: boolean
    threadCount: number
  }[]
  rolloutFiles: number
  canRepair: boolean
  warnings: string[]
}
