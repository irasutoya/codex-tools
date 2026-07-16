import { describe, expect, it } from "vitest"

import { epochMilliseconds } from "@/lib/time"

describe("epochMilliseconds", () => {
  it.each([
    [1_800_000_000, 1_800_000_000_000],
    [1_800_000_000_000, 1_800_000_000_000],
    [1_800_000_000_000_000, 1_800_000_000_000],
    [1_800_000_000_000_000_000, 1_800_000_000_000],
  ])("normalizes epoch value %s", (value, expected) => {
    expect(epochMilliseconds(value)).toBe(expected)
  })

  it("keeps invalid timestamps invalid", () => {
    expect(epochMilliseconds(Number.NaN)).toBeNaN()
    expect(epochMilliseconds(Number.POSITIVE_INFINITY)).toBeNaN()
  })
})
