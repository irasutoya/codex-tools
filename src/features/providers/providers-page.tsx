import { useCallback, useEffect, useMemo, useRef, useState } from "react"
import {
  Activity,
  ArrowRightLeft,
  Boxes,
  Check,
  Copy,
  ExternalLink,
  KeyRound,
  LogIn,
  MoreHorizontal,
  Pencil,
  Plus,
  RefreshCw,
  Save,
  Tags,
  Trash2,
  TriangleAlert,
  UserRound,
} from "lucide-react"

import { ErrorDetails } from "@/components/error-details"
import { SectionHeader } from "@/components/page-header"
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
import { Textarea } from "@/components/ui/textarea"
import { notify } from "@/lib/feedback"
import { call } from "@/lib/ipc"
import { refreshCoordinator, usePageRefresh } from "@/lib/refresh-coordinator"
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
  type UsageOverview,
  type UsageRow,
} from "@/types"

import { QuotaStatusView } from "./quota-status"
import {
  formatTokens,
  formatUsdMicrousd,
  getLocalRange,
} from "../usage/usage-format"

const MAX_DISPLAY_NAME_LENGTH = 100
const MAX_API_URL_LENGTH = 2_048
const MAX_API_KEY_LENGTH = 65_536
const MAX_ACCOUNT_ID_LENGTH = 512
const MAX_COOKIE_CREDENTIAL_LENGTH = 262_144
const accountTimestampFormatter = new Intl.DateTimeFormat("zh-CN", {
  dateStyle: "medium",
  timeStyle: "short",
})

export default function ProvidersPage({ active }: PageProps) {
  const refreshSignal = usePageRefresh("providers")
  const [providers, setProviders] = useState<Provider[]>([])
  const [accounts, setAccounts] = useState<Account[]>([])
  const [officialAccounts, setOfficialAccounts] = useState<
    OfficialAccountView[]
  >([])
  const [usage, setUsage] = useState<UsageOverview>()
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
  const [proxyLoginOpen, setProxyLoginOpen] = useState(false)
  const [pendingTask, setPendingTask] = useState<string>()
  const [overviewLoaded, setOverviewLoaded] = useState(false)
  const [overviewError, setOverviewError] = useState<string>()
  const [pendingDelete, setPendingDelete] = useState<
    | { kind: "provider"; id: string; name: string }
    | { kind: "account"; id: string; name: string }
  >()
  const running = useRef(false)
  const lastRefreshRevision = useRef<number | undefined>(undefined)
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
      setAccounts(overview.accounts)
      setOfficialAccounts(overview.officialAccounts)
      setOverviewLoaded(true)
      setOverviewError(undefined)
    } catch (error) {
      setOverviewError(String(error))
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
  useEffect(() => {
    if (!active || lastRefreshRevision.current === refreshSignal.revision) {
      return
    }
    lastRefreshRevision.current = refreshSignal.revision
    const timeout = window.setTimeout(() => {
      void load().catch(() => undefined)
      void loadUsage(true)
    }, 0)
    return () => window.clearTimeout(timeout)
  }, [active, load, loadUsage, refreshSignal.revision])

  const activeOfficialAccount = officialAccounts.find(
    (account) => account.active
  )
  const refreshActiveQuota = useCallback(async () => {
    if (!activeOfficialAccount) return false
    try {
      await call("refresh_official_account_quota", {
        accountId: activeOfficialAccount.id,
      })
      await load()
      refreshCoordinator.invalidate(["dashboard"])
      return true
    } catch {
      // Automatic quota refresh keeps the last known result on failure.
      return false
    }
  }, [activeOfficialAccount, load])

  useEffect(() => {
    if (
      !active ||
      !activeOfficialAccount ||
      document.visibilityState !== "visible"
    ) {
      return
    }
    let timer: number | undefined
    const lastAttempt = Math.max(
      activeOfficialAccount.quota.fetchedAt ?? 0,
      activeOfficialAccount.quota.lastAttemptAt ?? 0
    )
    const delay = Math.max(1_000, 5 * 60_000 - (Date.now() - lastAttempt))
    const schedule = (wait: number) => {
      timer = window.setTimeout(() => {
        timer = undefined
        void refreshActiveQuota().then((success) => {
          if (!success && document.visibilityState === "visible") {
            schedule(30_000)
          }
        })
      }, wait)
    }
    schedule(delay)
    return () => window.clearTimeout(timer)
  }, [active, activeOfficialAccount, refreshActiveQuota])

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
              void call("refresh_official_account_quota", {
                accountId: result.account.id,
              })
                .catch((error) =>
                  notify.warning("登录成功，但额度暂未更新", error)
                )
                .finally(() => {
                  refreshCoordinator.invalidate(["dashboard", "settings"])
                  return load().catch((error) =>
                    notify.warning("登录已完成，但无法读取最新账号列表", error)
                  )
                })
            })
            .catch((error) => {
              if (cancelled) return
              if (!pollErrorShown) {
                pollErrorShown = true
                notify.warning("暂时无法确认登录结果，程序将自动重试", error)
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
          notify.warning("操作已完成，但无法读取最新列表", error)
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
        <SectionHeader
          id="openai-title"
          title="OpenAI 账号"
          description="统一管理网页登录和 Cookie 账号；额度只显示 OpenAI 接口实际返回的结果。"
          actions={
            <>
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
                    <RefreshCw data-icon="inline-start" />
                  )}
                  刷新全部额度
                </Button>
              )}
              <Button
                size="sm"
                variant="outline"
                disabled={busy}
                onClick={() => setProxyLoginOpen(true)}
              >
                <KeyRound data-icon="inline-start" />
                导入 Cookie
              </Button>
              <Button
                size="sm"
                disabled={busy || Boolean(deviceAuthorization)}
                onClick={() => setConfirmOpenAiLogin(true)}
              >
                <LogIn data-icon="inline-start" />
                网页登录
              </Button>
            </>
          }
        />
        <div className="flex flex-col gap-3">
          {deviceAuthorization && (
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
                在打开的 OpenAI 页面输入此代码。完成后会自动刷新；代码有效期至
                {formatTimestamp(deviceAuthorization.expiresAt)}。
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
            </Alert>
          )}
          {!officialAccounts.length ? (
            <Card>
              <CardContent>
                <Empty className="min-h-48">
                  <EmptyHeader>
                    <EmptyMedia variant="icon">
                      <KeyRound />
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
                      <KeyRound data-icon="inline-start" />
                      导入 Cookie
                    </Button>
                    <Button
                      size="sm"
                      disabled={busy || Boolean(deviceAuthorization)}
                      onClick={() => setConfirmOpenAiLogin(true)}
                    >
                      <LogIn data-icon="inline-start" />
                      网页登录
                    </Button>
                  </EmptyContent>
                </Empty>
              </CardContent>
            </Card>
          ) : (
            <ItemGroup className="gap-3">
              {officialAccounts.map((item) => (
                <Item
                  key={item.id}
                  variant={item.active ? "muted" : "outline"}
                  className="items-start gap-4 p-4"
                >
                  <ItemMedia variant="icon">
                    {item.source === "proxy_import" ? (
                      <KeyRound />
                    ) : (
                      <UserRound />
                    )}
                  </ItemMedia>
                  <ItemContent className="min-w-0 gap-2">
                    <div className="flex min-w-0 flex-wrap items-center gap-2">
                      <ItemTitle className="max-w-full">{item.name}</ItemTitle>
                      <Badge variant="secondary">
                        {item.source === "proxy_import" ? "Cookie" : "网页登录"}
                      </Badge>
                    </div>
                    <div className="flex min-w-0 flex-col gap-0.5">
                      <ItemDescription className="truncate">
                        {item.email || item.accountId}
                      </ItemDescription>
                      <ItemDescription>
                        {item.expiresAt
                          ? `有效至 ${formatTimestamp(item.expiresAt)}${
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
                    </div>
                    <QuotaStatusView quota={item.quota} />
                  </ItemContent>
                  <ItemActions className="ml-auto w-auto justify-end self-start">
                    {item.active ? (
                      <Badge variant="default">
                        <Check data-icon="inline-start" />
                        当前账号
                      </Badge>
                    ) : (
                      <Button
                        size="sm"
                        variant="secondary"
                        disabled={busy}
                        onClick={() =>
                          setPendingOfficialAction({
                            kind: "activate",
                            account: item,
                          })
                        }
                      >
                        <ArrowRightLeft data-icon="inline-start" />
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
                        <MoreHorizontal data-icon="inline-start" />
                      </DropdownMenuTrigger>
                      <DropdownMenuContent align="end" className="w-40">
                        <DropdownMenuGroup>
                          <DropdownMenuItem
                            onClick={() =>
                              void run(
                                `official:quota:${item.id}`,
                                async () => {
                                  const quota = await call(
                                    "refresh_official_account_quota",
                                    { accountId: item.id }
                                  )
                                  if (quota.status === "success") {
                                    notify.success("OpenAI 额度已更新")
                                  } else {
                                    notify.warning(
                                      "OpenAI 额度未更新",
                                      quota.error ?? "OpenAI 暂未返回额度。"
                                    )
                                  }
                                }
                              )
                            }
                          >
                            {pendingTask === `official:quota:${item.id}` ? (
                              <Spinner />
                            ) : (
                              <RefreshCw />
                            )}
                            刷新额度
                          </DropdownMenuItem>
                          <DropdownMenuItem onClick={() => openUsagePage()}>
                            <Tags />
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
                            <Trash2 />
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
        </div>
      </section>
      <section
        className="flex flex-col gap-4"
        aria-labelledby="custom-api-title"
      >
        <SectionHeader
          id="custom-api-title"
          title="第三方 API"
          description="管理兼容 OpenAI Responses API 的服务；请求由 Codex 直接发送，本应用不参与转发。"
          actions={
            <Button size="sm" onClick={() => setDraft(emptyProvider())}>
              <Plus data-icon="inline-start" />
              添加服务
            </Button>
          }
        />
        <div className="grid gap-3">
          {!providers.length && (
            <Empty className="min-h-48 border">
              <EmptyHeader>
                <EmptyMedia variant="icon">
                  <Boxes />
                </EmptyMedia>
                <EmptyTitle>尚未添加第三方 API</EmptyTitle>
                <EmptyDescription>
                  先添加服务地址，再添加 API Key 并测试连接。
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
            const linkedUsage = linked
              .map((account) => usageByAccount.get(account.id))
              .filter((row): row is UsageRow => Boolean(row))
            const providerTokens = linkedUsage.reduce(
              (total, row) => total + row.tokens.totalTokens,
              0
            )
            return (
              <Card key={provider.id}>
                <CardHeader>
                  <CardTitle className="flex items-center gap-2">
                    {provider.name}
                    {provider.active && (
                      <Badge variant="default">
                        <Check data-icon="inline-start" />
                        使用中
                      </Badge>
                    )}
                    {!provider.enabled && (
                      <Badge variant="outline">不可用</Badge>
                    )}
                  </CardTitle>
                  <CardDescription className="flex min-w-0 flex-col gap-0.5">
                    <span className="truncate" title={provider.baseUrl}>
                      {provider.baseUrl}
                    </span>
                    <span>Responses API · 由 Codex 直接请求</span>
                    <span>
                      今日 {formatTokens(providerTokens)} ·{" "}
                      {usageCostLabel(linkedUsage)}
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
                        <MoreHorizontal data-icon="inline-start" />
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
                          <DropdownMenuItem onClick={() => openUsagePage()}>
                            <Tags />
                            价格规则
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
                          添加后可测试连接；切换后 Codex 才会使用此密钥。
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
                          className="items-start gap-3 p-3"
                        >
                          <ItemMedia variant="icon">
                            <KeyRound />
                          </ItemMedia>
                          <ItemContent>
                            <ItemTitle>
                              {item.name}
                              {item.active && (
                                <Badge variant="default">
                                  <Check data-icon="inline-start" />
                                  使用中
                                </Badge>
                              )}
                            </ItemTitle>
                            <ItemDescription>
                              API Key 保存在本机，切换到此服务时写入 Codex。
                            </ItemDescription>
                            <ItemDescription>
                              今日{" "}
                              {formatTokens(
                                usageByAccount.get(item.id)?.tokens
                                  .totalTokens ?? 0
                              )}{" "}
                              ·{" "}
                              {usageCostLabel(
                                usageByAccount.get(item.id)
                                  ? [usageByAccount.get(item.id)!]
                                  : []
                              )}
                            </ItemDescription>
                          </ItemContent>
                          <ItemActions className="ml-auto w-auto justify-end">
                            {!item.active && (
                              <Button
                                size="sm"
                                variant="secondary"
                                disabled={busy}
                                title="让 Codex 使用此 API 地址和 API Key"
                                onClick={() => {
                                  setPendingActivation({
                                    providerId: provider.id,
                                    accountId: item.id,
                                  })
                                }}
                              >
                                {pendingTask ===
                                `account:activate:${item.id}` ? (
                                  <Spinner data-icon="inline-start" />
                                ) : (
                                  <ArrowRightLeft data-icon="inline-start" />
                                )}
                                {pendingTask === `account:activate:${item.id}`
                                  ? "切换中…"
                                  : "使用"}
                              </Button>
                            )}
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
                                <MoreHorizontal data-icon="inline-start" />
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
                                            notify.error(
                                              "连接测试未通过",
                                              detail
                                            )
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
                const quota = await call("refresh_official_account_quota", {
                  accountId: imported.id,
                })
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
              Codex 将使用此服务的地址和 API Key，并直接请求 Responses API
              和模型列表。已保存的 OpenAI 账号会保留。
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

function formatTimestamp(value: number) {
  const date = new Date(epochMilliseconds(value))
  if (Number.isNaN(date.getTime())) return "时间未知"
  return accountTimestampFormatter.format(date)
}

function openUsagePage() {
  window.dispatchEvent(
    new CustomEvent("codex-tools:navigate", { detail: "usage" })
  )
}

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
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>
            {value.id ? "编辑 API 服务" : "添加 API 服务"}
          </DialogTitle>
          <DialogDescription>
            填写服务名称和 Responses API 地址。保存后再添加 API Key。
          </DialogDescription>
        </DialogHeader>
        <FieldGroup>
          <Field data-disabled={busy}>
            <FieldLabel htmlFor="provider-name">服务名称</FieldLabel>
            <Input
              id="provider-name"
              autoFocus
              disabled={busy}
              required
              maxLength={MAX_DISPLAY_NAME_LENGTH}
              placeholder="例如：公司 API"
              value={value.name}
              onChange={(event) =>
                onChange({ ...value, name: event.target.value })
              }
            />
            <FieldDescription>最多 100 个字符。</FieldDescription>
          </Field>
          <Field data-disabled={busy}>
            <FieldLabel htmlFor="provider-base-url">API 地址</FieldLabel>
            <Input
              id="provider-base-url"
              type="url"
              disabled={busy}
              required
              maxLength={MAX_API_URL_LENGTH}
              placeholder="https://api.example.com/v1"
              value={value.baseUrl}
              onChange={(event) =>
                onChange({ ...value, baseUrl: event.target.value })
              }
            />
            <FieldDescription>
              最多 2,048 个字符。填写服务商提供的 API 根地址，通常以 /v1 结尾。
            </FieldDescription>
          </Field>
          <Field orientation="horizontal" data-disabled={busy}>
            <FieldContent>
              <FieldTitle>启用此服务</FieldTitle>
              <FieldDescription>
                关闭后仍会保留配置，但不能切换使用。
              </FieldDescription>
            </FieldContent>
            <Switch
              id="provider-enabled"
              aria-label="启用此服务"
              disabled={busy}
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
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>添加 API Key</DialogTitle>
          <DialogDescription>
            可以为同一个第三方服务保存多个 API Key，并随时切换。
          </DialogDescription>
        </DialogHeader>
        <FieldGroup>
          <Field data-disabled={pending}>
            <FieldLabel htmlFor="account-name">密钥名称</FieldLabel>
            <Input
              id="account-name"
              autoFocus
              disabled={pending}
              required
              maxLength={MAX_DISPLAY_NAME_LENGTH}
              placeholder="例如：个人密钥"
              value={value.name}
              onChange={(event) =>
                onChange({ ...value, name: event.target.value })
              }
            />
            <FieldDescription>最多 100 个字符。</FieldDescription>
          </Field>
          <Field data-disabled={pending}>
            <FieldLabel htmlFor="account-api-key">API Key</FieldLabel>
            <Input
              id="account-api-key"
              required
              disabled={pending}
              type="password"
              autoComplete="off"
              maxLength={MAX_API_KEY_LENGTH}
              placeholder="sk-…"
              value={value.apiKey ?? ""}
              onChange={(event) =>
                onChange({
                  ...value,
                  apiKey: event.target.value,
                  authKind: "api_key",
                })
              }
            />
            <FieldDescription>
              最多 65,536 个字符。密钥保存在本机；切换到此服务时写入 Codex 的
              auth.json。
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

function ProxyLoginDialog({
  pending,
  onCancel,
  onLogin,
}: {
  pending: boolean
  onCancel: () => void
  onLogin: (
    name: string | undefined,
    accountId: string | undefined,
    content: string
  ) => void
}) {
  const [name, setName] = useState("")
  const [accountId, setAccountId] = useState("")
  const [hasContent, setHasContent] = useState(false)
  const contentRef = useRef<HTMLTextAreaElement>(null)

  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open && !pending) onCancel()
      }}
    >
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>导入 Cookie 账号</DialogTitle>
          <DialogDescription>
            粘贴 Cookie Token 或单账号 JSON。这里不会读取浏览器
            Cookie；导入后会尝试向 OpenAI 查询 5H/7D 额度。
          </DialogDescription>
        </DialogHeader>
        <FieldGroup>
          <Field data-disabled={pending}>
            <FieldLabel htmlFor="proxy-account-name">账号名称</FieldLabel>
            <Input
              id="proxy-account-name"
              autoFocus
              disabled={pending}
              maxLength={MAX_DISPLAY_NAME_LENGTH}
              placeholder="可选，例如：工作 Cookie 账号"
              value={name}
              onChange={(event) => setName(event.target.value)}
            />
          </Field>
          <Field data-disabled={pending}>
            <FieldLabel htmlFor="proxy-account-id">
              ChatGPT Account ID
            </FieldLabel>
            <Input
              id="proxy-account-id"
              autoComplete="off"
              disabled={pending}
              maxLength={MAX_ACCOUNT_ID_LENGTH}
              placeholder="可选；团队号查询额度时可能需要"
              value={accountId}
              onChange={(event) => setAccountId(event.target.value)}
            />
            <FieldDescription>
              最多 512 个字符。个人账号通常留空；单账号 JSON 已包含 accountId
              时也可留空。
            </FieldDescription>
          </Field>
          <Field data-disabled={pending}>
            <FieldLabel htmlFor="proxy-account-content">
              Cookie Token / 单账号 JSON
            </FieldLabel>
            <Textarea
              ref={contentRef}
              id="proxy-account-content"
              className="field-sizing-fixed h-40 max-h-40 min-h-40 max-w-full resize-none overflow-x-hidden overflow-y-auto font-mono text-xs break-all"
              autoComplete="off"
              disabled={pending}
              spellCheck={false}
              wrap="soft"
              maxLength={MAX_COOKIE_CREDENTIAL_LENGTH}
              placeholder='粘贴 at-…、accessToken，或包含 "access_token" / "refresh_token" 的单账号 JSON'
              onInput={(event) =>
                setHasContent(/\S/.test(event.currentTarget.value))
              }
            />
            <FieldDescription>
              最多 262,144 个字符。原始 JSON
              不会保存；程序只提取登录所需字段，并将凭据写入本机应用数据文件。
            </FieldDescription>
          </Field>
        </FieldGroup>
        <DialogFooter>
          <Button variant="outline" disabled={pending} onClick={onCancel}>
            取消
          </Button>
          <Button
            disabled={pending || !hasContent}
            onClick={() => {
              const content = contentRef.current?.value ?? ""
              if (!content.trim()) return
              onLogin(
                name.trim() || undefined,
                accountId.trim() || undefined,
                content
              )
            }}
          >
            {pending ? (
              <Spinner data-icon="inline-start" />
            ) : (
              <LogIn data-icon="inline-start" />
            )}
            {pending ? "正在导入…" : "导入并登录"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
