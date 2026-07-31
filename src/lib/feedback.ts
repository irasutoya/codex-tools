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
  let sanitized = ""
  for (const character of boundedRaw) {
    const code = character.charCodeAt(0)
    if (
      code === 9 ||
      code === 10 ||
      code === 13 ||
      (code >= 32 && code !== 127)
    ) {
      sanitized += character
    }
  }
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
