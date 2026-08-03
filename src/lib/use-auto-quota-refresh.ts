import { useEffect, useRef } from "react"

import type { AccountQuota } from "@/types"

import {
  planQuotaFailure,
  planQuotaSuccess,
  quotaSignature,
  shouldRefreshQuotaOnActivation,
} from "./quota-refresh-policy"

const QUOTA_ACTIVATION_MINIMUM_MS = 10_000

type AutoQuotaRefreshOptions = {
  accountId?: string
  active: boolean
  foreground: boolean
  quota?: AccountQuota
  refresh: () => Promise<AccountQuota>
}

type QuotaRuntime = {
  failureCount: number
  lastSignature?: string
  lastAutomaticRefreshAt?: number
  suspended: boolean
  unchangedCount: number
}

const runtimes = new Map<string, QuotaRuntime>()
const inFlight = new Map<string, Promise<AccountQuota>>()

export async function runQuotaRefresh(
  accountId: string,
  refresh: () => Promise<AccountQuota>
) {
  const existing = inFlight.get(accountId)
  if (existing) return existing

  const request = refresh()
  inFlight.set(accountId, request)
  try {
    const result = await request
    if (result.status === "success") {
      const runtime = runtimes.get(accountId)
      if (runtime) {
        runtime.failureCount = 0
        runtime.suspended = false
      }
    }
    return result
  } finally {
    if (inFlight.get(accountId) === request) inFlight.delete(accountId)
  }
}

export function useAutoQuotaRefresh({
  accountId,
  active,
  foreground,
  quota,
  refresh,
}: AutoQuotaRefreshOptions) {
  const refreshRef = useRef(refresh)
  const wasActive = useRef(false)
  const previousAccountId = useRef<string | undefined>(undefined)

  useEffect(() => {
    refreshRef.current = refresh
  }, [refresh])

  useEffect(() => {
    const entering =
      active &&
      Boolean(accountId) &&
      (!wasActive.current || previousAccountId.current !== accountId)
    wasActive.current = active
    previousAccountId.current = accountId

    if (!active || !foreground || !accountId) return

    const runtime = getRuntime(accountId)
    if (quota?.status === "success") runtime.suspended = false
    if (quota?.status === "unauthorized" || quota?.status === "unsupported") {
      runtime.suspended = true
    }
    if (runtime.suspended) return

    if (runtime.lastSignature === undefined && quota) {
      runtime.lastSignature = quotaSignature(quota)
    }

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
      runtime.lastAutomaticRefreshAt = Date.now()
      try {
        const result = await runQuotaRefresh(accountId, () =>
          refreshRef.current()
        )
        if (result.status === "success") {
          const signature = quotaSignature(result)
          runtime.unchangedCount =
            runtime.lastSignature === signature ? runtime.unchangedCount + 1 : 0
          runtime.lastSignature = signature
          runtime.failureCount = 0
          const plan = planQuotaSuccess(
            result,
            runtime.unchangedCount,
            Date.now()
          )
          if (plan.kind === "suspended") {
            runtime.suspended = true
            return
          }
          if (!cancelled && foreground) schedule(plan.delayMs)
          return
        }

        runtime.failureCount += 1
        const plan = planQuotaFailure(result, runtime.failureCount, Date.now())
        if (plan.kind === "suspended") {
          runtime.suspended = true
          return
        }
        if (!cancelled && foreground) schedule(plan.delayMs)
      } catch {
        runtime.failureCount += 1
        const plan = planQuotaFailure(
          undefined,
          runtime.failureCount,
          Date.now()
        )
        if (plan.kind === "schedule" && !cancelled && foreground) {
          schedule(plan.delayMs)
        }
      }
    }

    const activationAllowed =
      entering &&
      shouldRefreshQuotaOnActivation(quota, runtime.failureCount) &&
      (runtime.lastAutomaticRefreshAt === undefined ||
        Date.now() - runtime.lastAutomaticRefreshAt >=
          QUOTA_ACTIVATION_MINIMUM_MS)
    if (activationAllowed) {
      void refreshOnce()
    } else {
      schedule(initialDelay(quota, runtime))
    }
    return () => {
      cancelled = true
      if (timer !== undefined) window.clearTimeout(timer)
    }
  }, [accountId, active, foreground, quota])
}

function getRuntime(accountId: string) {
  const existing = runtimes.get(accountId)
  if (existing) return existing
  const runtime: QuotaRuntime = {
    failureCount: 0,
    suspended: false,
    unchangedCount: 0,
  }
  runtimes.set(accountId, runtime)
  return runtime
}

function initialDelay(quota: AccountQuota | undefined, runtime: QuotaRuntime) {
  if (!quota || quota.status === "never") return 1_000
  if (quota.status === "success") {
    const plan = planQuotaSuccess(quota, runtime.unchangedCount)
    if (plan.kind === "suspended") return Number.POSITIVE_INFINITY
    if (plan.reason === "exhausted") return plan.delayMs
    return remainingDelay(quota.lastAttemptAt ?? quota.fetchedAt, plan.delayMs)
  }

  const plan = planQuotaFailure(quota, Math.max(runtime.failureCount, 1))
  if (plan.kind === "suspended") return Number.POSITIVE_INFINITY
  return plan.delayMs
}

function remainingDelay(timestamp: number | undefined, delayMs: number) {
  if (!timestamp || !Number.isFinite(timestamp)) return 1_000
  const milliseconds =
    timestamp < 100_000_000_000 ? timestamp * 1_000 : timestamp
  return Math.max(1_000, milliseconds + delayMs - Date.now())
}
