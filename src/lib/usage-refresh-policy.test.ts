import { describe, expect, it } from "vitest"

import {
  USAGE_ENTRY_REFRESH_MINIMUM_MS,
  shouldRefreshUsageOnEntry,
} from "./usage-refresh-policy"

describe("usage refresh policy", () => {
  it("refreshes when entering without a recent automatic refresh", () => {
    const now = Date.parse("2026-08-03T12:00:00Z")

    expect(shouldRefreshUsageOnEntry(undefined, now)).toBe(true)
    expect(
      shouldRefreshUsageOnEntry(now - USAGE_ENTRY_REFRESH_MINIMUM_MS, now)
    ).toBe(true)
  })

  it("coalesces rapid page switching", () => {
    const now = Date.parse("2026-08-03T12:00:00Z")

    expect(
      shouldRefreshUsageOnEntry(now - USAGE_ENTRY_REFRESH_MINIMUM_MS + 1, now)
    ).toBe(false)
  })
})
