import { invoke } from "@tauri-apps/api/core"
import { beforeEach, describe, expect, it, vi } from "vitest"

import { call } from "@/lib/ipc"

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}))

const mockedInvoke = vi.mocked(invoke)

describe("Tauri IPC wrapper", () => {
  beforeEach(() => mockedInvoke.mockReset())

  it("forwards command arguments without mutation", async () => {
    mockedInvoke.mockResolvedValue({ operationId: "preview-id" })

    await expect(call("preview_activation")).resolves.toEqual({
      operationId: "preview-id",
    })
    expect(mockedInvoke).toHaveBeenCalledWith("preview_activation", undefined)
  })

  it.each([
    ["start_openai_device_auth", undefined],
    ["poll_openai_device_auth", { operationId: "device-operation" }],
    ["activate_openai_account", { id: "account-id" }],
    ["delete_openai_account", { id: "account-id" }],
    ["open_openai_device_page", undefined],
  ] as const)("forwards OpenAI command %s", async (command, args) => {
    mockedInvoke.mockResolvedValue({ status: "pending" })

    await expect(call(command, args)).resolves.toEqual({ status: "pending" })
    expect(mockedInvoke).toHaveBeenCalledTimes(1)
    expect(mockedInvoke).toHaveBeenCalledWith(command, args)
  })
})
