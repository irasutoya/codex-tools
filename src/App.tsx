import {
  lazy,
  memo,
  Suspense,
  useCallback,
  useEffect,
  useRef,
  useState,
} from "react"
import {
  ChartHistogramIcon,
  CheckIcon,
  ExternalLinkIcon,
  FileCheckIcon,
  Key01Icon,
  DashboardSquare01Icon,
  Message01Icon,
  MonitorDotIcon,
  Moon02Icon,
  Refresh01Icon,
  Sun03Icon,
} from "@hugeicons/core-free-icons"
import { HugeiconsIcon, type IconSvgElement } from "@hugeicons/react"
import { useTheme } from "next-themes"

import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { Skeleton } from "@/components/ui/skeleton"
import { Spinner } from "@/components/ui/spinner"
import { Toaster } from "@/components/ui/sonner"
import { TooltipProvider } from "@/components/ui/tooltip"
import { notify } from "@/lib/feedback"
import { call } from "@/lib/ipc"
import { refreshCoordinator } from "@/lib/refresh-coordinator"
import type { ModelUnlockStatus, Page } from "@/types"

const pageLoaders = {
  dashboard: () => import("@/features/dashboard/dashboard-page"),
  providers: () => import("@/features/providers/providers-page"),
  usage: () => import("@/features/usage/usage-page"),
  sessions: () => import("@/features/sessions/sessions-page"),
  settings: () => import("@/features/settings/settings-page"),
}

const pages = {
  dashboard: memo(lazy(pageLoaders.dashboard)),
  providers: memo(lazy(pageLoaders.providers)),
  usage: memo(lazy(pageLoaders.usage)),
  sessions: memo(lazy(pageLoaders.sessions)),
  settings: memo(lazy(pageLoaders.settings)),
}

type NavigationItem = {
  id: Page
  label: string
  description: string
  icon: IconSvgElement
}

const navigation: NavigationItem[] = [
  {
    id: "dashboard",
    label: "概览",
    description: "当前连接、账号额度和本机会话状态",
    icon: DashboardSquare01Icon,
  },
  {
    id: "providers",
    label: "连接",
    description: "管理 OpenAI 账号与第三方 API 服务",
    icon: Key01Icon,
  },
  {
    id: "usage",
    label: "用量",
    description: "查看本机 Token、模型与美元估算费用",
    icon: ChartHistogramIcon,
  },
  {
    id: "sessions",
    label: "会话",
    description: "查看本机会话，并更新连接归属",
    icon: Message01Icon,
  },
  {
    id: "settings",
    label: "设置",
    description: "检查 Codex 配置、应用与模型解锁",
    icon: FileCheckIcon,
  },
]

export default function App() {
  const contentRef = useRef<HTMLElement>(null)
  const [page, setPage] = useState<Page>("dashboard")
  const [visitedPages, setVisitedPages] = useState<Set<Page>>(
    () => new Set(["dashboard"])
  )

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

  const current = navigation.find((item) => item.id === page)!

  return (
    <TooltipProvider>
      <div className="flex h-full flex-col overflow-hidden">
        {/* 顶栏：左侧导航标签 + 右侧全局操作（所有控件统一 36px 高度对齐） */}
        <header className="flex h-14 shrink-0 items-center gap-2 border-b px-3">
          <nav
            className="flex min-w-0 flex-1 items-center gap-1"
            aria-label="主导航"
          >
            {navigation.map((item) => {
              const Icon = item.icon
              const active = page === item.id
              return (
                <Button
                  key={item.id}
                  variant={active ? "default" : "ghost"}
                  size="default"
                  aria-current={active ? "page" : undefined}
                  aria-label={item.label}
                  title={item.label}
                  onClick={() => navigate(item.id)}
                >
                  <HugeiconsIcon icon={Icon} aria-hidden="true" />
                  <span>{item.label}</span>
                </Button>
              )
            })}
          </nav>

          <div className="flex shrink-0 items-center gap-1.5">
            <Button
              variant="ghost"
              size="icon"
              aria-label="刷新当前页"
              title="刷新当前页"
              onClick={() => refreshCoordinator.invalidate([page])}
            >
              <HugeiconsIcon icon={Refresh01Icon} data-icon="inline-start" />
            </Button>
            <ThemeMenu />
            <QuickLaunch />
          </div>
        </header>

        {/* 内容区：固定窗口，全宽利用 */}
        <main
          ref={contentRef}
          className="min-h-0 flex-1 overflow-y-auto"
          aria-label={current.label}
        >
          <div className="flex w-full flex-col gap-6 p-6 lg:gap-8 lg:p-8">
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
      </div>
      <Toaster position="bottom-right" closeButton visibleToasts={4} />
    </TooltipProvider>
  )
}

function PageLoading() {
  return (
    <div className="flex flex-col gap-4" role="status" aria-live="polite">
      <div className="grid grid-cols-4 gap-3">
        {Array.from({ length: 4 }).map((_, index) => (
          <div key={index} className="flex flex-col gap-2">
            <Skeleton className="h-3 w-16" />
            <Skeleton className="h-6 w-14" />
          </div>
        ))}
      </div>
      <Skeleton className="h-64 w-full" />
    </div>
  )
}

async function launchCodex() {
  try {
    const result = await call("launch_codex")
    notify.success(
      result.injected ? "Codex 已启动并解锁模型列表" : result.message
    )
    refreshCoordinator.invalidate(["dashboard", "settings"])
  } catch (reason) {
    notify.error("无法启动 Codex", reason)
  }
}

/** 顶栏右侧的一键启动（全局快捷操作）。 */
function QuickLaunch() {
  const [launching, setLaunching] = useState(false)
  const [status, setStatus] = useState<ModelUnlockStatus>()

  useEffect(() => {
    let cancelled = false
    call("get_model_unlock_status")
      .then((result) => {
        if (!cancelled) setStatus(result)
      })
      .catch(() => undefined)
    return () => {
      cancelled = true
    }
  }, [])

  const launch = async () => {
    setLaunching(true)
    try {
      await launchCodex()
      setStatus(await call("get_model_unlock_status"))
    } catch {
      // launchCodex 已通过 notify 提示错误。
    } finally {
      setLaunching(false)
    }
  }

  const statusLabel = status?.injected
    ? "已解锁"
    : status?.appFound
      ? "未解锁"
      : "未检测到应用"

  return (
    <div className="flex items-center gap-1.5">
      <Button
        variant="default"
        size="default"
        title="启动 Codex（自动解锁）"
        disabled={launching}
        onClick={() => void launch()}
      >
        {launching ? (
          <Spinner data-icon="inline-start" />
        ) : (
          <HugeiconsIcon icon={ExternalLinkIcon} data-icon="inline-start" />
        )}
        {launching ? "启动中…" : "启动 Codex"}
      </Button>
      <Badge
        variant={
          status?.injected
            ? "default"
            : status?.appFound
              ? "secondary"
              : "outline"
        }
        className="inline-flex"
        title="Codex 模型解锁状态"
      >
        {statusLabel}
      </Badge>
    </div>
  )
}

const themeOptions = [
  { id: "system", label: "跟随系统", icon: MonitorDotIcon },
  { id: "light", label: "浅色", icon: Sun03Icon },
  { id: "dark", label: "深色", icon: Moon02Icon },
] as const

function ThemeMenu() {
  const { theme = "system", setTheme } = useTheme()
  const current = themeOptions.find((option) => option.id === theme)
  const CurrentIcon = current?.icon ?? MonitorDotIcon

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
        <HugeiconsIcon icon={CurrentIcon} data-icon="inline-start" />
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
                <HugeiconsIcon icon={Icon} aria-hidden="true" />
                <span>{option.label}</span>
                {theme === option.id && (
                  <HugeiconsIcon
                    icon={CheckIcon}
                    className="ml-auto"
                    aria-hidden="true"
                  />
                )}
              </DropdownMenuItem>
            )
          })}
        </DropdownMenuGroup>
      </DropdownMenuContent>
    </DropdownMenu>
  )
}
