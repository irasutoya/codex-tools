import { useCallback, useEffect, useState } from "react"
import {
  Check,
  FileDiff,
  ShieldCheck,
  TriangleAlert,
  Undo2,
} from "lucide-react"
import { toast } from "sonner"

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
import { Field, FieldDescription, FieldLabel } from "@/components/ui/field"
import { Spinner } from "@/components/ui/spinner"
import { Textarea } from "@/components/ui/textarea"
import { call } from "@/lib/ipc"
import type {
  ConfigInspection,
  ConfigPatchPreview,
  SettingsOverview,
} from "@/types"

export default function SettingsPage() {
  const [inspection, setInspection] = useState<ConfigInspection>()
  const [diagnostics, setDiagnostics] = useState<Record<string, unknown>>()
  const [canPreviewCustom, setCanPreviewCustom] = useState(false)
  const [preview, setPreview] = useState<ConfigPatchPreview>()
  const [confirmOfficial, setConfirmOfficial] = useState(false)
  const [error, setError] = useState<string>()
  const [previewing, setPreviewing] = useState(false)
  const [applyingPreview, setApplyingPreview] = useState(false)
  const [switchingOfficial, setSwitchingOfficial] = useState(false)
  const load = useCallback(async () => {
    const overview = await call<SettingsOverview>("get_settings_overview")
    setInspection(overview.inspection)
    setDiagnostics(overview.diagnostics)
    setCanPreviewCustom(overview.canPreviewCustom)
    setError(undefined)
  }, [])
  useEffect(() => {
    const timeout = window.setTimeout(() => {
      void load().catch((reason) => setError(String(reason)))
    }, 0)
    return () => window.clearTimeout(timeout)
  }, [load])
  if (error) {
    return (
      <Alert variant="destructive">
        <AlertTitle>暂时无法检查 Codex 配置</AlertTitle>
        <AlertDescription>{error}</AlertDescription>
      </Alert>
    )
  }
  if (!inspection) return <PageLoading />

  const createPreview = async () => {
    setPreviewing(true)
    try {
      const next = await call<ConfigPatchPreview>("preview_activation")
      setPreview(next)
    } catch (reason) {
      toast.error(String(reason))
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
      toast.success("Codex 配置已更新")
      await load()
    } catch (reason) {
      toast.error(String(reason))
    } finally {
      setApplyingPreview(false)
    }
  }

  const activateOfficial = async () => {
    setConfirmOfficial(false)
    setSwitchingOfficial(true)
    try {
      await call("activate_official")
      await load()
      toast.success("Codex 已切换到 OpenAI")
    } catch (reason) {
      toast.error(String(reason))
    } finally {
      setSwitchingOfficial(false)
    }
  }

  const busy = previewing || applyingPreview || switchingOfficial

  return (
    <div className="flex flex-col gap-6">
      <Alert>
        <ShieldCheck />
        <AlertTitle>只更新连接所需的设置</AlertTitle>
        <AlertDescription>
          切换账号或第三方 API 时，其他 Codex 设置会保持不变。登录信息和 API Key
          会直接同步到 Codex 自己的凭据文件。
        </AlertDescription>
      </Alert>
      <Card>
        <CardHeader>
          <CardTitle>当前配置</CardTitle>
          <CardDescription className="truncate" title={inspection.path}>
            {inspection.path}
          </CardDescription>
          <CardAction>
            <Badge variant={inspection.valid ? "default" : "destructive"}>
              {inspection.valid ? "可以使用" : "需要处理"}
            </Badge>
          </CardAction>
        </CardHeader>
        <CardContent className="flex flex-col gap-4">
          <dl>
            <div className="flex min-w-0 flex-col gap-1">
              <dt className="text-xs text-muted-foreground">当前连接方式</dt>
              <dd className="truncate font-medium">
                {inspection.activeProvider === "custom"
                  ? "第三方 API"
                  : inspection.activeProvider === "openai"
                    ? "OpenAI 官方账号"
                    : "OpenAI 默认设置"}
              </dd>
            </div>
          </dl>
          {inspection.warnings.length > 0 && (
            <Alert variant="destructive">
              <TriangleAlert />
              <AlertTitle>发现需要处理的问题</AlertTitle>
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
              <FileDiff data-icon="inline-start" />
            )}
            {previewing ? "正在准备预览…" : "预览第三方配置"}
          </Button>
          <Button
            variant="outline"
            disabled={busy}
            onClick={() => setConfirmOfficial(true)}
          >
            {switchingOfficial ? (
              <Spinner data-icon="inline-start" />
            ) : (
              <Undo2 data-icon="inline-start" />
            )}
            {switchingOfficial ? "正在切换…" : "切换到 OpenAI"}
          </Button>
        </CardFooter>
      </Card>
      <Card size="sm">
        <CardHeader>
          <CardTitle>故障排查信息</CardTitle>
          <CardDescription>
            可在反馈问题时使用；API Key 和自定义请求头已经隐藏。
          </CardDescription>
        </CardHeader>
        <CardContent>
          <pre
            className="max-h-72 overflow-auto rounded-xl bg-[var(--md-sys-color-surface-container-highest)] p-4 text-xs leading-5"
            tabIndex={0}
          >
            {JSON.stringify(diagnostics, null, 2)}
          </pre>
        </CardContent>
      </Card>
      <Dialog
        open={Boolean(preview)}
        onOpenChange={(open) => {
          if (!open && !applyingPreview) setPreview(undefined)
        }}
      >
        <DialogContent className="sm:max-w-2xl">
          <DialogHeader>
            <DialogTitle>确认将要写入的配置</DialogTitle>
            <DialogDescription>
              {preview
                ? `${preview.changes.join("；") || "没有检测到字段变化"} · API Key ${preview.apiKeyMasked}`
                : "检查即将交给 Codex 的第三方 API 配置。"}
            </DialogDescription>
          </DialogHeader>
          <Field>
            <FieldLabel htmlFor="config-preview">
              将写入 config.toml 的内容
            </FieldLabel>
            <Textarea
              id="config-preview"
              className="max-h-[60vh] min-h-72"
              readOnly
              value={preview?.rendered ?? ""}
            />
            <FieldDescription>
              此处只能查看。点击“写入配置”后才会修改 Codex。
            </FieldDescription>
          </Field>
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
              {applyingPreview ? "正在写入…" : "写入配置"}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
      <AlertDialog open={confirmOfficial} onOpenChange={setConfirmOfficial}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>切换到 OpenAI 官方服务？</AlertDialogTitle>
            <AlertDialogDescription>
              Codex 将使用当前保存的 OpenAI 账号。第三方 API 服务和其他 Codex
              设置都会保留，之后可以随时切回。若尚未登录，请先前往“账号与服务”。
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
