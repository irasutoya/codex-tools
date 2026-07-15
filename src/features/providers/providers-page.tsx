import { useCallback, useEffect, useMemo, useRef, useState } from "react"
import {
  Boxes,
  Copy,
  ExternalLink,
  KeyRound,
  LogIn,
  Pencil,
  Plus,
  RefreshCw,
  Save,
  Trash2,
  UserPlus,
  UserRound,
  Zap,
} from "lucide-react"
import { toast } from "sonner"

import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { Badge } from "@/components/ui/badge"
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
import { Button } from "@/components/ui/button"
import {
  Card,
  CardAction,
  CardContent,
  CardDescription,
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
  EmptyContent,
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
  Item,
  ItemActions,
  ItemContent,
  ItemDescription,
  ItemGroup,
  ItemMedia,
  ItemTitle,
} from "@/components/ui/item"
import { Spinner } from "@/components/ui/spinner"
import { Switch } from "@/components/ui/switch"
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group"
import { call } from "@/lib/ipc"
import {
  emptyAccount,
  emptyProvider,
  type Account,
  type DeviceAuthorization,
  type DeviceAuthPollResult,
  type FetchedModel,
  type OfficialAccountView,
  type Provider,
  type ProviderOverview,
} from "@/types"

export default function ProvidersPage() {
  const [providers, setProviders] = useState<Provider[]>([])
  const [accounts, setAccounts] = useState<Account[]>([])
  const [officialAccounts, setOfficialAccounts] = useState<
    OfficialAccountView[]
  >([])
  const [deviceAuthorization, setDeviceAuthorization] =
    useState<DeviceAuthorization>()
  const [draft, setDraft] = useState<Provider>()
  const [account, setAccount] = useState<Account>()
  const [pendingActivation, setPendingActivation] = useState<{
    providerId: string
    accountId: string
  }>()
  const [pendingOfficialAction, setPendingOfficialAction] = useState<{
    kind: "activate" | "delete"
    account: OfficialAccountView
  }>()
  const [confirmOpenAiLogin, setConfirmOpenAiLogin] = useState(false)
  const [pendingTask, setPendingTask] = useState<string>()
  const [pendingDelete, setPendingDelete] = useState<
    | { kind: "provider"; id: string; name: string }
    | { kind: "account"; id: string; name: string }
  >()
  const running = useRef(false)
  const busy = Boolean(pendingTask)
  const hasActiveProvider = providers.some((provider) => provider.active)
  const accountsByProvider = useMemo(() => {
    const grouped = new Map<string, Account[]>()
    for (const item of accounts) {
      if (!item.providerId) continue
      const group = grouped.get(item.providerId) ?? []
      group.push(item)
      grouped.set(item.providerId, group)
    }
    return grouped
  }, [accounts])
  const load = useCallback(async () => {
    const overview = await call<ProviderOverview>("get_provider_overview")
    setProviders(overview.providers)
    setAccounts(overview.accounts)
    setOfficialAccounts(overview.officialAccounts)
  }, [])
  useEffect(() => {
    const timeout = window.setTimeout(() => {
      void load().catch((error) => toast.error(String(error)))
    }, 0)
    return () => window.clearTimeout(timeout)
  }, [load])

  useEffect(() => {
    if (!deviceAuthorization) return
    const authorization = deviceAuthorization
    let cancelled = false
    let timer: number | undefined
    let pollErrorShown = false

    const scheduleNextPoll = () => {
      const remainingMs =
        timestampMilliseconds(authorization.expiresAt) - Date.now()
      if (remainingMs <= 0) {
        setDeviceAuthorization(undefined)
        toast.error("OpenAI 登录码已过期，请重新登录")
        return
      }
      timer = window.setTimeout(
        () => {
          void call<DeviceAuthPollResult>("poll_openai_device_auth", {
            operationId: authorization.operationId,
          })
            .then((result) => {
              if (cancelled) return
              pollErrorShown = false
              if (result.status === "pending") {
                scheduleNextPoll()
                return
              }
              setDeviceAuthorization(undefined)
              if (result.status === "expired") {
                toast.error("OpenAI 登录码已过期，请重新登录")
                return
              }
              toast.success(`已登录并激活 OpenAI 账号：${result.account.name}`)
              if (result.repair.warnings.length) {
                toast.warning(
                  `会话迁移完成，但有 ${result.repair.warnings.length} 条警告`
                )
              }
              void load().catch((error) => toast.error(String(error)))
            })
            .catch((error) => {
              if (cancelled) return
              if (!pollErrorShown) {
                pollErrorShown = true
                toast.error(`暂时无法检查登录状态：${String(error)}`)
              }
              scheduleNextPoll()
            })
        },
        Math.min(authorization.intervalSecs * 1000, remainingMs)
      )
    }

    scheduleNextPoll()
    return () => {
      cancelled = true
      if (timer !== undefined) window.clearTimeout(timer)
    }
  }, [deviceAuthorization, load])

  const run = async (task: string, action: () => Promise<unknown>) => {
    if (running.current) return
    running.current = true
    setPendingTask(task)
    try {
      await action()
      await load()
    } catch (error) {
      toast.error(String(error))
    } finally {
      running.current = false
      setPendingTask(undefined)
    }
  }

  return (
    <div className="flex flex-col gap-6">
      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2">
            OpenAI 账号
            {officialAccounts.some((item) => item.active) && (
              <Badge>官方模式</Badge>
            )}
          </CardTitle>
          <CardDescription>
            使用与 Codex CLI 兼容的设备授权登录。登录完成后会写入
            auth.json、清空整个 config.toml 并自动切换到该账号。
          </CardDescription>
          <CardAction>
            <Button
              disabled={busy || Boolean(deviceAuthorization)}
              onClick={() => setConfirmOpenAiLogin(true)}
            >
              <LogIn data-icon="inline-start" />
              登录 OpenAI
            </Button>
          </CardAction>
        </CardHeader>
        <CardContent className="flex flex-col gap-4">
          {deviceAuthorization && (
            <div className="flex flex-col gap-3">
              <Alert>
                <KeyRound />
                <AlertTitle className="flex flex-wrap items-center gap-2">
                  <span>
                    登录码：
                    <code className="font-mono font-semibold">
                      {deviceAuthorization.userCode}
                    </code>
                  </span>
                  <Badge variant="outline">
                    <Spinner data-icon="inline-start" />
                    等待授权
                  </Badge>
                </AlertTitle>
                <AlertDescription>
                  前往 {deviceAuthorization.verificationUri}{" "}
                  输入登录码。有效期至{" "}
                  {formatTimestamp(deviceAuthorization.expiresAt)}
                  ；本程序正在等待授权。
                </AlertDescription>
              </Alert>
              <div className="flex flex-wrap gap-2">
                <Button
                  size="sm"
                  variant="outline"
                  onClick={() =>
                    void navigator.clipboard
                      .writeText(deviceAuthorization.userCode)
                      .then(() => toast.success("登录码已复制"))
                      .catch((error) => toast.error(String(error)))
                  }
                >
                  <Copy data-icon="inline-start" />
                  复制登录码
                </Button>
                <Button
                  size="sm"
                  variant="outline"
                  onClick={() =>
                    void call("open_openai_device_page").catch((error) =>
                      toast.error(String(error))
                    )
                  }
                >
                  <ExternalLink data-icon="inline-start" />
                  打开登录页面
                </Button>
              </div>
            </div>
          )}
          {!officialAccounts.length ? (
            <Empty>
              <EmptyHeader>
                <EmptyMedia variant="icon">
                  <UserRound />
                </EmptyMedia>
                <EmptyTitle>尚未保存 OpenAI 账号</EmptyTitle>
                <EmptyDescription>
                  登录响应只由 Rust 后端处理，前端不会收到 credential 或 token。
                </EmptyDescription>
              </EmptyHeader>
              <EmptyContent>
                <Button
                  size="sm"
                  disabled={busy || Boolean(deviceAuthorization)}
                  onClick={() => setConfirmOpenAiLogin(true)}
                >
                  <LogIn data-icon="inline-start" />
                  登录 OpenAI
                </Button>
              </EmptyContent>
            </Empty>
          ) : (
            <ItemGroup className="gap-2">
              {officialAccounts.map((item) => (
                <Item
                  key={item.id}
                  variant={item.active ? "muted" : "outline"}
                  size="sm"
                >
                  <ItemMedia variant="icon">
                    <UserRound />
                  </ItemMedia>
                  <ItemContent className="min-w-0">
                    <ItemTitle>
                      {item.name}
                      {item.active && <Badge>当前</Badge>}
                    </ItemTitle>
                    <ItemDescription className="truncate">
                      {item.email || item.accountId}
                    </ItemDescription>
                    <ItemDescription>
                      {item.expiresAt
                        ? `访问令牌到期：${formatTimestamp(item.expiresAt)}`
                        : "访问令牌有效期由 OpenAI 管理"}
                    </ItemDescription>
                  </ItemContent>
                  <ItemActions className="ml-auto">
                    <Button
                      size="sm"
                      disabled={busy || item.active}
                      onClick={() =>
                        setPendingOfficialAction({
                          kind: "activate",
                          account: item,
                        })
                      }
                    >
                      <Zap data-icon="inline-start" />
                      切换
                    </Button>
                    <Button
                      size="icon-sm"
                      variant="ghost"
                      disabled={busy || item.active}
                      aria-label={`删除 OpenAI 账号 ${item.name}`}
                      title="删除账号"
                      onClick={() =>
                        setPendingOfficialAction({
                          kind: "delete",
                          account: item,
                        })
                      }
                    >
                      <Trash2 />
                    </Button>
                  </ItemActions>
                </Item>
              ))}
            </ItemGroup>
          )}
        </CardContent>
      </Card>
      <div className="flex flex-col justify-between gap-3 sm:flex-row sm:items-end">
        <div>
          <h2 className="text-lg font-semibold">上游供应商</h2>
          <p className="text-sm text-muted-foreground">
            切换第三方上游不会改写 Codex 会话。
          </p>
        </div>
        <Button onClick={() => setDraft(emptyProvider())}>
          <Plus data-icon="inline-start" />
          添加供应商
        </Button>
      </div>
      <div className="grid gap-4">
        {!providers.length && (
          <Empty>
            <EmptyHeader>
              <EmptyMedia variant="icon">
                <Boxes />
              </EmptyMedia>
              <EmptyTitle>尚未添加第三方供应商</EmptyTitle>
              <EmptyDescription>
                添加兼容 Responses、Chat Completions 或 Anthropic Messages
                的上游服务。
              </EmptyDescription>
            </EmptyHeader>
            <EmptyContent>
              <Button size="sm" onClick={() => setDraft(emptyProvider())}>
                <Plus data-icon="inline-start" />
                添加供应商
              </Button>
            </EmptyContent>
          </Empty>
        )}
        {providers.map((provider) => {
          const linked = accountsByProvider.get(provider.id) ?? []
          return (
            <Card key={provider.id} size="sm">
              <CardHeader>
                <CardTitle className="flex items-center gap-2">
                  {provider.name}
                  {provider.active && <Badge>当前</Badge>}
                  {!provider.enabled && (
                    <Badge variant="secondary">已停用</Badge>
                  )}
                </CardTitle>
                <CardDescription className="truncate">
                  {protocolLabel(provider.protocol)} · {provider.baseUrl}
                </CardDescription>
                <CardAction className="flex gap-1">
                  <Button
                    size="icon-sm"
                    variant="outline"
                    aria-label={`编辑供应商 ${provider.name}`}
                    title="编辑供应商"
                    onClick={() => setDraft(provider)}
                  >
                    <Pencil />
                  </Button>
                  <Button
                    size="icon-sm"
                    variant="outline"
                    aria-label={`为 ${provider.name} 添加账号`}
                    title="添加账号"
                    onClick={() => setAccount(emptyAccount(provider.id))}
                  >
                    <UserPlus />
                  </Button>
                  <Button
                    size="icon-sm"
                    variant="ghost"
                    disabled={provider.active || busy}
                    aria-label={`删除供应商 ${provider.name}`}
                    title="删除供应商"
                    onClick={() =>
                      setPendingDelete({
                        kind: "provider",
                        id: provider.id,
                        name: provider.name,
                      })
                    }
                  >
                    <Trash2 />
                  </Button>
                </CardAction>
              </CardHeader>
              <CardContent className="flex flex-col gap-3">
                <Item variant="muted" size="sm">
                  <ItemMedia variant="icon">
                    <Boxes />
                  </ItemMedia>
                  <ItemContent className="min-w-0">
                    <ItemTitle>可用模型</ItemTitle>
                    <ItemDescription className="truncate">
                      {provider.models.join(", ") || "尚未配置"}
                    </ItemDescription>
                  </ItemContent>
                </Item>
                {!linked.length ? (
                  <Empty className="py-4">
                    <EmptyHeader>
                      <EmptyTitle>尚未添加 API 账号</EmptyTitle>
                      <EmptyDescription>
                        添加凭据后即可测试连接并激活此供应商。
                      </EmptyDescription>
                    </EmptyHeader>
                    <EmptyContent>
                      <Button
                        size="sm"
                        variant="outline"
                        onClick={() => setAccount(emptyAccount(provider.id))}
                      >
                        <KeyRound data-icon="inline-start" />
                        添加账号
                      </Button>
                    </EmptyContent>
                  </Empty>
                ) : (
                  <ItemGroup className="gap-2">
                    {linked.map((item) => (
                      <Item
                        key={item.id}
                        variant={item.active ? "muted" : "outline"}
                        size="sm"
                      >
                        <ItemMedia variant="icon">
                          <KeyRound />
                        </ItemMedia>
                        <ItemContent>
                          <ItemTitle>
                            {item.name}
                            {item.active && <Badge>当前</Badge>}
                          </ItemTitle>
                          <ItemDescription>
                            API Key 已保存到 data/app.yaml
                          </ItemDescription>
                        </ItemContent>
                        <ItemActions className="ml-auto">
                          <Button
                            size="sm"
                            variant="outline"
                            disabled={busy}
                            onClick={() =>
                              void run(`account:test:${item.id}`, async () => {
                                const result = await call<{
                                  ok: boolean
                                  message: string
                                }>("test_provider", {
                                  id: provider.id,
                                  accountId: item.id,
                                })
                                if (result.ok) toast.success(result.message)
                                else toast.error(result.message)
                              })
                            }
                          >
                            {pendingTask === `account:test:${item.id}` && (
                              <Spinner data-icon="inline-start" />
                            )}
                            {pendingTask === `account:test:${item.id}`
                              ? "测试中"
                              : "测试"}
                          </Button>
                          <Button
                            size="sm"
                            disabled={busy || item.active}
                            onClick={() => {
                              if (!hasActiveProvider) {
                                setPendingActivation({
                                  providerId: provider.id,
                                  accountId: item.id,
                                })
                                return
                              }
                              void run(
                                `account:activate:${item.id}`,
                                async () => {
                                  await call("activate_upstream", {
                                    id: provider.id,
                                    accountId: item.id,
                                  })
                                  toast.success(
                                    "已热切换上游；会话保持不变，最小 custom 配置已同步"
                                  )
                                }
                              )
                            }}
                          >
                            {pendingTask === `account:activate:${item.id}` ? (
                              <Spinner data-icon="inline-start" />
                            ) : (
                              <Zap data-icon="inline-start" />
                            )}
                            {pendingTask === `account:activate:${item.id}`
                              ? "激活中"
                              : "激活"}
                          </Button>
                          <Button
                            size="icon-sm"
                            variant="ghost"
                            disabled={item.active || busy}
                            aria-label={`删除账号 ${item.name}`}
                            title="删除账号"
                            onClick={() =>
                              setPendingDelete({
                                kind: "account",
                                id: item.id,
                                name: item.name,
                              })
                            }
                          >
                            <Trash2 />
                          </Button>
                        </ItemActions>
                      </Item>
                    ))}
                  </ItemGroup>
                )}
              </CardContent>
            </Card>
          )
        })}
      </div>
      {draft && (
        <ProviderEditor
          key={`${draft.id}:${draft.models.join("\u001f")}`}
          value={draft}
          pendingTask={pendingTask}
          onChange={setDraft}
          onCancel={() => setDraft(undefined)}
          onSave={(provider) =>
            void run("provider:save", async () => {
              const saved = await call<Provider>("save_provider", {
                provider,
              })
              toast.success("供应商已保存")
              setDraft(undefined)
              if (!saved.models.length) {
                toast.info("请填写模型或保存后获取模型列表")
              }
            })
          }
          onFetch={() =>
            void run("provider:models", async () => {
              const linkedAccount = accounts.find(
                (item) => item.providerId === draft.id
              )
              if (!linkedAccount) {
                throw new Error("请先保存供应商并添加 API 账号")
              }
              const models = await call<FetchedModel[]>(
                "fetch_provider_models",
                {
                  providerId: draft.id,
                  accountId: linkedAccount.id,
                }
              )
              setDraft((current) =>
                current?.id === draft.id
                  ? {
                      ...current,
                      models: models.map((item) => item.id),
                      modelMetadata: models,
                    }
                  : current
              )
              toast.success(`获取到 ${models.length} 个模型`)
            })
          }
        />
      )}
      {account && (
        <AccountEditor
          value={account}
          pending={pendingTask === "account:save"}
          onChange={setAccount}
          onCancel={() => setAccount(undefined)}
          onSave={() =>
            void run("account:save", async () => {
              await call("save_provider_account", { account })
              setAccount(undefined)
              toast.success("账号已保存")
            })
          }
        />
      )}
      <AlertDialog
        open={confirmOpenAiLogin}
        onOpenChange={setConfirmOpenAiLogin}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>登录并切换到 OpenAI？</AlertDialogTitle>
            <AlertDialogDescription>
              完成设备授权后会自动激活新账号：整个 config.toml（包括
              MCP、Skills、Hooks、沙箱和未知字段）都会被清空，官方凭据将写入
              auth.json，并迁移受管会话。
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>取消</AlertDialogCancel>
            <AlertDialogAction
              disabled={busy}
              onClick={() => {
                setConfirmOpenAiLogin(false)
                void run("openai:login", async () => {
                  const authorization = await call<DeviceAuthorization>(
                    "start_openai_device_auth"
                  )
                  setDeviceAuthorization(authorization)
                  toast.info("登录码已生成，请在 OpenAI 页面完成授权")
                })
              }}
            >
              继续登录
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
      <AlertDialog
        open={Boolean(pendingActivation)}
        onOpenChange={(open) => {
          if (!open) setPendingActivation(undefined)
        }}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>切换到第三方 API？</AlertDialogTitle>
            <AlertDialogDescription>
              这会清空 auth.json，并清空后重新写入 config.toml 中第三方 API
              所需的最小配置。原有 MCP、Skills、Hooks、沙箱和其他未知字段都会被
              删除；受管会话将迁移到 custom，不会备份会话正文。
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>取消</AlertDialogCancel>
            <AlertDialogAction
              disabled={busy || !pendingActivation}
              onClick={() => {
                const activation = pendingActivation
                setPendingActivation(undefined)
                if (!activation) return
                void run("provider:activate", async () => {
                  await call("activate_provider", {
                    id: activation.providerId,
                    accountId: activation.accountId,
                  })
                  toast.success("已清空官方认证并切换到第三方 API")
                })
              }}
            >
              确认启用
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
      <AlertDialog
        open={Boolean(pendingOfficialAction)}
        onOpenChange={(open) => {
          if (!open) setPendingOfficialAction(undefined)
        }}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>
              {pendingOfficialAction?.kind === "delete"
                ? "删除 OpenAI 账号？"
                : "切换到官方账号？"}
            </AlertDialogTitle>
            <AlertDialogDescription>
              {pendingOfficialAction?.kind === "delete"
                ? `将从本地配置中删除账号“${pendingOfficialAction.account.name}”。此操作不会删除 OpenAI 云端账号，也不会改写当前 auth.json。`
                : `将清空整个 config.toml（包括 MCP、Skills、Hooks、沙箱和未知字段），再把账号“${pendingOfficialAction?.account.name ?? ""}”写入 auth.json，并迁移受管会话。`}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>取消</AlertDialogCancel>
            <AlertDialogAction
              variant={
                pendingOfficialAction?.kind === "delete"
                  ? "destructive"
                  : "default"
              }
              disabled={busy || !pendingOfficialAction}
              onClick={() => {
                const pending = pendingOfficialAction
                setPendingOfficialAction(undefined)
                if (!pending) return
                void run(
                  `openai:${pending.kind}:${pending.account.id}`,
                  async () => {
                    if (pending.kind === "delete") {
                      await call("delete_openai_account", {
                        id: pending.account.id,
                      })
                      toast.success("OpenAI 账号已从本地删除")
                      return
                    }
                    await call("activate_openai_account", {
                      id: pending.account.id,
                    })
                    toast.success("已清空第三方配置并切换到 OpenAI 官方账号")
                  }
                )
              }}
            >
              {pendingOfficialAction?.kind === "delete"
                ? "确认删除"
                : "确认切换"}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
      <AlertDialog
        open={Boolean(pendingDelete)}
        onOpenChange={(open) => {
          if (!open) setPendingDelete(undefined)
        }}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>
              删除{pendingDelete?.kind === "provider" ? "供应商" : "API 账号"}？
            </AlertDialogTitle>
            <AlertDialogDescription>
              将从本地配置中删除“{pendingDelete?.name ?? ""}”。
              {pendingDelete?.kind === "provider"
                ? "关联账号也会一并删除；当前正在使用的供应商不能删除。"
                : "此操作不会影响上游服务中的凭据。"}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>取消</AlertDialogCancel>
            <AlertDialogAction
              variant="destructive"
              disabled={busy || !pendingDelete}
              onClick={() => {
                const pending = pendingDelete
                setPendingDelete(undefined)
                if (!pending) return
                void run(`delete:${pending.kind}:${pending.id}`, async () => {
                  await call(
                    pending.kind === "provider"
                      ? "delete_provider"
                      : "delete_provider_account",
                    { id: pending.id }
                  )
                  toast.success(
                    pending.kind === "provider" ? "供应商已删除" : "账号已删除"
                  )
                })
              }}
            >
              确认删除
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  )
}

function formatTimestamp(value: number) {
  const date = new Date(timestampMilliseconds(value))
  if (Number.isNaN(date.getTime())) return "未知"
  return new Intl.DateTimeFormat("zh-CN", {
    dateStyle: "medium",
    timeStyle: "short",
  }).format(date)
}

function timestampMilliseconds(value: number) {
  return value < 10_000_000_000 ? value * 1000 : value
}

function ProviderEditor({
  value,
  pendingTask,
  onChange,
  onCancel,
  onSave,
  onFetch,
}: {
  value: Provider
  pendingTask?: string
  onChange: (value: Provider) => void
  onCancel: () => void
  onSave: (value: Provider) => void
  onFetch: () => void
}) {
  const [modelText, setModelText] = useState(() => value.models.join(", "))
  const busy = Boolean(pendingTask)

  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open && !busy) onCancel()
      }}
    >
      <DialogContent className="sm:max-w-xl">
        <DialogHeader>
          <DialogTitle>{value.id ? "编辑供应商" : "添加供应商"}</DialogTitle>
          <DialogDescription>
            配置上游地址、协议和可用模型。凭据在保存供应商后单独添加。
          </DialogDescription>
        </DialogHeader>
        <FieldGroup>
          <Field>
            <FieldLabel htmlFor="provider-name">名称</FieldLabel>
            <Input
              id="provider-name"
              autoFocus
              value={value.name}
              onChange={(event) =>
                onChange({ ...value, name: event.target.value })
              }
            />
          </Field>
          <Field>
            <FieldLabel id="provider-protocol-label">协议</FieldLabel>
            <ToggleGroup
              aria-labelledby="provider-protocol-label"
              value={[value.protocol]}
              onValueChange={(nextValue) => {
                const protocol = nextValue[0] as
                  Provider["protocol"] | undefined
                if (protocol) onChange({ ...value, protocol })
              }}
              variant="outline"
              spacing={0}
              className="grid w-full grid-cols-1 sm:grid-cols-3"
            >
              <ToggleGroupItem className="w-full" value="responses">
                Responses
              </ToggleGroupItem>
              <ToggleGroupItem className="w-full" value="chat_completions">
                Chat Completions
              </ToggleGroupItem>
              <ToggleGroupItem className="w-full" value="anthropic_messages">
                Anthropic
              </ToggleGroupItem>
            </ToggleGroup>
          </Field>
          <Field>
            <FieldLabel htmlFor="provider-base-url">Base URL</FieldLabel>
            <Input
              id="provider-base-url"
              value={value.baseUrl}
              onChange={(event) =>
                onChange({ ...value, baseUrl: event.target.value })
              }
            />
          </Field>
          <Field>
            <FieldLabel htmlFor="provider-models">模型</FieldLabel>
            <Input
              id="provider-models"
              value={modelText}
              onChange={(event) => setModelText(event.target.value)}
            />
            <FieldDescription>
              使用逗号分隔；保存供应商和账号后也可自动获取。
            </FieldDescription>
          </Field>
          <Field orientation="horizontal">
            <Switch
              id="provider-enabled"
              checked={value.enabled}
              onCheckedChange={(enabled) => onChange({ ...value, enabled })}
            />
            <FieldLabel htmlFor="provider-enabled">启用供应商</FieldLabel>
          </Field>
        </FieldGroup>
        <DialogFooter>
          <Button variant="outline" disabled={busy} onClick={onCancel}>
            取消
          </Button>
          {value.id && (
            <Button variant="outline" disabled={busy} onClick={onFetch}>
              {pendingTask === "provider:models" ? (
                <Spinner data-icon="inline-start" />
              ) : (
                <RefreshCw data-icon="inline-start" />
              )}
              {pendingTask === "provider:models" ? "获取中" : "获取模型"}
            </Button>
          )}
          <Button
            disabled={busy || !value.name || !value.baseUrl}
            onClick={() =>
              onSave({
                ...value,
                models: modelText
                  .split(",")
                  .map((item) => item.trim())
                  .filter(Boolean),
              })
            }
          >
            {pendingTask === "provider:save" ? (
              <Spinner data-icon="inline-start" />
            ) : (
              <Save data-icon="inline-start" />
            )}
            {pendingTask === "provider:save" ? "保存中" : "保存"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}

function AccountEditor({
  value,
  pending,
  onChange,
  onCancel,
  onSave,
}: {
  value: Account
  pending: boolean
  onChange: (value: Account) => void
  onCancel: () => void
  onSave: () => void
}) {
  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open && !pending) onCancel()
      }}
    >
      <DialogContent>
        <DialogHeader>
          <DialogTitle>添加 API 账号</DialogTitle>
          <DialogDescription>
            为供应商保存一组独立凭据，之后可测试连接并切换使用。
          </DialogDescription>
        </DialogHeader>
        <FieldGroup>
          <Field>
            <FieldLabel htmlFor="account-name">账号名称</FieldLabel>
            <Input
              id="account-name"
              autoFocus
              value={value.name}
              onChange={(event) =>
                onChange({ ...value, name: event.target.value })
              }
            />
          </Field>
          <Field>
            <FieldLabel htmlFor="account-api-key">API Key</FieldLabel>
            <Input
              id="account-api-key"
              type="password"
              autoComplete="off"
              value={value.apiKey ?? ""}
              onChange={(event) =>
                onChange({ ...value, apiKey: event.target.value })
              }
            />
            <FieldDescription>
              按你的要求以明文写入便携式 data/app.yaml。
            </FieldDescription>
          </Field>
        </FieldGroup>
        <DialogFooter>
          <Button variant="outline" disabled={pending} onClick={onCancel}>
            取消
          </Button>
          <Button disabled={pending || !value.apiKey} onClick={onSave}>
            {pending ? (
              <Spinner data-icon="inline-start" />
            ) : (
              <Save data-icon="inline-start" />
            )}
            {pending ? "保存中" : "保存"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}

function protocolLabel(protocol: Provider["protocol"]) {
  switch (protocol) {
    case "responses":
      return "OpenAI Responses"
    case "chat_completions":
      return "Chat Completions"
    case "anthropic_messages":
      return "Anthropic Messages"
  }
}
