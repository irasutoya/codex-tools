export function getLocalDateKey(now = new Date()) {
  const pad = (value: number) => String(value).padStart(2, "0")
  return `${now.getFullYear()}-${pad(now.getMonth() + 1)}-${pad(now.getDate())}`
}

export function millisecondsUntilNextLocalMidnight(now = new Date()) {
  const next = new Date(now)
  next.setHours(0, 0, 0, 0)
  next.setDate(next.getDate() + 1)
  return Math.max(250, next.getTime() - now.getTime())
}
