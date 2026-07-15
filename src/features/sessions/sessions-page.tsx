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

export default function SessionsPage() {
  const [scan, setScan] = useState<RepairScan>()
  const [sessions, setSessions] = useState<PageResult<Session>>()
  const [page, setPage] = useState(1)
  const [confirming, setConfirming] = useState(false)
  const [error, setError] = useState<string>()
  const [busy, setBusy] = useState(false)
  const [refreshing, setRefreshing] = useState(false)
  const [paging, setPaging] = useState(false)
  const [migrationTarget, setMigrationTarget] = useState<ManagedProvider>()
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
        <AlertTitle>无法读取 Codex 会话</AlertTitle>
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
  const target = migrationTarget ?? oppositeProvider(unifiedProvider)
  const totalPages = Math.max(1, Math.ceil(sessions.total / sessions.pageSize))
  const controlsDisabled = refreshing || paging || busy

  return (
    <div className="flex flex-col gap-5">
      <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-4">
        <MetricCard
          label="会话总数"
          value={sessions.total}
          icon={MessagesSquare}
          detail="当前会话索引"
        />
        <MetricCard
          label="迁移元数据"
          value={scan.sessionMetaCount}
          icon={ScanSearch}
          detail={`当前 ${scan.currentProvider}`}
        />
        <MetricCard
          label="JSONL 文件"
          value={scan.rolloutFiles}
          icon={FileText}
          detail="已识别的 rollout"
        />
        <MetricCard
          label="SQLite 来源"
          value={scan.databases.length}
          icon={Database}
          detail="已识别的数据库"
        />
      </div>

      <Card size="sm">
        <CardHeader>
          <CardTitle>统一迁移</CardTitle>
          <CardDescription>
            仅修改 provider 元数据，不复制会话正文或创建备份。
          </CardDescription>
          <CardAction>
            <Badge variant="outline">目标 {target}</Badge>
          </CardAction>
        </CardHeader>
        <CardContent className="flex flex-col gap-3">
          <Alert>
            <Wrench />
            <AlertTitle>迁移按当前数据执行</AlertTitle>
            <AlertDescription>
              若 Codex
              正在写入文件，该文件会安全中止迁移；再次执行时会重新扫描并重试。
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
                  <ItemTitle>{item.id}</ItemTitle>
                  <ItemDescription>
                    来源：{item.sources.join(" / ") || "未知"}
                  </ItemDescription>
                </ItemContent>
                <ItemActions>
                  <Badge variant={item.current ? "default" : "secondary"}>
                    {item.current ? "当前" : "可迁移"}
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
            {busy ? "迁移中…" : "执行迁移"}
          </Button>
        </CardFooter>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>会话索引</CardTitle>
          <CardDescription>
            直接读取 Codex JSONL 和 SQLite，不保存会话正文。
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
        <CardContent>
          {sessions.items.length ? (
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>标题</TableHead>
                  <TableHead>Provider</TableHead>
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
                        {session.provider || "未知"}
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
                <EmptyTitle>未找到会话</EmptyTitle>
                <EmptyDescription>
                  当前 Codex 数据目录中没有可显示的会话记录。
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
            <AlertDialogTitle>确认迁移到 {target}？</AlertDialogTitle>
            <AlertDialogDescription>
              仅修改识别出的 provider 字段，不备份或复制会话正文。若 Codex
              正在写入文件，迁移会安全中止并要求重试。
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
                      `修改 ${result.filesModified} 个文件、${result.rowsUpdated} 行`
                    )
                    result.warnings.forEach((warning) => toast.warning(warning))
                    setMigrationTarget(oppositeProvider(result.targetProvider))
                    return refreshAll()
                  })
                  .catch((reason) => toast.error(String(reason)))
                  .finally(() => setBusy(false))
              }}
            >
              确认迁移
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  )
}
