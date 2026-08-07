export type Page = "dashboard" | "providers" | "usage" | "sessions" | "settings"

export type PageProps = {
  active: boolean
}

export type Provider = {
  id: string
  name: string
  baseUrl: string
  headers: Record<string, string>
  timeoutSecs: number
  enabled: boolean
  active: boolean
  model?: string
  /** 服务接入方式：直连 Responses API，或经本机转换代理接入 Chat Completions API */
  apiType: "responses" | "chat"
  /** 从服务 /models 接口读取的模型上下文窗口（token），写入模型目录时优先使用 */
  modelContextWindows?: Record<string, number>
  /** 服务 /models 接口返回的可用模型列表（保存服务时静默获取） */
  availableModels?: string[]
  /** models.dev（models.json）匹配的模型元数据（slug → 元数据） */
  modelsDevMeta?: Record<string, ProviderModelsDevMeta>
  apiKey?: string
  hasApiKey?: boolean
  createdAt: number
  updatedAt: number
}

export type Dashboard = {
  providerCount: number
  activeProvider?: string
  activeKind: "none" | "provider" | "official"
  activeAccountId?: string
  activeAccount?: string
  activeModel?: string
  activeQuota?: AccountQuota
  codexHome: string
  databaseCount: number
  sessionCount: number
  databaseHealth: string
  todayUsage: TokenBreakdown
  todayRequests: number
  todayEstimatedCostMicrousd: number
  todaySubscriptionTokens: number
  todayUnpricedTokens: number
  todayPartialTokens: number
  todayUnattributedTokens: number
}

export type UsageSourceKind = "official" | "provider" | "unattributed"

export type UsageGroupBy = "model" | "account"

export type CostStatus =
  | "estimated"
  | "subscription"
  | "unpriced"
  | "partial"
  | "unattributed"
  | "zero"

export type UsageRange = {
  startAtMs: number
  endAtMs: number
}

export type UsageQuery = {
  range: UsageRange
  groupBy: UsageGroupBy
}

export type TokenBreakdown = {
  inputTokens: number
  cachedInputTokens: number
  cacheWriteInputTokens: number
  outputTokens: number
  reasoningOutputTokens: number
  totalTokens: number
}

export type UsageRow = {
  key: string
  model: string
  sourceKind: UsageSourceKind
  providerId?: string
  accountId?: string
  sourceName: string
  tokens: TokenBreakdown
  requests: number
  estimatedCostMicrousd?: number
  costStatus: CostStatus
  pricingRuleName?: string
  pricingRuleVersion?: number
}

export type UsageOverview = {
  range: UsageRange
  totals: {
    tokens: TokenBreakdown
    requests: number
    estimatedCostMicrousd: number
    subscriptionTokens: number
    unpricedTokens: number
    partialTokens: number
    unattributedTokens: number
  }
  rows: UsageRow[]
  lastRefreshedAtMs?: number
  collectionStartedAtMs?: number
  collectionStartedVersion?: string
  warnings: Array<{ path?: string; message: string }>
  /** 与 totals 同一趟查询产出的按日趋势点，避免对同一范围重复全量扫描。 */
  trendPoints: UsageTrendPoint[]
}

export type UsageTrend = {
  range: UsageRange
  points: UsageTrendPoint[]
}

export type UsageTrendPoint = {
  dayStartMs: number
  tokens: TokenBreakdown
  requests: number
  estimatedCostMicrousd: number
  unpricedTokens: number
  partialTokens: number
  unattributedTokens: number
}

export type OfficialPricingCatalog = {
  status: "waiting" | "cached"
  sourceUrl: string
  version?: number
  contentSha256?: string
  fetchedAtMs?: number
  etag?: string
  modelCount: number
  models: string[]
  rates: OfficialModelRate[]
}

export type OfficialModelRate = {
  model: string
  longContextThreshold?: number
  short: TokenRates
  long?: TokenRates
}

export type TokenRates = {
  input?: number
  cachedInput?: number
  cacheWrite?: number
  output?: number
}

export type UsageShareAccount = {
  key: string
  displayName: string
  maskedName: string
  sourceKind: UsageSourceKind
  totalTokens: number
  estimatedCostMicrousd: number
  unpricedTokens: number
  partialTokens: number
  requests: number
  costStatus: CostStatus
  models: UsageShareAccountModel[]
}

export type UsageShareAccountModel = {
  key: string
  model: string
  totalTokens: number
  estimatedCostMicrousd: number
  unpricedTokens: number
  partialTokens: number
  requests: number
  costStatus: CostStatus
}

export type UsageShareData = {
  dateLabel: string
  timezone: string
  totalTokens: number
  estimatedCostMicrousd: number
  unpricedTokens: number
  partialTokens: number
  requests: number
  accounts: UsageShareAccount[]
}

export type RepriceResult = {
  eventsRepriced: number
  estimatedCostMicrousd: number
  unpricedEvents: number
}

export type PricingScopeKind =
  "account_model" | "provider_model" | "global_model" | "provider_default"

export type PricingMatchKind = "exact" | "prefix"

export type BillingMode = "token" | "subscription" | "unpriced"

export type PricingScope = {
  scopeKind: PricingScopeKind
  providerId?: string
  accountId?: string
}

export type PricingRule = {
  id: string
  version: number
  active: boolean
  scopeKind: PricingScopeKind
  providerId?: string
  accountId?: string
  modelPattern: string
  matchKind: PricingMatchKind
  billingMode: BillingMode
  inputUsdPerMillion?: string
  cachedReadUsdPerMillion?: string
  cacheWriteUsdPerMillion?: string
  outputUsdPerMillion?: string
  requestFeeUsd?: string
  cacheWriteIncludedInInput: boolean
  effectiveFromMs: number
  createdAtMs: number
  updatedAtMs: number
}

export type QuotaStatus =
  | "never"
  | "success"
  | "unsupported"
  | "unauthorized"
  | "rate_limited"
  | "error"

export type QuotaWindow = {
  usedPercent: number
  remainingPercent: number
  windowSeconds?: number
  resetAt?: number
}

export type QuotaData = {
  kind: "windowed"
  primary?: QuotaWindow
  secondary?: QuotaWindow
}

export type AccountQuota = {
  status: QuotaStatus
  data?: QuotaData
  fetchedAt?: number
  lastAttemptAt?: number
  error?: string
}

export type QuotaRefreshResult = {
  accountId: string
  quota: AccountQuota
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

export type RepairTarget = {
  id: string
  sources: string[]
  current: boolean
  count: number
}

export type RepairScan = {
  currentProvider: string
  targets: RepairTarget[]
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
  source: "open_ai_oauth" | "proxy_import"
  expiresAt?: number
  quota: AccountQuota
  active: boolean
  createdAt: number
  updatedAt: number
}

export type ProviderOverview = {
  providers: Provider[]
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

export type ProviderTestResult = {
  ok: boolean
  status: number
  endpoint: string
  message: string
  suggestV1: boolean
}

export type ConfigPatchPreview = {
  operationId: string
  targetPath: string
  baseHash: string
  rendered: string
  changes: string[]
  apiKeyMasked: string
}

export type ConfigInspection = {
  path: string
  valid: boolean
  activeProvider?: string
  managedProviderPresent: boolean
  warnings: string[]
}

export type SettingsOverview = {
  inspection: ConfigInspection
  diagnostics: Record<string, unknown>
  canPreviewCustom: boolean
}

export type CodexAppSetting = {
  /** 手动配置的 Codex 应用路径（.app 目录或可执行文件） */
  configured?: string
  /** 实际检测到的 Codex 应用路径 */
  detected?: string
}

export type ProviderModelsDevMeta = {
  name?: string
  contextWindow?: number
  description?: string
}

export type ModelUnlockStatus = {
  appFound: boolean
  appRunning: boolean
  debugPort?: number
  injected: boolean
  modelCount: number
  models: string[]
  warning?: string
}

export type ModelUnlockResult = {
  port: number
  injected: boolean
  modelCount: number
  message: string
}

export const emptyProvider = (): Provider => ({
  id: "",
  name: "",
  baseUrl: "https://api.openai.com/v1",
  headers: {},
  timeoutSecs: 30,
  enabled: true,
  active: false,
  model: "",
  apiType: "responses",
  apiKey: "",
  hasApiKey: false,
  createdAt: 0,
  updatedAt: 0,
})
