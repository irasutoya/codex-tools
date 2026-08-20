import { describe, expect, it } from "vitest"

import { isLazyChunkLoadError } from "@/components/error-boundary-utils"

describe("ErrorBoundary chunk failure detection", () => {
  it("detects dynamic import failures from Chromium and WebKit", () => {
    expect(
      isLazyChunkLoadError(
        new TypeError("Failed to fetch dynamically imported module: app.js")
      )
    ).toBe(true)
    expect(
      isLazyChunkLoadError(
        new TypeError("Importing a module script failed: app.js")
      )
    ).toBe(true)
  })

  it("detects named and conventional chunk loading failures", () => {
    const named = new Error("request failed")
    named.name = "ChunkLoadError"

    expect(isLazyChunkLoadError(named)).toBe(true)
    expect(isLazyChunkLoadError(new Error("Loading chunk 42 failed"))).toBe(
      true
    )
    expect(isLazyChunkLoadError(new Error("Loading CSS chunk 7 failed"))).toBe(
      true
    )
  })

  it("does not classify ordinary render errors as chunk failures", () => {
    expect(
      isLazyChunkLoadError(new TypeError("Cannot read properties of null"))
    ).toBe(false)
    expect(isLazyChunkLoadError(new Error("无法读取应用状态"))).toBe(false)
  })
})
