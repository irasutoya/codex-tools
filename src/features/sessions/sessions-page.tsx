import { useCallback, useEffect, useState } from "react"
import {
  ChevronLeft,
  ChevronRight,
  Database,
  FileText,
  Info,
  MessagesSquare,
  RefreshCw,
  ScanSearch,
  Wrench,
  TriangleAlert,
} from "lucide-react"

import { ErrorDetails } from "@/components/error-details"
import { MetricCard } from "@/components/metric-card"
import { PageLoading } from "@/components/page-loading"
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
import { Spinner } from "@/components/ui/spinner"
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table"
import { call } from "@/lib/ipc"
import { notify } from "@/lib/feedback"
import { epochMilliseconds } from "@/lib/time"
import type { PageProps, PageResult, RepairScan, Session } from "@/types"

type ManagedProvider = "openai" | "custom"

const sessionTimestampFormatter = new Intl.DateTimeFormat("zh-CN", {
  dateStyle: "short",
  timeStyle: "medium",
})

function oppositeProvider(provider: string): ManagedProvider {
  return provider === "custom" ? "openai" : "custom"
}

function providerLabel(provider: string) {
  if (provider === "openai") return "OpenAI 官方账号"
  if (provider === "custom") return "第三方 API"
  return provider || "未识别"
}

function sourceLabel(source: string) {
  if (source === "rollout") return "会话文件"
  if (source === "sqlite") return "会话数据库"
  return source
}

function formatSessionTimestamp(value: number) {
  const date = new Date(epochMilliseconds(value))
  return Number.isNaN(date.getTime())
    ? "时间未知"
    : sessionTimestampFormatter.format(date)
}

export default function SessionsPage({ active }: PageProps) {
  const [scan, setScan] = useState<RepairScan>()
  const [sessions, setSessions] = useState<PageResult<Session>>()
  const [page, setPage] = useState(1)
  const [confirming, setConfirming] = useState(false)
  const [error, setError] = useState<string>()
  const [busy, setBusy] = useState(false)
  const [refreshing, setRefreshing] = useState(false)
  const [paging, setPaging] = useState(false)
  const [repairTarget, setRepairTarget] = useState<ManagedProvider>()
  const loadScan = useCallback(async () => {
    setScan(await call("scan_codex_data"))
  }, [])
  const loadSessions = useCallback(
    async (refresh = false) => {
      const result = await call("list_sessions", {
        page,
        pageSize: 50,
        refresh,
      })
      const lastPage = Math.max(1, Math.ceil(result.total / result.pageSize))
      if (page > lastPage) {
        setPaging(true)
        setPage(lastPage)
        return
      }
      setSessions(result)
    },
    [page]
  )
  const refreshAll = useCallback(async () => {
    await Promise.all([loadScan(), loadSessions(true)])
    setError(undefined)
  }, [loadScan, loadSessions])

  useEffect(() => {
    if (!active) return
    const timeout = window.setTimeout(() => {
      void loadScan().catch((reason) => setError(String(reason)))
    }, 0)
    return () => window.clearTimeout(timeout)
  }, [active, loadScan])

  useEffect(() => {
    if (!active) return
    const timeout = window.setTimeout(() => {
      void loadSessions()
        .catch((reason) => setError(String(reason)))
        .finally(() => setPaging(false))
    }, 0)
    return () => window.clearTimeout(timeout)
  }, [active, loadSessions])

  const retry = () => {
    setRefreshing(true)
    void refreshAll()
      .catch((reason) => setError(String(reason)))
      .finally(() => setRefreshing(false))
  }

  const refresh = () => {
    setRefreshing(true)
    void refreshAll()
      .then(() => notify.success("会话列表已刷新"))
      .catch((reason) => notify.error("会话列表刷新失败", reason))
      .finally(() => setRefreshing(false))
  }

  if (error) {
    return (
      <Alert variant="destructive">
        <TriangleAlert />
        <AlertTitle>无法读取历史会话</AlertTitle>
        <AlertDescription>
          <ErrorDetails
            error={error}
            action={
              <Button variant="outline" disabled={refreshing} onClick={retry}>
                {refreshing ? (
                  <Spinner data-icon="inline-start" aria-hidden="true" />
                ) : (
                  <RefreshCw data-icon="inline-start" />
                )}
                {refreshing ? "重试中…" : "重试"}
              </Button>
            }
          >
            请确认本应用可以访问 Codex 配置目录和会话数据库。
          </ErrorDetails>
        </AlertDescription>
      </Alert>
    )
  }

  if (!scan || !sessions) return <PageLoading label="正在读取历史会话" />

  const sessionProviders = scan.targets.filter((item) =>
    item.sources.some((source) => source === "rollout" || source === "sqlite")
  )
  const unifiedProvider =
    sessionProviders.length === 1 &&
    (sessionProviders[0].id === "openai" || sessionProviders[0].id === "custom")
      ? sessionProviders[0].id
      : scan.currentProvider
  const target =
    repairTarget && repairTarget !== unifiedProvider
      ? repairTarget
      : oppositeProvider(unifiedProvider)
  const totalPages = Math.max(1, Math.ceil(sessions.total / sessions.pageSize))
  const controlsDisabled = refreshing || paging || busy
  const repairSessions = async () => {
    setConfirming(false)
    setBusy(true)
    try {
      const result = await call("repair_codex_data", {
        targetProvider: target,
      })
      notify.success(
        "会话归属已更新",
        `修改了 ${result.filesModified} 个文件和 ${result.rowsUpdated} 条数据库记录。`
      )
      result.warnings.forEach((warning) =>
        notify.warning("部分会话未能更新", warning)
      )
      setRepairTarget(oppositeProvider(result.targetProvider))
      try {
        await refreshAll()
      } catch (reason) {
        notify.warning("归属已更新，但列表刷新失败", reason)
      }
    } catch (reason) {
      notify.error("会话归属更新失败", reason)
    } finally {
      setBusy(false)
    }
  }

  return (
    <div className="flex flex-col gap-6">
      <section
        className="flex flex-col gap-4"
        aria-labelledby="session-status-title"
      >
        <div className="flex flex-col gap-1">
          <h2 id="session-status-title" className="text-base font-medium">
            本机会话
          </h2>
          <p className="text-sm text-muted-foreground">
            Codex 保存在当前配置目录中的会话数据。
          </p>
        </div>
        <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
          <MetricCard
            label="会话总数"
            value={sessions.total}
            icon={MessagesSquare}
            detail="本机可查看的会话"
          />
          <MetricCard
            label="连接归属"
            value={scan.sessionMetaCount}
            icon={ScanSearch}
            detail={`当前标记为 ${providerLabel(scan.currentProvider)}`}
          />
          <MetricCard
            label="会话文件"
            value={scan.rolloutFiles}
            icon={FileText}
            detail="Codex 保存的对话文件"
          />
          <MetricCard
            label="数据库文件"
            value={scan.databases.length}
            icon={Database}
            detail="Codex 保存的会话数据库"
          />
        </div>
      </section>

      <Card size="sm">
        <CardHeader>
          <CardTitle>更新会话归属</CardTitle>
          <CardDescription>
            切换连接后，将已有会话标记为当前账号或 API 服务。
          </CardDescription>
          <CardAction>
            <Badge variant="outline">目标：{providerLabel(target)}</Badge>
          </CardAction>
        </CardHeader>
        <CardContent className="flex flex-col gap-4">
          <Alert>
            <Info />
            <AlertTitle>只更新归属信息</AlertTitle>
            <AlertDescription>
              不会改动或上传对话内容。正在使用的会话会自动跳过，可稍后重试。
            </AlertDescription>
          </Alert>
          <ItemGroup className="grid gap-2 sm:grid-cols-2">
            {scan.targets.map((item) => (
              <Item
                key={item.id}
                variant={item.current ? "muted" : "outline"}
                size="sm"
              >
                <ItemContent>
                  <ItemTitle>{providerLabel(item.id)}</ItemTitle>
                  <ItemDescription>
                    来源：
                    {item.sources.map(sourceLabel).join("、") || "未知"}
                  </ItemDescription>
                </ItemContent>
                <ItemActions>
                  <Badge variant={item.current ? "default" : "secondary"}>
                    {item.current ? "当前连接" : "待更新"}
                  </Badge>
                </ItemActions>
              </Item>
            ))}
          </ItemGroup>
        </CardContent>
        <CardFooter className="justify-end">
          <Button
            disabled={controlsDisabled}
            onClick={() => setConfirming(true)}
          >
            {busy ? (
              <Spinner data-icon="inline-start" aria-hidden="true" />
            ) : (
              <Wrench data-icon="inline-start" />
            )}
            {busy ? "更新中…" : "更新归属"}
          </Button>
        </CardFooter>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>会话记录</CardTitle>
          <CardDescription>
            仅查看 Codex 保存在本机的会话，不会复制或上传。
          </CardDescription>
          <CardAction>
            <Button
              size="sm"
              variant="outline"
              disabled={controlsDisabled}
              onClick={refresh}
            >
              {refreshing ? (
                <Spinner data-icon="inline-start" aria-hidden="true" />
              ) : (
                <RefreshCw data-icon="inline-start" />
              )}
              {refreshing ? "刷新中…" : "刷新"}
            </Button>
          </CardAction>
        </CardHeader>
        <CardContent className="min-w-0">
          {sessions.items.length ? (
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>标题</TableHead>
                  <TableHead>使用方式</TableHead>
                  <TableHead className="hidden lg:table-cell">项目</TableHead>
                  <TableHead>更新时间</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {sessions.items.map((session) => (
                  <TableRow key={session.identity}>
                    <TableCell className="max-w-80 truncate font-medium">
                      {session.title || session.id}
                    </TableCell>
                    <TableCell>
                      <Badge variant="outline">
                        {providerLabel(session.provider)}
                      </Badge>
                    </TableCell>
                    <TableCell className="hidden max-w-64 truncate lg:table-cell">
                      {session.cwd}
                    </TableCell>
                    <TableCell className="whitespace-nowrap tabular-nums">
                      {formatSessionTimestamp(session.updatedAt)}
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          ) : (
            <Empty className="min-h-52 border border-dashed">
              <EmptyHeader>
                <EmptyMedia variant="icon">
                  <MessagesSquare />
                </EmptyMedia>
                <EmptyTitle>暂无历史会话</EmptyTitle>
                <EmptyDescription>
                  当前 Codex 配置目录中没有找到会话记录。
                </EmptyDescription>
              </EmptyHeader>
            </Empty>
          )}
        </CardContent>
        {sessions.total > 0 && (
          <CardFooter className="justify-between gap-3">
            <div
              className="flex items-center gap-2 text-xs text-muted-foreground"
              aria-live="polite"
            >
              {paging && <Spinner aria-hidden="true" />}
              <span>
                第 {sessions.page} / {totalPages} 页 · 共 {sessions.total} 项
              </span>
            </div>
            <div className="flex items-center gap-1">
              <Button
                size="icon-sm"
                variant="outline"
                disabled={controlsDisabled || sessions.page <= 1}
                aria-label="上一页"
                title="上一页"
                onClick={() => {
                  setPaging(true)
                  setPage(Math.max(1, sessions.page - 1))
                }}
              >
                <ChevronLeft data-icon="inline-start" />
              </Button>
              <Button
                size="icon-sm"
                variant="outline"
                disabled={
                  controlsDisabled ||
                  sessions.page * sessions.pageSize >= sessions.total
                }
                aria-label="下一页"
                title="下一页"
                onClick={() => {
                  setPaging(true)
                  setPage(sessions.page + 1)
                }}
              >
                <ChevronRight data-icon="inline-start" />
              </Button>
            </div>
          </CardFooter>
        )}
      </Card>

      <AlertDialog open={confirming} onOpenChange={setConfirming}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>
              将会话归属更新为{providerLabel(target)}？
            </AlertDialogTitle>
            <AlertDialogDescription>
              不会改动或上传对话内容。Codex 正在使用的会话会保持原样。
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>取消</AlertDialogCancel>
            <AlertDialogAction
              disabled={busy}
              onClick={() => void repairSessions()}
            >
              确认更新
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  )
}
