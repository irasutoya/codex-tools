import { formatDate, quotaWindow } from "@/lib/format"
import { call } from "@/lib/ipc"
import type {
  OfficialAccountView,
  Provider,
  ProviderSaveInput,
  RepairResult,
} from "@/types"

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

/** 将编辑器状态转换为保存 DTO；null 明确表示“全部有效模型”。 */
export function providerSaveInputOf(provider: Provider): ProviderSaveInput {
  return {
    id: provider.id,
    name: provider.name,
    baseUrl: provider.baseUrl,
    headers: { ...provider.headers },
    timeoutSecs: provider.timeoutSecs,
    enabled: provider.enabled,
    apiType: provider.apiType,
    selectedModels: provider.selectedModels ?? null,
    customModels: provider.customModels,
    apiKey: provider.apiKey,
  }
}

/** 有效模型 = /models 同步模型 ∪ 自定义模型（保序去重）。 */
export function effectiveModelsOf(provider: Provider): string[] {
  const available = provider.availableModels ?? []
  const custom = provider.customModels ?? []
  return [...available, ...custom.filter((model) => !available.includes(model))]
}

/** 有效模型数 = /models 同步模型 ∪ 自定义模型（去重）。 */
export function effectiveModelCount(provider: Provider) {
  return effectiveModelsOf(provider).length
}

/** 将后端序列化的 null 统一转为 undefined，保持"未设置=全选"语义。 */
function selectedModelsOf(provider: Provider): string[] | undefined {
  return provider.selectedModels ?? undefined
}

/** 全选 = 未设置 selectedModels（默认写入全部），或所有有效模型均被选中。 */
export function allModelsSelected(provider: Provider): boolean {
  const selected = selectedModelsOf(provider)
  if (selected === undefined) return true
  return effectiveModelsOf(provider).every((model) => selected.includes(model))
}

/** 无选中 = 存在有效模型但 selectedModels 为空数组（此时禁止保存）。 */
export function noModelsSelected(provider: Provider): boolean {
  const selected = selectedModelsOf(provider)
  return effectiveModelsOf(provider).length > 0 && selected?.length === 0
}

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
    return `剩余 ${remainingPercent.toFixed(1)}%`
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
  const resetText = quota?.resetAt
    ? ` · 重置 ${formatDate(quota.resetAt, true)}`
    : ""
  return `${accountPlanText(account)} · ${quotaText}${resetText} · ${account.email || account.name || "OpenAI 账号"}`
}

export function accountPlanText(account: OfficialAccountView) {
  const planType = account.quota.planType?.trim()
  if (!planType) return "OpenAI"
  switch (planType.toLowerCase()) {
    case "plus":
      return "Plus"
    case "pro":
      return "Pro"
    case "pro_5x":
    case "pro-5x":
      return "Pro 5x"
    case "pro_20x":
    case "pro-20x":
      return "Pro 20x"
    default:
      return planType
        .split(/[_-]+/)
        .filter(Boolean)
        .map((part) => part[0]?.toUpperCase() + part.slice(1).toLowerCase())
        .join(" ")
  }
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

/** 在有效模型中切换某个模型的选中状态，返回新的 selectedModels。 */
export function toggleModelSelected(
  provider: Provider,
  model: string,
  checked: boolean
): string[] | undefined {
  const eff = effectiveModelsOf(provider)
  const current = selectedModelsOf(provider) ?? eff
  const next = checked
    ? [...new Set([...current, model])]
    : current.filter((value) => value !== model)
  return next.length === eff.length ? undefined : next
}

/** 添加自定义模型并保持“默认写入”语义（合并 customModels + selectedModels）。 */
export function addCustomModelTo(provider: Provider, model: string): Provider {
  const prevEffective = effectiveModelsOf(provider)
  const nextCustom = [...(provider.customModels ?? []), model]
  let nextSelected = selectedModelsOf(provider)
  // 新增的自定义模型默认写入 Codex（与“默认全选”一致）。
  if (nextSelected !== undefined) {
    const candidate = [...nextSelected, model]
    nextSelected =
      candidate.length === prevEffective.length + 1 ? undefined : candidate
  }
  return {
    ...provider,
    customModels: nextCustom,
    selectedModels: nextSelected,
  }
}

/** 移除自定义模型，并从 selectedModels 中同步剔除，必要时回退全选。 */
export function removeCustomModelFrom(
  provider: Provider,
  model: string
): Provider {
  const prevEffective = effectiveModelsOf(provider)
  const nextCustom = (provider.customModels ?? []).filter(
    (value) => value !== model
  )
  let nextSelected = selectedModelsOf(provider)
  if (nextSelected !== undefined) {
    const filtered = nextSelected.filter((value) => value !== model)
    nextSelected =
      filtered.length === prevEffective.length - 1 ? undefined : filtered
  }
  return {
    ...provider,
    customModels: nextCustom,
    selectedModels: nextSelected,
  }
}
