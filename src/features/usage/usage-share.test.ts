import { describe, expect, it, vi } from "vitest"

import type { UsageOverview, UsageRow } from "@/types"

import {
  buildUsageShareData,
  maskAccountName,
  renderSharePng,
  renderUsageShareSvg,
} from "./usage-share"

function row(
  overrides: Partial<UsageRow> &
    Pick<
      UsageRow,
      | "key"
      | "model"
      | "sourceKind"
      | "sourceName"
      | "accountId"
      | "tokens"
      | "requests"
      | "estimatedCostMicrousd"
      | "costStatus"
    >
): UsageRow {
  return overrides
}

const officialAccount = row({
  key: "account:official:official-account",
  model: "多个模型",
  sourceKind: "official",
  sourceName: "person@example.com",
  accountId: "official-account",
  tokens: {
    inputTokens: 40,
    cachedInputTokens: 0,
    cacheWriteInputTokens: 0,
    outputTokens: 20,
    reasoningOutputTokens: 0,
    totalTokens: 60,
  },
  requests: 2,
  estimatedCostMicrousd: 2_420_000,
  costStatus: "estimated",
})

const providerAccount = row({
  key: "account:provider:relay-account",
  model: "多个模型",
  sourceKind: "provider",
  sourceName: "开发中转站",
  providerId: "relay-provider",
  accountId: "relay-account",
  tokens: {
    inputTokens: 25,
    cachedInputTokens: 0,
    cacheWriteInputTokens: 0,
    outputTokens: 15,
    reasoningOutputTokens: 0,
    totalTokens: 40,
  },
  requests: 2,
  estimatedCostMicrousd: 1_000_000,
  costStatus: "estimated",
})

const accountOverview: UsageOverview = {
  range: { startAtMs: 0, endAtMs: 86_400_000 },
  totals: {
    tokens: {
      inputTokens: 65,
      cachedInputTokens: 0,
      cacheWriteInputTokens: 0,
      outputTokens: 35,
      reasoningOutputTokens: 0,
      totalTokens: 100,
    },
    requests: 4,
    estimatedCostMicrousd: 3_420_000,
    subscriptionTokens: 0,
    unpricedTokens: 0,
    partialTokens: 0,
    unattributedTokens: 0,
  },
  rows: [officialAccount, providerAccount],
  warnings: [],
  trendPoints: [],
}

const modelOverview: UsageOverview = {
  ...accountOverview,
  rows: [
    row({
      key: "model:gpt-5.6-luna:official:official-account",
      model: "gpt-5.6-luna",
      sourceKind: "official",
      sourceName: "person@example.com",
      accountId: "official-account",
      tokens: {
        inputTokens: 30,
        cachedInputTokens: 0,
        cacheWriteInputTokens: 0,
        outputTokens: 10,
        reasoningOutputTokens: 0,
        totalTokens: 40,
      },
      requests: 1,
      estimatedCostMicrousd: 1_500_000,
      costStatus: "estimated",
    }),
    row({
      key: "model:gpt-5.6-sol:official:official-account",
      model: "gpt-5.6-sol",
      sourceKind: "official",
      sourceName: "person@example.com",
      accountId: "official-account",
      tokens: {
        inputTokens: 10,
        cachedInputTokens: 0,
        cacheWriteInputTokens: 0,
        outputTokens: 10,
        reasoningOutputTokens: 0,
        totalTokens: 20,
      },
      requests: 1,
      estimatedCostMicrousd: 920_000,
      costStatus: "estimated",
    }),
    row({
      key: "model:gpt-5.6-luna:provider:relay-account",
      model: "gpt-5.6-luna",
      sourceKind: "provider",
      sourceName: "开发中转站",
      providerId: "relay-provider",
      accountId: "relay-account",
      tokens: {
        inputTokens: 15,
        cachedInputTokens: 0,
        cacheWriteInputTokens: 0,
        outputTokens: 15,
        reasoningOutputTokens: 0,
        totalTokens: 30,
      },
      requests: 1,
      estimatedCostMicrousd: 700_000,
      costStatus: "estimated",
    }),
    row({
      key: "model:gpt-5.6-terra:provider:relay-account",
      model: "gpt-5.6-terra",
      sourceKind: "provider",
      sourceName: "开发中转站",
      providerId: "relay-provider",
      accountId: "relay-account",
      tokens: {
        inputTokens: 10,
        cachedInputTokens: 0,
        cacheWriteInputTokens: 0,
        outputTokens: 0,
        reasoningOutputTokens: 0,
        totalTokens: 10,
      },
      requests: 1,
      estimatedCostMicrousd: 300_000,
      costStatus: "estimated",
    }),
  ],
}

describe("usage sharing", () => {
  it("masks account identifiers while preserving their domain", () => {
    expect(maskAccountName("person@example.com", "official")).toBe(
      "p***@example.com"
    )
    expect(maskAccountName("relay-a", "provider")).toBe("r***a")
  })

  it("keeps the same model separate under official and provider accounts", () => {
    const data = buildUsageShareData(
      accountOverview,
      modelOverview,
      "2026年8月1日",
      "Asia/Shanghai"
    )
    expect(data.totalTokens).toBe(100)
    expect(data.accounts).toHaveLength(2)
    const officialLuna = data.accounts[0]?.models.find(
      (model) => model.model === "gpt-5.6-luna"
    )
    const providerLuna = data.accounts[1]?.models.find(
      (model) => model.model === "gpt-5.6-luna"
    )
    expect(officialLuna?.totalTokens).toBe(40)
    expect(providerLuna?.totalTokens).toBe(30)
    expect(officialLuna?.key).not.toBe(providerLuna?.key)
  })

  it("renders account and model details without a global model merge", () => {
    const data = buildUsageShareData(
      accountOverview,
      modelOverview,
      "2026年8月1日",
      "Asia/Shanghai"
    )
    const svg = renderUsageShareSvg(data, "details")
    expect(svg).toContain("账号与模型明细")
    expect((svg.match(/gpt-5\.6-luna/g) ?? []).length).toBe(2)
    expect(svg).not.toContain("7.92M")
  })

  it("supports totals-only output without exposing account or model names", () => {
    const data = buildUsageShareData(
      accountOverview,
      modelOverview,
      "2026年8月1日",
      "Asia/Shanghai"
    )
    const svg = renderUsageShareSvg(data, "summary")
    expect(svg).toContain("今日 Token 用量")
    expect(svg).not.toContain("person@example.com")
    expect(svg).not.toContain("gpt-5.6-luna")
  })

  it("marks partial model costs as approximate instead of presenting a full price", () => {
    const partialOverview: UsageOverview = {
      ...modelOverview,
      rows: [
        {
          ...modelOverview.rows[0],
          costStatus: "partial",
          estimatedCostMicrousd: 1_000_000,
        },
      ],
    }
    const data = buildUsageShareData(
      accountOverview,
      partialOverview,
      "2026年8月1日",
      "Asia/Shanghai"
    )
    const officialLuna = data.accounts[0]?.models[0]
    expect(officialLuna?.partialTokens).toBe(40)
    expect(renderUsageShareSvg(data, "details")).toContain("约 $1.00")
  })

  it("marks mixed account costs as approximate when some tokens are unpriced", () => {
    const mixedAccount: UsageOverview = {
      ...accountOverview,
      rows: [
        {
          ...officialAccount,
          costStatus: "unpriced",
          estimatedCostMicrousd: 1_000_000,
        },
      ],
      totals: {
        ...accountOverview.totals,
        estimatedCostMicrousd: 1_000_000,
        unpricedTokens: officialAccount.tokens.totalTokens,
      },
    }
    const data = buildUsageShareData(
      mixedAccount,
      modelOverview,
      "2026年8月1日",
      "Asia/Shanghai"
    )
    expect(renderUsageShareSvg(data, "details")).toContain("约 $1.00")
  })

  it("renders PNG directly on canvas without decoding an SVG image", async () => {
    const gradient = { addColorStop: vi.fn() }
    const context = {
      scale: vi.fn(),
      fillRect: vi.fn(),
      createLinearGradient: vi.fn(() => gradient),
      save: vi.fn(),
      restore: vi.fn(),
      beginPath: vi.fn(),
      moveTo: vi.fn(),
      lineTo: vi.fn(),
      quadraticCurveTo: vi.fn(),
      closePath: vi.fn(),
      fill: vi.fn(),
      stroke: vi.fn(),
      fillText: vi.fn(),
      arc: vi.fn(),
    }
    const canvas = {
      width: 0,
      height: 0,
      getContext: () => context,
      toBlob: (callback: BlobCallback) =>
        callback(new Blob(["png"], { type: "image/png" })),
    }
    const imageConstructor = vi.fn(() => {
      throw new Error("SVG image decoding must not be used")
    })
    vi.stubGlobal("Image", imageConstructor)
    vi.stubGlobal("document", {
      createElement: (tag: string) => {
        if (tag === "canvas") return canvas
        throw new Error(`unexpected element: ${tag}`)
      },
    })

    try {
      const data = buildUsageShareData(
        accountOverview,
        modelOverview,
        "2026年8月1日",
        "Asia/Shanghai"
      )
      const result = await renderSharePng(
        data,
        "details",
        true,
        false,
        false,
        1
      )
      expect(result.type).toBe("image/png")
      expect(canvas.width).toBe(1080)
      expect(context.fillText).toHaveBeenCalled()
      expect(imageConstructor).not.toHaveBeenCalled()
    } finally {
      vi.unstubAllGlobals()
    }
  })
})
