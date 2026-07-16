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
  TriangleAlert,
  UserPlus,
  UserRound,
  Zap,
} from "lucide-react"
import { toast } from "sonner"

import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { PageLoading } from "@/components/page-loading"
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
import { call } from "@/lib/ipc"
import {
  emptyAccount,
  emptyProvider,
  type Account,
  type DeviceAuthorization,
  type DeviceAuthPollResult,
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
  const [overviewLoaded, setOverviewLoaded] = useState(false)
  const [overviewError, setOverviewError] = useState<string>()
  const [pendingDelete, setPendingDelete] = useState<
    | { kind: "provider"; id: string; name: string }
    | { kind: "account"; id: string; name: string }
  >()
  const running = useRef(false)
  const busy = Boolean(pendingTask)
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
    try {
      const overview = await call<ProviderOverview>("get_provider_overview")
      setProviders(overview.providers)
      setAccounts(overview.accounts)
      setOfficialAccounts(overview.officialAccounts)
      setOverviewLoaded(true)
      setOverviewError(undefined)
    } catch (error) {
      setOverviewError(String(error))
      throw error
    }
  }, [])
  useEffect(() => {
    const timeout = window.setTimeout(() => {
      void load().catch(() => undefined)
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
        toast.error("登录码已过期，请重新开始登录")
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
                toast.error("登录码已过期，请重新开始登录")
                return
              }
              toast.success(`已登录，Codex 现在使用 ${result.account.name}`)
              if (result.repair.warnings.length) {
                toast.warning(
                  `账号已切换，但有 ${result.repair.warnings.length} 个历史会话需要检查`
                )
              }
              void load().catch((error) => toast.error(String(error)))
            })
            .catch((error) => {
              if (cancelled) return
              if (!pollErrorShown) {
                pollErrorShown = true
                toast.error(
                  `暂时无法确认登录结果，将继续重试：${String(error)}`
                )
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

  if (!overviewLoaded) {
    if (!overviewError) return <PageLoading />
    return (
      <Alert variant="destructive">
        <TriangleAlert />
        <AlertTitle>暂时无法读取账号和服务</AlertTitle>
        <AlertDescription className="flex flex-wrap items-center gap-3">
          <span>{overviewError}</span>
          <Button
            size="sm"
            variant="outline"
            onClick={() => {
              setOverviewError(undefined)
              void load().catch(() => undefined)
            }}
          >
            <RefreshCw data-icon="inline-start" />
            重试
          </Button>
        </AlertDescription>
      </Alert>
    )
  }

  return (
    <div className="flex flex-col gap-8">
      {overviewError && (
        <Alert variant="destructive">
          <TriangleAlert />
          <AlertTitle>未能获取最新的账号和服务</AlertTitle>
          <AlertDescription className="flex flex-wrap items-center gap-3">
            <span>{overviewError}</span>
            <Button
              size="sm"
              variant="outline"
              onClick={() => void load().catch(() => undefined)}
            >
              <RefreshCw data-icon="inline-start" />
              重试
            </Button>
          </AlertDescription>
        </Alert>
      )}
      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2">
            OpenAI 账号
            {officialAccounts.some((item) => item.active) && (
              <Badge>使用中</Badge>
            )}
          </CardTitle>
          <CardDescription>
            在浏览器中完成 OpenAI 登录。切换账号时会保留其他 Codex 设置。
          </CardDescription>
          <CardAction className="max-sm:col-span-2 max-sm:row-start-auto max-sm:justify-self-start">
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
                    等待登录
                  </Badge>
                </AlertTitle>
                <AlertDescription>
                  打开 {deviceAuthorization.verificationUri}，输入上面的登录码。
                  登录码有效期至{" "}
                  {formatTimestamp(deviceAuthorization.expiresAt)}
                  。完成后此处会自动更新。
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
                <EmptyTitle>还没有登录 OpenAI</EmptyTitle>
                <EmptyDescription>
                  登录信息只保存在本机，并直接供 Codex 使用。
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
                      {item.active && <Badge>使用中</Badge>}
                    </ItemTitle>
                    <ItemDescription className="truncate">
                      {item.email || item.accountId}
                    </ItemDescription>
                    <ItemDescription>
                      {item.expiresAt
                        ? `当前登录有效至 ${formatTimestamp(item.expiresAt)}，到期前会自动续期`
                        : "登录有效期由 OpenAI 自动管理"}
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
                      使用此账号
                    </Button>
                    <Button
                      size="icon-sm"
                      variant="ghost"
                      disabled={busy || item.active}
                      aria-label={`删除已保存的 OpenAI 账号 ${item.name}`}
                      title="删除已保存的账号"
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
      <div className="md3-section-heading">
        <div>
          <h2>第三方 API 服务</h2>
          <p>服务地址和 API Key 会直接交给 Codex，本应用不会转发请求。</p>
        </div>
        <Button onClick={() => setDraft(emptyProvider())}>
          <Plus data-icon="inline-start" />
          添加服务
        </Button>
      </div>
      <div className="grid gap-5">
        {!providers.length && (
          <Empty>
            <EmptyHeader>
              <EmptyMedia variant="icon">
                <Boxes />
              </EmptyMedia>
              <EmptyTitle>还没有添加第三方 API 服务</EmptyTitle>
              <EmptyDescription>
                可添加任何兼容 OpenAI Responses API 的服务。
              </EmptyDescription>
            </EmptyHeader>
            <EmptyContent>
              <Button size="sm" onClick={() => setDraft(emptyProvider())}>
                <Plus data-icon="inline-start" />
                添加服务
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
                  {provider.active && <Badge>使用中</Badge>}
                  {!provider.enabled && (
                    <Badge variant="secondary">不可用</Badge>
                  )}
                </CardTitle>
                <CardDescription className="truncate">
                  OpenAI Responses · {provider.baseUrl}
                </CardDescription>
                <CardDescription className="truncate">
                  Codex 将直接从此服务读取可用模型。
                </CardDescription>
                <CardAction className="flex gap-1 max-sm:col-span-2 max-sm:row-start-auto max-sm:justify-self-end">
                  <Button
                    size="icon-sm"
                    variant="outline"
                    aria-label={`编辑服务 ${provider.name}`}
                    title="编辑服务"
                    onClick={() => setDraft(provider)}
                  >
                    <Pencil />
                  </Button>
                  <Button
                    size="icon-sm"
                    variant="outline"
                    aria-label={`为 ${provider.name} 添加 API Key`}
                    title="添加 API Key"
                    onClick={() => setAccount(emptyAccount(provider.id))}
                  >
                    <UserPlus />
                  </Button>
                  <Button
                    size="icon-sm"
                    variant="ghost"
                    disabled={provider.active || busy}
                    aria-label={`删除服务 ${provider.name}`}
                    title="删除服务"
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
                {!linked.length ? (
                  <Empty className="py-4">
                    <EmptyHeader>
                      <EmptyTitle>还没有添加 API Key</EmptyTitle>
                      <EmptyDescription>
                        保存 API Key 后即可测试连接或切换使用。
                      </EmptyDescription>
                    </EmptyHeader>
                    <EmptyContent>
                      <Button
                        size="sm"
                        variant="outline"
                        onClick={() => setAccount(emptyAccount(provider.id))}
                      >
                        <KeyRound data-icon="inline-start" />
                        添加 API Key
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
                            {item.active && <Badge>使用中</Badge>}
                          </ItemTitle>
                          <ItemDescription>
                            API Key 已保存在本机；使用此服务时会同步给 Codex。
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
                              ? "测试中…"
                              : "测试连接"}
                          </Button>
                          <Button
                            size="sm"
                            disabled={busy || item.active}
                            title="让 Codex 使用此 API 地址和 API Key"
                            onClick={() => {
                              setPendingActivation({
                                providerId: provider.id,
                                accountId: item.id,
                              })
                            }}
                          >
                            {pendingTask === `account:activate:${item.id}` ? (
                              <Spinner data-icon="inline-start" />
                            ) : (
                              <Zap data-icon="inline-start" />
                            )}
                            {pendingTask === `account:activate:${item.id}`
                              ? "切换中…"
                              : "使用"}
                          </Button>
                          <Button
                            size="icon-sm"
                            variant="ghost"
                            disabled={item.active || busy}
                            aria-label={`删除 API Key ${item.name}`}
                            title="删除 API Key"
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
          value={draft}
          pendingTask={pendingTask}
          onChange={setDraft}
          onCancel={() => setDraft(undefined)}
          onSave={(provider) =>
            void run("provider:save", async () => {
              await call<Provider>("save_provider", {
                provider,
              })
              toast.success("第三方 API 服务已保存")
              setDraft(undefined)
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
              toast.success("API Key 已保存")
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
            <AlertDialogTitle>登录 OpenAI 并立即使用？</AlertDialogTitle>
            <AlertDialogDescription>
              授权完成后，Codex 会自动切换到新登录的账号。已保存的第三方 API
              服务不会删除，其他 Codex 设置也会保留。
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
                  toast.info("登录码已生成，请在 OpenAI 页面完成登录")
                })
              }}
            >
              开始登录
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
            <AlertDialogTitle>改用这个第三方 API？</AlertDialogTitle>
            <AlertDialogDescription>
              Codex 将直接使用该服务的 API 地址和已保存的 API
              Key，并自行读取服务 提供的模型列表。OpenAI
              登录账号仍会保存在本应用中，之后可以随时切回。
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
                void run(
                  `account:activate:${activation.accountId}`,
                  async () => {
                    await call("activate_provider", {
                      id: activation.providerId,
                      accountId: activation.accountId,
                    })
                    toast.success("Codex 已切换到所选第三方 API 服务")
                  }
                )
              }}
            >
              确认切换
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
                ? "删除已保存的 OpenAI 账号？"
                : "改用这个 OpenAI 账号？"}
            </AlertDialogTitle>
            <AlertDialogDescription>
              {pendingOfficialAction?.kind === "delete"
                ? `只会删除本机保存的“${pendingOfficialAction.account.name}”登录信息，不会删除 OpenAI 云端账号。`
                : `Codex 将改用“${pendingOfficialAction?.account.name ?? ""}”登录。第三方 API 服务仍会保留，之后可以随时切回。`}
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
                      toast.success("已删除本机保存的 OpenAI 账号")
                      return
                    }
                    await call("activate_openai_account", {
                      id: pending.account.id,
                    })
                    toast.success("Codex 已切换到所选 OpenAI 账号")
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
              删除{pendingDelete?.kind === "provider" ? "服务" : "API Key"}？
            </AlertDialogTitle>
            <AlertDialogDescription>
              将删除本机保存的“{pendingDelete?.name ?? ""}”。
              {pendingDelete?.kind === "provider"
                ? "该服务下保存的所有 API Key 也会一并删除。正在使用的服务不能删除。"
                : "这不会撤销服务商网站上的 API Key。"}
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
                    pending.kind === "provider"
                      ? "第三方 API 服务已删除"
                      : "API Key 已删除"
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
  if (Number.isNaN(date.getTime())) return "时间未知"
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
}: {
  value: Provider
  pendingTask?: string
  onChange: (value: Provider) => void
  onCancel: () => void
  onSave: (value: Provider) => void
}) {
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
          <DialogTitle>
            {value.id ? "编辑 API 服务" : "添加 API 服务"}
          </DialogTitle>
          <DialogDescription>
            填写服务商提供的 OpenAI Responses API 地址。保存后再添加 API Key。
          </DialogDescription>
        </DialogHeader>
        <FieldGroup>
          <Field>
            <FieldLabel htmlFor="provider-name">服务名称</FieldLabel>
            <Input
              id="provider-name"
              autoFocus
              placeholder="例如：公司 API"
              value={value.name}
              onChange={(event) =>
                onChange({ ...value, name: event.target.value })
              }
            />
          </Field>
          <Field>
            <FieldLabel htmlFor="provider-base-url">API 地址</FieldLabel>
            <Input
              id="provider-base-url"
              value={value.baseUrl}
              onChange={(event) =>
                onChange({ ...value, baseUrl: event.target.value })
              }
            />
            <FieldDescription>
              通常以 /v1 结尾，例如 https://api.example.com/v1。
            </FieldDescription>
          </Field>
          <Field orientation="horizontal">
            <Switch
              id="provider-enabled"
              checked={value.enabled}
              onCheckedChange={(enabled) => onChange({ ...value, enabled })}
            />
            <FieldLabel htmlFor="provider-enabled">允许使用此服务</FieldLabel>
          </Field>
        </FieldGroup>
        <DialogFooter>
          <Button variant="outline" disabled={busy} onClick={onCancel}>
            取消
          </Button>
          <Button
            disabled={busy || !value.name || !value.baseUrl}
            onClick={() => onSave(value)}
          >
            {pendingTask === "provider:save" ? (
              <Spinner data-icon="inline-start" />
            ) : (
              <Save data-icon="inline-start" />
            )}
            {pendingTask === "provider:save" ? "正在保存…" : "保存服务"}
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
          <DialogTitle>添加 API Key</DialogTitle>
          <DialogDescription>
            为这个服务保存一个 API Key。可以保存多个，方便在不同账号间切换。
          </DialogDescription>
        </DialogHeader>
        <FieldGroup>
          <Field>
            <FieldLabel htmlFor="account-name">密钥名称</FieldLabel>
            <Input
              id="account-name"
              autoFocus
              placeholder="例如：个人密钥"
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
              API Key 会保存在当前系统账号的应用数据中，并在使用此服务时同步给
              Codex。
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
            {pending ? "正在保存…" : "保存 API Key"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
