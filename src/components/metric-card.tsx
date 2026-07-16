import type { LucideIcon } from "lucide-react"
import type { ReactNode } from "react"

import {
  Card,
  CardAction,
  CardContent,
  CardDescription,
  CardHeader,
} from "@/components/ui/card"

type MetricCardProps = {
  label: string
  value: ReactNode
  icon: LucideIcon
  detail?: ReactNode
}

export function MetricCard({
  label,
  value,
  icon: Icon,
  detail,
}: MetricCardProps) {
  return (
    <Card
      size="sm"
      className="min-h-32 bg-[var(--md-sys-color-surface-container)]"
    >
      <CardHeader>
        <CardDescription>{label}</CardDescription>
        <CardAction className="flex size-10 items-center justify-center rounded-full bg-[var(--md-sys-color-primary-container)] text-[var(--md-sys-color-on-primary-container)]">
          <Icon className="size-5" />
        </CardAction>
      </CardHeader>
      <CardContent className="flex flex-col gap-1">
        <div className="font-heading text-[2rem] leading-10 font-normal tracking-tight tabular-nums">
          {value}
        </div>
        {detail && (
          <div className="text-xs text-muted-foreground">{detail}</div>
        )}
      </CardContent>
    </Card>
  )
}
