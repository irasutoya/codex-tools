import { useCallback, useEffect, useState } from "react"
import {
  ChevronLeft,
  ChevronRight,
  Database,
  FileText,
  MessagesSquare,
  RefreshCw,
  ScanSearch,
  Wrench,
} from "lucide-react"
import { toast } from "sonner"

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
import type { PageResult, RepairResult, RepairScan, Session } from "@/types"

type ManagedProvider = "openai" | "custom"

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

export default function SessionsPage() {
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
    setScan(await call<RepairScan>("scan_codex_data"))
  }, [])
  const loadSessions = useCallback(
    async (refresh = false) => {
      setSessions(
        await call<PageResult<Session>>("list_sessions", {
          page,
          pageSize: 50,
          refresh,
        })
      )
    },
    [page]
  )
  const refreshAll = useCallback(async () => {
    await Promise.all([loadScan(), loadSessions(true)])
    setError(undefined)
  }, [loadScan, loadSessions])

  useEffect(() => {
    const timeout = window.setTimeout(() => {
      void loadScan().catch((reason) => setError(String(reason)))
    }, 0)
    return () => window.clearTimeout(timeout)
  }, [loadScan])

  useEffect(() => {
    const timeout = window.setTimeout(() => {
      void loadSessions()
        .catch((reason) => setError(String(reason)))
        .finally(() => setPaging(false))
    }, 0)
    return () => window.clearTimeout(timeout)
  }, [loadSessions])

  const retry = () => {
    setRefreshing(true)
    void refreshAll()
      .catch((reason) => setError(String(reason)))
      .finally(() => setRefreshing(false))
  }

  const refresh = () => {
    setRefreshing(true)
    void refreshAll()
      .catch((reason) => toast.error(String(reason)))
      .finally(() => setRefreshing(false))
  }

  if (error) {
    return (
      <Alert variant="destructive">
        <AlertTitle>暂时无法读取历史会话</AlertTitle>
        <AlertDescription className="flex flex-col items-start gap-3">
          <span>{error}</span>
          <Button variant="outline" disabled={refreshing} onClick={retry}>
            {refreshing ? (
              <Spinner data-icon="inline-start" aria-hidden="true" />
            ) : (
              <RefreshCw data-icon="inline-start" />
            )}
            {refreshing ? "重试中…" : "重试"}
          </Button>
        </AlertDescription>
      </Alert>
    )
  }

  if (!scan || !sessions) return <PageLoading />

  const sessionProviders = scan.targets.filter((item) =>
    item.sources.some((source) => source === "rollout" || source === "sqlite")
  )
  const unifiedProvider =
    sessionProviders.length === 1 &&
    (sessionProviders[0].id === "openai" || sessionProviders[0].id === "custom")
      ? sessionProviders[0].id
      : scan.currentProvider
  const target = repairTarget ?? oppositeProvider(unifiedProvider)
  const totalPages = Math.max(1, Math.ceil(sessions.total / sessions.pageSize))
  const controlsDisabled = refreshing || paging || busy

  return (
    <div className="flex flex-col gap-6">
      <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
        <MetricCard
          label="历史会话"
          value={sessions.total}
          icon={MessagesSquare}
          detail="在本机找到的记录"
        />
        <MetricCard
          label="归属记录"
          value={scan.sessionMetaCount}
          icon={ScanSearch}
          detail={`当前标记为${providerLabel(scan.currentProvider)}`}
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

      <Card size="sm">
        <CardHeader>
          <CardTitle>修复会话归属</CardTitle>
          <CardDescription>
            切换 OpenAI 账号和第三方 API 后，可更新旧会话的归属信息。
          </CardDescription>
          <CardAction className="max-sm:col-span-2 max-sm:row-start-auto max-sm:justify-self-start">
            <Badge variant="outline">改为{providerLabel(target)}</Badge>
          </CardAction>
        </CardHeader>
        <CardContent className="flex flex-col gap-3">
          <Alert>
            <Wrench />
            <AlertTitle>不会修改会话内容</AlertTitle>
            <AlertDescription>
              此操作只更新会话使用的连接方式。如果 Codex
              正在写入某个会话，该文件会保留原样并提示稍后重试。
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
                    发现位置：
                    {item.sources.map(sourceLabel).join("、") || "未知"}
                  </ItemDescription>
                </ItemContent>
                <ItemActions>
                  <Badge variant={item.current ? "default" : "secondary"}>
                    {item.current ? "当前归属" : "可以切换"}
                  </Badge>
                </ItemActions>
              </Item>
            ))}
          </ItemGroup>
        </CardContent>
        <CardFooter className="justify-end">
          <Button disabled={busy} onClick={() => setConfirming(true)}>
            {busy ? (
              <Spinner data-icon="inline-start" aria-hidden="true" />
            ) : (
              <Wrench data-icon="inline-start" />
            )}
            {busy ? "正在更新…" : "更新会话归属"}
          </Button>
        </CardFooter>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>会话记录</CardTitle>
          <CardDescription>
            直接读取 Codex 保存在本机的会话，不会复制或上传对话内容。
          </CardDescription>
          <CardAction className="max-sm:col-span-2 max-sm:row-start-auto max-sm:justify-self-start">
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
              {refreshing ? "正在刷新…" : "刷新列表"}
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
                    <TableCell className="tabular-nums">
                      {new Date(session.updatedAt * 1000).toLocaleString()}
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
                <EmptyTitle>还没有可显示的会话</EmptyTitle>
                <EmptyDescription>
                  Codex 配置目录中暂时没有找到历史会话。
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
              将历史会话改为{providerLabel(target)}？
            </AlertDialogTitle>
            <AlertDialogDescription>
              这只会更新会话的连接归属，不会改动、复制或上传对话内容。正在由
              Codex 写入的文件会保持原样。
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>取消</AlertDialogCancel>
            <AlertDialogAction
              disabled={busy}
              onClick={() => {
                setConfirming(false)
                setBusy(true)
                void call<RepairResult>("repair_codex_data", {
                  targetProvider: target,
                })
                  .then((result) => {
                    toast.success(
                      `会话归属已更新：${result.filesModified} 个文件、${result.rowsUpdated} 条数据库记录`
                    )
                    result.warnings.forEach((warning) => toast.warning(warning))
                    setRepairTarget(oppositeProvider(result.targetProvider))
                    return refreshAll()
                  })
                  .catch((reason) => toast.error(String(reason)))
                  .finally(() => setBusy(false))
              }}
            >
              确认更新
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  )
}
