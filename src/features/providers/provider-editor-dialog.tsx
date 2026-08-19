import { useState } from "react"

import { Button } from "@/components/ui/button"
import { Checkbox } from "@/components/ui/checkbox"
import {
  Dialog,
  DialogBody,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { Field, FieldGroup, FieldLabel } from "@/components/ui/field"
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
        selectedModels: provider.selectedModels,
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

  const availableModels = provider.availableModels ?? []
  const allModelsSelected =
    provider.selectedModels === undefined ||
    availableModels.every((model) => provider.selectedModels?.includes(model))
  const noModelsSelected =
    availableModels.length > 0 && provider.selectedModels?.length === 0

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
            {provider.availableModels &&
              provider.availableModels.length > 0 && (
                <Field data-disabled={saving}>
                  <div className="flex items-center justify-between gap-2">
                    <FieldLabel>写入 Codex 的模型</FieldLabel>
                    <Button
                      type="button"
                      size="sm"
                      variant="ghost"
                      disabled={saving || availableModels.length === 0}
                      onClick={() =>
                        update(
                          "selectedModels",
                          allModelsSelected ? [] : undefined
                        )
                      }
                    >
                      {allModelsSelected ? "取消全选" : "全选"}
                    </Button>
                  </div>
                  <div className="max-h-48 space-y-1 overflow-y-auto rounded-xl border border-border/60 p-2">
                    {availableModels.map((model) => {
                      const selected =
                        provider.selectedModels?.includes(model) ?? true
                      return (
                        <label
                          key={model}
                          className="flex items-center gap-2 rounded-lg px-2 py-1.5 text-sm hover:bg-muted/60"
                        >
                          <Checkbox
                            checked={selected}
                            disabled={saving}
                            onCheckedChange={(checked) => {
                              const current =
                                provider.selectedModels ?? availableModels
                              const next = checked
                                ? [...new Set([...current, model])]
                                : current.filter((value) => value !== model)
                              update(
                                "selectedModels",
                                next.length === availableModels.length
                                  ? undefined
                                  : next
                              )
                            }}
                          />
                          <span className="truncate">{model}</span>
                        </label>
                      )
                    })}
                  </div>
                  <div className="text-xs text-muted-foreground">
                    默认全部选中，取消选择后仅将选中模型写入 Codex。
                  </div>
                </Field>
              )}
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
            disabled={
              saving || !provider.name || !provider.baseUrl || noModelsSelected
            }
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
