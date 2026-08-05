export function taskFailureTitle(task: string) {
  if (task.startsWith("account:test:")) return "无法完成连接测试"
  if (task === "provider:save") return "无法保存 API 服务"
  if (task === "account:save") return "无法保存 API Key"
  if (task === "proxy:login") return "无法导入 Cookie 账号"
  if (task.startsWith("official:quota:")) return "无法刷新 OpenAI 额度"
  if (task === "openai:login") return "无法开始 OpenAI 登录"
  if (task.startsWith("account:activate:")) return "无法切换 API 服务"
  if (task.startsWith("openai:activate:")) return "无法切换 OpenAI 账号"
  if (task.startsWith("openai:delete:")) return "无法删除 OpenAI 账号"
  if (task.startsWith("delete:provider:")) return "无法删除 API 服务"
  if (task.startsWith("delete:account:")) return "无法删除 API Key"
  return "操作未完成"
}
