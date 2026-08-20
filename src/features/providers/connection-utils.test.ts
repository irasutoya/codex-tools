import { describe, expect, it } from "vitest"

import type { OfficialAccountView, Provider, RepairResult } from "@/types"

import {
  addCustomModelTo,
  allModelsSelected,
  accountWorkspaceIsDeactivated,
  buildFallbackCandidates,
  effectiveModelsOf,
  noModelsSelected,
  providerSaveInputOf,
  removeCustomModelFrom,
  repairWarning,
  quotaStatusText,
  toggleModelSelected,
} from "./connection-utils"

const makeAccount = (
  overrides: Partial<OfficialAccountView> = {}
): OfficialAccountView => ({
  id: "a1",
  name: "账号",
  remark: "",
  accountId: "workspace-1",
  email: "account@example.com",
  source: "open_ai_oauth",
  expiresAt: null,
  quota: { status: "never" },
  deviceSessionConvergenceAvailable: false,
  deviceSessionConvergenceEnabled: false,
  active: false,
  createdAt: 0,
  updatedAt: 0,
  ...overrides,
})

const makeProvider = (overrides: Partial<Provider> = {}): Provider => ({
  id: "p1",
  name: "服务",
  baseUrl: "https://api.example.com/v1",
  headers: {},
  timeoutSecs: 30,
  enabled: true,
  active: false,
  apiType: "responses",
  createdAt: 0,
  updatedAt: 0,
  ...overrides,
})

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

describe("providerSaveInputOf", () => {
  it("将默认全选明确序列化为 null", () => {
    const input = providerSaveInputOf(
      makeProvider({
        availableModels: ["a", "b"],
        selectedModels: undefined,
      })
    )
    expect(input.selectedModels).toBeNull()
    expect(input.customModels).toEqual([])
  })

  it("保留显式模型子集", () => {
    expect(
      providerSaveInputOf(
        makeProvider({
          availableModels: ["a", "b"],
          selectedModels: ["b"],
        })
      ).selectedModels
    ).toEqual(["b"])
  })
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

describe("账号额度错误", () => {
  it("优先展示后端给出的具体 HTTP 错误说明", () => {
    const account = makeAccount({
      quota: {
        status: "rate_limited",
        error: "OpenAI 请求过于频繁（HTTP 429：触发速率限制），请稍后重试",
      },
    })
    expect(quotaStatusText(account)).toContain("HTTP 429：触发速率限制")
  })

  it("识别已停用工作区并从自动切换候选中排除", () => {
    const deactivated = makeAccount({
      id: "deactivated",
      quota: {
        status: "unauthorized",
        errorCode: "deactivated_workspace",
        error: "账号所属工作区已停用（HTTP 402）",
      },
    })
    const usable = makeAccount({ id: "usable" })

    expect(accountWorkspaceIsDeactivated(deactivated)).toBe(true)
    expect(
      buildFallbackCandidates([deactivated, usable], []).map(
        (candidate) => candidate.id
      )
    ).toEqual(["usable"])
  })
})

describe("effectiveModelsOf", () => {
  it("合并同步模型与自定义模型并保持顺序去重", () => {
    const provider = makeProvider({
      availableModels: ["gpt-4o", "gpt-4o-mini"],
      customModels: ["gpt-4o", "my-model"],
    })
    expect(effectiveModelsOf(provider)).toEqual([
      "gpt-4o",
      "gpt-4o-mini",
      "my-model",
    ])
  })
})

describe("allModelsSelected / noModelsSelected", () => {
  it("undefined（未设置）视为全选", () => {
    const provider = makeProvider({
      availableModels: ["a", "b"],
      selectedModels: undefined,
    })
    expect(allModelsSelected(provider)).toBe(true)
    expect(noModelsSelected(provider)).toBe(false)
  })

  it("空数组不是全选而是无选中", () => {
    const provider = makeProvider({
      availableModels: ["a", "b"],
      selectedModels: [],
    })
    expect(allModelsSelected(provider)).toBe(false)
    expect(noModelsSelected(provider)).toBe(true)
  })

  it("选中全部时视为全选", () => {
    const provider = makeProvider({
      availableModels: ["a", "b"],
      selectedModels: ["a", "b"],
    })
    expect(allModelsSelected(provider)).toBe(true)
    expect(noModelsSelected(provider)).toBe(false)
  })
})

describe("null 处理（后端 Option::None 序列化为 null）", () => {
  it("null 等同于 undefined（全选）", () => {
    const provider = makeProvider({
      availableModels: ["a", "b"],
      selectedModels: null as unknown as string[] | undefined,
    })
    expect(allModelsSelected(provider)).toBe(true)
    expect(noModelsSelected(provider)).toBe(false)
  })

  it("null 时 toggleModelSelected 基于有效模型全选计算", () => {
    const provider = makeProvider({
      availableModels: ["a", "b"],
      selectedModels: null as unknown as string[] | undefined,
    })
    // 取消 a，应返回仅含 b 的数组
    expect(toggleModelSelected(provider, "a", false)).toEqual(["b"])
  })

  it("null 时 addCustomModelTo 保持全选语义", () => {
    const provider = makeProvider({
      availableModels: ["a", "b"],
      selectedModels: null as unknown as string[] | undefined,
    })
    const next = addCustomModelTo(provider, "custom")
    expect(next.customModels).toEqual(["custom"])
    expect(next.selectedModels).toBeUndefined()
  })
})

describe("toggleModelSelected", () => {
  it("未设置 selectedModels 时首次取消返回空数组（BUG1 单次反选）", () => {
    const provider = makeProvider({
      availableModels: ["a", "b", "c"],
      selectedModels: undefined,
    })
    const next = toggleModelSelected(provider, "a", false)
    expect(next).toEqual(["b", "c"])
  })

  it("取消后重新勾选返回 undefined（全选语义）", () => {
    const provider = makeProvider({
      availableModels: ["a", "b", "c"],
      selectedModels: ["b", "c"],
    })
    expect(toggleModelSelected(provider, "a", true)).toBeUndefined()
  })

  it("连续基于最新状态切换多个模型正确（BUG2 回归）", () => {
    let provider = makeProvider({
      availableModels: ["a", "b", "c", "d"],
      selectedModels: undefined,
    })
    // 第一次取消 a
    provider = {
      ...provider,
      selectedModels: toggleModelSelected(provider, "a", false),
    }
    expect(provider.selectedModels).toEqual(["b", "c", "d"])
    // 第二次取消 c（模拟搜索词改变后再切换第二个模型）
    provider = {
      ...provider,
      selectedModels: toggleModelSelected(provider, "c", false),
    }
    expect(provider.selectedModels).toEqual(["b", "d"])
  })
})

describe("addCustomModelTo", () => {
  it("在非全选态添加时保留 customModels 与 selectedModels（回归双 update bug）", () => {
    let provider = makeProvider({
      availableModels: ["a", "b"],
      selectedModels: ["b"],
    })
    provider = addCustomModelTo(provider, "my-model")
    expect(provider.customModels).toEqual(["my-model"])
    expect(provider.selectedModels).toEqual(["b", "my-model"])
    expect(effectiveModelsOf(provider)).toEqual(["a", "b", "my-model"])
  })

  it("全选态（undefined）添加时保持默认全选语义", () => {
    const provider = makeProvider({
      availableModels: ["a", "b"],
      selectedModels: undefined,
    })
    const next = addCustomModelTo(provider, "my-model")
    expect(next.customModels).toEqual(["my-model"])
    expect(next.selectedModels).toBeUndefined()
  })
})

describe("removeCustomModelFrom", () => {
  it("移除时从 selectedModels 剔除并回退 undefined", () => {
    let provider = makeProvider({
      availableModels: ["a", "b"],
      customModels: ["my-model"],
      selectedModels: ["a", "b", "my-model"],
    })
    provider = removeCustomModelFrom(provider, "my-model")
    expect(provider.customModels).toEqual([])
    // 剔除后选中 a+b = 剩余全部有效模型，回退 undefined（全选）。
    expect(provider.selectedModels).toBeUndefined()
  })

  it("移除后若还有未选模型则保留 selectedModels", () => {
    let provider = makeProvider({
      availableModels: ["a", "b", "c"],
      customModels: ["my-model"],
      selectedModels: ["b", "my-model"],
    })
    provider = removeCustomModelFrom(provider, "my-model")
    expect(provider.customModels).toEqual([])
    expect(provider.selectedModels).toEqual(["b"])
  })
})
