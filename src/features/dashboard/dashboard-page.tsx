import { useCallback, useEffect, useRef, useState } from "react"
import {
  ExternalLink,
  FileText,
  FolderCog,
  ArrowRight,
  KeyRound,
  MessagesSquare,
  RefreshCw,
  Server,
  ShieldCheck,
  TriangleAlert,
} from "lucide-react"

import { ErrorDetails } from "@/components/error-details"
import { MetricCard } from "@/components/metric-card"
import { SectionHeader } from "@/components/page-header"
import { PageLoading } from "@/components/page-loading"
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
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
  Item,
  ItemContent,
  ItemDescription,
  ItemGroup,
  ItemMedia,
  ItemSeparator,
  ItemTitle,
} from "@/components/ui/item"
import { Spinner } from "@/components/ui/spinner"
import { notify } from "@/lib/feedback"
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
import {
  formatEstimatedUsd,
  formatTokenDetail,
  formatTokens,
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

  const loadDashboard = useCallback(async () => {
    try {
      setData(await call("get_dashboard"))
      setError(undefined)
    } catch (reason) {
      setError(String(reason))
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
    if (
      !active ||
      !foreground ||
      lastRefreshRevision.current === refreshSignal.revision
    ) {
      return
    }
    lastRefreshRevision.current = refreshSignal.revision
    const timeout = window.setTimeout(() => {
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
      await call("launch_codex")
      notify.success("已发送打开 Codex 的请求")
    } catch (reason) {
      notify.error("无法打开 Codex", reason)
    } finally {
      setLaunching(false)
    }
  }

  const refresh = async () => {
    setRefreshing(true)
    try {
      await load()
      notify.success("状态已更新")
    } catch (reason) {
      notify.error("无法更新状态", reason)
    } finally {
      setRefreshing(false)
    }
  }

  const refreshQuota = async () => {
    if (!activeAccountId) return
    setQuotaRefreshing(true)
    try {
      const quota = await runQuotaRefresh(activeAccountId, refreshQuotaData)
      if (quota.status === "success") {
        notify.success("当前账号额度已更新")
      } else {
        notify.warning(
          "当前账号额度未更新",
          quota.error ?? "OpenAI 暂未返回额度。"
        )
      }
    } catch (reason) {
      notify.error("无法刷新当前账号额度", reason)
    } finally {
      setQuotaRefreshing(false)
    }
  }

  if (!data) {
    if (!error) return <PageLoading label="正在读取 Codex 状态" />
    return (
      <Alert variant="destructive">
        <TriangleAlert />
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
                  <RefreshCw data-icon="inline-start" />
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

  const metrics = [
    {
      label: "API 服务",
      value: data.providerCount,
      icon: Server,
      detail: "已添加的第三方服务",
    },
    {
      label: "历史会话",
      value: data.sessionCount,
      icon: MessagesSquare,
      detail: "本机找到的会话",
    },
    {
      label: "会话数据库",
      value: data.databaseCount,
      icon: FileText,
      detail: "找到的 Codex 数据库文件",
    },
    {
      label: "数据状态",
      value: data.databaseHealth,
      icon: ShieldCheck,
      detail: "本机会话索引结果",
    },
  ]
  const busy = launching || refreshing || quotaRefreshing

  return (
    <div className="flex flex-col gap-6">
      {error && (
        <Alert variant="destructive">
          <TriangleAlert />
          <AlertTitle>当前显示的是上次读取结果</AlertTitle>
          <AlertDescription>
            <ErrorDetails error={error}>
              暂时无法读取最新状态，请稍后刷新。
            </ErrorDetails>
          </AlertDescription>
        </Alert>
      )}

      <Card>
        <CardHeader>
          <CardTitle>当前连接</CardTitle>
          <CardDescription>
            Codex 下一次请求会使用这里显示的账号或 API 服务。
          </CardDescription>
          <CardAction>
            <Badge
              className="max-w-48 truncate"
              title={data.activeProvider}
              variant={data.activeProvider ? "default" : "secondary"}
            >
              {data.activeProvider ?? "尚未选择"}
            </Badge>
          </CardAction>
        </CardHeader>
        <CardContent>
          <ItemGroup className="gap-0">
            {data.activeAccount && (
              <>
                <Item>
                  <ItemMedia variant="icon">
                    <KeyRound />
                  </ItemMedia>
                  <ItemContent className="min-w-0">
                    <ItemTitle>{data.activeAccount}</ItemTitle>
                    <ItemDescription>
                      {data.activeKind === "official"
                        ? "OpenAI 账号"
                        : "第三方 API Key"}
                    </ItemDescription>
                    {data.activeKind === "official" && (
                      <QuotaStatusView quota={data.activeQuota} />
                    )}
                  </ItemContent>
                </Item>
                <ItemSeparator />
              </>
            )}
            <Item>
              <ItemMedia variant="icon">
                <FolderCog />
              </ItemMedia>
              <ItemContent className="min-w-0">
                <ItemTitle>配置目录</ItemTitle>
                <ItemDescription
                  className="truncate font-mono"
                  title={data.codexHome}
                >
                  {data.codexHome}
                </ItemDescription>
              </ItemContent>
            </Item>
          </ItemGroup>
        </CardContent>
        <CardFooter className="flex-wrap gap-2">
          <Button disabled={busy} onClick={() => void launchCodex()}>
            {launching ? (
              <Spinner data-icon="inline-start" />
            ) : (
              <ExternalLink data-icon="inline-start" />
            )}
            {launching ? "正在打开…" : "打开 Codex"}
          </Button>
          <Button
            variant="outline"
            disabled={busy}
            onClick={() => void refresh()}
          >
            {refreshing ? (
              <Spinner data-icon="inline-start" />
            ) : (
              <RefreshCw data-icon="inline-start" />
            )}
            {refreshing ? "正在刷新…" : "刷新状态"}
          </Button>
          {data.activeKind === "official" && data.activeAccountId && (
            <Button
              variant="outline"
              disabled={busy}
              onClick={() => void refreshQuota()}
            >
              {quotaRefreshing ? (
                <Spinner data-icon="inline-start" />
              ) : (
                <RefreshCw data-icon="inline-start" />
              )}
              {quotaRefreshing ? "正在查询…" : "刷新当前额度"}
            </Button>
          )}
        </CardFooter>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>今日本机用量</CardTitle>
          <CardDescription>
            官方账号和第三方中转站都从本机 Codex rollout
            日志统计；美元费用只包含已配置价格。
          </CardDescription>
          <CardAction>
            <div className="flex items-center gap-2">
              <Badge variant="secondary">{data.todayRequests} 次调用</Badge>
              <Button size="sm" variant="link" onClick={() => openUsagePage()}>
                查看明细
                <ArrowRight data-icon="inline-end" />
              </Button>
            </div>
          </CardAction>
        </CardHeader>
        <CardContent>
          <div className="grid gap-4 sm:grid-cols-3">
            <div>
              <p className="text-xs text-muted-foreground">总 Token</p>
              <p className="mt-1 text-2xl font-semibold tabular-nums">
                {formatTokens(data.todayUsage.totalTokens)}
              </p>
              <p className="mt-1 text-xs text-muted-foreground">
                {formatTokenDetail(data.todayUsage)}
              </p>
            </div>
            <div>
              <p className="text-xs text-muted-foreground">估算费用</p>
              <p className="mt-1 text-2xl font-semibold tabular-nums">
                {formatEstimatedUsd(
                  data.todayEstimatedCostMicrousd,
                  data.todayUnpricedTokens +
                    data.todayPartialTokens +
                    data.todayUnattributedTokens +
                    data.todaySubscriptionTokens
                )}
              </p>
              <p className="mt-1 text-xs text-muted-foreground">
                {data.todaySubscriptionTokens > 0
                  ? "官方套餐按 Token 统计"
                  : "仅统计已匹配 USD 价格的 Token"}
              </p>
            </div>
            <div>
              <p className="text-xs text-muted-foreground">待确认 Token</p>
              <p className="mt-1 text-2xl font-semibold tabular-nums">
                {formatTokens(
                  data.todayUnpricedTokens +
                    data.todayPartialTokens +
                    data.todayUnattributedTokens
                )}
              </p>
              <p className="mt-1 text-xs text-muted-foreground">
                未定价 {formatTokens(data.todayUnpricedTokens)} · 未归属{" "}
                {formatTokens(data.todayUnattributedTokens)}
              </p>
            </div>
          </div>
        </CardContent>
      </Card>

      <section
        className="flex flex-col gap-4"
        aria-labelledby="local-status-title"
      >
        <SectionHeader
          id="local-status-title"
          title="本机状态"
          description="这些统计来自当前设备上的 Codex 配置和会话文件。"
        />
        <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-4">
          {metrics.map((metric) => (
            <MetricCard key={metric.label} {...metric} />
          ))}
        </div>
      </section>
    </div>
  )
}

function openUsagePage() {
  window.dispatchEvent(
    new CustomEvent("codex-tools:navigate", { detail: "usage" })
  )
}
