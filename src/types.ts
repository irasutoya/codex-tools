export type Page =
  "dashboard" | "providers" | "routes" | "sessions" | "settings"
export type Protocol = "responses" | "chat_completions" | "anthropic_messages"

export type FetchedModel = {
  id: string
  ownedBy?: string
  [key: string]: unknown
}

export type Provider = {
  id: string
  name: string
  protocol: Protocol
  baseUrl: string
  models: string[]
  modelMetadata: FetchedModel[]
  modelAliases: Record<string, string>
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
  identity: string
  id: string
  title: string
  provider: string
  cwd: string
  archived: boolean
  updatedAt: number
  sourceDb: string
  sourceRollout?: string
  originalProvider: string
  hasUserEvent: boolean
}

export type PageResult<T> = {
  items: T[]
  total: number
  page: number
  pageSize: number
}

export type RouteSettings = {
  enabled: boolean
  listenAddress: string
  port: number
  requestTimeoutMs: number
  requestMaxRetries: number
  streamMaxRetries: number
  maxConcurrentRequests: number
}

export type RouteConsole = {
  settings: RouteSettings
  running: boolean
  baseUrl?: string
  upstreamUrl?: string
  providerName?: string
  accountName?: string
  model?: string
  startedAt?: number
  requestCount: number
  successCount: number
  errorCount: number
  activeRequests: number
  lastLatencyMs?: number
  logs: Array<{
    id: number
    timestamp: number
    method: string
    path: string
    status: number
    latencyMs: number
    message?: string
  }>
  logTotal: number
  logPage: number
  logPageSize: number
}

export type RepairScan = {
  currentProvider: string
  targets: Array<{ id: string; sources: string[]; current: boolean }>
  rolloutFiles: number
  sessionMetaCount: number
  databases: Array<{ path: string; schema: string; threadCount: number }>
  warnings: string[]
}

export type RepairResult = {
  targetProvider: string
  filesScanned: number
  filesModified: number
  filesSkipped: number
  filesFailed: number
  sessionMetaUpdated: number
  rowsUpdated: number
  warnings: string[]
}

export type OfficialAccountView = {
  id: string
  name: string
  accountId: string
  email: string
  expiresAt?: number
  active: boolean
  createdAt: number
  updatedAt: number
}

export type ProviderOverview = {
  providers: Provider[]
  accounts: Account[]
  officialAccounts: OfficialAccountView[]
}

export type DeviceAuthorization = {
  operationId: string
  userCode: string
  verificationUri: string
  expiresAt: number
  intervalSecs: number
}

export type DeviceAuthPollResult =
  | { status: "pending" }
  | { status: "expired" }
  | {
      status: "complete"
      account: OfficialAccountView
      repair: RepairResult
    }

export type ConfigPatchPreview = {
  operationId: string
  targetPath: string
  baseHash: string
  rendered: string
  changes: string[]
  compatibilityTokenMasked: string
}

export type ConfigInspection = {
  path: string
  valid: boolean
  activeProvider?: string
  managedProviderPresent: boolean
  modelCatalogPath: string
  warnings: string[]
}

export type SettingsOverview = {
  inspection: ConfigInspection
  diagnostics: Record<string, unknown>
}

export const emptyProvider = (): Provider => ({
  id: "",
  name: "",
  protocol: "responses",
  baseUrl: "https://api.openai.com/v1",
  models: [],
  modelMetadata: [],
  modelAliases: {},
  headers: {},
  timeoutSecs: 30,
  enabled: true,
  active: false,
  accountCount: 0,
})

export const emptyAccount = (providerId: string): Account => ({
  id: "",
  providerId,
  name: "默认账号",
  authKind: "api_key",
  apiKey: "",
  headers: {},
  active: false,
  createdAt: 0,
  updatedAt: 0,
})
