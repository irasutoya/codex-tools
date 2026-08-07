import { useEffect, useState } from "react"
import { Bar, CartesianGrid, ComposedChart, Line, XAxis, YAxis } from "recharts"
import { BoxesIcon } from "@hugeicons/core-free-icons"
import { HugeiconsIcon } from "@hugeicons/react"

import {
  ChartContainer,
  ChartTooltip,
  ChartTooltipContent,
  type ChartConfig,
} from "@/components/ui/chart"
import {
  Empty,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from "@/components/ui/empty"
import { Skeleton } from "@/components/ui/skeleton"
import { call } from "@/lib/ipc"
import { formatUsdMicrousd } from "./usage-format"
import type { UsageRange, UsageTrend, UsageTrendPoint } from "@/types"

const chartConfig = {
  tokens: { label: "总 Token", color: "var(--chart-1)" },
  cost: { label: "估算费用", color: "var(--chart-2)" },
} satisfies ChartConfig

export function UsageTrendChart({
  range,
  refreshKey = 0,
  points,
  emptyLabel = "所选范围暂无趋势数据",
}: {
  range: UsageRange
  /** 页面数据刷新序号：变化时重新拉取趋势，避免刷新后仍显示旧数据。 */
  refreshKey?: number
  /** 已聚合好的趋势点（用量页与 totals 同趟获得时传入，跳过二次查询）。 */
  points?: UsageTrendPoint[]
  emptyLabel?: string
}) {
  return (
    <TrendContent
      key={`${range.startAtMs}:${range.endAtMs}:${refreshKey}`}
      range={range}
      points={points}
      emptyLabel={emptyLabel}
    />
  )
}

function TrendContent({
  range,
  emptyLabel,
  points,
}: {
  range: UsageRange
  emptyLabel: string
  points?: UsageTrendPoint[]
}) {
  const { startAtMs, endAtMs } = range
  const [trend, setTrend] = useState<UsageTrend>()
  const [loading, setLoading] = useState(!points)
  const [failed, setFailed] = useState(false)

  useEffect(() => {
    if (points) return
    // 组件通过 key 随 refreshKey 重挂载，初始 loading 状态即为 true，
    // 无需在 effect 内再置位。
    let cancelled = false
    call("get_usage_trend", { range: { startAtMs, endAtMs } })
      .then((result) => {
        if (!cancelled) setTrend(result)
      })
      .catch(() => {
        if (!cancelled) setFailed(true)
      })
      .finally(() => {
        if (!cancelled) setLoading(false)
      })
    return () => {
      cancelled = true
    }
  }, [startAtMs, endAtMs, points])

  if (loading) {
    return <Skeleton className="h-40 w-full rounded-lg" />
  }

  const data =
    (points ?? trend?.points ?? []).map((point) => ({
      day: new Date(point.dayStartMs).toLocaleDateString("zh-CN", {
        month: "numeric",
        day: "numeric",
      }),
      tokens: point.tokens.totalTokens,
      cost: point.estimatedCostMicrousd,
    })) ?? []

  if (failed || data.length === 0) {
    return (
      <Empty className="min-h-40">
        <EmptyHeader>
          <EmptyMedia variant="icon">
            <HugeiconsIcon icon={BoxesIcon} />
          </EmptyMedia>
          <EmptyTitle>{failed ? "无法读取趋势数据" : emptyLabel}</EmptyTitle>
          <EmptyDescription>
            {failed ? "请稍后刷新重试。" : "所选时间范围内还没有可统计的记录。"}
          </EmptyDescription>
        </EmptyHeader>
      </Empty>
    )
  }

  return (
    <ChartContainer config={chartConfig} className="h-40 w-full">
      <ComposedChart
        data={data}
        margin={{ left: 0, right: 0, top: 4, bottom: 0 }}
      >
        <CartesianGrid vertical={false} />
        <XAxis
          dataKey="day"
          tickLine={false}
          axisLine={false}
          tickMargin={8}
          minTickGap={12}
        />
        <YAxis
          yAxisId="left"
          tickLine={false}
          axisLine={false}
          width={44}
          tickFormatter={(value) => compactNumber(value as number)}
        />
        <YAxis
          yAxisId="right"
          orientation="right"
          tickLine={false}
          axisLine={false}
          width={46}
          tickFormatter={(value) => formatUsdMicrousd(Number(value))}
        />
        <ChartTooltip
          content={
            <ChartTooltipContent
              formatter={(value, name) => {
                if (name === "tokens") {
                  return [compactNumber(Number(value)), "总 Token"]
                }
                return [formatUsdMicrousd(Number(value)), "估算费用"]
              }}
            />
          }
        />
        <Bar
          yAxisId="left"
          dataKey="tokens"
          fill="var(--color-tokens)"
          radius={[4, 4, 0, 0]}
          barSize={18}
        />
        <Line
          yAxisId="right"
          type="monotone"
          dataKey="cost"
          stroke="var(--color-cost)"
          strokeWidth={2}
          dot={false}
        />
      </ComposedChart>
    </ChartContainer>
  )
}

function compactNumber(value: number) {
  if (value >= 1_000_000) return `${(value / 1_000_000).toFixed(1)}M`
  if (value >= 1_000) return `${(value / 1_000).toFixed(1)}k`
  return String(value)
}
