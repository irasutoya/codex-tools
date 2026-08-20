import { afterAll, beforeAll, describe, expect, it, vi } from "vitest"

import { filterMockSelectedModels, mockCall } from "./ipc-mock"
import type { Provider, ProviderSaveInput } from "../types"

describe("filterMockSelectedModels", () => {
  it("keeps the default all-models selection undefined", () => {
    expect(filterMockSelectedModels(undefined, ["new-model"])).toBeUndefined()
  })

  it("filters an explicit selection against refreshed models", () => {
    expect(
      filterMockSelectedModels(
        ["new-model", "stale-model"],
        ["new-model", "other-model"]
      )
    ).toEqual(["new-model"])
  })
})

describe("mockCall", () => {
  beforeAll(() => vi.useFakeTimers())
  afterAll(() => vi.useRealTimers())

  const saveProvider = async (provider: ProviderSaveInput) => {
    const result = mockCall("connections_save_provider", { provider })
    await vi.runAllTimersAsync()
    return result as Promise<Provider>
  }

  it("saves and filters an explicit selection after a source change", async () => {
    const input = {
      id: "provider-mock-selection",
      name: "Mock selection",
      baseUrl: "https://mock.example/v1",
      headers: {},
      timeoutSecs: 30,
      enabled: true,
      apiType: "responses" as const,
      apiKey: "secret",
      selectedModels: ["gpt-5.6", "stale-model"],
    }
    await saveProvider(input)
    const saved = await saveProvider({
      ...input,
      baseUrl: "https://changed-mock.example/v1",
    })

    expect(saved.availableModels).toEqual(["gpt-5.6"])
    expect(saved.selectedModels).toEqual(["gpt-5.6"])
  })

  it("keeps undefined as the default all-models selection", async () => {
    const saved = await saveProvider({
      id: "provider-mock-all-models",
      name: "Mock all models",
      baseUrl: "https://mock-all.example/v1",
      headers: {},
      timeoutSecs: 30,
      enabled: true,
      apiType: "responses",
      apiKey: "secret",
      selectedModels: null,
    })

    expect(saved.selectedModels).toBeUndefined()
  })
})
