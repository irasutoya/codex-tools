import { useCallback, useEffect, useRef, useState } from "react"
import {
  Database01Icon,
  File01Icon,
  InformationCircleIcon,
  Message01Icon,
  Refresh01Icon,
  AiSearchIcon,
  ScanIcon,
  Alert01Icon,
  Wrench01Icon,
  Cancel01Icon,
} from "@hugeicons/core-free-icons"
import { HugeiconsIcon } from "@hugeicons/react"

import { ErrorDetails } from "@/components/error-details"
import { MetricCard } from "@/components/metric-card"
import { SectionHeader } from "@/components/page-header"
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
import {
  InputGroup,
  InputGroupAddon,
  InputGroupButton,
  InputGroupInput,
} from "@/components/ui/input-group"
import {
  Pagination,
  PaginationContent,
  PaginationItem,
  PaginationNext,
  PaginationPrevious,
} from "@/components/ui/pagination"
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
import { notifyRepairWarnings } from "@/lib/repair-feedback"
import { formatDateTime } from "@/lib/time"
import type { PageProps, PageResult, RepairScan, Session } from "@/types"

type ManagedProvider = "openai" | "custom"

function oppositeProvider(provider: string): ManagedProvider {
  return provider === "custom" ? "openai" : "custom"
}

function providerLabel(provider: string) {
  if (provider === "openai") return "OpenAI 账号"
  if (provider === "custom") return "第三方 API"
  return provider || "未识别"
}

function sourceLabel(source: string) {
  if (source === "rollout") return "会话文件"
  if (source === "sqlite") return "会话数据库"
  return source
}

export default function SessionsPage({ active }: PageProps) {
  const [scan, setScan] = useState<RepairScan>()
  const [sessions, setSessions] = useState<PageResult<Session>>()
  const [page, setPage] = useState(1)
  const [query, setQuery] = useState("")
  const [debouncedQuery, setDebouncedQuery] = useState("")
  const [confirming, setConfirming] = useState(false)
  const [error, setError] = useState<string>()
  const [busy, setBusy] = useState(false)
  const [refreshing, setRefreshing] = useState(false)
  const [paging, setPaging] = useState(false)
  const [repairTarget, setRepairTarget] = useState<ManagedProvider>()
  const requestSeq = useRef(0)
  const currentPageRef = useRef(1)
  const loadScan = useCallback(async () => {
    setScan(await call("scan_codex_data"))
  }, [])
  const loadSessions = useCallback(
    async (targetPage: number, refresh = false, search = "") => {
      const requestId = ++requestSeq.current
      currentPageRef.current = targetPage
      try {
        const result = await call("list_sessions", {
          query: search.trim() || undefined,
          page: targetPage,
          pageSize: 50,
          refresh,
        })
        if (requestId !== requestSeq.current) return
        const lastPage = Math.max(1, Math.ceil(result.total / result.pageSize))
        if (targetPage > lastPage) {
          setPaging(true)
          setPage(lastPage)
          return
        }
        setSessions(result)
        setPage(targetPage)
        setPaging(false)
      } catch (error) {
        if (requestId === requestSeq.current) {
          setPaging(false)
          throw error
        }
      }
    },
    []
  )
  const refreshAll = useCallback(async () => {
    await Promise.all([
      loadScan(),
      loadSessions(currentPageRef.current, true, debouncedQuery),
    ])
    setError(undefined)
  }, [debouncedQuery, loadScan, loadSessions])

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
      const nextQuery = query.trim()
      if (nextQuery !== debouncedQuery) {
        setPaging(true)
        setPage(1)
        setDebouncedQuery(nextQuery)
      }
    }, 300)
    return () => window.clearTimeout(timeout)
  }, [active, debouncedQuery, query])

  useEffect(() => {
    if (!active) return
    const timeout = window.setTimeout(() => {
      void loadSessions(page, false, debouncedQuery).catch((reason) => {
        setError(String(reason))
      })
    }, 0)
    return () => window.clearTimeout(timeout)
  }, [active, debouncedQuery, page, loadSessions])

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
      .catch((reason) => notify.error("无法刷新会话列表", reason))
      .finally(() => setRefreshing(false))
  }

  if (error) {
    return (
      <Alert variant="destructive">
        <HugeiconsIcon icon={Alert01Icon} />
        <AlertTitle>无法读取历史会话</AlertTitle>
        <AlertDescription>
          <ErrorDetails
            error={error}
            action={
              <Button variant="outline" disabled={refreshing} onClick={retry}>
                {refreshing ? (
                  <Spinner data-icon="inline-start" aria-hidden="true" />
                ) : (
                  <HugeiconsIcon
                    icon={Refresh01Icon}
                    data-icon="inline-start"
                  />
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
        `修改了 ${result.filesModified} 个会话文件中的 ${result.sessionMetaUpdated} 条连接标记，以及 ${result.rowsUpdated} 条数据库记录。`
      )
      notifyRepairWarnings(result, "会话归属更新已完成")
      setRepairTarget(oppositeProvider(result.targetProvider))
      try {
        await refreshAll()
      } catch (reason) {
        notify.warning("会话归属已更新，但无法读取最新列表", reason)
      }
    } catch (reason) {
      notify.error("无法更新会话归属", reason)
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
        <SectionHeader
          id="session-status-title"
          title="本机会话"
          description="仅统计当前 Codex 配置目录中的会话文件和数据库。"
          actions={
            <div className="flex flex-wrap items-center gap-2">
              <InputGroup className="w-full sm:w-64">
                <InputGroupAddon align="inline-start">
                  <HugeiconsIcon icon={AiSearchIcon} />
                </InputGroupAddon>
                <InputGroupInput
                  type="search"
                  value={query}
                  onChange={(event) => setQuery(event.target.value)}
                  placeholder="搜索标题或项目"
                  aria-label="搜索会话"
                  disabled={controlsDisabled}
                />
                {query && (
                  <InputGroupAddon align="inline-end">
                    <InputGroupButton
                      size="icon-xs"
                      aria-label="清空搜索"
                      title="清空搜索"
                      onClick={() => setQuery("")}
                    >
                      <HugeiconsIcon icon={Cancel01Icon} />
                    </InputGroupButton>
                  </InputGroupAddon>
                )}
              </InputGroup>
              <Button
                size="sm"
                variant="outline"
                disabled={controlsDisabled}
                onClick={refresh}
              >
                {refreshing ? (
                  <Spinner data-icon="inline-start" aria-hidden="true" />
                ) : (
                  <HugeiconsIcon
                    icon={Refresh01Icon}
                    data-icon="inline-start"
                  />
                )}
                {refreshing ? "刷新中…" : "刷新"}
              </Button>
            </div>
          }
        />
        <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-4">
          <MetricCard
            label="会话总数"
            value={sessions.total}
            icon={Message01Icon}
            detail="本机可查看的会话"
          />
          <MetricCard
            label="连接归属"
            value={scan.sessionMetaCount}
            icon={ScanIcon}
            detail={`配置当前使用 ${providerLabel(scan.currentProvider)}`}
          />
          <MetricCard
            label="会话文件"
            value={scan.rolloutFiles}
            icon={File01Icon}
            detail="Codex 保存的对话文件"
          />
          <MetricCard
            label="数据库文件"
            value={scan.databases.length}
            icon={Database01Icon}
            detail="Codex 保存的会话数据库"
          />
        </div>
      </section>

      {scan.warnings.length > 0 && (
        <Alert>
          <HugeiconsIcon icon={Alert01Icon} />
          <AlertTitle>部分会话数据未能检查</AlertTitle>
          <AlertDescription>
            <ErrorDetails error={scan.warnings.join("\n")}>
              已完成其余数据的扫描，共有 {scan.warnings.length}
              项警告。展开详情可查看首批原因。
            </ErrorDetails>
          </AlertDescription>
        </Alert>
      )}

      <Card>
        <CardHeader>
          <CardTitle>更新会话归属</CardTitle>
          <CardDescription>
            将已有会话的连接标记统一更新为 OpenAI 或第三方 API。
          </CardDescription>
          <CardAction>
            <Badge variant="outline">目标：{providerLabel(target)}</Badge>
          </CardAction>
        </CardHeader>
        <CardContent className="flex flex-col gap-4">
          <Alert>
            <HugeiconsIcon icon={InformationCircleIcon} />
            <AlertTitle>只更新归属信息</AlertTitle>
            <AlertDescription>
              只修改本机元数据中的连接标记，不读取或上传对话正文。若文件同时被
              Codex 修改，程序会保留原文件并报告警告。
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
              <HugeiconsIcon icon={Wrench01Icon} data-icon="inline-start" />
            )}
            {busy ? "更新中…" : "更新归属"}
          </Button>
        </CardFooter>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>会话记录</CardTitle>
          <CardDescription>
            列表来自本机 Codex 会话文件和已识别的数据库，不会上传。
          </CardDescription>
        </CardHeader>
        <CardContent className="min-w-0 overflow-x-auto">
          {sessions.items.length ? (
            <Table aria-label="历史会话列表">
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
                    <TableCell
                      className="max-w-80 truncate font-medium"
                      title={session.title || session.id}
                    >
                      {session.title || session.id}
                    </TableCell>
                    <TableCell>
                      <Badge variant="outline">
                        {providerLabel(session.provider)}
                      </Badge>
                    </TableCell>
                    <TableCell
                      className="hidden max-w-64 truncate lg:table-cell"
                      title={session.cwd}
                    >
                      {session.cwd}
                    </TableCell>
                    <TableCell className="whitespace-nowrap tabular-nums">
                      {formatDateTime(session.updatedAt, "compact")}
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          ) : (
            <Empty className="min-h-52 border">
              <EmptyHeader>
                <EmptyMedia variant="icon">
                  <HugeiconsIcon icon={Message01Icon} />
                </EmptyMedia>
                <EmptyTitle>
                  {debouncedQuery ? "未找到匹配会话" : "暂无历史会话"}
                </EmptyTitle>
                <EmptyDescription>
                  {debouncedQuery
                    ? `没有找到标题或项目包含“${debouncedQuery}”的会话。`
                    : "当前 Codex 配置目录中没有找到会话记录。"}
                </EmptyDescription>
              </EmptyHeader>
            </Empty>
          )}
        </CardContent>
        {sessions.total > 0 && (
          <CardFooter className="flex-wrap justify-between gap-3">
            <div
              className="flex items-center gap-2 text-xs text-muted-foreground"
              aria-live="polite"
            >
              {paging && <Spinner aria-hidden="true" />}
              <span>
                第 {sessions.page} / {totalPages} 页 · 共 {sessions.total} 项
              </span>
            </div>
            <Pagination className="m-0 w-fit">
              <PaginationContent>
                <PaginationItem>
                  <PaginationPrevious
                    text="上一页"
                    disabled={controlsDisabled || sessions.page <= 1}
                    onClick={() => {
                      setPaging(true)
                      setPage(Math.max(1, sessions.page - 1))
                    }}
                  />
                </PaginationItem>
                <PaginationItem>
                  <PaginationNext
                    text="下一页"
                    disabled={
                      controlsDisabled ||
                      sessions.page * sessions.pageSize >= sessions.total
                    }
                    onClick={() => {
                      setPaging(true)
                      setPage(sessions.page + 1)
                    }}
                  />
                </PaginationItem>
              </PaginationContent>
            </Pagination>
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
              程序只修改本机会话的连接标记。若 Codex
              同时写入某个会话文件，该文件会保持原样并在完成后报告。
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
