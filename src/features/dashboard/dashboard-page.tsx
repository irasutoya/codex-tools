import { useCallback, useEffect, useState } from "react"
import {
  ExternalLink,
  FileText,
  FolderCog,
  KeyRound,
  MessagesSquare,
  RefreshCw,
  Server,
  ShieldCheck,
  TriangleAlert,
} from "lucide-react"

import { ErrorDetails } from "@/components/error-details"
import { MetricCard } from "@/components/metric-card"
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
  ItemMedia,
  ItemTitle,
} from "@/components/ui/item"
import { Spinner } from "@/components/ui/spinner"
import { notify } from "@/lib/feedback"
import { call } from "@/lib/ipc"
import type { Dashboard, PageProps } from "@/types"

import { QuotaStatusView } from "../providers/quota-status"

export default function DashboardPage({ active }: PageProps) {
  const [data, setData] = useState<Dashboard>()
  const [launching, setLaunching] = useState(false)
  const [refreshing, setRefreshing] = useState(false)
  const [quotaRefreshing, setQuotaRefreshing] = useState(false)
  const [error, setError] = useState<string>()

  const load = useCallback(async () => {
    try {
      setData(await call("get_dashboard"))
      setError(undefined)
    } catch (reason) {
      setError(String(reason))
      throw reason
    }
  }, [])

  useEffect(() => {
    if (!active) return
    const timeout = window.setTimeout(() => {
      void load().catch(() => undefined)
    }, 0)
    return () => window.clearTimeout(timeout)
  }, [active, load])

  const launchCodex = async () => {
    setLaunching(true)
    try {
      await call("launch_codex")
      notify.success("Codex 已打开")
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
    if (!data?.activeAccountId) return
    setQuotaRefreshing(true)
    try {
      const quota = await call("refresh_official_account_quota", {
        accountId: data.activeAccountId,
      })
      await load()
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
            请确认 Codex 已安装，并允许本应用访问配置目录。
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
      detail: "Codex 本地数据文件",
    },
    {
      label: "数据状态",
      value: data.databaseHealth,
      icon: ShieldCheck,
      detail: "会话数据是否可读取",
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
            Codex 正在使用的账号或第三方 API 服务。
          </CardDescription>
          <CardAction>
            <Badge variant={data.activeProvider ? "default" : "secondary"}>
              {data.activeProvider ?? "尚未选择"}
            </Badge>
          </CardAction>
        </CardHeader>
        <CardContent className="flex flex-col gap-3">
          {data.activeAccount && (
            <Item variant="muted">
              <ItemMedia variant="icon">
                <KeyRound />
              </ItemMedia>
              <ItemContent className="min-w-0">
                <ItemTitle>{data.activeAccount}</ItemTitle>
                <ItemDescription>
                  {data.activeKind === "official"
                    ? "当前 Codex 账号"
                    : "当前 API Key"}
                </ItemDescription>
                {data.activeKind === "official" && (
                  <QuotaStatusView quota={data.activeQuota} />
                )}
              </ItemContent>
            </Item>
          )}
          <Item variant="muted">
            <ItemMedia variant="icon">
              <FolderCog />
            </ItemMedia>
            <ItemContent className="min-w-0">
              <ItemTitle>Codex 配置目录</ItemTitle>
              <ItemDescription
                className="truncate font-mono"
                title={data.codexHome}
              >
                {data.codexHome}
              </ItemDescription>
            </ItemContent>
          </Item>
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

      <section
        className="flex flex-col gap-4"
        aria-labelledby="local-status-title"
      >
        <div className="flex flex-col gap-1">
          <h2 id="local-status-title" className="text-base font-medium">
            本机状态
          </h2>
          <p className="text-sm text-muted-foreground">
            账号、服务和本地会话的当前概况。
          </p>
        </div>
        <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
          {metrics.map((metric) => (
            <MetricCard key={metric.label} {...metric} />
          ))}
        </div>
      </section>
    </div>
  )
}
