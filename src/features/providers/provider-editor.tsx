import { SaveIcon } from "@hugeicons/core-free-icons"
import { HugeiconsIcon } from "@hugeicons/react"

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
  Field,
  FieldContent,
  FieldDescription,
  FieldGroup,
  FieldLabel,
  FieldTitle,
} from "@/components/ui/field"
import { Input } from "@/components/ui/input"
import { Spinner } from "@/components/ui/spinner"
import { Switch } from "@/components/ui/switch"
import type { Provider } from "@/types"

import { MAX_API_URL_LENGTH, MAX_DISPLAY_NAME_LENGTH } from "./constants"

export function ProviderEditor({
  value,
  pendingTask,
  onChange,
  onCancel,
  onSave,
}: {
  value: Provider
  pendingTask?: string
  onChange: (value: Provider) => void
  onCancel: () => void
  onSave: (value: Provider) => void
}) {
  const busy = Boolean(pendingTask)

  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open && !busy) onCancel()
      }}
    >
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>
            {value.id ? "编辑 API 服务" : "添加 API 服务"}
          </DialogTitle>
          <DialogDescription>
            填写服务名称和 Responses API 地址。保存后再添加 API Key。
          </DialogDescription>
        </DialogHeader>
        <FieldGroup>
          <Field data-disabled={busy}>
            <FieldLabel htmlFor="provider-name">服务名称</FieldLabel>
            <Input
              id="provider-name"
              autoFocus
              disabled={busy}
              required
              maxLength={MAX_DISPLAY_NAME_LENGTH}
              placeholder="例如：公司 API"
              value={value.name}
              onChange={(event) =>
                onChange({ ...value, name: event.target.value })
              }
            />
            <FieldDescription>最多 100 个字符。</FieldDescription>
          </Field>
          <Field data-disabled={busy}>
            <FieldLabel htmlFor="provider-base-url">API 地址</FieldLabel>
            <Input
              id="provider-base-url"
              type="url"
              disabled={busy}
              required
              maxLength={MAX_API_URL_LENGTH}
              placeholder="https://api.example.com/v1"
              value={value.baseUrl}
              onChange={(event) =>
                onChange({ ...value, baseUrl: event.target.value })
              }
            />
            <FieldDescription>
              最多 2,048 个字符。填写服务商提供的 API 根地址，通常以 /v1 结尾。
            </FieldDescription>
          </Field>
          <Field orientation="horizontal" data-disabled={busy}>
            <FieldContent>
              <FieldTitle>启用此服务</FieldTitle>
              <FieldDescription>
                关闭后仍会保留配置，但不能切换使用。
              </FieldDescription>
            </FieldContent>
            <Switch
              id="provider-enabled"
              aria-label="启用此服务"
              disabled={busy}
              checked={value.enabled}
              onCheckedChange={(enabled) => onChange({ ...value, enabled })}
            />
          </Field>
        </FieldGroup>
        <DialogFooter>
          <Button variant="outline" disabled={busy} onClick={onCancel}>
            取消
          </Button>
          <Button
            disabled={busy || !value.name.trim() || !value.baseUrl.trim()}
            onClick={() => onSave(value)}
          >
            {pendingTask === "provider:save" ? (
              <Spinner data-icon="inline-start" />
            ) : (
              <HugeiconsIcon icon={SaveIcon} data-icon="inline-start" />
            )}
            {pendingTask === "provider:save" ? "正在保存…" : "保存服务"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
