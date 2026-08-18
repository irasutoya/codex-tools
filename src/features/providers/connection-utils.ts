import { formatDate, quotaWindow } from "@/lib/format"
import { call } from "@/lib/ipc"
import type { OfficialAccountView, Provider, RepairResult } from "@/types"

export type ConnectionKind = "account" | "provider"

export const emptyProvider = (): Provider => ({
  id: "",
  name: "",
  baseUrl: "https://api.openai.com/v1",
  headers: {},
  timeoutSecs: 30,
  enabled: true,
  active: false,
  apiType: "responses",
  apiKey: "",
  hasApiKey: false,
  createdAt: 0,
  updatedAt: 0,
})

export function accountIsExpired(account: OfficialAccountView) {
  return (
    account.expiresAt != null &&
    account.expiresAt <= Math.floor(Date.now() / 1000)
  )
}

export function quotaStatusText(
  account: OfficialAccountView,
  remainingPercent?: number
) {
  if (remainingPercent !== undefined) {
    const refreshed = account.quota.fetchedAt
      ? ` · ${formatDate(account.quota.fetchedAt, true)}`
      : ""
    return `剩余 ${remainingPercent.toFixed(1)}%${refreshed}`
  }
  switch (account.quota.status) {
    case "never":
      return "尚未刷新"
    case "unauthorized":
      return "登录已失效，请刷新登录"
    case "rate_limited":
      return "请求频繁，请稍后重试"
    case "unsupported":
      return "当前账号暂不支持额度查询"
    case "error":
      return account.quota.error || "额度刷新失败"
    default:
      return "暂无额度数据"
  }
}

export function accountDescription(account: OfficialAccountView) {
  const quota = quotaWindow(account.quota)
  const quotaText = quotaStatusText(account, quota?.remainingPercent)
  return `${quotaText} · ${account.email || account.name || "OpenAI 账号"}`
}

export type FallbackCandidate = {
  id: string
  kind: ConnectionKind
}

export function repairWarning(result: RepairResult) {
  const details: string[] = []
  if (result.filesFailed > 0) {
    details.push(`${result.filesFailed} 个会话文件修复失败`)
  }
  if (result.warnings.length > 0) {
    details.push(result.warnings.slice(0, 2).join("；"))
  }
  return details.length > 0 ? `连接已切换，但${details.join("；")}` : undefined
}

export function buildFallbackCandidates(
  accounts: OfficialAccountView[],
  providers: Provider[],
  excludedAccountIds: ReadonlySet<string> = new Set(),
  excludedProviderId?: string
): FallbackCandidate[] {
  const remainingAccounts = accounts.filter(
    (account) => !excludedAccountIds.has(account.id)
  )
  const healthyAccounts = remainingAccounts.filter(
    (account) =>
      account.quota.status !== "unauthorized" && !accountIsExpired(account)
  )
  const healthyIds = new Set(healthyAccounts.map((account) => account.id))
  const otherAccounts = remainingAccounts.filter(
    (account) => !healthyIds.has(account.id)
  )
  const enabledProviders = providers.filter(
    (provider) =>
      provider.id !== excludedProviderId &&
      provider.enabled &&
      provider.hasApiKey
  )

  return [
    ...healthyAccounts.map((account): FallbackCandidate => ({
      kind: "account",
      id: account.id,
    })),
    ...enabledProviders.map((provider): FallbackCandidate => ({
      kind: "provider",
      id: provider.id,
    })),
    ...otherAccounts.map((account): FallbackCandidate => ({
      kind: "account",
      id: account.id,
    })),
  ]
}

export async function switchActiveConnection(
  candidates: FallbackCandidate[]
): Promise<{ switchedId?: string; repair?: RepairResult; error?: unknown }> {
  let lastError: unknown
  for (const candidate of candidates) {
    try {
      const repair = await (candidate.kind === "account"
        ? call("connections_activate_account", { id: candidate.id })
        : call("connections_activate", { id: candidate.id }))
      return { switchedId: candidate.id, repair }
    } catch (reason) {
      lastError = reason
    }
  }
  return { error: lastError }
}
