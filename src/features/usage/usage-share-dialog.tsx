import { useMemo, useState } from "react"
import {
  Copy01Icon,
  Image01Icon,
  Share01Icon,
} from "@hugeicons/core-free-icons"
import { HugeiconsIcon } from "@hugeicons/react"

import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { Button } from "@/components/ui/button"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import {
  Empty,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from "@/components/ui/empty"
import {
  Field,
  FieldContent,
  FieldDescription,
  FieldTitle,
} from "@/components/ui/field"
import { Spinner } from "@/components/ui/spinner"
import { Switch } from "@/components/ui/switch"
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group"
import { notify } from "@/lib/feedback"
import type { UsageOverview, UsageShareData } from "@/types"

import {
  buildUsageShareData,
  copySharePngToClipboard,
  downloadShareFile,
  renderSharePng,
  renderUsageShareSvg,
  type UsageShareMode,
} from "./usage-share"

type UsageShareDialogProps = {
  open: boolean
  onOpenChange: (open: boolean) => void
  accountOverview?: UsageOverview
  modelOverview?: UsageOverview
  dateLabel: string
  timezone: string
  loading: boolean
  error?: string
}

export function UsageShareDialog({
  open,
  onOpenChange,
  accountOverview,
  modelOverview,
  dateLabel,
  timezone,
  loading,
  error,
}: UsageShareDialogProps) {
  const [mode, setMode] = useState<UsageShareMode>("details")
  const [maskAccounts, setMaskAccounts] = useState(true)
  const [showAllAccounts, setShowAllAccounts] = useState(false)
  const [showAllModels, setShowAllModels] = useState(false)
  const [exporting, setExporting] = useState(false)
  const data = useMemo<UsageShareData | undefined>(
    () =>
      accountOverview && modelOverview
        ? buildUsageShareData(
            accountOverview,
            modelOverview,
            dateLabel,
            timezone
          )
        : undefined,
    [accountOverview, dateLabel, modelOverview, timezone]
  )

  const withExport = async (
    action: (shareData: UsageShareData) => Promise<void>
  ) => {
    if (!data || exporting) return
    setExporting(true)
    try {
      await action(data)
    } catch (reason) {
      notify.error("分享图片失败", reason)
    } finally {
      setExporting(false)
    }
  }

  const modelCount =
    data?.accounts.reduce(
      (total, account) => total + account.models.length,
      0
    ) ?? 0
  const hasMoreModels =
    data?.accounts.some((account) => account.models.length > 4) ?? false

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-h-[calc(100dvh-3rem)] max-w-4xl overflow-y-auto">
        <DialogHeader className="pr-8">
          <DialogTitle className="flex items-center gap-2">
            <HugeiconsIcon icon={Share01Icon} size={16} />
            分享今日用量
          </DialogTitle>
          <DialogDescription>
            模型只显示在所属账号下；官方账号和 API 服务的同名模型不会合并。
          </DialogDescription>
        </DialogHeader>

        {error && (
          <Alert variant="destructive">
            <AlertTitle>今日用量读取失败</AlertTitle>
            <AlertDescription>{error}</AlertDescription>
          </Alert>
        )}

        {loading ? (
          <Empty className="min-h-48 border">
            <EmptyHeader>
              <EmptyMedia variant="icon">
                <Spinner aria-hidden="true" />
              </EmptyMedia>
              <EmptyTitle>正在刷新今日用量</EmptyTitle>
              <EmptyDescription>
                正在读取账号与模型明细，请稍候…
              </EmptyDescription>
            </EmptyHeader>
          </Empty>
        ) : data ? (
          <div className="grid gap-4 lg:grid-cols-[minmax(0,1fr)_260px]">
            <div
              className="max-h-[min(66dvh,780px)] overflow-auto rounded-xl border bg-muted/40 p-3"
              aria-label="分享卡片预览"
            >
              <div
                className="mx-auto w-fit"
                // 预览直接渲染 SVG：与导出的 PNG 完全一致（所见即所得）。
                dangerouslySetInnerHTML={{
                  __html: renderUsageShareSvg(
                    data,
                    mode,
                    maskAccounts,
                    showAllAccounts,
                    showAllModels
                  ),
                }}
              />
            </div>
            <div className="flex flex-col gap-4 rounded-xl border bg-muted/20 p-4">
              <div>
                <p className="text-sm font-medium">分享内容</p>
                <p className="mt-1 text-xs leading-relaxed text-muted-foreground">
                  顶部总览汇总全部来源，明细始终按账号保留模型归属。
                </p>
              </div>
              <ToggleGroup
                variant="outline"
                spacing={2}
                size="sm"
                className="flex-wrap"
                value={[mode]}
                onValueChange={(value, eventDetails) => {
                  const next = value[0] as UsageShareMode | undefined
                  if (next) setMode(next)
                  else eventDetails.isCanceled = true
                }}
                aria-label="分享卡片内容"
              >
                <ToggleGroupItem value="details">
                  总览 + 账号与模型
                </ToggleGroupItem>
                <ToggleGroupItem value="summary">仅总览</ToggleGroupItem>
              </ToggleGroup>
              <Field orientation="horizontal">
                <FieldContent>
                  <FieldTitle>账号名称脱敏</FieldTitle>
                  <FieldDescription>不显示邮箱完整本地名</FieldDescription>
                </FieldContent>
                <Switch
                  checked={maskAccounts}
                  onCheckedChange={setMaskAccounts}
                  aria-label="账号名称脱敏"
                />
              </Field>
              {mode === "details" && data.accounts.length > 6 && (
                <Field orientation="horizontal">
                  <FieldContent>
                    <FieldTitle>展开全部账号</FieldTitle>
                    <FieldDescription>
                      当前 {data.accounts.length} 个账号
                    </FieldDescription>
                  </FieldContent>
                  <Switch
                    checked={showAllAccounts}
                    onCheckedChange={setShowAllAccounts}
                    aria-label="展开全部账号"
                  />
                </Field>
              )}
              {mode === "details" && hasMoreModels && (
                <Field orientation="horizontal">
                  <FieldContent>
                    <FieldTitle>展开全部模型</FieldTitle>
                    <FieldDescription>
                      每个账号默认显示前 4 个模型
                    </FieldDescription>
                  </FieldContent>
                  <Switch
                    checked={showAllModels}
                    onCheckedChange={setShowAllModels}
                    aria-label="展开全部模型"
                  />
                </Field>
              )}
              <div className="rounded-lg border bg-background/70 p-3 text-xs leading-relaxed text-muted-foreground">
                <p>总 Token：{data.totalTokens.toLocaleString("en-US")}</p>
                <p>账号数：{data.accounts.length}</p>
                <p>账号内模型记录：{modelCount}</p>
                <p>统计时区：{data.timezone}</p>
              </div>
            </div>
          </div>
        ) : (
          <Empty className="min-h-48 border">
            <EmptyHeader>
              <EmptyMedia variant="icon">
                <HugeiconsIcon icon={Share01Icon} />
              </EmptyMedia>
              <EmptyTitle>今天还没有可分享的 Token 记录</EmptyTitle>
              <EmptyDescription>
                先在 Codex 中发起一次请求，再来生成分享卡片。
              </EmptyDescription>
            </EmptyHeader>
          </Empty>
        )}

        <DialogFooter className="flex-col-reverse sm:flex-row sm:items-center">
          <Button variant="outline" onClick={() => onOpenChange(false)}>
            关闭
          </Button>
          <div className="flex flex-wrap justify-end gap-2">
            <Button
              variant="outline"
              disabled={!data || exporting}
              onClick={() =>
                void withExport(async (shareData) => {
                  await copySharePngToClipboard(
                    shareData,
                    mode,
                    maskAccounts,
                    showAllAccounts,
                    showAllModels
                  )
                  notify.success("已复制分享图片", "可以直接粘贴到聊天窗口。")
                })
              }
            >
              {exporting ? (
                <Spinner data-icon="inline-start" />
              ) : (
                <HugeiconsIcon icon={Copy01Icon} data-icon="inline-start" />
              )}
              复制图片
            </Button>
            <Button
              variant="outline"
              disabled={!data || exporting}
              onClick={() =>
                void withExport(async (shareData) => {
                  downloadShareFile(
                    "codex-tools-今日用量.png",
                    await renderSharePng(
                      shareData,
                      mode,
                      maskAccounts,
                      showAllAccounts,
                      showAllModels
                    ),
                    "image/png"
                  )
                  notify.success("PNG 已保存")
                })
              }
            >
              <HugeiconsIcon icon={Image01Icon} data-icon="inline-start" />
              保存 PNG
            </Button>
          </div>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
