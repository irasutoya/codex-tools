import type { ReactNode } from "react"

import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from "@/components/ui/collapsible"
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
      <p className="text-sm">{children}</p>
      <Collapsible>
        <CollapsibleTrigger className="w-fit text-xs font-medium text-muted-foreground underline underline-offset-4 hover:text-foreground">
          查看错误详情
        </CollapsibleTrigger>
        <CollapsibleContent>
          <p className="max-h-40 overflow-auto rounded-md bg-muted p-3 font-mono text-xs break-all whitespace-pre-wrap">
            {formatError(error)}
          </p>
        </CollapsibleContent>
      </Collapsible>
      {action && <div className="flex flex-wrap gap-2">{action}</div>}
    </div>
  )
}
