import { useCallback, useEffect, useState } from "react"
import {
  Activity,
  CheckCircle2,
  CircleX,
  Clock,
  ListChecks,
  Radio,
  RefreshCw,
  Save,
  ScrollText,
  Server,
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
import {
  Empty,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from "@/components/ui/empty"
import {
  Field,
  FieldDescription,
  FieldGroup,
  FieldLabel,
} from "@/components/ui/field"
import { Input } from "@/components/ui/input"
import {
  Item,
  ItemContent,
  ItemDescription,
  ItemGroup,
  ItemMedia,
  ItemTitle,
} from "@/components/ui/item"
import { Spinner } from "@/components/ui/spinner"
import { Switch } from "@/components/ui/switch"
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table"
import { call } from "@/lib/ipc"
import type { RouteConsole } from "@/types"

type PendingAction = "save" | "refresh"

const isRetryCount = (value: number) =>
  Number.isInteger(value) && value >= 0 && value <= 100

const isRequestTimeout = (value: number) =>
  Number.isInteger(value) && value >= 1000 && value <= 600000

export default function RoutePage() {
  const [route, setRoute] = useState<RouteConsole>()
  const [error, setError] = useState<string>()
  const [pendingAction, setPendingAction] = useState<PendingAction>()
  const load = useCallback(async () => {
    setRoute(await call("get_route_console", { page: 1, pageSize: 50 }))
    setError(undefined)
  }, [])

  useEffect(() => {
    const timeout = window.setTimeout(() => {
      void load().catch((reason) => setError(String(reason)))
    }, 0)
    return () => window.clearTimeout(timeout)
  }, [load])

  const retry = () => {
    setPendingAction("refresh")
    void load()
      .catch((reason) => setError(String(reason)))
      .finally(() => setPendingAction(undefined))
  }

  const refresh = () => {
    setPendingAction("refresh")
    void load()
      .catch((reason) => toast.error(String(reason)))
      .finally(() => setPendingAction(undefined))
  }

  if (error) {
    return (
      <Alert variant="destructive">
        <AlertTitle>无法读取代理状态</AlertTitle>
        <AlertDescription className="flex flex-col items-start gap-3">
          <span>{error}</span>
          <Button
            variant="outline"
            disabled={pendingAction === "refresh"}
            onClick={retry}
          >
            {pendingAction === "refresh" ? (
              <Spinner data-icon="inline-start" aria-hidden="true" />
            ) : (
              <RefreshCw data-icon="inline-start" />
            )}
            {pendingAction === "refresh" ? "重试中…" : "重试"}
          </Button>
        </AlertDescription>
      </Alert>
    )
  }

  if (!route) return <PageLoading />

  const settings = route.settings
  const isPending = pendingAction !== undefined
  const settingsAreValid =
    isRequestTimeout(settings.requestTimeoutMs) &&
    isRetryCount(settings.requestMaxRetries) &&
    isRetryCount(settings.streamMaxRetries)
  const proxyUrl =
    route.baseUrl ?? `http://${settings.listenAddress}:${settings.port}/v1`
  const successRate = route.requestCount
    ? `${Math.round((route.successCount / route.requestCount) * 100)}% 成功率`
    : "暂无请求"

  return (
    <div className="flex flex-col gap-5">
      <div className="grid gap-5 lg:grid-cols-[minmax(0,3fr)_minmax(16rem,2fr)]">
        <Card>
          <CardHeader>
            <CardTitle>回环代理</CardTitle>
            <CardDescription>
              无客户端认证；永久限制在 127.0.0.1。
            </CardDescription>
            <CardAction>
              <Badge variant={route.running ? "default" : "secondary"}>
                {route.running ? "运行中" : "已停止"}
              </Badge>
            </CardAction>
          </CardHeader>
          <CardContent>
            <FieldGroup className="grid gap-4 md:grid-cols-2">
              <Field data-disabled>
                <FieldLabel htmlFor="route-address">监听地址</FieldLabel>
                <Input
                  id="route-address"
                  disabled
                  value={settings.listenAddress}
                />
              </Field>
              <Field data-disabled>
                <FieldLabel htmlFor="route-port">端口</FieldLabel>
                <Input id="route-port" disabled value={settings.port} />
              </Field>
              <Field className="md:col-span-2">
                <FieldLabel htmlFor="route-timeout">
                  请求超时（毫秒）
                </FieldLabel>
                <Input
                  id="route-timeout"
                  type="number"
                  min={1000}
                  max={600000}
                  step={1000}
                  aria-invalid={!isRequestTimeout(settings.requestTimeoutMs)}
                  value={settings.requestTimeoutMs}
                  onChange={(event) =>
                    setRoute({
                      ...route,
                      settings: {
                        ...settings,
                        requestTimeoutMs: Number(event.target.value),
                      },
                    })
                  }
                />
                <FieldDescription>
                  Codex 流空闲超时固定为 300000ms。
                </FieldDescription>
              </Field>
              <Field>
                <FieldLabel htmlFor="request-max-retries">
                  请求最大重试次数
                </FieldLabel>
                <Input
                  id="request-max-retries"
                  type="number"
                  min={0}
                  max={100}
                  step={1}
                  aria-invalid={!isRetryCount(settings.requestMaxRetries)}
                  value={settings.requestMaxRetries}
                  onChange={(event) =>
                    setRoute({
                      ...route,
                      settings: {
                        ...settings,
                        requestMaxRetries: Number(event.target.value),
                      },
                    })
                  }
                />
                <FieldDescription>可设置为 0–100，默认 4。</FieldDescription>
              </Field>
              <Field>
                <FieldLabel htmlFor="stream-max-retries">
                  流式请求最大重试次数
                </FieldLabel>
                <Input
                  id="stream-max-retries"
                  type="number"
                  min={0}
                  max={100}
                  step={1}
                  aria-invalid={!isRetryCount(settings.streamMaxRetries)}
                  value={settings.streamMaxRetries}
                  onChange={(event) =>
                    setRoute({
                      ...route,
                      settings: {
                        ...settings,
                        streamMaxRetries: Number(event.target.value),
                      },
                    })
                  }
                />
                <FieldDescription>可设置为 0–100，默认 3。</FieldDescription>
              </Field>
              <Field className="md:col-span-2" orientation="horizontal">
                <Switch
                  id="route-enabled"
                  checked={settings.enabled}
                  onCheckedChange={(enabled) =>
                    setRoute({ ...route, settings: { ...settings, enabled } })
                  }
                />
                <FieldLabel htmlFor="route-enabled">启用本地代理</FieldLabel>
              </Field>
            </FieldGroup>
          </CardContent>
          <CardFooter className="justify-end gap-2">
            <Button variant="outline" disabled={isPending} onClick={refresh}>
              {pendingAction === "refresh" ? (
                <Spinner data-icon="inline-start" aria-hidden="true" />
              ) : (
                <RefreshCw data-icon="inline-start" />
              )}
              {pendingAction === "refresh" ? "刷新中…" : "刷新"}
            </Button>
            <Button
              disabled={isPending || !settingsAreValid}
              onClick={() => {
                setPendingAction("save")
                void call("save_route_settings", { settings })
                  .then(load)
                  .then(() => toast.success("代理设置已保存"))
                  .catch((reason) => toast.error(String(reason)))
                  .finally(() => setPendingAction(undefined))
              }}
            >
              {pendingAction === "save" ? (
                <Spinner data-icon="inline-start" aria-hidden="true" />
              ) : (
                <Save data-icon="inline-start" />
              )}
              {pendingAction === "save" ? "保存中…" : "保存"}
            </Button>
          </CardFooter>
        </Card>

        <Card size="sm">
          <CardHeader>
            <CardTitle>当前连接</CardTitle>
            <CardDescription>代理端点与当前上游摘要</CardDescription>
            <CardAction>
              <Badge variant="outline">本机</Badge>
            </CardAction>
          </CardHeader>
          <CardContent>
            <ItemGroup>
              <Item variant="muted" size="sm">
                <ItemMedia variant="icon">
                  <Radio />
                </ItemMedia>
                <ItemContent>
                  <ItemTitle>代理端点</ItemTitle>
                  <ItemDescription title={proxyUrl}>{proxyUrl}</ItemDescription>
                </ItemContent>
              </Item>
              <Item variant="muted" size="sm">
                <ItemMedia variant="icon">
                  <Server />
                </ItemMedia>
                <ItemContent>
                  <ItemTitle>上游</ItemTitle>
                  <ItemDescription>
                    {route.providerName
                      ? `${route.providerName} / ${route.accountName ?? "默认账号"}`
                      : "尚未选择上游"}
                  </ItemDescription>
                </ItemContent>
              </Item>
              <Item variant="muted" size="sm">
                <ItemMedia variant="icon">
                  <Activity />
                </ItemMedia>
                <ItemContent>
                  <ItemTitle>模型</ItemTitle>
                  <ItemDescription>{route.model ?? "尚未报告"}</ItemDescription>
                </ItemContent>
              </Item>
            </ItemGroup>
          </CardContent>
        </Card>
      </div>

      <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-4">
        <MetricCard
          label="请求总数"
          value={route.requestCount}
          icon={ListChecks}
          detail={`${route.logTotal} 条日志`}
        />
        <MetricCard
          label="成功"
          value={route.successCount}
          icon={CheckCircle2}
          detail={successRate}
        />
        <MetricCard
          label="错误"
          value={route.errorCount}
          icon={CircleX}
          detail={route.errorCount ? "请检查请求日志" : "未发现错误"}
        />
        <MetricCard
          label="活动请求"
          value={route.activeRequests}
          icon={Clock}
          detail={
            route.lastLatencyMs === undefined
              ? "暂无延迟数据"
              : `最近 ${route.lastLatencyMs}ms`
          }
        />
      </div>

      <Card>
        <CardHeader>
          <CardTitle>请求日志</CardTitle>
          <CardDescription>
            {route.providerName
              ? `${route.providerName} / ${route.accountName ?? "默认账号"}`
              : "选择上游后可查看转发记录"}
          </CardDescription>
          <CardAction>
            <Badge variant="outline">{route.logTotal} 条</Badge>
          </CardAction>
        </CardHeader>
        <CardContent>
          {route.logs.length ? (
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>时间</TableHead>
                  <TableHead>请求</TableHead>
                  <TableHead>状态</TableHead>
                  <TableHead className="text-right">耗时</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {route.logs.map((log) => (
                  <TableRow key={log.id}>
                    <TableCell className="tabular-nums">
                      {new Date(log.timestamp * 1000).toLocaleTimeString()}
                    </TableCell>
                    <TableCell>
                      <div className="flex min-w-0 items-center gap-2">
                        <Badge variant="outline">{log.method}</Badge>
                        <span className="max-w-80 truncate" title={log.path}>
                          {log.path}
                        </span>
                      </div>
                    </TableCell>
                    <TableCell>
                      <Badge
                        variant={
                          log.status >= 400 ? "destructive" : "secondary"
                        }
                      >
                        {log.status}
                      </Badge>
                    </TableCell>
                    <TableCell className="text-right tabular-nums">
                      {log.latencyMs}ms
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          ) : (
            <Empty className="min-h-44 border border-dashed">
              <EmptyHeader>
                <EmptyMedia variant="icon">
                  <ScrollText />
                </EmptyMedia>
                <EmptyTitle>暂无请求日志</EmptyTitle>
                <EmptyDescription>
                  Codex 通过本地代理发送请求后，记录会显示在这里。
                </EmptyDescription>
              </EmptyHeader>
            </Empty>
          )}
        </CardContent>
      </Card>
    </div>
  )
}
