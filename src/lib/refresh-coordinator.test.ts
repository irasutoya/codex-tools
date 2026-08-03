import { describe, expect, it, vi } from "vitest"

import { RefreshCoordinator } from "./refresh-coordinator"

describe("RefreshCoordinator", () => {
  it("increments only the invalidated page revisions", () => {
    const coordinator = new RefreshCoordinator()
    const listener = vi.fn()
    const unsubscribe = coordinator.subscribe(listener)

    const before = coordinator.getSnapshot("dashboard")
    const providersBefore = coordinator.getSnapshot("providers")
    coordinator.invalidate(["dashboard"])

    expect(coordinator.getSnapshot("dashboard")).toEqual({
      revision: before.revision + 1,
      localDay: before.localDay,
    })
    expect(coordinator.getSnapshot("providers")).toBe(providersBefore)
    expect(listener).toHaveBeenCalledTimes(1)

    unsubscribe()
    coordinator.invalidate(["dashboard"])
    expect(listener).toHaveBeenCalledTimes(1)
  })
})
