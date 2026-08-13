import { useCallback, useEffect, useEffectEvent, useState } from "react"

import { errorMessage } from "@/lib/format"

type UseAsyncOptions<T> = {
  onError?: (message: string) => void
  onSuccess?: (data: T) => void
}

export function useAsync<T>(
  fetcher: () => Promise<T>,
  options?: UseAsyncOptions<T>,
  reloadKey?: unknown
) {
  const [data, setData] = useState<T>()
  const [error, setError] = useState<string>()
  const notifySuccess = useEffectEvent((next: T) => {
    options?.onSuccess?.(next)
  })
  const notifyError = useEffectEvent((message: string) => {
    options?.onError?.(message)
  })

  useEffect(() => {
    let cancelled = false
    void fetcher()
      .then((next) => {
        if (cancelled) return
        setData(next)
        setError(undefined)
        notifySuccess(next)
      })
      .catch((reason) => {
        if (cancelled) return
        const message = errorMessage(reason)
        setError(message)
        notifyError(message)
      })
    return () => {
      cancelled = true
    }
  }, [fetcher, reloadKey])

  const mutate = useCallback((next: T | ((current: T | undefined) => T)) => {
    setData((current) => {
      const value =
        typeof next === "function"
          ? (next as (current: T | undefined) => T)(current)
          : next
      return value
    })
    setError(undefined)
  }, [])

  return { data, error, mutate }
}
