import { useState } from "react"
import { Refresh01Icon, SaveIcon } from "@hugeicons/core-free-icons"
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
import {
  InputGroup,
  InputGroupAddon,
  InputGroupButton,
  InputGroupInput,
} from "@/components/ui/input-group"
import { Input } from "@/components/ui/input"
import { Spinner } from "@/components/ui/spinner"
import { Switch } from "@/components/ui/switch"
import { Textarea } from "@/components/ui/textarea"
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group"
import { notify } from "@/lib/feedback"
import { call } from "@/lib/ipc"
import type { Provider } from "@/types"

import {
  MAX_API_KEY_LENGTH,
  MAX_API_URL_LENGTH,
  MAX_DISPLAY_NAME_LENGTH,
} from "./constants"

const MAX_VISIBLE_MODELS = 24

function buildWindowsText(windows: Record<string, number> | undefined) {
  return Object.entries(windows ?? {})
    .map(([model, window]) => `${model} = ${window}`)
    .join("\n")
}

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
  const [models, setModels] = useState<string[]>()
  const [loadingModels, setLoadingModels] = useState(false)

  const canLoadModels = Boolean(
    value.id &&
    value.baseUrl.trim() &&
    (value.apiKey?.trim() || value.hasApiKey) &&
    !busy
  )

  // 模型上下文窗口编辑：每行 `模型名 = 窗口`，可留空。
  // 输入框保持用户原文（本地 draft），只把“能完整解析的行”写回
  // modelContextWindows；避免把“正在输入中”的中间态（如还没有数字的
  // `model =`）从文本框里吞掉，导致一边输入一边消失。
  const [windowsDraft, setWindowsDraft] = useState(() =>
    buildWindowsText(value.modelContextWindows)
  )

  const updateWindows = (text: string) => {
    setWindowsDraft(text)
    const windows: Record<string, number> = {}
    for (const rawLine of text.split(/[\r\n]+/)) {
      const line = rawLine.trim()
      if (!line) continue
      const match = line.match(/^(.+?)\s*[=:]\s*(\d+)$/)
      const model = (match?.[1] ?? line).trim()
      const window = match?.[2]
      if (model && window) {
        windows[model] = Number(window)
      }
    }
    onChange({ ...value, modelContextWindows: windows })
  }

  const loadModels = async () => {
    if (!canLoadModels || loadingModels) return
    setLoadingModels(true)
    try {
      setModels(await call("list_provider_models", { id: value.id }))
    } catch (reason) {
      notify.error("无法获取模型列表", reason)
    } finally {
      setLoadingModels(false)
    }
  }

  const visibleModels = models?.slice(0, MAX_VISIBLE_MODELS) ?? []
  const hiddenModelCount = Math.max(
    0,
    (models?.length ?? 0) - MAX_VISIBLE_MODELS
  )

  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open && !busy) onCancel()
      }}
    >
      <DialogContent className="max-h-[calc(100dvh-2rem)] overflow-y-auto sm:max-w-md">
        <DialogHeader>
          <DialogTitle>
            {value.id ? "编辑 API 服务" : "添加 API 服务"}
          </DialogTitle>
          <DialogDescription>
            一个服务对应一个 API Key。填写地址、API Key 和模型后即可让 Codex
            使用该服务。
          </DialogDescription>
        </DialogHeader>
        <FieldGroup className="gap-4">
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
          <Field data-disabled={busy}>
            <FieldLabel htmlFor="provider-api-type">接入方式</FieldLabel>
            <ToggleGroup
              id="provider-api-type"
              variant="outline"
              spacing={0}
              className="w-full"
              value={[value.apiType]}
              onValueChange={(next, eventDetails) => {
                const apiType = next[0] as "responses" | "chat" | undefined
                if (apiType) onChange({ ...value, apiType })
                else eventDetails.isCanceled = true
              }}
            >
              <ToggleGroupItem value="responses" className="flex-1">
                Responses 直连
              </ToggleGroupItem>
              <ToggleGroupItem value="chat" className="flex-1">
                Chat 转换
              </ToggleGroupItem>
            </ToggleGroup>
            <FieldDescription>
              {value.apiType === "chat"
                ? "服务只提供 Chat Completions API 时选择此项：本机启动转换代理把 Responses 请求转为 Chat 请求。"
                : "服务支持 OpenAI Responses API 时选择此项：Codex 直接请求该服务。"}
            </FieldDescription>
          </Field>
          <Field data-disabled={busy}>
            <FieldLabel htmlFor="provider-api-key">API Key</FieldLabel>
            <Input
              id="provider-api-key"
              disabled={busy}
              type="password"
              autoComplete="off"
              maxLength={MAX_API_KEY_LENGTH}
              placeholder={value.hasApiKey ? "留空保留已保存的 Key" : "sk-…"}
              value={value.apiKey ?? ""}
              onChange={(event) =>
                onChange({ ...value, apiKey: event.target.value })
              }
            />
            <FieldDescription>
              只保存在本机；切换到此服务时写入 Codex 的 auth.json。
              {value.hasApiKey
                ? "已保存 Key，留空保持不变。"
                : "建议填写，便于保存后测试连接或切换使用。"}
            </FieldDescription>
          </Field>
          <Field data-disabled={busy}>
            <FieldLabel htmlFor="provider-model">默认模型（可选）</FieldLabel>
            <InputGroup>
              <InputGroupInput
                id="provider-model"
                autoComplete="off"
                spellCheck={false}
                disabled={busy}
                placeholder="例如 gpt-5.6-luna；留空沿用 Codex 默认"
                value={value.model ?? ""}
                onChange={(event) =>
                  onChange({ ...value, model: event.target.value.trim() })
                }
              />
              <InputGroupAddon align="inline-end">
                <InputGroupButton
                  aria-label="从服务加载模型列表"
                  title="从服务加载模型列表"
                  disabled={!canLoadModels || loadingModels}
                  onClick={() => void loadModels()}
                >
                  {loadingModels ? (
                    <Spinner data-icon="inline-start" />
                  ) : (
                    <HugeiconsIcon
                      icon={Refresh01Icon}
                      data-icon="inline-start"
                    />
                  )}
                  {loadingModels ? "加载中…" : "加载模型"}
                </InputGroupButton>
              </InputGroupAddon>
            </InputGroup>
            {models && (
              <div className="flex max-h-28 flex-col gap-1.5 overflow-y-auto rounded-lg border p-2">
                <ToggleGroup
                  variant="outline"
                  spacing={2}
                  size="sm"
                  className="flex-wrap"
                  aria-label="可用模型"
                  value={value.model ? [value.model] : []}
                  onValueChange={(next, eventDetails) => {
                    const model = next[0]
                    if (model) onChange({ ...value, model })
                    else eventDetails.isCanceled = true
                  }}
                >
                  {visibleModels.map((model) => (
                    <ToggleGroupItem key={model} value={model}>
                      {model}
                    </ToggleGroupItem>
                  ))}
                </ToggleGroup>
                {hiddenModelCount > 0 && (
                  <span className="px-1 py-1 text-xs text-muted-foreground">
                    另有 {hiddenModelCount} 个模型，可滚动查看或手动输入
                  </span>
                )}
              </div>
            )}
            <FieldDescription>
              切换到此服务时，Codex 会直接使用该模型调用。
            </FieldDescription>
          </Field>
          <Field data-disabled={busy}>
            <FieldLabel htmlFor="provider-context-windows">
              模型上下文窗口（可选）
            </FieldLabel>
            <Textarea
              id="provider-context-windows"
              className="field-sizing-content min-h-14 font-mono text-xs"
              disabled={busy}
              placeholder={"deepseek-v4-pro = 1000000\ndeepseek-chat = 128000"}
              value={windowsDraft}
              onChange={(event) => updateWindows(event.target.value)}
            />
            <FieldDescription>
              每行一个：模型名 + 空格 + = + 窗口 token 数。留空则回退 Codex
              默认。
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
