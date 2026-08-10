import { invoke } from "@tauri-apps/api/core"

import type {
  AccountQuota,
  CodexAppSetting,
  ConfigPatchPreview,
  Dashboard,
  DeviceAuthPollResult,
  DeviceAuthorization,
  ModelUnlockResult,
  ModelUnlockStatus,
  OfficialAccountView,
  OfficialPricingCatalog,
  PageResult,
  PricingRule,
  PricingScope,
  Provider,
  ProviderOverview,
  ProviderTestResult,
  QuotaRefreshResult,
  RepairResult,
  RepairScan,
  RepriceResult,
  Session,
  SettingsOverview,
  SupportDiagnostics,
  UsageOverview,
  UsageQuery,
  UsageRange,
  UsageTrend,
} from "@/types"

type CommandSpec<Args, Result> = { args: Args; result: Result }

type CommandMap = {
  dashboard_get: CommandSpec<undefined, Dashboard>
  settings_get_overview: CommandSpec<undefined, SettingsOverview>
  settings_get_diagnostics: CommandSpec<undefined, SupportDiagnostics>
  settings_get_codex_app: CommandSpec<undefined, CodexAppSetting>
  settings_save_codex_app_path: CommandSpec<{ path: string | null }, void>
  connections_list: CommandSpec<undefined, ProviderOverview>
  connections_save_provider: CommandSpec<{ provider: Provider }, Provider>
  connections_delete_provider: CommandSpec<{ id: string }, void>
  connections_import_cookie: CommandSpec<
    { name?: string; accountId?: string; content: string },
    OfficialAccountView
  >
  connections_login_start: CommandSpec<undefined, DeviceAuthorization>
  connections_login_poll: CommandSpec<
    { operationId: string },
    DeviceAuthPollResult
  >
  connections_activate_account: CommandSpec<{ id: string }, RepairResult>
  connections_delete_account: CommandSpec<{ id: string }, void>
  connections_open_login_page: CommandSpec<undefined, void>
  connections_test_provider: CommandSpec<{ id: string }, ProviderTestResult>
  connections_list_models: CommandSpec<{ id: string }, string[]>
  connections_refresh_models: CommandSpec<undefined, string[]>
  connections_refresh_quota: CommandSpec<{ accountId: string }, AccountQuota>
  connections_refresh_all_quota: CommandSpec<undefined, QuotaRefreshResult[]>
  connections_activate: CommandSpec<{ id: string }, RepairResult>
  connections_activate_official: CommandSpec<undefined, RepairResult>
  settings_preview_activation: CommandSpec<
    { providerId?: string } | undefined,
    ConfigPatchPreview
  >
  settings_apply_activation: CommandSpec<{ operationId: string }, void>
  settings_model_unlock_status: CommandSpec<undefined, ModelUnlockStatus>
  settings_unlock_models: CommandSpec<undefined, ModelUnlockResult>
  settings_launch_codex_debug: CommandSpec<undefined, ModelUnlockResult>
  sessions_scan: CommandSpec<undefined, RepairScan>
  sessions_repair: CommandSpec<{ targetProvider: string }, RepairResult>
  sessions_list: CommandSpec<
    { query?: string; page?: number; pageSize?: number; refresh?: boolean },
    PageResult<Session>
  >
  dashboard_launch: CommandSpec<undefined, ModelUnlockResult>
  usage_get_overview: CommandSpec<{ query: UsageQuery }, UsageOverview>
  usage_refresh: CommandSpec<{ query: UsageQuery }, UsageOverview>
  usage_get_trend: CommandSpec<{ range: UsageRange }, UsageTrend>
  usage_get_official_pricing: CommandSpec<undefined, OfficialPricingCatalog>
  usage_refresh_official_pricing: CommandSpec<undefined, OfficialPricingCatalog>
  usage_list_pricing_rules: CommandSpec<{ scope?: PricingScope }, PricingRule[]>
  usage_save_pricing_rule: CommandSpec<{ input: PricingRule }, PricingRule>
  usage_delete_pricing_rule: CommandSpec<{ id: string }, void>
  usage_reprice: CommandSpec<{ range: UsageRange }, RepriceResult>
}

export type Command = keyof CommandMap
type CommandArgs<K extends Command> = CommandMap[K]["args"]
type CommandResult<K extends Command> = CommandMap[K]["result"]
type CallArguments<K extends Command> =
  undefined extends CommandArgs<K>
    ? [args?: Exclude<CommandArgs<K>, undefined>]
    : [args: CommandArgs<K>]

const mockMode =
  import.meta.env.DEV &&
  typeof window !== "undefined" &&
  new URLSearchParams(window.location.search).has("mock")

const mockEmptyConnections =
  mockMode &&
  new URLSearchParams(window.location.search).has("empty-connections")

export async function call<K extends Command>(
  command: K,
  ...input: CallArguments<K>
): Promise<CommandResult<K>> {
  const args = input[0]
  if (mockMode) return mockCall(command, args) as Promise<CommandResult<K>>
  try {
    return await invoke<CommandResult<K>>(command, args)
  } catch (error) {
    if (error instanceof Error) throw error
    throw new Error(typeof error === "string" ? error : String(error), {
      cause: error,
    })
  }
}

const now = Date.now()
const day = 86_400_000
const mockQuota: AccountQuota = {
  status: "success",
  fetchedAt: Math.floor(now / 1000),
  data: {
    kind: "windowed",
    primary: {
      usedPercent: 23.9,
      remainingPercent: 76.1,
      resetAt: Math.floor((now + 6 * day) / 1000),
    },
  },
}

const mockProviders: Provider[] = [
  {
    id: "provider-team",
    name: "团队网关",
    baseUrl: "https://api.team.example/v1",
    headers: {},
    timeoutSecs: 30,
    enabled: true,
    active: false,
    model: "deepseek-v4-pro",
    apiType: "responses",
    availableModels: ["deepseek-v4-pro", "gpt-5.6"],
    hasApiKey: true,
    createdAt: now - 30 * day,
    updatedAt: now - day,
  },
  {
    id: "provider-lab",
    name: "开发环境",
    baseUrl: "https://api.lab.example/v1",
    headers: {},
    timeoutSecs: 30,
    enabled: true,
    active: false,
    model: "qwen3-coder",
    apiType: "chat",
    availableModels: ["qwen3-coder"],
    hasApiKey: true,
    createdAt: now - 12 * day,
    updatedAt: now - 2 * day,
  },
]

const mockAccounts: OfficialAccountView[] = [
  {
    id: "account-work",
    name: "工作账号",
    accountId: "workspace",
    email: "work@example.com",
    source: "open_ai_oauth",
    quota: mockQuota,
    active: true,
    createdAt: now - 60 * day,
    updatedAt: now,
  },
  {
    id: "account-personal",
    name: "个人账号",
    accountId: "personal",
    email: "me@example.com",
    source: "open_ai_oauth",
    quota: { status: "never" },
    active: false,
    createdAt: now - 50 * day,
    updatedAt: now - 4 * day,
  },
]

const mockTrend = [28_000, 40_000, 31_000, 51_000, 34_000, 25_000, 38_000].map(
  (input, index) => ({
    dayStartMs: now - (6 - index) * day,
    tokens: {
      inputTokens: input,
      cachedInputTokens: 0,
      cacheWriteInputTokens: 0,
      outputTokens: Math.round(input * 0.42),
      reasoningOutputTokens: 0,
      totalTokens: Math.round(input * 1.42),
    },
    requests: 120 + index * 17,
    estimatedCostMicrousd: 310_000 + index * 32_000,
    unpricedTokens: 0,
    partialTokens: 0,
    unattributedTokens: 0,
  })
)

const mockUsage: UsageOverview = {
  range: { startAtMs: now - 7 * day, endAtMs: now + day },
  totals: {
    tokens: {
      inputTokens: 171_234,
      cachedInputTokens: 19_800,
      cacheWriteInputTokens: 4_100,
      outputTokens: 67_487,
      reasoningOutputTokens: 12_200,
      totalTokens: 238_721,
    },
    requests: 1_245,
    estimatedCostMicrousd: 3_423_000,
    subscriptionTokens: 0,
    unpricedTokens: 0,
    partialTokens: 0,
    unattributedTokens: 0,
  },
  rows: [
    {
      key: "gpt-5.6-work",
      model: "gpt-5.6",
      sourceKind: "official",
      accountId: "account-work",
      sourceName: "工作账号",
      tokens: {
        inputTokens: 121_234,
        cachedInputTokens: 19_800,
        cacheWriteInputTokens: 4_100,
        outputTokens: 47_487,
        reasoningOutputTokens: 10_200,
        totalTokens: 172_521,
      },
      requests: 830,
      estimatedCostMicrousd: 2_430_000,
      costStatus: "estimated",
      pricingRuleName: "OpenAI 官方参考价",
    },
    {
      key: "deepseek-team",
      model: "deepseek-v4-pro",
      sourceKind: "provider",
      providerId: "provider-team",
      sourceName: "团队网关",
      tokens: {
        inputTokens: 50_000,
        cachedInputTokens: 0,
        cacheWriteInputTokens: 0,
        outputTokens: 20_000,
        reasoningOutputTokens: 2_000,
        totalTokens: 66_200,
      },
      requests: 415,
      estimatedCostMicrousd: 993_000,
      costStatus: "estimated",
      pricingRuleName: "团队价格",
    },
  ],
  trendPoints: mockTrend,
  warnings: [],
  lastRefreshedAtMs: now,
  collectionStartedAtMs: now - 30 * day,
  collectionStartedVersion: "0.4.0",
}

const mockSessions: Session[] = Array.from({ length: 16 }, (_, index) => ({
  identity: `session-${index}`,
  id: `session-${index}`,
  title: ["重构账号切换流程", "实现用量图表", "排查模型同步问题", "发布检查"][
    index % 4
  ],
  provider: index % 3 === 0 ? "custom" : "openai",
  cwd: index % 2 === 0 ? "/Users/demo/project" : "/Users/demo/codex-tools",
  archived: false,
  updatedAt: now - index * 7_200_000,
  sourceDb: "state.sqlite",
  originalProvider: index % 3 === 0 ? "custom" : "openai",
  hasUserEvent: true,
}))

const mockRepair: RepairResult = {
  targetProvider: "openai",
  filesScanned: 8,
  filesModified: 3,
  filesSkipped: 5,
  filesFailed: 0,
  sessionMetaUpdated: 3,
  rowsUpdated: 3,
  warnings: [],
}

async function mockCall(command: Command, args: unknown): Promise<unknown> {
  await new Promise((resolve) => window.setTimeout(resolve, 120))
  switch (command) {
    case "dashboard_get":
      return {
        providerCount: mockProviders.length,
        activeProvider: "OpenAI · 工作账号",
        activeKind: "official",
        activeAccountId: "account-work",
        activeAccount: "工作账号",
        activeModel: "gpt-5.6",
        activeQuota: mockQuota,
        codexHome: "/Users/demo/.codex",
        databaseCount: 1,
        sessionCount: mockSessions.length,
        databaseHealth: "可以读取",
        todayUsage: mockUsage.totals.tokens,
        todayRequests: mockUsage.totals.requests,
        todayEstimatedCostMicrousd: mockUsage.totals.estimatedCostMicrousd,
        todaySubscriptionTokens: 0,
        todayUnpricedTokens: 0,
        todayPartialTokens: 0,
        todayUnattributedTokens: 0,
      } satisfies Dashboard
    case "connections_list":
      if (mockEmptyConnections) {
        return { providers: [], officialAccounts: [] }
      }
      return { providers: mockProviders, officialAccounts: mockAccounts }
    case "usage_get_overview":
    case "usage_refresh":
      return mockUsage
    case "usage_get_trend":
      return { range: mockUsage.range, points: mockTrend }
    case "sessions_list": {
      const input = (args ?? {}) as {
        query?: string
        page?: number
        pageSize?: number
      }
      const query = input.query?.toLowerCase() ?? ""
      const filtered = mockSessions.filter((session) =>
        `${session.title} ${session.cwd}`.toLowerCase().includes(query)
      )
      const requestedPage = input.page ?? 1
      const pageSize = input.pageSize ?? 8
      const pageCount = Math.max(1, Math.ceil(filtered.length / pageSize))
      const page = Math.min(Math.max(1, requestedPage), pageCount)
      return {
        items: filtered.slice((page - 1) * pageSize, page * pageSize),
        total: filtered.length,
        page,
        pageSize,
      }
    }
    case "sessions_scan":
      return {
        currentProvider: "openai",
        targets: [
          { id: "openai", sources: ["openai"], current: true, count: 12 },
          { id: "custom", sources: ["custom"], current: false, count: 4 },
        ],
        rolloutFiles: 16,
        sessionMetaCount: 16,
        databases: [
          {
            path: "/Users/demo/.codex/state.sqlite",
            schema: "threads",
            threadCount: 16,
          },
        ],
        warnings: [],
      } satisfies RepairScan
    case "sessions_repair":
    case "connections_activate":
    case "connections_activate_account":
    case "connections_activate_official":
      return mockRepair
    case "settings_get_overview":
      return {
        inspection: {
          path: "/Users/demo/.codex/config.toml",
          valid: true,
          activeProvider: "openai",
          managedProviderPresent: true,
          warnings: [],
        },
        diagnostics: {
          dataDirectory: "/Users/demo/Library/Application Support/Codex Tools",
          active: { kind: "official", accountId: "account-work" },
        },
        canPreviewCustom: true,
      } satisfies SettingsOverview
    case "settings_get_diagnostics":
      return {
        schemaVersion: 1,
        generatedAt: new Date(now).toISOString(),
        app: { name: "Codex Tools", version: "0.4.0", buildProfile: "debug" },
        system: { os: "macos", architecture: "aarch64", family: "unix" },
        paths: {
          dataDirectory: "~/Library/Application Support/Codex Tools",
          codexHome: "~/.codex",
          configFile: "~/.codex/config.toml",
        },
        configuration: {
          valid: true,
          activeProvider: "openai",
          managedProviderPresent: true,
          warnings: [],
        },
        connection: {
          activeKind: "official",
          providerCount: 2,
          officialAccountCount: 2,
          activeModel: "gpt-5.6",
        },
        storage: {
          files: [
            { name: "app.json", exists: true, readable: true, sizeBytes: 512 },
            {
              name: "connections.json",
              exists: true,
              readable: true,
              sizeBytes: 2048,
            },
            {
              name: "credentials.json",
              exists: true,
              readable: true,
              sizeBytes: 1024,
            },
          ],
          usageDatabase: {
            exists: true,
            sizeBytes: 65536,
            schemaVersion: 5,
            quickCheck: "ok",
            eventCount: 1245,
            cursorCount: 16,
          },
          sessionDatabaseCount: 1,
          indexedSessionCount: 16,
        },
        network: {
          environmentProxyConfigured: false,
          noProxyConfigured: false,
          systemProxyConfigured: false,
          tlsBackend: "rustls",
        },
        warnings: [],
        privacy: {
          homePathsRedacted: true,
          omitted: [
            "API keys",
            "OAuth and Cookie tokens",
            "custom header values",
            "account identifiers and email addresses",
            "proxy addresses",
          ],
        },
      } satisfies SupportDiagnostics
    case "settings_get_codex_app":
      return { detected: "/Applications/Codex.app" }
    case "settings_model_unlock_status":
      return {
        appFound: true,
        appRunning: true,
        debugPort: 9222,
        injected: true,
        modelCount: 3,
        models: ["gpt-5.6", "deepseek-v4-pro", "qwen3-coder"],
      } satisfies ModelUnlockStatus
    case "settings_preview_activation":
      return {
        operationId: "mock-preview",
        targetPath: "/Users/demo/.codex/config.toml",
        baseHash: "mock",
        rendered: 'model_provider = "custom"\nmodel = "deepseek-v4-pro"',
        changes: ["切换 model_provider", "更新默认模型"],
        apiKeyMasked: "sk-••••••••",
      } satisfies ConfigPatchPreview
    case "connections_test_provider":
      return {
        ok: true,
        status: 200,
        endpoint: "https://api.team.example/v1/models",
        message: "模型列表接口可以访问。",
        suggestV1: false,
      } satisfies ProviderTestResult
    case "connections_list_models":
    case "connections_refresh_models":
      return ["gpt-5.6", "deepseek-v4-pro", "qwen3-coder"]
    case "connections_refresh_quota":
      return mockQuota
    case "connections_refresh_all_quota":
      return mockAccounts.map((account) => ({
        accountId: account.id,
        quota: mockQuota,
      }))
    case "connections_save_provider": {
      const provider = (args as { provider: Provider }).provider
      return {
        ...provider,
        id: provider.id || `provider-${Date.now()}`,
        hasApiKey: true,
      }
    }
    case "connections_import_cookie":
      return mockAccounts[1]
    case "connections_login_start":
      return {
        operationId: "mock-login",
        userCode: "ABCD-EFGH",
        verificationUri: "https://auth.openai.com/codex/device",
        expiresAt: Math.floor((now + 15 * 60_000) / 1000),
        intervalSecs: 5,
      } satisfies DeviceAuthorization
    case "connections_login_poll":
      return { status: "pending" } satisfies DeviceAuthPollResult
    case "usage_get_official_pricing":
    case "usage_refresh_official_pricing":
      return {
        status: "cached",
        sourceUrl: "https://openai.com/api/pricing",
        fetchedAtMs: now,
        modelCount: 1,
        models: ["gpt-5.6"],
        rates: [],
      } satisfies OfficialPricingCatalog
    case "usage_list_pricing_rules":
      return []
    case "usage_save_pricing_rule":
      return (args as { input: PricingRule }).input
    case "usage_reprice":
      return {
        eventsRepriced: 2,
        estimatedCostMicrousd: 3_423_000,
        unpricedEvents: 0,
      }
    case "dashboard_launch":
    case "settings_unlock_models":
    case "settings_launch_codex_debug":
      return {
        port: 9222,
        injected: true,
        modelCount: 3,
        message: "Codex 已启动",
      }
    default:
      return undefined
  }
}
