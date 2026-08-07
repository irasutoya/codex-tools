import { memo, useCallback, useEffect, useRef, useState } from "react"
import {
  Alert01Icon,
  BookOpen01Icon,
  Calendar01Icon,
  Dollar01Icon,
  Refresh01Icon,
  SecurityCheckIcon,
  Settings02Icon,
  Share01Icon,
} from "@hugeicons/core-free-icons"
import { HugeiconsIcon } from "@hugeicons/react"

import { ErrorDetails } from "@/components/error-details"
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import {
  Card,
  CardAction,
  CardContent,
  CardDescription,
  CardFooter,
  CardHeader,
  CardTitle,
} from "@/components/ui/card"
import {
  Empty,
  EmptyContent,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from "@/components/ui/empty"
import { Field, FieldLabel } from "@/components/ui/field"
import { Input } from "@/components/ui/input"
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
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
} from "@/components/ui/sheet"
import { Spinner } from "@/components/ui/spinner"
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group"
import { notify, formatError } from "@/lib/feedback"
import { call } from "@/lib/ipc"
import { cn } from "@/lib/utils"
import {
  refreshCoordinator,
  useAppForeground,
  usePageRefresh,
} from "@/lib/refresh-coordinator"
import { shouldRefreshUsageOnEntry } from "@/lib/usage-refresh-policy"
import type {
  CostStatus,
  PageProps,
  PricingRule,
  ProviderOverview,
  UsageGroupBy,
  UsageOverview,
  UsageRow,
  OfficialPricingCatalog,
} from "@/types"

import { OfficialPricingDialog } from "./official-pricing-dialog"
import { PricingRuleDialog } from "./pricing-rule-dialog"
import { UsageShareDialog } from "./usage-share-dialog"
import { UsageTrendChart } from "./usage-trend-chart"
import {
  formatCostStatus,
  formatDateTime,
  formatEstimatedUsd,
  formatRangeLabel,
  formatTokenDetail,
  formatTokens,
  formatUsdMicrousd,
  getLocalDateInput,
  getLocalDateRange,
  getLocalDayRange,
  getLocalRange,
  formatTimezone,
  pickDisplayOverview,
} from "./usage-format"

const RANGE_OPTIONS = [
  { key: "today", label: "今天" },
  { key: "yesterday", label: "昨天" },
  { key: "7", label: "最近 7 天" },
  { key: "custom", label: "自定义" },
] as const

type RangeMode = (typeof RANGE_OPTIONS)[number]["key"]

type PricingField = "model" | "provider" | "prices" | "effective"

type PricingFieldErrors = Partial<Record<PricingField, string>>

export default function UsagePage({ active }: PageProps) {
  const refreshSignal = usePageRefresh("usage")
  const foreground = useAppForeground()
  const [rangeMode, setRangeMode] = useState<RangeMode>("today")
  const [customStart, setCustomStart] = useState(() => getLocalDateInput())
  const [customEnd, setCustomEnd] = useState(() => getLocalDateInput())
  const [groupBy, setGroupBy] = useState<UsageGroupBy>("model")
  const [overview, setOverview] = useState<UsageOverview>()
  const [officialCatalog, setOfficialCatalog] =
    useState<OfficialPricingCatalog>()
  const [officialCatalogLoading, setOfficialCatalogLoading] = useState(false)
  const [officialCatalogError, setOfficialCatalogError] = useState<string>()
  const [officialCatalogFailed, setOfficialCatalogFailed] = useState(false)
  const [officialCatalogRetryRevision, setOfficialCatalogRetryRevision] =
    useState(0)
  const [rules, setRules] = useState<PricingRule[]>([])
  const [providers, setProviders] = useState<ProviderOverview>()
  const [pricingDraft, setPricingDraft] = useState<PricingRule>()
  const [repriceAfterSave, setRepriceAfterSave] = useState(true)
  const [pendingDelete, setPendingDelete] = useState<PricingRule>()
  const [loading, setLoading] = useState(true)
  const [refreshing, setRefreshing] = useState(false)
  const [pricingLoading, setPricingLoading] = useState(false)
  const [error, setError] = useState<string>()
  const [pricingError, setPricingError] = useState<string>()
  const [pricingFieldErrors, setPricingFieldErrors] =
    useState<PricingFieldErrors>({})
  const [selectedRow, setSelectedRow] = useState<UsageRow>()
  const [shareOpen, setShareOpen] = useState(false)
  const [pricingDialogOpen, setPricingDialogOpen] = useState(false)
  const [shareAccountOverview, setShareAccountOverview] =
    useState<UsageOverview>()
  const [shareModelOverview, setShareModelOverview] = useState<UsageOverview>()
  const [shareLoading, setShareLoading] = useState(false)
  const [shareError, setShareError] = useState<string>()
  const running = useRef(false)
  const nextRefreshAt = useRef<number | undefined>(undefined)
  const refreshQueued = useRef(false)
  const refreshRequested = useRef(false)
  const requestVersion = useRef(0)
  const initialized = useRef(false)
  const lastQueryKey = useRef<string | undefined>(undefined)
  const lastRefreshRevision = useRef<number | undefined>(undefined)
  const lastAutomaticRefreshAt = useRef<number | undefined>(undefined)
  const wasActive = useRef(false)
  const officialCatalogRetryCount = useRef(0)

  const getSelectedRange = useCallback(() => {
    if (rangeMode === "yesterday") return getLocalDayRange(1)
    if (rangeMode === "7") return getLocalRange(7)
    if (rangeMode === "custom") {
      return getLocalDateRange(customStart, customEnd) ?? getLocalRange(1)
    }
    return getLocalRange(1)
  }, [customEnd, customStart, rangeMode])
  const customRangeValid =
    rangeMode !== "custom" || Boolean(getLocalDateRange(customStart, customEnd))
  const getQuery = useCallback(
    () => ({ range: getSelectedRange(), groupBy }),
    [getSelectedRange, groupBy]
  )

  const loadOverview = useCallback(
    async (refresh = false, silent = false) => {
      if (running.current) {
        refreshQueued.current = true
        refreshRequested.current ||= refresh
        return
      }
      running.current = true
      if (!silent) {
        if (refresh) setRefreshing(true)
        else setLoading(true)
      }
      try {
        let result: UsageOverview | undefined
        let shouldRefresh = refresh
        do {
          refreshQueued.current = false
          refreshRequested.current = false
          const version = ++requestVersion.current
          const query = getQuery()
          result = shouldRefresh
            ? await call("refresh_usage", { query })
            : await call("get_usage_overview", { query })
          const latestQuery = getQuery()
          const rangeStillMatches =
            query.groupBy === latestQuery.groupBy &&
            query.range.startAtMs === latestQuery.range.startAtMs &&
            query.range.endAtMs === latestQuery.range.endAtMs
          if (version === requestVersion.current && rangeStillMatches) {
            setOverview(result)
            setError(undefined)
            if (shouldRefresh) {
              refreshCoordinator.invalidate(["dashboard", "providers"])
            }
          }
          shouldRefresh = refreshRequested.current
        } while (refreshQueued.current)
        return result
      } catch (reason) {
        setError(formatError(reason))
        throw reason
      } finally {
        if (!silent) {
          if (refresh) setRefreshing(false)
          else setLoading(false)
        }
        running.current = false
      }
    },
    [getQuery]
  )

  const refreshAutomatically = useCallback(
    async (force = false) => {
      if (
        !force &&
        !shouldRefreshUsageOnEntry(lastAutomaticRefreshAt.current)
      ) {
        return
      }
      lastAutomaticRefreshAt.current = Date.now()
      await loadOverview(true, true)
    },
    [loadOverview]
  )

  const loadRules = useCallback(async () => {
    try {
      setRules(await call("list_pricing_rules", {}))
      setPricingError(undefined)
    } catch (reason) {
      setPricingError(String(reason))
      throw reason
    }
  }, [])

  const loadOfficialCatalog = useCallback(async (refresh = false) => {
    setOfficialCatalogLoading(true)
    try {
      const result = refresh
        ? await call("refresh_official_pricing_catalog")
        : await call("get_official_pricing_catalog")
      setOfficialCatalog(result)
      setOfficialCatalogError(undefined)
      setOfficialCatalogFailed(false)
      officialCatalogRetryCount.current = 0
    } catch (reason) {
      setOfficialCatalogError(String(reason))
      setOfficialCatalogFailed(true)
      setOfficialCatalogRetryRevision((revision) => revision + 1)
    } finally {
      setOfficialCatalogLoading(false)
    }
  }, [])

  useEffect(() => {
    if (
      !active ||
      !foreground ||
      officialCatalogLoading ||
      !officialCatalogFailed
    ) {
      return
    }
    const delay = Math.min(
      30_000 * 2 ** officialCatalogRetryCount.current,
      10 * 60_000
    )
    const timer = window.setTimeout(() => {
      officialCatalogRetryCount.current += 1
      void loadOfficialCatalog(true)
    }, delay)
    return () => window.clearTimeout(timer)
  }, [
    active,
    foreground,
    officialCatalogLoading,
    officialCatalogFailed,
    officialCatalogRetryRevision,
    loadOfficialCatalog,
  ])

  useEffect(() => {
    const entered = active && !wasActive.current
    wasActive.current = active
    if (!active || !foreground || !customRangeValid) return
    const currentQuery = getQuery()
    const queryKey = [
      currentQuery.groupBy,
      currentQuery.range.startAtMs,
      currentQuery.range.endAtMs,
    ].join(":")
    const revisionChanged =
      lastRefreshRevision.current !== refreshSignal.revision
    const queryChanged = lastQueryKey.current !== queryKey
    if (
      !revisionChanged &&
      initialized.current &&
      lastQueryKey.current === queryKey &&
      !entered
    ) {
      return
    }
    lastRefreshRevision.current = refreshSignal.revision
    lastQueryKey.current = queryKey
    if (initialized.current && (entered || revisionChanged || queryChanged)) {
      nextRefreshAt.current = Date.now() + 30_000
    }
    const timeout = window.setTimeout(() => {
      if (!initialized.current) {
        initialized.current = true
        void Promise.all([
          loadOverview().then(() => refreshAutomatically()),
          loadRules(),
          loadOfficialCatalog(true),
        ]).catch(() => undefined)
        return
      }
      void refreshAutomatically(revisionChanged || queryChanged).catch(
        () => undefined
      )
    }, 0)
    return () => window.clearTimeout(timeout)
  }, [
    active,
    customRangeValid,
    foreground,
    getQuery,
    loadOfficialCatalog,
    loadOverview,
    loadRules,
    refreshAutomatically,
    refreshSignal.revision,
  ])

  useEffect(() => {
    if (!active || !foreground || !customRangeValid) return
    let timer: number | undefined
    let cancelled = false
    const stopTimer = () => {
      if (timer !== undefined) {
        window.clearTimeout(timer)
        timer = undefined
      }
    }
    const startTimer = () => {
      if (timer !== undefined) return
      const target = nextRefreshAt.current ?? Date.now() + 30_000
      nextRefreshAt.current = target
      timer = window.setTimeout(
        async () => {
          timer = undefined
          await refreshAutomatically().catch(() => undefined)
          if (cancelled) return
          nextRefreshAt.current = Date.now() + 30_000
          startTimer()
        },
        Math.max(1_000, target - Date.now())
      )
    }
    startTimer()
    return () => {
      cancelled = true
      stopTimer()
    }
  }, [active, customRangeValid, foreground, refreshAutomatically])

  const refresh = async () => {
    try {
      await Promise.all([
        loadOverview(true),
        loadRules(),
        loadOfficialCatalog(true),
      ])
      refreshCoordinator.invalidate(["dashboard", "providers"])
      notify.success("用量已刷新", "已重新扫描本机 Codex rollout 文件。")
    } catch (reason) {
      notify.error("无法刷新本机用量", reason)
    }
  }

  const openShare = async () => {
    setShareOpen(true)
    setShareLoading(true)
    setShareError(undefined)
    try {
      const range = getLocalDayRange()
      const accountQuery = {
        range,
        groupBy: "account" as const,
      }
      const accountOverview = await call("refresh_usage", {
        query: accountQuery,
      })
      const modelOverview = await call("get_usage_overview", {
        query: { range, groupBy: "model" },
      })
      setShareAccountOverview(accountOverview)
      setShareModelOverview(modelOverview)
    } catch (reason) {
      setShareError(String(reason))
    } finally {
      setShareLoading(false)
    }
  }

  const openNewRule = useCallback(
    async (model?: string, providerId?: string) => {
      setPricingError(undefined)
      setPricingFieldErrors({})
      setRepriceAfterSave(true)
      setPricingDraft(createPricingDraft(model, providerId))
      await ensureProviders(setProviders)
    },
    []
  )

  const openEditRule = async (rule: PricingRule) => {
    setPricingError(undefined)
    setPricingFieldErrors({})
    setRepriceAfterSave(true)
    setPricingDraft({ ...rule })
    await ensureProviders(setProviders)
  }

  const saveRule = async () => {
    if (!pricingDraft) return
    const validationError = validatePricingDraft(pricingDraft)
    if (validationError) {
      setPricingError(validationError.message)
      setPricingFieldErrors(
        validationError.field
          ? { [validationError.field]: validationError.message }
          : {}
      )
      return
    }
    setPricingLoading(true)
    try {
      const quickRule: PricingRule = {
        ...pricingDraft,
        scopeKind: "provider_model",
        matchKind: "exact",
        billingMode: "token",
        accountId: undefined,
        requestFeeUsd: undefined,
        // 从重算范围起点生效，保证“保存后重算当前范围”能覆盖
        // 范围内已发生的历史事件，而不是只算保存后的新事件。
        effectiveFromMs: getQuery().range.startAtMs,
        cachedReadUsdPerMillion:
          pricingDraft.cachedReadUsdPerMillion ??
          pricingDraft.inputUsdPerMillion,
        cacheWriteUsdPerMillion:
          pricingDraft.cacheWriteUsdPerMillion ??
          pricingDraft.inputUsdPerMillion,
      }
      const saved = await call("save_pricing_rule", { input: quickRule })
      if (repriceAfterSave) {
        await call("reprice_usage", { range: getQuery().range })
      }
      setPricingDraft(undefined)
      setPricingFieldErrors({})
      await Promise.all([loadRules(), loadOverview()])
      refreshCoordinator.invalidate(["dashboard"])
      notify.success(
        "美元价格规则已保存",
        repriceAfterSave
          ? `已保存并重新估算 ${saved.modelPattern} 的当前范围。`
          : `已保存 ${saved.modelPattern}。`
      )
    } catch (reason) {
      setPricingError(String(reason))
      setPricingFieldErrors({})
      notify.error("无法保存美元价格规则", reason)
    } finally {
      setPricingLoading(false)
    }
  }

  const deleteRule = async () => {
    if (!pendingDelete) return
    setPricingLoading(true)
    try {
      await call("delete_pricing_rule", { id: pendingDelete.id })
      setPendingDelete(undefined)
      await Promise.all([loadRules(), loadOverview()])
      refreshCoordinator.invalidate(["dashboard"])
      notify.success(
        "美元价格规则已停用",
        "更新后新周期内未匹配其他规则的数据会显示为未配置价格。"
      )
    } catch (reason) {
      notify.error("无法停用美元价格规则", reason)
    } finally {
      setPricingLoading(false)
    }
  }

  const currentRange = getSelectedRange()
  const { display: displayOverview, stale: hasStaleOverview } =
    pickDisplayOverview(overview, currentRange, customRangeValid)

  // 失败时（error 已设置）不要停在骨架屏：即使旧数据与当前范围不匹配
  // （stale），也要显示错误提示与重试入口，否则页面会一直转圈且无法恢复。
  if (!displayOverview && !error && (loading || hasStaleOverview))
    return <UsageLoading />

  if (!displayOverview) {
    return (
      <Alert variant="destructive">
        <HugeiconsIcon icon={Alert01Icon} />
        <AlertTitle>无法读取本机 Token 用量</AlertTitle>
        <AlertDescription>
          <ErrorDetails
            error={error ?? "未返回用量数据。"}
            action={
              <Button
                size="sm"
                variant="outline"
                disabled={loading}
                onClick={() => void loadOverview().catch(() => undefined)}
              >
                {loading ? (
                  <Spinner data-icon="inline-start" />
                ) : (
                  <HugeiconsIcon
                    icon={Refresh01Icon}
                    data-icon="inline-start"
                  />
                )}
                {loading ? "正在重试…" : "重试"}
              </Button>
            }
          >
            请确认 Codex 配置目录存在，并允许 Codex Tools 读取本机 rollout
            文件。
          </ErrorDetails>
        </AlertDescription>
      </Alert>
    )
  }

  const attentionTokens =
    displayOverview.totals.unpricedTokens +
    displayOverview.totals.partialTokens +
    displayOverview.totals.unattributedTokens
  const hasWarnings =
    displayOverview.warnings.length > 0 ||
    attentionTokens > 0 ||
    Boolean(pricingError) ||
    Boolean(officialCatalogError)
  const relayRules = rules.filter(
    (rule) =>
      rule.scopeKind === "provider_model" ||
      rule.scopeKind === "provider_default"
  )

  return (
    <div className="flex flex-col gap-6">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <p className="max-w-prose text-sm text-muted-foreground">
          从本机 Codex rollout 日志统计 Token 与美元估算费用；官方账号自动使用
          OpenAI 参考价，API 服务按服务和模型匹配价格。
        </p>
        <div className="flex flex-wrap items-center gap-2">
          <Button
            variant="outline"
            disabled={shareLoading}
            onClick={() => void openShare()}
          >
            {shareLoading ? (
              <Spinner data-icon="inline-start" />
            ) : (
              <HugeiconsIcon icon={Share01Icon} data-icon="inline-start" />
            )}
            分享
          </Button>
          <Button
            variant="outline"
            disabled={refreshing || !customRangeValid}
            onClick={() => void refresh()}
          >
            {refreshing ? (
              <Spinner data-icon="inline-start" />
            ) : (
              <HugeiconsIcon icon={Refresh01Icon} data-icon="inline-start" />
            )}
            {refreshing ? "正在刷新…" : "刷新"}
          </Button>
        </div>
      </div>
      <Tabs defaultValue="details">
        <TabsList>
          <TabsTrigger value="details">用量明细</TabsTrigger>
          <TabsTrigger value="pricing">价格规则</TabsTrigger>
        </TabsList>
        <TabsContent value="details" className="flex flex-col gap-8">
          <Alert>
            <HugeiconsIcon icon={Calendar01Icon} />
            <AlertTitle>当前统计周期</AlertTitle>
            <AlertDescription>
              当前仅统计软件更新后产生的新用量
              {displayOverview.collectionStartedAtMs
                ? ` · ${formatDateTime(displayOverview.collectionStartedAtMs)} 起`
                : ""}
              。更新前的数据不计入当前周期。
            </AlertDescription>
          </Alert>

          {error && (
            <Alert variant="destructive">
              <HugeiconsIcon icon={Alert01Icon} />
              <AlertTitle>当前显示的是上次读取结果</AlertTitle>
              <AlertDescription>
                <ErrorDetails error={error}>
                  暂时无法读取最新用量，请稍后刷新。
                </ErrorDetails>
              </AlertDescription>
            </Alert>
          )}

          <Card>
            <CardHeader className="gap-3 sm:flex-row sm:flex-wrap sm:items-center sm:justify-between">
              <div>
                <CardTitle>统计范围</CardTitle>
                <CardDescription>
                  选择时间范围与汇总维度；自定义日期按本机时区计算，最多 366
                  天。
                </CardDescription>
              </div>
              <CardAction className="flex flex-wrap items-center gap-2">
                <ToggleGroup
                  variant="outline"
                  spacing={0}
                  size="sm"
                  value={[rangeMode]}
                  onValueChange={(value, eventDetails) => {
                    const next = value[0] as RangeMode | undefined
                    if (next) setRangeMode(next)
                    else eventDetails.isCanceled = true
                  }}
                  aria-label="用量时间范围"
                >
                  {RANGE_OPTIONS.map((option) => (
                    <ToggleGroupItem key={option.key} value={option.key}>
                      {option.label}
                    </ToggleGroupItem>
                  ))}
                </ToggleGroup>
                <ToggleGroup
                  variant="outline"
                  spacing={0}
                  size="sm"
                  value={[groupBy]}
                  onValueChange={(value, eventDetails) => {
                    const next = value[0] as UsageGroupBy | undefined
                    if (next) setGroupBy(next)
                    else eventDetails.isCanceled = true
                  }}
                  aria-label="用量汇总维度"
                >
                  <ToggleGroupItem value="model">按模型</ToggleGroupItem>
                  <ToggleGroupItem value="account">按账号</ToggleGroupItem>
                </ToggleGroup>
              </CardAction>
            </CardHeader>
            {rangeMode === "custom" && (
              <div className="flex flex-wrap items-end gap-3 border-t px-6 pt-4">
                <Field className="w-auto min-w-36">
                  <FieldLabel htmlFor="usage-range-start">开始日期</FieldLabel>
                  <Input
                    id="usage-range-start"
                    type="date"
                    value={customStart}
                    onChange={(event) => setCustomStart(event.target.value)}
                  />
                </Field>
                <Field className="w-auto min-w-36">
                  <FieldLabel htmlFor="usage-range-end">
                    结束日期（含当天）
                  </FieldLabel>
                  <Input
                    id="usage-range-end"
                    type="date"
                    value={customEnd}
                    onChange={(event) => setCustomEnd(event.target.value)}
                  />
                </Field>
                <span className="pb-1 text-xs text-muted-foreground">
                  最多查询 366 天；时间按本机时区计算。
                </span>
              </div>
            )}
            {!customRangeValid && (
              <p className="px-6 pt-3 text-sm text-destructive">
                自定义日期无效：结束日期必须不早于开始日期。
              </p>
            )}
            <CardContent className="flex flex-col gap-6">
              <div className="grid grid-cols-2 gap-5">
                <StatCard
                  label="总 Token"
                  value={formatTokens(
                    displayOverview.totals.tokens.totalTokens
                  )}
                  detail={`${formatTokenDetail(displayOverview.totals.tokens)} · ${displayOverview.totals.requests} 次调用`}
                />
                <StatCard
                  label="模型调用"
                  value={String(displayOverview.totals.requests)}
                  detail={`统计范围 ${formatRangeLabel(displayOverview.range)}`}
                />
                <StatCard
                  label="估算费用"
                  value={formatEstimatedUsd(
                    displayOverview.totals.estimatedCostMicrousd,
                    displayOverview.totals.unpricedTokens +
                      displayOverview.totals.partialTokens +
                      displayOverview.totals.unattributedTokens +
                      displayOverview.totals.subscriptionTokens
                  )}
                  detail="官方金额为参考估算，不代表套餐实际账单"
                />
                <StatCard
                  label="需处理 Token"
                  value={formatTokens(attentionTokens)}
                  detail="未配置价格、数据不完整或未归属"
                />
              </div>
              <div className="flex flex-col gap-2">
                <div className="text-sm text-muted-foreground">每日趋势</div>
                <UsageTrendChart
                  range={displayOverview.range}
                  refreshKey={refreshSignal.revision}
                  points={displayOverview.trendPoints}
                />
              </div>
              <div className="flex flex-wrap items-center justify-between gap-2 text-xs text-muted-foreground">
                <span className="inline-flex items-center gap-1.5">
                  <HugeiconsIcon
                    icon={Calendar01Icon}
                    size={14}
                    aria-hidden="true"
                  />
                  {formatRangeLabel(displayOverview.range)} · 上次刷新{" "}
                  {formatDateTime(displayOverview.lastRefreshedAtMs)}
                </span>
                <span>
                  {formatTimezone()} · 页面打开时自动刷新，之后每 30 秒扫描一次
                </span>
              </div>
            </CardContent>
          </Card>

          {hasWarnings && (
            <Alert variant={attentionTokens > 0 ? "default" : "destructive"}>
              <HugeiconsIcon icon={Alert01Icon} />
              <AlertTitle>部分用量需要处理</AlertTitle>
              <AlertDescription>
                <div className="flex flex-col gap-1.5">
                  {displayOverview.totals.unpricedTokens > 0 && (
                    <span>
                      {formatTokens(displayOverview.totals.unpricedTokens)}{" "}
                      Token 没有可用价格；可在下方添加 USD 规则后重新计算。
                    </span>
                  )}
                  {displayOverview.totals.unattributedTokens > 0 && (
                    <span>
                      {formatTokens(displayOverview.totals.unattributedTokens)}{" "}
                      Token 无法确认对应来源；程序不会强行归到当前账号。
                    </span>
                  )}
                  {displayOverview.totals.partialTokens > 0 && (
                    <span>
                      {formatTokens(displayOverview.totals.partialTokens)} Token
                      的原始日志字段不完整，费用按保守规则处理。
                    </span>
                  )}
                  {pricingError && (
                    <span>价格规则读取失败：{pricingError}</span>
                  )}
                  {officialCatalogError && (
                    <span>官方价格同步失败：{officialCatalogError}</span>
                  )}
                  {displayOverview.warnings.length > 0 && (
                    <ul className="list-disc pl-5">
                      {displayOverview.warnings
                        .slice(0, 5)
                        .map((warning, index) => (
                          <li key={`${warning.path ?? "warning"}-${index}`}>
                            {warning.message}
                          </li>
                        ))}
                    </ul>
                  )}
                </div>
              </AlertDescription>
            </Alert>
          )}

          <Card>
            <CardHeader>
              <CardTitle>模型与账号明细</CardTitle>
              <CardDescription>
                输入、缓存读取、缓存写入和输出分开统计；汇总维度在上方切换。
              </CardDescription>
            </CardHeader>
            <CardContent>
              {displayOverview.rows.length === 0 ? (
                <Empty className="min-h-48 border">
                  <EmptyHeader>
                    <EmptyMedia variant="icon">
                      <HugeiconsIcon icon={BookOpen01Icon} />
                    </EmptyMedia>
                    <EmptyTitle>这个时间范围还没有 Token 记录</EmptyTitle>
                    <EmptyDescription>
                      先在 Codex 中发起一次请求，再点击“刷新”；官方账号和 API
                      服务都会记录在本机数据库中。
                    </EmptyDescription>
                  </EmptyHeader>
                  <EmptyContent>
                    <Button
                      size="sm"
                      onClick={() => void refresh()}
                      disabled={refreshing}
                    >
                      {refreshing ? (
                        <Spinner data-icon="inline-start" />
                      ) : (
                        <HugeiconsIcon
                          icon={Refresh01Icon}
                          data-icon="inline-start"
                        />
                      )}
                      扫描本机日志
                    </Button>
                  </EmptyContent>
                </Empty>
              ) : (
                <>
                  <div className="hidden sm:block">
                    <Table className="min-w-[44rem]">
                      <TableHeader>
                        <TableRow>
                          <TableHead>
                            {groupBy === "model"
                              ? "模型 / 来源"
                              : "账号 / 模型"}
                          </TableHead>
                          <TableHead className="px-2 text-right">
                            调用
                          </TableHead>
                          <TableHead className="px-2 text-right">
                            总输入
                          </TableHead>
                          <TableHead className="hidden px-2 text-right xl:table-cell">
                            缓存读取
                          </TableHead>
                          <TableHead className="hidden px-2 text-right xl:table-cell">
                            缓存写入
                          </TableHead>
                          <TableHead className="px-2 text-right">
                            输出
                          </TableHead>
                          <TableHead className="px-2 text-right">
                            总 Token
                          </TableHead>
                          <TableHead className="px-2 text-right">
                            费用
                          </TableHead>
                          <TableHead className="px-2">状态</TableHead>
                          <TableHead className="px-2 text-right">
                            操作
                          </TableHead>
                        </TableRow>
                      </TableHeader>
                      <TableBody>
                        {displayOverview.rows.map((row) => (
                          <UsageTableRow
                            key={row.key}
                            row={row}
                            onSelect={setSelectedRow}
                            onOpenNewRule={openNewRule}
                          />
                        ))}
                      </TableBody>
                    </Table>
                  </div>
                  <div className="flex flex-col gap-2 sm:hidden">
                    {displayOverview.rows.map((row) => (
                      <UsageRowCard
                        key={row.key}
                        row={row}
                        onSelect={setSelectedRow}
                      />
                    ))}
                  </div>
                </>
              )}
            </CardContent>
          </Card>
        </TabsContent>
        <TabsContent value="pricing" className="flex flex-col gap-8">
          <Card>
            <CardHeader className="gap-3 sm:flex-row sm:items-center sm:justify-between">
              <div>
                <CardTitle>OpenAI 官方实时价格</CardTitle>
                <CardDescription>
                  价格从官方 Markdown 运行时同步，不写死模型；API
                  服务价格规则完全独立。
                </CardDescription>
              </div>
              <Button
                size="sm"
                variant="outline"
                className="min-w-28"
                disabled={officialCatalogLoading}
                onClick={() => void loadOfficialCatalog(true)}
              >
                {officialCatalogLoading ? (
                  <Spinner data-icon="inline-start" />
                ) : (
                  <HugeiconsIcon
                    icon={Refresh01Icon}
                    data-icon="inline-start"
                  />
                )}
                {officialCatalogLoading ? "正在同步…" : "立即同步"}
              </Button>
            </CardHeader>
            <CardContent className="flex flex-wrap items-center gap-3 text-sm">
              <Badge
                variant={
                  officialCatalog?.status === "cached" ? "secondary" : "outline"
                }
              >
                {officialCatalog?.status === "cached" ? "已缓存" : "等待同步"}
              </Badge>
              <span className="text-muted-foreground">
                {officialCatalog?.modelCount ?? 0} 个官方模型
              </span>
              {officialCatalog?.fetchedAtMs && (
                <span className="text-muted-foreground">
                  最近同步：{formatDateTime(officialCatalog.fetchedAtMs)}
                </span>
              )}
              <Button
                variant="link"
                size="sm"
                className="h-auto px-0"
                onClick={() => setPricingDialogOpen(true)}
              >
                查看官方价格
              </Button>
            </CardContent>
          </Card>

          <Card>
            <CardHeader>
              <CardTitle>API 服务价格</CardTitle>
              <CardDescription>
                官方 OpenAI 自动使用内置参考价；API
                服务只需设置服务和模型的输入、输出价格。
              </CardDescription>
              <CardAction>
                <div className="flex flex-wrap gap-2">
                  <Button size="sm" onClick={() => void openNewRule()}>
                    <HugeiconsIcon
                      icon={Settings02Icon}
                      data-icon="inline-start"
                    />
                    添加价格规则
                  </Button>
                </div>
              </CardAction>
            </CardHeader>
            <CardContent>
              <Alert className="mb-4">
                <HugeiconsIcon icon={SecurityCheckIcon} aria-hidden="true" />
                <AlertTitle className="flex flex-wrap items-center gap-2">
                  OpenAI 官方实时价格
                  <Badge variant="outline">只读</Badge>
                  <Badge variant="secondary">自动启用</Badge>
                </AlertTitle>
                <AlertDescription>
                  官方 OpenAI / Cookie 账号无需单独添加规则；金额按官方 API
                  价格换算，仅作为参考估算，不代表套餐实际账单。
                </AlertDescription>
              </Alert>
              {relayRules.length === 0 ? (
                <Empty className="min-h-32 border">
                  <EmptyHeader>
                    <EmptyMedia variant="icon">
                      <HugeiconsIcon icon={Dollar01Icon} />
                    </EmptyMedia>
                    <EmptyTitle>还没有价格规则</EmptyTitle>
                    <EmptyDescription>
                      官方 OpenAI 账号自动使用内置参考价；API 服务要显示费用，
                      请为模型配置价格。
                    </EmptyDescription>
                  </EmptyHeader>
                  <EmptyContent>
                    <Button
                      size="sm"
                      variant="outline"
                      onClick={() => void openNewRule()}
                    >
                      添加第一条价格规则
                    </Button>
                  </EmptyContent>
                </Empty>
              ) : (
                <ItemGroup className="gap-3">
                  {relayRules.map((rule) => (
                    <Item
                      key={rule.id}
                      variant="outline"
                      className="flex-wrap items-center justify-between p-4"
                    >
                      <ItemContent className="min-w-0">
                        <div className="flex flex-wrap items-center gap-2">
                          <ItemTitle>{rule.modelPattern}</ItemTitle>
                          <Badge variant="outline">{scopeLabel(rule)}</Badge>
                          <Badge variant="secondary">
                            {billingLabel(rule.billingMode)}
                          </Badge>
                        </div>
                        <ItemDescription className="line-clamp-none text-xs">
                          {priceSummary(rule)}
                        </ItemDescription>
                      </ItemContent>
                      <ItemActions className="shrink-0">
                        <Button
                          size="sm"
                          variant="outline"
                          onClick={() => void openEditRule(rule)}
                        >
                          编辑
                        </Button>
                        <Button
                          size="sm"
                          variant="ghost"
                          disabled={pricingLoading}
                          onClick={() => setPendingDelete(rule)}
                        >
                          停用
                        </Button>
                      </ItemActions>
                    </Item>
                  ))}
                </ItemGroup>
              )}
            </CardContent>
          </Card>
        </TabsContent>
      </Tabs>

      <PricingRuleDialog
        open={Boolean(pricingDraft)}
        draft={pricingDraft}
        providers={providers}
        pending={pricingLoading}
        error={pricingError}
        fieldErrors={pricingFieldErrors}
        repriceAfterSave={repriceAfterSave}
        onOpenChange={(open) => {
          if (!open && !pricingLoading) setPricingDraft(undefined)
        }}
        onRepriceChange={setRepriceAfterSave}
        onChange={(rule) => {
          setPricingDraft(rule)
          setPricingError(undefined)
          setPricingFieldErrors({})
        }}
        onSave={() => void saveRule()}
      />

      <OfficialPricingDialog
        open={pricingDialogOpen}
        onOpenChange={setPricingDialogOpen}
        catalog={officialCatalog}
        loading={officialCatalogLoading}
        error={officialCatalogError}
        onRefresh={() => void loadOfficialCatalog(true)}
      />

      <UsageShareDialog
        open={shareOpen}
        onOpenChange={setShareOpen}
        accountOverview={shareAccountOverview}
        modelOverview={shareModelOverview}
        dateLabel={new Date().toLocaleDateString("zh-CN", {
          year: "numeric",
          month: "long",
          day: "numeric",
        })}
        timezone={formatTimezone()}
        loading={shareLoading}
        error={shareError}
      />

      <Sheet
        open={Boolean(selectedRow)}
        onOpenChange={(open) => {
          if (!open) setSelectedRow(undefined)
        }}
      >
        <SheetContent
          side="right"
          className="overflow-y-auto data-[side=right]:w-full sm:max-w-lg"
        >
          {selectedRow && (
            <>
              <SheetHeader>
                <SheetTitle>{selectedRow.model}</SheetTitle>
                <SheetDescription>
                  {selectedRow.sourceName} · {selectedRow.requests} 次调用 ·
                  点击表格行打开详情
                </SheetDescription>
              </SheetHeader>
              <div className="flex flex-col gap-5 px-4 pb-6">
                <div className="grid grid-cols-2 gap-2">
                  <DetailMetric
                    label="总 Token"
                    value={formatTokens(selectedRow.tokens.totalTokens)}
                  />
                  <DetailMetric
                    label="费用"
                    value={formatRowCost(
                      selectedRow.costStatus,
                      selectedRow.estimatedCostMicrousd
                    )}
                  />
                  <DetailMetric
                    label="输入"
                    value={formatTokens(selectedRow.tokens.inputTokens)}
                  />
                  <DetailMetric
                    label="缓存读取"
                    value={formatTokens(selectedRow.tokens.cachedInputTokens)}
                  />
                  <DetailMetric
                    label="缓存写入"
                    value={formatTokens(
                      selectedRow.tokens.cacheWriteInputTokens
                    )}
                  />
                  <DetailMetric
                    label="输出"
                    value={formatTokens(selectedRow.tokens.outputTokens)}
                  />
                  <DetailMetric
                    label="推理输出"
                    value={formatTokens(
                      selectedRow.tokens.reasoningOutputTokens
                    )}
                  />
                  <DetailMetric
                    label="状态"
                    value={displayCostStatus(selectedRow)}
                  />
                </div>
                <Card size="sm">
                  <CardContent className="flex flex-col gap-1">
                    <CardTitle className="text-sm font-medium">
                      费用说明
                    </CardTitle>
                    <p className="text-sm leading-relaxed text-muted-foreground">
                      {detailCostMessage(selectedRow)}
                    </p>
                  </CardContent>
                </Card>
                <div className="flex flex-col gap-2 text-sm">
                  <DetailLine
                    label="来源类型"
                    value={sourceLabel(selectedRow)}
                  />
                  <DetailLine
                    label="来源账号 / 服务"
                    value={selectedRow.sourceName}
                  />
                  <DetailLine
                    label="匹配价格规则"
                    value={
                      selectedRow.pricingRuleName
                        ? `${selectedRow.pricingRuleName}${selectedRow.pricingRuleVersion ? ` · v${selectedRow.pricingRuleVersion}` : ""}`
                        : "未匹配"
                    }
                  />
                  <DetailLine
                    label="Provider ID"
                    value={selectedRow.providerId ?? "—"}
                    mono
                  />
                  <DetailLine
                    label="账号 ID"
                    value={selectedRow.accountId ?? "—"}
                    mono
                  />
                </div>
                {selectedRow.costStatus === "unattributed" && (
                  <Alert>
                    <HugeiconsIcon icon={Alert01Icon} />
                    <AlertTitle>未归属记录</AlertTitle>
                    <AlertDescription>
                      这条日志发生时没有可确认的账号激活时间线，程序不会把它强行归到当前账号。
                    </AlertDescription>
                  </Alert>
                )}
              </div>
            </>
          )}
        </SheetContent>
      </Sheet>

      <AlertDialog
        open={Boolean(pendingDelete)}
        onOpenChange={(open) => {
          if (!open && !pricingLoading) setPendingDelete(undefined)
        }}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>停用这条价格规则？</AlertDialogTitle>
            <AlertDialogDescription>
              停用后不会删除本机账本记录；后续新周期重算时，未匹配其他规则的数据将显示为“未配置价格”。
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel disabled={pricingLoading}>
              取消
            </AlertDialogCancel>
            <AlertDialogAction
              disabled={pricingLoading}
              onClick={() => void deleteRule()}
            >
              {pricingLoading ? <Spinner data-icon="inline-start" /> : null}
              停用规则
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  )
}

function createPricingDraft(model?: string, providerId?: string): PricingRule {
  return {
    id: "",
    version: 0,
    active: true,
    scopeKind: "provider_model",
    providerId,
    modelPattern: model?.trim() || "gpt-5.6-sol",
    matchKind: "exact",
    billingMode: "token",
    inputUsdPerMillion: undefined,
    cachedReadUsdPerMillion: undefined,
    cacheWriteUsdPerMillion: undefined,
    outputUsdPerMillion: undefined,
    requestFeeUsd: undefined,
    cacheWriteIncludedInInput: false,
    effectiveFromMs: Date.now(),
    createdAtMs: 0,
    updatedAtMs: 0,
  }
}

function validatePricingDraft(
  rule: PricingRule
): { message: string; field?: PricingField } | undefined {
  if (!rule.modelPattern.trim()) {
    return { message: "请填写模型匹配规则。", field: "model" }
  }
  if (!rule.providerId) {
    return { message: "请选择 API 服务。", field: "provider" }
  }
  const prices = [
    rule.inputUsdPerMillion,
    rule.cachedReadUsdPerMillion,
    rule.cacheWriteUsdPerMillion,
    rule.outputUsdPerMillion,
    rule.requestFeeUsd,
  ]
  if (
    prices.some(
      (value) => value !== undefined && !/^\d+(\.\d{1,6})?$/.test(value.trim())
    )
  ) {
    return {
      message: "USD 价格必须是非负十进制数字，最多 6 位小数。",
      field: "prices",
    }
  }
  if (!rule.inputUsdPerMillion && !rule.outputUsdPerMillion) {
    return {
      message: "请至少填写输入或输出价格。",
      field: "prices",
    }
  }
  return undefined
}

async function ensureProviders(
  setProviders: (value: ProviderOverview) => void
) {
  try {
    setProviders(await call("get_provider_overview"))
  } catch {
    // The pricing dialog remains usable for global model rules when provider data is unavailable.
  }
}

// 表格行与移动端卡片是刷新时整表重建的重渲染热点；
// 通过 memo 让行只在 row 引用变化（数据刷新）时重渲染，
// 而不是在 refreshing / shareOpen / pricingLoading 等状态切换时全部重建。
const UsageTableRow = memo(function UsageTableRow({
  row,
  onSelect,
  onOpenNewRule,
}: {
  row: UsageRow
  onSelect: (row: UsageRow) => void
  onOpenNewRule: (model: string, providerId?: string) => void
}) {
  return (
    <TableRow
      className="cursor-pointer"
      role="button"
      tabIndex={0}
      aria-label={`查看 ${row.model} 的用量详情`}
      onClick={() => onSelect(row)}
      onKeyDown={(event) => {
        if (event.key === "Enter" || event.key === " ") {
          event.preventDefault()
          onSelect(row)
        }
      }}
    >
      <TableCell>
        <div className="max-w-56 truncate font-medium" title={row.model}>
          {row.model}
        </div>
        <div className="mt-1 flex max-w-56 items-center gap-1.5">
          <Badge
            variant={
              row.sourceKind === "official"
                ? "secondary"
                : row.sourceKind === "provider"
                  ? "outline"
                  : "destructive"
            }
          >
            {sourceLabel(row)}
          </Badge>
          <span
            className="truncate text-xs text-muted-foreground"
            title={row.sourceName}
          >
            {row.sourceName}
          </span>
        </div>
      </TableCell>
      <TableCell className="px-2 text-right tabular-nums">
        {row.requests}
      </TableCell>
      <TableCell className="px-2 text-right tabular-nums">
        {formatTokens(row.tokens.inputTokens)}
      </TableCell>
      <TableCell className="hidden px-2 text-right tabular-nums xl:table-cell">
        {formatTokens(row.tokens.cachedInputTokens)}
      </TableCell>
      <TableCell className="hidden px-2 text-right tabular-nums xl:table-cell">
        {formatTokens(row.tokens.cacheWriteInputTokens)}
      </TableCell>
      <TableCell className="px-2 text-right tabular-nums">
        {formatTokens(row.tokens.outputTokens)}
      </TableCell>
      <TableCell className="px-2 text-right font-medium tabular-nums">
        {formatTokens(row.tokens.totalTokens)}
      </TableCell>
      <TableCell className="px-2 text-right font-medium tabular-nums">
        {formatRowCost(row.costStatus, row.estimatedCostMicrousd)}
      </TableCell>
      <TableCell className="px-2">
        <Badge variant={statusVariant(row.costStatus)}>
          {displayCostStatus(row)}
        </Badge>
      </TableCell>
      <TableCell className="px-2 text-right">
        {row.sourceKind === "provider" && row.costStatus === "unpriced" ? (
          <Button
            size="sm"
            variant="outline"
            onClick={(event) => {
              event.stopPropagation()
              void onOpenNewRule(row.model, row.providerId)
            }}
          >
            设置价格
          </Button>
        ) : (
          <span className="text-xs text-muted-foreground">查看</span>
        )}
      </TableCell>
    </TableRow>
  )
})

const UsageRowCard = memo(function UsageRowCard({
  row,
  onSelect,
}: {
  row: UsageRow
  onSelect: (row: UsageRow) => void
}) {
  return (
    <button
      type="button"
      className="flex w-full flex-col gap-3 rounded-xl border bg-card p-3 text-left transition-colors hover:bg-muted/50 focus-visible:border-ring focus-visible:ring-3 focus-visible:ring-ring/50"
      onClick={() => onSelect(row)}
    >
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <p className="truncate font-medium" title={row.model}>
            {row.model}
          </p>
          <div className="mt-1 flex items-center gap-1.5">
            <Badge variant="outline">{sourceLabel(row)}</Badge>
            <p
              className="truncate text-xs text-muted-foreground"
              title={row.sourceName}
            >
              {row.sourceName}
            </p>
          </div>
        </div>
        <div className="flex shrink-0 flex-col items-end gap-1">
          <span className="font-medium tabular-nums">
            {formatRowCost(row.costStatus, row.estimatedCostMicrousd)}
          </span>
          <Badge variant={statusVariant(row.costStatus)}>
            {displayCostStatus(row)}
          </Badge>
        </div>
      </div>
      <div className="grid grid-cols-2 gap-x-4 gap-y-1 text-xs text-muted-foreground">
        <span>调用 {row.requests}</span>
        <span className="text-right">
          总 Token {formatTokens(row.tokens.totalTokens)}
        </span>
        <span>输入 {formatTokens(row.tokens.inputTokens)}</span>
        <span className="text-right">
          输出 {formatTokens(row.tokens.outputTokens)}
        </span>
        <span>缓存读取 {formatTokens(row.tokens.cachedInputTokens)}</span>
        <span className="text-right">
          缓存写入 {formatTokens(row.tokens.cacheWriteInputTokens)}
        </span>
      </div>
    </button>
  )
})

function StatCard({
  label,
  value,
  detail,
}: {
  label: string
  value: string
  detail?: string
}) {
  return (
    <Card>
      <CardHeader>
        <CardDescription>{label}</CardDescription>
        <CardTitle className="text-2xl font-semibold tabular-nums">
          {value}
        </CardTitle>
      </CardHeader>
      {detail && (
        <CardFooter className="text-sm text-muted-foreground">
          {detail}
        </CardFooter>
      )}
    </Card>
  )
}

function DetailMetric({ label, value }: { label: string; value: string }) {
  return (
    <Card size="sm">
      <CardContent className="flex flex-col gap-1">
        <p className="text-xs text-muted-foreground">{label}</p>
        <p className="font-medium tabular-nums">{value}</p>
      </CardContent>
    </Card>
  )
}

function DetailLine({
  label,
  value,
  mono = false,
}: {
  label: string
  value: string
  mono?: boolean
}) {
  return (
    <div className="flex items-start justify-between gap-4 border-b pb-2 last:border-b-0 last:pb-0">
      <span className="shrink-0 text-muted-foreground">{label}</span>
      <span
        className={cn(
          "min-w-0 text-right break-all",
          mono && "font-mono text-xs"
        )}
      >
        {value}
      </span>
    </div>
  )
}

function sourceLabel(row: UsageRow) {
  if (row.sourceKind === "official") return "官方 OpenAI"
  if (row.sourceKind === "provider") return "API 服务"
  return "未识别"
}

function displayCostStatus(row: UsageRow) {
  if (
    row.sourceKind === "official" &&
    row.costStatus === "estimated" &&
    row.pricingRuleName === "OpenAI 官方参考价"
  ) {
    return "官方参考价"
  }
  return formatCostStatus(row.costStatus)
}

function detailCostMessage(row: UsageRow) {
  if (
    row.sourceKind === "official" &&
    row.pricingRuleName === "OpenAI 官方参考价" &&
    row.estimatedCostMicrousd !== undefined
  ) {
    return `按 OpenAI 官方参考价分别计算普通输入、缓存读取、缓存写入和输出，当前参考估算为 ${formatUsdMicrousd(row.estimatedCostMicrousd)}；这不是官方套餐实际账单。`
  }
  if (row.costStatus === "estimated") {
    return `按规则“${row.pricingRuleName ?? "未命名规则"}”分别计算普通输入、缓存读取、缓存写入、输出和请求固定费，当前估算为 ${formatUsdMicrousd(row.estimatedCostMicrousd ?? 0)}。`
  }
  if (row.costStatus === "subscription")
    return "此来源按套餐统计，只显示 Token，不把套餐用量伪造成美元账单。"
  if (row.costStatus === "partial")
    return "原始日志有字段缺失，Token 已保留，但费用只能标记为部分数据。"
  if (row.costStatus === "unattributed")
    return "没有可靠的账号激活记录，费用不会错误归到当前账号。"
  return "没有匹配的完整 USD 价格规则；添加规则后可重新计算当前时间范围。"
}

function formatRowCost(status: CostStatus, value?: number) {
  if (status === "subscription") return "套餐"
  if (
    status === "unpriced" ||
    status === "unattributed" ||
    (status === "partial" && value === undefined)
  )
    return "—"
  if (status === "partial") return `约 ${formatUsdMicrousd(value)}`
  return formatUsdMicrousd(value ?? 0)
}

function statusVariant(
  status: CostStatus
): "default" | "secondary" | "destructive" | "outline" {
  if (status === "estimated" || status === "zero") return "default"
  if (status === "subscription") return "secondary"
  if (status === "unpriced" || status === "unattributed") return "destructive"
  return "outline"
}

function scopeLabel(rule: PricingRule) {
  if (rule.scopeKind === "global_model") return "全局模型"
  if (rule.scopeKind === "provider_default")
    return `API 服务默认 · ${rule.providerId ?? "未知"}`
  if (rule.scopeKind === "provider_model")
    return `API 服务模型 · ${rule.providerId ?? "未知"}`
  return `账号模型 · ${rule.accountId ?? "未知"}`
}

function billingLabel(mode: PricingRule["billingMode"]) {
  if (mode === "subscription") return "套餐统计"
  if (mode === "unpriced") return "不估算"
  return "按 Token"
}

function priceSummary(rule: PricingRule) {
  if (rule.billingMode !== "token") return "只统计 Token，不生成美元估算。"
  const parts = [
    `输入 ${rule.inputUsdPerMillion ?? "—"}`,
    `缓存读 ${rule.cachedReadUsdPerMillion ?? "—"}`,
    `缓存写 ${rule.cacheWriteUsdPerMillion ?? "—"}`,
    `输出 ${rule.outputUsdPerMillion ?? "—"}`,
  ]
  return `${parts.join(" · ")} USD / 1M${rule.requestFeeUsd ? ` · 请求费 $${rule.requestFeeUsd}` : ""}`
}

function UsageLoading() {
  return (
    <div className="flex flex-col gap-6" role="status" aria-live="polite">
      <div className="grid grid-cols-2 gap-4">
        {Array.from({ length: 4 }).map((_, index) => (
          <div key={index} className="flex flex-col gap-2">
            <Skeleton className="h-4 w-24" />
            <Skeleton className="h-8 w-16" />
            <Skeleton className="h-3 w-32" />
          </div>
        ))}
      </div>
      <Skeleton className="h-40 w-full" />
      <Skeleton className="h-64 w-full" />
    </div>
  )
}
