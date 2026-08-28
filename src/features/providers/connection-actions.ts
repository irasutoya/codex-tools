import { call } from "@/lib/ipc"
import type {
  AccountQuota,
  ProviderTestResult,
  QuotaEstimateResult,
} from "@/types"

const quotaFailureMessage: Record<
  Exclude<AccountQuota["status"], "success">,
  string
> = {
  never: "尚未获取额度数据",
  unsupported: "当前账号暂不支持额度查询",
  unauthorized: "登录已失效，请先刷新登录",
  rate_limited: "请求频繁，请稍后重试",
  error: "额度刷新失败",
}

export async function refreshAccountQuota(accountId: string) {
  const result = await call("connections_refresh_quota", { accountId })
  if (result.status !== "success") {
    throw new Error(result.error || quotaFailureMessage[result.status])
  }
  return result
}

export function estimateAccountQuota(
  accountId: string
): Promise<QuotaEstimateResult> {
  return call("connections_estimate_quota", { accountId })
}

export function refreshAccountLogin(id: string) {
  return call("connections_refresh_login", { id })
}

export function syncProviderModels(id: string) {
  return call("connections_list_models", { id })
}

export async function testProviderConnection(id: string) {
  const result: ProviderTestResult = await call("connections_test_provider", {
    id,
  })
  if (!result.ok) {
    throw new Error(
      result.message ||
        (result.status > 0 ? `服务返回 HTTP ${result.status}` : "连接测试失败")
    )
  }
  return result
}
