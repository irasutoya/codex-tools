import type { Account, Provider } from "@/types"

export const blankProvider: Provider = {
  id: "",
  name: "",
  protocol: "responses",
  baseUrl: "https://api.openai.com/v1",
  defaultModel: "gpt-5",
  models: [],
  codexChatReasoning: undefined,
  headers: {},
  timeoutSecs: 30,
  enabled: true,
  active: false,
  accountCount: 0,
}

export function blankAccount(providerId: string): Account {
  return {
    id: "",
    providerId,
    name: "默认账号",
    authKind: "api_key",
    apiKey: "",
    headers: {},
    active: false,
    createdAt: 0,
    updatedAt: 0,
  }
}

export function commaList(value: string) {
  return value
    .split(",")
    .map((item) => item.trim())
    .filter(Boolean)
}

export function optionalNumber(value: string) {
  const parsed = Number(value)
  return value && Number.isFinite(parsed) && parsed > 0 ? parsed : undefined
}

export function parseHeaders(
  value: string
): Record<string, string> | undefined {
  try {
    const parsed: unknown = JSON.parse(value || "{}")
    if (
      parsed &&
      typeof parsed === "object" &&
      !Array.isArray(parsed) &&
      Object.values(parsed).every((item) => typeof item === "string")
    ) {
      return parsed as Record<string, string>
    }
  } catch {
    // An incomplete JSON value remains editable until it becomes valid.
  }
  return undefined
}

export function maskKey(value?: string) {
  if (!value) return "未设置"
  if (value.length < 9) return "••••••••"
  return `${value.slice(0, 4)}••••${value.slice(-4)}`
}
