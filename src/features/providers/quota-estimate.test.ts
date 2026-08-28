import { describe, expect, it } from "vitest"

import type { AccountQuota } from "@/types"

import { displayQuotaWindows, quotaWindowEstimate } from "./quota-estimate"

describe("displayQuotaWindows", () => {
  it("优先按 windowSeconds 识别 5H 与 7D", () => {
    const quota: AccountQuota = {
      status: "success",
      data: {
        kind: "windowed",
        primary: {
          usedPercent: 20,
          remainingPercent: 80,
          windowSeconds: 604_800,
          resetAt: 20,
        },
        secondary: {
          usedPercent: 30,
          remainingPercent: 70,
          windowSeconds: 18_000,
          resetAt: 10,
        },
      },
    }
    expect(displayQuotaWindows(quota).map((window) => window.label)).toEqual([
      "7D",
      "5H",
    ])
  })

  it("仅有 7D 时绝不渲染 5H 占位", () => {
    const quota: AccountQuota = {
      status: "success",
      data: {
        kind: "windowed",
        secondary: { usedPercent: 20, remainingPercent: 80, resetAt: 20 },
      },
    }
    expect(displayQuotaWindows(quota).map((window) => window.label)).toEqual([
      "7D",
    ])
  })

  it("只接受同一窗口时长和重置时间的持久化结果", () => {
    const [window] = displayQuotaWindows({
      status: "success",
      data: {
        kind: "windowed",
        primary: { usedPercent: 20, remainingPercent: 80, resetAt: 20 },
      },
    })
    expect(window).toBeDefined()
    expect(
      quotaWindowEstimate(
        [
          {
            windowSeconds: 18_000,
            resetAt: 21,
            estimatedAt: 1,
            estimatedTotalMicrousd: 100,
          },
        ],
        window!
      )
    ).toBeUndefined()
  })
})
