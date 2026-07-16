import { toast } from "sonner"

function formatError(error: unknown) {
  const message =
    error instanceof Error
      ? error.message
      : typeof error === "string"
        ? error
        : String(error)

  return message.replace(/^Error:\s*/i, "").trim() || "没有返回更多错误信息"
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
