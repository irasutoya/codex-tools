// @vitest-environment jsdom

import { act, renderHook, waitFor } from "@testing-library/react"
import { describe, expect, it, vi } from "vitest"

import { useAsync } from "@/hooks/use-async"

describe("useAsync", () => {
  it("clears stale data while a changed query is loading", async () => {
    const first = vi.fn().mockResolvedValue("active sessions")
    const pending = new Promise<string>(() => {})
    const second = vi.fn(() => pending)
    const { result, rerender } = renderHook(
      ({ fetcher }) => useAsync(fetcher, { clearOnLoad: true }),
      { initialProps: { fetcher: first } }
    )

    await waitFor(() => expect(result.current.data).toBe("active sessions"))

    await act(() => {
      rerender({ fetcher: second })
    })

    expect(result.current.data).toBeUndefined()
    expect(second).toHaveBeenCalledOnce()
  })
})
