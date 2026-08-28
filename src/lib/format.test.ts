import { describe, expect, it } from "vitest"

import {
  cacheHitRate,
  formatDate,
  formatPercent,
  formatTokens,
  formatUsd,
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

  it("formats precise USD amounts with a narrow dollar symbol", () => {
    const value = formatUsd(123_456)

    expect(value).toBe("$0.1235")
    expect(value[0]).toBe("$")
    expect(value).not.toContain("US$")
  })

  it("keeps compact USD amount precision with a narrow dollar symbol", () => {
    const value = formatUsd(100_000)

    expect(value).toBe("$0.10")
    expect(value[0]).toBe("$")
    expect(value).not.toContain("US$")
  })

  it("formats invalid timestamps as unavailable", () => {
    expect(formatDate(Number.POSITIVE_INFINITY)).toBe("—")
    expect(formatDate(Number.MAX_VALUE)).toBe("—")
  })
})
