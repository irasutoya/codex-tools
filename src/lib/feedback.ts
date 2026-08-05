import { toast } from "sonner"

const MAX_ERROR_DETAIL_LENGTH = 2_000

function formatError(error: unknown) {
  const raw =
    error instanceof Error
      ? error.message
      : typeof error === "string"
        ? error
        : error === null || error === undefined
          ? ""
          : String(error)
  const boundedRaw = raw.slice(0, MAX_ERROR_DETAIL_LENGTH + 1)
  // eslint-disable-next-line no-control-regex -- 移除错误详情中的控制字符是刻意行为
  const sanitized = boundedRaw.replace(/[\x00-\x08\x0B\x0C\x0E-\x1F\x7F]/g, "")
  const message = sanitized
    .trim()
    .replace(/^Error:\s*/i, "")
    .trim()

  if (!message) return "未提供错误详情。"
  if (
    raw.length <= MAX_ERROR_DETAIL_LENGTH &&
    message.length <= MAX_ERROR_DETAIL_LENGTH
  ) {
    return message
  }
  return `${message.slice(0, MAX_ERROR_DETAIL_LENGTH)}…（错误详情已截断）`
}

function options(detail?: unknown) {
  if (detail === undefined) return undefined
  return { description: formatError(detail) }
}

const notify = {
  success(message: string, detail?: string) {
    toast.success(message, detail ? { description: detail } : undefined)
  },
  info(message: string, detail?: string) {
    toast.info(message, detail ? { description: detail } : undefined)
  },
  warning(message: string, detail?: unknown) {
    toast.warning(message, options(detail))
  },
  error(message: string, detail?: unknown) {
    toast.error(message, options(detail))
  },
}

export { formatError, notify }
