import { describe, expect, it, vi } from "vitest"

import {
  FOCUS_REFRESH_MINIMUM_MS,
  RefreshCoordinator,
} from "./refresh-coordinator"

const originalWindow = globalThis.window
const originalDocument = globalThis.document

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
      foreground: before.foreground,
    })
    expect(coordinator.getSnapshot("providers")).toBe(providersBefore)
    expect(listener).toHaveBeenCalledTimes(1)

    unsubscribe()
    coordinator.invalidate(["dashboard"])
    expect(listener).toHaveBeenCalledTimes(1)
  })

  it("pauses while unfocused and refreshes after a long absence", () => {
    vi.useFakeTimers()
    let focused = true
    let visibility: DocumentVisibilityState = "visible"
    const windowListeners = new Map<string, () => void>()
    const documentListeners = new Map<string, () => void>()
    const fakeWindow = {
      addEventListener: (type: string, listener: () => void) =>
        windowListeners.set(type, listener),
      removeEventListener: (type: string) => windowListeners.delete(type),
      setTimeout,
      clearTimeout,
    }
    const fakeDocument = {
      get visibilityState() {
        return visibility
      },
      hasFocus: () => focused,
      addEventListener: (type: string, listener: () => void) =>
        documentListeners.set(type, listener),
      removeEventListener: (type: string) => documentListeners.delete(type),
    }
    Object.assign(globalThis, {
      window: fakeWindow,
      document: fakeDocument,
    })

    try {
      const coordinator = new RefreshCoordinator()
      coordinator.start()
      const initialRevision = coordinator.getSnapshot("dashboard").revision

      focused = false
      windowListeners.get("blur")?.()
      expect(coordinator.getForeground()).toBe(false)

      focused = true
      windowListeners.get("focus")?.()
      expect(coordinator.getSnapshot("dashboard").revision).toBe(
        initialRevision
      )

      focused = false
      windowListeners.get("blur")?.()
      vi.advanceTimersByTime(FOCUS_REFRESH_MINIMUM_MS + 1)
      focused = true
      windowListeners.get("focus")?.()
      expect(coordinator.getSnapshot("dashboard").revision).toBe(
        initialRevision + 1
      )

      visibility = "hidden"
      documentListeners.get("visibilitychange")?.()
      expect(coordinator.getForeground()).toBe(false)
      coordinator.stop()
    } finally {
      Object.assign(globalThis, {
        window: originalWindow,
        document: originalDocument,
      })
      vi.useRealTimers()
    }
  })

  it("starts in the foreground even when the webview document has no focus yet", () => {
    vi.useFakeTimers()
    const visibility: DocumentVisibilityState = "visible"
    const windowListeners = new Map<string, () => void>()
    const documentListeners = new Map<string, () => void>()
    const fakeWindow = {
      addEventListener: (type: string, listener: () => void) =>
        windowListeners.set(type, listener),
      removeEventListener: (type: string) => windowListeners.delete(type),
      setTimeout,
      clearTimeout,
    }
    const fakeDocument = {
      get visibilityState() {
        return visibility
      },
      // Tauri WebView 初次加载时 document.hasFocus() 恒为 false，
      // 即使窗口已经聚焦。
      hasFocus: () => false,
      addEventListener: (type: string, listener: () => void) =>
        documentListeners.set(type, listener),
      removeEventListener: (type: string) => documentListeners.delete(type),
    }
    Object.assign(globalThis, {
      window: fakeWindow,
      document: fakeDocument,
    })

    try {
      const coordinator = new RefreshCoordinator()
      expect(coordinator.getForeground()).toBe(true)

      coordinator.start()
      expect(coordinator.getForeground()).toBe(true)
      const initialRevision = coordinator.getSnapshot("dashboard").revision

      // 窗口真正失焦时仍应暂停。
      windowListeners.get("blur")?.()
      expect(coordinator.getForeground()).toBe(false)

      // 短时间回前台不触发全局刷新，但 foreground 恢复。
      windowListeners.get("focus")?.()
      expect(coordinator.getForeground()).toBe(true)
      expect(coordinator.getSnapshot("dashboard").revision).toBe(
        initialRevision
      )
      coordinator.stop()
    } finally {
      Object.assign(globalThis, {
        window: originalWindow,
        document: originalDocument,
      })
      vi.useRealTimers()
    }
  })
})
