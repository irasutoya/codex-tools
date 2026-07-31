import { lazy, Suspense, useRef, useState } from "react"
import {
  Check,
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

import { PageLoading } from "@/components/page-loading"
import { Button } from "@/components/ui/button"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { Separator } from "@/components/ui/separator"
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
  SidebarRail,
  SidebarTrigger,
  useSidebar,
} from "@/components/ui/sidebar"
import { Toaster } from "@/components/ui/sonner"
import { TooltipProvider } from "@/components/ui/tooltip"
import type { Page } from "@/types"

const pageLoaders = {
  dashboard: () => import("@/features/dashboard/dashboard-page"),
  providers: () => import("@/features/providers/providers-page"),
  sessions: () => import("@/features/sessions/sessions-page"),
  settings: () => import("@/features/settings/settings-page"),
}

const pages = {
  dashboard: lazy(pageLoaders.dashboard),
  providers: lazy(pageLoaders.providers),
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
    description: "查看 Codex 当前连接和本机会话状态",
    icon: LayoutDashboard,
  },
  {
    id: "providers",
    label: "账号与服务",
    description: "管理 OpenAI 账号和兼容 Responses API 的服务",
    icon: KeyRound,
  },
  {
    id: "sessions",
    label: "历史会话",
    description: "查看本机会话，并按需更新连接标记",
    icon: MessagesSquare,
  },
  {
    id: "settings",
    label: "配置",
    description: "检查 Codex 配置，并在写入前预览修改",
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

  const navigate = (nextPage: Page) => {
    contentRef.current?.scrollTo({ top: 0 })
    setVisitedPages((current) => {
      if (current.has(nextPage)) return current
      const next = new Set(current)
      next.add(nextPage)
      return next
    })
    setPage(nextPage)
  }

  return (
    <TooltipProvider>
      <SidebarProvider className="h-full overflow-hidden">
        <AppSidebar page={page} onNavigate={navigate} />
        <SidebarInset className="min-h-0 overflow-hidden">
          <header className="flex h-16 shrink-0 items-center gap-3 border-b px-4 sm:px-6">
            <SidebarTrigger
              aria-label="展开或收起导航"
              title="展开或收起导航"
            />
            <Separator orientation="vertical" className="h-4" />
            <div className="min-w-0 flex-1">
              <h1 className="truncate text-base font-medium">
                {currentNavigation.label}
              </h1>
              <p className="hidden truncate text-sm text-muted-foreground sm:block">
                {currentNavigation.description}
              </p>
            </div>
            <ThemeMenu />
          </header>

          <main
            ref={contentRef}
            className="min-h-0 flex-1 overflow-y-auto"
            aria-label={currentNavigation.label}
          >
            <div className="mx-auto flex w-full max-w-6xl flex-col p-4 sm:p-6 lg:p-8">
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
  const { setOpenMobile } = useSidebar()

  const handleNavigate = (nextPage: Page) => {
    onNavigate(nextPage)
    setOpenMobile(false)
  }

  return (
    <Sidebar collapsible="icon">
      <SidebarHeader>
        <SidebarMenu>
          <SidebarMenuItem>
            <SidebarMenuButton size="lg" tooltip="Codex Tools">
              <img
                src="/codex-tools.svg"
                alt=""
                className="size-8 rounded-md"
              />
              <div className="grid min-w-0 flex-1 text-left leading-tight">
                <span className="truncate font-medium">Codex Tools</span>
                <span className="truncate text-xs text-muted-foreground">
                  Codex 连接管理
                </span>
              </div>
            </SidebarMenuButton>
          </SidebarMenuItem>
        </SidebarMenu>
      </SidebarHeader>

      <SidebarContent>
        <SidebarGroup>
          <SidebarGroupLabel>导航</SidebarGroupLabel>
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
                      onClick={() => handleNavigate(item.id)}
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
        <div className="flex items-center gap-2 px-2 py-1 text-xs text-muted-foreground group-data-[collapsible=icon]:hidden">
          <ShieldCheck className="size-4 shrink-0" aria-hidden="true" />
          <span>账号凭据和 API Key 存储在本机</span>
        </div>
      </SidebarFooter>
      <SidebarRail aria-label="展开或收起导航" title="展开或收起导航" />
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
            variant="ghost"
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
