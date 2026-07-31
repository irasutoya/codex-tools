import { notify } from "@/lib/feedback"
import type { RepairResult } from "@/types"

export function notifyRepairWarnings(
  result: RepairResult,
  completedAction = "连接已切换"
) {
  if (result.warnings.length === 0 && result.filesFailed === 0) return

  const issueCount = Math.max(result.warnings.length, result.filesFailed)
  const firstWarning = result.warnings[0]
  notify.warning(
    "部分历史会话未更新",
    `${completedAction}，但有 ${issueCount} 项未能自动处理。${
      firstWarning ? `首项原因：${firstWarning}` : ""
    }`
  )
}
