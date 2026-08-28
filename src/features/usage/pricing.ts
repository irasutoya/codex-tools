import type { BillingMode, PricingRule, Provider } from "@/types"

const GLOBAL_SOURCE_VALUE = "__all_third_party_apis__"
const PROVIDER_SOURCE_PREFIX = "__provider__:"

export type PricingSource =
  { kind: "global" } | { kind: "provider"; providerId: string }

export type PricingSourceOption = {
  label: string
  value: string
}

export function pricingSourceFromValue(
  value: string
): PricingSource | undefined {
  if (value === GLOBAL_SOURCE_VALUE) return { kind: "global" }
  const providerId = value.startsWith(PROVIDER_SOURCE_PREFIX)
    ? value.slice(PROVIDER_SOURCE_PREFIX.length).trim()
    : value.trim()
  return providerId ? { kind: "provider", providerId } : undefined
}

export function pricingSourceValue(source: PricingSource) {
  return source.kind === "global"
    ? GLOBAL_SOURCE_VALUE
    : `${PROVIDER_SOURCE_PREFIX}${source.providerId}`
}

export function pricingSourceValueForRule(rule: PricingRule) {
  if (rule.scopeKind === "global_model") {
    return pricingSourceValue({ kind: "global" })
  }
  if (
    (rule.scopeKind === "provider_model" ||
      rule.scopeKind === "provider_default") &&
    rule.providerId
  ) {
    return pricingSourceValue({
      kind: "provider",
      providerId: rule.providerId,
    })
  }
  return ""
}

export function pricingSourceOptions(
  providers: Provider[]
): PricingSourceOption[] {
  return [
    {
      value: pricingSourceValue({ kind: "global" }),
      label: "所有第三方 API（通用规则）",
    },
    ...providers.map((provider) => ({
      value: pricingSourceValue({
        kind: "provider",
        providerId: provider.id,
      }),
      label: provider.enabled ? provider.name : `${provider.name}（已停用）`,
    })),
  ]
}

export function pricingScopeForSource(source?: PricingSource) {
  if (!source) return undefined
  if (source.kind === "global") {
    return {
      scopeKind: "global_model" as const,
      providerId: undefined,
      accountId: undefined,
    }
  }
  return {
    scopeKind: "provider_model" as const,
    providerId: source.providerId,
    accountId: undefined,
  }
}

export function findEquivalentPricingRule(
  rules: PricingRule[],
  candidate: Pick<
    PricingRule,
    "scopeKind" | "providerId" | "accountId" | "modelPattern" | "matchKind"
  >
) {
  return rules.find(
    (rule) =>
      rule.scopeKind === candidate.scopeKind &&
      rule.providerId === candidate.providerId &&
      rule.accountId === candidate.accountId &&
      rule.modelPattern === candidate.modelPattern &&
      rule.matchKind === candidate.matchKind
  )
}

export function pricingSourceLabel(rule: PricingRule, providers: Provider[]) {
  if (rule.scopeKind === "global_model") return "所有第三方 API"
  if (rule.scopeKind === "provider_model") {
    const provider = providers.find(
      (candidate) => candidate.id === rule.providerId
    )
    if (!provider) return rule.providerId || "未知第三方 API"
    return provider.enabled ? provider.name : `${provider.name}（已停用）`
  }
  if (rule.scopeKind === "account_model") return rule.accountId || "账号"
  return rule.providerId || "第三方 API 默认规则"
}

export function pricingSummary(rule: PricingRule) {
  if (rule.billingMode === "subscription") {
    return "旧版订阅规则（将按不计价迁移）"
  }
  if (rule.billingMode === "unpriced") return "不计价"
  const price = (value?: string) => (value ? `$${value}` : "未设置")
  return `输入 ${price(rule.inputUsdPerMillion)} · 缓存读取 ${price(rule.cachedReadUsdPerMillion)} · 缓存写入 ${price(rule.cacheWriteUsdPerMillion)} · 输出 ${price(rule.outputUsdPerMillion)} / 1M`
}

export function billingModeLabel(mode: BillingMode) {
  if (mode === "token") return "按 Token"
  if (mode === "unpriced") return "不计价"
  return "旧版订阅规则"
}
