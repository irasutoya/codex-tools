import { useCallback, useEffect, useState } from "react"
import {
  Activity,
  Database,
  Play,
  RefreshCw,
  Server,
  ShieldCheck,
  TriangleAlert,
} from "lucide-react"
import { toast } from "sonner"

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
import { Spinner } from "@/components/ui/spinner"
import { call } from "@/lib/ipc"
import type { Dashboard } from "@/types"

export default function DashboardPage() {
  const [data, setData] = useState<Dashboard>()
  const [launching, setLaunching] = useState(false)
  const [refreshing, setRefreshing] = useState(false)
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
    const timeout = window.setTimeout(() => {
      void load().catch(() => undefined)
    }, 0)
    return () => window.clearTimeout(timeout)
  }, [load])

  const launchCodex = async () => {
    setLaunching(true)
    try {
      await call("launch_codex")
      toast.success("Codex 已启动")
    } catch (error) {
      toast.error(String(error))
    } finally {
      setLaunching(false)
    }
  }

  const refresh = async () => {
    setRefreshing(true)
    try {
      await load()
      toast.success("已获取最新状态")
    } catch (error) {
      toast.error(String(error))
    } finally {
      setRefreshing(false)
    }
  }

  if (!data) {
    if (!error) return <PageLoading />
    return (
      <Alert variant="destructive">
        <TriangleAlert />
        <AlertTitle>暂时无法获取 Codex 状态</AlertTitle>
        <AlertDescription className="flex flex-wrap items-center gap-3">
          <span>{error}</span>
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
            重试
          </Button>
        </AlertDescription>
      </Alert>
    )
  }

  const metrics = [
    {
      label: "API 服务",
      value: data.providerCount,
      icon: Server,
      detail: "已保存的第三方服务",
    },
    {
      label: "历史会话",
      value: data.sessionCount,
      icon: Activity,
      detail: "在本机找到的记录",
    },
    {
      label: "数据文件",
      value: data.databaseCount,
      icon: Database,
      detail: "Codex 使用的数据库",
    },
    {
      label: "数据状态",
      value: data.databaseHealth,
      icon: ShieldCheck,
      detail: "历史会话是否可读取",
    },
  ]
  const busy = launching || refreshing

  return (
    <div className="flex flex-col gap-6">
      {error && (
        <Alert variant="destructive">
          <TriangleAlert />
          <AlertTitle>未能获取最新状态</AlertTitle>
          <AlertDescription>{error}</AlertDescription>
        </Alert>
      )}
      <Card className="border-transparent bg-[var(--md-sys-color-secondary-container)] text-[var(--md-sys-color-on-secondary-container)]">
        <CardHeader>
          <CardTitle>当前连接</CardTitle>
          <CardDescription className="text-current/70">
            Codex 当前使用的账号或 API 服务
          </CardDescription>
          <CardAction>
            <Badge variant={data.activeProvider ? "default" : "secondary"}>
              {data.activeProvider ?? "尚未选择"}
            </Badge>
          </CardAction>
        </CardHeader>
        <CardContent>
          <div className="flex min-w-0 flex-col gap-1">
            <span className="text-xs text-current/70">Codex 配置目录</span>
            <code className="truncate" title={data.codexHome}>
              {data.codexHome}
            </code>
          </div>
        </CardContent>
        <CardFooter className="flex-wrap gap-2 bg-transparent">
          <Button disabled={busy} onClick={() => void launchCodex()}>
            {launching ? (
              <Spinner data-icon="inline-start" />
            ) : (
              <Play data-icon="inline-start" />
            )}
            {launching ? "正在启动…" : "打开 Codex"}
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
        </CardFooter>
      </Card>
      <div className="grid gap-4 sm:grid-cols-2 xl:grid-cols-4">
        {metrics.map((metric) => (
          <MetricCard key={metric.label} {...metric} />
        ))}
      </div>
    </div>
  )
}
