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
  private activityListener: (() => void) | undefined
  private started = false

  constructor() {
    const localDay = getLocalDateKey()
    const foreground = isForeground()
    this.activity = {
      foreground,
      backgroundSince: foreground ? undefined : Date.now(),
    }
    for (const page of refreshablePages) {
      this.snapshots.set(page, { revision: 0, localDay, foreground })
    }
  }

  getSnapshot = (page: Page) => this.snapshots.get(page)!

  getActivitySnapshot = () => this.activity

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

    const handleActivity = () => this.updateForeground(isForeground())

    const scheduleMidnight = () => {
      this.midnightTimer = window.setTimeout(() => {
        if (this.activity.foreground) {
          this.invalidate(["dashboard", "providers", "usage"])
        }
        scheduleMidnight()
      }, millisecondsUntilNextLocalMidnight())
    }

    this.activityListener = handleActivity
    window.addEventListener("focus", handleActivity)
    window.addEventListener("blur", handleActivity)
    document.addEventListener("visibilitychange", handleActivity)
    handleActivity()
    scheduleMidnight()
  }

  stop = () => {
    if (!this.started) return
    this.started = false
    if (this.midnightTimer !== undefined) {
      window.clearTimeout(this.midnightTimer)
      this.midnightTimer = undefined
    }
    if (this.activityListener) {
      window.removeEventListener("focus", this.activityListener)
      window.removeEventListener("blur", this.activityListener)
      document.removeEventListener("visibilitychange", this.activityListener)
      this.activityListener = undefined
    }
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

function isForeground() {
  if (typeof document === "undefined") return true
  return (
    document.visibilityState === "visible" &&
    (typeof document.hasFocus !== "function" || document.hasFocus())
  )
}
