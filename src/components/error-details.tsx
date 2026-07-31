import type { ReactNode } from "react"

import { formatError } from "@/lib/feedback"

export function ErrorDetails({
  error,
  children,
  action,
}: {
  error: unknown
  children: ReactNode
  action?: ReactNode
}) {
  return (
    <div className="flex flex-col gap-3">
      <p>{children}</p>
      <details>
        <summary className="cursor-pointer text-xs font-medium">
          查看错误详情
        </summary>
        <p className="mt-2 max-h-40 overflow-auto font-mono text-xs break-all whitespace-pre-wrap">
          {formatError(error)}
        </p>
      </details>
      {action && <div className="flex flex-wrap gap-2">{action}</div>}
    </div>
  )
}
