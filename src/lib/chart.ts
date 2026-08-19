import type { ChartConfig } from "@/components/ui/chart"
import { formatDate, formatTokens } from "@/lib/format"
import type { UsageTrendPoint } from "@/types"

export const usageChartConfig = {
  input: { label: "输入 Token", color: "var(--chart-1)" },
  output: { label: "输出 Token", color: "var(--chart-2)" },
  cache: { label: "缓存 Token", color: "var(--chart-3)" },
} satisfies ChartConfig

export type TrendSeriesPoint = {
  date: string
  input: number
  output: number
  cache: number
}

export function trendPointsToSeries(
  points: UsageTrendPoint[],
  hourly = false
): TrendSeriesPoint[] {
  return points.map((point) => ({
    date: formatDate(point.dayStartMs, hourly),
    input: point.tokens.inputTokens,
    output: point.tokens.outputTokens + point.tokens.reasoningOutputTokens,
    cache: point.tokens.cachedInputTokens + point.tokens.cacheWriteInputTokens,
  }))
}

export function tokenTickFormatter(value: number | string | undefined) {
  return formatTokens(Number(value))
}
