import { describe, expect, it } from "vitest"

import type { PricingRule, Provider } from "@/types"

import {
  findEquivalentPricingRule,
  pricingScopeForSource,
  pricingSourceFromValue,
  pricingSourceLabel,
  pricingSourceOptions,
  pricingSourceValue,
  pricingSourceValueForRule,
  pricingSummary,
} from "./pricing"

const providers: Provider[] = [
  {
    id: "relay-a",
    name: "中转 A",
    baseUrl: "https://a.example.com",
    headers: {},
    timeoutSecs: 30,
    enabled: true,
    active: false,
    apiType: "responses",
    createdAt: 0,
    updatedAt: 0,
  },
  {
    id: "relay-disabled",
    name: "中转 B",
    baseUrl: "https://b.example.com",
    headers: {},
    timeoutSecs: 30,
    enabled: false,
    active: false,
    apiType: "responses",
    createdAt: 0,
    updatedAt: 0,
  },
]

function rule(overrides: Partial<PricingRule> = {}): PricingRule {
  return {
    id: "rule",
    version: 1,
    active: true,
    scopeKind: "provider_model",
    providerId: "relay-a",
    modelPattern: "gpt-5.6",
    matchKind: "exact",
    billingMode: "token",
    cacheWriteIncludedInInput: true,
    effectiveFromMs: 0,
    createdAtMs: 0,
    updatedAtMs: 0,
    ...overrides,
  }
}

describe("价格规则来源", () => {
  it("将通用和具体 Provider 映射为可保存的范围", () => {
    const global = pricingSourceFromValue(
      pricingSourceValue({ kind: "global" })
    )
    const provider = pricingSourceFromValue("relay-a")

    expect(pricingScopeForSource(global)).toEqual({
      scopeKind: "global_model",
      providerId: undefined,
      accountId: undefined,
    })
    expect(pricingScopeForSource(provider)).toEqual({
      scopeKind: "provider_model",
      providerId: "relay-a",
      accountId: undefined,
    })
  })

  it("未选择来源时不会产生有效范围", () => {
    expect(pricingSourceFromValue("")).toBeUndefined()
    expect(pricingScopeForSource(undefined)).toBeUndefined()
  })

  it("为来源 Select 提供内部值到可读标签的映射", () => {
    expect(pricingSourceOptions(providers)).toEqual([
      {
        value: "__all_third_party_apis__",
        label: "所有第三方 API（通用规则）",
      },
      {
        value: "__provider__:relay-a",
        label: "中转 A",
      },
      {
        value: "__provider__:relay-disabled",
        label: "中转 B（已停用）",
      },
    ])
    expect(
      pricingSourceValueForRule(
        rule({
          scopeKind: "global_model",
          providerId: undefined,
        })
      )
    ).toBe("__all_third_party_apis__")
    expect(pricingSourceValueForRule(rule())).toBe("__provider__:relay-a")
  })

  it("同模型的不同 Provider 不会互相替换", () => {
    const relayA = rule()
    const relayB = rule({ id: "rule-b", providerId: "relay-b" })

    expect(
      findEquivalentPricingRule([relayA, relayB], {
        scopeKind: "provider_model",
        providerId: "relay-b",
        accountId: undefined,
        modelPattern: "gpt-5.6",
        matchKind: "exact",
      })
    ).toBe(relayB)
    expect(
      findEquivalentPricingRule([relayA, relayB], {
        scopeKind: "global_model",
        providerId: undefined,
        accountId: undefined,
        modelPattern: "gpt-5.6",
        matchKind: "exact",
      })
    ).toBeUndefined()
  })

  it("显示通用、停用和已删除来源", () => {
    expect(
      pricingSourceLabel(
        rule({ scopeKind: "global_model", providerId: undefined }),
        providers
      )
    ).toBe("所有第三方 API")
    expect(
      pricingSourceLabel(rule({ providerId: "relay-disabled" }), providers)
    ).toBe("中转 B（已停用）")
    expect(
      pricingSourceLabel(rule({ providerId: "deleted-provider" }), providers)
    ).toBe("deleted-provider")
  })

  it("计费摘要不暴露内部规则范围", () => {
    expect(
      pricingSummary(
        rule({
          inputUsdPerMillion: "1",
          cachedReadUsdPerMillion: "0.1",
          cacheWriteUsdPerMillion: "1.2",
          outputUsdPerMillion: "3",
        })
      )
    ).toBe("输入 $1 · 缓存读取 $0.1 · 缓存写入 $1.2 · 输出 $3 / 1M")
    expect(pricingSummary(rule({ billingMode: "unpriced" }))).toBe("不计价")
    expect(pricingSummary(rule({ billingMode: "subscription" }))).toBe(
      "旧版订阅规则（将按不计价迁移）"
    )
  })
})
