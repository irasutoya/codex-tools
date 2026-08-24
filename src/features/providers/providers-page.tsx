import { useCallback, useEffect, useMemo, useState } from "react"
import {
  Add01Icon,
  CheckmarkCircle02Icon,
  Key01Icon,
  Login03Icon,
  Refresh01Icon,
  TestTube01Icon,
} from "@hugeicons/core-free-icons"
import { HugeiconsIcon } from "@hugeicons/react"

import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import {
  Card,
  CardContent,
  CardFooter,
  CardHeader,
  CardTitle,
} from "@/components/ui/card"
import {
  Empty,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from "@/components/ui/empty"
import { Spinner } from "@/components/ui/spinner"
import { Skeleton } from "@/components/ui/skeleton"
import { toast } from "@/components/ui/toast"
import { errorMessage, formatDate, quotaWindow } from "@/lib/format"
import { useAsyncAction } from "@/hooks/use-async-action"
import { call } from "@/lib/ipc"
import type {
  DeviceAuthorization,
  OfficialAccountView,
  Provider,
  ProviderOverview,
} from "@/types"
import { emptyProvider } from "./connection-utils"

import {
  AccountLoginDialog,
  type AccountLoginMode,
} from "./account-login-dialog"
import {
  refreshAccountQuota,
  testProviderConnection,
} from "./connection-actions"
import {
  accountPlanText,
  credentialMaintenanceMessage,
  credentialRefreshText,
  loginVerificationText,
  effectiveModelCount,
  quotaStatusText,
} from "./connection-utils"
import { ProviderEditorDialog } from "./provider-editor-dialog"

export function ProvidersPage({
  connections,
  selectedId,
  onSelectedIdChange,
  onRefresh,
}: {
  refreshRevision: number
  onRefresh: () => void
  connections?: ProviderOverview
  selectedId?: string
  onSelectedIdChange: (id: string) => void
}) {
  const [editorOpen, setEditorOpen] = useState(false)
  const [loginOpen, setLoginOpen] = useState(false)
  const [loginMode, setLoginMode] = useState<AccountLoginMode>("browser")
  const [loginError, setLoginError] = useState<string>()
  const [editor, setEditor] = useState<Provider>(emptyProvider())
  const [authorization, setAuthorization] = useState<DeviceAuthorization>()
  const { busy, begin, end, run } = useAsyncAction<string>()

  const selected = useMemo(() => {
    const account = connections?.officialAccounts.find(
      (item) => item.id === selectedId
    )
    if (account) return { kind: "account" as const, value: account }
    const provider = connections?.providers.find(
      (item) => item.id === selectedId
    )
    if (provider) return { kind: "provider" as const, value: provider }
    const fallback =
      connections?.officialAccounts.find((item) => item.active) ??
      connections?.providers.find((item) => item.active) ??
      connections?.officialAccounts[0] ??
      connections?.providers[0]
    if (!fallback) return undefined
    return "email" in fallback
      ? { kind: "account" as const, value: fallback }
      : { kind: "provider" as const, value: fallback }
  }, [connections, selectedId])

  useEffect(() => {
    if (selected && selectedId !== selected.value.id) {
      onSelectedIdChange(selected.value.id)
    }
  }, [onSelectedIdChange, selected, selectedId])

  const startLogin = async () => {
    if (!begin("login")) return
    setLoginError(undefined)
    try {
      setAuthorization(await call("connections_login_start"))
    } catch (reason) {
      setLoginError(errorMessage(reason))
      toast.add({
        title: "无法获取 OpenAI 授权码",
        description: errorMessage(reason),
        type: "error",
      })
    } finally {
      end("login")
    }
  }

  const finishLoginPoll = useCallback(
    (result: Awaited<ReturnType<typeof call<"connections_login_poll">>>) => {
      if (result.status === "complete") {
        toast.add({
          title: "OpenAI 账号已保存",
          description: "当前连接未切换；需要使用时请在账号列表中手动切换。",
          type: "success",
        })
        setAuthorization(undefined)
        setLoginOpen(false)
        onSelectedIdChange(result.account.id)
        onRefresh()
        return true
      }
      if (result.status === "expired") {
        setAuthorization(undefined)
        setLoginError("授权码已过期，请重新获取授权码。")
        return true
      }
      return false
    },
    [onRefresh, onSelectedIdChange]
  )

  const checkLogin = async () => {
    if (!authorization) return
    if (!begin("poll")) return
    try {
      const result = await call("connections_login_poll", {
        operationId: authorization.operationId,
      })
      if (!finishLoginPoll(result)) {
        toast.add({
          title: "OpenAI 尚未确认授权",
        })
      }
    } catch (reason) {
      toast.add({
        title: "无法检查授权结果",
        description: errorMessage(reason),
        type: "error",
      })
    } finally {
      end("poll")
    }
  }

  useEffect(() => {
    if (!authorization || !loginOpen || loginMode !== "browser") return
    let cancelled = false
    let timer: number | undefined

    const schedule = () => {
      timer = window.setTimeout(
        () => {
          if (!begin("poll")) {
            schedule()
            return
          }
          void call("connections_login_poll", {
            operationId: authorization.operationId,
          })
            .then((result) => {
              if (cancelled || finishLoginPoll(result)) return
              schedule()
            })
            .catch((reason) => {
              if (cancelled) return
              setLoginError(
                `暂时无法连接 OpenAI，本应用将自动重试：${errorMessage(reason)}`
              )
              schedule()
            })
            .finally(() => end("poll"))
        },
        Math.max(1, authorization.intervalSecs) * 1000
      )
    }

    schedule()
    return () => {
      cancelled = true
      if (timer !== undefined) window.clearTimeout(timer)
    }
  }, [authorization, begin, end, finishLoginPoll, loginMode, loginOpen])

  const openAccountLogin = (mode: AccountLoginMode) => {
    if (busy) return
    if (mode !== "browser") setAuthorization(undefined)
    setLoginMode(mode)
    setLoginError(undefined)
    setLoginOpen(true)
  }

  const importCookie = async (
    name: string | undefined,
    accountId: string | undefined,
    content: string
  ) => {
    if (!begin("import")) return
    try {
      const imported = await call("connections_import_cookie", {
        name,
        accountId,
        content,
      })
      const firstAccount = imported.accounts[0]
      const formatLabel =
        imported.detectedFormats.join("、") || "Cookie 登录数据"
      toast.add({
        title:
          imported.accounts.length > 1
            ? `已导入 ${imported.accounts.length} 个 Cookie 账号`
            : "Cookie 登录数据已导入",
        description: `${formatLabel}${firstAccount ? ` · ${firstAccount.name}` : ""}`,
        type: "success",
      })
      setAuthorization(undefined)
      setLoginOpen(false)
      if (firstAccount) onSelectedIdChange(firstAccount.id)
      onRefresh()
    } catch (reason) {
      setLoginError(errorMessage(reason))
      toast.add({
        title: "Cookie 登录数据导入失败",
        description: errorMessage(reason),
        type: "error",
      })
    } finally {
      end("import")
    }
  }

  const connectionEditors = (
    <>
      <ProviderEditorDialog
        open={editorOpen}
        onOpenChange={setEditorOpen}
        provider={editor}
        onProviderChange={setEditor}
        onSaved={(saved) => {
          setEditor(saved)
          setEditorOpen(false)
          onRefresh()
        }}
      />
      <AccountLoginDialog
        key={loginOpen ? "login-open" : "login-closed"}
        open={loginOpen}
        mode={loginMode}
        onModeChange={(mode) => {
          if (mode !== "browser") setAuthorization(undefined)
          setLoginMode(mode)
        }}
        onOpenChange={(open) => {
          if (!open && busy) return
          setLoginOpen(open)
          if (!open && !authorization) setLoginError(undefined)
        }}
        authorization={authorization}
        starting={busy === "login"}
        polling={busy === "poll"}
        importing={busy === "import"}
        error={loginError}
        onStart={() => void startLogin()}
        onOpenPage={() =>
          void call("connections_open_login_page").catch((reason) =>
            toast.add({
              title: "无法打开登录页面",
              description: errorMessage(reason),
              type: "error",
            })
          )
        }
        onCheck={() => void checkLogin()}
        onImport={(name, accountId, content) =>
          void importCookie(name, accountId, content)
        }
      />
    </>
  )

  if (!connections) {
    return <ProvidersLoading />
  }

  if (!selected) {
    return (
      <div className="min-h-full px-3 pt-1 pb-3">
        <Card size="sm" className="min-h-full justify-center">
          <Empty>
            <EmptyHeader>
              <EmptyMedia variant="icon">
                <HugeiconsIcon icon={Key01Icon} />
              </EmptyMedia>
              <EmptyTitle>还没有连接</EmptyTitle>
              <EmptyDescription>
                登录 OpenAI 账号，或连接兼容 Responses API 的服务。
              </EmptyDescription>
            </EmptyHeader>
            <div className="grid w-full grid-cols-3 gap-2">
              <Button
                type="button"
                variant="outline"
                className="min-w-0 px-2 text-xs"
                disabled={Boolean(busy)}
                onClick={() => openAccountLogin("browser")}
              >
                <HugeiconsIcon icon={Login03Icon} data-icon="inline-start" />
                OpenAI 授权
              </Button>
              <Button
                type="button"
                variant="outline"
                className="min-w-0 px-2 text-xs"
                disabled={Boolean(busy)}
                onClick={() => openAccountLogin("cookie")}
              >
                <HugeiconsIcon icon={Key01Icon} data-icon="inline-start" />
                导入 Cookie
              </Button>
              <Button
                type="button"
                variant="outline"
                className="min-w-0 px-2 text-xs"
                disabled={Boolean(busy)}
                onClick={() => {
                  setEditor(emptyProvider())
                  setEditorOpen(true)
                }}
              >
                <HugeiconsIcon icon={Add01Icon} data-icon="inline-start" />
                添加 API 服务
              </Button>
            </div>
          </Empty>
        </Card>
        {connectionEditors}
      </div>
    )
  }

  const isAccount = selected.kind === "account"
  const item = selected.value
  const account = isAccount ? (item as OfficialAccountView) : undefined
  const provider = !isAccount ? (item as Provider) : undefined
  const quota = quotaWindow(account?.quota)
  const displayName = account?.remark || item.name
  const actionBusy = Boolean(busy)

  return (
    <div className="flex min-h-full flex-col gap-3 px-3 pt-1 pb-3">
      <Card size="sm" className="shrink-0">
        <CardContent className="flex flex-col gap-2">
          <div className="flex flex-col gap-0.5">
            <div className="text-sm font-medium">添加连接</div>
            <div className="text-xs text-muted-foreground">
              使用官方授权或 Cookie 登录 OpenAI，也可以连接兼容 API 服务。
            </div>
          </div>
          <div className="grid w-full grid-cols-3 gap-2">
            <Button
              type="button"
              size="sm"
              variant="outline"
              className="min-w-0 px-2 text-xs"
              disabled={actionBusy}
              onClick={() => openAccountLogin("browser")}
            >
              <HugeiconsIcon icon={Login03Icon} data-icon="inline-start" />
              OpenAI 授权
            </Button>
            <Button
              type="button"
              size="sm"
              variant="outline"
              className="min-w-0 px-2 text-xs"
              disabled={actionBusy}
              onClick={() => openAccountLogin("cookie")}
            >
              <HugeiconsIcon icon={Key01Icon} data-icon="inline-start" />
              导入 Cookie
            </Button>
            <Button
              type="button"
              size="sm"
              variant="outline"
              className="min-w-0 px-2 text-xs"
              disabled={actionBusy}
              onClick={() => {
                setEditor(emptyProvider())
                setEditorOpen(true)
              }}
            >
              <HugeiconsIcon icon={Add01Icon} data-icon="inline-start" />
              添加 API 服务
            </Button>
          </div>
        </CardContent>
      </Card>

      <Card key={item.id} size="sm" className="shrink-0">
        <CardHeader className="border-b">
          <div className="flex items-center gap-2">
            <CardTitle>{displayName}</CardTitle>
            {item.active && (
              <Badge>
                <HugeiconsIcon icon={CheckmarkCircle02Icon} />
                当前连接
              </Badge>
            )}
            <Badge variant="outline">
              {isAccount ? "OpenAI 账号" : "API 服务"}
            </Badge>
          </div>
          <div className="text-sm text-muted-foreground">
            {account?.email || provider?.baseUrl}
          </div>
        </CardHeader>
        <CardContent className="grid grid-cols-2 gap-x-6 gap-y-4">
          {isAccount && (
            <Detail label="账号类型" value={accountPlanText(account!)} />
          )}
          <Detail
            label="接入方式"
            value={
              isAccount
                ? account?.source === "proxy_import"
                  ? "Cookie 登录数据"
                  : "OpenAI 官方授权"
                : provider?.apiType === "chat"
                  ? "Chat Completions"
                  : "Responses API"
            }
          />
          {isAccount && (
            <Detail label="账号备注" value={account?.remark || "未设置"} />
          )}
          {isAccount && (
            <Detail label="自动维护" value={credentialRefreshText(account!)} />
          )}
          {isAccount && (
            <Detail
              label="最近刷新尝试"
              value={formatDate(account?.credentialRefresh.lastAttemptAt, true)}
            />
          )}
          {isAccount && (
            <Detail
              label="最近刷新"
              value={formatDate(account?.credentialRefresh.lastRefreshAt, true)}
            />
          )}
          {isAccount && (
            <Detail
              label="最近同步"
              value={formatDate(account?.credentialRefresh.lastSyncAt, true)}
            />
          )}
          {isAccount && (
            <Detail
              label="最近检查"
              value={formatDate(account?.credentialRefresh.lastCheckAt, true)}
            />
          )}
          {isAccount && account?.credentialRefresh.nextRetryAt && (
            <Detail
              label="下次重试"
              value={formatDate(account.credentialRefresh.nextRetryAt, true)}
            />
          )}
          <Detail
            label={isAccount ? "登录状态" : "凭据状态"}
            value={
              isAccount
                ? credentialLabel(account)
                : provider?.hasApiKey
                  ? "API Key 已保存"
                  : "等待填写 API Key"
            }
          />
          <Detail
            label={isAccount ? "账号资料更新" : "最近更新"}
            value={formatDate(item.updatedAt, true)}
          />
          {isAccount && (
            <Detail
              label="额度状态"
              value={quotaLabel(account, quota?.remainingPercent)}
            />
          )}
          {isAccount && (
            <Detail
              label="额度重置时间"
              value={formatDate(quota?.resetAt, true)}
            />
          )}
          {!isAccount && (
            <Detail
              label="可用模型"
              value={`${provider ? effectiveModelCount(provider) : 0} 个`}
            />
          )}
        </CardContent>
        <CardFooter className="flex-wrap gap-2">
          {isAccount ? (
            <>
              <Button
                disabled={actionBusy}
                aria-busy={busy === "quota"}
                onClick={() =>
                  void run("quota", () => refreshAccountQuota(item.id), {
                    success: "额度已刷新",
                    onSuccess: onRefresh,
                  })
                }
              >
                {busy === "quota" ? (
                  <Spinner data-icon="inline-start" />
                ) : (
                  <HugeiconsIcon
                    icon={Refresh01Icon}
                    data-icon="inline-start"
                  />
                )}
                刷新额度
              </Button>
              <Button
                variant="outline"
                disabled={actionBusy}
                aria-busy={busy === "login-refresh"}
                onClick={() =>
                  void run(
                    "login-refresh",
                    () => call("connections_refresh_login", { id: item.id }),
                    {
                      success: "登录维护已完成",
                      successDescription: credentialMaintenanceMessage,
                      onSuccess: onRefresh,
                    }
                  )
                }
              >
                {busy === "login-refresh" ? (
                  <Spinner data-icon="inline-start" />
                ) : (
                  <HugeiconsIcon icon={Login03Icon} data-icon="inline-start" />
                )}
                立即刷新登录
              </Button>
            </>
          ) : (
            <>
              <Button
                disabled={actionBusy}
                aria-busy={busy === "test"}
                onClick={() =>
                  void run("test", () => testProviderConnection(item.id), {
                    success: "连接测试通过",
                    onSuccess: onRefresh,
                  })
                }
              >
                {busy === "test" ? (
                  <Spinner data-icon="inline-start" />
                ) : (
                  <HugeiconsIcon
                    icon={TestTube01Icon}
                    data-icon="inline-start"
                  />
                )}
                测试连接
              </Button>
              <Button
                variant="outline"
                disabled={actionBusy}
                aria-busy={busy === "models"}
                onClick={() =>
                  void run(
                    "models",
                    () => call("connections_list_models", { id: item.id }),
                    { success: "模型已同步", onSuccess: onRefresh }
                  )
                }
              >
                {busy === "models" ? (
                  <Spinner data-icon="inline-start" />
                ) : (
                  <HugeiconsIcon
                    icon={Refresh01Icon}
                    data-icon="inline-start"
                  />
                )}
                同步模型
              </Button>
            </>
          )}
        </CardFooter>
      </Card>

      {connectionEditors}
    </div>
  )
}

function ProvidersLoading() {
  return (
    <div
      className="flex min-h-full flex-col gap-3 px-3 pt-1 pb-3"
      aria-label="正在读取账号与服务"
      aria-busy="true"
    >
      <Card size="sm" className="shrink-0">
        <CardContent className="flex items-center gap-3">
          <div className="flex flex-1 flex-col gap-2">
            <Skeleton className="h-4 w-24" />
            <Skeleton className="h-3 w-52" />
          </div>
          <Skeleton className="h-8 w-24" />
          <Skeleton className="h-8 w-28" />
        </CardContent>
      </Card>
      <Card size="sm" className="shrink-0">
        <CardHeader className="border-b">
          <Skeleton className="h-5 w-36" />
          <Skeleton className="h-3.5 w-56" />
        </CardHeader>
        <CardContent className="grid grid-cols-2 gap-x-6 gap-y-4">
          {Array.from({ length: 6 }, (_, index) => (
            <div key={index} className="flex flex-col gap-1.5">
              <Skeleton className="h-3 w-16" />
              <Skeleton className="h-4 w-28" />
            </div>
          ))}
        </CardContent>
        <CardFooter className="gap-2">
          <Skeleton className="h-8 w-24" />
          <Skeleton className="h-8 w-24" />
        </CardFooter>
      </Card>
    </div>
  )
}

function quotaLabel(
  account: OfficialAccountView | undefined,
  remainingPercent: number | undefined
) {
  if (!account) return "—"
  return quotaStatusText(account, remainingPercent)
}

function credentialLabel(account: OfficialAccountView | undefined) {
  if (!account) return "—"
  return loginVerificationText(account)
}

function Detail({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex flex-col gap-0.5">
      <span className="text-xs text-muted-foreground">{label}</span>
      <span className="truncate font-medium">{value}</span>
    </div>
  )
}
