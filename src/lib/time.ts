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

const timestampFormatters = {
  default: new Intl.DateTimeFormat("zh-CN", {
    dateStyle: "medium",
    timeStyle: "short",
  }),
  compact: new Intl.DateTimeFormat("zh-CN", {
    dateStyle: "short",
    timeStyle: "medium",
  }),
} as const

export type TimestampFormat = keyof typeof timestampFormatters
export function formatDateTime(
  value: number,
  format: TimestampFormat = "default"
) {
  const date = new Date(epochMilliseconds(value))
  if (Number.isNaN(date.getTime())) return "时间未知"
  return timestampFormatters[format].format(date)
}
