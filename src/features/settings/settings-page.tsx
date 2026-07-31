import { useCallback, useEffect, useMemo, useState } from "react"
import {
  Check,
  Copy,
  Eye,
  Info,
  LogIn,
  RefreshCw,
  TriangleAlert,
} from "lucide-react"

import { ErrorDetails } from "@/components/error-details"
import { SectionHeader } from "@/components/page-header"
import { PageLoading } from "@/components/page-loading"
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
  ItemTitle,
} from "@/components/ui/item"
import { Spinner } from "@/components/ui/spinner"
import { Textarea } from "@/components/ui/textarea"
import { notify } from "@/lib/feedback"
import { call } from "@/lib/ipc"
import { notifyRepairWarnings } from "@/lib/repair-feedback"
import type { ConfigInspection, ConfigPatchPreview, PageProps } from "@/types"

export default function SettingsPage({ active }: PageProps) {
  const [inspection, setInspection] = useState<ConfigInspection>()
  const [diagnostics, setDiagnostics] = useState<Record<string, unknown>>()
  const [canPreviewCustom, setCanPreviewCustom] = useState(false)
  const [preview, setPreview] = useState<ConfigPatchPreview>()
  const [confirmOfficial, setConfirmOfficial] = useState(false)
  const [error, setError] = useState<string>()
  const [previewing, setPreviewing] = useState(false)
  const [applyingPreview, setApplyingPreview] = useState(false)
  const [switchingOfficial, setSwitchingOfficial] = useState(false)
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
  }, [])

  useEffect(() => {
    if (!active) return
    const timeout = window.setTimeout(() => {
      void load().catch((reason) => setError(String(reason)))
    }, 0)
    return () => window.clearTimeout(timeout)
  }, [active, load])

  if (error) {
    return (
      <Alert variant="destructive">
        <TriangleAlert />
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
                <RefreshCw data-icon="inline-start" />
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

  if (!inspection) return <PageLoading label="正在检查 Codex 配置" />

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

  const busy = previewing || applyingPreview || switchingOfficial
  const connectionLabel =
    inspection.activeProvider === "custom"
      ? "第三方 API"
      : inspection.activeProvider === "openai"
        ? "OpenAI 账号"
        : "OpenAI 默认设置"

  return (
    <div className="flex flex-col gap-8">
      <Alert>
        <Info />
        <AlertTitle>只修改连接所需字段</AlertTitle>
        <AlertDescription>
          本应用只管理账号、API 地址、认证信息和连接标记；其他 Codex
          设置保持原样。
        </AlertDescription>
      </Alert>

      <section className="flex flex-col gap-3" aria-labelledby="config-title">
        <SectionHeader
          id="config-title"
          title="配置文件"
          description="检查当前连接，并在写入前预览本应用负责的字段。"
        />

        <div className="grid items-start gap-3 lg:grid-cols-[minmax(0,3fr)_minmax(18rem,2fr)]">
          <Card>
            <CardHeader>
              <CardTitle>当前配置</CardTitle>
              <CardDescription className="truncate" title={inspection.path}>
                {inspection.path}
              </CardDescription>
              <CardAction>
                <Badge variant={inspection.valid ? "default" : "destructive"}>
                  {inspection.valid ? "可以读取" : "需要处理"}
                </Badge>
              </CardAction>
            </CardHeader>
            <CardContent className="flex flex-col gap-4">
              <Item>
                <ItemContent>
                  <ItemTitle>当前连接</ItemTitle>
                  <ItemDescription>{connectionLabel}</ItemDescription>
                </ItemContent>
              </Item>

              {inspection.warnings.length > 0 && (
                <Alert>
                  <TriangleAlert />
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
            <CardFooter className="flex-wrap gap-2">
              <Button
                variant="outline"
                disabled={busy || !canPreviewCustom}
                title={
                  canPreviewCustom
                    ? undefined
                    : "请先在“账号与服务”中使用一个第三方 API"
                }
                onClick={() => void createPreview()}
              >
                {previewing ? (
                  <Spinner data-icon="inline-start" />
                ) : (
                  <Eye data-icon="inline-start" />
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
                  <LogIn data-icon="inline-start" />
                )}
                {switchingOfficial ? "正在切换…" : "使用 OpenAI"}
              </Button>
            </CardFooter>
          </Card>

          <Card>
            <CardHeader>
              <CardTitle>诊断信息</CardTitle>
              <CardDescription>
                可在反馈问题时复制；这里不包含已保存的 API Key 或登录凭据。
              </CardDescription>
            </CardHeader>
            <CardContent>
              <pre className="max-h-72 overflow-auto rounded-lg bg-muted/70 p-3 text-xs leading-relaxed whitespace-pre-wrap">
                {diagnosticsText}
              </pre>
            </CardContent>
            <CardFooter className="justify-end">
              <Button size="sm" variant="outline" onClick={copyDiagnostics}>
                <Copy data-icon="inline-start" />
                复制诊断信息
              </Button>
            </CardFooter>
          </Card>
        </div>
      </section>

      <Dialog
        open={Boolean(preview)}
        onOpenChange={(open) => {
          if (!open && !applyingPreview) setPreview(undefined)
        }}
      >
        <DialogContent className="overflow-hidden sm:max-w-3xl">
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
                <Check data-icon="inline-start" />
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
    </div>
  )
}
