import { describe, expect, it } from "vitest"

import {
  getLocalDateKey,
  millisecondsUntilNextLocalMidnight,
} from "./local-time"

describe("local time", () => {
  it("returns the local calendar date", () => {
    expect(getLocalDateKey(new Date(2026, 7, 3, 23, 59, 50))).toBe("2026-08-03")
    expect(getLocalDateKey(new Date(2026, 7, 4, 0, 0, 1))).toBe("2026-08-04")
  })

  it("schedules the next local midnight", () => {
    expect(
      millisecondsUntilNextLocalMidnight(new Date(2026, 7, 3, 23, 59, 50))
    ).toBe(10_000)
  })
})
