import { useCallback, useEffect, useMemo, useState } from "react"
import { save } from "@tauri-apps/plugin-dialog"
import {
  Activity,
  Database,
  Eye,
  EyeOff,
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
  CodeXml,
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
  ["auth", "OAuth 认证中心", KeyRound],
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
    account: Account
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

  const officialAccounts = useMemo(
    () => accounts.filter((account) => account.authKind === "official_oauth"),
    [accounts]
  )
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
  const captureOfficial = async () => {
    setBusy(true)
    try {
      await call("capture_official_account", {
        name: `官方账号 ${officialAccounts.length + 1}`,
      })
      toast.success("已保存当前 Codex 官方登录")
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
      const result =
        activating.account.authKind === "official_oauth"
          ? await call<{ rowsUpdated: number }>("activate_official_account", {
              accountId: activating.account.id,
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
                API Key 和官方登录快照保存在本机数据库中，请勿分享完整备份。
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
                onActivateOfficial={(account) => setActivating({ account })}
                onCaptureOfficial={captureOfficial}
              />
            )}
            {page === "sessions" && (
              <SessionsPage
                sessions={sessions}
                onExport={exportOne}
                onDelete={setDeletingSession}
              />
            )}
            {page === "auth" && (
              <AuthCenterPage
                accounts={authAccounts}
                busy={busy}
                onBusy={setBusy}
                onReload={reload}
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
                将先备份 Codex
                配置和会话数据，再切换账号并把已识别的历史记录统一到 custom
                会话桶。任何一步失败都会回滚。
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
  onCaptureOfficial,
}: {
  providers: Provider[]
  accounts: Account[]
  officialAccounts: Account[]
  busy: boolean
  onNew: () => void
  onEdit: (provider: Provider) => void
  onDelete: (provider: Provider) => void
  onNewAccount: (providerId: string) => void
  onEditAccount: (account: Account) => void
  onDeleteAccount: (account: Account) => void
  onTest: (provider: Provider, account: Account) => void
  onActivate: (provider: Provider, account: Account) => void
  onActivateOfficial: (account: Account) => void
  onCaptureOfficial: () => void
}) {
  return (
    <>
      <Card>
        <CardHeader>
          <CardTitle>官方 Codex 账号</CardTitle>
          <CardDescription>
            保存当前 auth.json 的完整登录快照，之后可以一键切换回来。切换第三方
            API 时不会覆盖这些 OAuth 数据。
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
                    <TableCell>{account.email || "已保存登录快照"}</TableCell>
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
                          重命名
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
                <EmptyTitle>还没有官方账号快照</EmptyTitle>
                <EmptyDescription>
                  先在 Codex 中完成 ChatGPT 登录，然后捕获当前账号。
                </EmptyDescription>
              </EmptyHeader>
            </Empty>
          )}
        </CardContent>
        <CardFooter>
          <Button variant="outline" disabled={busy} onClick={onCaptureOfficial}>
            <Plus data-icon="inline-start" />
            捕获当前官方登录
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
          来自 Codex SQLite 的最近记录。第三方与官方账号切换后都使用 custom
          会话桶。
        </CardDescription>
      </CardHeader>
      <CardContent>
        {sessions.length ? (
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>标题</TableHead>
                <TableHead>Provider</TableHead>
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

function AuthCenterPage({
  accounts,
  busy,
  onBusy,
  onReload,
}: {
  accounts: AuthAccount[]
  busy: boolean
  onBusy: (value: boolean) => void
  onReload: () => Promise<void>
}) {
  const [clientId, setClientId] = useState("")
  const [device, setDevice] = useState<{
    operationId: string
    userCode: string
    verificationUri: string
  }>()
  const captureOpenAi = async () => {
    onBusy(true)
    try {
      await call("capture_openai_account", { name: "OpenAI 官方账号" })
      toast.success("已保存当前 Codex 官方登录与配置快照")
      await onReload()
    } catch (error) {
      toast.error(String(error))
    } finally {
      onBusy(false)
    }
  }
  const startGitHub = async () => {
    onBusy(true)
    try {
      const next = await call<{
        operationId: string
        userCode: string
        verificationUri: string
      }>("start_github_device_auth", {
        clientId,
        scopes: ["read:user", "user:email"],
      })
      setDevice(next)
      toast.success("设备授权已创建，请在 GitHub 完成确认")
    } catch (error) {
      toast.error(String(error))
    } finally {
      onBusy(false)
    }
  }
  const completeGitHub = async () => {
    if (!device) return
    onBusy(true)
    try {
      await call("complete_github_device_auth", {
        operationId: device.operationId,
      })
      setDevice(undefined)
      toast.success("GitHub 账号已添加")
      await onReload()
    } catch (error) {
      toast.error(String(error))
    } finally {
      onBusy(false)
    }
  }
  const activateOpenAi = async (id: string) => {
    onBusy(true)
    try {
      await call("activate_openai_account", { id })
      toast.success("已恢复 OpenAI 官方登录")
      await onReload()
    } catch (error) {
      toast.error(String(error))
    } finally {
      onBusy(false)
    }
  }
  return (
    <>
      <Alert>
        <ShieldCheck />
        <AlertTitle>认证与模型路由分离</AlertTitle>
        <AlertDescription>
          OpenAI 快照可直接恢复到 Codex。GitHub 登录只保存身份与授权，不会伪装成
          OpenAI 凭据；使用 GitHub Models 时仍需单独配置兼容 Provider。
        </AlertDescription>
      </Alert>
      <div className="grid gap-4 xl:grid-cols-2">
        <Card>
          <CardHeader>
            <CardTitle>OpenAI / Codex</CardTitle>
            <CardDescription>
              捕获当前 auth.json 和官方配置；切换第三方前也会自动保存一次。
            </CardDescription>
          </CardHeader>
          <CardContent className="flex flex-col gap-3">
            {accounts
              .filter((account) => account.service === "open_ai")
              .map((account) => (
                <div
                  key={account.id}
                  className="flex items-center justify-between rounded-lg border p-3"
                >
                  <div>
                    <div className="font-medium">{account.name}</div>
                    <div className="text-sm text-muted-foreground">
                      {account.email || "官方登录快照"}
                    </div>
                  </div>
                  <div className="flex gap-2">
                    {account.active && <Badge>当前</Badge>}
                    <Button
                      size="sm"
                      disabled={busy || account.active}
                      onClick={() => void activateOpenAi(account.id)}
                    >
                      恢复到 Codex
                    </Button>
                  </div>
                </div>
              ))}
          </CardContent>
          <CardFooter>
            <Button disabled={busy} onClick={() => void captureOpenAi()}>
              <KeyRound data-icon="inline-start" />
              保存当前官方登录
            </Button>
          </CardFooter>
        </Card>
        <Card>
          <CardHeader>
            <CardTitle>GitHub Device Flow</CardTitle>
            <CardDescription>
              使用你自己的 GitHub OAuth App Client
              ID；令牌按当前项目约定明文保存在本机数据库。
            </CardDescription>
          </CardHeader>
          <CardContent>
            <FieldGroup>
              <Field>
                <FieldLabel>OAuth App Client ID</FieldLabel>
                <Input
                  value={clientId}
                  onChange={(event) => setClientId(event.target.value)}
                />
              </Field>
              {device && (
                <Alert>
                  <CodeXml />
                  <AlertTitle>授权码：{device.userCode}</AlertTitle>
                  <AlertDescription>{device.verificationUri}</AlertDescription>
                </Alert>
              )}
              {accounts
                .filter((account) => account.service === "github")
                .map((account) => (
                  <div
                    key={account.id}
                    className="flex items-center gap-3 rounded-lg border p-3"
                  >
                    <CodeXml />
                    <div>
                      <div className="font-medium">{account.name}</div>
                      <div className="text-sm text-muted-foreground">
                        @{account.login}
                      </div>
                    </div>
                  </div>
                ))}
            </FieldGroup>
          </CardContent>
          <CardFooter className="flex gap-2">
            <Button
              variant="outline"
              disabled={busy || !clientId}
              onClick={() => void startGitHub()}
            >
              开始授权
            </Button>
            {device && (
              <Button disabled={busy} onClick={() => void completeGitHub()}>
                我已授权，完成添加
              </Button>
            )}
          </CardFooter>
        </Card>
      </div>
    </>
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
          备份并统一历史
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
              <FieldLabel>修复前完整备份</FieldLabel>
              <FieldDescription>
                包含配置、rollout、SQLite 主库及 WAL/SHM。
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
              <Input
                value={value.models.join(", ")}
                onChange={(event) =>
                  onChange({ ...value, models: commaList(event.target.value) })
                }
              />
              <FieldDescription>用英文逗号分隔。</FieldDescription>
            </Field>
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
