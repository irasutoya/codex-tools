import type { AccountQuota, QuotaWindow } from "@/types"

export type QuotaRow = {
  label: string
  value: string
  detail?: string
  remainingPercent?: number
  resetAt?: number
}

export function quotaRows(quota?: AccountQuota): QuotaRow[] {
  const data = quota?.data
  if (!data) return []
  return [
    data.primary ? windowRow(data.primary, "主窗口") : undefined,
    data.secondary ? windowRow(data.secondary, "次窗口") : undefined,
  ].filter((row): row is QuotaRow => Boolean(row))
}

export function quotaStatusText(quota?: AccountQuota): string {
  switch (quota?.status ?? "never") {
    case "success":
      return "额度已更新"
    case "unsupported":
      return "暂不支持额度查询"
    case "unauthorized":
      return "凭据无效或无查询权限"
    case "rate_limited":
      return "查询过于频繁"
    case "error":
      return "额度查询失败"
    case "never":
      return "尚未查询额度"
  }
}

function windowRow(window: QuotaWindow, fallback: string): QuotaRow {
  return {
    label: windowLabel(window.windowSeconds, fallback),
    value: `剩余 ${window.remainingPercent}%`,
    detail: `已用 ${window.usedPercent}%`,
    remainingPercent: window.remainingPercent,
    resetAt: window.resetAt,
  }
}

function windowLabel(seconds: number | undefined, fallback: string): string {
  if (seconds === 18_000) return "5H"
  if (seconds === 604_800) return "7D"
  if (seconds && seconds % 86_400 === 0) return `${seconds / 86_400} 天`
  if (seconds && seconds % 3_600 === 0) return `${seconds / 3_600} 小时`
  return fallback
}
