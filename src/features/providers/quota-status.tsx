import { Clock3, Gauge } from "lucide-react"

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
    <div className="flex min-w-0 flex-col gap-2">
      <div className="flex flex-wrap items-center gap-2">
        <span className="flex items-center gap-1.5 text-xs font-medium">
          <Gauge className="size-3.5" aria-hidden="true" />
          额度
        </span>
        <Badge variant={successful ? "default" : "secondary"}>
          {quotaStatusText(quota)}
        </Badge>
        {quota?.fetchedAt && (
          <span className="flex items-center gap-1 text-xs text-muted-foreground">
            <Clock3 className="size-3" aria-hidden="true" />
            {formatTimestamp(quota.fetchedAt)}
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
                className="min-w-0 border-l-2 border-border pl-2"
              >
                <div className="text-xs text-muted-foreground">{row.label}</div>
                <div className="truncate text-sm font-medium">{row.value}</div>
                {(row.detail || row.resetAt) && (
                  <div className="truncate text-xs text-muted-foreground">
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
