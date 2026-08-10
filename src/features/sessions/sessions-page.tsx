import { useEffect, useRef, useState } from "react"
import {
  Database01Icon,
  Folder01Icon,
  InformationCircleIcon,
  Wrench01Icon,
} from "@hugeicons/core-free-icons"
import { HugeiconsIcon } from "@hugeicons/react"

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
  CardContent,
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
  ItemMedia,
  ItemTitle,
} from "@/components/ui/item"
import {
  Pagination,
  PaginationContent,
  PaginationItem,
  PaginationNext,
  PaginationPrevious,
} from "@/components/ui/pagination"
import { Skeleton } from "@/components/ui/skeleton"
import { Spinner } from "@/components/ui/spinner"
import { toast } from "@/components/ui/toast"
import { errorMessage, formatDate, formatInteger } from "@/lib/format"
import { call } from "@/lib/ipc"
import type { PageResult, RepairScan, RepairTarget, Session } from "@/types"

const pageSize = 6

export function SessionsPage({
  refreshRevision,
  query,
  onRefresh,
}: {
  refreshRevision: number
  onRefresh: () => void
  query: string
}) {
  const [page, setPage] = useState(1)
  const [result, setResult] = useState<PageResult<Session>>()
  const [scan, setScan] = useState<RepairScan>()
  const [repairTarget, setRepairTarget] = useState<RepairTarget>()
  const [busy, setBusy] = useState(false)
  const [listError, setListError] = useState<string>()
  const [scanError, setScanError] = useState<string>()
  const loadedRefreshRevision = useRef(refreshRevision)

  useEffect(() => {
    let cancelled = false
    const forceRefresh = refreshRevision !== loadedRefreshRevision.current
    loadedRefreshRevision.current = refreshRevision
    void call("sessions_list", {
      query,
      page,
      pageSize,
      refresh: forceRefresh,
    })
      .then((nextResult) => {
        if (cancelled) return
        setResult(nextResult)
        if (nextResult.page !== page) setPage(nextResult.page)
        setListError(undefined)
      })
      .catch((reason) => {
        if (cancelled) return
        const message = errorMessage(reason)
        setListError(message)
        toast.add({
          title: "无法读取会话列表",
          description: message,
          type: "error",
        })
      })
    return () => {
      cancelled = true
    }
  }, [page, query, refreshRevision])

  useEffect(() => {
    let cancelled = false
    void call("sessions_scan")
      .then((nextScan) => {
        if (cancelled) return
        setScan(nextScan)
        setScanError(undefined)
      })
      .catch((reason) => {
        if (cancelled) return
        const message = errorMessage(reason)
        setScanError(message)
        toast.add({
          title: "无法扫描会话",
          description: message,
          type: "error",
        })
      })
    return () => {
      cancelled = true
    }
  }, [refreshRevision])

  const repair = async () => {
    if (!repairTarget) return
    setBusy(true)
    try {
      const response = await call("sessions_repair", {
        targetProvider: repairTarget.id,
      })
      toast.add({
        title: "会话归属已修复",
        description: `已更新 ${response.rowsUpdated} 条记录。`,
        type: "success",
      })
      setRepairTarget(undefined)
      onRefresh()
    } catch (reason) {
      toast.add({
        title: "修复失败",
        description: errorMessage(reason),
        type: "error",
      })
    } finally {
      setBusy(false)
    }
  }

  if ((!result && listError) || (!scan && scanError))
    return (
      <div className="flex min-h-full flex-col gap-3 px-3 pt-1 pb-3">
        {listError && (
          <Alert variant="destructive">
            <HugeiconsIcon icon={InformationCircleIcon} />
            <AlertTitle>无法读取会话列表</AlertTitle>
            <AlertDescription>{listError}</AlertDescription>
          </Alert>
        )}
        {scanError && (
          <Alert variant="destructive">
            <HugeiconsIcon icon={InformationCircleIcon} />
            <AlertTitle>无法扫描会话</AlertTitle>
            <AlertDescription>{scanError}</AlertDescription>
          </Alert>
        )}
      </div>
    )

  if (!result || !scan)
    return (
      <div className="grid grid-rows-[72px_256px] gap-3 px-3 pt-1 pb-3">
        <Skeleton className="rounded-2xl" />
        <Skeleton className="rounded-2xl" />
      </div>
    )
  const pageCount = Math.max(1, Math.ceil(result.total / pageSize))
  const currentPage = result.page

  return (
    <div className="flex min-h-full flex-col gap-3 px-3 pt-1 pb-3">
      {listError && (
        <Alert variant="destructive">
          <HugeiconsIcon icon={InformationCircleIcon} />
          <AlertTitle>会话列表刷新失败</AlertTitle>
          <AlertDescription>{listError}</AlertDescription>
        </Alert>
      )}
      {scanError && (
        <Alert variant="destructive">
          <HugeiconsIcon icon={InformationCircleIcon} />
          <AlertTitle>会话扫描失败</AlertTitle>
          <AlertDescription>{scanError}</AlertDescription>
        </Alert>
      )}
      <Card size="sm" className="shrink-0">
        <CardContent className="grid grid-cols-[1fr_1fr_auto] items-center gap-4">
          <Summary
            icon={Database01Icon}
            label="已索引会话"
            value={formatInteger(scan.sessionMetaCount)}
          />
          <Summary
            icon={Folder01Icon}
            label="Rollout 文件"
            value={formatInteger(scan.rolloutFiles)}
          />
          <div className="flex gap-2">
            {scan.targets
              .filter((target) => !target.current && target.count > 0)
              .slice(0, 1)
              .map((target) => (
                <Button
                  key={target.id}
                  size="sm"
                  variant="outline"
                  onClick={() => setRepairTarget(target)}
                >
                  <HugeiconsIcon icon={Wrench01Icon} data-icon="inline-start" />
                  修复 {target.count} 条
                </Button>
              ))}
          </div>
        </CardContent>
      </Card>
      <Card size="sm" className="min-h-64 shrink-0">
        <CardHeader className="grid grid-cols-[1fr_auto] items-center">
          <div>
            <CardTitle>会话</CardTitle>
            <div className="mt-1 text-xs text-muted-foreground">
              {query ? `“${query}” 的结果` : "全部本地会话"}
            </div>
          </div>
          <Badge variant="outline">{result.total} 条</Badge>
        </CardHeader>
        <CardContent>
          {result.items.length ? (
            <ItemGroup>
              {result.items.map((session) => (
                <Item
                  key={session.identity}
                  size="xs"
                  variant="outline"
                  className="flex-nowrap"
                >
                  <ItemMedia variant="icon">
                    <HugeiconsIcon
                      icon={session.archived ? Database01Icon : Folder01Icon}
                    />
                  </ItemMedia>
                  <ItemContent className="min-w-0">
                    <ItemTitle className="w-full min-w-0">
                      <span
                        className="min-w-0 truncate"
                        title={session.title || "未命名会话"}
                      >
                        {session.title || "未命名会话"}
                      </span>
                      <Badge variant="secondary" className="shrink-0">
                        {session.provider}
                      </Badge>
                    </ItemTitle>
                    <ItemDescription className="truncate">
                      {session.cwd}
                    </ItemDescription>
                  </ItemContent>
                  <ItemActions className="shrink-0">
                    <div className="text-right text-xs text-muted-foreground">
                      <div>{formatDate(session.updatedAt, true)}</div>
                      <div>{session.archived ? "已归档" : "活跃"}</div>
                    </div>
                  </ItemActions>
                </Item>
              ))}
            </ItemGroup>
          ) : (
            <Empty>
              <EmptyHeader>
                <EmptyMedia variant="icon">
                  <HugeiconsIcon icon={InformationCircleIcon} />
                </EmptyMedia>
                <EmptyTitle>没有匹配的会话</EmptyTitle>
                <EmptyDescription>调整搜索词或刷新本地索引。</EmptyDescription>
              </EmptyHeader>
            </Empty>
          )}
        </CardContent>
        {pageCount > 1 && (
          <CardFooter className="px-3 py-1.5">
            <Pagination>
              <PaginationContent>
                <PaginationItem>
                  <PaginationPrevious
                    href="#"
                    text="上一页"
                    aria-disabled={currentPage === 1}
                    onClick={(event) => {
                      event.preventDefault()
                      if (currentPage > 1) setPage(currentPage - 1)
                    }}
                  />
                </PaginationItem>
                <PaginationItem>
                  <span className="px-2 text-xs text-muted-foreground">
                    {currentPage} / {pageCount}
                  </span>
                </PaginationItem>
                <PaginationItem>
                  <PaginationNext
                    href="#"
                    text="下一页"
                    aria-disabled={currentPage === pageCount}
                    onClick={(event) => {
                      event.preventDefault()
                      if (currentPage < pageCount) setPage(currentPage + 1)
                    }}
                  />
                </PaginationItem>
              </PaginationContent>
            </Pagination>
          </CardFooter>
        )}
      </Card>

      <AlertDialog
        open={Boolean(repairTarget)}
        onOpenChange={(open) => !open && setRepairTarget(undefined)}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>修复会话归属？</AlertDialogTitle>
            <AlertDialogDescription>
              将 {repairTarget?.count ?? 0} 条来自{" "}
              {repairTarget?.sources.join("、")} 的会话更新为当前提供方“
              {scan.currentProvider}”。操作会修改本地会话元数据。
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>取消</AlertDialogCancel>
            <AlertDialogAction disabled={busy} onClick={() => void repair()}>
              {busy && <Spinner data-icon="inline-start" />}确认修复
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  )
}

function Summary({
  icon,
  label,
  value,
}: {
  icon: typeof Database01Icon
  label: string
  value: string
}) {
  return (
    <div className="flex items-center gap-2">
      <div className="flex size-8 items-center justify-center rounded-xl bg-muted">
        <HugeiconsIcon icon={icon} />
      </div>
      <div>
        <div className="text-xs text-muted-foreground">{label}</div>
        <div className="font-medium tabular-nums">{value}</div>
      </div>
    </div>
  )
}
