import { useRef, useState } from "react"
import {
  Copy01Icon,
  ExternalLinkIcon,
  Key01Icon,
  Login03Icon,
} from "@hugeicons/core-free-icons"
import { HugeiconsIcon } from "@hugeicons/react"

import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import {
  Dialog,
  DialogBody,
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
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { Textarea } from "@/components/ui/textarea"
import type { DeviceAuthorization } from "@/types"

const MAX_DISPLAY_NAME_LENGTH = 128
const MAX_ACCOUNT_ID_LENGTH = 512
const MAX_COOKIE_CREDENTIAL_LENGTH = 262_144

export type AccountLoginMode = "browser" | "cookie"

export function AccountLoginDialog({
  open,
  mode,
  onModeChange,
  onOpenChange,
  authorization,
  starting,
  polling,
  importing,
  error,
  onStart,
  onOpenPage,
  onCheck,
  onImport,
}: {
  open: boolean
  mode: AccountLoginMode
  onModeChange: (mode: AccountLoginMode) => void
  onOpenChange: (open: boolean) => void
  authorization?: DeviceAuthorization
  starting: boolean
  polling: boolean
  importing: boolean
  error?: string
  onStart: () => void
  onOpenPage: () => void
  onCheck: () => void
  onImport: (
    name: string | undefined,
    accountId: string | undefined,
    content: string
  ) => void
}) {
  const [name, setName] = useState("")
  const [accountId, setAccountId] = useState("")
  const [hasContent, setHasContent] = useState(false)
  const contentRef = useRef<HTMLTextAreaElement>(null)
  const busy = starting || polling || importing

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent showCloseButton={!busy} aria-busy={busy}>
        <DialogHeader>
          <DialogTitle>添加 OpenAI 账号</DialogTitle>
          <DialogDescription>
            选择浏览器授权，或粘贴反代账号登录材料。
          </DialogDescription>
        </DialogHeader>

        <DialogBody>
          {error && (
            <Alert variant="destructive">
              <AlertTitle>登录暂时无法继续</AlertTitle>
              <AlertDescription>{error}</AlertDescription>
            </Alert>
          )}

          <Tabs
            value={mode}
            onValueChange={(value) => onModeChange(value as AccountLoginMode)}
          >
            <TabsList className="grid w-full grid-cols-2">
              <TabsTrigger value="browser" disabled={busy}>
                <HugeiconsIcon icon={Login03Icon} />
                浏览器登录
              </TabsTrigger>
              <TabsTrigger value="cookie" disabled={busy}>
                <HugeiconsIcon icon={Key01Icon} />
                反代账号登录
              </TabsTrigger>
            </TabsList>

            <TabsContent value="browser" className="flex flex-col gap-3 pt-1">
              {authorization ? (
                <Alert>
                  <HugeiconsIcon icon={Login03Icon} />
                  <AlertTitle className="flex flex-wrap items-center gap-2">
                    登录码：
                    <code className="font-mono font-semibold">
                      {authorization.userCode}
                    </code>
                    <Badge variant="outline">
                      <Spinner data-icon="inline-start" />
                      等待登录
                    </Badge>
                  </AlertTitle>
                  <AlertDescription>
                    在 OpenAI
                    登录页面输入此代码；完成后程序会自动确认并刷新账号。
                  </AlertDescription>
                </Alert>
              ) : (
                <div className="flex min-h-24 flex-col items-center justify-center gap-2 rounded-2xl bg-muted px-4 text-center">
                  {starting ? (
                    <>
                      <Spinner />
                      <div className="text-sm font-medium">正在生成登录码…</div>
                    </>
                  ) : (
                    <>
                      <div className="text-sm font-medium">
                        使用 OpenAI 设备授权登录
                      </div>
                      <div className="text-xs text-muted-foreground">
                        点击后会生成一次性登录码，不会读取浏览器 Cookie。
                      </div>
                      <Button type="button" disabled={busy} onClick={onStart}>
                        <HugeiconsIcon
                          icon={Login03Icon}
                          data-icon="inline-start"
                        />
                        生成登录码
                      </Button>
                    </>
                  )}
                </div>
              )}

              {authorization && (
                <div className="flex flex-wrap gap-2">
                  <Button
                    type="button"
                    size="sm"
                    variant="outline"
                    onClick={() =>
                      void navigator.clipboard.writeText(authorization.userCode)
                    }
                  >
                    <HugeiconsIcon icon={Copy01Icon} data-icon="inline-start" />
                    复制登录码
                  </Button>
                  <Button
                    type="button"
                    size="sm"
                    variant="outline"
                    onClick={onOpenPage}
                  >
                    <HugeiconsIcon
                      icon={ExternalLinkIcon}
                      data-icon="inline-start"
                    />
                    打开登录页面
                  </Button>
                  <Button
                    type="button"
                    size="sm"
                    disabled={busy}
                    onClick={onCheck}
                  >
                    {polling && <Spinner data-icon="inline-start" />}
                    立即检查
                  </Button>
                </div>
              )}
            </TabsContent>

            <TabsContent value="cookie" className="pt-1">
              <FieldGroup>
                <Field data-disabled={busy}>
                  <FieldLabel htmlFor="cookie-account-name">
                    账号名称
                  </FieldLabel>
                  <Input
                    id="cookie-account-name"
                    disabled={busy}
                    maxLength={MAX_DISPLAY_NAME_LENGTH}
                    placeholder="可选，例如：工作 Cookie 账号"
                    value={name}
                    onChange={(event) => setName(event.target.value)}
                  />
                </Field>
                <Field data-disabled={busy}>
                  <FieldLabel htmlFor="cookie-account-id">
                    ChatGPT Account ID
                  </FieldLabel>
                  <Input
                    id="cookie-account-id"
                    autoComplete="off"
                    disabled={busy}
                    maxLength={MAX_ACCOUNT_ID_LENGTH}
                    placeholder="可选；团队账号查询额度时可能需要"
                    value={accountId}
                    onChange={(event) => setAccountId(event.target.value)}
                  />
                  <FieldDescription>
                    个人账号通常留空；JSON 已包含 accountId 时也可留空。
                  </FieldDescription>
                </Field>
                <Field data-disabled={busy}>
                  <FieldLabel htmlFor="cookie-account-content">
                    RT / Token / 反代账号 JSON
                  </FieldLabel>
                  <Textarea
                    ref={contentRef}
                    id="cookie-account-content"
                    className="field-sizing-fixed h-28 max-h-28 min-h-28 max-w-full resize-none overflow-y-auto font-mono text-xs break-all"
                    autoComplete="off"
                    disabled={busy}
                    spellCheck={false}
                    wrap="soft"
                    maxLength={MAX_COOKIE_CREDENTIAL_LENGTH}
                    placeholder={
                      "可粘贴纯 RT、at-…、JWT，或 CPA / Sub2API / Cockpit / 9router JSON"
                    }
                    onInput={(event) =>
                      setHasContent(/\S/.test(event.currentTarget.value))
                    }
                  />
                  <FieldDescription>
                    程序会自动识别格式并拆分多账号；原始内容只提取登录字段写入本机应用数据。
                  </FieldDescription>
                </Field>
              </FieldGroup>
            </TabsContent>
          </Tabs>
        </DialogBody>

        <DialogFooter>
          <Button
            type="button"
            variant="outline"
            disabled={busy}
            onClick={() => onOpenChange(false)}
          >
            取消
          </Button>
          {mode === "cookie" && (
            <Button
              type="button"
              disabled={busy || !hasContent}
              onClick={() => {
                const content = contentRef.current?.value ?? ""
                if (!content.trim()) return
                onImport(
                  name.trim() || undefined,
                  accountId.trim() || undefined,
                  content
                )
              }}
            >
              {importing ? (
                <Spinner data-icon="inline-start" />
              ) : (
                <HugeiconsIcon icon={Key01Icon} data-icon="inline-start" />
              )}
              {importing ? "正在识别并登录…" : "识别并登录"}
            </Button>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
