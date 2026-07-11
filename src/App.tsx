import { useCallback, useEffect, useMemo, useState } from "react"
import { save } from "@tauri-apps/plugin-dialog"
import { openUrl } from "@tauri-apps/plugin-opener"
import {
  Activity,
  Copy,
  Database,
  Eye,
  EyeOff,
  ExternalLink,
  FileArchive,
  Gauge,
  KeyRound,
  Network,
  Plus,
  RefreshCw,
  Server,
  Settings,
  ShieldCheck,
  Trash2,
  UserRound,
} from "lucide-react"
import { toast } from "sonner"
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
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
  CardDescription,
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
  InputGroup,
  InputGroupAddon,
  InputGroupButton,
  InputGroupInput,
} from "@/components/ui/input-group"
import {
  Sidebar,
  SidebarContent,
  SidebarFooter,
  SidebarGroup,
  SidebarGroupContent,
  SidebarGroupLabel,
  SidebarHeader,
  SidebarInset,
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
  SidebarProvider,
  SidebarTrigger,
} from "@/components/ui/sidebar"
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table"
import { Textarea } from "@/components/ui/textarea"
import { Switch } from "@/components/ui/switch"
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group"
import { TooltipProvider } from "@/components/ui/tooltip"
import { Toaster } from "@/components/ui/sonner"
import {
  blankAccount,
  blankProvider,
  commaList,
  maskKey,
  optionalNumber,
  parseHeaders,
} from "@/features/providers/model"
import { call } from "@/lib/tauri"
import type {
  Account,
  AuthAccount,
  Dashboard,
  FetchedModel,
  Page,
  Protocol,
  Provider,
  RepairScan,
  RouteConsole,
  Session,
} from "@/types"
const nav: [Page, string, React.ComponentType][] = [
  ["dashboard", "概览", Gauge],
  ["providers", "Provider 与账号", Server],
  ["sessions", "会话历史", FileArchive],
  ["repair", "数据库修复", Database],
  ["routes", "本地路由", Network],
  ["settings", "诊断与设置", Settings],
]

export default function App() {
  const [page, setPage] = useState<Page>("dashboard")
  const [dashboard, setDashboard] = useState<Dashboard>()
  const [providers, setProviders] = useState<Provider[]>([])
  const [accounts, setAccounts] = useState<Account[]>([])
  const [sessions, setSessions] = useState<Session[]>([])
  const [authAccounts, setAuthAccounts] = useState<AuthAccount[]>([])
  const [route, setRoute] = useState<RouteConsole>()
  const [scan, setScan] = useState<RepairScan>()
  const [editingProvider, setEditingProvider] = useState<Provider>()
  const [editingAccount, setEditingAccount] = useState<Account>()
  const [activating, setActivating] = useState<{
    provider?: Provider
    account: Account | AuthAccount
    official?: boolean
  }>()
  const [deletingProvider, setDeletingProvider] = useState<Provider>()
  const [deletingAccount, setDeletingAccount] = useState<Account>()
  const [deletingSession, setDeletingSession] = useState<Session>()
  const [busy, setBusy] = useState(false)

  const reload = useCallback(async () => {
    try {
      const [
        nextDashboard,
        nextProviders,
        nextAccounts,
        nextSessions,
        nextScan,
        nextAuthAccounts,
        nextRoute,
      ] = await Promise.all([
        call<Dashboard>("get_dashboard"),
        call<Provider[]>("list_providers"),
        call<Account[]>("list_provider_accounts"),
        call<Session[]>("list_sessions"),
        call<RepairScan>("scan_codex_data"),
        call<AuthAccount[]>("list_auth_accounts"),
        call<RouteConsole>("get_route_console"),
      ])
      setDashboard(nextDashboard)
      setProviders(nextProviders)
      setAccounts(nextAccounts)
      setSessions(nextSessions)
      setScan(nextScan)
      setAuthAccounts(nextAuthAccounts)
      setRoute(nextRoute)
    } catch (error) {
      toast.error(String(error))
    }
  }, [])
  useEffect(() => {
    const task = window.setTimeout(() => void reload(), 0)
    return () => window.clearTimeout(task)
  }, [reload])

  const officialAccounts = useMemo(() => authAccounts, [authAccounts])
  const saveProvider = async () => {
    if (!editingProvider) return
    setBusy(true)
    try {
      await call("save_provider", { provider: editingProvider })
      toast.success("Provider 已保存")
      setEditingProvider(undefined)
      await reload()
    } catch (error) {
      toast.error(String(error))
    } finally {
      setBusy(false)
    }
  }
  const saveAccount = async () => {
    if (!editingAccount) return
    setBusy(true)
    try {
      await call("save_provider_account", { account: editingAccount })
      toast.success("账号已保存")
      setEditingAccount(undefined)
      await reload()
    } catch (error) {
      toast.error(String(error))
    } finally {
      setBusy(false)
    }
  }
  const testAccount = async (provider: Provider, account: Account) => {
    setBusy(true)
    try {
      const result = await call<{
        ok: boolean
        status: number
        message: string
        suggestV1: boolean
      }>("test_provider", { id: provider.id, accountId: account.id })
      if (result.ok) toast.success(`连接成功（HTTP ${result.status}）`)
      else
        toast.error(
          `${result.message}${result.suggestV1 ? "；Base URL 可能缺少 /v1" : ""}`
        )
    } catch (error) {
      toast.error(String(error))
    } finally {
      setBusy(false)
    }
  }
  const activate = async () => {
    if (!activating) return
    setBusy(true)
    try {
      const result = activating.official
        ? await call<{ rowsUpdated: number }>("activate_openai_account", {
            id: activating.account.id,
          })
        : await call<{ rowsUpdated: number }>("activate_provider", {
            id: activating.provider?.id,
            accountId: activating.account.id,
            force: false,
          })
      toast.success(`切换完成，统一了 ${result.rowsUpdated} 条会话数据`)
      setActivating(undefined)
      await reload()
    } catch (error) {
      toast.error(String(error))
    } finally {
      setBusy(false)
    }
  }
  const repair = async () => {
    if (!scan) return
    setBusy(true)
    try {
      const result = await call<{ rowsUpdated: number }>("repair_codex_data", {
        operationId: scan.operationId,
      })
      toast.success(`修复完成，更新 ${result.rowsUpdated} 行`)
      await reload()
    } catch (error) {
      toast.error(String(error))
    } finally {
      setBusy(false)
    }
  }
  const exportOne = async (session: Session) => {
    const target = await save({
      defaultPath: `${session.title || session.id}.md`,
      filters: [{ name: "Markdown", extensions: ["md"] }],
    })
    if (!target) return
    try {
      await call("export_sessions", { ids: [session.id], target })
      toast.success("导出完成")
    } catch (error) {
      toast.error(String(error))
    }
  }
  const removeSession = async () => {
    if (!deletingSession) return
    try {
      await call("delete_sessions_permanently", { ids: [deletingSession.id] })
      toast.success("会话已永久删除")
      setDeletingSession(undefined)
      await reload()
    } catch (error) {
      toast.error(String(error))
    }
  }
  const removeProvider = async () => {
    if (!deletingProvider) return
    try {
      await call("delete_provider", { id: deletingProvider.id })
      toast.success("Provider 和所属账号已删除")
      setDeletingProvider(undefined)
      await reload()
    } catch (error) {
      toast.error(String(error))
    }
  }
  const removeAccount = async () => {
    if (!deletingAccount) return
    try {
      await call("delete_provider_account", { id: deletingAccount.id })
      toast.success("账号已删除")
      setDeletingAccount(undefined)
      await reload()
    } catch (error) {
      toast.error(String(error))
    }
  }

  return (
    <TooltipProvider>
      <SidebarProvider>
        <Sidebar>
          <SidebarHeader>
            <div className="flex items-center gap-3 p-2">
              <div className="flex size-9 items-center justify-center rounded-lg bg-primary text-primary-foreground">
                <Activity />
              </div>
              <div>
                <div className="font-medium">Codex Tools</div>
                <div className="text-xs text-muted-foreground">
                  账号切换与会话统一
                </div>
              </div>
            </div>
          </SidebarHeader>
          <SidebarContent>
            <SidebarGroup>
              <SidebarGroupLabel>工作台</SidebarGroupLabel>
              <SidebarGroupContent>
                <SidebarMenu>
                  {nav.map(([id, label, Icon]) => (
                    <SidebarMenuItem key={id}>
                      <SidebarMenuButton
                        isActive={page === id}
                        onClick={() => setPage(id)}
                      >
                        <Icon />
                        <span>{label}</span>
                      </SidebarMenuButton>
                    </SidebarMenuItem>
                  ))}
                </SidebarMenu>
              </SidebarGroupContent>
            </SidebarGroup>
          </SidebarContent>
          <SidebarFooter>
            <Alert>
              <KeyRound />
              <AlertTitle>明文凭据</AlertTitle>
              <AlertDescription>
                API Key
                和官方登录快照以明文保存在本机数据库中，请勿分享数据库文件。
              </AlertDescription>
            </Alert>
          </SidebarFooter>
        </Sidebar>
        <SidebarInset>
          <header className="flex h-14 items-center gap-3 border-b px-5">
            <SidebarTrigger />
            <h1 className="font-medium">
              {nav.find(([id]) => id === page)?.[1]}
            </h1>
            <Button
              className="ml-auto"
              variant="outline"
              size="sm"
              onClick={() => void reload()}
            >
              <RefreshCw data-icon="inline-start" />
              刷新
            </Button>
          </header>
          <main className="flex flex-1 flex-col gap-5 p-6">
            {page === "dashboard" && (
              <DashboardPage
                dashboard={dashboard}
                officialCount={officialAccounts.length}
              />
            )}
            {page === "providers" && (
              <ProvidersPage
                providers={providers}
                accounts={accounts}
                officialAccounts={officialAccounts}
                busy={busy}
                onNew={() => setEditingProvider({ ...blankProvider })}
                onEdit={setEditingProvider}
                onDelete={setDeletingProvider}
                onNewAccount={(id) => setEditingAccount(blankAccount(id))}
                onEditAccount={setEditingAccount}
                onDeleteAccount={setDeletingAccount}
                onTest={testAccount}
                onActivate={(provider, account) =>
                  setActivating({ provider, account })
                }
                onActivateOfficial={(account) =>
                  setActivating({ account, official: true })
                }
                onBusy={setBusy}
                onReload={reload}
              />
            )}
            {page === "sessions" && (
              <SessionsPage
                sessions={sessions}
                onExport={exportOne}
                onDelete={setDeletingSession}
              />
            )}
            {page === "repair" && (
              <RepairPage scan={scan} busy={busy} onRepair={repair} />
            )}
            {page === "routes" && (
              <RouteConsolePage route={route} onReload={reload} />
            )}
            {page === "settings" && <SettingsPage dashboard={dashboard} />}
          </main>
        </SidebarInset>
        <ProviderDialog
          value={editingProvider}
          busy={busy}
          onChange={setEditingProvider}
          onSave={saveProvider}
        />
        <AccountDialog
          value={editingAccount}
          busy={busy}
          onChange={setEditingAccount}
          onSave={saveAccount}
        />
        <AlertDialog
          open={!!activating}
          onOpenChange={(open) => !open && setActivating(undefined)}
        >
          <AlertDialogContent>
            <AlertDialogHeader>
              <AlertDialogTitle>
                切换到“{activating?.account.name}”？
              </AlertDialogTitle>
              <AlertDialogDescription>
                切换期间会创建临时回滚副本，再把已识别的历史记录统一到目标
                Provider。成功或回滚后副本会立即删除，Codex Tools
                不长期保存聊天记录。
              </AlertDialogDescription>
            </AlertDialogHeader>
            <AlertDialogFooter>
              <AlertDialogCancel>取消</AlertDialogCancel>
              <AlertDialogAction
                disabled={busy}
                onClick={() => void activate()}
              >
                确认切换
              </AlertDialogAction>
            </AlertDialogFooter>
          </AlertDialogContent>
        </AlertDialog>
        <AlertDialog
          open={!!deletingSession}
          onOpenChange={(open) => !open && setDeletingSession(undefined)}
        >
          <AlertDialogContent>
            <AlertDialogHeader>
              <AlertDialogTitle>永久删除会话？</AlertDialogTitle>
              <AlertDialogDescription>
                “{deletingSession?.title || deletingSession?.id}”将从已识别的
                Codex 数据库中永久删除，无法撤销。
              </AlertDialogDescription>
            </AlertDialogHeader>
            <AlertDialogFooter>
              <AlertDialogCancel>取消</AlertDialogCancel>
              <AlertDialogAction
                variant="destructive"
                onClick={() => void removeSession()}
              >
                永久删除
              </AlertDialogAction>
            </AlertDialogFooter>
          </AlertDialogContent>
        </AlertDialog>
        <AlertDialog
          open={!!deletingProvider}
          onOpenChange={(open) => !open && setDeletingProvider(undefined)}
        >
          <AlertDialogContent>
            <AlertDialogHeader>
              <AlertDialogTitle>删除 Provider？</AlertDialogTitle>
              <AlertDialogDescription>
                “{deletingProvider?.name}”及其全部 API 账号将从 Codex Tools
                数据库中删除。当前 Provider 不允许删除。
              </AlertDialogDescription>
            </AlertDialogHeader>
            <AlertDialogFooter>
              <AlertDialogCancel>取消</AlertDialogCancel>
              <AlertDialogAction
                variant="destructive"
                onClick={() => void removeProvider()}
              >
                删除
              </AlertDialogAction>
            </AlertDialogFooter>
          </AlertDialogContent>
        </AlertDialog>
        <AlertDialog
          open={!!deletingAccount}
          onOpenChange={(open) => !open && setDeletingAccount(undefined)}
        >
          <AlertDialogContent>
            <AlertDialogHeader>
              <AlertDialogTitle>删除账号？</AlertDialogTitle>
              <AlertDialogDescription>
                “{deletingAccount?.name}
                ”及其明文凭据将被永久删除。当前账号不允许删除。
              </AlertDialogDescription>
            </AlertDialogHeader>
            <AlertDialogFooter>
              <AlertDialogCancel>取消</AlertDialogCancel>
              <AlertDialogAction
                variant="destructive"
                onClick={() => void removeAccount()}
              >
                删除
              </AlertDialogAction>
            </AlertDialogFooter>
          </AlertDialogContent>
        </AlertDialog>
        <Toaster richColors />
      </SidebarProvider>
    </TooltipProvider>
  )
}

function DashboardPage({
  dashboard,
  officialCount,
}: {
  dashboard?: Dashboard
  officialCount: number
}) {
  return (
    <>
      <Alert>
        <ShieldCheck />
        <AlertTitle>统一会话模式已启用</AlertTitle>
        <AlertDescription>
          第三方 Provider 与官方账号共用 custom 历史桶；工具不会注入
          Codex，也不会修改 Codex 安装目录。
        </AlertDescription>
      </Alert>
      <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-4">
        {[
          ["当前 Provider", dashboard?.activeProvider || "官方模式"],
          ["Provider", dashboard?.providerCount ?? 0],
          ["已保存官方账号", officialCount],
          ["数据库", dashboard?.databaseHealth || "检查中"],
        ].map(([label, value]) => (
          <Card key={label}>
            <CardHeader>
              <CardDescription>{label}</CardDescription>
              <CardTitle>{value}</CardTitle>
            </CardHeader>
          </Card>
        ))}
      </div>
      <Card>
        <CardHeader>
          <CardTitle>Codex 数据目录</CardTitle>
          <CardDescription>{dashboard?.codexHome}</CardDescription>
        </CardHeader>
        <CardFooter className="flex gap-2">
          <Badge variant="secondary">
            {dashboard?.databaseCount ?? 0} 个数据库
          </Badge>
          <Badge variant="outline">{dashboard?.sessionCount ?? 0} 个会话</Badge>
        </CardFooter>
      </Card>
    </>
  )
}

function ProvidersPage({
  providers,
  accounts,
  officialAccounts,
  busy,
  onNew,
  onEdit,
  onDelete,
  onNewAccount,
  onEditAccount,
  onDeleteAccount,
  onTest,
  onActivate,
  onActivateOfficial,
  onBusy,
  onReload,
}: {
  providers: Provider[]
  accounts: Account[]
  officialAccounts: AuthAccount[]
  busy: boolean
  onNew: () => void
  onEdit: (provider: Provider) => void
  onDelete: (provider: Provider) => void
  onNewAccount: (providerId: string) => void
  onEditAccount: (account: Account) => void
  onDeleteAccount: (account: Account) => void
  onTest: (provider: Provider, account: Account) => void
  onActivate: (provider: Provider, account: Account) => void
  onActivateOfficial: (account: AuthAccount) => void
  onBusy: (value: boolean) => void
  onReload: () => Promise<void>
}) {
  const [device, setDevice] = useState<{
    operationId: string
    userCode: string
    verificationUri: string
    expiresAt: number
    intervalSecs: number
  }>()

  const startOfficialLogin = async () => {
    onBusy(true)
    try {
      const authorization = await call<typeof device & object>(
        "start_openai_device_auth"
      )
      setDevice(authorization)
      toast.success("设备登录已创建，请在 OpenAI 页面输入授权码")
    } catch (error) {
      toast.error(String(error))
    } finally {
      onBusy(false)
    }
  }

  const copyDeviceCode = async () => {
    if (!device) return
    try {
      await navigator.clipboard.writeText(device.userCode)
      toast.success("授权码已复制")
    } catch (error) {
      toast.error(`复制授权码失败：${String(error)}`)
    }
  }

  const openDeviceAuthorization = async () => {
    if (!device) return
    try {
      await openUrl(device.verificationUri)
    } catch (error) {
      toast.error(`打开默认浏览器失败：${String(error)}`)
    }
  }

  useEffect(() => {
    if (!device) return
    let stopped = false
    const timer = window.setInterval(
      async () => {
        try {
          const result = await call<{
            status: "pending" | "expired" | "complete"
          }>("poll_openai_device_auth", { operationId: device.operationId })
          if (stopped || result.status === "pending") return
          window.clearInterval(timer)
          setDevice(undefined)
          if (result.status === "complete") {
            toast.success("OpenAI 官方账号已添加")
            await onReload()
          } else {
            toast.error("OpenAI 设备授权已过期，请重新登录")
          }
        } catch (error) {
          window.clearInterval(timer)
          setDevice(undefined)
          toast.error(String(error))
        }
      },
      Math.max(device.intervalSecs, 3) * 1000
    )
    return () => {
      stopped = true
      window.clearInterval(timer)
    }
  }, [device, onReload])

  const deleteOfficial = async (account: AuthAccount) => {
    try {
      await call("delete_auth_account", { id: account.id })
      toast.success("官方账号已删除")
      await onReload()
    } catch (error) {
      toast.error(String(error))
    }
  }

  return (
    <>
      <Card>
        <CardHeader>
          <CardTitle>OpenAI 官方账号</CardTitle>
          <CardDescription>
            使用 OpenAI Device Code OAuth 添加账号，无需调用 Codex
            客户端登录。切换第三方 API 时只更新路由配置，保留这里的官方登录。
          </CardDescription>
        </CardHeader>
        <CardContent>
          {officialAccounts.length ? (
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>账号</TableHead>
                  <TableHead>标识</TableHead>
                  <TableHead>状态</TableHead>
                  <TableHead className="text-right">操作</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {officialAccounts.map((account) => (
                  <TableRow key={account.id}>
                    <TableCell>{account.name}</TableCell>
                    <TableCell>
                      {account.email || account.login || "OpenAI 账号"}
                    </TableCell>
                    <TableCell>
                      {account.active ? (
                        <Badge>当前</Badge>
                      ) : (
                        <Badge variant="secondary">待用</Badge>
                      )}
                    </TableCell>
                    <TableCell>
                      <div className="flex justify-end gap-2">
                        <Button
                          variant="outline"
                          size="sm"
                          disabled={account.active}
                          onClick={() => void deleteOfficial(account)}
                        >
                          删除
                        </Button>
                        <Button
                          size="sm"
                          disabled={busy || account.active}
                          onClick={() => onActivateOfficial(account)}
                        >
                          切换
                        </Button>
                      </div>
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          ) : (
            <Empty>
              <EmptyHeader>
                <EmptyMedia variant="icon">
                  <UserRound />
                </EmptyMedia>
                <EmptyTitle>还没有官方账号</EmptyTitle>
                <EmptyDescription>
                  点击下方按钮，通过 OpenAI 设备授权页面登录 ChatGPT 账号。
                </EmptyDescription>
              </EmptyHeader>
            </Empty>
          )}
        </CardContent>
        <CardFooter className="flex flex-col items-start gap-3">
          {device && (
            <Alert>
              <KeyRound />
              <AlertTitle>在浏览器中完成授权</AlertTitle>
              <AlertDescription className="flex flex-col gap-3">
                <span>
                  复制下面的授权码，然后使用电脑默认浏览器打开 OpenAI
                  设备授权页面。Codex Tools 会自动等待登录完成。
                </span>
                <InputGroup>
                  <InputGroupInput
                    aria-label="OpenAI 设备授权码"
                    readOnly
                    value={device.userCode}
                    onFocus={(event) => event.currentTarget.select()}
                  />
                  <InputGroupAddon align="inline-end">
                    <InputGroupButton
                      aria-label="复制授权码"
                      title="复制授权码"
                      onClick={() => void copyDeviceCode()}
                    >
                      <Copy />
                    </InputGroupButton>
                  </InputGroupAddon>
                </InputGroup>
                <Button
                  type="button"
                  size="sm"
                  className="self-start"
                  onClick={() => void openDeviceAuthorization()}
                >
                  <ExternalLink data-icon="inline-start" />
                  用默认浏览器打开授权页面
                </Button>
              </AlertDescription>
            </Alert>
          )}
          <Button
            variant="outline"
            disabled={busy || !!device}
            onClick={() => void startOfficialLogin()}
          >
            <Plus data-icon="inline-start" />
            添加官方账号
          </Button>
        </CardFooter>
      </Card>
      <div className="flex justify-end">
        <Button onClick={onNew}>
          <Plus data-icon="inline-start" />
          新增 Provider
        </Button>
      </div>
      {providers.length ? (
        providers.map((provider) => {
          const providerAccounts = accounts.filter(
            (account) => account.providerId === provider.id
          )
          return (
            <Card key={provider.id}>
              <CardHeader>
                <div className="flex items-start justify-between gap-4">
                  <div>
                    <CardTitle>{provider.name}</CardTitle>
                    <CardDescription>
                      {provider.baseUrl} · {provider.defaultModel}
                    </CardDescription>
                  </div>
                  <div className="flex gap-2">
                    <Badge variant="outline">
                      {provider.protocol === "responses"
                        ? "Responses"
                        : "Chat Completions"}
                    </Badge>
                    {provider.active && <Badge>当前</Badge>}
                  </div>
                </div>
              </CardHeader>
              <CardContent>
                {providerAccounts.length ? (
                  <Table>
                    <TableHeader>
                      <TableRow>
                        <TableHead>账号</TableHead>
                        <TableHead>凭据</TableHead>
                        <TableHead>状态</TableHead>
                        <TableHead className="text-right">操作</TableHead>
                      </TableRow>
                    </TableHeader>
                    <TableBody>
                      {providerAccounts.map((account) => (
                        <TableRow key={account.id}>
                          <TableCell>{account.name}</TableCell>
                          <TableCell>
                            <span className="font-mono text-xs">
                              {maskKey(account.apiKey)}
                            </span>
                          </TableCell>
                          <TableCell>
                            {account.active ? (
                              <Badge>当前</Badge>
                            ) : (
                              <Badge variant="secondary">待用</Badge>
                            )}
                          </TableCell>
                          <TableCell>
                            <div className="flex justify-end gap-2">
                              <Button
                                variant="outline"
                                size="sm"
                                onClick={() => onEditAccount(account)}
                              >
                                编辑
                              </Button>
                              <Button
                                variant="outline"
                                size="sm"
                                disabled={account.active}
                                onClick={() => onDeleteAccount(account)}
                              >
                                删除
                              </Button>
                              <Button
                                variant="outline"
                                size="sm"
                                disabled={busy}
                                onClick={() => void onTest(provider, account)}
                              >
                                测试
                              </Button>
                              <Button
                                size="sm"
                                disabled={busy || account.active}
                                onClick={() => onActivate(provider, account)}
                              >
                                切换
                              </Button>
                            </div>
                          </TableCell>
                        </TableRow>
                      ))}
                    </TableBody>
                  </Table>
                ) : (
                  <Empty>
                    <EmptyHeader>
                      <EmptyMedia variant="icon">
                        <KeyRound />
                      </EmptyMedia>
                      <EmptyTitle>还没有 API 账号</EmptyTitle>
                      <EmptyDescription>
                        Provider 保存端点配置，账号保存具体 API Key。
                      </EmptyDescription>
                    </EmptyHeader>
                  </Empty>
                )}
              </CardContent>
              <CardFooter className="flex gap-2">
                <Button variant="outline" onClick={() => onEdit(provider)}>
                  编辑 Provider
                </Button>
                <Button
                  variant="outline"
                  onClick={() => onNewAccount(provider.id)}
                >
                  <Plus data-icon="inline-start" />
                  新增账号
                </Button>
                <Button
                  variant="destructive"
                  disabled={provider.active}
                  onClick={() => onDelete(provider)}
                >
                  <Trash2 data-icon="inline-start" />
                  删除 Provider
                </Button>
              </CardFooter>
            </Card>
          )
        })
      ) : (
        <Empty>
          <EmptyHeader>
            <EmptyMedia variant="icon">
              <Server />
            </EmptyMedia>
            <EmptyTitle>还没有第三方 Provider</EmptyTitle>
            <EmptyDescription>
              添加 Responses 或 Chat Completions 兼容端点。
            </EmptyDescription>
          </EmptyHeader>
        </Empty>
      )}
    </>
  )
}

function SessionsPage({
  sessions,
  onExport,
  onDelete,
}: {
  sessions: Session[]
  onExport: (session: Session) => void
  onDelete: (session: Session) => void
}) {
  return (
    <Card>
      <CardHeader>
        <CardTitle>统一会话历史</CardTitle>
        <CardDescription>
          合并 rollout、session_index.jsonl 与已识别 SQLite
          目录。切到第三方时会登记原本属于 OpenAI
          的会话，切回官方只按该账本精确恢复，不会猜测 custom 会话的来源。
        </CardDescription>
      </CardHeader>
      <CardContent>
        <Alert className="mb-4">
          <ShieldCheck />
          <AlertTitle>原始数据仍是唯一真相来源</AlertTitle>
          <AlertDescription>
            此处的统一列表是可重建索引；SQLite 中的显式标题优先，随后使用 Codex
            会话索引、真实首条用户消息和项目目录名。subagent
            与环境注入记录会被排除。
          </AlertDescription>
        </Alert>
        {sessions.length ? (
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>标题</TableHead>
                <TableHead>Provider</TableHead>
                <TableHead>原始归属</TableHead>
                <TableHead>项目</TableHead>
                <TableHead className="text-right">操作</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {sessions.map((session) => (
                <TableRow key={`${session.sourceDb}-${session.id}`}>
                  <TableCell>{session.title || session.id}</TableCell>
                  <TableCell>
                    <Badge variant="outline">{session.provider || "-"}</Badge>
                  </TableCell>
                  <TableCell>
                    <Badge variant="secondary">
                      {session.originalProvider || "-"}
                    </Badge>
                  </TableCell>
                  <TableCell className="max-w-64 truncate">
                    {session.cwd || "-"}
                  </TableCell>
                  <TableCell>
                    <div className="flex justify-end gap-2">
                      <Button
                        variant="outline"
                        size="sm"
                        onClick={() => void onExport(session)}
                      >
                        导出
                      </Button>
                      <Button
                        variant="destructive"
                        size="sm"
                        onClick={() => onDelete(session)}
                      >
                        <Trash2 data-icon="inline-start" />
                        删除
                      </Button>
                    </div>
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        ) : (
          <Empty>
            <EmptyHeader>
              <EmptyMedia variant="icon">
                <FileArchive />
              </EmptyMedia>
              <EmptyTitle>没有发现会话</EmptyTitle>
              <EmptyDescription>
                确认 Codex 数据库存在并且结构受支持。
              </EmptyDescription>
            </EmptyHeader>
          </Empty>
        )}
      </CardContent>
    </Card>
  )
}

function RouteConsolePage({
  route,
  onReload,
}: {
  route?: RouteConsole
  onReload: () => Promise<void>
}) {
  const stop = async () => {
    try {
      await call("stop_local_route")
      toast.success("本地路由已停止")
      await onReload()
    } catch (error) {
      toast.error(String(error))
    }
  }
  const clear = async () => {
    await call("clear_route_logs")
    await onReload()
  }
  if (!route?.running) {
    return (
      <Empty>
        <EmptyHeader>
          <EmptyMedia variant="icon">
            <Network />
          </EmptyMedia>
          <EmptyTitle>本地路由未运行</EmptyTitle>
          <EmptyDescription>
            激活 Chat Completions Provider 后，会自动启动仅监听 127.0.0.1
            的协议路由。
          </EmptyDescription>
        </EmptyHeader>
      </Empty>
    )
  }
  return (
    <>
      <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-4">
        {[
          ["请求", route.requestCount],
          ["成功", route.successCount],
          ["错误", route.errorCount],
          ["最近延迟", `${route.lastLatencyMs ?? 0} ms`],
        ].map(([label, value]) => (
          <Card key={label}>
            <CardHeader>
              <CardDescription>{label}</CardDescription>
              <CardTitle>{value}</CardTitle>
            </CardHeader>
          </Card>
        ))}
      </div>
      <Card>
        <CardHeader>
          <CardTitle>
            {route.providerName} / {route.model}
          </CardTitle>
          <CardDescription>
            {route.baseUrl} → {route.upstreamUrl}
          </CardDescription>
        </CardHeader>
        <CardContent>
          {route.logs.length ? (
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>时间</TableHead>
                  <TableHead>请求</TableHead>
                  <TableHead>状态</TableHead>
                  <TableHead>耗时</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {route.logs.map((entry) => (
                  <TableRow key={entry.id}>
                    <TableCell>
                      {new Date(entry.timestamp * 1000).toLocaleTimeString()}
                    </TableCell>
                    <TableCell className="font-mono text-xs">
                      {entry.method} {entry.path}
                    </TableCell>
                    <TableCell>
                      <Badge
                        variant={
                          entry.status < 400 ? "secondary" : "destructive"
                        }
                      >
                        {entry.status}
                      </Badge>
                    </TableCell>
                    <TableCell>{entry.latencyMs} ms</TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          ) : (
            <div className="text-sm text-muted-foreground">暂无请求记录。</div>
          )}
        </CardContent>
        <CardFooter className="flex gap-2">
          <Button variant="outline" onClick={() => void clear()}>
            清空日志
          </Button>
          <Button variant="destructive" onClick={() => void stop()}>
            停止路由
          </Button>
        </CardFooter>
      </Card>
    </>
  )
}

function RepairPage({
  scan,
  busy,
  onRepair,
}: {
  scan?: RepairScan
  busy: boolean
  onRepair: () => void
}) {
  return (
    <Card>
      <CardHeader>
        <CardTitle>扫描结果</CardTitle>
        <CardDescription>
          {scan?.rolloutFiles ?? 0} 个 rollout，{scan?.databases.length ?? 0}{" "}
          个数据库。支持默认目录、sqlite_home 和 CODEX_SQLITE_HOME。
        </CardDescription>
      </CardHeader>
      <CardContent>
        <div className="flex flex-col gap-3">
          {scan?.databases.map((database) => (
            <div
              key={database.path}
              className="flex items-center justify-between rounded-lg border p-3"
            >
              <div>
                <div className="font-medium">{database.path}</div>
                <div className="text-sm text-muted-foreground">
                  {database.threadCount} 个会话
                </div>
              </div>
              <Badge
                variant={
                  database.health === "ok" && database.knownSchema
                    ? "secondary"
                    : "destructive"
                }
              >
                {database.health} ·{" "}
                {database.knownSchema ? "已识别" : "未知 schema"}
              </Badge>
            </div>
          ))}
          {scan?.warnings.map((warning) => (
            <Alert key={warning} variant="destructive">
              <AlertTitle>无法安全写入</AlertTitle>
              <AlertDescription>{warning}</AlertDescription>
            </Alert>
          ))}
        </div>
      </CardContent>
      <CardFooter>
        <Button disabled={busy || !scan?.canRepair} onClick={onRepair}>
          <Database data-icon="inline-start" />
          安全统一历史
        </Button>
      </CardFooter>
    </Card>
  )
}

function SettingsPage({ dashboard }: { dashboard?: Dashboard }) {
  return (
    <Card>
      <CardHeader>
        <CardTitle>安全与兼容策略</CardTitle>
        <CardDescription>
          当前实现只写入明确识别的 Codex 配置和数据库字段。
        </CardDescription>
      </CardHeader>
      <CardContent>
        <FieldGroup>
          <Field orientation="horizontal">
            <div>
              <FieldLabel>未知 schema 只读</FieldLabel>
              <FieldDescription>不会猜测 Codex 内部字段。</FieldDescription>
            </div>
            <Badge variant="secondary">已启用</Badge>
          </Field>
          <Field orientation="horizontal">
            <div>
              <FieldLabel>操作期临时回滚</FieldLabel>
              <FieldDescription>
                修改期间临时复制 rollout、SQLite 主库及
                WAL/SHM；完成后立即删除。
              </FieldDescription>
            </div>
            <Badge variant="secondary">已启用</Badge>
          </Field>
          <Field orientation="horizontal">
            <div>
              <FieldLabel>应用数据</FieldLabel>
              <FieldDescription>{dashboard?.codexHome}</FieldDescription>
            </div>
            <Badge variant="outline">本机</Badge>
          </Field>
        </FieldGroup>
      </CardContent>
    </Card>
  )
}

function ProviderDialog({
  value,
  busy,
  onChange,
  onSave,
}: {
  value?: Provider
  busy: boolean
  onChange: (provider?: Provider) => void
  onSave: () => void
}) {
  const [fetchingModels, setFetchingModels] = useState(false)
  const fetchModels = async () => {
    if (!value?.id) {
      toast.error("请先保存 Provider 并添加 API 账号，再获取模型。")
      return
    }
    setFetchingModels(true)
    try {
      const models = await call<FetchedModel[]>("fetch_provider_models", {
        provider: value,
      })
      onChange({ ...value, models: models.map((model) => model.id) })
      toast.success(`已获取 ${models.length} 个模型，默认全部选中。`)
    } catch (error) {
      toast.error(String(error))
    } finally {
      setFetchingModels(false)
    }
  }
  return (
    <Dialog open={!!value} onOpenChange={(open) => !open && onChange()}>
      <DialogContent className="max-h-[90vh] overflow-y-auto">
        <DialogHeader>
          <DialogTitle>{value?.id ? "编辑" : "新增"} Provider</DialogTitle>
          <DialogDescription>
            Provider 只保存端点和模型配置；API Key 在账号中单独管理。
          </DialogDescription>
        </DialogHeader>
        {value && (
          <FieldGroup>
            <Field>
              <FieldLabel>名称</FieldLabel>
              <Input
                value={value.name}
                onChange={(event) =>
                  onChange({ ...value, name: event.target.value })
                }
              />
            </Field>
            <Field>
              <FieldLabel>协议</FieldLabel>
              <ToggleGroup
                value={[value.protocol]}
                onValueChange={(next) =>
                  next[0] &&
                  onChange({ ...value, protocol: next[0] as Protocol })
                }
              >
                <ToggleGroupItem value="responses">Responses</ToggleGroupItem>
                <ToggleGroupItem value="chat_completions">
                  Chat Completions
                </ToggleGroupItem>
              </ToggleGroup>
              <FieldDescription>
                Chat Completions 会由本程序的 loopback 代理转换为 Responses。
              </FieldDescription>
            </Field>
            <Field>
              <FieldLabel>Base URL</FieldLabel>
              <Input
                value={value.baseUrl}
                onChange={(event) =>
                  onChange({ ...value, baseUrl: event.target.value })
                }
              />
            </Field>
            <Field>
              <FieldLabel>默认模型</FieldLabel>
              <Input
                value={value.defaultModel}
                onChange={(event) =>
                  onChange({ ...value, defaultModel: event.target.value })
                }
              />
            </Field>
            <Field>
              <FieldLabel>可选模型</FieldLabel>
              <InputGroup>
                <InputGroupInput
                  value={value.models.join(", ")}
                  onChange={(event) =>
                    onChange({
                      ...value,
                      models: commaList(event.target.value),
                    })
                  }
                  placeholder="留空则不启用自定义模型目录"
                />
                <InputGroupAddon align="inline-end">
                  <InputGroupButton
                    aria-label="从 Provider 获取全部模型"
                    title="获取全部模型"
                    disabled={fetchingModels || !value.id}
                    onClick={() => void fetchModels()}
                  >
                    <RefreshCw data-icon="inline-start" />
                    {fetchingModels ? "获取中" : "获取模型"}
                  </InputGroupButton>
                </InputGroupAddon>
              </InputGroup>
              <FieldDescription>
                实时请求 Provider 的
                /models，成功后默认填入全部模型；留空时不生成 Codex
                自定义模型目录。
              </FieldDescription>
            </Field>
            {value.protocol === "chat_completions" && (
              <Field>
                <div className="flex items-center justify-between gap-3">
                  <div className="flex flex-col gap-1">
                    <FieldLabel htmlFor="reasoning-override">
                      自定义推理映射
                    </FieldLabel>
                    <FieldDescription>
                      默认按 CC-Switch 规则根据
                      Provider、地址和当前模型实时识别。
                    </FieldDescription>
                  </div>
                  <Switch
                    id="reasoning-override"
                    checked={!!value.codexChatReasoning}
                    onCheckedChange={(checked) =>
                      onChange({
                        ...value,
                        codexChatReasoning: checked
                          ? {
                              supportsThinking: true,
                              supportsEffort: true,
                              thinkingParam: "thinking",
                              effortParam: "reasoning_effort",
                              effortValueMode: "standard",
                              outputFormat: "reasoning_content",
                            }
                          : undefined,
                      })
                    }
                  />
                </div>
              </Field>
            )}
            {value.protocol === "chat_completions" &&
              value.codexChatReasoning && (
                <FieldGroup>
                  <Field orientation="horizontal">
                    <FieldLabel htmlFor="supports-thinking">
                      发送思考开关
                    </FieldLabel>
                    <Switch
                      id="supports-thinking"
                      checked={
                        value.codexChatReasoning.supportsThinking ?? false
                      }
                      onCheckedChange={(checked) =>
                        onChange({
                          ...value,
                          codexChatReasoning: {
                            ...value.codexChatReasoning,
                            supportsThinking: checked,
                          },
                        })
                      }
                    />
                  </Field>
                  <Field orientation="horizontal">
                    <FieldLabel htmlFor="supports-effort">
                      发送推理强度
                    </FieldLabel>
                    <Switch
                      id="supports-effort"
                      checked={value.codexChatReasoning.supportsEffort ?? false}
                      onCheckedChange={(checked) =>
                        onChange({
                          ...value,
                          codexChatReasoning: {
                            ...value.codexChatReasoning,
                            supportsEffort: checked,
                          },
                        })
                      }
                    />
                  </Field>
                  <Field>
                    <FieldLabel>思考参数</FieldLabel>
                    <Input
                      value={value.codexChatReasoning.thinkingParam ?? ""}
                      placeholder="thinking / enable_thinking / reasoning_split"
                      onChange={(event) =>
                        onChange({
                          ...value,
                          codexChatReasoning: {
                            ...value.codexChatReasoning,
                            thinkingParam: event.target.value || undefined,
                          },
                        })
                      }
                    />
                  </Field>
                  <Field>
                    <FieldLabel>推理强度参数</FieldLabel>
                    <Input
                      value={value.codexChatReasoning.effortParam ?? ""}
                      placeholder="reasoning_effort / reasoning.effort"
                      onChange={(event) =>
                        onChange({
                          ...value,
                          codexChatReasoning: {
                            ...value.codexChatReasoning,
                            effortParam: event.target.value || undefined,
                          },
                        })
                      }
                    />
                  </Field>
                </FieldGroup>
              )}
            <Field>
              <FieldLabel>上下文窗口</FieldLabel>
              <Input
                type="number"
                min={1}
                value={value.contextWindow ?? ""}
                onChange={(event) =>
                  onChange({
                    ...value,
                    contextWindow: optionalNumber(event.target.value),
                  })
                }
              />
            </Field>
            <Field>
              <FieldLabel>自动压缩阈值</FieldLabel>
              <Input
                type="number"
                min={1}
                value={value.autoCompactThreshold ?? ""}
                onChange={(event) =>
                  onChange({
                    ...value,
                    autoCompactThreshold: optionalNumber(event.target.value),
                  })
                }
              />
            </Field>
            <Field>
              <FieldLabel>超时（秒）</FieldLabel>
              <Input
                type="number"
                min={1}
                value={value.timeoutSecs}
                onChange={(event) =>
                  onChange({
                    ...value,
                    timeoutSecs: Number(event.target.value) || 30,
                  })
                }
              />
            </Field>
            <HeaderEditor
              label="Provider Headers（JSON）"
              value={value.headers}
              description="必须是字符串键值对象；账号 Header 会覆盖同名项。"
              onChange={(headers) => onChange({ ...value, headers })}
            />
          </FieldGroup>
        )}
        <DialogFooter>
          <Button variant="outline" onClick={() => onChange()}>
            取消
          </Button>
          <Button disabled={busy} onClick={onSave}>
            保存
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}

function AccountDialog({
  value,
  busy,
  onChange,
  onSave,
}: {
  value?: Account
  busy: boolean
  onChange: (account?: Account) => void
  onSave: () => void
}) {
  const [visible, setVisible] = useState(false)
  return (
    <Dialog open={!!value} onOpenChange={(open) => !open && onChange()}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{value?.id ? "编辑" : "新增"} API 账号</DialogTitle>
          <DialogDescription>
            凭据以明文保存在 codex-tools.db；账号专属 Header 会覆盖 Provider
            Header。
          </DialogDescription>
        </DialogHeader>
        {value && (
          <FieldGroup>
            <Field>
              <FieldLabel>账号名称</FieldLabel>
              <Input
                value={value.name}
                onChange={(event) =>
                  onChange({ ...value, name: event.target.value })
                }
              />
            </Field>
            {value.authKind === "api_key" && (
              <>
                <Field>
                  <FieldLabel>API Key</FieldLabel>
                  <InputGroup>
                    <InputGroupInput
                      type={visible ? "text" : "password"}
                      value={value.apiKey || ""}
                      onChange={(event) =>
                        onChange({ ...value, apiKey: event.target.value })
                      }
                    />
                    <InputGroupAddon align="inline-end">
                      <InputGroupButton
                        aria-label={visible ? "隐藏 API Key" : "显示 API Key"}
                        onClick={() => setVisible((current) => !current)}
                      >
                        {visible ? <EyeOff /> : <Eye />}
                      </InputGroupButton>
                    </InputGroupAddon>
                  </InputGroup>
                  <FieldDescription>
                    界面和诊断日志不会显示完整密钥。
                  </FieldDescription>
                </Field>
                <HeaderEditor
                  label="账号 Headers（JSON）"
                  value={value.headers}
                  description={`例如 {"x-api-key":"value"}。`}
                  onChange={(headers) => onChange({ ...value, headers })}
                />
              </>
            )}
          </FieldGroup>
        )}
        <DialogFooter>
          <Button variant="outline" onClick={() => onChange()}>
            取消
          </Button>
          <Button disabled={busy} onClick={onSave}>
            保存
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}

function HeaderEditor({
  label,
  value,
  description,
  onChange,
}: {
  label: string
  value: Record<string, string>
  description: string
  onChange: (value: Record<string, string>) => void
}) {
  const [text, setText] = useState(() => JSON.stringify(value, null, 2))
  const parsed = parseHeaders(text)
  return (
    <Field data-invalid={!parsed}>
      <FieldLabel>{label}</FieldLabel>
      <Textarea
        value={text}
        aria-invalid={!parsed}
        onChange={(event) => {
          const next = event.target.value
          setText(next)
          const valid = parseHeaders(next)
          if (valid) onChange(valid)
        }}
        onBlur={() => {
          if (!parsed) toast.error("Headers 必须是字符串键值 JSON 对象")
        }}
      />
      <FieldDescription>{description}</FieldDescription>
    </Field>
  )
}
