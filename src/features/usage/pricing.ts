import type { BillingMode, PricingRule } from "@/types"

export function pricingSummary(rule: PricingRule) {
  if (rule.billingMode === "subscription") {
    return `${rule.scopeKind} · 旧版订阅规则（将按不计价迁移）`
  }
  if (rule.billingMode === "unpriced") return `${rule.scopeKind} · 不计价`
  const price = (value?: string) => (value ? `$${value}` : "未设置")
  return `${rule.scopeKind} · 输入 ${price(rule.inputUsdPerMillion)} · 缓存读取 ${price(rule.cachedReadUsdPerMillion)} · 缓存写入 ${price(rule.cacheWriteUsdPerMillion)} · 输出 ${price(rule.outputUsdPerMillion)} / 1M`
}

export function billingModeLabel(mode: BillingMode) {
  if (mode === "token") return "按 Token"
  if (mode === "unpriced") return "不计价"
  return "旧版订阅规则"
}
