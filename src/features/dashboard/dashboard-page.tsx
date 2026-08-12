import { useCallback, useMemo } from "react"
import { InformationCircleIcon } from "@hugeicons/core-free-icons"
import { HugeiconsIcon } from "@hugeicons/react"

import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card"
import { TrendChart } from "@/components/ui/trend-chart"
import { Progress, ProgressLabel } from "@/components/ui/progress"
import { Skeleton } from "@/components/ui/skeleton"
import {
  formatDate,
  formatInteger,
  formatTokens,
  formatUsd,
  quotaWindow,
  todayRange,
  tokenInput,
} from "@/lib/format"
import { trendPointsToSeries } from "@/lib/chart"
import { useAsync } from "@/hooks/use-async"
import { call } from "@/lib/ipc"
import type { Dashboard } from "@/types"

export function DashboardPage({
  dashboard,
  refreshRevision,
}: {
  dashboard?: Dashboard
  refreshRevision: number
}) {
  const fetchUsage = useCallback(
    () =>
      call("usage_get_overview", {
        query: { range: todayRange(7), groupBy: "model" },
      }),
    []
  )
  const { data: usage, error } = useAsync(
    fetchUsage,
    undefined,
    refreshRevision
  )

  const points = useMemo(
    () => trendPointsToSeries(usage?.trendPoints ?? []),
    [usage]
  )
  const quota = quotaWindow(dashboard?.activeQuota)

  if (!dashboard || (!usage && !error)) {
    return (
      <div className="grid min-h-full grid-rows-[minmax(240px,1fr)_80px] gap-3 px-3 pt-1 pb-3">
        <Skeleton className="rounded-2xl" />
        <Skeleton className="rounded-2xl" />
      </div>
    )
  }

  if (!usage) {
    return (
      <div className="min-h-full px-3 pt-1 pb-3">
        <Alert variant="destructive">
          <HugeiconsIcon icon={InformationCircleIcon} />
          <AlertTitle>无法读取概览用量</AlertTitle>
          <AlertDescription>{error}</AlertDescription>
        </Alert>
      </div>
    )
  }

  return (
    <div className="flex min-h-full flex-col gap-3 px-3 pt-1 pb-3">
      {error && (
        <Alert variant="destructive">
          <HugeiconsIcon icon={InformationCircleIcon} />
          <AlertTitle>数据刷新失败</AlertTitle>
          <AlertDescription>{error}</AlertDescription>
        </Alert>
      )}

      <Card size="sm" className="min-h-60 flex-1">
        <CardHeader className="grid grid-cols-[1fr_auto] items-center">
          <CardTitle>Token 趋势（最近 7 天）</CardTitle>
          <span className="text-xs text-muted-foreground">按本地日期</span>
        </CardHeader>
        <CardContent className="flex min-h-0 flex-1">
          <TrendChart
            points={points}
            showDots
            className="min-h-0 w-full flex-1"
          />
        </CardContent>
      </Card>

      <Card size="sm" className="shrink-0">
        <CardContent className="grid grid-cols-3 divide-x divide-border">
          <Metric
            label="总 Token"
            value={formatTokens(usage.totals.tokens.totalTokens)}
            detail={`输入 ${formatTokens(tokenInput(usage.totals.tokens))} · 输出 ${formatTokens(usage.totals.tokens.outputTokens)}`}
          />
          <Metric
            label="请求"
            value={formatInteger(usage.totals.requests)}
            detail="已统计请求"
          />
          <Metric
            label="估算费用"
            value={formatUsd(usage.totals.estimatedCostMicrousd)}
            detail="按当前价格规则"
          />
        </CardContent>
      </Card>

      <Card size="sm" className="shrink-0">
        <CardContent className="grid grid-cols-[minmax(0,1fr)_auto] items-end gap-4">
          <Progress value={quota?.remainingPercent ?? 0}>
            <ProgressLabel>Token 剩余额度</ProgressLabel>
            <span className="ml-auto text-sm text-muted-foreground tabular-nums">
              {quota ? `${quota.remainingPercent.toFixed(1)}%` : "暂不可用"}
            </span>
          </Progress>
          <div className="text-right">
            <div className="text-xs text-muted-foreground">重置日期</div>
            <div className="mt-1 font-medium tabular-nums">
              {formatDate(quota?.resetAt)}
            </div>
          </div>
        </CardContent>
      </Card>
    </div>
  )
}

function Metric({
  label,
  value,
  detail,
}: {
  label: string
  value: string
  detail: string
}) {
  return (
    <div className="flex min-w-0 flex-col gap-1 px-3 first:pl-0 last:pr-0">
      <span className="text-xs text-muted-foreground">{label}</span>
      <span className="text-xl font-medium tracking-tight tabular-nums">
        {value}
      </span>
      <span className="truncate text-xs text-muted-foreground">{detail}</span>
    </div>
  )
}
