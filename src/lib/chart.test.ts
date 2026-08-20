import { describe, expect, it } from "vitest"

import { regularOutputTokens, trendPointsToSeries } from "@/lib/chart"
import type { UsageTrendPoint } from "@/types"

const point: UsageTrendPoint = {
  dayStartMs: Date.UTC(2026, 0, 1),
  tokens: {
    inputTokens: 100,
    cachedInputTokens: 20,
    cacheWriteInputTokens: 5,
    outputTokens: 30,
    reasoningOutputTokens: 10,
    totalTokens: 130,
  },
  requests: 1,
  estimatedCostMicrousd: 0,
  unpricedTokens: 0,
  partialTokens: 0,
  unattributedTokens: 0,
}

describe("usage chart helpers", () => {
  it("does not add reasoning tokens to reported output again", () => {
    expect(trendPointsToSeries([point])[0]?.output).toBe(30)
  })

  it("subtracts reasoning from regular output", () => {
    expect(regularOutputTokens(30, 10)).toBe(20)
  })

  it("does not return negative regular output", () => {
    expect(regularOutputTokens(5, 10)).toBe(0)
  })
})
