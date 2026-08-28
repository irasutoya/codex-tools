import { invoke } from "@tauri-apps/api/core"

import type {
  AccountQuota,
  CodexAppSetting,
  CredentialMaintenanceResult,
  ConfigPatchPreview,
  Dashboard,
  DeviceAuthPollResult,
  DeviceAuthorization,
  ModelUnlockResult,
  ModelUnlockStatus,
  OfficialAccountView,
  ProxyImportResult,
  OfficialPricingCatalog,
  PageResult,
  PricingRule,
  PricingScope,
  Provider,
  ProviderOverview,
  ProviderSaveInput,
  ProviderTestResult,
  QuotaRefreshResult,
  QuotaEstimateResult,
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
  connections_save_provider: CommandSpec<
    { provider: ProviderSaveInput },
    Provider
  >
  connections_delete_provider: CommandSpec<{ id: string }, void>
  connections_import_cookie: CommandSpec<
    { name?: string; accountId?: string; content: string },
    ProxyImportResult
  >
  connections_login_start: CommandSpec<undefined, DeviceAuthorization>
  connections_login_poll: CommandSpec<
    { operationId: string },
    DeviceAuthPollResult
  >
  connections_activate_account: CommandSpec<{ id: string }, RepairResult>
  connections_refresh_login: CommandSpec<
    { id: string },
    CredentialMaintenanceResult
  >
  connections_update_account_remark: CommandSpec<
    { id: string; remark: string },
    OfficialAccountView
  >
  connections_update_account_remarks: CommandSpec<
    { updates: Array<{ id: string; remark: string }> },
    OfficialAccountView[]
  >
  connections_delete_account: CommandSpec<{ id: string }, void>
  connections_delete_accounts: CommandSpec<{ ids: string[] }, void>
  connections_open_login_page: CommandSpec<undefined, void>
  connections_test_provider: CommandSpec<{ id: string }, ProviderTestResult>
  connections_list_models: CommandSpec<{ id: string }, string[]>
  connections_refresh_models: CommandSpec<undefined, string[]>
  connections_refresh_quota: CommandSpec<{ accountId: string }, AccountQuota>
  connections_estimate_quota: CommandSpec<
    { accountId: string },
    QuotaEstimateResult
  >
  connections_refresh_all_quota: CommandSpec<undefined, QuotaRefreshResult[]>
  connections_activate: CommandSpec<{ id: string }, RepairResult>
  connections_activate_official: CommandSpec<undefined, RepairResult>
  settings_preview_activation: CommandSpec<undefined, ConfigPatchPreview>
  settings_apply_activation: CommandSpec<{ operationId: string }, void>
  settings_model_unlock_status: CommandSpec<undefined, ModelUnlockStatus>
  settings_unlock_models: CommandSpec<undefined, ModelUnlockResult>
  settings_launch_codex_debug: CommandSpec<undefined, ModelUnlockResult>
  sessions_scan: CommandSpec<undefined, RepairScan>
  sessions_repair: CommandSpec<{ targetProvider: string }, RepairResult>
  sessions_list: CommandSpec<
    {
      query?: string
      page?: number
      pageSize?: number
      refresh?: boolean
      status?: "active" | "archived"
    },
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
let mockModulePromise: Promise<typeof import("@/lib/ipc-mock")> | undefined

export async function call<K extends Command>(
  command: K,
  ...input: CallArguments<K>
): Promise<CommandResult<K>> {
  const args = input[0]
  if (mockMode) {
    mockModulePromise ??= import("@/lib/ipc-mock")
    const { mockCall } = await mockModulePromise
    return mockCall(command, args) as Promise<CommandResult<K>>
  }
  try {
    return await invoke<CommandResult<K>>(command, args)
  } catch (error) {
    if (error instanceof Error) throw error
    throw new Error(typeof error === "string" ? error : String(error), {
      cause: error,
    })
  }
}
