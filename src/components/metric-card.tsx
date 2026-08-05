import { HugeiconsIcon, type IconSvgElement } from "@hugeicons/react"

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
  icon: IconSvgElement
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
        <CardAction>
          <div className="flex size-8 items-center justify-center rounded-lg bg-secondary text-secondary-foreground ring-1 ring-border [&_svg:not([class*='size-'])]:size-4">
            <HugeiconsIcon icon={Icon} aria-hidden="true" />
          </div>
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
