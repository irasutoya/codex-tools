import { useCallback, useRef, useState } from "react"

import { errorMessage } from "@/lib/format"
import { toast } from "@/components/ui/toast"

type RunOptions<T> = {
  success?: string
  successDescription?: (result: T) => string | undefined
  onSuccess?: () => void
}

export function useAsyncAction<TKey = string>() {
  const [busy, setBusy] = useState<TKey>()
  const busyRef = useRef<TKey | undefined>(undefined)

  const begin = useCallback((key: TKey) => {
    if (busyRef.current !== undefined) return false
    busyRef.current = key
    setBusy(key)
    return true
  }, [])

  const end = useCallback((key: TKey) => {
    if (busyRef.current !== key) return
    busyRef.current = undefined
    setBusy(undefined)
  }, [])

  const run = useCallback(
    async <T>(
      key: TKey,
      action: () => Promise<T>,
      options?: RunOptions<T>
    ): Promise<boolean> => {
      if (!begin(key)) return false
      try {
        const result = await action()
        if (options?.success) {
          toast.add({
            title: options.success,
            description: options.successDescription?.(result),
            type: "success",
          })
        }
        options?.onSuccess?.()
        return true
      } catch (reason) {
        toast.add({
          title: "操作失败",
          description: errorMessage(reason),
          type: "error",
        })
        return false
      } finally {
        end(key)
      }
    },
    [begin, end]
  )

  return { busy, begin, end, run }
}
