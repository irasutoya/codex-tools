import type { LucideIcon } from "lucide-react"
import type { ReactNode } from "react"

type PageHeaderProps = {
  title: string
  description: string
  icon: LucideIcon
  actions?: ReactNode
}

export function PageHeader({
  title,
  description,
  icon: Icon,
  actions,
}: PageHeaderProps) {
  return (
    <div className="flex items-start justify-between gap-6">
      <div className="flex min-w-0 items-start gap-3">
        <div className="flex size-10 shrink-0 items-center justify-center rounded-xl bg-secondary text-secondary-foreground ring-1 ring-border">
          <Icon className="size-4" aria-hidden="true" />
        </div>
        <div className="min-w-0">
          <h1 className="font-heading text-2xl leading-tight font-semibold tracking-tight">
            {title}
          </h1>
          <p className="mt-1 max-w-3xl text-sm leading-relaxed text-muted-foreground">
            {description}
          </p>
        </div>
      </div>
      {actions && (
        <div className="flex shrink-0 flex-wrap items-center justify-end gap-2">
          {actions}
        </div>
      )}
    </div>
  )
}

type SectionHeaderProps = {
  title: string
  description?: string
  actions?: ReactNode
  id?: string
}

export function SectionHeader({
  title,
  description,
  actions,
  id,
}: SectionHeaderProps) {
  return (
    <div className="flex items-end justify-between gap-6">
      <div className="min-w-0">
        <h2 id={id} className="font-heading text-base font-medium">
          {title}
        </h2>
        {description && (
          <p className="mt-1 max-w-3xl text-sm leading-relaxed text-muted-foreground">
            {description}
          </p>
        )}
      </div>
      {actions && (
        <div className="flex shrink-0 flex-wrap items-center justify-end gap-2">
          {actions}
        </div>
      )}
    </div>
  )
}
