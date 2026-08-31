import { describe, expect, it } from "vitest"

import { commandInventory } from "@/lib/ipc"
import backendSource from "../../src-tauri/src/lib.rs?raw"

describe("IPC command contract", () => {
  it("keeps frontend commands aligned with registered Tauri handlers", () => {
    const handler = backendSource.match(
      /\.invoke_handler\(tauri::generate_handler!\[([\s\S]*?)\]\)/
    )?.[1]
    expect(handler).toBeDefined()

    const registered = [...handler!.matchAll(/commands::\w+::(\w+)/g)].map(
      ([, name]) => name
    )

    expect(new Set(registered).size).toBe(registered.length)
    expect(new Set(commandInventory).size).toBe(commandInventory.length)
    expect([...commandInventory].sort()).toEqual(registered.sort())
  })
})
