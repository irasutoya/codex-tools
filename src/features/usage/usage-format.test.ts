import { afterEach, describe, expect, it, vi } from "vitest"

import {
  formatCostStatus,
  formatEstimatedUsd,
  formatTokens,
  formatUsdMicrousd,
  getLocalDateRange,
  getLocalRange,
} from "./usage-format"

describe("usage formatting", () => {
  afterEach(() => vi.useRealTimers())

  it("formats token counts into readable compact values", () => {
    expect(formatTokens(0)).toBe("0")
    expect(formatTokens(999)).toBe("999")
    expect(formatTokens(12_345)).toBe("12.3K")
    expect(formatTokens(1_230_000)).toBe("1.23M")
  })

  it("formats micro USD without losing small estimates", () => {
    expect(formatUsdMicrousd(0)).toBe("$0.00")
    expect(formatUsdMicrousd(3_420_000)).toBe("$3.42")
    expect(formatUsdMicrousd(1)).toBe("$0.000001")
    expect(formatUsdMicrousd()).toBe("未估算")
    expect(formatEstimatedUsd(0, 10)).toBe("未估算")
  })

  it("keeps cost status wording explicit", () => {
    expect(formatCostStatus("unpriced")).toBe("未配置价格")
    expect(formatCostStatus("subscription")).toBe("套餐统计")
  })

  it("creates a local-day range", () => {
    const range = getLocalRange(7)
    expect(range.endAtMs).toBeGreaterThan(range.startAtMs)
    expect(range.endAtMs - range.startAtMs).toBeGreaterThanOrEqual(
      6 * 24 * 60 * 60 * 1_000
    )
  })

  it("recomputes relative ranges after local midnight", () => {
    vi.useFakeTimers()
    vi.setSystemTime(new Date("2026-08-03T23:59:50"))
    const before = getLocalRange(1)

    vi.setSystemTime(new Date("2026-08-04T00:00:20"))
    const after = getLocalRange(1)

    expect(after.startAtMs).toBeGreaterThan(before.startAtMs)
    expect(new Date(after.startAtMs).getDate()).toBe(4)
  })

  it("accepts a valid inclusive local date range", () => {
    const range = getLocalDateRange("2026-08-01", "2026-08-03")
    expect(range).toBeDefined()
    expect(range!.endAtMs).toBeGreaterThan(range!.startAtMs)
    expect(getLocalDateRange("2026-08-03", "2026-08-01")).toBeUndefined()
  })
})
