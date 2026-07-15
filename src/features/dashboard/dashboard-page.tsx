import { useCallback, useEffect, useState } from "react"
import {
  Activity,
  Database,
  Play,
  RefreshCw,
  Server,
  ShieldCheck,
} from "lucide-react"
import { toast } from "sonner"

import { MetricCard } from "@/components/metric-card"
import { PageLoading } from "@/components/page-loading"
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
  const load = useCallback(async () => setData(await call("get_dashboard")), [])

  useEffect(() => {
    void call<Dashboard>("get_dashboard")
      .then(setData)
      .catch((error) => toast.error(String(error)))
  }, [])

  const launchCodex = async () => {
    setLaunching(true)
    try {
      await call("launch_codex")
      toast.success("已启动 Codex")
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
      toast.success("状态已刷新")
    } catch (error) {
      toast.error(String(error))
    } finally {
      setRefreshing(false)
    }
  }

  if (!data) return <PageLoading />

  const metrics = [
    {
      label: "供应商",
      value: data.providerCount,
      icon: Server,
      detail: data.activeProvider ?? "尚未激活",
    },
    {
      label: "会话",
      value: data.sessionCount,
      icon: Activity,
      detail: "本地会话索引",
    },
    {
      label: "会话数据库",
      value: data.databaseCount,
      icon: Database,
      detail: "已发现的数据源",
    },
    {
      label: "数据库状态",
      value: data.databaseHealth,
      icon: ShieldCheck,
      detail: "SQLite 健康检查",
    },
  ]
  const busy = launching || refreshing

  return (
    <div className="flex flex-col gap-5">
      <Card>
        <CardHeader>
          <CardTitle>运行状态</CardTitle>
          <CardDescription>本机 Codex 与当前上游的运行概况</CardDescription>
          <CardAction>
            <Badge variant={data.activeProvider ? "default" : "secondary"}>
              {data.activeProvider ?? "尚未激活"}
            </Badge>
          </CardAction>
        </CardHeader>
        <CardContent>
          <div className="flex min-w-0 flex-col gap-1">
            <span className="text-xs text-muted-foreground">Codex Home</span>
            <code className="truncate" title={data.codexHome}>
              {data.codexHome}
            </code>
          </div>
        </CardContent>
        <CardFooter className="flex-wrap gap-2">
          <Button disabled={busy} onClick={() => void launchCodex()}>
            {launching ? (
              <Spinner data-icon="inline-start" />
            ) : (
              <Play data-icon="inline-start" />
            )}
            {launching ? "正在启动..." : "启动 Codex"}
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
            {refreshing ? "正在刷新..." : "刷新"}
          </Button>
        </CardFooter>
      </Card>
      <div className="grid gap-3 sm:grid-cols-2 xl:grid-cols-4">
        {metrics.map((metric) => (
          <MetricCard key={metric.label} {...metric} />
        ))}
      </div>
    </div>
  )
}
