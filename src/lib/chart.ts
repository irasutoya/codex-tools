import { formatDate } from "@/lib/format"
import type { UsageTrendPoint } from "@/types"

export type TrendSeriesKey = "input" | "output" | "cache"

export const usageChartConfig: Record<
  TrendSeriesKey,
  { label: string; color: string; dashed?: boolean }
> = {
  input: { label: "输入 Token", color: "var(--chart-1)" },
  output: { label: "输出 Token", color: "var(--chart-2)" },
  cache: { label: "缓存 Token", color: "var(--chart-3)", dashed: true },
}

export type TrendSeriesPoint = {
  date: string
  input: number
  output: number
  cache: number
}

export function trendPointsToSeries(
  points: UsageTrendPoint[]
): TrendSeriesPoint[] {
  return points.map((point) => ({
    date: formatDate(point.dayStartMs),
    input: point.tokens.inputTokens,
    output: point.tokens.outputTokens + point.tokens.reasoningOutputTokens,
    cache: point.tokens.cachedInputTokens + point.tokens.cacheWriteInputTokens,
  }))
}
