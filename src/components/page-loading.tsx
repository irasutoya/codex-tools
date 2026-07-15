import { Skeleton } from "@/components/ui/skeleton"

export function PageLoading() {
  return (
    <div className="flex flex-col gap-5" aria-label="正在加载" aria-busy="true">
      <div className="grid gap-3 sm:grid-cols-2 xl:grid-cols-4">
        {Array.from({ length: 4 }, (_, index) => (
          <Skeleton key={index} className="h-24 w-full" />
        ))}
      </div>
      <Skeleton className="h-56 w-full" />
    </div>
  )
}
