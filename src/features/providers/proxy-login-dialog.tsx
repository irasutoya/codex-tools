import { useRef, useState } from "react"
import { Login01Icon } from "@hugeicons/core-free-icons"
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
import { Textarea } from "@/components/ui/textarea"

import {
  MAX_ACCOUNT_ID_LENGTH,
  MAX_COOKIE_CREDENTIAL_LENGTH,
  MAX_DISPLAY_NAME_LENGTH,
} from "./constants"

export function ProxyLoginDialog({
  pending,
  onCancel,
  onLogin,
}: {
  pending: boolean
  onCancel: () => void
  onLogin: (
    name: string | undefined,
    accountId: string | undefined,
    content: string
  ) => void
}) {
  const [name, setName] = useState("")
  const [accountId, setAccountId] = useState("")
  const [hasContent, setHasContent] = useState(false)
  const contentRef = useRef<HTMLTextAreaElement>(null)

  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open && !pending) onCancel()
      }}
    >
      <DialogContent className="max-h-[calc(100dvh-2rem)] w-full max-w-lg overflow-y-auto">
        <DialogHeader>
          <DialogTitle>导入 Cookie 账号</DialogTitle>
          <DialogDescription>
            粘贴 Cookie Token 或单账号 JSON。这里不会读取浏览器
            Cookie；导入后会尝试向 OpenAI 查询 5H/7D 额度。
          </DialogDescription>
        </DialogHeader>
        <FieldGroup>
          <Field data-disabled={pending}>
            <FieldLabel htmlFor="proxy-account-name">账号名称</FieldLabel>
            <Input
              id="proxy-account-name"
              autoFocus
              disabled={pending}
              maxLength={MAX_DISPLAY_NAME_LENGTH}
              placeholder="可选，例如：工作 Cookie 账号"
              value={name}
              onChange={(event) => setName(event.target.value)}
            />
          </Field>
          <Field data-disabled={pending}>
            <FieldLabel htmlFor="proxy-account-id">
              ChatGPT Account ID
            </FieldLabel>
            <Input
              id="proxy-account-id"
              autoComplete="off"
              disabled={pending}
              maxLength={MAX_ACCOUNT_ID_LENGTH}
              placeholder="可选；团队号查询额度时可能需要"
              value={accountId}
              onChange={(event) => setAccountId(event.target.value)}
            />
            <FieldDescription>
              最多 512 个字符。个人账号通常留空；单账号 JSON 已包含 accountId
              时也可留空。
            </FieldDescription>
          </Field>
          <Field data-disabled={pending}>
            <FieldLabel htmlFor="proxy-account-content">
              Cookie Token / 单账号 JSON
            </FieldLabel>
            <Textarea
              ref={contentRef}
              id="proxy-account-content"
              className="field-sizing-fixed h-40 max-h-40 min-h-40 max-w-full resize-none overflow-x-hidden overflow-y-auto font-mono text-xs break-all"
              autoComplete="off"
              disabled={pending}
              spellCheck={false}
              wrap="soft"
              maxLength={MAX_COOKIE_CREDENTIAL_LENGTH}
              placeholder='粘贴 at-…、accessToken，或包含 "access_token" / "refresh_token" 的单账号 JSON'
              onInput={(event) =>
                setHasContent(/\S/.test(event.currentTarget.value))
              }
            />
            <FieldDescription>
              最多 262,144 个字符。原始 JSON
              不会保存；程序只提取登录所需字段，并将凭据写入本机应用数据文件。
            </FieldDescription>
          </Field>
        </FieldGroup>
        <DialogFooter>
          <Button variant="outline" disabled={pending} onClick={onCancel}>
            取消
          </Button>
          <Button
            disabled={pending || !hasContent}
            onClick={() => {
              const content = contentRef.current?.value ?? ""
              if (!content.trim()) return
              onLogin(
                name.trim() || undefined,
                accountId.trim() || undefined,
                content
              )
            }}
          >
            {pending ? (
              <Spinner data-icon="inline-start" />
            ) : (
              <HugeiconsIcon icon={Login01Icon} data-icon="inline-start" />
            )}
            {pending ? "正在导入…" : "导入并登录"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
