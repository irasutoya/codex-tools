import { describe, expect, it } from "vitest"

import { shouldUseMock } from "./ipc"

describe("shouldUseMock", () => {
  it("defaults to mock for a browser development preview", () => {
    expect(shouldUseMock({ dev: true, tauri: false, search: "" })).toBe(true)
  })

  it("keeps a Tauri development window on real IPC by default", () => {
    expect(shouldUseMock({ dev: true, tauri: true, search: "" })).toBe(false)
  })

  it("allows ?mock to force mock mode in development", () => {
    expect(shouldUseMock({ dev: true, tauri: true, search: "?mock" })).toBe(
      true
    )
  })

  it("keeps the empty-connections query compatible with browser mock mode", () => {
    expect(
      shouldUseMock({ dev: true, tauri: false, search: "?empty-connections" })
    ).toBe(true)
  })

  it("never enables mock mode in production", () => {
    expect(shouldUseMock({ dev: false, tauri: false, search: "?mock" })).toBe(
      false
    )
  })
})
