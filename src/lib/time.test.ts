import { describe, expect, it } from "vitest"

import { epochMilliseconds, formatDateTime } from "@/lib/time"

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

describe("formatDateTime", () => {
  const millis = 1_800_000_000_000

  it("formats a millisecond timestamp in the default style", () => {
    const expected = new Intl.DateTimeFormat("zh-CN", {
      dateStyle: "medium",
      timeStyle: "short",
    }).format(new Date(millis))
    expect(formatDateTime(millis)).toBe(expected)
  })

  it("normalizes second-precision timestamps", () => {
    expect(formatDateTime(1_800_000_000)).toBe(formatDateTime(millis))
  })

  it("supports the compact style", () => {
    const expected = new Intl.DateTimeFormat("zh-CN", {
      dateStyle: "short",
      timeStyle: "medium",
    }).format(new Date(millis))
    expect(formatDateTime(millis, "compact")).toBe(expected)
    expect(formatDateTime(millis, "compact")).not.toBe(formatDateTime(millis))
  })

  it("falls back for invalid timestamps", () => {
    expect(formatDateTime(Number.NaN)).toBe("时间未知")
    expect(formatDateTime(Number.POSITIVE_INFINITY)).toBe("时间未知")
  })
})
