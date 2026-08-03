import type { AccountQuota, QuotaWindow } from "@/types"

export const QUOTA_REFRESH_INTERVAL_MS = 5 * 60_000
export const QUOTA_UNCHANGED_INTERVAL_MS = 15 * 60_000
export const QUOTA_IDLE_INTERVAL_MS = 30 * 60_000
export const QUOTA_RATE_LIMIT_INTERVAL_MS = 5 * 60_000
export const QUOTA_EXHAUSTED_GRACE_MS = 10_000

const retryDelays = [
  30_000,
  60_000,
  2 * 60_000,
  5 * 60_000,
  10 * 60_000,
  QUOTA_IDLE_INTERVAL_MS,
]

export type QuotaRefreshPlan =
  | { kind: "schedule"; delayMs: number; reason: string }
  | { kind: "suspended"; reason: string }

export function quotaSignature(quota?: AccountQuota) {
  if (!quota) return "none"
  return JSON.stringify({
    status: quota.status,
    primary: windowSignature(quota.data?.primary),
    secondary: windowSignature(quota.data?.secondary),
  })
}

export function planQuotaSuccess(
  quota: AccountQuota,
  unchangedCount: number,
  now = Date.now()
): QuotaRefreshPlan {
  if (quota.status === "unauthorized") {
    return { kind: "suspended", reason: "unauthorized" }
  }
  if (quota.status === "unsupported") {
    return { kind: "suspended", reason: "unsupported" }
  }
  if (quota.status === "rate_limited") {
    return {
      kind: "schedule",
      delayMs: QUOTA_RATE_LIMIT_INTERVAL_MS,
      reason: "rate-limited",
    }
  }

  const resetDelay = exhaustedResetDelay(quota, now)
  if (resetDelay !== undefined) {
    return { kind: "schedule", delayMs: resetDelay, reason: "exhausted" }
  }

  return {
    kind: "schedule",
    delayMs:
      unchangedCount >= 4
        ? QUOTA_IDLE_INTERVAL_MS
        : unchangedCount >= 2
          ? QUOTA_UNCHANGED_INTERVAL_MS
          : QUOTA_REFRESH_INTERVAL_MS,
    reason: unchangedCount > 0 ? "unchanged" : "changed",
  }
}

export function planQuotaFailure(
  quota: AccountQuota | undefined,
  failureCount: number,
  now = Date.now()
): QuotaRefreshPlan {
  if (quota?.status === "unauthorized") {
    return { kind: "suspended", reason: "unauthorized" }
  }
  if (quota?.status === "unsupported") {
    return { kind: "suspended", reason: "unsupported" }
  }
  if (quota?.status === "rate_limited") {
    return {
      kind: "schedule",
      delayMs: Math.max(
        QUOTA_RATE_LIMIT_INTERVAL_MS,
        remainingDelay(quota.lastAttemptAt, QUOTA_RATE_LIMIT_INTERVAL_MS, now)
      ),
      reason: "rate-limited",
    }
  }

  const delayMs = retryDelays[Math.min(Math.max(failureCount - 1, 0), 5)]
  return {
    kind: "schedule",
    delayMs: Math.max(
      delayMs,
      remainingDelay(quota?.lastAttemptAt, delayMs, now)
    ),
    reason: "retry",
  }
}

export function shouldRefreshQuotaOnActivation(
  quota: AccountQuota | undefined,
  failureCount: number,
  now = Date.now()
) {
  if (!quota || quota.status === "never") return true
  if (quota.status === "unauthorized" || quota.status === "unsupported") {
    return false
  }

  if (quota.status === "rate_limited") {
    return (
      quota.lastAttemptAt !== undefined &&
      remainingDelay(quota.lastAttemptAt, QUOTA_RATE_LIMIT_INTERVAL_MS, now) ===
        0
    )
  }

  if (quota.status === "success") {
    const resetDelay = exhaustedResetDelay(quota, now)
    return resetDelay === undefined || resetDelay <= 1_000
  }

  const retryDelay = retryDelays[Math.min(Math.max(failureCount - 1, 0), 5)]
  return remainingDelay(quota.lastAttemptAt, retryDelay, now) === 0
}

function exhaustedResetDelay(quota: AccountQuota, now: number) {
  const exhaustedWindows = [quota.data?.primary, quota.data?.secondary]
    .filter((item): item is QuotaWindow => Boolean(item))
    .filter((item) => item.remainingPercent <= 0)

  if (exhaustedWindows.length === 0) return undefined

  const resetValues = exhaustedWindows
    .map((item) => item.resetAt)
    .filter((value): value is number => Number.isFinite(value))
    .map((value) => epochMilliseconds(value))
  const resetAt = resetValues
    .filter((value) => value > now)
    .sort((a, b) => a - b)[0]

  if (resetAt === undefined && resetValues.length > 0) return 1_000
  return resetAt === undefined
    ? QUOTA_IDLE_INTERVAL_MS
    : resetAt - now + QUOTA_EXHAUSTED_GRACE_MS
}

function remainingDelay(
  lastAttemptAt: number | undefined,
  delayMs: number,
  now: number
) {
  if (!lastAttemptAt || !Number.isFinite(lastAttemptAt)) return 0
  return Math.max(0, epochMilliseconds(lastAttemptAt) + delayMs - now)
}

function windowSignature(window: QuotaWindow | undefined) {
  if (!window) return undefined
  return [
    window.usedPercent,
    window.remainingPercent,
    window.windowSeconds,
    window.resetAt,
  ]
}

function epochMilliseconds(value: number) {
  return value < 100_000_000_000 ? value * 1_000 : value
}
