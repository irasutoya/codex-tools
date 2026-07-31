import { describe, expect, it } from "vitest"

import { formatError } from "@/lib/feedback"

describe("formatError", () => {
  it("normalizes empty and prefixed errors", () => {
    expect(formatError(undefined)).toBe("未提供错误详情。")
    expect(formatError(" Error: connection failed ")).toBe("connection failed")
  })

  it("removes unsafe control characters", () => {
    expect(formatError("读取\u0000失败\u0007")).toBe("读取失败")
  })

  it("bounds unusually large error details", () => {
    const message = formatError("x".repeat(10_000))

    expect(message.length).toBeLessThanOrEqual(2_020)
    expect(message.endsWith("…（错误详情已截断）")).toBe(true)
  })
})
