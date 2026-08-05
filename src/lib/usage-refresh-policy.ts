export const USAGE_ENTRY_REFRESH_MINIMUM_MS = 10_000

export function shouldRefreshUsageOnEntry(
  lastAutomaticRefreshAt: number | undefined,
  now = Date.now()
) {
  return (
    lastAutomaticRefreshAt === undefined ||
    now - lastAutomaticRefreshAt >= USAGE_ENTRY_REFRESH_MINIMUM_MS
  )
}
