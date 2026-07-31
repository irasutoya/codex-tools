import { Clock3 } from "lucide-react"

import { Badge } from "@/components/ui/badge"
import { epochMilliseconds } from "@/lib/time"
import { cn } from "@/lib/utils"
import type { AccountQuota } from "@/types"

import { quotaRows, quotaStatusText } from "./quota-format"

const quotaTimestampFormatter = new Intl.DateTimeFormat("zh-CN", {
  dateStyle: "short",
  timeStyle: "short",
})

export function QuotaStatusView({
  quota,
  compact = false,
}: {
  quota?: AccountQuota
  compact?: boolean
}) {
  const rows = quotaRows(quota)
  const successful = quota?.status === "success"

  return (
    <div className="flex min-w-0 flex-col gap-2.5">
      <div className="flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
        <Badge variant={successful ? "secondary" : "outline"}>
          {quotaStatusText(quota)}
        </Badge>
        {quota?.fetchedAt && (
          <span className="flex items-center gap-1">
            <Clock3 className="size-3" aria-hidden="true" />
            更新于 {formatTimestamp(quota.fetchedAt)}
          </span>
        )}
      </div>
      {rows.length > 0 && (
        <>
          {!successful && (
            <p className="text-xs text-muted-foreground">
              以下为上次成功查询结果
            </p>
          )}
          <div
            className={cn(
              "grid gap-2",
              compact ? "sm:grid-cols-2" : "md:grid-cols-2"
            )}
          >
            {rows.map((row) => (
              <div
                key={row.label}
                className="min-w-0 rounded-lg bg-muted/60 px-3 py-2.5"
              >
                <div className="text-xs text-muted-foreground">{row.label}</div>
                <div className="mt-0.5 truncate text-sm font-semibold tabular-nums">
                  {row.value}
                </div>
                {(row.detail || row.resetAt) && (
                  <div className="mt-0.5 truncate text-xs text-muted-foreground">
                    {[
                      row.detail,
                      row.resetAt &&
                        `重置/到期 ${formatTimestamp(row.resetAt)}`,
                    ]
                      .filter(Boolean)
                      .join(" · ")}
                  </div>
                )}
              </div>
            ))}
          </div>
        </>
      )}
      {quota?.error && !successful && (
        <p className="max-h-20 overflow-auto text-xs break-all text-muted-foreground">
          {quota.error}
        </p>
      )}
    </div>
  )
}

function formatTimestamp(value: number) {
  const date = new Date(epochMilliseconds(value))
  if (Number.isNaN(date.getTime())) return "时间未知"
  return quotaTimestampFormatter.format(date)
}
