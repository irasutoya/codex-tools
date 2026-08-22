import { ask, save } from "@tauri-apps/plugin-dialog"

import { call } from "@/lib/ipc"
import type { AccountQuota, ProviderTestResult } from "@/types"

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

function exportFileName(now = new Date()) {
  const part = (value: number) => String(value).padStart(2, "0")
  return `codex-tools-account-${now.getFullYear()}${part(
    now.getMonth() + 1
  )}${part(now.getDate())}-${part(now.getHours())}${part(
    now.getMinutes()
  )}${part(now.getSeconds())}.json`
}

export async function exportAccountCredentials(id: string) {
  const confirmed = await ask(
    "导出的文件包含可接管此账号的登录凭据。转移到新电脑后请立即删除该文件；新电脑刷新登录后，旧电脑的 Refresh Token 可能失效。是否继续？",
    { title: "导出登录凭据警告", kind: "warning" }
  )
  if (!confirmed) return false

  const path = await save({
    title: "导出登录凭据",
    defaultPath: exportFileName(),
    filters: [{ name: "JSON", extensions: ["json"] }],
  })
  if (!path) return false

  await call("connections_export_account", { id, path })
  return true
}
