import { Clock03Icon } from "@hugeicons/core-free-icons"
import { HugeiconsIcon } from "@hugeicons/react"

import { Badge } from "@/components/ui/badge"
import {
  Item,
  ItemActions,
  ItemContent,
  ItemDescription,
  ItemGroup,
  ItemTitle,
} from "@/components/ui/item"
import { Progress } from "@/components/ui/progress"
import { formatDateTime } from "@/lib/time"
import { cn } from "@/lib/utils"
import type { AccountQuota, QuotaStatus } from "@/types"

import { quotaRows, quotaStatusText } from "./quota-format"

const quotaBadgeVariants = {
  success: "default",
  unsupported: "outline",
  unauthorized: "destructive",
  rate_limited: "outline",
  error: "destructive",
  never: "outline",
} as const satisfies Record<QuotaStatus, "default" | "outline" | "destructive">

export function QuotaStatusView({ quota }: { quota?: AccountQuota }) {
  const rows = quotaRows(quota)
  const successful = quota?.status === "success"

  return (
    <div className="flex min-w-0 flex-col gap-2.5">
      <div className="flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
        <Badge variant={quotaBadgeVariants[quota?.status ?? "never"]}>
          {quotaStatusText(quota)}
        </Badge>
        {quota?.fetchedAt && (
          <span className="flex items-center gap-1">
            <HugeiconsIcon
              icon={Clock03Icon}
              className="size-3"
              aria-hidden="true"
            />
            更新于 {formatDateTime(quota.fetchedAt)}
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
          <ItemGroup
            className={cn(
              "grid gap-2",
              rows.length > 1 ? "grid-cols-2" : "grid-cols-1"
            )}
          >
            {rows.map((row) => (
              <Item key={row.label} variant="muted" size="sm">
                <ItemContent className="gap-0.5">
                  <ItemDescription className="line-clamp-none text-xs">
                    {row.label}
                  </ItemDescription>
                  <ItemTitle className="text-base tabular-nums">
                    {row.value}
                  </ItemTitle>
                  {row.remainingPercent != null && (
                    <Progress
                      value={row.remainingPercent}
                      aria-label={`${row.label}剩余额度`}
                      className="mt-0.5 max-w-72"
                    />
                  )}
                </ItemContent>
                {(row.detail || row.resetAt) && (
                  <ItemActions className="ml-auto max-w-[65%]">
                    <ItemDescription className="text-right text-xs tabular-nums">
                      {[
                        row.detail,
                        row.resetAt &&
                          `重置/到期 ${formatDateTime(row.resetAt)}`,
                      ]
                        .filter(Boolean)
                        .join(" · ")}
                    </ItemDescription>
                  </ItemActions>
                )}
              </Item>
            ))}
          </ItemGroup>
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
