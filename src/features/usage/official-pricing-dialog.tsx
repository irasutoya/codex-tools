import { useMemo, useState } from "react"
import {
  Alert02Icon,
  BookOpen01Icon,
  Refresh01Icon,
  Search01Icon,
} from "@hugeicons/core-free-icons"
import { HugeiconsIcon } from "@hugeicons/react"
import { openUrl } from "@tauri-apps/plugin-opener"

import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import {
  Empty,
  EmptyContent,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from "@/components/ui/empty"
import {
  InputGroup,
  InputGroupAddon,
  InputGroupInput,
} from "@/components/ui/input-group"
import { Spinner } from "@/components/ui/spinner"
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table"
import type { OfficialPricingCatalog } from "@/types"

import { cn } from "@/lib/utils"

import { formatDateTime, formatTokens, formatUsdMicrousd } from "./usage-format"

type OfficialPricingDialogProps = {
  open: boolean
  onOpenChange: (open: boolean) => void
  catalog?: OfficialPricingCatalog
  loading: boolean
  error?: string
  onRefresh?: () => void
}

function PriceCell({
  value,
  className,
}: {
  value?: number
  className?: string
}) {
  return (
    <TableCell
      className={cn("text-right whitespace-nowrap tabular-nums", className)}
    >
      {value === undefined ? (
        <span className="text-muted-foreground">—</span>
      ) : (
        formatUsdMicrousd(value)
      )}
    </TableCell>
  )
}

export function OfficialPricingDialog({
  open,
  onOpenChange,
  catalog,
  loading,
  error,
  onRefresh,
}: OfficialPricingDialogProps) {
  const [query, setQuery] = useState("")

  const filtered = useMemo(() => {
    const rates = catalog?.rates ?? []
    const normalized = query.trim().toLowerCase()
    if (!normalized) return rates
    return rates.filter((rate) => rate.model.toLowerCase().includes(normalized))
  }, [catalog?.rates, query])

  const hasCatalog = (catalog?.rates.length ?? 0) > 0

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-h-[calc(100dvh-3rem)] overflow-y-auto sm:max-w-4xl">
        <DialogHeader>
          <DialogTitle>OpenAI 官方实时价格</DialogTitle>
          <DialogDescription>
            每百万 Token 的美元估算，来自官方定价文档；金额仅供参考，不代表
            实际账单。
            {catalog?.fetchedAtMs !== undefined && (
              <> 最近同步：{formatDateTime(catalog.fetchedAtMs)}</>
            )}
          </DialogDescription>
        </DialogHeader>

        {hasCatalog && (
          <div className="flex items-center justify-between gap-3">
            <InputGroup className="max-w-64">
              <InputGroupAddon>
                <HugeiconsIcon icon={Search01Icon} aria-hidden="true" />
              </InputGroupAddon>
              <InputGroupInput
                value={query}
                onChange={(event) => setQuery(event.target.value)}
                placeholder="搜索模型…"
                aria-label="搜索模型"
              />
            </InputGroup>
            <span className="shrink-0 text-xs text-muted-foreground">
              {query.trim()
                ? `匹配 ${filtered.length} / 共 ${catalog?.modelCount ?? 0} 个模型`
                : `共 ${catalog?.modelCount ?? 0} 个模型`}
            </span>
          </div>
        )}

        {loading ? (
          <div className="flex min-h-40 items-center justify-center gap-2 text-muted-foreground">
            <Spinner data-icon="inline-start" />
            正在同步官方价格…
          </div>
        ) : error ? (
          <Empty className="min-h-40 border">
            <EmptyHeader>
              <EmptyMedia variant="icon">
                <HugeiconsIcon icon={Alert02Icon} />
              </EmptyMedia>
              <EmptyTitle>价格同步失败</EmptyTitle>
              <EmptyDescription>{error}</EmptyDescription>
            </EmptyHeader>
            {onRefresh && (
              <EmptyContent>
                <Button size="sm" variant="outline" onClick={onRefresh}>
                  重试同步
                </Button>
              </EmptyContent>
            )}
          </Empty>
        ) : !hasCatalog ? (
          <Empty className="min-h-40 border">
            <EmptyHeader>
              <EmptyMedia variant="icon">
                <HugeiconsIcon icon={BookOpen01Icon} />
              </EmptyMedia>
              <EmptyTitle>还没有官方价格数据</EmptyTitle>
              <EmptyDescription>
                点击“立即同步”获取 OpenAI 官方定价文档后再查看。
              </EmptyDescription>
            </EmptyHeader>
            {onRefresh && (
              <EmptyContent>
                <Button size="sm" variant="outline" onClick={onRefresh}>
                  立即同步
                </Button>
              </EmptyContent>
            )}
          </Empty>
        ) : filtered.length === 0 ? (
          <Empty className="min-h-40 border">
            <EmptyHeader>
              <EmptyMedia variant="icon">
                <HugeiconsIcon icon={Search01Icon} />
              </EmptyMedia>
              <EmptyTitle>没有匹配的模型</EmptyTitle>
              <EmptyDescription>换个关键词试试，或清空搜索。</EmptyDescription>
            </EmptyHeader>
          </Empty>
        ) : (
          <div className="max-h-96 overflow-auto rounded-xl border">
            <Table className="border-separate border-spacing-0">
              <TableHeader>
                <TableRow className="hover:bg-transparent">
                  <TableHead rowSpan={2} className="w-56 border-r">
                    模型
                  </TableHead>
                  <TableHead colSpan={4} className="text-center">
                    短上下文 · $/1M
                  </TableHead>
                  <TableHead colSpan={4} className="border-l text-center">
                    长上下文 · $/1M
                  </TableHead>
                </TableRow>
                <TableRow className="hover:bg-transparent">
                  <TableHead className="text-right">输入</TableHead>
                  <TableHead className="text-right">缓存读</TableHead>
                  <TableHead className="text-right">缓存写</TableHead>
                  <TableHead className="text-right">输出</TableHead>
                  <TableHead className="border-l text-right">输入</TableHead>
                  <TableHead className="text-right">缓存读</TableHead>
                  <TableHead className="text-right">缓存写</TableHead>
                  <TableHead className="text-right">输出</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {filtered.map((rate) => (
                  <TableRow key={rate.model}>
                    <TableCell className="w-56 max-w-56 border-r">
                      <div className="flex items-center gap-1.5">
                        <span
                          className="truncate font-medium"
                          title={rate.model}
                        >
                          {rate.model}
                        </span>
                        {rate.longContextThreshold !== undefined &&
                          rate.long !== undefined && (
                            <Badge
                              variant="secondary"
                              className="shrink-0"
                              title="长上下文"
                            >
                              {formatTokens(rate.longContextThreshold)}
                            </Badge>
                          )}
                      </div>
                    </TableCell>
                    <PriceCell value={rate.short.input} />
                    <PriceCell value={rate.short.cachedInput} />
                    <PriceCell value={rate.short.cacheWrite} />
                    <PriceCell value={rate.short.output} />
                    <PriceCell value={rate.long?.input} className="border-l" />
                    <PriceCell value={rate.long?.cachedInput} />
                    <PriceCell value={rate.long?.cacheWrite} />
                    <PriceCell value={rate.long?.output} />
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </div>
        )}

        <DialogFooter>
          {hasCatalog && onRefresh && (
            <Button
              variant="outline"
              onClick={onRefresh}
              disabled={loading}
              className="mr-auto"
            >
              <HugeiconsIcon icon={Refresh01Icon} data-icon="inline-start" />
              立即同步
            </Button>
          )}
          <Button
            variant="outline"
            onClick={() => void openUrl(catalog?.sourceUrl ?? "")}
            disabled={!catalog?.sourceUrl}
          >
            <HugeiconsIcon icon={BookOpen01Icon} data-icon="inline-start" />
            在浏览器打开官方来源
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
