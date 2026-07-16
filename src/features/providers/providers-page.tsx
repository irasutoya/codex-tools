import { useCallback, useEffect, useMemo, useRef, useState } from "react"
import {
  Activity,
  ArrowRightLeft,
  Boxes,
  Copy,
  ExternalLink,
  KeyRound,
  LogIn,
  MoreHorizontal,
  Pencil,
  Plus,
  RefreshCw,
  Save,
  Trash2,
  TriangleAlert,
  UserRound,
} from "lucide-react"

import { ErrorDetails } from "@/components/error-details"
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
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
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
  FieldContent,
  FieldDescription,
  FieldGroup,
  FieldLabel,
  FieldTitle,
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
import { notify } from "@/lib/feedback"
import { call } from "@/lib/ipc"
import { notifyRepairWarnings } from "@/lib/repair-feedback"
import { epochMilliseconds } from "@/lib/time"
import {
  emptyAccount,
  emptyProvider,
  type Account,
  type DeviceAuthorization,
  type OfficialAccountView,
  type PageProps,
  type Provider,
} from "@/types"

export default function ProvidersPage({ active }: PageProps) {
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
      const overview = await call("get_provider_overview")
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
    if (!active) return
    const timeout = window.setTimeout(() => {
      void load().catch(() => undefined)
    }, 0)
    return () => window.clearTimeout(timeout)
  }, [active, load])

  useEffect(() => {
    if (!deviceAuthorization) return
    const authorization = deviceAuthorization
    let cancelled = false
    let timer: number | undefined
    let pollErrorShown = false

    const scheduleNextPoll = () => {
      const remainingMs =
        epochMilliseconds(authorization.expiresAt) - Date.now()
      if (remainingMs <= 0) {
        setDeviceAuthorization(undefined)
        notify.error("登录码已过期", "请重新生成登录码后继续登录。")
        return
      }
      timer = window.setTimeout(
        () => {
          void call("poll_openai_device_auth", {
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
                notify.error("登录码已过期", "请重新生成登录码后继续登录。")
                return
              }
              notify.success(
                "OpenAI 登录成功",
                `Codex 现在使用 ${result.account.name}。`
              )
              notifyRepairWarnings(result.repair)
              void load().catch((error) =>
                notify.error("账号列表刷新失败", error)
              )
            })
            .catch((error) => {
              if (cancelled) return
              if (!pollErrorShown) {
                pollErrorShown = true
                notify.warning("正在重新确认登录结果", error)
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

  const run = async (
    task: string,
    action: () => Promise<unknown>,
    refreshAfter = true
  ) => {
    if (running.current) return
    running.current = true
    setPendingTask(task)
    try {
      await action()
      if (refreshAfter) {
        try {
          await load()
        } catch (error) {
          notify.warning("操作已完成，但列表刷新失败", error)
        }
      }
    } catch (error) {
      notify.error(taskFailureTitle(task), error)
    } finally {
      running.current = false
      setPendingTask(undefined)
    }
  }

  if (!overviewLoaded) {
    if (!overviewError) return <PageLoading label="正在读取账号和 API 服务" />
    return (
      <Alert variant="destructive">
        <TriangleAlert />
        <AlertTitle>无法读取账号和 API 服务</AlertTitle>
        <AlertDescription>
          <ErrorDetails
            error={overviewError}
            action={
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
            }
          >
            请确认本应用可以访问应用数据目录。
          </ErrorDetails>
        </AlertDescription>
      </Alert>
    )
  }

  return (
    <div className="flex flex-col gap-6">
      {overviewError && (
        <Alert variant="destructive">
          <TriangleAlert />
          <AlertTitle>显示的是上次读取的账号和服务</AlertTitle>
          <AlertDescription>
            <ErrorDetails
              error={overviewError}
              action={
                <Button
                  size="sm"
                  variant="outline"
                  onClick={() => void load().catch(() => undefined)}
                >
                  <RefreshCw data-icon="inline-start" />
                  刷新
                </Button>
              }
            >
              暂时无法获取最新列表，页面仍显示上次读取的结果。
            </ErrorDetails>
          </AlertDescription>
        </Alert>
      )}
      <section className="flex flex-col gap-4" aria-labelledby="openai-title">
        <div className="flex flex-col gap-3 sm:flex-row sm:items-end sm:justify-between">
          <div className="flex flex-col gap-1">
            <h2 id="openai-title" className="text-base font-medium">
              OpenAI 官方账号
            </h2>
            <p className="text-sm text-muted-foreground">
              通过浏览器安全登录，授权信息只保存在这台电脑。
            </p>
          </div>
          {officialAccounts.length > 0 && (
            <Button
              variant="secondary"
              disabled={busy || Boolean(deviceAuthorization)}
              onClick={() => setConfirmOpenAiLogin(true)}
            >
              <LogIn data-icon="inline-start" />
              添加账号
            </Button>
          )}
        </div>
        <Card>
          <CardHeader>
            <CardTitle className="flex items-center gap-2">
              已登录账号
              {officialAccounts.some((item) => item.active) && (
                <Badge>使用中</Badge>
              )}
            </CardTitle>
            <CardDescription>
              Codex 一次使用一个账号，可随时切换。
            </CardDescription>
          </CardHeader>
          <CardContent className="flex flex-col gap-4">
            {deviceAuthorization && (
              <div className="flex flex-col gap-3">
                <Alert>
                  <LogIn />
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
                    在打开的 OpenAI
                    页面输入此代码。完成后会自动刷新；代码有效期至
                    {formatTimestamp(deviceAuthorization.expiresAt)}。
                  </AlertDescription>
                </Alert>
                <div className="flex flex-wrap gap-2">
                  <Button
                    size="sm"
                    variant="outline"
                    onClick={() =>
                      void navigator.clipboard
                        .writeText(deviceAuthorization.userCode)
                        .then(() => notify.success("登录码已复制"))
                        .catch((error) => notify.error("无法复制登录码", error))
                    }
                  >
                    <Copy data-icon="inline-start" />
                    复制
                  </Button>
                  <Button
                    size="sm"
                    variant="outline"
                    onClick={() =>
                      void call("open_openai_device_page").catch((error) =>
                        notify.error("无法打开 OpenAI 登录页面", error)
                      )
                    }
                  >
                    <ExternalLink data-icon="inline-start" />
                    打开 OpenAI
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
                  <EmptyTitle>尚未登录 OpenAI</EmptyTitle>
                  <EmptyDescription>
                    登录后即可使用官方服务，凭据仅保存在本机。
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
                          ? `有效至 ${formatTimestamp(item.expiresAt)}，到期前自动续期`
                          : "有效期由 OpenAI 自动管理"}
                      </ItemDescription>
                    </ItemContent>
                    <ItemActions className="ml-auto">
                      <Button
                        size="sm"
                        variant="secondary"
                        disabled={busy || item.active}
                        onClick={() =>
                          setPendingOfficialAction({
                            kind: "activate",
                            account: item,
                          })
                        }
                      >
                        <ArrowRightLeft data-icon="inline-start" />
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
      </section>
      <section
        className="flex flex-col gap-4"
        aria-labelledby="custom-api-title"
      >
        <div className="flex flex-col gap-3 sm:flex-row sm:items-end sm:justify-between">
          <div className="flex flex-col gap-1">
            <h2 id="custom-api-title" className="text-base font-medium">
              第三方 API
            </h2>
            <p className="text-sm text-muted-foreground">
              连接兼容 Responses API 的服务；请求由 Codex 直接发送。
            </p>
          </div>
          <Button onClick={() => setDraft(emptyProvider())}>
            <Plus data-icon="inline-start" />
            添加服务
          </Button>
        </div>
        <div className="grid gap-4">
          {!providers.length && (
            <Empty>
              <EmptyHeader>
                <EmptyMedia variant="icon">
                  <Boxes />
                </EmptyMedia>
                <EmptyTitle>尚未添加第三方 API</EmptyTitle>
                <EmptyDescription>
                  添加兼容 OpenAI Responses API 的服务即可开始使用。
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
                  <CardDescription className="flex min-w-0 flex-col gap-1">
                    <span className="truncate" title={provider.baseUrl}>
                      {provider.baseUrl}
                    </span>
                    <span>Responses API · 模型列表由 Codex 读取</span>
                  </CardDescription>
                  <CardAction>
                    <DropdownMenu>
                      <DropdownMenuTrigger
                        render={
                          <Button
                            size="icon-sm"
                            variant="ghost"
                            aria-label={`${provider.name} 的更多操作`}
                            title="更多操作"
                          />
                        }
                      >
                        <MoreHorizontal />
                      </DropdownMenuTrigger>
                      <DropdownMenuContent align="end" className="w-44">
                        <DropdownMenuGroup>
                          <DropdownMenuItem onClick={() => setDraft(provider)}>
                            <Pencil />
                            编辑服务
                          </DropdownMenuItem>
                          <DropdownMenuItem
                            onClick={() =>
                              setAccount(emptyAccount(provider.id))
                            }
                          >
                            <KeyRound />
                            添加 API Key
                          </DropdownMenuItem>
                          <DropdownMenuItem
                            variant="destructive"
                            disabled={provider.active || busy}
                            onClick={() =>
                              setPendingDelete({
                                kind: "provider",
                                id: provider.id,
                                name: provider.name,
                              })
                            }
                          >
                            <Trash2 />
                            删除服务
                          </DropdownMenuItem>
                        </DropdownMenuGroup>
                      </DropdownMenuContent>
                    </DropdownMenu>
                  </CardAction>
                </CardHeader>
                <CardContent className="flex flex-col gap-3">
                  {!linked.length ? (
                    <Empty className="min-h-40 py-6">
                      <EmptyHeader>
                        <EmptyTitle>尚未添加 API Key</EmptyTitle>
                        <EmptyDescription>
                          添加后即可测试连接并使用此服务。
                        </EmptyDescription>
                      </EmptyHeader>
                      <EmptyContent>
                        <Button
                          size="sm"
                          variant="secondary"
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
                              仅保存在本机，启用此服务时写入 Codex。
                            </ItemDescription>
                          </ItemContent>
                          <ItemActions className="ml-auto">
                            <Button
                              size="sm"
                              variant="secondary"
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
                                <ArrowRightLeft data-icon="inline-start" />
                              )}
                              {pendingTask === `account:activate:${item.id}`
                                ? "切换中…"
                                : "使用"}
                            </Button>
                            <DropdownMenu>
                              <DropdownMenuTrigger
                                render={
                                  <Button
                                    size="icon-sm"
                                    variant="ghost"
                                    aria-label={`${item.name} 的更多操作`}
                                    title="更多操作"
                                  />
                                }
                              >
                                <MoreHorizontal />
                              </DropdownMenuTrigger>
                              <DropdownMenuContent align="end" className="w-40">
                                <DropdownMenuGroup>
                                  <DropdownMenuItem
                                    disabled={busy}
                                    onClick={() =>
                                      void run(
                                        `account:test:${item.id}`,
                                        async () => {
                                          const result = await call(
                                            "test_provider",
                                            {
                                              id: provider.id,
                                              accountId: item.id,
                                            }
                                          )
                                          const detail = result.suggestV1
                                            ? `${result.message} 请确认 API 地址以 /v1 结尾。`
                                            : result.message
                                          if (result.ok) {
                                            notify.success(
                                              "连接测试成功",
                                              detail
                                            )
                                          } else {
                                            notify.error("连接测试失败", detail)
                                          }
                                        },
                                        false
                                      )
                                    }
                                  >
                                    {pendingTask ===
                                    `account:test:${item.id}` ? (
                                      <Spinner />
                                    ) : (
                                      <Activity />
                                    )}
                                    测试连接
                                  </DropdownMenuItem>
                                  <DropdownMenuItem
                                    variant="destructive"
                                    disabled={item.active || busy}
                                    onClick={() =>
                                      setPendingDelete({
                                        kind: "account",
                                        id: item.id,
                                        name: item.name,
                                      })
                                    }
                                  >
                                    <Trash2 />
                                    删除 API Key
                                  </DropdownMenuItem>
                                </DropdownMenuGroup>
                              </DropdownMenuContent>
                            </DropdownMenu>
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
      </section>
      {draft && (
        <ProviderEditor
          value={draft}
          pendingTask={pendingTask}
          onChange={setDraft}
          onCancel={() => setDraft(undefined)}
          onSave={(provider) =>
            void run("provider:save", async () => {
              await call("save_provider", {
                provider,
              })
              notify.success("API 服务已保存")
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
              notify.success("API Key 已保存")
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
              登录成功后，Codex 会立即使用新账号。第三方 API
              和其他设置不会删除。
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>取消</AlertDialogCancel>
            <AlertDialogAction
              disabled={busy}
              onClick={() => {
                setConfirmOpenAiLogin(false)
                void run("openai:login", async () => {
                  const authorization = await call("start_openai_device_auth")
                  setDeviceAuthorization(authorization)
                  notify.info(
                    "登录码已生成",
                    "请在打开的 OpenAI 页面完成登录，本页会自动更新。"
                  )
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
            <AlertDialogTitle>切换到此 API 服务？</AlertDialogTitle>
            <AlertDialogDescription>
              Codex 将使用此服务的地址和密钥，并自动读取模型列表。OpenAI
              账号仍会保留。
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
                    const repair = await call("activate_provider", {
                      id: activation.providerId,
                      accountId: activation.accountId,
                    })
                    notify.success("Codex 已切换到所选 API 服务")
                    notifyRepairWarnings(repair)
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
                : "切换到此 OpenAI 账号？"}
            </AlertDialogTitle>
            <AlertDialogDescription>
              {pendingOfficialAction?.kind === "delete"
                ? `只会删除本机保存的“${pendingOfficialAction.account.name}”登录信息，不会删除 OpenAI 云端账号。`
                : `Codex 将使用“${pendingOfficialAction?.account.name ?? ""}”。第三方 API 仍会保留。`}
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
                      notify.success("OpenAI 账号已从本机删除")
                      return
                    }
                    const repair = await call("activate_openai_account", {
                      id: pending.account.id,
                    })
                    notify.success("Codex 已切换到所选 OpenAI 账号")
                    notifyRepairWarnings(repair)
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
              将从本机删除“{pendingDelete?.name ?? ""}”。
              {pendingDelete?.kind === "provider"
                ? "该服务下的所有 API Key 也会一并删除。"
                : "服务商网站上的 API Key 不会被撤销。"}
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
                  notify.success(
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

function taskFailureTitle(task: string) {
  if (task.startsWith("account:test:")) return "连接测试失败"
  if (task === "provider:save") return "API 服务保存失败"
  if (task === "account:save") return "API Key 保存失败"
  if (task === "openai:login") return "无法开始 OpenAI 登录"
  if (task.startsWith("account:activate:")) return "无法切换 API 服务"
  if (task.startsWith("openai:activate:")) return "无法切换 OpenAI 账号"
  if (task.startsWith("openai:delete:")) return "OpenAI 账号删除失败"
  if (task.startsWith("delete:provider:")) return "API 服务删除失败"
  if (task.startsWith("delete:account:")) return "API Key 删除失败"
  return "操作未完成"
}

function formatTimestamp(value: number) {
  const date = new Date(epochMilliseconds(value))
  if (Number.isNaN(date.getTime())) return "时间未知"
  return new Intl.DateTimeFormat("zh-CN", {
    dateStyle: "medium",
    timeStyle: "short",
  }).format(date)
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
      <DialogContent>
        <DialogHeader>
          <DialogTitle>
            {value.id ? "编辑 API 服务" : "添加 API 服务"}
          </DialogTitle>
          <DialogDescription>
            填写服务名称和 Responses API 地址。保存后再添加 API Key。
          </DialogDescription>
        </DialogHeader>
        <FieldGroup>
          <Field>
            <FieldLabel htmlFor="provider-name">服务名称</FieldLabel>
            <Input
              id="provider-name"
              autoFocus
              required
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
              type="url"
              required
              placeholder="https://api.example.com/v1"
              value={value.baseUrl}
              onChange={(event) =>
                onChange({ ...value, baseUrl: event.target.value })
              }
            />
            <FieldDescription>
              填写服务商提供的 API 根地址，通常以 /v1 结尾。
            </FieldDescription>
          </Field>
          <Field orientation="horizontal">
            <FieldContent>
              <FieldTitle>启用此服务</FieldTitle>
              <FieldDescription>
                关闭后仍会保留配置，但不能切换使用。
              </FieldDescription>
            </FieldContent>
            <Switch
              id="provider-enabled"
              aria-label="启用此服务"
              checked={value.enabled}
              onCheckedChange={(enabled) => onChange({ ...value, enabled })}
            />
          </Field>
        </FieldGroup>
        <DialogFooter>
          <Button variant="outline" disabled={busy} onClick={onCancel}>
            取消
          </Button>
          <Button
            disabled={busy || !value.name.trim() || !value.baseUrl.trim()}
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
            可以为同一服务保存多个密钥，并随时切换。
          </DialogDescription>
        </DialogHeader>
        <FieldGroup>
          <Field>
            <FieldLabel htmlFor="account-name">密钥名称</FieldLabel>
            <Input
              id="account-name"
              autoFocus
              required
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
              required
              type="password"
              autoComplete="off"
              placeholder="sk-…"
              value={value.apiKey ?? ""}
              onChange={(event) =>
                onChange({ ...value, apiKey: event.target.value })
              }
            />
            <FieldDescription>
              仅保存在本机；启用服务后写入 Codex 的 auth.json。
            </FieldDescription>
          </Field>
        </FieldGroup>
        <DialogFooter>
          <Button variant="outline" disabled={pending} onClick={onCancel}>
            取消
          </Button>
          <Button
            disabled={pending || !value.name.trim() || !value.apiKey?.trim()}
            onClick={onSave}
          >
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
