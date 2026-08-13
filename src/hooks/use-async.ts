import { useCallback, useEffect, useEffectEvent, useState } from "react"

import { errorMessage } from "@/lib/format"
import type { RequestGate } from "@/lib/request-gate"

type UseAsyncOptions<T> = {
  onError?: (message: string) => void
  onSuccess?: (data: T) => void
  requestGate?: RequestGate
}

export function useAsync<T>(
  fetcher: () => Promise<T>,
  options?: UseAsyncOptions<T>,
  reloadKey?: unknown
) {
  const [data, setData] = useState<T>()
  const [error, setError] = useState<string>()
  const requestGate = options?.requestGate
  const notifySuccess = useEffectEvent((next: T) => {
    options?.onSuccess?.(next)
  })
  const notifyError = useEffectEvent((message: string) => {
    options?.onError?.(message)
  })

  useEffect(() => {
    let cancelled = false
    const request = requestGate?.begin()
    void fetcher()
      .then((next) => {
        if (
          cancelled ||
          (request !== undefined && !requestGate?.isCurrent(request))
        )
          return
        setData(next)
        setError(undefined)
        notifySuccess(next)
      })
      .catch((reason) => {
        if (
          cancelled ||
          (request !== undefined && !requestGate?.isCurrent(request))
        )
          return
        const message = errorMessage(reason)
        setError(message)
        notifyError(message)
      })
      .finally(() => {
        if (request !== undefined) requestGate?.finish(request)
      })
    return () => {
      cancelled = true
      if (request !== undefined) requestGate?.finish(request)
    }
  }, [fetcher, reloadKey, requestGate])

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
