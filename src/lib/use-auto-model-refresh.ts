import { useEffect, useRef } from "react"

/**
 * 第三方 API 模型列表的自动同步（用户无感知）。
 *
 * 服务商可能随时新增/下线模型，本应用在设置页处于前台时：
 * - 进入页面：若距离上次成功同步超过 5 分钟，静默刷新一次；
 * - 之后每 10 分钟同步一次；
 * - 失败时按 30s / 1m / 2m / 5m 退避，连续失败 5 次后暂停，直到下次进入页面。
 * 运行状态按服务 id 保存在模块级，跨页面切换保持节流。
 */

const MODEL_REFRESH_INTERVAL_MS = 10 * 60 * 1000
const MODEL_REFRESH_ON_ENTER_MS = 5 * 60 * 1000
const MODEL_REFRESH_MINIMUM_MS = 30 * 1000
const MODEL_REFRESH_MAX_FAILURES = 5
const MODEL_REFRESH_BACKOFF_MS = [
  30_000,
  60_000,
  2 * 60_000,
  5 * 60_000,
] as const

type ModelRefreshRuntime = {
  failureCount: number
  lastAttemptAt?: number
  lastSuccessAt?: number
  suspended: boolean
}

const runtimes = new Map<string, ModelRefreshRuntime>()
const inFlight = new Map<string, Promise<unknown>>()

async function runModelRefresh(
  providerId: string,
  refresh: () => Promise<unknown>
) {
  const existing = inFlight.get(providerId)
  if (existing) return existing

  const request = refresh()
  inFlight.set(providerId, request)
  try {
    const result = await request
    const runtime = getRuntime(providerId)
    runtime.failureCount = 0
    runtime.lastSuccessAt = Date.now()
    runtime.suspended = false
    return result
  } finally {
    if (inFlight.get(providerId) === request) inFlight.delete(providerId)
  }
}

type AutoModelRefreshOptions = {
  /** 当前激活的第三方服务 id；官方账号/未启用时为 undefined */
  providerId?: string
  active: boolean
  foreground: boolean
  refresh: () => Promise<unknown>
  onRefreshed?: () => void
}

export function useAutoModelRefresh({
  providerId,
  active,
  foreground,
  refresh,
  onRefreshed,
}: AutoModelRefreshOptions) {
  const refreshRef = useRef(refresh)
  const onRefreshedRef = useRef(onRefreshed)

  useEffect(() => {
    refreshRef.current = refresh
  }, [refresh])
  useEffect(() => {
    onRefreshedRef.current = onRefreshed
  }, [onRefreshed])

  useEffect(() => {
    if (!active || !foreground || !providerId) return

    const runtime = getRuntime(providerId)
    // 重新进入页面时允许重试（上一轮连续失败触发的暂停在此解除）。
    runtime.suspended = false

    let cancelled = false
    let timer: number | undefined
    const schedule = (delayMs: number) => {
      timer = window.setTimeout(
        () => {
          timer = undefined
          void refreshOnce()
        },
        Math.max(1_000, delayMs)
      )
    }
    const refreshOnce = async () => {
      if (cancelled) return
      const now = Date.now()
      if (
        runtime.lastAttemptAt !== undefined &&
        now - runtime.lastAttemptAt < MODEL_REFRESH_MINIMUM_MS
      ) {
        schedule(MODEL_REFRESH_MINIMUM_MS - (now - runtime.lastAttemptAt))
        return
      }
      runtime.lastAttemptAt = now
      try {
        await runModelRefresh(providerId, () => refreshRef.current())
        if (!cancelled && foreground) {
          onRefreshedRef.current?.()
          schedule(MODEL_REFRESH_INTERVAL_MS)
        }
      } catch {
        runtime.failureCount += 1
        if (runtime.failureCount >= MODEL_REFRESH_MAX_FAILURES) {
          runtime.suspended = true
          return
        }
        const backoff =
          MODEL_REFRESH_BACKOFF_MS[
            Math.min(runtime.failureCount, MODEL_REFRESH_BACKOFF_MS.length) - 1
          ]
        if (!cancelled && foreground) schedule(backoff)
      }
    }

    const entering =
      runtime.lastSuccessAt === undefined ||
      Date.now() - runtime.lastSuccessAt >= MODEL_REFRESH_ON_ENTER_MS
    if (entering) {
      void refreshOnce()
    } else {
      schedule(
        Math.max(
          1_000,
          MODEL_REFRESH_INTERVAL_MS -
            (Date.now() - (runtime.lastSuccessAt ?? Date.now()))
        )
      )
    }
    return () => {
      cancelled = true
      if (timer !== undefined) window.clearTimeout(timer)
    }
  }, [active, foreground, providerId])
}

function getRuntime(providerId: string) {
  const existing = runtimes.get(providerId)
  if (existing) return existing
  const runtime: ModelRefreshRuntime = {
    failureCount: 0,
    suspended: false,
  }
  runtimes.set(providerId, runtime)
  return runtime
}
