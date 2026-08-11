import { useEffect, useMemo, useState } from "react"
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
  type ChartConfig,
} from "@/components/ui/chart"
import {
  Dialog,
  DialogBody,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import {
  Empty,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from "@/components/ui/empty"
import {
  Field,
  FieldContent,
  FieldDescription,
  FieldGroup,
  FieldLabel,
} from "@/components/ui/field"
import { Input } from "@/components/ui/input"
import { Progress, ProgressLabel } from "@/components/ui/progress"
import {
  Item,
  ItemActions,
  ItemContent,
  ItemDescription,
  ItemGroup,
  ItemTitle,
} from "@/components/ui/item"
import { Skeleton } from "@/components/ui/skeleton"
import {
  Sheet,
  SheetBody,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
} from "@/components/ui/sheet"
import { Spinner } from "@/components/ui/spinner"
import { Switch } from "@/components/ui/switch"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group"
import { toast } from "@/components/ui/toast"
import {
  cacheHitRate,
  errorMessage,
  formatDate,
  formatInteger,
  formatPercent,
  formatRange,
  formatTokens,
  formatUsd,
  todayRange,
} from "@/lib/format"
import { call } from "@/lib/ipc"
import type {
  BillingMode,
  PricingRule,
  UsageGroupBy,
  UsageOverview,
  UsageRow,
} from "@/types"

const chartConfig = {
  input: { label: "输入 Token", color: "var(--chart-1)" },
  output: { label: "输出 Token", color: "var(--chart-2)" },
  cache: { label: "缓存 Token", color: "var(--chart-3)" },
} satisfies ChartConfig

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
  const [overview, setOverview] = useState<UsageOverview>()
  const [rules, setRules] = useState<PricingRule[]>([])
  const [tab, setTab] = useState("details")
  const [selected, setSelected] = useState<UsageRow>()
  const [ruleOpen, setRuleOpen] = useState(false)
  const [busy, setBusy] = useState(false)
  const [overviewError, setOverviewError] = useState<string>()
  const [rulesError, setRulesError] = useState<string>()

  const query = useMemo(
    () => ({ range: todayRange(days), groupBy }),
    [days, groupBy]
  )

  useEffect(() => {
    let cancelled = false
    void call("usage_get_overview", { query })
      .then((nextOverview) => {
        if (cancelled) return
        setOverview(nextOverview)
        setOverviewError(undefined)
      })
      .catch((reason) => !cancelled && setOverviewError(errorMessage(reason)))
    return () => {
      cancelled = true
    }
  }, [query, refreshRevision])

  useEffect(() => {
    let cancelled = false
    void call("usage_list_pricing_rules", {})
      .then((nextRules) => {
        if (cancelled) return
        setRules(nextRules)
        setRulesError(undefined)
      })
      .catch((reason) => !cancelled && setRulesError(errorMessage(reason)))
    return () => {
      cancelled = true
    }
  }, [refreshRevision])

  const refreshUsage = async () => {
    setBusy(true)
    try {
      setOverview(await call("usage_refresh", { query }))
      setOverviewError(undefined)
      toast.add({ title: "用量已刷新", type: "success" })
    } catch (reason) {
      setOverviewError(errorMessage(reason))
      toast.add({
        title: "刷新失败",
        description: errorMessage(reason),
        type: "error",
      })
    } finally {
      setBusy(false)
    }
  }

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

  const points = overview.trendPoints.map((point) => ({
    date: formatDate(point.dayStartMs),
    input: point.tokens.inputTokens,
    output: point.tokens.outputTokens + point.tokens.reasoningOutputTokens,
    cache: point.tokens.cachedInputTokens + point.tokens.cacheWriteInputTokens,
  }))
  const cacheRows = overview.rows.filter(
    (row) =>
      row.tokens.cachedInputTokens > 0 || row.tokens.cacheWriteInputTokens > 0
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
            config={chartConfig}
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
                tickFormatter={(value) => formatTokens(Number(value))}
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
              {rules.length ? (
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

  async function deleteRule(id: string) {
    try {
      await call("usage_delete_pricing_rule", { id })
      setRules((value) => value.filter((rule) => rule.id !== id))
      toast.add({ title: "价格规则已删除", type: "success" })
    } catch (reason) {
      toast.add({
        title: "删除失败",
        description: errorMessage(reason),
        type: "error",
      })
    }
  }
}

function Metric({ label, value }: { label: string; value: string }) {
  return (
    <div>
      <div className="text-xs text-muted-foreground">{label}</div>
      <div className="mt-0.5 text-base font-medium tabular-nums">{value}</div>
    </div>
  )
}

function UsageDetail({
  row,
  groupBy,
  onOpenChange,
}: {
  row?: UsageRow
  groupBy: UsageGroupBy
  onOpenChange: (open: boolean) => void
}) {
  if (!row) return null
  const hitRate = cacheHitRate(row.tokens)
  const details = [
    ["普通输入", row.tokens.inputTokens],
    ["缓存输入", row.tokens.cachedInputTokens],
    ["缓存写入", row.tokens.cacheWriteInputTokens],
    ["普通输出", row.tokens.outputTokens],
    ["推理输出", row.tokens.reasoningOutputTokens],
  ] as const
  return (
    <Sheet open onOpenChange={onOpenChange}>
      <SheetContent>
        <SheetHeader>
          <SheetTitle>
            {groupBy === "model" ? row.model : row.sourceName}
          </SheetTitle>
          <SheetDescription>
            {groupBy === "model" ? "模型" : "账号"}汇总的 Token 构成
          </SheetDescription>
        </SheetHeader>
        <SheetBody className="grid content-start gap-2">
          <Progress
            value={hitRate ?? 0}
            className="rounded-2xl bg-muted/40 p-3"
          >
            <ProgressLabel>缓存命中率</ProgressLabel>
            <span className="ml-auto text-sm text-muted-foreground tabular-nums">
              {formatPercent(hitRate)}
            </span>
          </Progress>
          {details.map(([label, value]) => (
            <div
              key={label}
              className="flex items-center justify-between rounded-2xl bg-muted px-3 py-2"
            >
              <span className="text-muted-foreground">{label}</span>
              <span className="font-medium tabular-nums">
                {formatInteger(value)}
              </span>
            </div>
          ))}
          <div className="flex items-center justify-between border-t pt-4">
            <span className="font-medium">合计</span>
            <span className="text-lg font-medium tabular-nums">
              {formatInteger(row.tokens.totalTokens)}
            </span>
          </div>
        </SheetBody>
      </SheetContent>
    </Sheet>
  )
}

function PricingEditor({
  open,
  onOpenChange,
  onSaved,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  onSaved: () => void
}) {
  const [pattern, setPattern] = useState("")
  const [billingMode, setBillingMode] =
    useState<Extract<BillingMode, "token" | "unpriced">>("token")
  const [input, setInput] = useState("2.50")
  const [cachedRead, setCachedRead] = useState("0.25")
  const [cacheWrite, setCacheWrite] = useState("3.00")
  const [output, setOutput] = useState("10.00")
  const [cacheWriteIncluded, setCacheWriteIncluded] = useState(true)
  const [active, setActive] = useState(true)
  const [busy, setBusy] = useState(false)
  const save = async () => {
    setBusy(true)
    const now = Date.now()
    const rule: PricingRule = {
      id: `rule-${now}`,
      version: 1,
      active,
      scopeKind: "global_model",
      modelPattern: pattern,
      matchKind: "exact",
      billingMode,
      inputUsdPerMillion: billingMode === "token" ? input : undefined,
      cachedReadUsdPerMillion: billingMode === "token" ? cachedRead : undefined,
      cacheWriteUsdPerMillion: billingMode === "token" ? cacheWrite : undefined,
      outputUsdPerMillion: billingMode === "token" ? output : undefined,
      cacheWriteIncludedInInput: cacheWriteIncluded,
      effectiveFromMs: now,
      createdAtMs: now,
      updatedAtMs: now,
    }
    try {
      await call("usage_save_pricing_rule", { input: rule })
      toast.add({ title: "价格规则已保存", type: "success" })
      onSaved()
    } catch (reason) {
      toast.add({
        title: "保存失败",
        description: errorMessage(reason),
        type: "error",
      })
    } finally {
      setBusy(false)
    }
  }
  return (
    <Dialog
      open={open}
      onOpenChange={(nextOpen) => {
        if (!nextOpen && busy) return
        onOpenChange(nextOpen)
      }}
    >
      <DialogContent showCloseButton={!busy} aria-busy={busy}>
        <DialogHeader>
          <DialogTitle>添加价格规则</DialogTitle>
          <DialogDescription>
            选择不计价，或按每百万 Token 的美元金额计价。
          </DialogDescription>
        </DialogHeader>
        <DialogBody>
          <FieldGroup>
            <Field>
              <FieldLabel htmlFor="price-model">模型</FieldLabel>
              <Input
                id="price-model"
                value={pattern}
                placeholder="gpt-5.6"
                onChange={(e) => setPattern(e.target.value)}
              />
            </Field>
            <Field>
              <FieldLabel>计费方式</FieldLabel>
              <ToggleGroup
                variant="outline"
                spacing={0}
                className="w-full"
                value={[billingMode]}
                onValueChange={(value) => {
                  if (value[0] === "token" || value[0] === "unpriced") {
                    setBillingMode(value[0])
                  }
                }}
              >
                <ToggleGroupItem className="flex-1" value="token">
                  按 Token 计价
                </ToggleGroupItem>
                <ToggleGroupItem className="flex-1" value="unpriced">
                  不计价
                </ToggleGroupItem>
              </ToggleGroup>
              <FieldDescription>
                不计价规则会保留 Token 用量，但不估算费用。
              </FieldDescription>
            </Field>
            {billingMode === "token" && (
              <FieldGroup className="grid grid-cols-2 gap-3">
                <Field>
                  <FieldLabel htmlFor="price-input">普通输入</FieldLabel>
                  <Input
                    id="price-input"
                    inputMode="decimal"
                    value={input}
                    onChange={(e) => setInput(e.target.value)}
                  />
                </Field>
                <Field>
                  <FieldLabel htmlFor="price-cached-read">缓存读取</FieldLabel>
                  <Input
                    id="price-cached-read"
                    inputMode="decimal"
                    value={cachedRead}
                    onChange={(e) => setCachedRead(e.target.value)}
                  />
                </Field>
                <Field>
                  <FieldLabel htmlFor="price-cache-write">缓存写入</FieldLabel>
                  <Input
                    id="price-cache-write"
                    inputMode="decimal"
                    value={cacheWrite}
                    onChange={(e) => setCacheWrite(e.target.value)}
                  />
                </Field>
                <Field>
                  <FieldLabel htmlFor="price-output">输出</FieldLabel>
                  <Input
                    id="price-output"
                    inputMode="decimal"
                    value={output}
                    onChange={(e) => setOutput(e.target.value)}
                  />
                </Field>
              </FieldGroup>
            )}
            {billingMode === "token" && (
              <Field orientation="horizontal">
                <FieldContent>
                  <FieldLabel htmlFor="price-cache-write-included">
                    输入总量包含缓存写入
                  </FieldLabel>
                  <FieldDescription>
                    开启后，普通输入计费会扣除缓存读取和缓存写入 Token。
                  </FieldDescription>
                </FieldContent>
                <Switch
                  id="price-cache-write-included"
                  checked={cacheWriteIncluded}
                  onCheckedChange={setCacheWriteIncluded}
                />
              </Field>
            )}
            <Field orientation="horizontal">
              <FieldLabel htmlFor="price-active">立即启用</FieldLabel>
              <Switch
                id="price-active"
                checked={active}
                onCheckedChange={setActive}
              />
            </Field>
          </FieldGroup>
        </DialogBody>
        <DialogFooter>
          <Button
            variant="outline"
            disabled={busy}
            onClick={() => onOpenChange(false)}
          >
            取消
          </Button>
          <Button disabled={busy || !pattern} onClick={() => void save()}>
            {busy && <Spinner data-icon="inline-start" />}保存
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}

function pricingSummary(rule: PricingRule) {
  if (rule.billingMode === "subscription") {
    return `${rule.scopeKind} · 旧版订阅规则（将按不计价迁移）`
  }
  if (rule.billingMode === "unpriced") return `${rule.scopeKind} · 不计价`
  const price = (value?: string) => (value ? `$${value}` : "未设置")
  return `${rule.scopeKind} · 输入 ${price(rule.inputUsdPerMillion)} · 缓存读取 ${price(rule.cachedReadUsdPerMillion)} · 缓存写入 ${price(rule.cacheWriteUsdPerMillion)} · 输出 ${price(rule.outputUsdPerMillion)} / 1M`
}

function billingModeLabel(mode: BillingMode) {
  if (mode === "token") return "按 Token"
  if (mode === "unpriced") return "不计价"
  return "旧版订阅规则"
}
