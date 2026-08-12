import { useCallback, useRef, useState } from "react"

import { errorMessage } from "@/lib/format"
import { toast } from "@/components/ui/toast"

type RunOptions = {
  success?: string
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
    async (
      key: TKey,
      action: () => Promise<unknown>,
      options?: RunOptions
    ): Promise<boolean> => {
      if (!begin(key)) return false
      try {
        await action()
        if (options?.success) {
          toast.add({ title: options.success, type: "success" })
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
