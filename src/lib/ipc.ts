import { invoke } from "@tauri-apps/api/core"

import type {
  Account,
  AccountQuota,
  ConfigPatchPreview,
  Dashboard,
  DeviceAuthorization,
  DeviceAuthPollResult,
  OfficialAccountView,
  PageResult,
  Provider,
  ProviderOverview,
  ProviderTestResult,
  QuotaRefreshResult,
  RepairResult,
  RepairScan,
  Session,
  SettingsOverview,
} from "@/types"

type CommandSpec<Args, Result> = {
  args: Args
  result: Result
}

type CommandMap = {
  get_dashboard: CommandSpec<undefined, Dashboard>
  get_settings_overview: CommandSpec<undefined, SettingsOverview>
  get_provider_overview: CommandSpec<undefined, ProviderOverview>
  save_provider: CommandSpec<{ provider: Provider }, Provider>
  delete_provider: CommandSpec<{ id: string }, void>
  save_provider_account: CommandSpec<{ account: Account }, Account>
  delete_provider_account: CommandSpec<{ id: string }, void>
  import_proxy_account: CommandSpec<
    { name?: string; accountId?: string; content: string },
    OfficialAccountView
  >
  start_openai_device_auth: CommandSpec<undefined, DeviceAuthorization>
  poll_openai_device_auth: CommandSpec<
    { operationId: string },
    DeviceAuthPollResult
  >
  activate_openai_account: CommandSpec<{ id: string }, RepairResult>
  delete_openai_account: CommandSpec<{ id: string }, void>
  open_openai_device_page: CommandSpec<undefined, void>
  test_provider: CommandSpec<
    { id: string; accountId: string },
    ProviderTestResult
  >
  refresh_official_account_quota: CommandSpec<
    { accountId: string },
    AccountQuota
  >
  refresh_all_official_quotas: CommandSpec<undefined, QuotaRefreshResult[]>
  preview_activation: CommandSpec<
    { providerId?: string } | undefined,
    ConfigPatchPreview
  >
  apply_activation: CommandSpec<{ operationId: string }, void>
  activate_provider: CommandSpec<
    { id: string; accountId: string },
    RepairResult
  >
  activate_official: CommandSpec<undefined, RepairResult>
  scan_codex_data: CommandSpec<undefined, RepairScan>
  repair_codex_data: CommandSpec<{ targetProvider: string }, RepairResult>
  list_sessions: CommandSpec<
    {
      query?: string
      page?: number
      pageSize?: number
      refresh?: boolean
    },
    PageResult<Session>
  >
  launch_codex: CommandSpec<undefined, void>
}

type Command = keyof CommandMap
type CommandArgs<K extends Command> = CommandMap[K]["args"]
type CommandResult<K extends Command> = CommandMap[K]["result"]
type CallArguments<K extends Command> =
  undefined extends CommandArgs<K>
    ? [args?: Exclude<CommandArgs<K>, undefined>]
    : [args: CommandArgs<K>]

export async function call<K extends Command>(
  command: K,
  ...input: CallArguments<K>
): Promise<CommandResult<K>> {
  const args = input[0]
  try {
    return await invoke<CommandResult<K>>(command, args)
  } catch (error) {
    if (error instanceof Error) throw error
    throw new Error(typeof error === "string" ? error : String(error), {
      cause: error,
    })
  }
}
