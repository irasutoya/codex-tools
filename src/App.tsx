import { lazy, Suspense, useState } from "react"
import {
  CircleGauge,
  Database,
  Monitor,
  Moon,
  Server,
  Settings,
  Sun,
  type LucideIcon,
} from "lucide-react"

import { PageLoading } from "@/components/page-loading"
import { useTheme } from "@/components/theme-provider"
import { Button } from "@/components/ui/button"
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
    description: "查看 Codex 当前使用的账号、服务和本机会话状态",
    icon: CircleGauge,
  },
  {
    id: "providers",
    label: "账号与服务",
    description: "登录 OpenAI，或添加兼容 Responses API 的第三方服务",
    icon: Server,
  },
  {
    id: "sessions",
    label: "历史会话",
    description: "查看本机会话，并修复切换连接后的归属信息",
    icon: Database,
  },
  {
    id: "settings",
    label: "配置检查",
    description: "检查 Codex 配置，预览写入内容并获取排查信息",
    icon: Settings,
  },
]

export default function App() {
  const [page, setPage] = useState<Page>("dashboard")
  const [visitedPages, setVisitedPages] = useState<Set<Page>>(
    () => new Set(["dashboard"])
  )
  const currentNavigation = navigation.find((item) => item.id === page)!

  const navigate = (nextPage: Page) => {
    setVisitedPages((current) => {
      if (current.has(nextPage)) return current
      const next = new Set(current)
      next.add(nextPage)
      return next
    })
    setPage(nextPage)
  }

  return (
    <TooltipProvider delay={500}>
      <div className="md3-app-shell">
        <NavigationDrawer page={page} onNavigate={navigate} />
        <NavigationRail page={page} onNavigate={navigate} />

        <div className="md3-main-area">
          <header className="md3-top-app-bar">
            <div className="md3-top-app-bar__identity">
              <img src="/codex-tools.svg" alt="" className="md3-mobile-logo" />
              <div className="min-w-0">
                <h1 className="md3-page-title">{currentNavigation.label}</h1>
                <p className="md3-page-description">
                  {currentNavigation.description}
                </p>
              </div>
            </div>
            <ThemeToggle />
          </header>

          <main
            className="md3-content-scroll"
            aria-label={currentNavigation.label}
          >
            <div className="md3-content-frame">
              {navigation
                .filter((item) => visitedPages.has(item.id))
                .map((item) => {
                  const PageComponent = pages[item.id]
                  const active = page === item.id
                  return (
                    <section
                      key={item.id}
                      className="md3-page-panel"
                      hidden={!active}
                      aria-hidden={!active}
                      aria-label={item.label}
                    >
                      <Suspense fallback={<PageLoading />}>
                        <PageComponent />
                      </Suspense>
                    </section>
                  )
                })}
            </div>
          </main>
        </div>

        <BottomNavigation page={page} onNavigate={navigate} />
      </div>
      <Toaster richColors position="bottom-right" />
    </TooltipProvider>
  )
}

function NavigationDrawer({
  page,
  onNavigate,
}: {
  page: Page
  onNavigate: (page: Page) => void
}) {
  return (
    <aside className="md3-nav-drawer" aria-label="主导航">
      <div className="md3-brand">
        <img src="/codex-tools.svg" alt="" className="md3-brand__logo" />
        <div className="md3-brand__copy">
          <div className="md3-brand__title">Codex Tools</div>
          <div className="md3-brand__subtitle">账号与 Responses API 管理</div>
        </div>
      </div>

      <nav className="md3-nav-list">
        <div className="md3-nav-label">主要功能</div>
        {navigation.map((item) => (
          <NavigationButton
            key={item.id}
            item={item}
            current={page === item.id}
            className="md3-nav-item"
            onNavigate={onNavigate}
          />
        ))}
      </nav>

      <div className="md3-drawer-footer">
        <span>本地保存 · 直接连接 Codex</span>
      </div>
    </aside>
  )
}

function NavigationRail({
  page,
  onNavigate,
}: {
  page: Page
  onNavigate: (page: Page) => void
}) {
  return (
    <aside className="md3-nav-rail" aria-label="主导航">
      <img src="/codex-tools.svg" alt="Codex Tools" className="md3-rail-logo" />
      <nav className="md3-rail-list">
        {navigation.map((item) => (
          <NavigationButton
            key={item.id}
            item={item}
            current={page === item.id}
            className="md3-rail-item"
            onNavigate={onNavigate}
            compact
          />
        ))}
      </nav>
      <ThemeToggle />
    </aside>
  )
}

function BottomNavigation({
  page,
  onNavigate,
}: {
  page: Page
  onNavigate: (page: Page) => void
}) {
  return (
    <nav className="md3-bottom-nav" aria-label="主导航">
      {navigation.map((item) => (
        <NavigationButton
          key={item.id}
          item={item}
          current={page === item.id}
          className="md3-bottom-item"
          onNavigate={onNavigate}
          compact
        />
      ))}
    </nav>
  )
}

function NavigationButton({
  item,
  current,
  className,
  onNavigate,
  compact = false,
}: {
  item: NavigationItem
  current: boolean
  className: string
  onNavigate: (page: Page) => void
  compact?: boolean
}) {
  const Icon = item.icon

  return (
    <button
      type="button"
      className={className}
      aria-current={current ? "page" : undefined}
      aria-label={compact ? item.label : undefined}
      onFocus={() => void pageLoaders[item.id]()}
      onPointerEnter={() => void pageLoaders[item.id]()}
      onClick={() => onNavigate(item.id)}
    >
      {compact ? (
        <>
          <span
            className={
              className === "md3-bottom-item"
                ? "md3-bottom-indicator"
                : "md3-rail-indicator"
            }
          >
            <Icon aria-hidden="true" />
          </span>
          <span>{item.label}</span>
        </>
      ) : (
        <>
          <Icon aria-hidden="true" />
          <span>{item.label}</span>
        </>
      )}
    </button>
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
            size="icon"
            aria-label={`切换界面主题，当前为${option.label}`}
            onClick={cycleTheme}
          />
        }
      >
        <Icon />
      </TooltipTrigger>
      <TooltipContent side="bottom">界面主题：{option.label}</TooltipContent>
    </Tooltip>
  )
}
