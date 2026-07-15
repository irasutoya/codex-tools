import { lazy, Suspense, useState } from "react"
import {
  CircleGauge,
  Database,
  Monitor,
  Moon,
  Network,
  Server,
  Settings,
  Sun,
} from "lucide-react"

import { PageLoading } from "@/components/page-loading"
import { useTheme } from "@/components/theme-provider"
import { Button } from "@/components/ui/button"
import {
  Sidebar,
  SidebarContent,
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
} from "@/components/ui/sidebar"
import { Toaster } from "@/components/ui/sonner"
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip"
import type { Page } from "@/types"

const pageLoaders = {
  dashboard: () => import("@/features/dashboard/dashboard-page"),
  providers: () => import("@/features/providers/providers-page"),
  routes: () => import("@/features/routes/route-page"),
  sessions: () => import("@/features/sessions/sessions-page"),
  settings: () => import("@/features/settings/settings-page"),
}

const pages = {
  dashboard: lazy(pageLoaders.dashboard),
  providers: lazy(pageLoaders.providers),
  routes: lazy(pageLoaders.routes),
  sessions: lazy(pageLoaders.sessions),
  settings: lazy(pageLoaders.settings),
}

const navigation = [
  {
    id: "dashboard" as const,
    label: "概览",
    description: "查看运行模式与本地数据状态",
    icon: CircleGauge,
  },
  {
    id: "providers" as const,
    label: "供应商与账号",
    description: "管理官方账号、第三方供应商和 API 凭据",
    icon: Server,
  },
  {
    id: "routes" as const,
    label: "本地代理",
    description: "配置本地入口并观察请求状态",
    icon: Network,
  },
  {
    id: "sessions" as const,
    label: "会话迁移",
    description: "检查会话索引并统一迁移 provider 标记",
    icon: Database,
  },
  {
    id: "settings" as const,
    label: "配置与诊断",
    description: "预览配置变更并查看脱敏诊断信息",
    icon: Settings,
  },
]

export default function App() {
  const [page, setPage] = useState<Page>("dashboard")
  const CurrentPage = pages[page]
  const currentNavigation = navigation.find((item) => item.id === page)!

  return (
    <TooltipProvider>
      <SidebarProvider className="h-svh overflow-hidden">
        <Sidebar collapsible="icon">
          <SidebarHeader className="border-b px-3 py-3">
            <div className="flex items-center gap-3 overflow-hidden">
              <img
                src="/codex-tools.svg"
                alt=""
                className="size-9 shrink-0 rounded-lg"
              />
              <div className="min-w-0 group-data-[collapsible=icon]:hidden">
                <div className="truncate font-semibold">Codex Tools</div>
                <div className="truncate text-xs text-muted-foreground">
                  轻量本地 Responses 代理
                </div>
              </div>
            </div>
          </SidebarHeader>
          <SidebarContent>
            <SidebarGroup>
              <SidebarGroupLabel>管理</SidebarGroupLabel>
              <SidebarGroupContent>
                <SidebarMenu>
                  {navigation.map((item) => (
                    <SidebarMenuItem key={item.id}>
                      <SidebarMenuButton
                        isActive={page === item.id}
                        tooltip={item.label}
                        aria-current={page === item.id ? "page" : undefined}
                        onFocus={() => void pageLoaders[item.id]()}
                        onPointerEnter={() => void pageLoaders[item.id]()}
                        onClick={() => setPage(item.id)}
                      >
                        <item.icon />
                        <span>{item.label}</span>
                      </SidebarMenuButton>
                    </SidebarMenuItem>
                  ))}
                </SidebarMenu>
              </SidebarGroupContent>
            </SidebarGroup>
          </SidebarContent>
          <SidebarRail />
        </Sidebar>
        <SidebarInset className="min-w-0 overflow-hidden">
          <header className="flex h-16 shrink-0 items-center justify-between gap-4 border-b px-4 sm:px-6">
            <div className="flex min-w-0 items-center gap-3">
              <SidebarTrigger className="shrink-0" />
              <div className="min-w-0">
                <h1 className="truncate text-base font-medium">
                  {currentNavigation.label}
                </h1>
                <div className="truncate text-xs text-muted-foreground">
                  {currentNavigation.description}
                </div>
              </div>
            </div>
            <ThemeToggle />
          </header>
          <section
            aria-label={currentNavigation.label}
            className="min-h-0 flex-1 overflow-y-auto"
          >
            <div className="mx-auto w-full max-w-6xl px-4 py-5 sm:px-6 lg:px-8">
              <Suspense fallback={<PageLoading />}>
                <CurrentPage />
              </Suspense>
            </div>
          </section>
        </SidebarInset>
      </SidebarProvider>
      <Toaster richColors />
    </TooltipProvider>
  )
}

const themeOptions = {
  system: { label: "跟随系统", icon: Monitor },
  light: { label: "浅色", icon: Sun },
  dark: { label: "深色", icon: Moon },
} as const

function ThemeToggle() {
  const { theme, setTheme } = useTheme()
  const option = themeOptions[theme]
  const Icon = option.icon

  const cycleTheme = () => {
    setTheme(
      theme === "system" ? "light" : theme === "light" ? "dark" : "system"
    )
  }

  return (
    <Tooltip>
      <TooltipTrigger
        render={
          <Button
            variant="ghost"
            size="icon-sm"
            aria-label={`当前主题：${option.label}`}
            onClick={cycleTheme}
          />
        }
      >
        <Icon />
      </TooltipTrigger>
      <TooltipContent side="bottom">主题：{option.label}</TooltipContent>
    </Tooltip>
  )
}
