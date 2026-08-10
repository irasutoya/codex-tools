import { describe, expect, it } from "vitest"

import {
  cacheHitRate,
  formatPercent,
  formatTokens,
  quotaWindow,
  tokenInput,
} from "@/lib/format"

describe("format helpers", () => {
  it("keeps cached token details inside the reported total input", () => {
    expect(
      tokenInput({
        inputTokens: 100,
        cachedInputTokens: 20,
        cacheWriteInputTokens: 5,
        outputTokens: 30,
        reasoningOutputTokens: 10,
        totalTokens: 165,
      })
    ).toBe(100)
  })

  it("calculates and formats cache hit rate from total input", () => {
    const rate = cacheHitRate({
      inputTokens: 100,
      cachedInputTokens: 25,
      cacheWriteInputTokens: 5,
      outputTokens: 10,
      reasoningOutputTokens: 0,
      totalTokens: 110,
    })
    expect(rate).toBe(25)
    expect(formatPercent(rate)).toBe("25.0%")
    expect(cacheHitRate()).toBeUndefined()
  })

  it("uses a compact token label", () => {
    expect(formatTokens(12_400)).toContain("万")
  })

  it("selects the primary quota window", () => {
    expect(
      quotaWindow({
        status: "success",
        data: {
          kind: "windowed",
          primary: { usedPercent: 25, remainingPercent: 75 },
          secondary: { usedPercent: 50, remainingPercent: 50 },
        },
      })?.usedPercent
    ).toBe(25)
  })
})
