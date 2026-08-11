import { useCallback, useEffect, useMemo, useRef, useState } from "react"
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
import { call } from "@/lib/ipc"
import type {
  DeviceAuthorization,
  OfficialAccountView,
  Provider,
  ProviderOverview,
} from "@/types"
import { emptyProvider } from "@/types"

import {
  AccountLoginDialog,
  type AccountLoginMode,
} from "./account-login-dialog"
import {
  refreshAccountQuota,
  testProviderConnection,
} from "./connection-actions"
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
  const [busy, setBusy] = useState<string>()
  const busyRef = useRef<string | undefined>(undefined)

  const beginBusy = useCallback((key: string) => {
    if (busyRef.current) return false
    busyRef.current = key
    setBusy(key)
    return true
  }, [])

  const endBusy = useCallback((key: string) => {
    if (busyRef.current !== key) return
    busyRef.current = undefined
    setBusy(undefined)
  }, [])

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

  const run = async (
    key: string,
    action: () => Promise<unknown>,
    success: string
  ) => {
    if (!beginBusy(key)) return false
    try {
      await action()
      toast.add({ title: success, type: "success" })
      onRefresh()
      return true
    } catch (reason) {
      toast.add({
        title: "操作失败",
        description: errorMessage(reason),
        type: "error",
      })
      return false
    } finally {
      endBusy(key)
    }
  }

  const startLogin = async () => {
    if (!beginBusy("login")) return
    setLoginError(undefined)
    try {
      setAuthorization(await call("connections_login_start"))
    } catch (reason) {
      setLoginError(errorMessage(reason))
      toast.add({
        title: "无法开始登录",
        description: errorMessage(reason),
        type: "error",
      })
    } finally {
      endBusy("login")
    }
  }

  const finishLoginPoll = useCallback(
    (result: Awaited<ReturnType<typeof call<"connections_login_poll">>>) => {
      if (result.status === "complete") {
        toast.add({ title: "OpenAI 登录成功", type: "success" })
        setAuthorization(undefined)
        setLoginOpen(false)
        onSelectedIdChange(result.account.id)
        onRefresh()
        return true
      }
      if (result.status === "expired") {
        setAuthorization(undefined)
        setLoginError("登录码已过期，请重新生成后继续。")
        return true
      }
      return false
    },
    [onRefresh, onSelectedIdChange]
  )

  const checkLogin = async () => {
    if (!authorization) return
    if (!beginBusy("poll")) return
    try {
      const result = await call("connections_login_poll", {
        operationId: authorization.operationId,
      })
      if (!finishLoginPoll(result)) {
        toast.add({
          title: "仍在等待授权",
        })
      }
    } catch (reason) {
      toast.add({
        title: "登录状态检查失败",
        description: errorMessage(reason),
        type: "error",
      })
    } finally {
      endBusy("poll")
    }
  }

  useEffect(() => {
    if (!authorization || !loginOpen) return
    let cancelled = false
    let timer: number | undefined

    const schedule = () => {
      timer = window.setTimeout(
        () => {
          if (!beginBusy("poll")) {
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
                `暂时无法确认登录结果，程序会自动重试：${errorMessage(reason)}`
              )
              schedule()
            })
            .finally(() => endBusy("poll"))
        },
        Math.max(1, authorization.intervalSecs) * 1000
      )
    }

    schedule()
    return () => {
      cancelled = true
      if (timer !== undefined) window.clearTimeout(timer)
    }
  }, [authorization, beginBusy, endBusy, finishLoginPoll, loginOpen])

  const openAccountLogin = (mode: AccountLoginMode) => {
    if (busyRef.current) return
    setLoginMode(mode)
    setLoginError(undefined)
    setLoginOpen(true)
  }

  const importCookie = async (
    name: string | undefined,
    accountId: string | undefined,
    content: string
  ) => {
    if (!beginBusy("import")) return
    try {
      const imported = await call("connections_import_cookie", {
        name,
        accountId,
        content,
      })
      toast.add({
        title: "Cookie 账号已导入",
        description: imported.name,
        type: "success",
      })
      setAuthorization(undefined)
      setLoginOpen(false)
      onSelectedIdChange(imported.id)
      onRefresh()
    } catch (reason) {
      setLoginError(errorMessage(reason))
      toast.add({
        title: "Cookie 登录失败",
        description: errorMessage(reason),
        type: "error",
      })
    } finally {
      endBusy("import")
    }
  }

  const connectionEditors = (
    <>
      <ProviderEditorDialog
        open={editorOpen}
        onOpenChange={setEditorOpen}
        provider={editor}
        onProviderChange={setEditor}
        onSaved={() => {
          setEditorOpen(false)
          onRefresh()
        }}
      />
      <AccountLoginDialog
        open={loginOpen}
        mode={loginMode}
        onModeChange={setLoginMode}
        onOpenChange={(open) => {
          if (!open && busyRef.current) return
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
                添加 OpenAI 账号或自定义 API 服务。
              </EmptyDescription>
            </EmptyHeader>
            <div className="flex gap-2">
              <Button
                type="button"
                disabled={Boolean(busy)}
                onClick={() => openAccountLogin("browser")}
              >
                <HugeiconsIcon icon={Login03Icon} data-icon="inline-start" />
                添加账号
              </Button>
              <Button
                type="button"
                variant="outline"
                disabled={Boolean(busy)}
                onClick={() => openAccountLogin("cookie")}
              >
                <HugeiconsIcon icon={Key01Icon} data-icon="inline-start" />
                Cookie 登录
              </Button>
              <Button
                type="button"
                variant="outline"
                disabled={Boolean(busy)}
                onClick={() => {
                  setEditor(emptyProvider())
                  setEditorOpen(true)
                }}
              >
                <HugeiconsIcon icon={Add01Icon} data-icon="inline-start" />
                添加 API
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
        <CardContent className="flex flex-wrap items-center gap-2">
          <div className="mr-auto flex min-w-40 flex-col gap-0.5">
            <div className="text-sm font-medium">添加连接</div>
            <div className="text-xs text-muted-foreground">
              登录 OpenAI 账号，或添加兼容 API 服务。
            </div>
          </div>
          <Button
            type="button"
            size="sm"
            disabled={actionBusy}
            onClick={() => openAccountLogin("browser")}
          >
            <HugeiconsIcon icon={Login03Icon} data-icon="inline-start" />
            添加账号
          </Button>
          <Button
            type="button"
            size="sm"
            variant="outline"
            disabled={actionBusy}
            onClick={() => {
              setEditor(emptyProvider())
              setEditorOpen(true)
            }}
          >
            <HugeiconsIcon icon={Add01Icon} data-icon="inline-start" />
            添加 API 服务
          </Button>
          <Button
            type="button"
            size="sm"
            variant="ghost"
            disabled={actionBusy}
            onClick={() => openAccountLogin("cookie")}
          >
            Cookie 登录
          </Button>
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
          <Detail
            label="接入方式"
            value={
              isAccount
                ? "OpenAI OAuth"
                : provider?.apiType === "chat"
                  ? "Chat Completions"
                  : "Responses API"
            }
          />
          {isAccount && (
            <Detail label="账号备注" value={account?.remark || "未设置"} />
          )}
          <Detail
            label="凭据状态"
            value={
              isAccount
                ? credentialLabel(account)
                : provider?.hasApiKey
                  ? "API Key 已保存"
                  : "等待填写 API Key"
            }
          />
          <Detail label="最近更新" value={formatDate(item.updatedAt, true)} />
          {isAccount && (
            <Detail
              label="额度状态"
              value={quotaLabel(account, quota?.remainingPercent)}
            />
          )}
          {!isAccount && (
            <Detail
              label="可用模型"
              value={`${provider?.availableModels?.length ?? 0} 个`}
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
                  void run(
                    "quota",
                    () => refreshAccountQuota(item.id),
                    "额度已刷新"
                  )
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
                    "登录状态已刷新"
                  )
                }
              >
                {busy === "login-refresh" ? (
                  <Spinner data-icon="inline-start" />
                ) : (
                  <HugeiconsIcon icon={Login03Icon} data-icon="inline-start" />
                )}
                刷新登录
              </Button>
            </>
          ) : (
            <>
              <Button
                disabled={actionBusy}
                aria-busy={busy === "test"}
                onClick={() =>
                  void run(
                    "test",
                    () => testProviderConnection(item.id),
                    "连接测试通过"
                  )
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
                    "模型已同步"
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
  if (remainingPercent !== undefined) {
    const refreshed = account.quota.fetchedAt
      ? ` · ${formatDate(account.quota.fetchedAt, true)}`
      : ""
    return `剩余 ${remainingPercent.toFixed(1)}%${refreshed}`
  }
  switch (account.quota.status) {
    case "never":
      return "尚未刷新"
    case "unauthorized":
      return "登录已失效，请刷新登录"
    case "rate_limited":
      return "请求频繁，请稍后重试"
    case "unsupported":
      return "当前账号暂不支持额度查询"
    case "error":
      return account.quota.error || "额度刷新失败"
    default:
      return "暂无额度数据"
  }
}

function credentialLabel(account: OfficialAccountView | undefined) {
  if (!account) return "—"
  if (account.quota.status === "unauthorized") return "登录已失效"
  if (
    account.expiresAt != null &&
    account.expiresAt <= Math.floor(Date.now() / 1000)
  ) {
    return "登录已过期"
  }
  return account.source === "proxy_import"
    ? "Cookie 登录有效"
    : "OAuth 登录有效"
}

function Detail({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex flex-col gap-0.5">
      <span className="text-xs text-muted-foreground">{label}</span>
      <span className="truncate font-medium">{value}</span>
    </div>
  )
}
