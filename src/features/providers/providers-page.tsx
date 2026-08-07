import { useCallback, useEffect, useMemo, useRef, useState } from "react"
import {
  Activity01Icon,
  ArrowUpDownIcon,
  BoxesIcon,
  CheckIcon,
  Copy01Icon,
  ExternalLinkIcon,
  Key01Icon,
  Login01Icon,
  More01Icon,
  PencilIcon,
  Add01Icon,
  Refresh01Icon,
  Delete01Icon,
  Alert01Icon,
  User02Icon,
  Tag01Icon,
} from "@hugeicons/core-free-icons"
import { HugeiconsIcon } from "@hugeicons/react"

import { ErrorDetails } from "@/components/error-details"
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
  Item,
  ItemActions,
  ItemContent,
  ItemDescription,
  ItemGroup,
  ItemMedia,
  ItemTitle,
} from "@/components/ui/item"
import { Skeleton } from "@/components/ui/skeleton"
import { Spinner } from "@/components/ui/spinner"
import { notify, formatError } from "@/lib/feedback"
import { call } from "@/lib/ipc"
import {
  refreshCoordinator,
  useAppForeground,
  usePageRefresh,
} from "@/lib/refresh-coordinator"
import { notifyRepairWarnings } from "@/lib/repair-feedback"
import { formatDateTime } from "@/lib/time"
import {
  runQuotaRefresh,
  useAutoQuotaRefresh,
} from "@/lib/use-auto-quota-refresh"
import { useAutoModelRefresh } from "@/lib/use-auto-model-refresh"
import {
  emptyProvider,
  type OfficialAccountView,
  type PageProps,
  type Provider,
  type UsageOverview,
  type UsageRow,
} from "@/types"

import { QuotaStatusView } from "./quota-status"
import { ProviderEditor } from "./provider-editor"
import { ProxyLoginDialog } from "./proxy-login-dialog"
import { useDeviceAuthorizationPolling } from "./use-device-auth"
import { taskFailureTitle } from "./provider-utils"
import {
  formatTokens,
  formatUsdMicrousd,
  getLocalRange,
} from "../usage/usage-format"

export default function ProvidersPage({ active }: PageProps) {
  const refreshSignal = usePageRefresh("providers")
  const foreground = useAppForeground()
  const [providers, setProviders] = useState<Provider[]>([])
  const [officialAccounts, setOfficialAccounts] = useState<
    OfficialAccountView[]
  >([])
  const [usage, setUsage] = useState<UsageOverview>()
  const [draft, setDraft] = useState<Provider>()
  const [pendingActivation, setPendingActivation] = useState<string>()
  const [pendingOfficialAction, setPendingOfficialAction] = useState<{
    kind: "activate" | "delete"
    account: OfficialAccountView
  }>()
  const [confirmOpenAiLogin, setConfirmOpenAiLogin] = useState(false)
  const [proxyLoginOpen, setProxyLoginOpen] = useState(false)
  const [pendingTask, setPendingTask] = useState<string>()
  const [overviewLoaded, setOverviewLoaded] = useState(false)
  const [overviewError, setOverviewError] = useState<string>()
  const [pendingDelete, setPendingDelete] = useState<{
    id: string
    name: string
  }>()
  const running = useRef(false)
  const lastRefreshRevision = useRef<number | undefined>(undefined)
  const busy = Boolean(pendingTask)
  const activeProviderId = providers.find((provider) => provider.active)?.id

  // 第三方 API 模型列表自动同步：进入页面/前台时静默刷新，之后每 10 分钟一次。
  useAutoModelRefresh({
    providerId: activeProviderId,
    active,
    foreground,
    refresh: () => call("refresh_active_provider_models"),
    onRefreshed: () => {
      void load().catch(() => undefined)
    },
  })
  const usageByProvider = useMemo(() => {
    const result = new Map<string, UsageRow>()
    for (const row of usage?.rows ?? []) {
      if (row.providerId) result.set(row.providerId, row)
    }
    return result
  }, [usage])
  const usageByAccount = useMemo(() => {
    const result = new Map<string, UsageRow>()
    for (const row of usage?.rows ?? []) {
      if (row.accountId) result.set(row.accountId, row)
    }
    return result
  }, [usage])
  const load = useCallback(async () => {
    try {
      const overview = await call("get_provider_overview")
      setProviders(overview.providers)
      setOfficialAccounts(overview.officialAccounts)
      setOverviewLoaded(true)
      setOverviewError(undefined)
    } catch (error) {
      setOverviewError(formatError(error))
      throw error
    }
  }, [])
  const loadUsage = useCallback(async (refresh = false) => {
    const query = { range: getLocalRange(1), groupBy: "account" as const }
    try {
      const result = refresh
        ? await call("refresh_usage", { query })
        : await call("get_usage_overview", { query })
      setUsage(result)
      if (refresh) refreshCoordinator.invalidate(["dashboard", "usage"])
      return result
    } catch {
      // Provider management remains available when usage files are unavailable.
      return undefined
    }
  }, [])
  const initialized = useRef(false)
  useEffect(() => {
    if (!active) return
    const firstLoad = !initialized.current
    if (
      !firstLoad &&
      (!foreground || lastRefreshRevision.current === refreshSignal.revision)
    ) {
      return
    }
    const timeout = window.setTimeout(() => {
      initialized.current = true
      lastRefreshRevision.current = refreshSignal.revision
      void load().catch(() => undefined)
      void loadUsage(true)
    }, 0)
    return () => window.clearTimeout(timeout)
  }, [active, foreground, load, loadUsage, refreshSignal.revision])

  const activeOfficialAccount = officialAccounts.find(
    (account) => account.active
  )
  const refreshActiveQuota = useCallback(async () => {
    if (!activeOfficialAccount) {
      throw new Error("当前没有可刷新的 OpenAI 账号")
    }
    const result = await call("refresh_official_account_quota", {
      accountId: activeOfficialAccount.id,
    })
    if (!refreshCoordinator.getForeground()) {
      refreshCoordinator.invalidate(["dashboard", "providers"])
      return result
    }
    await load()
    refreshCoordinator.invalidate(["dashboard"])
    return result
  }, [activeOfficialAccount, load])

  useAutoQuotaRefresh({
    accountId: activeOfficialAccount?.id,
    active,
    foreground,
    quota: activeOfficialAccount?.quota,
    refresh: refreshActiveQuota,
  })

  const [deviceAuthorization, setDeviceAuthorization] =
    useDeviceAuthorizationPolling(load)

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
          notify.warning("操作已完成，但无法读取最新列表", error)
        }
        // 切换/删除/导入账号会影响用量归属，同时刷新本页用量与用量页，
        // 避免旧的“今日用量”归属数据残留。
        if (TASKS_REFRESHING_USAGE.some((prefix) => task.startsWith(prefix))) {
          void loadUsage(true)
          refreshCoordinator.invalidate(["usage"])
        }
      }
      refreshCoordinator.invalidate(["dashboard", "settings"])
    } catch (error) {
      notify.error(taskFailureTitle(task), error)
    } finally {
      running.current = false
      setPendingTask(undefined)
    }
  }

  if (!overviewLoaded) {
    if (!overviewError) return <ProvidersLoading />
    return (
      <Alert variant="destructive">
        <HugeiconsIcon icon={Alert01Icon} />
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
                <HugeiconsIcon icon={Refresh01Icon} data-icon="inline-start" />
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
      <div className="flex flex-wrap items-center justify-between gap-2">
        <p className="max-w-prose text-sm text-muted-foreground">
          管理 OpenAI 账号与第三方 API 服务；切换后 Codex
          的下一次请求即使用所选连接。
        </p>
        <div className="flex flex-wrap items-center gap-2">
          <Button
            variant="outline"
            disabled={busy}
            onClick={() => setDraft(emptyProvider())}
          >
            <HugeiconsIcon icon={Add01Icon} data-icon="inline-start" />
            添加 API 服务
          </Button>
          <Button
            variant="outline"
            disabled={busy}
            onClick={() => setProxyLoginOpen(true)}
          >
            <HugeiconsIcon icon={Key01Icon} data-icon="inline-start" />
            导入 Cookie
          </Button>
          <Button
            disabled={busy || Boolean(deviceAuthorization)}
            onClick={() => setConfirmOpenAiLogin(true)}
          >
            <HugeiconsIcon icon={Login01Icon} data-icon="inline-start" />
            网页登录
          </Button>
        </div>
      </div>

      {overviewError && (
        <Alert variant="destructive">
          <HugeiconsIcon icon={Alert01Icon} />
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
                  <HugeiconsIcon
                    icon={Refresh01Icon}
                    data-icon="inline-start"
                  />
                  刷新
                </Button>
              }
            >
              暂时无法获取最新列表，页面仍显示上次读取的结果。
            </ErrorDetails>
          </AlertDescription>
        </Alert>
      )}

      {/* OpenAI 账号 */}
      <section className="flex flex-col gap-6">
        <div className="flex flex-wrap items-center justify-between gap-3">
          <div>
            <h3 className="text-base font-medium">
              OpenAI 账号（{officialAccounts.length}）
            </h3>
            <p className="text-sm text-muted-foreground">
              网页登录和 Cookie 账号统一管理；额度只显示 OpenAI
              接口实际返回的结果。
            </p>
          </div>
          {officialAccounts.length > 0 && (
            <Button
              size="sm"
              variant="outline"
              disabled={busy}
              onClick={() =>
                void run("official:quota:all", async () => {
                  const results = await call("refresh_all_official_quotas")
                  const succeeded = results.filter(
                    (result) => result.quota.status === "success"
                  ).length
                  notify.success(
                    "账号额度刷新完成",
                    succeeded === results.length
                      ? `${results.length} 个账号均查询成功。`
                      : `${succeeded}/${results.length} 个账号查询成功；其余账号请查看各自状态。`
                  )
                })
              }
            >
              {pendingTask === "official:quota:all" ? (
                <Spinner data-icon="inline-start" />
              ) : (
                <HugeiconsIcon icon={Refresh01Icon} data-icon="inline-start" />
              )}
              刷新全部额度
            </Button>
          )}
        </div>
        {deviceAuthorization && (
          <Alert>
            <HugeiconsIcon icon={Login01Icon} />
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
              在打开的 OpenAI 页面输入此代码。完成后会自动刷新；代码有效期至
              {formatDateTime(deviceAuthorization.expiresAt)}。
            </AlertDescription>
            <div className="mt-3 flex flex-wrap gap-2">
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
                <HugeiconsIcon icon={Copy01Icon} data-icon="inline-start" />
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
                <HugeiconsIcon
                  icon={ExternalLinkIcon}
                  data-icon="inline-start"
                />
                打开 OpenAI
              </Button>
            </div>
          </Alert>
        )}
        {!officialAccounts.length ? (
          <Empty className="min-h-48 border">
            <EmptyHeader>
              <EmptyMedia variant="icon">
                <HugeiconsIcon icon={User02Icon} />
              </EmptyMedia>
              <EmptyTitle>尚未添加 OpenAI 账号</EmptyTitle>
              <EmptyDescription>
                通过 OpenAI 网页登录，或粘贴 Cookie 凭据导入账号。
              </EmptyDescription>
            </EmptyHeader>
            <EmptyContent className="flex flex-wrap gap-2">
              <Button
                size="sm"
                variant="outline"
                disabled={busy}
                onClick={() => setProxyLoginOpen(true)}
              >
                <HugeiconsIcon icon={Key01Icon} data-icon="inline-start" />
                导入 Cookie
              </Button>
              <Button
                size="sm"
                disabled={busy || Boolean(deviceAuthorization)}
                onClick={() => setConfirmOpenAiLogin(true)}
              >
                <HugeiconsIcon icon={Login01Icon} data-icon="inline-start" />
                网页登录
              </Button>
            </EmptyContent>
          </Empty>
        ) : (
          <ItemGroup className="gap-4">
            {officialAccounts.map((item) => (
              <Item
                key={item.id}
                variant={item.active ? "muted" : "outline"}
                className="items-start gap-5 p-5"
              >
                <ItemMedia variant="icon">
                  {item.source === "proxy_import" ? (
                    <HugeiconsIcon icon={Key01Icon} />
                  ) : (
                    <HugeiconsIcon icon={User02Icon} />
                  )}
                </ItemMedia>
                <ItemContent className="min-w-0 gap-2">
                  <div className="flex min-w-0 flex-wrap items-center gap-2">
                    <ItemTitle className="max-w-full">{item.name}</ItemTitle>
                    <Badge variant="secondary">
                      {item.source === "proxy_import" ? "Cookie" : "网页登录"}
                    </Badge>
                  </div>
                  <ItemDescription className="truncate">
                    {item.email || item.accountId}
                  </ItemDescription>
                  <ItemDescription>
                    {item.expiresAt
                      ? `有效至 ${formatDateTime(item.expiresAt)}${
                          item.source === "open_ai_oauth"
                            ? "，到期前自动续期"
                            : ""
                        }`
                      : item.source === "proxy_import"
                        ? "有效期取决于 Cookie 中的访问凭据"
                        : "有效期由 OpenAI 自动管理"}
                  </ItemDescription>
                  <ItemDescription>
                    今日{" "}
                    {formatTokens(
                      usageByAccount.get(item.id)?.tokens.totalTokens ?? 0
                    )}{" "}
                    ·{" "}
                    {usageCostLabel(
                      usageByAccount.get(item.id)
                        ? [usageByAccount.get(item.id)!]
                        : []
                    )}
                  </ItemDescription>
                  <QuotaStatusView quota={item.quota} />
                </ItemContent>
                <ItemActions className="ml-auto w-auto self-start">
                  {item.active ? (
                    <Badge variant="default">
                      <HugeiconsIcon
                        icon={CheckIcon}
                        data-icon="inline-start"
                      />
                      当前账号
                    </Badge>
                  ) : (
                    <Button
                      size="sm"
                      variant="secondary"
                      className="min-w-24"
                      disabled={busy}
                      onClick={() =>
                        setPendingOfficialAction({
                          kind: "activate",
                          account: item,
                        })
                      }
                    >
                      <HugeiconsIcon
                        icon={ArrowUpDownIcon}
                        data-icon="inline-start"
                      />
                      切换
                    </Button>
                  )}
                  <DropdownMenu>
                    <DropdownMenuTrigger
                      render={
                        <Button
                          size="icon-sm"
                          variant="ghost"
                          disabled={busy}
                          aria-label={`${item.name} 的更多操作`}
                          title="更多操作"
                        />
                      }
                    >
                      <HugeiconsIcon
                        icon={More01Icon}
                        data-icon="inline-start"
                      />
                    </DropdownMenuTrigger>
                    <DropdownMenuContent align="end" className="w-40">
                      <DropdownMenuGroup>
                        <DropdownMenuItem
                          onClick={() =>
                            void run(`official:quota:${item.id}`, async () => {
                              const quota = await runQuotaRefresh(item.id, () =>
                                call("refresh_official_account_quota", {
                                  accountId: item.id,
                                })
                              )
                              if (quota.status === "success") {
                                notify.success("OpenAI 额度已更新")
                              } else {
                                notify.warning(
                                  "OpenAI 额度未更新",
                                  quota.error ?? "OpenAI 暂未返回额度。"
                                )
                              }
                            })
                          }
                        >
                          {pendingTask === `official:quota:${item.id}` ? (
                            <Spinner />
                          ) : (
                            <HugeiconsIcon icon={Refresh01Icon} />
                          )}
                          刷新额度
                        </DropdownMenuItem>
                        <DropdownMenuItem onClick={() => openUsagePage()}>
                          <HugeiconsIcon icon={Tag01Icon} />
                          价格规则
                        </DropdownMenuItem>
                        <DropdownMenuItem
                          variant="destructive"
                          disabled={item.active}
                          onClick={() =>
                            setPendingOfficialAction({
                              kind: "delete",
                              account: item,
                            })
                          }
                        >
                          <HugeiconsIcon icon={Delete01Icon} />
                          删除账号
                        </DropdownMenuItem>
                      </DropdownMenuGroup>
                    </DropdownMenuContent>
                  </DropdownMenu>
                </ItemActions>
              </Item>
            ))}
          </ItemGroup>
        )}
      </section>

      {/* API 服务 */}
      <section className="flex flex-col gap-6">
        <div>
          <h3 className="text-base font-medium">
            API 服务（{providers.length}）
          </h3>
          <p className="text-sm text-muted-foreground">
            管理兼容 OpenAI Responses API 与 Chat Completions API 的服务；Chat
            Completions 服务经本机转换代理接入，请求不出本机。
          </p>
        </div>
        <div className="grid gap-4">
          {!providers.length && (
            <Empty className="min-h-48 border">
              <EmptyHeader>
                <EmptyMedia variant="icon">
                  <HugeiconsIcon icon={BoxesIcon} />
                </EmptyMedia>
                <EmptyTitle>尚未添加 API 服务</EmptyTitle>
                <EmptyDescription>
                  先添加 API 地址，再添加 API Key 并测试连接。
                </EmptyDescription>
              </EmptyHeader>
              <EmptyContent>
                <Button size="sm" onClick={() => setDraft(emptyProvider())}>
                  <HugeiconsIcon icon={Add01Icon} data-icon="inline-start" />
                  添加 API 服务
                </Button>
              </EmptyContent>
            </Empty>
          )}
          {providers.map((provider) => {
            const providerUsage = usageByProvider.get(provider.id)
            const providerTokens = providerUsage?.tokens.totalTokens ?? 0
            return (
              <Card key={provider.id}>
                <CardHeader>
                  <CardTitle className="flex items-center gap-2">
                    {provider.name}
                    {provider.active && (
                      <Badge variant="default">
                        <HugeiconsIcon
                          icon={CheckIcon}
                          data-icon="inline-start"
                        />
                        使用中
                      </Badge>
                    )}
                    {!provider.enabled && (
                      <Badge variant="outline">不可用</Badge>
                    )}
                    {provider.apiType === "chat" && (
                      <Badge variant="secondary">Chat 转换</Badge>
                    )}
                  </CardTitle>
                  <CardDescription className="flex min-w-0 flex-col gap-0.5">
                    <span className="truncate" title={provider.baseUrl}>
                      {provider.baseUrl}
                    </span>
                    <span>
                      {provider.apiType === "chat"
                        ? "Chat Completions API · 由本机转换代理转发"
                        : "Responses API · 由 Codex 直接请求"}
                    </span>
                    {provider.model && (
                      <span className="truncate">
                        默认模型：{provider.model}
                      </span>
                    )}
                    <span>
                      今日 {formatTokens(providerTokens)} ·{" "}
                      {usageCostLabel(providerUsage ? [providerUsage] : [])}
                    </span>
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
                        <HugeiconsIcon
                          icon={More01Icon}
                          data-icon="inline-start"
                        />
                      </DropdownMenuTrigger>
                      <DropdownMenuContent align="end" className="w-44">
                        <DropdownMenuGroup>
                          <DropdownMenuItem onClick={() => setDraft(provider)}>
                            <HugeiconsIcon icon={PencilIcon} />
                            编辑服务
                          </DropdownMenuItem>
                          <DropdownMenuItem
                            disabled={busy || !provider.hasApiKey}
                            onClick={() =>
                              void run(
                                `provider:test:${provider.id}`,
                                async () => {
                                  const result = await call("test_provider", {
                                    id: provider.id,
                                  })
                                  const detail = result.suggestV1
                                    ? `${result.message} 请确认 API 地址以 /v1 结尾。`
                                    : result.message
                                  if (result.ok) {
                                    notify.success("连接测试成功", detail)
                                  } else {
                                    notify.error("连接测试未通过", detail)
                                  }
                                },
                                false
                              )
                            }
                          >
                            {pendingTask === `provider:test:${provider.id}` ? (
                              <Spinner />
                            ) : (
                              <HugeiconsIcon icon={Activity01Icon} />
                            )}
                            测试连接
                          </DropdownMenuItem>
                          <DropdownMenuItem
                            disabled={busy || !provider.hasApiKey}
                            title={
                              provider.hasApiKey
                                ? "重新获取服务端 /models 返回的可用模型"
                                : "请先编辑服务并填写 API Key"
                            }
                            onClick={() =>
                              void run(
                                `provider:models:${provider.id}`,
                                async () => {
                                  const models = await call(
                                    "list_provider_models",
                                    { id: provider.id }
                                  )
                                  notify.success(
                                    `已同步 ${models.length} 个模型`
                                  )
                                }
                              )
                            }
                          >
                            {pendingTask ===
                            `provider:models:${provider.id}` ? (
                              <Spinner />
                            ) : (
                              <HugeiconsIcon icon={Refresh01Icon} />
                            )}
                            同步模型
                          </DropdownMenuItem>
                          <DropdownMenuItem onClick={() => openUsagePage()}>
                            <HugeiconsIcon icon={Tag01Icon} />
                            价格规则
                          </DropdownMenuItem>
                          <DropdownMenuItem
                            variant="destructive"
                            disabled={provider.active || busy}
                            onClick={() =>
                              setPendingDelete({
                                id: provider.id,
                                name: provider.name,
                              })
                            }
                          >
                            <HugeiconsIcon icon={Delete01Icon} />
                            删除服务
                          </DropdownMenuItem>
                        </DropdownMenuGroup>
                      </DropdownMenuContent>
                    </DropdownMenu>
                  </CardAction>
                </CardHeader>
                <CardContent className="flex flex-col gap-4">
                  <Item
                    variant={provider.active ? "muted" : "outline"}
                    className="items-start gap-4 p-4"
                  >
                    <ItemMedia variant="icon">
                      <HugeiconsIcon icon={Key01Icon} />
                    </ItemMedia>
                    <ItemContent>
                      <ItemTitle>
                        API Key
                        {provider.active && (
                          <Badge variant="default">
                            <HugeiconsIcon
                              icon={CheckIcon}
                              data-icon="inline-start"
                            />
                            使用中
                          </Badge>
                        )}
                        {!provider.hasApiKey && (
                          <Badge variant="outline">未填写</Badge>
                        )}
                      </ItemTitle>
                      <ItemDescription>
                        {provider.hasApiKey
                          ? "API Key 已保存在本机，切换到此服务时写入 Codex。"
                          : "编辑服务并填写 API Key 后，即可测试连接或切换使用。"}
                      </ItemDescription>
                      {providerUsage && (
                        <ItemDescription>
                          今日 {formatTokens(providerUsage.tokens.totalTokens)}{" "}
                          · {usageCostLabel([providerUsage])}
                        </ItemDescription>
                      )}
                    </ItemContent>
                    <ItemActions className="ml-auto w-auto">
                      {!provider.active && (
                        <Button
                          size="sm"
                          variant="secondary"
                          className="min-w-24"
                          disabled={busy || !provider.hasApiKey}
                          title={
                            provider.hasApiKey
                              ? "让 Codex 使用此 API 地址和 API Key"
                              : "请先编辑服务并填写 API Key"
                          }
                          onClick={() => setPendingActivation(provider.id)}
                        >
                          {pendingTask ===
                          `provider:activate:${provider.id}` ? (
                            <Spinner data-icon="inline-start" />
                          ) : (
                            <HugeiconsIcon
                              icon={ArrowUpDownIcon}
                              data-icon="inline-start"
                            />
                          )}
                          {pendingTask === `provider:activate:${provider.id}`
                            ? "切换中…"
                            : "使用"}
                        </Button>
                      )}
                    </ItemActions>
                  </Item>
                </CardContent>
              </Card>
            )
          })}
        </div>
      </section>

      {draft && (
        <ProviderEditor
          key={draft.id || "new"}
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
      {proxyLoginOpen && (
        <ProxyLoginDialog
          pending={pendingTask === "proxy:login"}
          onCancel={() => setProxyLoginOpen(false)}
          onLogin={(name, accountId, content) =>
            void run("proxy:login", async () => {
              const imported = await call("import_proxy_account", {
                name,
                accountId,
                content,
              })
              setProxyLoginOpen(false)
              notify.success("Cookie 账号已导入", imported.name)
              try {
                const quota = await runQuotaRefresh(imported.id, () =>
                  call("refresh_official_account_quota", {
                    accountId: imported.id,
                  })
                )
                if (quota.status !== "success") {
                  notify.warning(
                    "Cookie 账号已保存，但额度暂未更新",
                    quota.error ?? "OpenAI 暂未返回额度。"
                  )
                }
              } catch (error) {
                notify.warning("Cookie 账号已保存，但额度暂未更新", error)
              }
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
              登录完成后，Codex 会切换到新账号。已保存的第三方 API 服务和其他
              Codex 设置会保留。
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
              {providers.find((provider) => provider.id === pendingActivation)
                ?.apiType === "chat"
                ? "Codex 将请求本机转换代理，由代理把 Responses API 请求自动转换为该服务的 Chat Completions API 请求；API Key 仍只保存在本机，不经过第三方。"
                : "Codex 将使用此服务的地址和 API Key，并直接请求 Responses API 和模型列表。"}{" "}
              已保存的 OpenAI 账号会保留。
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>取消</AlertDialogCancel>
            <AlertDialogAction
              disabled={busy || !pendingActivation}
              onClick={() => {
                const providerId = pendingActivation
                setPendingActivation(undefined)
                if (!providerId) return
                void run(`provider:activate:${providerId}`, async () => {
                  const repair = await call("activate_provider", {
                    id: providerId,
                  })
                  notify.success("Codex 已切换到所选 API 服务")
                  notifyRepairWarnings(repair)
                })
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
            <AlertDialogTitle>删除这个 API 服务？</AlertDialogTitle>
            <AlertDialogDescription>
              将从本机删除“{pendingDelete?.name ?? ""}”。该服务对应的 API Key
              也会一并删除；服务商网站上的 API Key 不会被撤销。
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
                void run(`delete:provider:${pending.id}`, async () => {
                  await call("delete_provider", { id: pending.id })
                  notify.success("第三方 API 服务已删除")
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

function openUsagePage() {
  window.dispatchEvent(
    new CustomEvent("codex-tools:navigate", { detail: "usage" })
  )
}

/** 会改变用量归属（账号/服务维度）的任务前缀：完成后需要刷新用量数据。 */
const TASKS_REFRESHING_USAGE = [
  "provider:activate:",
  "openai:activate:",
  "openai:delete:",
  "delete:provider:",
  "proxy:login",
  "openai:login",
] as const

function usageCostLabel(rows: UsageRow[]) {
  if (!rows.length) return "—"
  const estimated = rows.filter((row) => row.costStatus === "estimated")
  if (estimated.length) {
    const cost = estimated.reduce(
      (total, row) => total + (row.estimatedCostMicrousd ?? 0),
      0
    )
    return formatUsdMicrousd(cost)
  }
  if (rows.some((row) => row.costStatus === "subscription")) return "套餐统计"
  if (rows.some((row) => row.costStatus === "unpriced")) return "未配置价格"
  if (rows.some((row) => row.costStatus === "unattributed")) return "未归属"
  if (rows.some((row) => row.costStatus === "partial")) return "部分数据"
  return "$0.00"
}

function ProvidersLoading() {
  return (
    <div className="flex flex-col gap-6" role="status" aria-live="polite">
      <div className="flex flex-col gap-2">
        <Skeleton className="h-7 w-24" />
        <Skeleton className="h-4 w-96" />
      </div>
      <Skeleton className="h-48 w-full" />
      <Skeleton className="h-64 w-full" />
    </div>
  )
}
