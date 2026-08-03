import {
  lazy,
  Suspense,
  useCallback,
  useEffect,
  useRef,
  useState,
  type CSSProperties,
} from "react"
import {
  Check,
  ChartNoAxesCombined,
  FileCheck2,
  KeyRound,
  LayoutDashboard,
  MessagesSquare,
  Monitor,
  Moon,
  ShieldCheck,
  Sun,
  type LucideIcon,
} from "lucide-react"
import { useTheme } from "next-themes"

import { PageHeader } from "@/components/page-header"
import { PageLoading } from "@/components/page-loading"
import { Button } from "@/components/ui/button"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
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
} from "@/components/ui/sidebar"
import { Toaster } from "@/components/ui/sonner"
import { TooltipProvider } from "@/components/ui/tooltip"
import { refreshCoordinator } from "@/lib/refresh-coordinator"
import type { Page } from "@/types"

const pageLoaders = {
  dashboard: () => import("@/features/dashboard/dashboard-page"),
  providers: () => import("@/features/providers/providers-page"),
  usage: () => import("@/features/usage/usage-page"),
  sessions: () => import("@/features/sessions/sessions-page"),
  settings: () => import("@/features/settings/settings-page"),
}

const pages = {
  dashboard: lazy(pageLoaders.dashboard),
  providers: lazy(pageLoaders.providers),
  usage: lazy(pageLoaders.usage),
  sessions: lazy(pageLoaders.sessions),
  settings: lazy(pageLoaders.settings),
}

type NavigationItem = {
  id: Page
  label: string
  description: string
  icon: LucideIcon
}

const navigation: NavigationItem[] = [
  {
    id: "dashboard",
    label: "概览",
    description: "查看当前连接、账号额度和本机会话状态",
    icon: LayoutDashboard,
  },
  {
    id: "providers",
    label: "账号与服务",
    description: "管理 OpenAI 账号与兼容 Responses API 的第三方服务",
    icon: KeyRound,
  },
  {
    id: "usage",
    label: "用量与费用",
    description: "查看本机 Token、模型与美元估算费用",
    icon: ChartNoAxesCombined,
  },
  {
    id: "sessions",
    label: "历史会话",
    description: "查看本机会话，并仅在需要时更新连接归属",
    icon: MessagesSquare,
  },
  {
    id: "settings",
    label: "配置",
    description: "检查本机 Codex 配置，并在写入前预览变更",
    icon: FileCheck2,
  },
]

export default function App() {
  const contentRef = useRef<HTMLElement>(null)
  const [page, setPage] = useState<Page>("dashboard")
  const [visitedPages, setVisitedPages] = useState<Set<Page>>(
    () => new Set(["dashboard"])
  )
  const currentNavigation = navigation.find((item) => item.id === page)!

  useEffect(() => {
    refreshCoordinator.start()
    return () => refreshCoordinator.stop()
  }, [])

  const navigate = useCallback((nextPage: Page) => {
    contentRef.current?.scrollTo({ top: 0 })
    setVisitedPages((current) => {
      if (current.has(nextPage)) return current
      const next = new Set(current)
      next.add(nextPage)
      return next
    })
    setPage(nextPage)
  }, [])

  useEffect(() => {
    const handleNavigation = (event: Event) => {
      const nextPage = (event as CustomEvent<unknown>).detail
      if (
        nextPage === "dashboard" ||
        nextPage === "providers" ||
        nextPage === "usage" ||
        nextPage === "sessions" ||
        nextPage === "settings"
      ) {
        navigate(nextPage)
      }
    }
    window.addEventListener("codex-tools:navigate", handleNavigation)
    return () =>
      window.removeEventListener("codex-tools:navigate", handleNavigation)
  }, [navigate])

  const CurrentPageIcon = currentNavigation.icon

  return (
    <TooltipProvider>
      <SidebarProvider
        className="h-full overflow-hidden"
        style={{ "--sidebar-width": "14.5rem" } as CSSProperties}
      >
        <AppSidebar page={page} onNavigate={navigate} />
        <SidebarInset className="min-h-0 overflow-hidden">
          <main
            ref={contentRef}
            className="min-h-0 flex-1 overflow-y-auto"
            aria-label={currentNavigation.label}
          >
            <div className="mx-auto flex w-full max-w-6xl flex-col gap-6 px-8 py-7">
              <PageHeader
                title={currentNavigation.label}
                description={currentNavigation.description}
                icon={CurrentPageIcon}
                actions={<ThemeMenu />}
              />
              {navigation
                .filter((item) => visitedPages.has(item.id))
                .map((item) => {
                  const PageComponent = pages[item.id]
                  const active = page === item.id
                  return (
                    <section
                      key={item.id}
                      hidden={!active}
                      aria-hidden={!active}
                      aria-label={item.label}
                    >
                      <Suspense fallback={<PageLoading />}>
                        <PageComponent active={active} />
                      </Suspense>
                    </section>
                  )
                })}
            </div>
          </main>
        </SidebarInset>
      </SidebarProvider>
      <Toaster position="bottom-right" closeButton visibleToasts={4} />
    </TooltipProvider>
  )
}

function AppSidebar({
  page,
  onNavigate,
}: {
  page: Page
  onNavigate: (page: Page) => void
}) {
  return (
    <Sidebar collapsible="none" className="border-r border-sidebar-border">
      <SidebarHeader className="px-3 pt-5">
        <div className="flex h-12 items-center gap-3 px-2">
          <img
            src="/codex-tools.svg"
            alt=""
            className="size-8 shrink-0 rounded-lg"
          />
          <div className="min-w-0">
            <div className="truncate text-base font-semibold">Codex Tools</div>
            <div className="truncate text-xs text-muted-foreground">
              本机连接管理
            </div>
          </div>
        </div>
      </SidebarHeader>

      <SidebarContent className="px-1">
        <SidebarGroup className="pt-4">
          <SidebarGroupLabel>工作台</SidebarGroupLabel>
          <SidebarGroupContent>
            <SidebarMenu>
              {navigation.map((item) => {
                const Icon = item.icon
                return (
                  <SidebarMenuItem key={item.id}>
                    <SidebarMenuButton
                      isActive={page === item.id}
                      tooltip={item.label}
                      aria-current={page === item.id ? "page" : undefined}
                      onFocus={() => void pageLoaders[item.id]()}
                      onPointerEnter={() => void pageLoaders[item.id]()}
                      onClick={() => onNavigate(item.id)}
                    >
                      <Icon aria-hidden="true" />
                      <span>{item.label}</span>
                    </SidebarMenuButton>
                  </SidebarMenuItem>
                )
              })}
            </SidebarMenu>
          </SidebarGroupContent>
        </SidebarGroup>
      </SidebarContent>

      <SidebarFooter>
        <div className="flex items-start gap-2 rounded-lg bg-sidebar-accent px-3 py-3 text-xs leading-relaxed text-muted-foreground">
          <ShieldCheck className="mt-0.5 size-4 shrink-0" aria-hidden="true" />
          <span>凭据仅保存在这台设备上</span>
        </div>
      </SidebarFooter>
    </Sidebar>
  )
}

const themeOptions = [
  { id: "system", label: "跟随系统", icon: Monitor },
  { id: "light", label: "浅色", icon: Sun },
  { id: "dark", label: "深色", icon: Moon },
] as const

function ThemeMenu() {
  const { theme = "system", setTheme } = useTheme()
  const current = themeOptions.find((option) => option.id === theme)
  const CurrentIcon = current?.icon ?? Monitor

  return (
    <DropdownMenu>
      <DropdownMenuTrigger
        render={
          <Button
            variant="outline"
            size="icon"
            aria-label={`界面主题：${current?.label ?? "跟随系统"}`}
            title="选择界面主题"
          />
        }
      >
        <CurrentIcon data-icon="inline-start" />
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="w-40">
        <DropdownMenuGroup>
          {themeOptions.map((option) => {
            const Icon = option.icon
            return (
              <DropdownMenuItem
                key={option.id}
                onClick={() => setTheme(option.id)}
              >
                <Icon aria-hidden="true" />
                <span>{option.label}</span>
                {theme === option.id && (
                  <Check className="ml-auto" aria-hidden="true" />
                )}
              </DropdownMenuItem>
            )
          })}
        </DropdownMenuGroup>
      </DropdownMenuContent>
    </DropdownMenu>
  )
}
