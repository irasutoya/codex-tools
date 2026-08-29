import type { AccountQuota, QuotaEstimate, QuotaWindow } from "@/types"

export type DisplayQuotaWindow = QuotaWindow & {
  windowSeconds: number
  label: "5H" | "7D" | string
}

export const CURRENT_QUOTA_ESTIMATE_CALCULATION_VERSION = 1

/** 先按明确时长识别；老响应缺时长时才使用 primary/secondary 的兼容位置。 */
export function displayQuotaWindows(
  quota?: AccountQuota
): DisplayQuotaWindow[] {
  if (quota?.status !== "success" || quota.data?.kind !== "windowed") return []
  const candidates: Array<[QuotaWindow | undefined, number]> = [
    [quota.data.primary, 18_000],
    [quota.data.secondary, 604_800],
  ]
  const windows: DisplayQuotaWindow[] = []
  for (const [window, fallbackSeconds] of candidates) {
    if (!window) continue
    const windowSeconds = window.windowSeconds ?? fallbackSeconds
    if (windowSeconds <= 0) continue
    if (
      windows.some(
        (candidate) =>
          candidate.windowSeconds === windowSeconds &&
          candidate.resetAt === window.resetAt
      )
    ) {
      continue
    }
    windows.push({
      ...window,
      windowSeconds,
      label:
        windowSeconds === 18_000
          ? "5H"
          : windowSeconds === 604_800
            ? "7D"
            : `${Math.round(windowSeconds / 3_600)}H`,
    })
  }
  return windows
}

export function quotaWindowEstimate(
  estimates: QuotaEstimate[],
  window: DisplayQuotaWindow
) {
  if (!window.resetAt) return undefined
  return estimates.find(
    (estimate) =>
      estimate.windowSeconds === window.windowSeconds &&
      estimate.resetAt === window.resetAt &&
      estimate.calculationVersion === CURRENT_QUOTA_ESTIMATE_CALCULATION_VERSION
  )
}
