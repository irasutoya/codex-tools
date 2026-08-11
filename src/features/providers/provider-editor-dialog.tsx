import { useState } from "react"

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
  FieldContent,
  FieldDescription,
  FieldGroup,
  FieldLabel,
} from "@/components/ui/field"
import { Input } from "@/components/ui/input"
import {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { Spinner } from "@/components/ui/spinner"
import { Switch } from "@/components/ui/switch"
import { toast } from "@/components/ui/toast"
import { errorMessage } from "@/lib/format"
import { call } from "@/lib/ipc"
import type { Provider, ProviderSaveInput } from "@/types"

export function ProviderEditorDialog({
  open,
  onOpenChange,
  provider,
  onProviderChange,
  onSaved,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  provider: Provider
  onProviderChange: (provider: Provider) => void
  onSaved: () => void
}) {
  const [saving, setSaving] = useState(false)
  const update = <K extends keyof Provider>(key: K, value: Provider[K]) =>
    onProviderChange({ ...provider, [key]: value })

  const save = async () => {
    setSaving(true)
    try {
      const input: ProviderSaveInput = {
        id: provider.id,
        name: provider.name,
        baseUrl: provider.baseUrl,
        headers: { ...provider.headers },
        timeoutSecs: provider.timeoutSecs,
        enabled: provider.enabled,
        apiType: provider.apiType,
        apiKey: provider.apiKey,
      }
      await call("connections_save_provider", { provider: input })
      toast.add({ title: "API 服务已保存", type: "success" })
      onSaved()
    } catch (reason) {
      toast.add({
        title: "保存失败",
        description: errorMessage(reason),
        type: "error",
      })
    } finally {
      setSaving(false)
    }
  }

  return (
    <Dialog
      open={open}
      onOpenChange={(nextOpen) => {
        if (!nextOpen && saving) return
        onOpenChange(nextOpen)
      }}
    >
      <DialogContent showCloseButton={!saving} aria-busy={saving}>
        <DialogHeader>
          <DialogTitle>
            {provider.id ? "编辑 API 服务" : "添加 API 服务"}
          </DialogTitle>
          <DialogDescription>
            填写与 OpenAI 兼容的接口信息，模型列表将从 API 自动同步。
          </DialogDescription>
        </DialogHeader>
        <DialogBody>
          <FieldGroup>
            <Field data-disabled={saving}>
              <FieldLabel htmlFor="provider-name">名称</FieldLabel>
              <Input
                id="provider-name"
                disabled={saving}
                value={provider.name}
                onChange={(event) => update("name", event.target.value)}
              />
            </Field>
            <Field data-disabled={saving}>
              <FieldLabel htmlFor="provider-url">Base URL</FieldLabel>
              <Input
                id="provider-url"
                disabled={saving}
                value={provider.baseUrl}
                onChange={(event) => update("baseUrl", event.target.value)}
              />
            </Field>
            <Field data-disabled={saving}>
              <FieldLabel htmlFor="provider-key">API Key</FieldLabel>
              <Input
                id="provider-key"
                disabled={saving}
                type="password"
                value={provider.apiKey ?? ""}
                placeholder={
                  provider.hasApiKey ? "留空以保留现有密钥" : "sk-..."
                }
                onChange={(event) => update("apiKey", event.target.value)}
              />
            </Field>
            <Field data-disabled={saving}>
              <FieldLabel>接口类型</FieldLabel>
              <Select
                items={[
                  { label: "Responses API", value: "responses" },
                  { label: "Chat Completions", value: "chat" },
                ]}
                value={provider.apiType}
                disabled={saving}
                onValueChange={(value) =>
                  value && update("apiType", value as Provider["apiType"])
                }
              >
                <SelectTrigger className="w-full">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectGroup>
                    <SelectItem value="responses">Responses API</SelectItem>
                    <SelectItem value="chat">Chat Completions</SelectItem>
                  </SelectGroup>
                </SelectContent>
              </Select>
            </Field>
            <Field orientation="horizontal" data-disabled={saving}>
              <FieldContent>
                <FieldLabel htmlFor="provider-enabled">启用服务</FieldLabel>
                <FieldDescription>
                  关闭后不会出现在可切换列表。
                </FieldDescription>
              </FieldContent>
              <Switch
                id="provider-enabled"
                disabled={saving}
                checked={provider.enabled}
                onCheckedChange={(checked) => update("enabled", checked)}
              />
            </Field>
          </FieldGroup>
        </DialogBody>
        <DialogFooter>
          <Button
            type="button"
            variant="outline"
            disabled={saving}
            onClick={() => onOpenChange(false)}
          >
            取消
          </Button>
          <Button
            type="button"
            disabled={saving || !provider.name || !provider.baseUrl}
            onClick={() => void save()}
          >
            {saving && <Spinner data-icon="inline-start" />}
            保存
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
