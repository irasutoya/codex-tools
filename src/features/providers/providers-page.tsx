import { useCallback, useEffect, useMemo, useState } from "react"
import {
  Add01Icon,
  CheckmarkCircle02Icon,
  Delete02Icon,
  Edit02Icon,
  Key01Icon,
  Login03Icon,
  Refresh01Icon,
  TestTube01Icon,
} from "@hugeicons/core-free-icons"
import { HugeiconsIcon } from "@hugeicons/react"

import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog"
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
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import {
  Empty,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from "@/components/ui/empty"
import {
  Field,
  FieldDescription,
  FieldGroup,
  FieldLabel,
} from "@/components/ui/field"
import { Input } from "@/components/ui/input"
import {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { Spinner } from "@/components/ui/spinner"
import { Switch } from "@/components/ui/switch"
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
  const [deleteOpen, setDeleteOpen] = useState(false)

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
      connections?.officialAccounts[0] ?? connections?.providers[0]
    if (!fallback) return undefined
    return "email" in fallback
      ? { kind: "account" as const, value: fallback }
      : { kind: "provider" as const, value: fallback }
  }, [connections, selectedId])

  useEffect(() => {
    if (!selectedId && selected) onSelectedIdChange(selected.value.id)
  }, [onSelectedIdChange, selected, selectedId])

  const run = async (
    key: string,
    action: () => Promise<unknown>,
    success: string
  ) => {
    setBusy(key)
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
      setBusy(undefined)
    }
  }

  const startLogin = async () => {
    setBusy("login")
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
      setBusy(undefined)
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
    setBusy("poll")
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
      setBusy(undefined)
    }
  }

  useEffect(() => {
    if (!authorization || !loginOpen) return
    let cancelled = false
    let timer: number | undefined

    const schedule = () => {
      timer = window.setTimeout(
        () => {
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
        },
        Math.max(1, authorization.intervalSecs) * 1000
      )
    }

    schedule()
    return () => {
      cancelled = true
      if (timer !== undefined) window.clearTimeout(timer)
    }
  }, [authorization, finishLoginPoll, loginOpen])

  const openAccountLogin = (mode: AccountLoginMode) => {
    setLoginMode(mode)
    setLoginError(undefined)
    setLoginOpen(true)
  }

  const importCookie = async (
    name: string | undefined,
    accountId: string | undefined,
    content: string
  ) => {
    setBusy("import")
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
      setBusy(undefined)
    }
  }

  const connectionEditors = (
    <>
      <ProviderEditor
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
              <Button type="button" onClick={() => openAccountLogin("browser")}>
                <HugeiconsIcon icon={Login03Icon} data-icon="inline-start" />
                添加账号
              </Button>
              <Button
                type="button"
                variant="outline"
                onClick={() => openAccountLogin("cookie")}
              >
                <HugeiconsIcon icon={Key01Icon} data-icon="inline-start" />
                Cookie 登录
              </Button>
              <Button
                type="button"
                variant="outline"
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

  return (
    <div className="flex min-h-full flex-col gap-3 px-3 pt-1 pb-3">
      <Card size="sm" className="shrink-0">
        <CardHeader className="border-b">
          <div className="flex items-center gap-2">
            <CardTitle>{item.name}</CardTitle>
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
          <Detail label="默认模型" value={provider?.model || "由 Codex 管理"} />
          <Detail
            label="凭据状态"
            value={
              isAccount
                ? account?.source === "proxy_import"
                  ? "Cookie 导入"
                  : "授权有效"
                : provider?.hasApiKey
                  ? "API Key 已保存"
                  : "等待填写 API Key"
            }
          />
          <Detail label="最近更新" value={formatDate(item.updatedAt, true)} />
          {isAccount && (
            <Detail
              label="额度状态"
              value={
                quota
                  ? `剩余 ${quota.remainingPercent.toFixed(1)}%`
                  : account?.quota.status === "never"
                    ? "尚未刷新"
                    : "暂不支持"
              }
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
                disabled={busy === "quota"}
                onClick={() =>
                  void run(
                    "quota",
                    () =>
                      call("connections_refresh_quota", { accountId: item.id }),
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
              {!item.active && (
                <Button
                  variant="outline"
                  onClick={() =>
                    void run(
                      "activate",
                      () =>
                        call("connections_activate_account", { id: item.id }),
                      "账号已切换"
                    )
                  }
                >
                  设为当前
                </Button>
              )}
            </>
          ) : (
            <>
              <Button
                disabled={busy === "test"}
                onClick={() =>
                  void run(
                    "test",
                    () => call("connections_test_provider", { id: item.id }),
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
                onClick={() =>
                  void run(
                    "models",
                    () => call("connections_list_models", { id: item.id }),
                    "模型已同步"
                  )
                }
              >
                同步模型
              </Button>
              <Button
                variant="outline"
                onClick={() => {
                  setEditor({ ...provider! })
                  setEditorOpen(true)
                }}
              >
                <HugeiconsIcon icon={Edit02Icon} data-icon="inline-start" />
                编辑
              </Button>
              {!item.active && (
                <Button
                  variant="outline"
                  onClick={() =>
                    void run(
                      "activate",
                      () => call("connections_activate", { id: item.id }),
                      "服务已切换"
                    )
                  }
                >
                  设为当前
                </Button>
              )}
            </>
          )}
          <Button
            variant="destructive"
            className="ml-auto"
            onClick={() => setDeleteOpen(true)}
          >
            <HugeiconsIcon icon={Delete02Icon} data-icon="inline-start" />
            删除
          </Button>
        </CardFooter>
      </Card>

      <Card size="sm" className="shrink-0">
        <CardContent className="flex items-center gap-2">
          <Button
            type="button"
            size="sm"
            onClick={() => openAccountLogin("browser")}
          >
            <HugeiconsIcon icon={Login03Icon} data-icon="inline-start" />
            添加账号
          </Button>
          <Button
            type="button"
            size="sm"
            variant="outline"
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
            onClick={() => openAccountLogin("cookie")}
          >
            Cookie 登录
          </Button>
        </CardContent>
      </Card>

      {connectionEditors}

      <AlertDialog open={deleteOpen} onOpenChange={setDeleteOpen}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>删除“{item.name}”？</AlertDialogTitle>
            <AlertDialogDescription>
              此操作会移除本地保存的连接信息，无法撤销。
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>取消</AlertDialogCancel>
            <AlertDialogAction
              variant="destructive"
              disabled={busy === "delete"}
              onClick={() =>
                void run(
                  "delete",
                  () =>
                    isAccount
                      ? call("connections_delete_account", { id: item.id })
                      : call("connections_delete_provider", { id: item.id }),
                  "连接已删除"
                ).then((deleted) => deleted && setDeleteOpen(false))
              }
            >
              {busy === "delete" && <Spinner data-icon="inline-start" />}
              删除
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  )
}

function Detail({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex flex-col gap-0.5">
      <span className="text-xs text-muted-foreground">{label}</span>
      <span className="truncate font-medium">{value}</span>
    </div>
  )
}

function ProviderEditor({
  open,
  onOpenChange,
  provider,
  onProviderChange,
  onSaved,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  provider: Provider
  onProviderChange: (provider: Provider) => void
  onSaved: () => void
}) {
  const [saving, setSaving] = useState(false)
  const update = <K extends keyof Provider>(key: K, value: Provider[K]) =>
    onProviderChange({ ...provider, [key]: value })
  const save = async () => {
    setSaving(true)
    try {
      await call("connections_save_provider", { provider })
      toast.add({ title: "API 服务已保存", type: "success" })
      onSaved()
    } catch (reason) {
      toast.add({
        title: "保存失败",
        description: errorMessage(reason),
        type: "error",
      })
    } finally {
      setSaving(false)
    }
  }
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>
            {provider.id ? "编辑 API 服务" : "添加 API 服务"}
          </DialogTitle>
          <DialogDescription>填写与 OpenAI 兼容的接口信息。</DialogDescription>
        </DialogHeader>
        <FieldGroup>
          <Field>
            <FieldLabel htmlFor="provider-name">名称</FieldLabel>
            <Input
              id="provider-name"
              value={provider.name}
              onChange={(e) => update("name", e.target.value)}
            />
          </Field>
          <Field>
            <FieldLabel htmlFor="provider-url">Base URL</FieldLabel>
            <Input
              id="provider-url"
              value={provider.baseUrl}
              onChange={(e) => update("baseUrl", e.target.value)}
            />
          </Field>
          <Field>
            <FieldLabel htmlFor="provider-key">API Key</FieldLabel>
            <Input
              id="provider-key"
              type="password"
              value={provider.apiKey ?? ""}
              placeholder={provider.hasApiKey ? "留空以保留现有密钥" : "sk-..."}
              onChange={(e) => update("apiKey", e.target.value)}
            />
          </Field>
          <Field>
            <FieldLabel>接口类型</FieldLabel>
            <Select
              items={[
                { label: "Responses API", value: "responses" },
                { label: "Chat Completions", value: "chat" },
              ]}
              value={provider.apiType}
              onValueChange={(value) =>
                value && update("apiType", value as Provider["apiType"])
              }
            >
              <SelectTrigger className="w-full">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectGroup>
                  <SelectItem value="responses">Responses API</SelectItem>
                  <SelectItem value="chat">Chat Completions</SelectItem>
                </SelectGroup>
              </SelectContent>
            </Select>
          </Field>
          <Field>
            <FieldLabel htmlFor="provider-model">默认模型</FieldLabel>
            <Input
              id="provider-model"
              value={provider.model ?? ""}
              onChange={(e) => update("model", e.target.value)}
            />
          </Field>
          <Field orientation="horizontal">
            <div>
              <FieldLabel htmlFor="provider-enabled">启用服务</FieldLabel>
              <FieldDescription>关闭后不会出现在可切换列表。</FieldDescription>
            </div>
            <Switch
              id="provider-enabled"
              checked={provider.enabled}
              onCheckedChange={(checked) => update("enabled", checked)}
            />
          </Field>
        </FieldGroup>
        <DialogFooter>
          <Button
            type="button"
            variant="outline"
            onClick={() => onOpenChange(false)}
          >
            取消
          </Button>
          <Button
            type="button"
            disabled={saving || !provider.name || !provider.baseUrl}
            onClick={() => void save()}
          >
            {saving && <Spinner data-icon="inline-start" />}保存
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
