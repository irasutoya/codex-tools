import type { AccountQuota, TokenBreakdown, UsageRange } from "@/types"

const compactNumber = new Intl.NumberFormat("zh-CN", {
  notation: "compact",
  maximumFractionDigits: 1,
})

export function formatTokens(value = 0) {
  return compactNumber.format(value)
}

export function formatInteger(value = 0) {
  return new Intl.NumberFormat("zh-CN").format(value)
}

export function formatUsd(microusd = 0) {
  return new Intl.NumberFormat("zh-CN", {
    style: "currency",
    currency: "USD",
    minimumFractionDigits: microusd >= 100_000 ? 2 : 4,
    maximumFractionDigits: 4,
  }).format(microusd / 1_000_000)
}

export function formatDate(value?: number, includeTime = false) {
  if (!value) return "—"
  return new Intl.DateTimeFormat("zh-CN", {
    month: "2-digit",
    day: "2-digit",
    ...(includeTime ? { hour: "2-digit", minute: "2-digit" } : {}),
  }).format(new Date(value > 10_000_000_000 ? value : value * 1000))
}

export function formatRange(range: UsageRange) {
  return `${formatDate(range.startAtMs)} – ${formatDate(range.endAtMs - 1)}`
}

export function todayRange(days = 1): UsageRange {
  const end = new Date()
  end.setHours(24, 0, 0, 0)
  const start = new Date(end)
  start.setDate(start.getDate() - days)
  return { startAtMs: start.getTime(), endAtMs: end.getTime() }
}

export function tokenInput(tokens?: TokenBreakdown) {
  return tokens?.inputTokens ?? 0
}

export function cacheHitRate(tokens?: TokenBreakdown) {
  const input = tokens?.inputTokens ?? 0
  if (input <= 0) return undefined
  const cached = Math.min(Math.max(tokens?.cachedInputTokens ?? 0, 0), input)
  return (cached / input) * 100
}

export function formatPercent(value?: number) {
  return value === undefined
    ? "—"
    : new Intl.NumberFormat("zh-CN", {
        style: "percent",
        minimumFractionDigits: 1,
        maximumFractionDigits: 1,
      }).format(value / 100)
}

export function quotaWindow(quota?: AccountQuota) {
  return quota?.data?.primary ?? quota?.data?.secondary
}

export function errorMessage(reason: unknown) {
  return reason instanceof Error ? reason.message : String(reason)
}
