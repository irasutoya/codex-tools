import { useMemo, useState } from "react"
import {
  Copy01Icon,
  Download01Icon,
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
import { Spinner } from "@/components/ui/spinner"
import { Switch } from "@/components/ui/switch"
import { notify } from "@/lib/feedback"
import type { UsageOverview, UsageShareData } from "@/types"

import { UsageShareCard } from "./usage-share-card"
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
  const svg = useMemo(
    () =>
      data
        ? renderUsageShareSvg(
            data,
            mode,
            maskAccounts,
            showAllAccounts,
            showAllModels
          )
        : "",
    [data, maskAccounts, mode, showAllAccounts, showAllModels]
  )

  const withExport = async (action: (markup: string) => Promise<void>) => {
    if (!svg || !data || exporting) return
    setExporting(true)
    try {
      await action(svg)
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
      <DialogContent className="max-w-4xl sm:max-w-4xl">
        <DialogHeader className="pr-8">
          <DialogTitle className="flex items-center gap-2">
            <HugeiconsIcon icon={Share01Icon} className="size-4" />
            分享今日用量
          </DialogTitle>
          <DialogDescription>
            模型只显示在所属账号下；官方账号和中转站的同名模型不会合并。
          </DialogDescription>
        </DialogHeader>

        {error && (
          <Alert variant="destructive">
            <AlertTitle>今日用量读取失败</AlertTitle>
            <AlertDescription>{error}</AlertDescription>
          </Alert>
        )}

        {loading ? (
          <div className="flex min-h-48 items-center justify-center rounded-xl border">
            <Spinner />
            <span className="ml-2 text-sm text-muted-foreground">
              正在刷新今日用量和模型明细…
            </span>
          </div>
        ) : data ? (
          <div className="grid gap-4 lg:grid-cols-[minmax(0,1fr)_260px]">
            <UsageShareCard svg={svg} />
            <div className="flex flex-col gap-4 rounded-xl border bg-muted/20 p-4">
              <div>
                <p className="text-sm font-medium">分享内容</p>
                <p className="mt-1 text-xs leading-relaxed text-muted-foreground">
                  顶部总览汇总全部来源，明细始终按账号保留模型归属。
                </p>
              </div>
              <div
                className="flex flex-wrap gap-2"
                role="group"
                aria-label="分享卡片内容"
              >
                <Button
                  size="sm"
                  variant={mode === "details" ? "secondary" : "outline"}
                  aria-pressed={mode === "details"}
                  onClick={() => setMode("details")}
                >
                  总览 + 账号与模型
                </Button>
                <Button
                  size="sm"
                  variant={mode === "summary" ? "secondary" : "outline"}
                  aria-pressed={mode === "summary"}
                  onClick={() => setMode("summary")}
                >
                  仅总览
                </Button>
              </div>
              <label className="flex items-center justify-between gap-3 text-sm">
                <span>
                  <span className="block font-medium">账号名称脱敏</span>
                  <span className="mt-1 block text-xs text-muted-foreground">
                    不显示邮箱完整本地名
                  </span>
                </span>
                <Switch
                  checked={maskAccounts}
                  onCheckedChange={setMaskAccounts}
                  aria-label="账号名称脱敏"
                />
              </label>
              {mode === "details" && data.accounts.length > 6 && (
                <label className="flex items-center justify-between gap-3 text-sm">
                  <span>
                    <span className="block font-medium">展开全部账号</span>
                    <span className="mt-1 block text-xs text-muted-foreground">
                      当前 {data.accounts.length} 个账号
                    </span>
                  </span>
                  <Switch
                    checked={showAllAccounts}
                    onCheckedChange={setShowAllAccounts}
                    aria-label="展开全部账号"
                  />
                </label>
              )}
              {mode === "details" && hasMoreModels && (
                <label className="flex items-center justify-between gap-3 text-sm">
                  <span>
                    <span className="block font-medium">展开全部模型</span>
                    <span className="mt-1 block text-xs text-muted-foreground">
                      每个账号默认显示前 4 个模型
                    </span>
                  </span>
                  <Switch
                    checked={showAllModels}
                    onCheckedChange={setShowAllModels}
                    aria-label="展开全部模型"
                  />
                </label>
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
          <div className="flex min-h-48 items-center justify-center rounded-xl border text-sm text-muted-foreground">
            今天还没有可分享的 Token 记录。
          </div>
        )}

        <DialogFooter className="flex-col-reverse sm:flex-row sm:items-center">
          <Button variant="outline" onClick={() => onOpenChange(false)}>
            关闭
          </Button>
          <div className="flex flex-wrap justify-end gap-2">
            <Button
              variant="outline"
              disabled={!svg || exporting}
              onClick={() =>
                void withExport(async () => {
                  if (!data) return
                  await copySharePngToClipboard(
                    data,
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
              disabled={!svg || exporting}
              onClick={() =>
                void withExport(async (markup) => {
                  downloadShareFile(
                    "codex-tools-今日用量.svg",
                    markup,
                    "image/svg+xml;charset=utf-8"
                  )
                  notify.success("SVG 已保存")
                })
              }
            >
              <HugeiconsIcon icon={Download01Icon} data-icon="inline-start" />
              保存 SVG
            </Button>
            <Button
              disabled={!svg || exporting}
              onClick={() =>
                void withExport(async () => {
                  if (!data) return
                  downloadShareFile(
                    "codex-tools-今日用量.png",
                    await renderSharePng(
                      data,
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
