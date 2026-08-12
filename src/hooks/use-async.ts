import { useCallback, useEffect, useRef, useState } from "react"

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
  const optionsRef = useRef(options)
  const fetcherRef = useRef(fetcher)

  useEffect(() => {
    optionsRef.current = options
  })
  useEffect(() => {
    fetcherRef.current = fetcher
  })

  useEffect(() => {
    let cancelled = false
    void fetcherRef
      .current()
      .then((next) => {
        if (cancelled) return
        setData(next)
        setError(undefined)
        optionsRef.current?.onSuccess?.(next)
      })
      .catch((reason) => {
        if (cancelled) return
        const message = errorMessage(reason)
        setError(message)
        optionsRef.current?.onError?.(message)
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
