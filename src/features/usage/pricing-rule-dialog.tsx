import { Save } from "lucide-react"

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
  FieldError,
  FieldGroup,
  FieldLabel,
} from "@/components/ui/field"
import { Input } from "@/components/ui/input"
import { Spinner } from "@/components/ui/spinner"
import { Switch } from "@/components/ui/switch"
import type { PricingRule, ProviderOverview } from "@/types"

type PricingRuleDialogProps = {
  open: boolean
  draft?: PricingRule
  providers?: ProviderOverview
  pending?: boolean
  error?: string
  fieldErrors?: {
    model?: string
    provider?: string
    prices?: string
  }
  repriceAfterSave?: boolean
  onOpenChange: (open: boolean) => void
  onChange: (rule: PricingRule) => void
  onRepriceChange: (checked: boolean) => void
  onSave: () => void
}

function updateRule(
  rule: PricingRule,
  patch: Partial<PricingRule>,
  onChange: (rule: PricingRule) => void
) {
  onChange({ ...rule, ...patch })
}

export function PricingRuleDialog({
  open,
  draft,
  providers,
  pending = false,
  onOpenChange,
  onChange,
  onSave,
  error,
  fieldErrors,
  repriceAfterSave = true,
  onRepriceChange,
}: PricingRuleDialogProps) {
  if (!draft) return null

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="w-full max-w-md">
        <DialogHeader>
          <DialogTitle>
            {draft.id ? "编辑中转站价格" : "设置中转站价格"}
          </DialogTitle>
          <DialogDescription>
            只用于本机 Token 估算，不会修改中转站账单。规则自动按“中转站 +
            模型”精确匹配，金额均为 USD / 1M Token。
          </DialogDescription>
        </DialogHeader>

        <FieldGroup>
          {error && (
            <p className="text-sm text-destructive" role="alert">
              {error}
            </p>
          )}

          <Field>
            <FieldLabel htmlFor="pricing-provider">中转站服务</FieldLabel>
            <select
              id="pricing-provider"
              className="h-9 w-full rounded-lg border border-input bg-transparent px-2.5 text-sm outline-none focus-visible:border-ring focus-visible:ring-3 focus-visible:ring-ring/50"
              disabled={pending}
              value={draft.providerId ?? ""}
              onChange={(event) =>
                updateRule(
                  draft,
                  { providerId: event.target.value || undefined },
                  onChange
                )
              }
            >
              <option value="">请选择中转站</option>
              {providers?.providers
                .filter((provider) => provider.active || provider.enabled)
                .map((provider) => (
                  <option key={provider.id} value={provider.id}>
                    {provider.name}
                  </option>
                ))}
            </select>
            <FieldDescription>
              价格会自动应用到这个中转站的当前模型。
            </FieldDescription>
            <FieldError>{fieldErrors?.provider}</FieldError>
          </Field>

          <Field>
            <FieldLabel htmlFor="pricing-model">模型</FieldLabel>
            <Input
              id="pricing-model"
              disabled={pending}
              value={draft.modelPattern}
              placeholder="例如：gpt-5.6-luna"
              onChange={(event) =>
                updateRule(
                  draft,
                  { modelPattern: event.target.value },
                  onChange
                )
              }
            />
            <FieldError>{fieldErrors?.model}</FieldError>
          </Field>

          <div className="grid gap-3 sm:grid-cols-2">
            <PriceInput
              id="pricing-input"
              label="输入 USD / 1M"
              value={draft.inputUsdPerMillion}
              disabled={pending}
              onChange={(value) =>
                updateRule(draft, { inputUsdPerMillion: value }, onChange)
              }
            />
            <PriceInput
              id="pricing-output"
              label="输出 USD / 1M"
              value={draft.outputUsdPerMillion}
              disabled={pending}
              onChange={(value) =>
                updateRule(draft, { outputUsdPerMillion: value }, onChange)
              }
            />
          </div>
          <FieldError>{fieldErrors?.prices}</FieldError>

          <details className="rounded-lg border px-3 py-2 text-sm">
            <summary className="cursor-pointer font-medium">
              缓存价格（可选）
            </summary>
            <div className="mt-3 grid gap-3 sm:grid-cols-2">
              <PriceInput
                id="pricing-cache-read"
                label="缓存读取 USD / 1M"
                value={draft.cachedReadUsdPerMillion}
                disabled={pending}
                onChange={(value) =>
                  updateRule(
                    draft,
                    { cachedReadUsdPerMillion: value },
                    onChange
                  )
                }
              />
              <PriceInput
                id="pricing-cache-write"
                label="缓存写入 USD / 1M"
                value={draft.cacheWriteUsdPerMillion}
                disabled={pending}
                onChange={(value) =>
                  updateRule(
                    draft,
                    { cacheWriteUsdPerMillion: value },
                    onChange
                  )
                }
              />
            </div>
            <p className="mt-2 text-xs text-muted-foreground">
              留空时缓存按输入价格估算。
            </p>
          </details>

          <div className="flex items-center justify-between gap-4 rounded-lg border px-3 py-3">
            <div>
              <p className="text-sm font-medium">保存后重算当前范围</p>
              <p className="text-xs text-muted-foreground">
                只重算更新后新周期内的当前筛选范围。
              </p>
            </div>
            <Switch
              id="pricing-reprice-after-save"
              aria-label="保存后重算当前范围"
              disabled={pending}
              checked={repriceAfterSave}
              onCheckedChange={onRepriceChange}
            />
          </div>
        </FieldGroup>

        <DialogFooter>
          <Button
            variant="outline"
            disabled={pending}
            onClick={() => onOpenChange(false)}
          >
            取消
          </Button>
          <Button disabled={pending} onClick={onSave}>
            {pending ? (
              <Spinner data-icon="inline-start" />
            ) : (
              <Save data-icon="inline-start" />
            )}
            {pending
              ? repriceAfterSave
                ? "保存并重算中…"
                : "保存中…"
              : repriceAfterSave
                ? "保存并重算"
                : "保存价格"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}

function PriceInput({
  id,
  label,
  value,
  disabled,
  onChange,
}: {
  id: string
  label: string
  value?: string
  disabled: boolean
  onChange: (value: string | undefined) => void
}) {
  return (
    <Field>
      <FieldLabel htmlFor={id}>{label}</FieldLabel>
      <Input
        id={id}
        inputMode="decimal"
        disabled={disabled}
        placeholder="0.00"
        value={value ?? ""}
        onChange={(event) =>
          onChange(event.target.value.trim() ? event.target.value : undefined)
        }
      />
    </Field>
  )
}
