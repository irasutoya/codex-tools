import { useEffect, useMemo, useRef, useState } from "react"
import {
  ChartHistogramIcon,
  Clock01Icon,
  FilterIcon,
  Home01Icon,
  InformationCircleIcon,
  Key01Icon,
  Link01Icon,
  Refresh01Icon,
  Rocket01Icon,
  Search01Icon,
  Settings01Icon,
  Tick02Icon,
} from "@hugeicons/core-free-icons"
import { HugeiconsIcon, type IconSvgElement } from "@hugeicons/react"

import { DashboardPage } from "@/features/dashboard/dashboard-page"
import { ProvidersPage } from "@/features/providers/providers-page"
import { SessionsPage } from "@/features/sessions/sessions-page"
import { SettingsPage } from "@/features/settings/settings-page"
import { UsagePage } from "@/features/usage/usage-page"
import { Badge } from "@/components/ui/badge"
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { Button } from "@/components/ui/button"
import { Field, FieldGroup, FieldLabel } from "@/components/ui/field"
import {
  InputGroup,
  InputGroupAddon,
  InputGroupInput,
} from "@/components/ui/input-group"
import {
  Item,
  ItemActions,
  ItemContent,
  ItemDescription,
  ItemGroup,
  ItemMedia,
  ItemTitle,
} from "@/components/ui/item"
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
} from "@/components/ui/sheet"
import { Spinner } from "@/components/ui/spinner"
import { Toaster, toast } from "@/components/ui/toast"
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group"
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip"
import { errorMessage } from "@/lib/format"
import { call } from "@/lib/ipc"
import type { Dashboard, Page, ProviderOverview, UsageGroupBy } from "@/types"

const navigation: Array<{
  id: Page
  label: string
  icon: IconSvgElement
}> = [
  { id: "dashboard", label: "概览", icon: Home01Icon },
  { id: "providers", label: "连接", icon: Link01Icon },
  { id: "usage", label: "用量", icon: ChartHistogramIcon },
  { id: "sessions", label: "会话", icon: Clock01Icon },
  { id: "settings", label: "设置", icon: Settings01Icon },
]

export type SettingsSection = "config" | "diagnostics" | "app" | "unlock"

export default function App() {
  const mainRef = useRef<HTMLElement>(null)
  const [page, setPage] = useState<Page>("dashboard")
  const [contextOpen, setContextOpen] = useState(false)
  const [refreshRevision, setRefreshRevision] = useState(0)
  const [dashboard, setDashboard] = useState<Dashboard>()
  const [connections, setConnections] = useState<ProviderOverview>()
  const [selectedConnection, setSelectedConnection] = useState<string>()
  const [usageDays, setUsageDays] = useState(7)
  const [usageGroupBy, setUsageGroupBy] = useState<UsageGroupBy>("model")
  const [sessionQuery, setSessionQuery] = useState("")
  const [settingsSection, setSettingsSection] =
    useState<SettingsSection>("config")
  const [launching, setLaunching] = useState(false)
  const [stateError, setStateError] = useState<string>()

  useEffect(() => {
    if (refreshRevision > 0 && page !== "dashboard" && page !== "providers") {
      return
    }
    let cancelled = false
    void Promise.all([call("dashboard_get"), call("connections_list")])
      .then(([nextDashboard, nextConnections]) => {
        if (cancelled) return
        setDashboard(nextDashboard)
        setConnections(nextConnections)
        setStateError(undefined)
        setSelectedConnection(
          (current) =>
            current ??
            nextConnections.officialAccounts.find((account) => account.active)
              ?.id ??
            nextConnections.providers.find((provider) => provider.active)?.id
        )
      })
      .catch((reason) => {
        if (cancelled) return
        const message = errorMessage(reason)
        setStateError(message)
        toast.add({
          title: "无法读取应用状态",
          description: message,
          type: "error",
        })
      })
    return () => {
      cancelled = true
    }
  }, [page, refreshRevision])

  const refresh = () => setRefreshRevision((value) => value + 1)

  const launch = async () => {
    setLaunching(true)
    try {
      const result = await call("dashboard_launch")
      toast.add({
        title: "Codex 已启动",
        description: result.message,
        type: "success",
      })
      refresh()
    } catch (reason) {
      toast.add({
        title: "无法启动 Codex",
        description: errorMessage(reason),
        type: "error",
      })
    } finally {
      setLaunching(false)
    }
  }

  const contextLabel = useMemo(() => {
    if (page === "usage")
      return `最近 ${usageDays} 天 · ${usageGroupBy === "model" ? "按模型" : "按账号"}`
    if (page === "sessions") return sessionQuery || "全部会话"
    if (page === "settings") {
      return {
        config: "当前配置",
        diagnostics: "诊断信息",
        app: "启动程序",
        unlock: "模型解锁",
      }[settingsSection]
    }
    return dashboard?.activeAccount ?? dashboard?.activeProvider ?? "选择连接"
  }, [dashboard, page, sessionQuery, settingsSection, usageDays, usageGroupBy])

  const contextIcon =
    page === "usage"
      ? FilterIcon
      : page === "sessions"
        ? Search01Icon
        : undefined

  const sharedProps = {
    refreshRevision,
    onRefresh: refresh,
  }

  useEffect(() => {
    mainRef.current?.scrollTo({ top: 0 })
  }, [page])

  return (
    <TooltipProvider>
      <div className="grid size-full grid-cols-[52px_minmax(0,1fr)] overflow-hidden bg-background">
        <nav
          aria-label="主导航"
          className="flex min-h-0 flex-col items-center gap-2 border-r border-sidebar-border bg-sidebar px-2 py-2"
        >
          {navigation.map((item) => (
            <NavButton
              key={item.id}
              active={page === item.id}
              icon={item.icon}
              label={item.label}
              onClick={() => {
                setPage(item.id)
                setContextOpen(false)
              }}
            />
          ))}
        </nav>

        <div className="flex min-w-0 flex-col overflow-hidden">
          <header className="flex h-13 shrink-0 items-center gap-2 px-3">
            <Button
              variant="outline"
              size="sm"
              className="max-w-48"
              onClick={() => setContextOpen(true)}
            >
              {contextIcon && (
                <HugeiconsIcon icon={contextIcon} data-icon="inline-start" />
              )}
              <span className="truncate">{contextLabel}</span>
            </Button>
            {(page === "dashboard" || page === "providers") &&
              dashboard?.activeModel && (
                <Badge variant="outline" className="max-w-36 truncate">
                  {dashboard.activeModel}
                </Badge>
              )}
            {(page === "dashboard" || page === "providers") && (
              <Badge variant={stateError ? "destructive" : "secondary"}>
                {stateError ? "异常" : dashboard ? "正常" : "读取中"}
              </Badge>
            )}
            <div className="ml-auto flex items-center gap-2">
              <Tooltip>
                <TooltipTrigger
                  render={
                    <Button
                      variant="outline"
                      size="icon-sm"
                      aria-label="刷新当前页面"
                      onClick={refresh}
                    />
                  }
                >
                  <HugeiconsIcon icon={Refresh01Icon} />
                </TooltipTrigger>
                <TooltipContent side="bottom">刷新</TooltipContent>
              </Tooltip>
              <Button
                size="sm"
                disabled={launching}
                onClick={() => void launch()}
              >
                {launching ? (
                  <Spinner data-icon="inline-start" />
                ) : (
                  <HugeiconsIcon icon={Rocket01Icon} data-icon="inline-start" />
                )}
                启动 Codex
              </Button>
            </div>
          </header>

          <main
            ref={mainRef}
            className="min-h-0 flex-1 [scrollbar-gutter:stable] overflow-y-auto overscroll-contain"
          >
            {stateError &&
              ((page === "dashboard" && !dashboard) ||
                (page === "providers" && !connections)) && (
                <div className="px-3 pt-1">
                  <Alert variant="destructive">
                    <HugeiconsIcon icon={InformationCircleIcon} />
                    <AlertTitle>无法读取应用状态</AlertTitle>
                    <AlertDescription>{stateError}</AlertDescription>
                  </Alert>
                </div>
              )}
            {page === "dashboard" && (!stateError || dashboard) && (
              <DashboardPage {...sharedProps} dashboard={dashboard} />
            )}
            {page === "providers" && (!stateError || connections) && (
              <ProvidersPage
                {...sharedProps}
                connections={connections}
                selectedId={selectedConnection}
                onSelectedIdChange={setSelectedConnection}
              />
            )}
            {page === "usage" && (
              <UsagePage
                {...sharedProps}
                days={usageDays}
                groupBy={usageGroupBy}
              />
            )}
            {page === "sessions" && (
              <SessionsPage
                key={sessionQuery}
                {...sharedProps}
                query={sessionQuery}
              />
            )}
            {page === "settings" && (
              <SettingsPage {...sharedProps} section={settingsSection} />
            )}
          </main>
        </div>
      </div>

      <ContextSheet
        open={contextOpen}
        onOpenChange={setContextOpen}
        page={page}
        dashboard={dashboard}
        connections={connections}
        selectedConnection={selectedConnection}
        onSelectedConnectionChange={setSelectedConnection}
        usageDays={usageDays}
        onUsageDaysChange={setUsageDays}
        usageGroupBy={usageGroupBy}
        onUsageGroupByChange={setUsageGroupBy}
        sessionQuery={sessionQuery}
        onSessionQueryChange={setSessionQuery}
        settingsSection={settingsSection}
        onSettingsSectionChange={setSettingsSection}
        onChanged={refresh}
      />
      <Toaster timeout={4500} limit={3} />
    </TooltipProvider>
  )
}

function NavButton({
  active,
  icon,
  label,
  onClick,
}: {
  active: boolean
  icon: IconSvgElement
  label: string
  onClick: () => void
}) {
  return (
    <Tooltip>
      <TooltipTrigger
        render={
          <Button
            variant={active ? "secondary" : "outline"}
            size="icon-sm"
            aria-current={active ? "page" : undefined}
            aria-label={label}
            className={
              active ? "shrink-0 ring-1 ring-foreground/10" : "shrink-0"
            }
            onClick={onClick}
          />
        }
      >
        <HugeiconsIcon icon={icon} />
      </TooltipTrigger>
      <TooltipContent side="right">{label}</TooltipContent>
    </Tooltip>
  )
}

function ContextSheet({
  open,
  onOpenChange,
  page,
  dashboard,
  connections,
  selectedConnection,
  onSelectedConnectionChange,
  usageDays,
  onUsageDaysChange,
  usageGroupBy,
  onUsageGroupByChange,
  sessionQuery,
  onSessionQueryChange,
  settingsSection,
  onSettingsSectionChange,
  onChanged,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  page: Page
  dashboard?: Dashboard
  connections?: ProviderOverview
  selectedConnection?: string
  onSelectedConnectionChange: (id: string) => void
  usageDays: number
  onUsageDaysChange: (days: number) => void
  usageGroupBy: UsageGroupBy
  onUsageGroupByChange: (groupBy: UsageGroupBy) => void
  sessionQuery: string
  onSessionQueryChange: (query: string) => void
  settingsSection: SettingsSection
  onSettingsSectionChange: (section: SettingsSection) => void
  onChanged: () => void
}) {
  const connectionPage = page === "dashboard" || page === "providers"

  const activate = async (kind: "account" | "provider", id: string) => {
    try {
      await (kind === "account"
        ? call("connections_activate_account", { id })
        : call("connections_activate", { id }))
      onSelectedConnectionChange(id)
      onOpenChange(false)
      onChanged()
      toast.add({ title: "连接已切换", type: "success" })
    } catch (reason) {
      toast.add({
        title: "无法切换连接",
        description: errorMessage(reason),
        type: "error",
      })
    }
  }

  return (
    <Sheet open={open} onOpenChange={onOpenChange}>
      <SheetContent side="left" className="w-64 overflow-y-auto">
        <SheetHeader>
          <SheetTitle>
            {connectionPage
              ? "账号与服务"
              : page === "usage"
                ? "用量筛选"
                : page === "sessions"
                  ? "搜索会话"
                  : "设置章节"}
          </SheetTitle>
          <SheetDescription>
            {connectionPage
              ? "选择 Codex 当前使用的连接。"
              : page === "usage"
                ? "调整统计范围与汇总方式。"
                : page === "sessions"
                  ? "按标题或项目路径搜索。"
                  : "选择要查看的设置。"}
          </SheetDescription>
        </SheetHeader>

        <div className="flex flex-col gap-3 px-3 pb-4">
          {connectionPage && connections && (
            <>
              <ConnectionGroup
                label="OpenAI 账号"
                items={connections.officialAccounts.map((account) => ({
                  id: account.id,
                  title: account.name,
                  description: account.email || "OpenAI 账号",
                  active:
                    account.active || dashboard?.activeAccountId === account.id,
                  enabled: true,
                  kind: "account" as const,
                }))}
                selectedId={selectedConnection}
                onSelect={(kind, id) => void activate(kind, id)}
              />
              <ConnectionGroup
                label="API 服务"
                items={connections.providers.map((provider) => ({
                  id: provider.id,
                  title: provider.name,
                  description: provider.model || provider.baseUrl,
                  active: provider.active,
                  enabled: provider.enabled,
                  kind: "provider" as const,
                }))}
                selectedId={selectedConnection}
                onSelect={(kind, id) => void activate(kind, id)}
              />
            </>
          )}

          {page === "usage" && (
            <FieldGroup>
              <Field>
                <FieldLabel>时间范围</FieldLabel>
                <ToggleGroup
                  variant="outline"
                  spacing={0}
                  value={[String(usageDays)]}
                  onValueChange={(value) => {
                    if (value[0]) onUsageDaysChange(Number(value[0]))
                  }}
                >
                  <ToggleGroupItem value="1">今天</ToggleGroupItem>
                  <ToggleGroupItem value="7">7 天</ToggleGroupItem>
                  <ToggleGroupItem value="30">30 天</ToggleGroupItem>
                </ToggleGroup>
              </Field>
              <Field>
                <FieldLabel>汇总方式</FieldLabel>
                <ToggleGroup
                  variant="outline"
                  spacing={0}
                  value={[usageGroupBy]}
                  onValueChange={(value) => {
                    if (value[0]) onUsageGroupByChange(value[0] as UsageGroupBy)
                  }}
                >
                  <ToggleGroupItem value="model">按模型</ToggleGroupItem>
                  <ToggleGroupItem value="account">按账号</ToggleGroupItem>
                </ToggleGroup>
              </Field>
            </FieldGroup>
          )}

          {page === "sessions" && (
            <FieldGroup>
              <Field>
                <FieldLabel htmlFor="session-search">搜索</FieldLabel>
                <InputGroup>
                  <InputGroupAddon>
                    <HugeiconsIcon icon={Search01Icon} />
                  </InputGroupAddon>
                  <InputGroupInput
                    id="session-search"
                    value={sessionQuery}
                    placeholder="标题或项目路径"
                    onChange={(event) =>
                      onSessionQueryChange(event.target.value)
                    }
                  />
                </InputGroup>
              </Field>
              <Button onClick={() => onOpenChange(false)}>应用搜索</Button>
            </FieldGroup>
          )}

          {page === "settings" && (
            <ToggleGroup
              orientation="vertical"
              variant="outline"
              className="w-full"
              value={[settingsSection]}
              onValueChange={(value) => {
                if (!value[0]) return
                onSettingsSectionChange(value[0] as SettingsSection)
                onOpenChange(false)
              }}
            >
              <ToggleGroupItem value="config">当前配置</ToggleGroupItem>
              <ToggleGroupItem value="diagnostics">诊断信息</ToggleGroupItem>
              <ToggleGroupItem value="app">启动程序</ToggleGroupItem>
              <ToggleGroupItem value="unlock">模型解锁</ToggleGroupItem>
            </ToggleGroup>
          )}
        </div>
      </SheetContent>
    </Sheet>
  )
}

type ConnectionItem = {
  id: string
  title: string
  description: string
  active: boolean
  enabled: boolean
  kind: "account" | "provider"
}

function ConnectionGroup({
  label,
  items,
  selectedId,
  onSelect,
}: {
  label: string
  items: ConnectionItem[]
  selectedId?: string
  onSelect: (kind: ConnectionItem["kind"], id: string) => void
}) {
  return (
    <div className="flex flex-col gap-1.5">
      <div className="px-1 text-xs font-medium text-muted-foreground">
        {label}
      </div>
      <ItemGroup>
        {items.map((item) => (
          <Item
            key={item.id}
            size="xs"
            className="flex-nowrap"
            variant={
              item.id === selectedId || item.active ? "muted" : "default"
            }
            render={<button type="button" disabled={!item.enabled} />}
            aria-current={item.active ? "true" : undefined}
            onClick={() => onSelect(item.kind, item.id)}
          >
            <ItemMedia variant="icon">
              <HugeiconsIcon
                icon={item.kind === "account" ? Key01Icon : ChartHistogramIcon}
              />
            </ItemMedia>
            <ItemContent>
              <ItemTitle className="w-full">{item.title}</ItemTitle>
              <ItemDescription className="truncate">
                {item.description}
              </ItemDescription>
            </ItemContent>
            {(item.active || item.id === selectedId) && (
              <ItemActions className="ml-auto self-center">
                <HugeiconsIcon icon={Tick02Icon} aria-label="当前连接" />
              </ItemActions>
            )}
          </Item>
        ))}
      </ItemGroup>
    </div>
  )
}
