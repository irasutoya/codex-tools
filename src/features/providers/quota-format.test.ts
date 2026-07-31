import { describe, expect, it } from "vitest"

import { quotaRows } from "./quota-format"

describe("quotaRows", () => {
  it("labels official-shaped windows by their actual duration", () => {
    expect(
      quotaRows({
        status: "success",
        data: {
          kind: "windowed",
          primary: {
            usedPercent: 24,
            remainingPercent: 76,
            windowSeconds: 18_000,
          },
          secondary: {
            usedPercent: 57,
            remainingPercent: 43,
            windowSeconds: 604_800,
          },
        },
      })
    ).toEqual([
      { label: "5H", value: "剩余 76%", detail: "已用 24%" },
      { label: "7D", value: "剩余 43%", detail: "已用 57%" },
    ])
  })
})
