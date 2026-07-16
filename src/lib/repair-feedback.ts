import { notify } from "@/lib/feedback"
import type { RepairResult } from "@/types"

export function notifyRepairWarnings(result: RepairResult) {
  if (result.warnings.length === 0 && result.filesFailed === 0) return

  const issueCount = Math.max(result.warnings.length, result.filesFailed)
  notify.warning(
    "部分历史会话未能自动更新",
    `连接已切换，但有 ${issueCount} 项需要检查。请前往“历史会话”查看详情。`
  )
}
