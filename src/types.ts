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
  activeKind: "none" | "provider" | "official"
  activeAccountId?: string
  activeAccount?: string
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

export type UsageRefreshResult = {
  filesScanned: number
  eventsAdded: number
  eventsSkipped: number
  partialLines: number
  warnings: Array<{ path?: string; message: string }>
  lastRefreshedAtMs: number
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
  source: "open_ai_oauth" | "proxy_import"
  expiresAt?: number
  quota: AccountQuota
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

export const emptyProvider = (): Provider => ({
  id: "",
  name: "",
  baseUrl: "https://api.openai.com/v1",
  headers: {},
  timeoutSecs: 30,
  enabled: true,
  active: false,
  accountCount: 0,
})

export const emptyAccount = (providerId: string): Account => ({
  id: "",
  providerId,
  name: "默认密钥",
  authKind: "api_key",
  apiKey: "",
  headers: {},
  active: false,
  createdAt: 0,
  updatedAt: 0,
})
