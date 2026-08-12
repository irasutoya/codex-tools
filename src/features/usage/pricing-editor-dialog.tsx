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
import { Spinner } from "@/components/ui/spinner"
import { Switch } from "@/components/ui/switch"
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group"
import { toast } from "@/components/ui/toast"
import { errorMessage } from "@/lib/format"
import { call } from "@/lib/ipc"
import type { BillingMode, PricingRule } from "@/types"

export function PricingEditor({
  open,
  onOpenChange,
  onSaved,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  onSaved: () => void
}) {
  const [pattern, setPattern] = useState("")
  const [billingMode, setBillingMode] =
    useState<Extract<BillingMode, "token" | "unpriced">>("token")
  const [input, setInput] = useState("2.50")
  const [cachedRead, setCachedRead] = useState("0.25")
  const [cacheWrite, setCacheWrite] = useState("3.00")
  const [output, setOutput] = useState("10.00")
  const [cacheWriteIncluded, setCacheWriteIncluded] = useState(true)
  const [active, setActive] = useState(true)
  const [busy, setBusy] = useState(false)

  const save = async () => {
    setBusy(true)
    const now = Date.now()
    const rule: PricingRule = {
      id: `rule-${now}`,
      version: 1,
      active,
      scopeKind: "global_model",
      modelPattern: pattern,
      matchKind: "exact",
      billingMode,
      inputUsdPerMillion: billingMode === "token" ? input : undefined,
      cachedReadUsdPerMillion: billingMode === "token" ? cachedRead : undefined,
      cacheWriteUsdPerMillion: billingMode === "token" ? cacheWrite : undefined,
      outputUsdPerMillion: billingMode === "token" ? output : undefined,
      cacheWriteIncludedInInput: cacheWriteIncluded,
      effectiveFromMs: now,
      createdAtMs: now,
      updatedAtMs: now,
    }
    try {
      await call("usage_save_pricing_rule", { input: rule })
      toast.add({ title: "价格规则已保存", type: "success" })
      onSaved()
    } catch (reason) {
      toast.add({
        title: "保存失败",
        description: errorMessage(reason),
        type: "error",
      })
    } finally {
      setBusy(false)
    }
  }

  return (
    <Dialog
      open={open}
      onOpenChange={(nextOpen) => {
        if (!nextOpen && busy) return
        onOpenChange(nextOpen)
      }}
    >
      <DialogContent showCloseButton={!busy} aria-busy={busy}>
        <DialogHeader>
          <DialogTitle>添加价格规则</DialogTitle>
          <DialogDescription>
            选择不计价，或按每百万 Token 的美元金额计价。
          </DialogDescription>
        </DialogHeader>
        <DialogBody>
          <FieldGroup>
            <Field>
              <FieldLabel htmlFor="price-model">模型</FieldLabel>
              <Input
                id="price-model"
                value={pattern}
                placeholder="gpt-5.6"
                onChange={(e) => setPattern(e.target.value)}
              />
            </Field>
            <Field>
              <FieldLabel>计费方式</FieldLabel>
              <ToggleGroup
                variant="outline"
                spacing={0}
                className="w-full"
                value={[billingMode]}
                onValueChange={(value) => {
                  if (value[0] === "token" || value[0] === "unpriced") {
                    setBillingMode(value[0])
                  }
                }}
              >
                <ToggleGroupItem className="flex-1" value="token">
                  按 Token 计价
                </ToggleGroupItem>
                <ToggleGroupItem className="flex-1" value="unpriced">
                  不计价
                </ToggleGroupItem>
              </ToggleGroup>
              <FieldDescription>
                不计价规则会保留 Token 用量，但不估算费用。
              </FieldDescription>
            </Field>
            {billingMode === "token" && (
              <FieldGroup className="grid grid-cols-2 gap-3">
                <Field>
                  <FieldLabel htmlFor="price-input">普通输入</FieldLabel>
                  <Input
                    id="price-input"
                    inputMode="decimal"
                    value={input}
                    onChange={(e) => setInput(e.target.value)}
                  />
                </Field>
                <Field>
                  <FieldLabel htmlFor="price-cached-read">缓存读取</FieldLabel>
                  <Input
                    id="price-cached-read"
                    inputMode="decimal"
                    value={cachedRead}
                    onChange={(e) => setCachedRead(e.target.value)}
                  />
                </Field>
                <Field>
                  <FieldLabel htmlFor="price-cache-write">缓存写入</FieldLabel>
                  <Input
                    id="price-cache-write"
                    inputMode="decimal"
                    value={cacheWrite}
                    onChange={(e) => setCacheWrite(e.target.value)}
                  />
                </Field>
                <Field>
                  <FieldLabel htmlFor="price-output">输出</FieldLabel>
                  <Input
                    id="price-output"
                    inputMode="decimal"
                    value={output}
                    onChange={(e) => setOutput(e.target.value)}
                  />
                </Field>
              </FieldGroup>
            )}
            {billingMode === "token" && (
              <Field orientation="horizontal">
                <FieldContent>
                  <FieldLabel htmlFor="price-cache-write-included">
                    输入总量包含缓存写入
                  </FieldLabel>
                  <FieldDescription>
                    开启后，普通输入计费会扣除缓存读取和缓存写入 Token。
                  </FieldDescription>
                </FieldContent>
                <Switch
                  id="price-cache-write-included"
                  checked={cacheWriteIncluded}
                  onCheckedChange={setCacheWriteIncluded}
                />
              </Field>
            )}
            <Field orientation="horizontal">
              <FieldLabel htmlFor="price-active">立即启用</FieldLabel>
              <Switch
                id="price-active"
                checked={active}
                onCheckedChange={setActive}
              />
            </Field>
          </FieldGroup>
        </DialogBody>
        <DialogFooter>
          <Button
            variant="outline"
            disabled={busy}
            onClick={() => onOpenChange(false)}
          >
            取消
          </Button>
          <Button disabled={busy || !pattern} onClick={() => void save()}>
            {busy && <Spinner data-icon="inline-start" />}保存
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
