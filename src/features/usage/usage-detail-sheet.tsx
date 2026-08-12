import { useState } from "react"
import { Progress, ProgressLabel } from "@/components/ui/progress"
import {
  Sheet,
  SheetBody,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
} from "@/components/ui/sheet"
import { cacheHitRate, formatInteger, formatPercent } from "@/lib/format"
import type { UsageGroupBy, UsageRow } from "@/types"

export function UsageDetail({
  row,
  groupBy,
  onOpenChange,
}: {
  row?: UsageRow
  groupBy: UsageGroupBy
  onOpenChange: (open: boolean) => void
}) {
  const [lastRow, setLastRow] = useState<UsageRow>()
  if (row && row !== lastRow) setLastRow(row)
  const display = row ?? lastRow

  if (!display) return null
  const hitRate = cacheHitRate(display.tokens)
  const details = [
    ["普通输入", display.tokens.inputTokens],
    ["缓存输入", display.tokens.cachedInputTokens],
    ["缓存写入", display.tokens.cacheWriteInputTokens],
    ["普通输出", display.tokens.outputTokens],
    ["推理输出", display.tokens.reasoningOutputTokens],
  ] as const
  return (
    <Sheet open={Boolean(row)} onOpenChange={onOpenChange}>
      <SheetContent>
        <SheetHeader>
          <SheetTitle>
            {groupBy === "model" ? display.model : display.sourceName}
          </SheetTitle>
          <SheetDescription>
            {groupBy === "model" ? "模型" : "账号"}汇总的 Token 构成
          </SheetDescription>
        </SheetHeader>
        <SheetBody className="grid content-start gap-2">
          <Progress
            value={hitRate ?? 0}
            className="rounded-2xl bg-muted/40 p-3"
          >
            <ProgressLabel>缓存命中率</ProgressLabel>
            <span className="ml-auto text-sm text-muted-foreground tabular-nums">
              {formatPercent(hitRate)}
            </span>
          </Progress>
          {details.map(([label, value]) => (
            <div
              key={label}
              className="flex items-center justify-between rounded-2xl bg-muted px-3 py-2"
            >
              <span className="text-muted-foreground">{label}</span>
              <span className="font-medium tabular-nums">
                {formatInteger(value)}
              </span>
            </div>
          ))}
          <div className="flex items-center justify-between border-t pt-4">
            <span className="font-medium">合计</span>
            <span className="text-lg font-medium tabular-nums">
              {formatInteger(display.tokens.totalTokens)}
            </span>
          </div>
        </SheetBody>
      </SheetContent>
    </Sheet>
  )
}
