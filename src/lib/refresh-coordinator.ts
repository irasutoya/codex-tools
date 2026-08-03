import { useSyncExternalStore } from "react"

import type { Page } from "@/types"

import {
  getLocalDateKey,
  millisecondsUntilNextLocalMidnight,
} from "./local-time"

type RefreshSnapshot = {
  revision: number
  localDay: string
}

const refreshablePages: Page[] = [
  "dashboard",
  "providers",
  "usage",
  "sessions",
  "settings",
]

class RefreshCoordinator {
  private readonly listeners = new Set<() => void>()
  private readonly snapshots = new Map<Page, RefreshSnapshot>()
  private midnightTimer: number | undefined
  private focusListener: (() => void) | undefined
  private visibilityListener: (() => void) | undefined
  private started = false

  constructor() {
    const localDay = getLocalDateKey()
    for (const page of refreshablePages) {
      this.snapshots.set(page, { revision: 0, localDay })
    }
  }

  getSnapshot = (page: Page) => this.snapshots.get(page)!

  subscribe = (listener: () => void) => {
    this.listeners.add(listener)
    return () => this.listeners.delete(listener)
  }

  invalidate = (pages: Page[]) => {
    const localDay = getLocalDateKey()
    for (const page of pages) {
      const current = this.snapshots.get(page)!
      this.snapshots.set(page, {
        revision: current.revision + 1,
        localDay,
      })
    }
    for (const listener of this.listeners) listener()
  }

  invalidateAll = () => this.invalidate(refreshablePages)

  start = () => {
    if (this.started || typeof window === "undefined") return
    this.started = true

    const checkExternalChanges = () => {
      if (document.visibilityState === "visible") this.invalidateAll()
    }

    const scheduleMidnight = () => {
      this.midnightTimer = window.setTimeout(() => {
        if (document.visibilityState === "visible") {
          this.invalidate(["dashboard", "providers", "usage"])
        }
        scheduleMidnight()
      }, millisecondsUntilNextLocalMidnight())
    }

    this.focusListener = checkExternalChanges
    this.visibilityListener = checkExternalChanges
    window.addEventListener("focus", checkExternalChanges)
    document.addEventListener("visibilitychange", checkExternalChanges)
    scheduleMidnight()
  }

  stop = () => {
    if (!this.started) return
    this.started = false
    if (this.midnightTimer !== undefined) {
      window.clearTimeout(this.midnightTimer)
      this.midnightTimer = undefined
    }
    if (this.focusListener) {
      window.removeEventListener("focus", this.focusListener)
      this.focusListener = undefined
    }
    if (this.visibilityListener) {
      document.removeEventListener("visibilitychange", this.visibilityListener)
      this.visibilityListener = undefined
    }
  }
}

export const refreshCoordinator = new RefreshCoordinator()
export { RefreshCoordinator }

export function usePageRefresh(page: Page) {
  return useSyncExternalStore(
    refreshCoordinator.subscribe,
    () => refreshCoordinator.getSnapshot(page),
    () => refreshCoordinator.getSnapshot(page)
  )
}
