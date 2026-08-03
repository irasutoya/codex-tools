import { describe, expect, it } from "vitest"

import type { AccountQuota } from "@/types"

import {
  QUOTA_EXHAUSTED_GRACE_MS,
  QUOTA_IDLE_INTERVAL_MS,
  QUOTA_REFRESH_INTERVAL_MS,
  QUOTA_UNCHANGED_INTERVAL_MS,
  planQuotaFailure,
  planQuotaSuccess,
  quotaSignature,
} from "./quota-refresh-policy"
import type { QuotaRefreshPlan } from "./quota-refresh-policy"

const now = Date.parse("2026-08-03T12:00:00Z")

function quota(overrides: Partial<AccountQuota> = {}): AccountQuota {
  return {
    status: "success",
    data: {
      kind: "windowed",
      primary: {
        usedPercent: 20,
        remainingPercent: 80,
        windowSeconds: 18_000,
        resetAt: now + 60 * 60_000,
      },
    },
    ...overrides,
  }
}

describe("quota refresh policy", () => {
  it("slows down when successful quota values stay unchanged", () => {
    expect(planQuotaSuccess(quota(), 0, now)).toEqual({
      kind: "schedule",
      delayMs: QUOTA_REFRESH_INTERVAL_MS,
      reason: "changed",
    })
    expect(scheduledDelay(planQuotaSuccess(quota(), 1, now))).toBe(
      QUOTA_REFRESH_INTERVAL_MS
    )
    expect(scheduledDelay(planQuotaSuccess(quota(), 2, now))).toBe(
      QUOTA_UNCHANGED_INTERVAL_MS
    )
    expect(scheduledDelay(planQuotaSuccess(quota(), 4, now))).toBe(
      QUOTA_IDLE_INTERVAL_MS
    )
  })

  it("waits for an exhausted quota reset", () => {
    const resetAt = now + 60 * 60_000
    const plan = planQuotaSuccess(
      quota({
        data: {
          kind: "windowed",
          primary: {
            usedPercent: 100,
            remainingPercent: 0,
            windowSeconds: 18_000,
            resetAt,
          },
        },
      }),
      0,
      now
    )

    expect(plan).toEqual({
      kind: "schedule",
      delayMs: resetAt - now + QUOTA_EXHAUSTED_GRACE_MS,
      reason: "exhausted",
    })

    expect(
      scheduledDelay(
        planQuotaSuccess(
          quota({
            data: {
              kind: "windowed",
              primary: {
                usedPercent: 100,
                remainingPercent: 0,
                windowSeconds: 18_000,
                resetAt: now - 1_000,
              },
            },
          }),
          0,
          now
        )
      )
    ).toBe(1_000)
  })

  it("backs off transient failures and suspends terminal states", () => {
    expect(scheduledDelay(planQuotaFailure(undefined, 1, now))).toBe(30_000)
    expect(scheduledDelay(planQuotaFailure(undefined, 2, now))).toBe(60_000)
    expect(
      scheduledDelay(planQuotaFailure({ status: "rate_limited" }, 1, now))
    ).toBe(5 * 60_000)
    expect(planQuotaFailure({ status: "unauthorized" }, 1, now)).toEqual({
      kind: "suspended",
      reason: "unauthorized",
    })
  })

  it("identifies changes in either quota window", () => {
    const before = quotaSignature(quota())
    const after = quotaSignature(
      quota({
        data: {
          kind: "windowed",
          primary: {
            usedPercent: 21,
            remainingPercent: 79,
            windowSeconds: 18_000,
            resetAt: now + 60 * 60_000,
          },
        },
      })
    )

    expect(after).not.toBe(before)
  })
})

function scheduledDelay(plan: QuotaRefreshPlan) {
  if (plan.kind !== "schedule") throw new Error("expected a scheduled retry")
  return plan.delayMs
}
