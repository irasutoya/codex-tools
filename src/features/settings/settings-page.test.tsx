// @vitest-environment jsdom

import { act, fireEvent, render, screen } from "@testing-library/react"
import { describe, expect, it, vi } from "vitest"

import { SettingsPage } from "@/features/settings/settings-page"
import type { ConfigPatchPreview, SettingsOverview } from "@/types"

const callMock = vi.hoisted(() => vi.fn())

vi.mock("@/lib/ipc", () => ({ call: callMock }))

describe("SettingsPage", () => {
  it("discards a configuration preview after leaving its section", async () => {
    let resolvePreview!: (preview: ConfigPatchPreview) => void
    const pendingPreview = new Promise<ConfigPatchPreview>((resolve) => {
      resolvePreview = resolve
    })
    const overview: SettingsOverview = {
      inspection: {
        path: "/tmp/config.toml",
        valid: true,
        managedProviderPresent: true,
        warnings: [],
      },
      diagnostics: {},
      canPreviewCustom: true,
    }
    callMock.mockImplementation((command: string) => {
      if (command === "settings_get_overview") return Promise.resolve(overview)
      if (command === "settings_preview_activation") return pendingPreview
      if (command === "settings_get_codex_app") {
        return Promise.resolve({ detected: null, configured: null })
      }
      throw new Error(`Unexpected command: ${command}`)
    })
    const { rerender } = render(
      <SettingsPage refreshRevision={0} section="config" onRefresh={vi.fn()} />
    )
    const previewButton = await screen.findByRole("button", {
      name: "预览自定义配置",
    })
    fireEvent.click(previewButton)

    rerender(
      <SettingsPage refreshRevision={0} section="app" onRefresh={vi.fn()} />
    )
    await act(async () => {
      resolvePreview({
        operationId: "stale-operation",
        targetPath: "/tmp/config.toml",
        baseHash: "hash",
        rendered: "model = stale",
        changes: ["stale change"],
        apiKeyMasked: "***",
      })
      await pendingPreview
    })

    expect(screen.queryByRole("dialog", { name: "配置变更预览" })).toBeNull()
  })
})
