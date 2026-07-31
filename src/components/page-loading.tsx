import { Skeleton } from "@/components/ui/skeleton"

export function PageLoading({ label = "正在加载" }: { label?: string }) {
  return (
    <div
      className="flex flex-col gap-5"
      role="status"
      aria-live="polite"
      aria-label={label}
    >
      <span className="sr-only">{label}</span>
      <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-4">
        {Array.from({ length: 4 }).map((_, index) => (
          <div
            key={index}
            className="flex flex-col gap-3 rounded-xl bg-card p-4 ring-1 ring-foreground/10"
          >
            <Skeleton className="h-4 w-24" />
            <Skeleton className="h-8 w-16" />
            <Skeleton className="h-3 w-32" />
          </div>
        ))}
      </div>
      <Skeleton className="h-56 w-full rounded-xl" />
    </div>
  )
}
