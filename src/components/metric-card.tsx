import type { LucideIcon } from "lucide-react"

import {
  Card,
  CardAction,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card"

type MetricCardProps = {
  label: string
  value: string | number
  detail?: string
  icon: LucideIcon
}

export function MetricCard({
  label,
  value,
  detail,
  icon: Icon,
}: MetricCardProps) {
  return (
    <Card size="sm" className="min-h-28">
      <CardHeader>
        <CardDescription>{label}</CardDescription>
        <CardAction className="text-muted-foreground">
          <Icon className="size-4" aria-hidden="true" />
        </CardAction>
      </CardHeader>
      <CardContent className="flex flex-col gap-0.5">
        <CardTitle className="text-xl tabular-nums">{value}</CardTitle>
        {detail && (
          <p className="line-clamp-2 text-xs leading-relaxed text-muted-foreground">
            {detail}
          </p>
        )}
      </CardContent>
    </Card>
  )
}
