import { HugeiconsIcon, type IconSvgElement } from "@hugeicons/react"
import type { ReactNode } from "react"

type PageHeaderProps = {
  title: string
  description: string
  icon: IconSvgElement
  actions?: ReactNode
}

export function PageHeader({
  title,
  description,
  icon: Icon,
  actions,
}: PageHeaderProps) {
  return (
    <div className="flex flex-col gap-4 sm:flex-row sm:items-start sm:justify-between sm:gap-6">
      <div className="flex min-w-0 items-start gap-3">
        <div className="flex size-10 shrink-0 items-center justify-center rounded-lg bg-secondary text-secondary-foreground ring-1 ring-border [&_svg:not([class*='size-'])]:size-4">
          <HugeiconsIcon icon={Icon} aria-hidden="true" />
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
        <div className="flex w-full flex-wrap items-center justify-start gap-2 sm:w-auto sm:shrink-0 sm:justify-end">
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
    <div className="flex flex-col gap-3 sm:flex-row sm:items-end sm:justify-between sm:gap-6">
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
        <div className="flex w-full flex-wrap items-center justify-start gap-2 sm:w-auto sm:shrink-0 sm:justify-end">
          {actions}
        </div>
      )}
    </div>
  )
}
