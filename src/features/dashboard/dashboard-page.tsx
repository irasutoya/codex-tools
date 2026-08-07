import { useCallback, useEffect, useRef, useState } from "react"
import {
  Alert01Icon,
  ArrowRightIcon,
  BoxIcon,
  ExternalLinkIcon,
  File01Icon,
  Key01Icon,
  Message01Icon,
  Refresh01Icon,
  Shield01Icon,
} from "@hugeicons/core-free-icons"
import { HugeiconsIcon } from "@hugeicons/react"
import { ErrorDetails } from "@/components/error-details"
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card"
import {
  Item,
  ItemActions,
  ItemContent,
  ItemDescription,
  ItemGroup,
  ItemMedia,
  ItemSeparator,
  ItemTitle,
} from "@/components/ui/item"
import { Skeleton } from "@/components/ui/skeleton"
import { Spinner } from "@/components/ui/spinner"
import { notify, formatError } from "@/lib/feedback"
import { call } from "@/lib/ipc"
import {
  refreshCoordinator,
  useAppForeground,
  usePageRefresh,
} from "@/lib/refresh-coordinator"
import {
  runQuotaRefresh,
  useAutoQuotaRefresh,
} from "@/lib/use-auto-quota-refresh"
import type { Dashboard, PageProps } from "@/types"

import { QuotaStatusView } from "../providers/quota-status"
import { UsageTrendChart } from "../usage/usage-trend-chart"
import {
  formatEstimatedUsd,
  formatTokens,
  getLocalDayRange,
  getLocalRange,
} from "../usage/usage-format"

export default function DashboardPage({ active }: PageProps) {
  const refreshSignal = usePageRefresh("dashboard")
  const foreground = useAppForeground()
  const [data, setData] = useState<Dashboard>()
  const [launching, setLaunching] = useState(false)
  const [refreshing, setRefreshing] = useState(false)
  const [quotaRefreshing, setQuotaRefreshing] = useState(false)
  const [error, setError] = useState<string>()
  const lastRefreshRevision = useRef<number | undefined>(undefined)
  const initialized = useRef(false)

  const loadDashboard = useCallback(async () => {
    try {
      const dashboard = await call("get_dashboard")
      setData(dashboard)
      setError(undefined)
    } catch (reason) {
      setError(formatError(reason))
      throw reason
    }
  }, [])

  const load = useCallback(async () => {
    await loadDashboard()
    void call("refresh_usage", {
      query: { range: getLocalRange(1), groupBy: "account" },
    })
      .then(() => loadDashboard())
      .then(() => refreshCoordinator.invalidate(["providers", "usage"]))
      .catch(() => undefined)
  }, [loadDashboard])

  const activeAccountId =
    data?.activeKind === "official" ? data.activeAccountId : undefined
  const refreshQuotaData = useCallback(async () => {
    if (!activeAccountId) throw new Error("当前没有可刷新的 OpenAI 账号")
    const result = await call("refresh_official_account_quota", {
      accountId: activeAccountId,
    })
    if (!refreshCoordinator.getForeground()) {
      refreshCoordinator.invalidate(["dashboard", "providers"])
      return result
    }
    await loadDashboard()
    refreshCoordinator.invalidate(["providers"])
    return result
  }, [activeAccountId, loadDashboard])

  useEffect(() => {
    if (!active) return
    const firstLoad = !initialized.current
    if (
      !firstLoad &&
      (!foreground || lastRefreshRevision.current === refreshSignal.revision)
    ) {
      return
    }
    const timeout = window.setTimeout(() => {
      // StrictMode 双挂载时，若在 effect 体内同步写入 revision ref，
      // 首次加载会被第二次挂载的守卫吞掉（load 永不执行）。
      // 因此在回调中再写入，保证首次加载一定执行。
      initialized.current = true
      lastRefreshRevision.current = refreshSignal.revision
      void load().catch(() => undefined)
    }, 0)
    return () => window.clearTimeout(timeout)
  }, [active, foreground, load, refreshSignal.revision])

  useAutoQuotaRefresh({
    accountId: activeAccountId,
    active,
    foreground,
    quota: data?.activeQuota,
    refresh: refreshQuotaData,
  })

  const launchCodex = async () => {
    setLaunching(true)
    try {
      const result = await call("launch_codex")
      notify.success(
        result.injected ? "Codex 已启动并解锁模型列表" : result.message
      )
      refreshCoordinator.invalidate(["settings"])
    } catch (reason) {
      notify.error("无法启动 Codex", reason)
    } finally {
      setLaunching(false)
    }
  }

  const refreshQuota = async () => {
    if (!activeAccountId) return true
    setQuotaRefreshing(true)
    try {
      const quota = await runQuotaRefresh(activeAccountId, refreshQuotaData)
      if (quota.status === "success") {
        notify.success("当前账号额度已更新")
        return true
      } else {
        notify.warning(
          "当前账号额度未更新",
          quota.error ?? "OpenAI 暂未返回额度。"
        )
        return false
      }
    } catch (reason) {
      notify.error("无法刷新当前账号额度", reason)
      return false
    } finally {
      setQuotaRefreshing(false)
    }
  }

  const refresh = async () => {
    setRefreshing(true)
    try {
      await load()
      const quotaUpdated = await refreshQuota()
      notify.success(quotaUpdated ? "状态和额度已更新" : "状态已更新")
    } catch (reason) {
      notify.error("无法更新状态", reason)
    } finally {
      setRefreshing(false)
    }
  }

  if (!data) {
    if (!error) return <DashboardLoading />
    return (
      <Alert variant="destructive">
        <HugeiconsIcon icon={Alert01Icon} />
        <AlertTitle>无法读取 Codex 状态</AlertTitle>
        <AlertDescription>
          <ErrorDetails
            error={error}
            action={
              <Button
                size="sm"
                variant="outline"
                disabled={refreshing}
                onClick={() => void refresh()}
              >
                {refreshing ? (
                  <Spinner data-icon="inline-start" />
                ) : (
                  <HugeiconsIcon
                    icon={Refresh01Icon}
                    data-icon="inline-start"
                  />
                )}
                {refreshing ? "正在重试…" : "重试"}
              </Button>
            }
          >
            请检查 Codex 配置目录是否存在，并确认本应用有读取权限。
          </ErrorDetails>
        </AlertDescription>
      </Alert>
    )
  }

  const today = getLocalDayRange(0)
  const weekStart = getLocalDayRange(6)

  return (
    <div className="flex flex-col gap-6">
      {/* 当前连接：一行横排 */}
      <Card>
        <CardContent className="flex items-center justify-between gap-3 p-5">
          <div className="flex min-w-0 items-center gap-3">
            <div className="flex size-10 shrink-0 items-center justify-center rounded-lg bg-primary/10 text-primary">
              <HugeiconsIcon
                icon={data.activeKind === "official" ? Key01Icon : BoxIcon}
                className="size-4"
                aria-hidden="true"
              />
            </div>
            <div className="flex min-w-0 flex-col gap-0.5">
              <div className="flex items-center gap-1.5">
                <span className="truncate text-sm font-medium">
                  {data.activeProvider ?? "尚未选择连接"}
                </span>
                <Badge
                  variant={data.activeProvider ? "default" : "secondary"}
                  className="shrink-0"
                >
                  {data.activeKind === "official"
                    ? "OpenAI"
                    : data.activeKind === "provider"
                      ? "API"
                      : "未连接"}
                </Badge>
              </div>
              <span
                className="truncate font-mono text-[11px] text-muted-foreground"
                title={data.codexHome}
              >
                {data.codexHome}
              </span>
            </div>
          </div>
          <div className="flex shrink-0 items-center gap-1.5">
            <Button
              size="sm"
              disabled={launching || refreshing}
              onClick={() => void launchCodex()}
            >
              {launching ? (
                <Spinner data-icon="inline-start" />
              ) : (
                <HugeiconsIcon
                  icon={ExternalLinkIcon}
                  data-icon="inline-start"
                />
              )}
              {launching ? "启动中…" : "启动 Codex"}
            </Button>
            <Button
              size="sm"
              variant="outline"
              disabled={launching || refreshing}
              onClick={() => void refresh()}
            >
              {refreshing ? (
                <Spinner data-icon="inline-start" />
              ) : (
                <HugeiconsIcon icon={Refresh01Icon} data-icon="inline-start" />
              )}
              {refreshing ? "刷新中…" : "刷新"}
            </Button>
          </div>
        </CardContent>
      </Card>

      {error && (
        <Alert variant="destructive">
          <HugeiconsIcon icon={Alert01Icon} />
          <AlertTitle>当前显示的是上次读取结果</AlertTitle>
          <AlertDescription>
            <ErrorDetails error={error}>
              暂时无法读取最新状态，请稍后刷新。
            </ErrorDetails>
          </AlertDescription>
        </Alert>
      )}

      {/* 今日数据：4 列横排 */}
      <div className="grid grid-cols-4 gap-4">
        <MetricCard
          label="今日 Token"
          value={formatTokens(data.todayUsage.totalTokens)}
        />
        <MetricCard
          label="估算费用"
          value={formatEstimatedUsd(
            data.todayEstimatedCostMicrousd,
            data.todayUnpricedTokens +
              data.todayPartialTokens +
              data.todayUnattributedTokens +
              data.todaySubscriptionTokens
          )}
        />
        <MetricCard label="调用" value={String(data.todayRequests)} />
        <MetricCard
          label="待确认"
          value={formatTokens(
            data.todayUnpricedTokens +
              data.todayPartialTokens +
              data.todayUnattributedTokens
          )}
        />
      </div>

      {/* 额度 + 趋势 + 本机状态：横向并排利用宽度 */}
      <div className="grid grid-cols-1 gap-6 lg:grid-cols-[minmax(0,1.2fr)_minmax(0,1fr)]">
        <Card>
          <CardHeader className="pb-2">
            <div className="flex items-center justify-between gap-2">
              <CardTitle className="text-sm">最近 7 天趋势</CardTitle>
              <Button
                size="sm"
                variant="link"
                className="h-auto p-0 text-xs"
                onClick={() => openPage("usage")}
              >
                明细
                <HugeiconsIcon icon={ArrowRightIcon} data-icon="inline-end" />
              </Button>
            </div>
          </CardHeader>
          <CardContent>
            <UsageTrendChart
              range={{
                startAtMs: weekStart.startAtMs,
                endAtMs: today.endAtMs,
              }}
              refreshKey={refreshSignal.revision}
            />
          </CardContent>
        </Card>

        <div className="flex flex-col gap-6">
          {/* 当前账号额度 */}
          {data.activeKind === "official" && data.activeQuota && (
            <Card>
              <CardHeader className="pb-2">
                <div className="flex items-center justify-between gap-2">
                  <CardTitle className="text-sm">账号额度</CardTitle>
                  <Button
                    variant="outline"
                    size="sm"
                    className="h-7"
                    disabled={quotaRefreshing}
                    onClick={() => void refreshQuota()}
                  >
                    {quotaRefreshing ? (
                      <Spinner data-icon="inline-start" />
                    ) : (
                      <HugeiconsIcon
                        icon={Refresh01Icon}
                        data-icon="inline-start"
                      />
                    )}
                    {quotaRefreshing ? "查询中…" : "刷新"}
                  </Button>
                </div>
              </CardHeader>
              <CardContent>
                <QuotaStatusView quota={data.activeQuota} />
              </CardContent>
            </Card>
          )}

          {/* 本机状态 */}
          <Card className="flex-1">
            <CardHeader className="pb-2">
              <CardTitle className="text-sm">本机状态</CardTitle>
            </CardHeader>
            <CardContent>
              <ItemGroup className="gap-0">
                <Item size="default">
                  <ItemMedia variant="icon">
                    <HugeiconsIcon icon={BoxIcon} />
                  </ItemMedia>
                  <ItemContent>
                    <ItemTitle>API 服务</ItemTitle>
                    <ItemDescription>{data.providerCount} 个</ItemDescription>
                  </ItemContent>
                  <ItemActions>
                    <Button
                      size="sm"
                      variant="ghost"
                      className="h-7"
                      onClick={() => openPage("providers")}
                    >
                      管理
                    </Button>
                  </ItemActions>
                </Item>
                <ItemSeparator />
                <Item size="default">
                  <ItemMedia variant="icon">
                    <HugeiconsIcon icon={Message01Icon} />
                  </ItemMedia>
                  <ItemContent>
                    <ItemTitle>历史会话</ItemTitle>
                    <ItemDescription>{data.sessionCount} 个</ItemDescription>
                  </ItemContent>
                  <ItemActions>
                    <Button
                      size="sm"
                      variant="ghost"
                      className="h-7"
                      onClick={() => openPage("sessions")}
                    >
                      查看
                    </Button>
                  </ItemActions>
                </Item>
                <ItemSeparator />
                <Item size="default">
                  <ItemMedia variant="icon">
                    <HugeiconsIcon icon={File01Icon} />
                  </ItemMedia>
                  <ItemContent>
                    <ItemTitle>会话数据库</ItemTitle>
                    <ItemDescription>{data.databaseCount} 个</ItemDescription>
                  </ItemContent>
                </Item>
                <ItemSeparator />
                <Item size="default">
                  <ItemMedia variant="icon">
                    <HugeiconsIcon icon={Shield01Icon} />
                  </ItemMedia>
                  <ItemContent>
                    <ItemTitle>数据状态</ItemTitle>
                    <ItemDescription>{data.databaseHealth}</ItemDescription>
                  </ItemContent>
                  <ItemActions>
                    <Button
                      size="sm"
                      variant="ghost"
                      className="h-7"
                      onClick={() => openPage("settings")}
                    >
                      检查
                    </Button>
                  </ItemActions>
                </Item>
              </ItemGroup>
            </CardContent>
          </Card>
        </div>
      </div>
    </div>
  )
}

function MetricCard({ label, value }: { label: string; value: string }) {
  return (
    <Card>
      <CardContent className="flex flex-col gap-1 p-5">
        <p className="truncate text-sm text-muted-foreground/80">{label}</p>
        <p className="truncate text-xl font-semibold tracking-tight tabular-nums">
          {value}
        </p>
      </CardContent>
    </Card>
  )
}

function DashboardLoading() {
  return (
    <div className="flex flex-col gap-4" role="status" aria-live="polite">
      <Skeleton className="h-14 w-full" />
      <div className="grid grid-cols-4 gap-3">
        {Array.from({ length: 4 }).map((_, index) => (
          <div key={index} className="flex flex-col gap-1.5">
            <Skeleton className="h-3 w-16" />
            <Skeleton className="h-6 w-14" />
          </div>
        ))}
      </div>
      <Skeleton className="h-64 w-full" />
    </div>
  )
}

function openPage(page: "providers" | "usage" | "sessions" | "settings") {
  window.dispatchEvent(
    new CustomEvent("codex-tools:navigate", { detail: page })
  )
}
