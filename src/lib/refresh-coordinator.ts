import { useSyncExternalStore } from "react"

import type { Page } from "@/types"

import {
  getLocalDateKey,
  millisecondsUntilNextLocalMidnight,
} from "./local-time"

type RefreshSnapshot = {
  revision: number
  localDay: string
  foreground: boolean
}

type ActivitySnapshot = {
  foreground: boolean
  backgroundSince?: number
}

export const FOCUS_REFRESH_MINIMUM_MS = 30_000

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
  private activity: ActivitySnapshot
  private midnightTimer: number | undefined
  private cleanupActivity: (() => void) | undefined
  private started = false
  // Tauri WebView 初次加载时 document.hasFocus() 可能一直返回 false，即使窗口已经聚焦；
  // 因此初始乐观视为前台，之后由 focus/blur/visibilitychange 事件修正。
  private windowFocused = true

  constructor() {
    const localDay = getLocalDateKey()
    const foreground = this.isForeground()
    this.activity = {
      foreground,
      backgroundSince: foreground ? undefined : Date.now(),
    }
    for (const page of refreshablePages) {
      this.snapshots.set(page, { revision: 0, localDay, foreground })
    }
  }

  getSnapshot = (page: Page) => this.snapshots.get(page)!

  getForeground = () => this.activity.foreground

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
        foreground: this.activity.foreground,
      })
    }
    for (const listener of this.listeners) listener()
  }

  invalidateAll = () => this.invalidate(refreshablePages)

  start = () => {
    if (this.started || typeof window === "undefined") return
    this.started = true

    const handleFocus = () => {
      this.windowFocused = true
      this.updateForeground(this.isForeground())
    }
    const handleBlur = () => {
      this.windowFocused = false
      this.updateForeground(this.isForeground())
    }
    const handleVisibility = () => {
      this.updateForeground(this.isForeground())
    }

    const scheduleMidnight = () => {
      this.midnightTimer = window.setTimeout(() => {
        if (this.activity.foreground) {
          this.invalidate(["dashboard", "providers", "usage"])
        }
        scheduleMidnight()
      }, millisecondsUntilNextLocalMidnight())
    }

    window.addEventListener("focus", handleFocus)
    window.addEventListener("blur", handleBlur)
    document.addEventListener("visibilitychange", handleVisibility)
    this.cleanupActivity = () => {
      window.removeEventListener("focus", handleFocus)
      window.removeEventListener("blur", handleBlur)
      document.removeEventListener("visibilitychange", handleVisibility)
    }
    // 初次进入时同步一次真实状态（minimize 等场景），
    // 但不主动将 document.hasFocus() 的不可靠初值覆盖乐观前台状态。
    handleFocus()
    scheduleMidnight()
  }

  stop = () => {
    if (!this.started) return
    this.started = false
    if (this.midnightTimer !== undefined) {
      window.clearTimeout(this.midnightTimer)
      this.midnightTimer = undefined
    }
    this.cleanupActivity?.()
  }

  private isForeground() {
    if (typeof document === "undefined") return true
    if (document.visibilityState === "hidden") return false
    return this.windowFocused
  }

  private updateForeground(foreground: boolean) {
    if (foreground === this.activity.foreground) return

    const now = Date.now()
    if (!foreground) {
      this.activity = { foreground: false, backgroundSince: now }
      this.updateSnapshotForeground(false)
      return
    }

    const backgroundSince = this.activity.backgroundSince ?? now
    const localDay = getLocalDateKey()
    const dayChanged = this.snapshots.get("dashboard")?.localDay !== localDay
    const awayLongEnough = now - backgroundSince >= FOCUS_REFRESH_MINIMUM_MS
    this.activity = { foreground: true }
    this.updateSnapshotForeground(true)
    if (dayChanged || awayLongEnough) this.invalidateAll()
  }

  private updateSnapshotForeground(foreground: boolean) {
    for (const [page, current] of this.snapshots) {
      this.snapshots.set(page, { ...current, foreground })
    }
    for (const listener of this.listeners) listener()
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

export function useAppForeground() {
  return useSyncExternalStore(
    refreshCoordinator.subscribe,
    refreshCoordinator.getForeground,
    refreshCoordinator.getForeground
  )
}
