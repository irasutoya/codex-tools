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
  FieldDescription,
  FieldGroup,
  FieldLabel,
} from "@/components/ui/field"
import { Input } from "@/components/ui/input"
import { Spinner } from "@/components/ui/spinner"
import type { Account } from "@/types"

import { MAX_API_KEY_LENGTH, MAX_DISPLAY_NAME_LENGTH } from "./constants"

export function AccountEditor({
  value,
  pending,
  onChange,
  onCancel,
  onSave,
}: {
  value: Account
  pending: boolean
  onChange: (value: Account) => void
  onCancel: () => void
  onSave: () => void
}) {
  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open && !pending) onCancel()
      }}
    >
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>添加 API Key</DialogTitle>
          <DialogDescription>
            可以为同一个第三方服务保存多个 API Key，并随时切换。
          </DialogDescription>
        </DialogHeader>
        <FieldGroup>
          <Field data-disabled={pending}>
            <FieldLabel htmlFor="account-name">密钥名称</FieldLabel>
            <Input
              id="account-name"
              autoFocus
              disabled={pending}
              required
              maxLength={MAX_DISPLAY_NAME_LENGTH}
              placeholder="例如：个人密钥"
              value={value.name}
              onChange={(event) =>
                onChange({ ...value, name: event.target.value })
              }
            />
            <FieldDescription>最多 100 个字符。</FieldDescription>
          </Field>
          <Field data-disabled={pending}>
            <FieldLabel htmlFor="account-api-key">API Key</FieldLabel>
            <Input
              id="account-api-key"
              required
              disabled={pending}
              type="password"
              autoComplete="off"
              maxLength={MAX_API_KEY_LENGTH}
              placeholder="sk-…"
              value={value.apiKey ?? ""}
              onChange={(event) =>
                onChange({
                  ...value,
                  apiKey: event.target.value,
                  authKind: "api_key",
                })
              }
            />
            <FieldDescription>
              最多 65,536 个字符。密钥保存在本机；切换到此服务时写入 Codex 的
              auth.json。
            </FieldDescription>
          </Field>
        </FieldGroup>
        <DialogFooter>
          <Button variant="outline" disabled={pending} onClick={onCancel}>
            取消
          </Button>
          <Button
            disabled={pending || !value.name.trim() || !value.apiKey?.trim()}
            onClick={onSave}
          >
            {pending ? (
              <Spinner data-icon="inline-start" />
            ) : (
              <HugeiconsIcon icon={SaveIcon} data-icon="inline-start" />
            )}
            {pending ? "正在保存…" : "保存 API Key"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
