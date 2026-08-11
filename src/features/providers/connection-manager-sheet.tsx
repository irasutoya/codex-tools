import { useMemo, useRef, useState } from "react"
import {
  ApiIcon,
  Delete02Icon,
  Edit02Icon,
  Key01Icon,
  Login03Icon,
  MoreHorizontalIcon,
  Refresh01Icon,
  TestTube01Icon,
  UserMultipleIcon,
} from "@hugeicons/core-free-icons"
import { HugeiconsIcon } from "@hugeicons/react"

import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog"
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
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import {
  Field,
  FieldDescription,
  FieldGroup,
  FieldLabel,
} from "@/components/ui/field"
import { Input } from "@/components/ui/input"
import {
  Item,
  ItemActions,
  ItemContent,
  ItemDescription,
  ItemFooter,
  ItemGroup,
  ItemMedia,
  ItemTitle,
} from "@/components/ui/item"
import {
  Sheet,
  SheetBody,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
} from "@/components/ui/sheet"
import { Skeleton } from "@/components/ui/skeleton"
import { Spinner } from "@/components/ui/spinner"
import { hiddenOverlayStyles } from "@/components/ui/overlay-styles"
import { toast } from "@/components/ui/toast"
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip"
import { errorMessage, quotaWindow } from "@/lib/format"
import { call } from "@/lib/ipc"
import type { OfficialAccountView, Provider, ProviderOverview } from "@/types"
import { emptyProvider } from "@/types"

import { AccountManagerDialog } from "./account-manager-dialog"
import {
  refreshAccountQuota,
  testProviderConnection,
} from "./connection-actions"
import { ProviderEditorDialog } from "./provider-editor-dialog"

type ConnectionKind = "account" | "provider"
type ConnectionPage = "dashboard" | "providers"

type PendingAction = {
  action:
    "activate" | "delete" | "quota" | "login" | "test" | "models" | "remark"
  id: string
}

type DeleteTarget = {
  active: boolean
  id: string
  kind: ConnectionKind
  name: string
}

type FallbackCandidate = {
  id: string
  kind: ConnectionKind
}

const EMPTY_ACCOUNTS: OfficialAccountView[] = []
const EMPTY_PROVIDERS: Provider[] = []

export function ConnectionManagerSheet({
  open,
  onOpenChange,
  page,
  connections,
  selectedId,
  onSelectedIdChange,
  onChanged,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  page: ConnectionPage
  connections?: ProviderOverview
  selectedId?: string
  onSelectedIdChange: (id: string) => void
  onChanged: () => void
}) {
  const [pending, setPending] = useState<PendingAction>()
  const pendingRef = useRef<PendingAction | undefined>(undefined)
  const [remarkAccount, setRemarkAccount] = useState<OfficialAccountView>()
  const [remarkDraft, setRemarkDraft] = useState("")
  const [providerDraft, setProviderDraft] = useState<Provider>(emptyProvider())
  const [providerEditorOpen, setProviderEditorOpen] = useState(false)
  const [deleteTarget, setDeleteTarget] = useState<DeleteTarget>()
  const [accountManagerOpen, setAccountManagerOpen] = useState(false)

  const accounts = connections?.officialAccounts ?? EMPTY_ACCOUNTS
  const providers = connections?.providers ?? EMPTY_PROVIDERS
  const frozen = Boolean(pending)
  const childOverlayOpen = Boolean(
    remarkAccount || providerEditorOpen || deleteTarget || accountManagerOpen
  )

  const beginAction = (action: PendingAction["action"], id: string) => {
    if (pendingRef.current) return false
    const next = { action, id }
    pendingRef.current = next
    setPending(next)
    return true
  }

  const endAction = (action: PendingAction["action"], id: string) => {
    if (pendingRef.current?.action !== action || pendingRef.current.id !== id) {
      return
    }
    pendingRef.current = undefined
    setPending(undefined)
  }

  const runAction = async (
    action: PendingAction["action"],
    id: string,
    task: () => Promise<unknown>,
    success: string
  ) => {
    if (!beginAction(action, id)) return false
    try {
      await task()
      onChanged()
      toast.add({ title: success, type: "success" })
      return true
    } catch (reason) {
      onChanged()
      toast.add({
        title: "操作失败",
        description: errorMessage(reason),
        type: "error",
      })
      return false
    } finally {
      endAction(action, id)
    }
  }

  const activate = async (kind: ConnectionKind, id: string) => {
    const item =
      kind === "account"
        ? accounts.find((account) => account.id === id)
        : providers.find((provider) => provider.id === id)
    if (!item || item.active || pendingRef.current) return
    const activated = await runAction(
      "activate",
      id,
      () =>
        kind === "account"
          ? call("connections_activate_account", { id })
          : call("connections_activate", { id }),
      "连接已切换"
    )
    if (activated) onSelectedIdChange(id)
  }

  const editAccount = (account: OfficialAccountView) => {
    if (pendingRef.current) return
    setRemarkAccount(account)
    setRemarkDraft(account.remark)
  }

  const saveRemark = async () => {
    if (!remarkAccount) return
    const saved = await runAction(
      "remark",
      remarkAccount.id,
      () =>
        call("connections_update_account_remark", {
          id: remarkAccount.id,
          remark: remarkDraft,
        }),
      "账号备注已保存"
    )
    if (saved) setRemarkAccount(undefined)
  }

  const editProvider = (provider: Provider) => {
    if (pendingRef.current) return
    setProviderDraft({
      ...provider,
      headers: { ...provider.headers },
      availableModels: [...(provider.availableModels ?? [])],
    })
    setProviderEditorOpen(true)
  }

  const fallbackCandidates = (target: DeleteTarget) => {
    const remainingAccounts = accounts.filter(
      (account) => !(target.kind === "account" && account.id === target.id)
    )
    const healthyAccounts = remainingAccounts.filter(
      (account) =>
        account.quota.status !== "unauthorized" && !accountIsExpired(account)
    )
    const healthyIds = new Set(healthyAccounts.map((account) => account.id))
    const otherAccounts = remainingAccounts.filter(
      (account) => !healthyIds.has(account.id)
    )
    const enabledProviders = providers.filter(
      (provider) =>
        !(target.kind === "provider" && provider.id === target.id) &&
        provider.enabled &&
        provider.hasApiKey
    )

    return [
      ...healthyAccounts.map((account): FallbackCandidate => ({
        kind: "account",
        id: account.id,
      })),
      ...enabledProviders.map((provider): FallbackCandidate => ({
        kind: "provider",
        id: provider.id,
      })),
      ...otherAccounts.map((account): FallbackCandidate => ({
        kind: "account",
        id: account.id,
      })),
    ]
  }

  const requestDelete = (target: DeleteTarget) => {
    if (pendingRef.current) return
    if (target.active && fallbackCandidates(target).length === 0) {
      toast.add({
        title: `无法删除“${target.name}”`,
        description: "至少保留一个可用连接，才能删除当前连接。",
        type: "error",
      })
      return
    }
    setDeleteTarget(target)
  }

  const deleteConnection = async () => {
    const target = deleteTarget
    if (!target || !beginAction("delete", target.id)) return

    let switchedId: string | undefined
    let lastSwitchError: unknown
    try {
      if (target.active) {
        for (const candidate of fallbackCandidates(target)) {
          try {
            await (candidate.kind === "account"
              ? call("connections_activate_account", { id: candidate.id })
              : call("connections_activate", { id: candidate.id }))
            switchedId = candidate.id
            break
          } catch (reason) {
            lastSwitchError = reason
          }
        }

        if (!switchedId) {
          onChanged()
          toast.add({
            title: "无法切换连接，未执行删除",
            description: lastSwitchError
              ? `其余连接均无法启用：${errorMessage(lastSwitchError)}`
              : "至少保留一个可用连接，才能删除当前连接。",
            type: "error",
          })
          setDeleteTarget(undefined)
          return
        }
      }

      await (target.kind === "account"
        ? call("connections_delete_accounts", { ids: [target.id] })
        : call("connections_delete_provider", { id: target.id }))

      if (!selectedId || selectedId === target.id) {
        const remaining = [
          ...accounts.filter(
            (account) =>
              !(target.kind === "account" && account.id === target.id)
          ),
          ...providers.filter(
            (provider) =>
              !(target.kind === "provider" && provider.id === target.id)
          ),
        ]
        const nextId =
          switchedId ??
          remaining.find((connection) => connection.active)?.id ??
          remaining[0]?.id ??
          ""
        onSelectedIdChange(nextId)
      }

      onChanged()
      toast.add({ title: `已删除“${target.name}”`, type: "success" })
      setDeleteTarget(undefined)
    } catch (reason) {
      onChanged()
      toast.add({
        title: switchedId ? "已切换连接，但删除失败" : "删除连接失败",
        description: errorMessage(reason),
        type: "error",
      })
      setDeleteTarget(undefined)
    } finally {
      endAction("delete", target.id)
    }
  }

  const requestSheetOpenChange = (nextOpen: boolean) => {
    if (!nextOpen && pendingRef.current) return
    onOpenChange(nextOpen)
  }

  const openBatchManager = () => {
    if (pendingRef.current || accounts.length === 0) return
    onOpenChange(false)
    setAccountManagerOpen(true)
  }

  const accountRows = useMemo(
    () =>
      accounts.map((account) => ({
        account,
        expired: accountIsExpired(account),
      })),
    [accounts]
  )

  return (
    <>
      <Sheet open={open} onOpenChange={requestSheetOpenChange}>
        <SheetContent
          side="left"
          showCloseButton={!frozen}
          overlayClassName={childOverlayOpen ? hiddenOverlayStyles : undefined}
          aria-busy={frozen}
        >
          <SheetHeader>
            <SheetTitle>账号与服务</SheetTitle>
            <SheetDescription>
              查看、切换并管理 Codex 保存的全部连接。
            </SheetDescription>
          </SheetHeader>

          <SheetBody className="gap-4">
            {!connections && <ConnectionListSkeleton />}

            {connections && (
              <>
                <section
                  className="flex flex-col gap-1.5"
                  aria-labelledby="account-group-title"
                >
                  <div className="flex items-center justify-between gap-2 px-1">
                    <h3
                      id="account-group-title"
                      className="text-xs font-medium text-muted-foreground"
                    >
                      OpenAI 账号
                    </h3>
                    <Button
                      type="button"
                      size="xs"
                      variant="ghost"
                      disabled={frozen || accounts.length === 0}
                      onClick={openBatchManager}
                    >
                      <HugeiconsIcon
                        icon={UserMultipleIcon}
                        data-icon="inline-start"
                      />
                      批量管理
                    </Button>
                  </div>
                  <ItemGroup>
                    {accountRows.length === 0 && (
                      <EmptyConnectionItem label="暂无 OpenAI 账号" />
                    )}
                    {accountRows.map(({ account, expired }) => (
                      <ConnectionItem
                        key={account.id}
                        kind="account"
                        id={account.id}
                        name={account.remark || account.name}
                        description={accountDescription(account)}
                        active={account.active}
                        canView={page === "providers"}
                        selected={
                          page === "providers" && selectedId === account.id
                        }
                        unavailable={
                          expired || account.quota.status === "unauthorized"
                        }
                        unavailableLabel="登录失效"
                        frozen={frozen}
                        pending={pending}
                        onView={() => {
                          onSelectedIdChange(account.id)
                          onOpenChange(false)
                        }}
                        onActivate={() => void activate("account", account.id)}
                        onEdit={() => editAccount(account)}
                        onDelete={() =>
                          requestDelete({
                            active: account.active,
                            id: account.id,
                            kind: "account",
                            name: account.remark || account.name,
                          })
                        }
                        moreActions={[
                          {
                            label: "刷新额度",
                            icon: Refresh01Icon,
                            onSelect: () =>
                              void runAction(
                                "quota",
                                account.id,
                                () => refreshAccountQuota(account.id),
                                "额度已刷新"
                              ),
                          },
                          {
                            label: "刷新登录",
                            icon: Login03Icon,
                            onSelect: () =>
                              void runAction(
                                "login",
                                account.id,
                                () =>
                                  call("connections_refresh_login", {
                                    id: account.id,
                                  }),
                                "登录状态已刷新"
                              ),
                          },
                        ]}
                      />
                    ))}
                  </ItemGroup>
                </section>

                <section
                  className="flex flex-col gap-1.5"
                  aria-labelledby="provider-group-title"
                >
                  <h3
                    id="provider-group-title"
                    className="px-1 text-xs font-medium text-muted-foreground"
                  >
                    API 服务
                  </h3>
                  <ItemGroup>
                    {providers.length === 0 && (
                      <EmptyConnectionItem label="暂无 API 服务" />
                    )}
                    {providers.map((provider) => (
                      <ConnectionItem
                        key={provider.id}
                        kind="provider"
                        id={provider.id}
                        name={provider.name}
                        description={`${
                          provider.availableModels?.length
                            ? `${provider.availableModels.length} 个模型`
                            : "模型尚未同步"
                        } · ${provider.baseUrl}`}
                        active={provider.active}
                        canView={page === "providers"}
                        selected={
                          page === "providers" && selectedId === provider.id
                        }
                        unavailable={!provider.enabled || !provider.hasApiKey}
                        unavailableLabel={
                          provider.enabled ? "缺少密钥" : "已停用"
                        }
                        activateDisabled={
                          !provider.enabled || !provider.hasApiKey
                        }
                        frozen={frozen}
                        pending={pending}
                        onView={() => {
                          onSelectedIdChange(provider.id)
                          onOpenChange(false)
                        }}
                        onActivate={() =>
                          void activate("provider", provider.id)
                        }
                        onEdit={() => editProvider(provider)}
                        onDelete={() =>
                          requestDelete({
                            active: provider.active,
                            id: provider.id,
                            kind: "provider",
                            name: provider.name,
                          })
                        }
                        moreActions={[
                          {
                            label: "测试连接",
                            icon: TestTube01Icon,
                            onSelect: () =>
                              void runAction(
                                "test",
                                provider.id,
                                () => testProviderConnection(provider.id),
                                "连接测试通过"
                              ),
                          },
                          {
                            label: "同步模型",
                            icon: Refresh01Icon,
                            onSelect: () =>
                              void runAction(
                                "models",
                                provider.id,
                                () =>
                                  call("connections_list_models", {
                                    id: provider.id,
                                  }),
                                "模型已同步"
                              ),
                          },
                        ]}
                      />
                    ))}
                  </ItemGroup>
                </section>
              </>
            )}
          </SheetBody>
        </SheetContent>
      </Sheet>

      <Dialog
        open={Boolean(remarkAccount)}
        onOpenChange={(nextOpen) => {
          if (!nextOpen && pendingRef.current) return
          if (!nextOpen) setRemarkAccount(undefined)
        }}
      >
        <DialogContent showCloseButton={!frozen} aria-busy={frozen}>
          <DialogHeader>
            <DialogTitle>编辑账号备注</DialogTitle>
            <DialogDescription>
              备注仅保存在本机，用于区分相近的账号。
            </DialogDescription>
          </DialogHeader>
          <DialogBody>
            <FieldGroup>
              <Field data-disabled={frozen}>
                <FieldLabel htmlFor="sheet-account-remark">账号备注</FieldLabel>
                <Input
                  id="sheet-account-remark"
                  autoFocus
                  disabled={frozen}
                  maxLength={200}
                  placeholder={remarkAccount?.name || "例如：工作账号"}
                  value={remarkDraft}
                  onChange={(event) => setRemarkDraft(event.target.value)}
                />
                <FieldDescription>留空可恢复显示账号原名称。</FieldDescription>
              </Field>
            </FieldGroup>
          </DialogBody>
          <DialogFooter>
            <Button
              type="button"
              variant="outline"
              disabled={frozen}
              onClick={() => setRemarkAccount(undefined)}
            >
              取消
            </Button>
            <Button
              type="button"
              disabled={
                frozen || remarkDraft.trim() === (remarkAccount?.remark ?? "")
              }
              onClick={() => void saveRemark()}
            >
              {pending?.action === "remark" && (
                <Spinner data-icon="inline-start" />
              )}
              保存
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <ProviderEditorDialog
        open={providerEditorOpen}
        onOpenChange={setProviderEditorOpen}
        provider={providerDraft}
        onProviderChange={setProviderDraft}
        onSaved={() => {
          setProviderEditorOpen(false)
          onChanged()
        }}
      />

      <AlertDialog
        open={Boolean(deleteTarget)}
        onOpenChange={(nextOpen) => {
          if (!nextOpen && pendingRef.current) return
          if (!nextOpen) setDeleteTarget(undefined)
        }}
      >
        <AlertDialogContent aria-busy={pending?.action === "delete"}>
          <AlertDialogHeader>
            <AlertDialogTitle>
              删除“{deleteTarget?.name ?? "这个连接"}”？
            </AlertDialogTitle>
            <AlertDialogDescription>
              {deleteTarget?.active
                ? "这是当前连接。程序会先切换到其他可用连接，再执行删除。"
                : "此操作会移除本机保存的连接与凭据信息，无法撤销。"}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel disabled={pending?.action === "delete"}>
              取消
            </AlertDialogCancel>
            <AlertDialogAction
              variant="destructive"
              disabled={pending?.action === "delete"}
              onClick={() => void deleteConnection()}
            >
              {pending?.action === "delete" && (
                <Spinner data-icon="inline-start" />
              )}
              删除“{deleteTarget?.name ?? "连接"}”
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>

      <AccountManagerDialog
        open={accountManagerOpen}
        onOpenChange={setAccountManagerOpen}
        accounts={accounts}
        providers={providers}
        initialSelectedIds={[]}
        selectedId={selectedId}
        onSelectedIdChange={onSelectedIdChange}
        onRefresh={onChanged}
      />
    </>
  )
}

type MoreAction = {
  icon: Parameters<typeof HugeiconsIcon>[0]["icon"]
  label: string
  onSelect: () => void
}

function ConnectionItem({
  kind,
  id,
  name,
  description,
  active,
  canView,
  selected,
  unavailable,
  unavailableLabel,
  activateDisabled = false,
  frozen,
  pending,
  onView,
  onActivate,
  onEdit,
  onDelete,
  moreActions,
}: {
  kind: ConnectionKind
  id: string
  name: string
  description: string
  active: boolean
  canView: boolean
  selected: boolean
  unavailable: boolean
  unavailableLabel: string
  activateDisabled?: boolean
  frozen: boolean
  pending?: PendingAction
  onView: () => void
  onActivate: () => void
  onEdit: () => void
  onDelete: () => void
  moreActions: MoreAction[]
}) {
  const activating = pending?.action === "activate" && pending.id === id
  const rowPending = pending?.id === id

  return (
    <Item
      size="sm"
      variant={active || selected ? "muted" : "outline"}
      aria-label={`${kind === "account" ? "账号" : "API 服务"} ${name}`}
    >
      <ItemMedia variant="icon">
        <HugeiconsIcon icon={kind === "account" ? Key01Icon : ApiIcon} />
      </ItemMedia>
      <ItemContent title={description}>
        <ItemTitle className="w-full">{name}</ItemTitle>
        <ItemDescription className="truncate">{description}</ItemDescription>
      </ItemContent>
      <ItemActions className="max-w-full flex-wrap justify-end gap-1 self-start">
        {active && <Badge>当前</Badge>}
        {selected && <Badge variant="secondary">已选</Badge>}
        {unavailable && <Badge variant="destructive">{unavailableLabel}</Badge>}
      </ItemActions>
      <ItemFooter className="justify-end gap-1.5">
        {canView && !selected && (
          <Button
            type="button"
            size="xs"
            variant="ghost"
            className="mr-auto"
            disabled={frozen}
            onClick={onView}
          >
            查看
          </Button>
        )}
        <Button
          type="button"
          size="xs"
          variant="outline"
          disabled={frozen || active || activateDisabled}
          aria-busy={activating}
          onClick={onActivate}
        >
          {activating && <Spinner data-icon="inline-start" />}
          设为当前
        </Button>
        <Tooltip>
          <TooltipTrigger
            render={
              <Button
                type="button"
                size="icon-xs"
                variant="outline"
                aria-label={`编辑${kind === "account" ? "账号" : "服务"}：${name}`}
                disabled={frozen}
                onClick={onEdit}
              />
            }
          >
            <HugeiconsIcon icon={Edit02Icon} />
          </TooltipTrigger>
          <TooltipContent>编辑</TooltipContent>
        </Tooltip>
        <Tooltip>
          <TooltipTrigger
            render={
              <Button
                type="button"
                size="icon-xs"
                variant="destructive"
                aria-label={`删除${kind === "account" ? "账号" : "服务"}：${name}`}
                disabled={frozen}
                onClick={onDelete}
              />
            }
          >
            <HugeiconsIcon icon={Delete02Icon} />
          </TooltipTrigger>
          <TooltipContent>删除</TooltipContent>
        </Tooltip>
        <DropdownMenu>
          <DropdownMenuTrigger
            render={
              <Button
                type="button"
                size="icon-xs"
                variant="ghost"
                aria-label={`更多管理操作：${name}`}
                title={`更多管理操作：${name}`}
                disabled={frozen}
              />
            }
          >
            {rowPending && !activating && pending?.action !== "delete" ? (
              <Spinner />
            ) : (
              <HugeiconsIcon icon={MoreHorizontalIcon} />
            )}
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end">
            <DropdownMenuGroup>
              {moreActions.map((action) => (
                <DropdownMenuItem
                  key={action.label}
                  disabled={frozen}
                  onClick={action.onSelect}
                >
                  <HugeiconsIcon icon={action.icon} />
                  {action.label}
                </DropdownMenuItem>
              ))}
            </DropdownMenuGroup>
          </DropdownMenuContent>
        </DropdownMenu>
      </ItemFooter>
    </Item>
  )
}

function EmptyConnectionItem({ label }: { label: string }) {
  return (
    <Item size="xs" variant="outline">
      <ItemContent>
        <ItemDescription>{label}</ItemDescription>
      </ItemContent>
    </Item>
  )
}

function accountDescription(account: OfficialAccountView) {
  const quota = quotaWindow(account.quota)
  const quotaText = quota
    ? `剩余 ${quota.remainingPercent.toFixed(1)}%`
    : account.quota.status === "never"
      ? "额度未刷新"
      : account.quota.status === "unauthorized"
        ? "登录已失效"
        : "额度不可用"
  return `${quotaText} · ${account.email || account.name || "OpenAI 账号"}`
}

function accountIsExpired(account: OfficialAccountView) {
  return (
    account.expiresAt != null &&
    account.expiresAt <= Math.floor(Date.now() / 1000)
  )
}

function ConnectionListSkeleton() {
  return (
    <div className="flex flex-col gap-4" aria-label="正在读取连接">
      {[0, 1].map((group) => (
        <div key={group} className="flex flex-col gap-2">
          <Skeleton className="h-3 w-20" />
          {[0, 1].map((item) => (
            <div key={item} className="flex items-center gap-2 px-1 py-1.5">
              <Skeleton className="size-6 rounded-full" />
              <div className="flex min-w-0 flex-1 flex-col gap-1.5">
                <Skeleton className="h-3.5 w-24" />
                <Skeleton className="h-3 w-full" />
              </div>
            </div>
          ))}
        </div>
      ))}
    </div>
  )
}
