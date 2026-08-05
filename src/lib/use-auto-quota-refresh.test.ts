import { describe, expect, it } from "vitest"

import type { AccountQuota } from "@/types"

import { runQuotaRefresh } from "./use-auto-quota-refresh"

describe("runQuotaRefresh", () => {
  it("shares an in-flight request for the same account", async () => {
    let resolveRequest: ((value: AccountQuota) => void) | undefined
    let calls = 0
    const request = new Promise<AccountQuota>((resolve) => {
      resolveRequest = resolve
    })
    const refresh = () => {
      calls += 1
      return request
    }

    const first = runQuotaRefresh("account-1", refresh)
    const second = runQuotaRefresh("account-1", refresh)
    resolveRequest!({ status: "success" })

    await expect(Promise.all([first, second])).resolves.toEqual([
      { status: "success" },
      { status: "success" },
    ])
    expect(calls).toBe(1)
  })
})
