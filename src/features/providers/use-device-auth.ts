import { useEffect, useState } from "react"

import { notify } from "@/lib/feedback"
import { call } from "@/lib/ipc"
import { refreshCoordinator } from "@/lib/refresh-coordinator"
import { notifyRepairWarnings } from "@/lib/repair-feedback"
import { epochMilliseconds } from "@/lib/time"
import { runQuotaRefresh } from "@/lib/use-auto-quota-refresh"
import type { DeviceAuthorization } from "@/types"

export function useDeviceAuthorizationPolling(onRefresh: () => Promise<void>) {
  const [deviceAuthorization, setDeviceAuthorization] =
    useState<DeviceAuthorization>()

  useEffect(() => {
    if (!deviceAuthorization) return
    const authorization = deviceAuthorization
    let cancelled = false
    let timer: number | undefined
    let pollErrorShown = false

    const scheduleNextPoll = () => {
      const remainingMs =
        epochMilliseconds(authorization.expiresAt) - Date.now()
      if (remainingMs <= 0) {
        setDeviceAuthorization(undefined)
        notify.error("登录码已过期", "请重新生成登录码后继续登录。")
        return
      }
      timer = window.setTimeout(
        () => {
          void call("poll_openai_device_auth", {
            operationId: authorization.operationId,
          })
            .then((result) => {
              if (cancelled) return
              pollErrorShown = false
              if (result.status === "pending") {
                scheduleNextPoll()
                return
              }
              setDeviceAuthorization(undefined)
              if (result.status === "expired") {
                notify.error("登录码已过期", "请重新生成登录码后继续登录。")
                return
              }
              notify.success(
                "OpenAI 登录成功",
                `Codex 现在使用 ${result.account.name}。`
              )
              notifyRepairWarnings(result.repair)
              void runQuotaRefresh(result.account.id, () =>
                call("refresh_official_account_quota", {
                  accountId: result.account.id,
                })
              )
                .catch((error) =>
                  notify.warning("登录成功，但额度暂未更新", error)
                )
                .finally(() => {
                  refreshCoordinator.invalidate(["dashboard", "settings"])
                  return onRefresh().catch((error) =>
                    notify.warning("登录已完成，但无法读取最新账号列表", error)
                  )
                })
            })
            .catch((error) => {
              if (cancelled) return
              if (!pollErrorShown) {
                pollErrorShown = true
                notify.warning("暂时无法确认登录结果，程序将自动重试", error)
              }
              scheduleNextPoll()
            })
        },
        Math.min(authorization.intervalSecs * 1000, remainingMs)
      )
    }

    scheduleNextPoll()
    return () => {
      cancelled = true
      if (timer !== undefined) window.clearTimeout(timer)
    }
  }, [deviceAuthorization, onRefresh])

  return [deviceAuthorization, setDeviceAuthorization] as const
}
