import { useCallback, useEffect, useState } from "react"
import {
  Check,
  FileDiff,
  RotateCw,
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

type PreviewRequest = "custom" | "token"

export default function SettingsPage() {
  const [inspection, setInspection] = useState<ConfigInspection>()
  const [diagnostics, setDiagnostics] = useState<Record<string, unknown>>()
  const [preview, setPreview] = useState<ConfigPatchPreview>()
  const [confirmOfficial, setConfirmOfficial] = useState(false)
  const [error, setError] = useState<string>()
  const [previewRequest, setPreviewRequest] = useState<PreviewRequest>()
  const [applyingPreview, setApplyingPreview] = useState(false)
  const [switchingOfficial, setSwitchingOfficial] = useState(false)
  const load = useCallback(async () => {
    const overview = await call<SettingsOverview>("get_settings_overview")
    setInspection(overview.inspection)
    setDiagnostics(overview.diagnostics)
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
        <AlertTitle>无法读取 Codex 配置</AlertTitle>
        <AlertDescription>{error}</AlertDescription>
      </Alert>
    )
  }
  if (!inspection) return <PageLoading />

  const createPreview = async (request: PreviewRequest) => {
    setPreviewRequest(request)
    try {
      const next =
        request === "token"
          ? await call<ConfigPatchPreview>("regenerate_compatibility_token")
          : await call<ConfigPatchPreview>("preview_activation", {
              mode: "custom",
            })
      setPreview(next)
    } catch (reason) {
      toast.error(String(reason))
    } finally {
      setPreviewRequest(undefined)
    }
  }

  const applyPreview = async () => {
    if (!preview) return
    setApplyingPreview(true)
    try {
      await call("apply_activation", { operationId: preview.operationId })
      setPreview(undefined)
      toast.success("配置已原子写入")
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
      toast.success("已清空第三方配置并切回 OpenAI 官方账号")
    } catch (reason) {
      toast.error(String(reason))
    } finally {
      setSwitchingOfficial(false)
    }
  }

  const busy = Boolean(previewRequest) || applyingPreview || switchingOfficial

  return (
    <div className="flex flex-col gap-5">
      <Alert>
        <ShieldCheck />
        <AlertTitle>官方账号与第三方 API 互斥</AlertTitle>
        <AlertDescription>
          官方模式会清空整个 config.toml 并写入 auth.json；第三方模式会清空
          auth.json，并清空后写入最小 custom 配置。切换会删除 config.toml
          中原有的 MCP、Skills、Hooks、沙箱及未知字段。
        </AlertDescription>
      </Alert>
      <Card>
        <CardHeader>
          <CardTitle>Codex 配置</CardTitle>
          <CardDescription className="truncate" title={inspection.path}>
            {inspection.path}
          </CardDescription>
          <CardAction>
            <Badge variant={inspection.valid ? "default" : "destructive"}>
              {inspection.valid ? "有效" : "无效"}
            </Badge>
          </CardAction>
        </CardHeader>
        <CardContent className="flex flex-col gap-4">
          <dl className="grid gap-3 sm:grid-cols-2">
            <div className="flex min-w-0 flex-col gap-1">
              <dt className="text-xs text-muted-foreground">当前 provider</dt>
              <dd className="truncate font-medium">
                {inspection.activeProvider ?? "默认"}
              </dd>
            </div>
            <div className="flex min-w-0 flex-col gap-1">
              <dt className="text-xs text-muted-foreground">模型目录</dt>
              <dd
                className="truncate font-medium"
                title={inspection.modelCatalogPath}
              >
                {inspection.modelCatalogPath}
              </dd>
            </div>
          </dl>
          {inspection.warnings.length > 0 && (
            <Alert variant="destructive">
              <TriangleAlert />
              <AlertTitle>配置需要处理</AlertTitle>
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
            disabled={busy}
            onClick={() => void createPreview("custom")}
          >
            {previewRequest === "custom" ? (
              <Spinner data-icon="inline-start" />
            ) : (
              <FileDiff data-icon="inline-start" />
            )}
            {previewRequest === "custom"
              ? "正在生成预览..."
              : "预览 custom 配置"}
          </Button>
          <Button
            variant="outline"
            disabled={busy}
            onClick={() => void createPreview("token")}
          >
            {previewRequest === "token" ? (
              <Spinner data-icon="inline-start" />
            ) : (
              <RotateCw data-icon="inline-start" />
            )}
            {previewRequest === "token"
              ? "正在生成 token..."
              : "重新生成兼容 token"}
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
            {switchingOfficial ? "正在切换..." : "切回 OpenAI"}
          </Button>
        </CardFooter>
      </Card>
      <Card size="sm">
        <CardHeader>
          <CardTitle>诊断</CardTitle>
          <CardDescription>
            输出已排除 API Key 和完整兼容 token。
          </CardDescription>
        </CardHeader>
        <CardContent>
          <pre
            className="max-h-56 overflow-auto rounded-md bg-muted p-3 text-xs"
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
            <DialogTitle>配置差异预览</DialogTitle>
            <DialogDescription>
              {preview
                ? `${preview.changes.join("；") || "没有检测到字段变化"} · token ${preview.compatibilityTokenMasked}`
                : "检查即将写入的 custom 配置。"}
            </DialogDescription>
          </DialogHeader>
          <Field>
            <FieldLabel htmlFor="config-preview">
              渲染后的 config.toml
            </FieldLabel>
            <Textarea
              id="config-preview"
              className="max-h-[60vh] min-h-72"
              readOnly
              value={preview?.rendered ?? ""}
            />
            <FieldDescription>
              此处为只读预览，确认后才会写入磁盘。
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
              {applyingPreview ? "正在应用..." : "确认应用"}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
      <AlertDialog open={confirmOfficial} onOpenChange={setConfirmOfficial}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>切回 OpenAI？</AlertDialogTitle>
            <AlertDialogDescription>
              程序会清空整个 config.toml（包括
              MCP、Skills、Hooks、沙箱和未知字段）， 使用当前保存的官方账号重写
              auth.json、迁移受管会话并停止本地代理。
              如果尚未登录官方账号，请先到“供应商”页面完成登录。
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>取消</AlertDialogCancel>
            <AlertDialogAction
              variant="destructive"
              disabled={switchingOfficial}
              onClick={() => void activateOfficial()}
            >
              {switchingOfficial && <Spinner data-icon="inline-start" />}
              {switchingOfficial ? "正在切换..." : "确认切换"}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  )
}
