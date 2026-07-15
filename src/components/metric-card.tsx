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
    <Card size="sm">
      <CardHeader>
        <CardDescription>{label}</CardDescription>
        <CardAction>
          <Icon className="text-muted-foreground" />
        </CardAction>
      </CardHeader>
      <CardContent className="flex flex-col gap-1">
        <div className="font-heading text-2xl font-semibold tabular-nums">
          {value}
        </div>
        {detail && (
          <div className="text-xs text-muted-foreground">{detail}</div>
        )}
      </CardContent>
    </Card>
  )
}
