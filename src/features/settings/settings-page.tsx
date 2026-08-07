import { useCallback, useEffect, useMemo, useRef, useState } from "react"
import {
  CheckIcon,
  Copy01Icon,
  EyeIcon,
  FileCheckIcon,
  InformationCircleIcon,
  Layers01Icon,
  Login01Icon,
  MagicWand01Icon,
  Refresh01Icon,
  Rocket01Icon,
  Alert01Icon,
  Wifi01Icon,
  Folder01Icon,
} from "@hugeicons/core-free-icons"
import { HugeiconsIcon } from "@hugeicons/react"
import { open } from "@tauri-apps/plugin-dialog"

import { ErrorDetails } from "@/components/error-details"
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
  CardAction,
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
  Field,
  FieldDescription,
  FieldGroup,
  FieldLabel,
} from "@/components/ui/field"
import {
  Item,
  ItemContent,
  ItemDescription,
  ItemGroup,
  ItemMedia,
  ItemSeparator,
  ItemTitle,
} from "@/components/ui/item"
import { Skeleton } from "@/components/ui/skeleton"
import { Spinner } from "@/components/ui/spinner"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { Textarea } from "@/components/ui/textarea"
import { notify } from "@/lib/feedback"
import { call } from "@/lib/ipc"
import {
  refreshCoordinator,
  useAppForeground,
  usePageRefresh,
} from "@/lib/refresh-coordinator"
import { notifyRepairWarnings } from "@/lib/repair-feedback"
import type {
  CodexAppSetting,
  ConfigInspection,
  ConfigPatchPreview,
  ModelUnlockStatus,
  PageProps,
} from "@/types"

export default function SettingsPage({ active }: PageProps) {
  const refreshSignal = usePageRefresh("settings")
  const foreground = useAppForeground()
  const [inspection, setInspection] = useState<ConfigInspection>()
  const [diagnostics, setDiagnostics] = useState<Record<string, unknown>>()
  const [canPreviewCustom, setCanPreviewCustom] = useState(false)
  const [preview, setPreview] = useState<ConfigPatchPreview>()
  const [confirmOfficial, setConfirmOfficial] = useState(false)
  const [error, setError] = useState<string>()
  const [previewing, setPreviewing] = useState(false)
  const [applyingPreview, setApplyingPreview] = useState(false)
  const [switchingOfficial, setSwitchingOfficial] = useState(false)
  const [unlockStatus, setUnlockStatus] = useState<ModelUnlockStatus>()
  const [unlockBusy, setUnlockBusy] = useState(false)
  const [codexApp, setCodexApp] = useState<CodexAppSetting>()
  const [confirmRelaunch, setConfirmRelaunch] = useState(false)
  const lastRefreshRevision = useRef<number | undefined>(undefined)
  const initialized = useRef(false)
  const diagnosticsText = useMemo(
    () => JSON.stringify(diagnostics ?? {}, null, 2),
    [diagnostics]
  )

  const load = useCallback(async () => {
    const overview = await call("get_settings_overview")
    setInspection(overview.inspection)
    setDiagnostics(overview.diagnostics)
    setCanPreviewCustom(overview.canPreviewCustom)
    setError(undefined)
    try {
      setUnlockStatus(await call("get_model_unlock_status"))
    } catch {
      // 模型解锁状态只是附加信息，失败时不阻塞配置页。
    }
    try {
      setCodexApp(await call("get_codex_app_setting"))
    } catch {
      // Codex 应用路径只是附加信息，失败时不阻塞配置页。
    }
  }, [])

  useEffect(() => {
    if (!active) return
    const firstLoad = !initialized.current
    if (
      !firstLoad &&
      (!foreground || lastRefreshRevision.current === refreshSignal.revision)
    ) {
      return
    }
    const timeout = window.setTimeout(() => {
      initialized.current = true
      lastRefreshRevision.current = refreshSignal.revision
      void load().catch((reason) => setError(String(reason)))
    }, 0)
    return () => window.clearTimeout(timeout)
  }, [active, foreground, load, refreshSignal.revision])

  if (error) {
    return (
      <Alert variant="destructive">
        <HugeiconsIcon icon={Alert01Icon} />
        <AlertTitle>无法检查 Codex 配置</AlertTitle>
        <AlertDescription>
          <ErrorDetails
            error={error}
            action={
              <Button
                variant="outline"
                size="sm"
                onClick={() => {
                  setError(undefined)
                  void load().catch((reason) => setError(String(reason)))
                }}
              >
                <HugeiconsIcon icon={Refresh01Icon} data-icon="inline-start" />
                重试
              </Button>
            }
          >
            请确认本应用可以访问 Codex 配置目录。
          </ErrorDetails>
        </AlertDescription>
      </Alert>
    )
  }

  if (!inspection) return <SettingsLoading />

  const createPreview = async () => {
    setPreviewing(true)
    try {
      setPreview(await call("preview_activation"))
    } catch (reason) {
      notify.error("无法生成配置预览", reason)
    } finally {
      setPreviewing(false)
    }
  }

  const applyPreview = async () => {
    if (!preview) return
    setApplyingPreview(true)
    try {
      await call("apply_activation", { operationId: preview.operationId })
      setPreview(undefined)
      notify.success("Codex 配置已更新")
      refreshCoordinator.invalidate(["dashboard", "providers"])
      try {
        await load()
      } catch (reason) {
        notify.warning("配置已写入，但无法读取最新状态", reason)
      }
    } catch (reason) {
      notify.error("无法写入 Codex 配置", reason)
    } finally {
      setApplyingPreview(false)
    }
  }

  const activateOfficial = async () => {
    setConfirmOfficial(false)
    setSwitchingOfficial(true)
    try {
      const repair = await call("activate_official")
      notify.success("Codex 已切换到 OpenAI")
      notifyRepairWarnings(repair)
      refreshCoordinator.invalidate(["dashboard", "providers"])
      try {
        await load()
      } catch (reason) {
        notify.warning("连接已切换，但无法读取最新状态", reason)
      }
    } catch (reason) {
      notify.error("无法切换到 OpenAI", reason)
    } finally {
      setSwitchingOfficial(false)
    }
  }

  const copyDiagnostics = () => {
    void navigator.clipboard
      .writeText(diagnosticsText)
      .then(() => notify.success("诊断信息已复制"))
      .catch((reason) => notify.error("无法复制诊断信息", reason))
  }

  const refreshUnlockStatus = async () => {
    setUnlockBusy(true)
    try {
      setUnlockStatus(await call("get_model_unlock_status"))
    } catch (reason) {
      notify.error("无法读取模型解锁状态", reason)
    } finally {
      setUnlockBusy(false)
    }
  }

  const chooseCodexApp = async () => {
    const isWindows = /Windows/i.test(navigator.userAgent)
    const picked = await open({
      multiple: false,
      // macOS 的 .app 是目录；Windows 选择可执行文件。
      directory: !isWindows,
      filters: isWindows
        ? [{ name: "Codex 可执行文件", extensions: ["exe"] }]
        : undefined,
      title: "选择 Codex 桌面应用",
    })
    if (!picked || typeof picked !== "string") return
    try {
      await call("save_codex_app_path", { path: picked })
      setCodexApp(await call("get_codex_app_setting"))
      setUnlockStatus(await call("get_model_unlock_status"))
      notify.success("已保存 Codex 应用路径")
    } catch (reason) {
      notify.error("无法保存 Codex 应用路径", reason)
    }
  }

  const resetCodexApp = async () => {
    try {
      await call("save_codex_app_path", { path: null })
      setCodexApp(await call("get_codex_app_setting"))
      setUnlockStatus(await call("get_model_unlock_status"))
      notify.success("已恢复自动检测 Codex 应用")
    } catch (reason) {
      notify.error("无法恢复自动检测", reason)
    }
  }

  const unlockModels = async () => {
    setUnlockBusy(true)
    try {
      const result = await call("unlock_codex_models")
      notify.success(result.message)
      setUnlockStatus(await call("get_model_unlock_status"))
    } catch (reason) {
      notify.error("无法解锁模型列表", reason)
    } finally {
      setUnlockBusy(false)
    }
  }

  const relaunchAndUnlock = async () => {
    setConfirmRelaunch(false)
    setUnlockBusy(true)
    try {
      const result = await call("launch_codex_with_debug")
      notify.success(result.message)
      setUnlockStatus(await call("get_model_unlock_status"))
    } catch (reason) {
      notify.error("无法以调试模式重启 Codex", reason)
    } finally {
      setUnlockBusy(false)
    }
  }

  const busy = previewing || applyingPreview || switchingOfficial
  const connectionLabel =
    inspection.activeProvider === "custom"
      ? "第三方 API"
      : inspection.activeProvider === "openai"
        ? "OpenAI 账号"
        : "OpenAI 默认设置"

  return (
    <div className="flex flex-col gap-6">
      <p className="max-w-prose text-sm text-muted-foreground">
        检查本机 Codex 配置并在写入前预览变更；本应用只管理账号、API
        地址、认证信息和连接标记，其他 Codex 设置保持原样。
      </p>

      <Tabs defaultValue="config">
        <TabsList>
          <TabsTrigger value="config">
            <HugeiconsIcon icon={FileCheckIcon} aria-hidden="true" />
            配置文件
          </TabsTrigger>
          <TabsTrigger value="codex-app">
            <HugeiconsIcon icon={Layers01Icon} aria-hidden="true" />
            Codex 应用
          </TabsTrigger>
          <TabsTrigger value="unlock">
            <HugeiconsIcon icon={MagicWand01Icon} aria-hidden="true" />
            模型解锁
          </TabsTrigger>
        </TabsList>
        <TabsContent value="config" className="flex flex-col gap-6">
          <Tabs defaultValue="config">
            <TabsList>
              <TabsTrigger value="config">当前配置</TabsTrigger>
              <TabsTrigger value="diagnostics">诊断信息</TabsTrigger>
            </TabsList>
            <TabsContent value="config">
              <Card>
                <CardHeader>
                  <CardTitle>当前配置</CardTitle>
                  <CardDescription className="truncate" title={inspection.path}>
                    {inspection.path}
                  </CardDescription>
                  <CardAction>
                    <Badge
                      variant={inspection.valid ? "default" : "destructive"}
                    >
                      {inspection.valid ? "可以读取" : "需要处理"}
                    </Badge>
                  </CardAction>
                </CardHeader>
                <CardContent className="flex flex-col gap-6">
                  <Item>
                    <ItemMedia variant="icon">
                      <HugeiconsIcon icon={FileCheckIcon} />
                    </ItemMedia>
                    <ItemContent>
                      <ItemTitle>当前连接</ItemTitle>
                      <ItemDescription>{connectionLabel}</ItemDescription>
                    </ItemContent>
                  </Item>

                  {inspection.warnings.length > 0 && (
                    <Alert>
                      <HugeiconsIcon icon={Alert01Icon} />
                      <AlertTitle>发现需要确认的配置</AlertTitle>
                      <AlertDescription>
                        <ul className="flex list-disc flex-col gap-1 pl-4">
                          {inspection.warnings.map((warning) => (
                            <li key={warning}>{warning}</li>
                          ))}
                        </ul>
                      </AlertDescription>
                    </Alert>
                  )}
                </CardContent>
                <CardFooter className="flex-wrap gap-3">
                  <Button
                    variant="outline"
                    disabled={busy || !canPreviewCustom}
                    title={
                      canPreviewCustom
                        ? undefined
                        : "请先在“账号与服务”中使用一个 API 服务"
                    }
                    onClick={() => void createPreview()}
                  >
                    {previewing ? (
                      <Spinner data-icon="inline-start" />
                    ) : (
                      <HugeiconsIcon icon={EyeIcon} data-icon="inline-start" />
                    )}
                    {previewing ? "正在准备…" : "预览修改"}
                  </Button>
                  <Button
                    variant="outline"
                    disabled={busy}
                    onClick={() => setConfirmOfficial(true)}
                  >
                    {switchingOfficial ? (
                      <Spinner data-icon="inline-start" />
                    ) : (
                      <HugeiconsIcon
                        icon={Login01Icon}
                        data-icon="inline-start"
                      />
                    )}
                    {switchingOfficial ? "正在切换…" : "使用 OpenAI"}
                  </Button>
                </CardFooter>
              </Card>
            </TabsContent>
            <TabsContent value="diagnostics">
              <Card>
                <CardHeader>
                  <CardTitle>诊断信息</CardTitle>
                  <CardDescription>
                    可在反馈问题时复制；这里不包含已保存的 API Key 或登录凭据。
                  </CardDescription>
                </CardHeader>
                <CardContent>
                  <pre className="max-h-72 overflow-auto rounded-lg bg-muted p-3 text-xs leading-relaxed whitespace-pre-wrap">
                    {diagnosticsText}
                  </pre>
                </CardContent>
                <CardFooter className="justify-end">
                  <Button size="sm" variant="outline" onClick={copyDiagnostics}>
                    <HugeiconsIcon icon={Copy01Icon} data-icon="inline-start" />
                    复制诊断信息
                  </Button>
                </CardFooter>
              </Card>
            </TabsContent>
          </Tabs>
        </TabsContent>

        <TabsContent value="codex-app" className="flex flex-col gap-6">
          <Card>
            <CardHeader>
              <CardTitle>运行程序</CardTitle>
              <CardAction>
                <Badge
                  variant={
                    codexApp?.configured || codexApp?.detected
                      ? "default"
                      : "destructive"
                  }
                >
                  {codexApp?.configured || codexApp?.detected
                    ? "已识别"
                    : "未识别到"}
                </Badge>
              </CardAction>
            </CardHeader>
            <CardContent className="flex flex-col gap-6">
              <ItemGroup className="gap-0">
                <Item>
                  <ItemMedia variant="icon">
                    <HugeiconsIcon icon={Layers01Icon} />
                  </ItemMedia>
                  <ItemContent className="min-w-0">
                    <ItemTitle>应用路径</ItemTitle>
                    <ItemDescription
                      className="truncate"
                      title={codexApp?.configured ?? codexApp?.detected}
                    >
                      {codexApp?.configured ?? codexApp?.detected ?? "尚未找到"}
                    </ItemDescription>
                  </ItemContent>
                </Item>
                {codexApp?.configured && (
                  <>
                    <ItemSeparator />
                    <Item>
                      <ItemMedia variant="icon">
                        <HugeiconsIcon icon={Wifi01Icon} />
                      </ItemMedia>
                      <ItemContent className="min-w-0">
                        <ItemTitle>自动检测</ItemTitle>
                        <ItemDescription className="truncate">
                          {codexApp.detected ?? "未检测到"}
                        </ItemDescription>
                      </ItemContent>
                    </Item>
                  </>
                )}
              </ItemGroup>
            </CardContent>
            <CardFooter className="flex-wrap gap-3">
              <Button variant="outline" onClick={() => void chooseCodexApp()}>
                <HugeiconsIcon icon={Folder01Icon} data-icon="inline-start" />
                手动选择…
              </Button>
              {codexApp?.configured && (
                <Button variant="ghost" onClick={() => void resetCodexApp()}>
                  <HugeiconsIcon
                    icon={Refresh01Icon}
                    data-icon="inline-start"
                  />
                  恢复自动检测
                </Button>
              )}
            </CardFooter>
          </Card>
        </TabsContent>

        <TabsContent value="unlock" className="flex flex-col gap-6">
          <Card>
            <CardHeader>
              <CardTitle>解锁模型列表</CardTitle>
              <CardDescription>
                模型目录 = 当前服务商实际存在的模型（服务 /models 接口返回的可用
                模型），不包含内置 GPT 模型
              </CardDescription>
              <CardAction>
                <Badge
                  variant={
                    unlockStatus?.injected
                      ? "default"
                      : unlockStatus?.debugPort
                        ? "secondary"
                        : "outline"
                  }
                >
                  {unlockStatus?.injected
                    ? "已解锁"
                    : unlockStatus?.debugPort
                      ? "可解锁"
                      : "未连接"}
                </Badge>
              </CardAction>
            </CardHeader>
            <CardContent className="flex flex-col gap-6">
              <ItemGroup className="gap-0">
                <Item>
                  <ItemMedia variant="icon">
                    <HugeiconsIcon icon={Layers01Icon} />
                  </ItemMedia>
                  <ItemContent className="min-w-0">
                    <ItemTitle>模型目录</ItemTitle>
                    <ItemDescription
                      className="truncate"
                      title={unlockStatus?.models.join("、")}
                    >
                      {unlockStatus
                        ? `${unlockStatus.modelCount} 个 · ${unlockStatus.models.slice(0, 6).join("、")}${unlockStatus.modelCount > 6 ? " 等" : ""}`
                        : "正在读取…"}
                    </ItemDescription>
                  </ItemContent>
                </Item>
                <ItemSeparator />
                <Item>
                  <ItemMedia variant="icon">
                    <HugeiconsIcon icon={Wifi01Icon} />
                  </ItemMedia>
                  <ItemContent className="min-w-0">
                    <ItemTitle>调试端口</ItemTitle>
                    <ItemDescription className="font-mono">
                      {unlockStatus?.debugPort
                        ? `127.0.0.1:${unlockStatus.debugPort}`
                        : "未开启"}
                    </ItemDescription>
                  </ItemContent>
                </Item>
              </ItemGroup>

              {unlockStatus?.warning && (
                <Alert>
                  <HugeiconsIcon icon={InformationCircleIcon} />
                  <AlertTitle>需要处理</AlertTitle>
                  <AlertDescription>{unlockStatus.warning}</AlertDescription>
                </Alert>
              )}
            </CardContent>
            <CardFooter className="flex-wrap gap-3">
              <Button
                variant="outline"
                disabled={unlockBusy}
                onClick={() => void refreshUnlockStatus()}
              >
                {unlockBusy ? (
                  <Spinner data-icon="inline-start" />
                ) : (
                  <HugeiconsIcon
                    icon={Refresh01Icon}
                    data-icon="inline-start"
                  />
                )}
                {unlockBusy ? "正在检查…" : "刷新状态"}
              </Button>
              <Button
                disabled={unlockBusy || !unlockStatus?.debugPort}
                onClick={() => void unlockModels()}
              >
                {unlockBusy ? (
                  <Spinner data-icon="inline-start" />
                ) : (
                  <HugeiconsIcon
                    icon={MagicWand01Icon}
                    data-icon="inline-start"
                  />
                )}
                {unlockStatus?.injected ? "刷新解锁目录" : "解锁模型列表"}
              </Button>
              <Button
                variant="outline"
                disabled={unlockBusy}
                onClick={() => setConfirmRelaunch(true)}
              >
                <HugeiconsIcon icon={Rocket01Icon} data-icon="inline-start" />
                以调试模式重启 Codex 并解锁
              </Button>
            </CardFooter>
          </Card>
        </TabsContent>
      </Tabs>

      <Dialog
        open={Boolean(preview)}
        onOpenChange={(open) => {
          if (!open && !applyingPreview) setPreview(undefined)
        }}
      >
        <DialogContent className="max-h-[calc(100dvh-2rem)] overflow-y-auto sm:max-w-3xl">
          <DialogHeader>
            <DialogTitle>预览配置修改</DialogTitle>
            <DialogDescription>
              {preview
                ? `${preview.changes.join("；") || "没有检测到字段变化"} · API Key ${preview.apiKeyMasked}`
                : "检查即将写入 Codex 的第三方 API 配置。"}
            </DialogDescription>
          </DialogHeader>
          <FieldGroup>
            <Field>
              <FieldLabel htmlFor="config-preview">
                config.toml 修改预览
              </FieldLabel>
              <Textarea
                id="config-preview"
                className="field-sizing-fixed h-[min(24rem,50dvh)] min-h-40 resize-none overflow-auto font-mono text-xs"
                readOnly
                value={preview?.rendered ?? ""}
              />
              <FieldDescription>确认后才会写入配置文件。</FieldDescription>
            </Field>
          </FieldGroup>
          <DialogFooter>
            <Button
              variant="outline"
              disabled={applyingPreview}
              onClick={() => setPreview(undefined)}
            >
              取消
            </Button>
            <Button
              disabled={applyingPreview || !preview}
              onClick={() => void applyPreview()}
            >
              {applyingPreview ? (
                <Spinner data-icon="inline-start" />
              ) : (
                <HugeiconsIcon icon={CheckIcon} data-icon="inline-start" />
              )}
              {applyingPreview ? "正在写入…" : "确认写入"}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <AlertDialog open={confirmOfficial} onOpenChange={setConfirmOfficial}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>切换到 OpenAI 账号？</AlertDialogTitle>
            <AlertDialogDescription>
              Codex 将使用最近使用或更新的 OpenAI 账号。已保存的第三方 API
              服务和其他配置会保留；如果没有可用账号，请先前往“账号与服务”登录。
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>取消</AlertDialogCancel>
            <AlertDialogAction
              disabled={switchingOfficial}
              onClick={() => void activateOfficial()}
            >
              {switchingOfficial && <Spinner data-icon="inline-start" />}
              {switchingOfficial ? "正在切换…" : "确认切换"}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>

      <AlertDialog open={confirmRelaunch} onOpenChange={setConfirmRelaunch}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>以调试模式重启 Codex？</AlertDialogTitle>
            <AlertDialogDescription>
              如果 Codex
              已在调试模式下运行，将直接刷新模型目录；否则先退出正在运行
              的实例，再用调试模式重新打开并自动注入解锁脚本。当前窗口中的会话会保留，
              但请先保存手头的工作。
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>取消</AlertDialogCancel>
            <AlertDialogAction
              disabled={unlockBusy}
              onClick={() => void relaunchAndUnlock()}
            >
              {unlockBusy && <Spinner data-icon="inline-start" />}
              {unlockBusy ? "正在重启并解锁…" : "确认重启并解锁"}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  )
}

function SettingsLoading() {
  return (
    <div className="flex flex-col gap-6" role="status" aria-live="polite">
      <div className="flex flex-col gap-2">
        <Skeleton className="h-7 w-24" />
        <Skeleton className="h-4 w-96" />
      </div>
      <div className="flex flex-col gap-4 lg:flex-row">
        <Skeleton className="h-48 w-52" />
        <Skeleton className="h-64 flex-1" />
      </div>
    </div>
  )
}
