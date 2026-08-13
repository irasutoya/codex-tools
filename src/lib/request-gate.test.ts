import { describe, expect, it } from "vitest"

import { createRequestGate } from "@/lib/request-gate"

describe("request gate", () => {
  it("only accepts the latest request", () => {
    const gate = createRequestGate()
    const first = gate.begin()
    const second = gate.begin()

    expect(gate.isCurrent(first)).toBe(false)
    expect(gate.isCurrent(second)).toBe(true)
  })

  it("invalidates a request when its query changes", () => {
    const gate = createRequestGate()
    const request = gate.begin()

    gate.invalidate()

    expect(gate.isCurrent(request)).toBe(false)
  })

  it("keeps a scan current when a background read starts", () => {
    const gate = createRequestGate()
    const scan = gate.begin("scan")
    const background = gate.begin("background")

    expect(gate.isCurrent(scan)).toBe(true)
    expect(gate.isCurrent(background)).toBe(false)
  })

  it("lets a scan supersede ordinary reads", () => {
    const gate = createRequestGate()
    const read = gate.begin("read")
    const background = gate.begin("background")
    const scan = gate.begin("scan")

    expect(gate.isCurrent(read)).toBe(false)
    expect(gate.isCurrent(background)).toBe(false)
    expect(gate.isCurrent(scan)).toBe(true)
  })

  it("accepts lower-priority work after the current scan finishes", () => {
    const gate = createRequestGate()
    const scan = gate.begin("scan")

    gate.finish(scan)
    const background = gate.begin("background")

    expect(gate.isCurrent(scan)).toBe(false)
    expect(gate.isCurrent(background)).toBe(true)
  })

  it("invalidates every priority when the query changes", () => {
    const gate = createRequestGate()
    const scan = gate.begin("scan")

    gate.invalidate()
    const read = gate.begin("read")

    expect(gate.isCurrent(scan)).toBe(false)
    expect(gate.isCurrent(read)).toBe(true)
  })

  it("lets a waiting background reload eventually replace an active read", async () => {
    const gate = createRequestGate()
    const read = gate.begin("read")
    let displayed = "initial"
    let backgroundStarted = false

    const reloadInBackground = async () => {
      let request = gate.begin("background")
      while (!gate.isCurrent(request)) {
        await gate.waitForChange()
        request = gate.begin("background")
      }
      backgroundStarted = true
      displayed = "new pricing"
      gate.finish(request)
    }

    const reload = reloadInBackground()
    await Promise.resolve()
    expect(backgroundStarted).toBe(false)

    if (gate.isCurrent(read)) displayed = "old pricing"
    gate.finish(read)
    await reload

    expect(displayed).toBe("new pricing")
  })

  it("does not start a waiting background reload until a scan finishes", async () => {
    const gate = createRequestGate()
    const scan = gate.begin("scan")
    let backgroundStarted = false

    const reloadInBackground = async () => {
      let request = gate.begin("background")
      while (!gate.isCurrent(request)) {
        await gate.waitForChange()
        request = gate.begin("background")
      }
      backgroundStarted = true
      gate.finish(request)
    }

    const reload = reloadInBackground()
    await Promise.resolve()
    expect(backgroundStarted).toBe(false)
    expect(gate.isCurrent(scan)).toBe(true)

    gate.finish(scan)
    await reload

    expect(backgroundStarted).toBe(true)
  })
})
