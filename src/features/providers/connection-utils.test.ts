import { describe, expect, it } from "vitest"

import type { RepairResult } from "@/types"

import { repairWarning } from "./connection-utils"

const repair = (overrides: Partial<RepairResult> = {}): RepairResult => ({
  targetProvider: "openai",
  filesScanned: 0,
  filesModified: 0,
  filesSkipped: 0,
  filesFailed: 0,
  sessionMetaUpdated: 0,
  rowsUpdated: 0,
  warnings: [],
  ...overrides,
})

describe("repairWarning", () => {
  it("stays silent after a complete repair", () => {
    expect(repairWarning(repair())).toBeUndefined()
  })

  it("surfaces partial repair failures and backend warnings", () => {
    expect(
      repairWarning(
        repair({ filesFailed: 2, warnings: ["数据库被占用", "索引刷新失败"] })
      )
    ).toContain("2 个会话文件修复失败；数据库被占用；索引刷新失败")
  })
})
