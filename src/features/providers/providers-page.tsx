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
} from "@hugeicons/core-free-icons"
import { HugeiconsIcon } from "@hugeicons/react"

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
import { Spinner } from "@/components/ui/spinner"
import { notify } from "@/lib/feedback"
import { call } from "@/lib/ipc"
import { notifyRepairWarnings } from "@/lib/repair-feedback"
import { formatDateTime } from "@/lib/time"
import {
  emptyAccount,
  emptyProvider,
  type Account,
  type OfficialAccountView,
  type PageProps,
  type Provider,
} from "@/types"

import { QuotaStatusView } from "./quota-status"
import { AccountEditor } from "./account-editor"
import { ProviderEditor } from "./provider-editor"
import { ProxyLoginDialog } from "./proxy-login-dialog"
import { useDeviceAuthorizationPolling } from "./use-device-auth"
import { taskFailureTitle } from "./provider-utils"

export default function ProvidersPage({ active }: PageProps) {
  const [providers, setProviders] = useState<Provider[]>([])
  const [accounts, setAccounts] = useState<Account[]>([])
  const [officialAccounts, setOfficialAccounts] = useState<
    OfficialAccountView[]
  >([])
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
                    <HugeiconsIcon
                      icon={Refresh01Icon}
                      data-icon="inline-start"
                    />
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
            </>
          }
        />
        <div className="flex flex-col gap-3">
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
            <Card>
              <CardContent>
                <Empty className="min-h-48">
                  <EmptyHeader>
                    <EmptyMedia variant="icon">
                      <HugeiconsIcon icon={Key01Icon} />
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
                      <HugeiconsIcon
                        icon={Key01Icon}
                        data-icon="inline-start"
                      />
                      导入 Cookie
                    </Button>
                    <Button
                      size="sm"
                      disabled={busy || Boolean(deviceAuthorization)}
                      onClick={() => setConfirmOpenAiLogin(true)}
                    >
                      <HugeiconsIcon
                        icon={Login01Icon}
                        data-icon="inline-start"
                      />
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
                              <HugeiconsIcon icon={Refresh01Icon} />
                            )}
                            刷新额度
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
              <HugeiconsIcon icon={Add01Icon} data-icon="inline-start" />
              添加服务
            </Button>
          }
        />
        <div className="grid gap-3">
          {!providers.length && (
            <Empty className="min-h-48 border">
              <EmptyHeader>
                <EmptyMedia variant="icon">
                  <HugeiconsIcon icon={BoxesIcon} />
                </EmptyMedia>
                <EmptyTitle>尚未添加第三方 API</EmptyTitle>
                <EmptyDescription>
                  先添加服务地址，再添加 API Key 并测试连接。
                </EmptyDescription>
              </EmptyHeader>
              <EmptyContent>
                <Button size="sm" onClick={() => setDraft(emptyProvider())}>
                  <HugeiconsIcon icon={Add01Icon} data-icon="inline-start" />
                  添加服务
                </Button>
              </EmptyContent>
            </Empty>
          )}
          {providers.map((provider) => {
            const linked = accountsByProvider.get(provider.id) ?? []
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
                  </CardTitle>
                  <CardDescription className="flex min-w-0 flex-col gap-0.5">
                    <span className="truncate" title={provider.baseUrl}>
                      {provider.baseUrl}
                    </span>
                    <span>Responses API · 由 Codex 直接请求</span>
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
                            onClick={() =>
                              setAccount(emptyAccount(provider.id))
                            }
                          >
                            <HugeiconsIcon icon={Key01Icon} />
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
                            <HugeiconsIcon icon={Delete01Icon} />
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
                          <HugeiconsIcon
                            icon={Key01Icon}
                            data-icon="inline-start"
                          />
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
                            <HugeiconsIcon icon={Key01Icon} />
                          </ItemMedia>
                          <ItemContent>
                            <ItemTitle>
                              {item.name}
                              {item.active && (
                                <Badge variant="default">
                                  <HugeiconsIcon
                                    icon={CheckIcon}
                                    data-icon="inline-start"
                                  />
                                  使用中
                                </Badge>
                              )}
                            </ItemTitle>
                            <ItemDescription>
                              API Key 保存在本机，切换到此服务时写入 Codex。
                            </ItemDescription>
                          </ItemContent>
                          <ItemActions className="ml-auto w-auto">
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
                                  <HugeiconsIcon
                                    icon={ArrowUpDownIcon}
                                    data-icon="inline-start"
                                  />
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
                                <HugeiconsIcon
                                  icon={More01Icon}
                                  data-icon="inline-start"
                                />
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
                                      <HugeiconsIcon icon={Activity01Icon} />
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
