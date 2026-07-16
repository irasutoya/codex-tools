const MILLISECOND_EPOCH_THRESHOLD = 100_000_000_000
const MICROSECOND_EPOCH_THRESHOLD = 100_000_000_000_000
const NANOSECOND_EPOCH_THRESHOLD = 100_000_000_000_000_000

export function epochMilliseconds(value: number) {
  if (!Number.isFinite(value)) return Number.NaN

  const absolute = Math.abs(value)
  if (absolute >= NANOSECOND_EPOCH_THRESHOLD) return value / 1_000_000
  if (absolute >= MICROSECOND_EPOCH_THRESHOLD) return value / 1_000
  if (absolute < MILLISECOND_EPOCH_THRESHOLD) return value * 1_000
  return value
}
