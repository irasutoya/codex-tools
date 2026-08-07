import { formatDateTime as formatSharedDateTime } from "@/lib/time"
import type {
  CostStatus,
  TokenBreakdown,
  UsageOverview,
  UsageRange,
} from "@/types"

import { getLocalDateKey } from "@/lib/local-time"

const compactTokenFormatter = new Intl.NumberFormat("en-US", {
  maximumFractionDigits: 1,
})

const millionTokenFormatter = new Intl.NumberFormat("en-US", {
  maximumFractionDigits: 2,
})

const integerFormatter = new Intl.NumberFormat("en-US", {
  maximumFractionDigits: 0,
})

const statusLabels: Record<CostStatus, string> = {
  estimated: "已估算",
  subscription: "套餐统计",
  unpriced: "未配置价格",
  partial: "部分数据",
  unattributed: "未归属",
  zero: "免费 / $0",
}

export function formatTokens(value: number) {
  if (!Number.isFinite(value) || value <= 0) return "0"
  if (value >= 1_000_000_000) {
    return `${millionTokenFormatter.format(value / 1_000_000_000)}B`
  }
  if (value >= 1_000_000) {
    return `${millionTokenFormatter.format(value / 1_000_000)}M`
  }
  if (value >= 1_000) {
    return `${compactTokenFormatter.format(value / 1_000)}K`
  }
  return integerFormatter.format(value)
}

export function formatUsdMicrousd(value?: number) {
  if (value === undefined || !Number.isFinite(value)) return "未估算"
  if (value <= 0) return "$0.00"

  const dollars = value / 1_000_000
  const fractionDigits = dollars < 0.01 ? 6 : 2
  const formatted = new Intl.NumberFormat("en-US", {
    minimumFractionDigits: 2,
    maximumFractionDigits: fractionDigits,
  }).format(dollars)
  return `$${formatted}`
}

export function formatEstimatedUsd(value: number, unpricedTokens: number) {
  if (value <= 0 && unpricedTokens > 0) return "未估算"
  return formatUsdMicrousd(value)
}

export function formatCostStatus(status: CostStatus) {
  return statusLabels[status]
}

export function formatDateTime(timestamp?: number) {
  if (timestamp === undefined || !Number.isFinite(timestamp)) return "尚未刷新"
  return formatSharedDateTime(timestamp)
}

export function formatTokenDetail(tokens: TokenBreakdown) {
  return `输入 ${formatTokens(tokens.inputTokens)} · 输出 ${formatTokens(tokens.outputTokens)} · 缓存读取 ${formatTokens(tokens.cachedInputTokens)}`
}

export function getLocalDayRange(daysAgo = 0): UsageRange {
  const now = new Date()
  const start = new Date(
    now.getFullYear(),
    now.getMonth(),
    now.getDate() - daysAgo
  )
  const end = new Date(
    start.getFullYear(),
    start.getMonth(),
    start.getDate() + 1
  )
  return { startAtMs: start.getTime(), endAtMs: end.getTime() }
}

export function getLocalRange(days: number): UsageRange {
  const today = getLocalDayRange()
  const start = new Date(today.startAtMs)
  start.setDate(start.getDate() - Math.max(0, days - 1))
  return { startAtMs: start.getTime(), endAtMs: today.endAtMs }
}

export function getLocalDateInput(daysAgo = 0) {
  const date = new Date()
  date.setHours(0, 0, 0, 0)
  date.setDate(date.getDate() - daysAgo)
  return getLocalDateKey(date)
}

export function getLocalDateRange(startDate: string, endDate: string) {
  if (
    !/^\d{4}-\d{2}-\d{2}$/.test(startDate) ||
    !/^\d{4}-\d{2}-\d{2}$/.test(endDate)
  ) {
    return undefined
  }
  const start = new Date(`${startDate}T00:00:00`)
  const end = new Date(`${endDate}T00:00:00`)
  if (!Number.isFinite(start.getTime()) || !Number.isFinite(end.getTime())) {
    return undefined
  }
  const endExclusive = new Date(end)
  endExclusive.setDate(endExclusive.getDate() + 1)
  const range = { startAtMs: start.getTime(), endAtMs: endExclusive.getTime() }
  return range.endAtMs > range.startAtMs ? range : undefined
}

export function formatTimezone() {
  return Intl.DateTimeFormat().resolvedOptions().timeZone || "本机时区"
}

export function formatRangeLabel(range: UsageRange) {
  const start = new Date(range.startAtMs)
  const end = new Date(range.endAtMs - 1)
  const startLabel = start.toLocaleDateString("zh-CN", {
    month: "short",
    day: "numeric",
  })
  const endLabel = end.toLocaleDateString("zh-CN", {
    month: "short",
    day: "numeric",
  })
  return startLabel === endLabel ? startLabel : `${startLabel} – ${endLabel}`
}

/**
 * 选择与当前查询范围匹配的概览数据。
 * 返回 `stale: true` 表示旧数据与当前范围不匹配（页面应显示加载或错误态）。
 */
export function pickDisplayOverview(
  overview: UsageOverview | undefined,
  currentRange: UsageRange,
  customRangeValid: boolean
): { display?: UsageOverview; stale: boolean } {
  if (!customRangeValid) return { display: overview, stale: false }
  if (!overview) return { display: undefined, stale: false }
  const matches =
    overview.range.startAtMs === currentRange.startAtMs &&
    overview.range.endAtMs === currentRange.endAtMs
  return { display: overview, stale: !matches }
}
