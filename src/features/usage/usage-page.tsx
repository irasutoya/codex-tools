import { useCallback, useMemo, useState } from "react"
import {
  Add01Icon,
  Database02Icon,
  Delete02Icon,
  InformationCircleIcon,
  Refresh01Icon,
} from "@hugeicons/core-free-icons"
import { HugeiconsIcon } from "@hugeicons/react"
import { CartesianGrid, Line, LineChart, XAxis, YAxis } from "recharts"

import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card"
import {
  ChartContainer,
  ChartLegend,
  ChartLegendContent,
  ChartTooltip,
  ChartTooltipContent,
} from "@/components/ui/chart"
import {
  Empty,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from "@/components/ui/empty"
import {
  Item,
  ItemActions,
  ItemContent,
  ItemDescription,
  ItemGroup,
  ItemTitle,
} from "@/components/ui/item"
import { Skeleton } from "@/components/ui/skeleton"
import { Spinner } from "@/components/ui/spinner"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { toast } from "@/components/ui/toast"
import {
  cacheHitRate,
  errorMessage,
  formatInteger,
  formatPercent,
  formatRange,
  formatTokens,
  formatUsd,
  todayRange,
} from "@/lib/format"
import {
  tokenTickFormatter,
  trendPointsToSeries,
  usageChartConfig,
} from "@/lib/chart"
import { useAsync } from "@/hooks/use-async"
import { call } from "@/lib/ipc"
import type { UsageGroupBy, UsageRow } from "@/types"

import { PricingEditor } from "./pricing-editor-dialog"
import { billingModeLabel, pricingSummary } from "./pricing"
import { UsageDetail } from "./usage-detail-sheet"

export function UsagePage({
  refreshRevision,
  days,
  groupBy,
  onRefresh,
}: {
  refreshRevision: number
  onRefresh: () => void
  days: number
  groupBy: UsageGroupBy
}) {
  const [tab, setTab] = useState("details")
  const [selected, setSelected] = useState<UsageRow>()
  const [ruleOpen, setRuleOpen] = useState(false)
  const [busy, setBusy] = useState(false)

  const query = useMemo(
    () => ({ range: todayRange(days), groupBy }),
    [days, groupBy]
  )

  const fetchOverview = useCallback(
    () => call("usage_get_overview", { query }),
    [query]
  )
  const {
    data: overview,
    error: overviewError,
    mutate: setOverview,
  } = useAsync(fetchOverview, undefined, refreshRevision)

  const fetchRules = useCallback(() => call("usage_list_pricing_rules", {}), [])
  const {
    data: rules,
    error: rulesError,
    mutate: setRules,
  } = useAsync(fetchRules, undefined, refreshRevision)

  const refreshUsage = async () => {
    setBusy(true)
    try {
      setOverview(await call("usage_refresh", { query }))
      toast.add({ title: "用量已刷新", type: "success" })
    } catch (reason) {
      toast.add({
        title: "刷新失败",
        description: errorMessage(reason),
        type: "error",
      })
    } finally {
      setBusy(false)
    }
  }

  const deleteRule = async (id: string) => {
    try {
      await call("usage_delete_pricing_rule", { id })
      setRules((current) => (current ?? []).filter((rule) => rule.id !== id))
      toast.add({ title: "价格规则已删除", type: "success" })
    } catch (reason) {
      toast.add({
        title: "删除失败",
        description: errorMessage(reason),
        type: "error",
      })
    }
  }

  const points = useMemo(
    () => trendPointsToSeries(overview?.trendPoints ?? []),
    [overview]
  )
  const cacheRows = useMemo(
    () =>
      overview?.rows.filter(
        (row) =>
          row.tokens.cachedInputTokens > 0 ||
          row.tokens.cacheWriteInputTokens > 0
      ) ?? [],
    [overview]
  )

  if (!overview && overviewError)
    return (
      <div className="min-h-full px-3 pt-1 pb-3">
        <Alert variant="destructive">
          <HugeiconsIcon icon={InformationCircleIcon} />
          <AlertTitle>无法读取用量</AlertTitle>
          <AlertDescription>{overviewError}</AlertDescription>
        </Alert>
      </div>
    )

  if (!overview)
    return (
      <div className="grid min-h-full grid-rows-[164px_minmax(208px,1fr)] gap-3 px-3 pt-1 pb-3">
        <Skeleton className="rounded-2xl" />
        <Skeleton className="rounded-2xl" />
      </div>
    )

  return (
    <div className="flex min-h-full flex-col gap-3 px-3 pt-1 pb-3">
      {overviewError && (
        <Alert variant="destructive">
          <HugeiconsIcon icon={InformationCircleIcon} />
          <AlertTitle>用量刷新失败</AlertTitle>
          <AlertDescription>{overviewError}</AlertDescription>
        </Alert>
      )}
      {rulesError && (
        <Alert variant="destructive">
          <HugeiconsIcon icon={InformationCircleIcon} />
          <AlertTitle>无法读取价格规则</AlertTitle>
          <AlertDescription>{rulesError}</AlertDescription>
        </Alert>
      )}
      <Card size="sm" className="shrink-0">
        <CardHeader className="grid grid-cols-[1fr_auto] items-center">
          <div>
            <CardTitle>用量趋势</CardTitle>
            <div className="mt-0.5 text-xs text-muted-foreground">
              {formatRange(overview.range)}
            </div>
          </div>
          <Button
            size="sm"
            variant="outline"
            disabled={busy}
            onClick={() => void refreshUsage()}
          >
            {busy ? (
              <Spinner data-icon="inline-start" />
            ) : (
              <HugeiconsIcon icon={Refresh01Icon} data-icon="inline-start" />
            )}
            扫描
          </Button>
        </CardHeader>
        <CardContent className="grid grid-cols-[minmax(0,1fr)_auto] items-center gap-3">
          <ChartContainer
            config={usageChartConfig}
            className="aspect-auto h-28 w-full"
            initialDimension={{ width: 450, height: 112 }}
          >
            <LineChart
              data={points}
              margin={{ left: 4, right: 8, top: 4, bottom: 0 }}
            >
              <CartesianGrid vertical={false} strokeDasharray="4 4" />
              <XAxis dataKey="date" tickLine={false} axisLine={false} />
              <YAxis
                width={56}
                tickLine={false}
                axisLine={false}
                tickMargin={8}
                tickFormatter={tokenTickFormatter}
              />
              <ChartTooltip
                cursor={false}
                content={<ChartTooltipContent indicator="line" />}
              />
              <ChartLegend content={<ChartLegendContent />} />
              <Line
                dataKey="input"
                type="linear"
                stroke="var(--color-input)"
                strokeWidth={2}
                dot={false}
              />
              <Line
                dataKey="output"
                type="linear"
                stroke="var(--color-output)"
                strokeWidth={2}
                dot={false}
              />
              <Line
                dataKey="cache"
                type="linear"
                stroke="var(--color-cache)"
                strokeWidth={1.75}
                strokeDasharray="5 4"
                dot={false}
              />
            </LineChart>
          </ChartContainer>
          <div className="grid min-w-44 grid-cols-2 gap-x-4 gap-y-3">
            <Metric
              label="总 Token"
              value={formatTokens(overview.totals.tokens.totalTokens)}
            />
            <Metric
              label="请求"
              value={formatInteger(overview.totals.requests)}
            />
            <Metric
              label="缓存命中"
              value={formatPercent(cacheHitRate(overview.totals.tokens))}
            />
            <Metric
              label="估算费用"
              value={formatUsd(overview.totals.estimatedCostMicrousd)}
            />
          </div>
        </CardContent>
      </Card>

      <Card size="sm" className="min-h-52 flex-1">
        <Tabs
          value={tab}
          onValueChange={setTab}
          className="flex min-h-0 flex-1 flex-col"
        >
          <CardHeader className="grid grid-cols-[1fr_auto] items-center">
            <TabsList>
              <TabsTrigger value="details">明细</TabsTrigger>
              <TabsTrigger value="cache">缓存</TabsTrigger>
              <TabsTrigger value="pricing">价格规则</TabsTrigger>
            </TabsList>
            {tab === "pricing" && (
              <Button size="sm" onClick={() => setRuleOpen(true)}>
                <HugeiconsIcon icon={Add01Icon} data-icon="inline-start" />
                添加规则
              </Button>
            )}
          </CardHeader>
          <CardContent className="min-h-0 flex-1 overflow-y-auto">
            <TabsContent value="details" className="mt-0">
              {overview.rows.length ? (
                <ItemGroup>
                  {overview.rows.map((row) => (
                    <Item
                      key={row.key}
                      size="xs"
                      variant="outline"
                      className="flex-nowrap"
                      render={<button type="button" />}
                      onClick={() => setSelected(row)}
                    >
                      <ItemContent>
                        <ItemTitle>
                          {groupBy === "model" ? row.model : row.sourceName}
                        </ItemTitle>
                        <ItemDescription>
                          {formatInteger(row.requests)} 次请求 ·{" "}
                          {row.pricingRuleName ?? "未匹配价格规则"}
                        </ItemDescription>
                      </ItemContent>
                      <ItemActions className="text-right">
                        <div>
                          <div className="font-medium tabular-nums">
                            {formatTokens(row.tokens.totalTokens)}
                          </div>
                          <div className="text-xs text-muted-foreground">
                            {formatUsd(row.estimatedCostMicrousd)}
                          </div>
                        </div>
                      </ItemActions>
                    </Item>
                  ))}
                </ItemGroup>
              ) : (
                <Empty>
                  <EmptyHeader>
                    <EmptyMedia variant="icon">
                      <HugeiconsIcon icon={InformationCircleIcon} />
                    </EmptyMedia>
                    <EmptyTitle>暂无用量</EmptyTitle>
                    <EmptyDescription>
                      扫描 Codex 会话后会显示详细数据。
                    </EmptyDescription>
                  </EmptyHeader>
                </Empty>
              )}
            </TabsContent>
            <TabsContent value="pricing" className="mt-0">
              {rules?.length ? (
                <ItemGroup>
                  {rules.map((rule) => (
                    <Item
                      key={rule.id}
                      size="xs"
                      variant="outline"
                      className="flex-nowrap"
                    >
                      <ItemContent>
                        <ItemTitle>
                          {rule.modelPattern}
                          <Badge variant="secondary">
                            {billingModeLabel(rule.billingMode)}
                          </Badge>
                        </ItemTitle>
                        <ItemDescription>
                          {pricingSummary(rule)}
                        </ItemDescription>
                      </ItemContent>
                      <ItemActions>
                        <Button
                          size="icon-sm"
                          variant="ghost"
                          aria-label="删除规则"
                          onClick={() => void deleteRule(rule.id)}
                        >
                          <HugeiconsIcon icon={Delete02Icon} />
                        </Button>
                      </ItemActions>
                    </Item>
                  ))}
                </ItemGroup>
              ) : (
                <Empty>
                  <EmptyHeader>
                    <EmptyTitle>使用官方参考价格</EmptyTitle>
                    <EmptyDescription>
                      还没有自定义价格规则；官方账号会使用缓存的参考价格。
                    </EmptyDescription>
                  </EmptyHeader>
                </Empty>
              )}
            </TabsContent>
            <TabsContent value="cache" className="mt-0">
              {cacheRows.length ? (
                <ItemGroup>
                  {cacheRows.map((row) => (
                    <Item
                      key={row.key}
                      size="xs"
                      variant="outline"
                      className="flex-nowrap"
                      render={<button type="button" />}
                      onClick={() => setSelected(row)}
                    >
                      <ItemContent>
                        <ItemTitle>
                          {groupBy === "model" ? row.model : row.sourceName}
                        </ItemTitle>
                        <ItemDescription>
                          读取 {formatTokens(row.tokens.cachedInputTokens)} ·
                          写入 {formatTokens(row.tokens.cacheWriteInputTokens)}
                        </ItemDescription>
                      </ItemContent>
                      <ItemActions className="text-right">
                        <div>
                          <div className="font-medium tabular-nums">
                            {formatPercent(cacheHitRate(row.tokens))}
                          </div>
                          <div className="text-xs text-muted-foreground">
                            命中率
                          </div>
                        </div>
                      </ItemActions>
                    </Item>
                  ))}
                </ItemGroup>
              ) : (
                <Empty>
                  <EmptyHeader>
                    <EmptyMedia variant="icon">
                      <HugeiconsIcon icon={Database02Icon} />
                    </EmptyMedia>
                    <EmptyTitle>暂无缓存数据</EmptyTitle>
                    <EmptyDescription>
                      当前范围内没有检测到缓存读取或写入 Token。
                    </EmptyDescription>
                  </EmptyHeader>
                </Empty>
              )}
            </TabsContent>
          </CardContent>
        </Tabs>
      </Card>

      <UsageDetail
        row={selected}
        groupBy={groupBy}
        onOpenChange={(open) => !open && setSelected(undefined)}
      />
      <PricingEditor
        open={ruleOpen}
        onOpenChange={setRuleOpen}
        onSaved={() => {
          setRuleOpen(false)
          onRefresh()
        }}
      />
    </div>
  )
}

function Metric({ label, value }: { label: string; value: string }) {
  return (
    <div>
      <div className="text-xs text-muted-foreground">{label}</div>
      <div className="mt-0.5 text-base font-medium tabular-nums">{value}</div>
    </div>
  )
}
