import type {
  CostStatus,
  UsageOverview,
  UsageRow,
  UsageShareAccount,
  UsageShareAccountModel,
  UsageShareData,
  UsageSourceKind,
} from "@/types"

import { formatTokens, formatUsdMicrousd } from "./usage-format"

export type UsageShareMode = "details" | "summary"

const SHARE_WIDTH = 1080
const SHARE_HEADER_HEIGHT = 350
const SHARE_SECTION_HEIGHT = 54
const SHARE_ACCOUNT_HEADER_HEIGHT = 68
const SHARE_MODEL_ROW_HEIGHT = 72
const SHARE_FOOTER_HEIGHT = 92
const MAX_VISIBLE_ACCOUNTS = 6
const MAX_VISIBLE_MODELS = 4

function accountKeyForRow(row: UsageRow) {
  return `account:${row.sourceKind}:${row.providerId ?? ""}:${row.accountId ?? "unattributed"}`
}

function sourceLabel(sourceKind: UsageSourceKind) {
  if (sourceKind === "official") return "官方 OpenAI"
  if (sourceKind === "provider") return "中转站"
  return "未识别"
}

function rowUnpricedTokens(row: UsageRow) {
  return row.costStatus === "unpriced" ? row.tokens.totalTokens : 0
}

function rowPartialTokens(row: UsageRow) {
  return row.costStatus === "partial" ? row.tokens.totalTokens : 0
}

function mergeCostStatus(left: CostStatus, right: CostStatus): CostStatus {
  const priority: Record<CostStatus, number> = {
    zero: 0,
    estimated: 1,
    subscription: 2,
    unattributed: 3,
    unpriced: 4,
    partial: 5,
  }
  return priority[right] > priority[left] ? right : left
}

function rowToModel(row: UsageRow, accountKey: string): UsageShareAccountModel {
  return {
    key: `${accountKey}:${row.model}`,
    model: row.model,
    totalTokens: row.tokens.totalTokens,
    estimatedCostMicrousd: row.estimatedCostMicrousd ?? 0,
    unpricedTokens: rowUnpricedTokens(row),
    partialTokens: rowPartialTokens(row),
    requests: row.requests,
    costStatus: row.costStatus,
  }
}

function rowToAccount(row: UsageRow): UsageShareAccount {
  return {
    key: accountKeyForRow(row),
    displayName:
      row.sourceName ||
      (row.sourceKind === "official" ? "官方 OpenAI" : "未知账号"),
    maskedName: maskAccountName(row.sourceName, row.sourceKind),
    sourceKind: row.sourceKind,
    totalTokens: row.tokens.totalTokens,
    estimatedCostMicrousd: row.estimatedCostMicrousd ?? 0,
    unpricedTokens: rowUnpricedTokens(row),
    partialTokens: rowPartialTokens(row),
    requests: row.requests,
    costStatus: row.costStatus,
    models: [],
  }
}

export function maskAccountName(name: string, sourceKind: UsageSourceKind) {
  const value =
    name.trim() || (sourceKind === "official" ? "官方 OpenAI" : "未知账号")
  const at = value.indexOf("@")
  if (at > 0 && at < value.length - 1) {
    return `${value.slice(0, 1)}***@${value.slice(at + 1)}`
  }
  if (value.length <= 4) return `${value.slice(0, 1)}***`
  return `${value.slice(0, 1)}***${value.slice(-1)}`
}

export function buildUsageShareData(
  accountOverview: UsageOverview,
  modelOverview: UsageOverview,
  dateLabel: string,
  timezone: string
): UsageShareData {
  const accountsByKey = new Map<string, UsageShareAccount>()
  for (const row of accountOverview.rows) {
    accountsByKey.set(accountKeyForRow(row), rowToAccount(row))
  }

  for (const row of modelOverview.rows) {
    const accountKey = accountKeyForRow(row)
    const account = accountsByKey.get(accountKey)
    if (account) {
      const nextModel = rowToModel(row, accountKey)
      const existing = account.models.find(
        (model) => model.key === nextModel.key
      )
      if (existing) {
        existing.totalTokens += nextModel.totalTokens
        existing.estimatedCostMicrousd += nextModel.estimatedCostMicrousd
        existing.unpricedTokens += nextModel.unpricedTokens
        existing.partialTokens += nextModel.partialTokens
        existing.requests += nextModel.requests
        existing.costStatus = mergeCostStatus(
          existing.costStatus,
          nextModel.costStatus
        )
      } else {
        account.models.push(nextModel)
      }
      continue
    }

    // Keep a model visible even if the account aggregate did not return a row.
    const fallback = rowToAccount(row)
    fallback.totalTokens = 0
    fallback.estimatedCostMicrousd = 0
    fallback.unpricedTokens = 0
    fallback.partialTokens = 0
    fallback.requests = 0
    fallback.models.push(rowToModel(row, accountKey))
    accountsByKey.set(accountKey, fallback)
  }

  const accounts = [...accountsByKey.values()]
    .map((account) => ({
      ...account,
      models: account.models.sort(
        (left, right) => right.totalTokens - left.totalTokens
      ),
    }))
    .sort((left, right) => right.totalTokens - left.totalTokens)

  return {
    dateLabel,
    timezone,
    totalTokens: accountOverview.totals.tokens.totalTokens,
    estimatedCostMicrousd: accountOverview.totals.estimatedCostMicrousd,
    unpricedTokens: accountOverview.totals.unpricedTokens,
    partialTokens: accountOverview.totals.partialTokens,
    requests: accountOverview.totals.requests,
    accounts,
  }
}

function escapeXml(value: string) {
  return value.replace(
    /[&<>'"]/g,
    (character) =>
      ({
        "&": "&amp;",
        "<": "&lt;",
        ">": "&gt;",
        "'": "&apos;",
        '"': "&quot;",
      })[character] ?? character
  )
}

function shareCost(value: number) {
  return formatUsdMicrousd(value)
}

function shareTokens(value: number) {
  return formatTokens(value)
}

function modelCost(model: UsageShareAccountModel) {
  if (model.costStatus === "subscription") return "套餐"
  if (model.costStatus === "unpriced") return "未配置价格"
  if (model.costStatus === "unattributed") return "未归属"
  if (model.costStatus === "partial") {
    return model.estimatedCostMicrousd > 0
      ? `约 ${shareCost(model.estimatedCostMicrousd)}`
      : "部分未估算"
  }
  return shareCost(model.estimatedCostMicrousd)
}

function accountCost(account: UsageShareAccount) {
  if (account.costStatus === "subscription") return "套餐"
  if (account.costStatus === "unattributed") return "未归属"
  if (account.unpricedTokens > 0) {
    return account.estimatedCostMicrousd > 0
      ? `约 ${shareCost(account.estimatedCostMicrousd)}`
      : "未配置价格"
  }
  if (account.partialTokens > 0) {
    return account.estimatedCostMicrousd > 0
      ? `约 ${shareCost(account.estimatedCostMicrousd)}`
      : "部分未估算"
  }
  return shareCost(account.estimatedCostMicrousd)
}

type ShareAccountLayout = {
  account: UsageShareAccount
  models: UsageShareAccountModel[]
  hiddenModels: number
  y: number
}

function getShareLayout(
  data: UsageShareData,
  mode: UsageShareMode,
  showAllAccounts: boolean,
  showAllModels: boolean
) {
  const visibleAccounts = showAllAccounts
    ? data.accounts
    : data.accounts.slice(0, MAX_VISIBLE_ACCOUNTS)
  const hiddenAccounts = Math.max(
    0,
    data.accounts.length - visibleAccounts.length
  )
  const accountLayouts: ShareAccountLayout[] = []
  let cursor =
    mode === "details"
      ? SHARE_HEADER_HEIGHT + SHARE_SECTION_HEIGHT
      : SHARE_HEADER_HEIGHT

  if (mode === "details") {
    for (const account of visibleAccounts) {
      const models = showAllModels
        ? account.models
        : account.models.slice(0, MAX_VISIBLE_MODELS)
      const hiddenModels = Math.max(0, account.models.length - models.length)
      accountLayouts.push({ account, models, hiddenModels, y: cursor })
      cursor +=
        SHARE_ACCOUNT_HEADER_HEIGHT +
        models.length * SHARE_MODEL_ROW_HEIGHT +
        (hiddenModels > 0 ? 30 : 0)
    }
    if (hiddenAccounts > 0) cursor += 34
  }

  return {
    visibleAccounts,
    hiddenAccounts,
    accountLayouts,
    height: cursor + SHARE_FOOTER_HEIGHT,
    title:
      mode === "summary" ? "今日 Token 用量" : "今日 Token 用量 · 账号与模型",
    costLabel:
      data.unpricedTokens > 0 || data.partialTokens > 0
        ? data.estimatedCostMicrousd > 0
          ? `约 ${shareCost(data.estimatedCostMicrousd)}`
          : "部分未估算"
        : shareCost(data.estimatedCostMicrousd),
  }
}

export function renderUsageShareSvg(
  data: UsageShareData,
  mode: UsageShareMode = "details",
  maskAccounts = true,
  showAllAccounts = false,
  showAllModels = false
) {
  const layout = getShareLayout(data, mode, showAllAccounts, showAllModels)
  const accountGroups = layout.accountLayouts
    .map(({ account, models, hiddenModels, y }) => {
      const maxTokens = Math.max(1, ...models.map((model) => model.totalTokens))
      const name = maskAccounts ? account.maskedName : account.displayName
      const modelRows = models
        .map((model, index) => {
          const modelY =
            y + SHARE_ACCOUNT_HEADER_HEIGHT + index * SHARE_MODEL_ROW_HEIGHT
          const barWidth = Math.max(
            12,
            Math.round((model.totalTokens / maxTokens) * 400)
          )
          const meta = `${model.requests} 次调用 · 账号内 ${Math.round((model.totalTokens / Math.max(1, account.totalTokens)) * 100)}%`
          return `<g>
  <text class="model-name" x="106" y="${modelY + 17}">${escapeXml(model.model)}</text>
  <text class="model-meta" x="106" y="${modelY + 42}">${escapeXml(meta)}</text>
  <rect x="420" y="${modelY + 14}" width="400" height="10" rx="5" fill="#e4e4e7" />
  <rect x="420" y="${modelY + 14}" width="${barWidth}" height="10" rx="5" fill="#18181b" />
  <text class="model-value" x="1008" y="${modelY + 18}" text-anchor="end">${escapeXml(shareTokens(model.totalTokens))}</text>
  <text class="model-cost" x="1008" y="${modelY + 42}" text-anchor="end">${escapeXml(modelCost(model))}</text>
</g>`
        })
        .join("")
      const hiddenLabel =
        hiddenModels > 0
          ? `<text class="more-label" x="106" y="${y + SHARE_ACCOUNT_HEADER_HEIGHT + models.length * SHARE_MODEL_ROW_HEIGHT + 8}">另外 ${hiddenModels} 个模型</text>`
          : ""
      return `<g>
  <line x1="72" y1="${y}" x2="1008" y2="${y}" stroke="#dedee3" stroke-width="1" />
  <text class="account-name" x="72" y="${y + 19}">${escapeXml(name)}</text>
  <rect x="${Math.min(350, 92 + name.length * 13)}" y="${y + 7}" width="108" height="24" rx="12" fill="#fafafa" stroke="#dedee3" />
  <text class="source-badge" x="${Math.min(350, 92 + name.length * 13) + 54}" y="${y + 23}" text-anchor="middle">${escapeXml(sourceLabel(account.sourceKind))}</text>
  <text class="account-total" x="1008" y="${y + 19}" text-anchor="end">${escapeXml(shareTokens(account.totalTokens))} · ${account.requests} 次</text>
  <text class="account-cost" x="1008" y="${y + 43}" text-anchor="end">${escapeXml(accountCost(account))}</text>
  ${modelRows}
  ${hiddenLabel}
</g>`
    })
    .join("")
  const details =
    mode === "details"
      ? `<text class="section-title" x="72" y="${SHARE_HEADER_HEIGHT + 32}">账号与模型明细</text>
  <text class="section-count" x="1008" y="${SHARE_HEADER_HEIGHT + 32}" text-anchor="end">${layout.visibleAccounts.length} 个账号</text>
  ${accountGroups}
  ${layout.hiddenAccounts > 0 ? `<text class="more-label" x="72" y="${layout.height - SHARE_FOOTER_HEIGHT - 21}">另外 ${layout.hiddenAccounts} 个账号</text>` : ""}`
      : ""
  const footerY = layout.height - SHARE_FOOTER_HEIGHT

  return `<svg xmlns="http://www.w3.org/2000/svg" width="${SHARE_WIDTH}" height="${layout.height}" viewBox="0 0 ${SHARE_WIDTH} ${layout.height}" role="img" aria-label="${escapeXml(layout.title)}">
  <style>
    .title { fill: #18181b; font: 500 32px 'Segoe UI', 'Microsoft YaHei', sans-serif; }
    .subtitle, .period, .metric-label, .model-meta, .account-cost, .model-cost, .section-count, .more-label, .footer { fill: #71717a; font: 400 16px 'Segoe UI', 'Microsoft YaHei', sans-serif; }
    .metric-value { fill: #18181b; font: 500 28px 'Segoe UI', 'Microsoft YaHei', sans-serif; }
    .section-title { fill: #18181b; font: 500 20px 'Segoe UI', 'Microsoft YaHei', sans-serif; }
    .account-name { fill: #18181b; font: 500 20px 'Segoe UI', 'Microsoft YaHei', sans-serif; }
    .source-badge { fill: #52525b; font: 400 14px 'Segoe UI', 'Microsoft YaHei', sans-serif; }
    .account-total { fill: #18181b; font: 500 18px 'Segoe UI', 'Microsoft YaHei', sans-serif; }
    .model-name { fill: #18181b; font: 500 17px 'Segoe UI', 'Microsoft YaHei', sans-serif; }
    .model-value { fill: #18181b; font: 500 17px 'Segoe UI', 'Microsoft YaHei', sans-serif; }
  </style>
  <rect width="${SHARE_WIDTH}" height="${layout.height}" fill="#f7f7f8" />
  <rect x="24" y="24" width="1032" height="${layout.height - 48}" rx="24" fill="#ffffff" stroke="#dedee3" />
  <rect x="72" y="60" width="34" height="34" rx="9" fill="#18181b" />
  <text x="89" y="83" text-anchor="middle" fill="#ffffff" font="500 15px 'Segoe UI'">C<tspan fill="#84cc16">T</tspan></text>
  <text class="title" x="122" y="68">Codex Tools · 今日用量</text>
  <text class="subtitle" x="122" y="96">${escapeXml(data.dateLabel)} · ${escapeXml(data.timezone)}</text>
  <text class="period" x="1008" y="72" text-anchor="end">当前统计周期</text>
  <text class="period" x="1008" y="96" text-anchor="end">软件更新后产生的新用量</text>
  <rect x="72" y="145" width="300" height="74" rx="10" fill="#f7f7f8" stroke="#ececf0" />
  <rect x="390" y="145" width="300" height="74" rx="10" fill="#f7f7f8" stroke="#ececf0" />
  <rect x="708" y="145" width="300" height="74" rx="10" fill="#f7f7f8" stroke="#ececf0" />
  <text class="metric-label" x="90" y="168">总 Token</text><text class="metric-value" x="90" y="192">${escapeXml(shareTokens(data.totalTokens))}</text>
  <text class="metric-label" x="408" y="168">估算费用</text><text class="metric-value" x="408" y="192">${escapeXml(layout.costLabel)}</text>
  <text class="metric-label" x="726" y="168">调用次数</text><text class="metric-value" x="726" y="192">${data.requests.toLocaleString("en-US")}</text>
  <line x1="72" y1="272" x2="1008" y2="272" stroke="#ececf0" />
  ${details}
  <text class="footer" x="72" y="${footerY + 48}">Codex Tools · 本机统计 · 费用为估算值，不代表官方账单</text>
</svg>`
}

function roundedRect(
  context: CanvasRenderingContext2D,
  x: number,
  y: number,
  width: number,
  height: number,
  radius: number
) {
  const safeRadius = Math.min(radius, width / 2, height / 2)
  context.beginPath()
  context.moveTo(x + safeRadius, y)
  context.lineTo(x + width - safeRadius, y)
  context.quadraticCurveTo(x + width, y, x + width, y + safeRadius)
  context.lineTo(x + width, y + height - safeRadius)
  context.quadraticCurveTo(
    x + width,
    y + height,
    x + width - safeRadius,
    y + height
  )
  context.lineTo(x + safeRadius, y + height)
  context.quadraticCurveTo(x, y + height, x, y + height - safeRadius)
  context.lineTo(x, y + safeRadius)
  context.quadraticCurveTo(x, y, x + safeRadius, y)
  context.closePath()
}

function canvasFont(weight: number, size: number) {
  return `${weight} ${size}px "Segoe UI", "Microsoft YaHei", sans-serif`
}

export async function renderSharePng(
  data: UsageShareData,
  mode: UsageShareMode = "details",
  maskAccounts = true,
  showAllAccounts = false,
  showAllModels = false,
  scale = 2
) {
  const layout = getShareLayout(data, mode, showAllAccounts, showAllModels)
  const canvas = document.createElement("canvas")
  canvas.width = SHARE_WIDTH * scale
  canvas.height = layout.height * scale
  const context = canvas.getContext("2d")
  if (!context) throw new Error("当前环境不支持图片导出。")
  context.scale(scale, scale)
  context.fillStyle = "#f7f7f8"
  context.fillRect(0, 0, SHARE_WIDTH, layout.height)
  roundedRect(context, 24, 24, 1032, layout.height - 48, 24)
  context.fillStyle = "#ffffff"
  context.fill()
  context.strokeStyle = "#dedee3"
  context.lineWidth = 1
  context.stroke()

  // SVG 文本默认使用 alphabetic baseline；PNG 也使用同一基线和坐标，
  // 确保预览、复制和保存后的图片排版一致。
  context.textBaseline = "alphabetic"
  roundedRect(context, 72, 60, 34, 34, 9)
  context.fillStyle = "#18181b"
  context.fill()
  context.fillStyle = "#ffffff"
  context.font = canvasFont(500, 15)
  context.textAlign = "center"
  context.fillText("CT", 89, 83)
  context.fillStyle = "#84cc16"
  context.fillText("T", 94, 83)
  context.textAlign = "left"
  context.fillStyle = "#18181b"
  context.font = canvasFont(500, 32)
  context.fillText("Codex Tools · 今日用量", 122, 68)
  context.fillStyle = "#71717a"
  context.font = canvasFont(400, 16)
  context.fillText(`${data.dateLabel} · ${data.timezone}`, 122, 96)
  context.textAlign = "right"
  context.fillText("当前统计周期", 1008, 72)
  context.fillText("软件更新后产生的新用量", 1008, 96)
  context.textAlign = "left"

  const metricLabels = ["总 Token", "估算费用", "调用次数"]
  const metricValues = [
    shareTokens(data.totalTokens),
    layout.costLabel,
    data.requests.toLocaleString("en-US"),
  ]
  for (let index = 0; index < 3; index += 1) {
    const x = 72 + index * 318
    roundedRect(context, x, 145, 300, 74, 10)
    context.fillStyle = "#f7f7f8"
    context.fill()
    context.strokeStyle = "#ececf0"
    context.stroke()
    context.fillStyle = "#71717a"
    context.font = canvasFont(400, 16)
    context.fillText(metricLabels[index] ?? "", x + 18, 168)
    context.fillStyle = "#18181b"
    context.font = canvasFont(500, 28)
    context.fillText(metricValues[index] ?? "", x + 18, 192)
  }

  if (mode === "details") {
    context.beginPath()
    context.moveTo(72, 272)
    context.lineTo(1008, 272)
    context.strokeStyle = "#ececf0"
    context.stroke()
    context.fillStyle = "#18181b"
    context.font = canvasFont(500, 20)
    context.fillText("账号与模型明细", 72, SHARE_HEADER_HEIGHT + 32)
    context.fillStyle = "#71717a"
    context.font = canvasFont(400, 16)
    context.textAlign = "right"
    context.fillText(
      `${layout.visibleAccounts.length} 个账号`,
      1008,
      SHARE_HEADER_HEIGHT + 32
    )
    context.textAlign = "left"

    for (const { account, models, hiddenModels, y } of layout.accountLayouts) {
      const maxTokens = Math.max(1, ...models.map((model) => model.totalTokens))
      const name = maskAccounts ? account.maskedName : account.displayName
      context.beginPath()
      context.moveTo(72, y)
      context.lineTo(1008, y)
      context.strokeStyle = "#dedee3"
      context.stroke()
      context.fillStyle = "#18181b"
      context.font = canvasFont(500, 20)
      context.fillText(name, 72, y + 19)
      const badgeX = Math.min(350, 92 + name.length * 13)
      roundedRect(context, badgeX, y + 7, 108, 24, 12)
      context.fillStyle = "#fafafa"
      context.fill()
      context.strokeStyle = "#dedee3"
      context.lineWidth = 1
      context.stroke()
      context.fillStyle = "#52525b"
      context.font = canvasFont(400, 14)
      context.textAlign = "center"
      context.fillText(sourceLabel(account.sourceKind), badgeX + 54, y + 23)
      context.textAlign = "left"
      context.fillStyle = "#18181b"
      context.font = canvasFont(500, 18)
      context.textAlign = "right"
      context.fillText(
        `${shareTokens(account.totalTokens)} · ${account.requests} 次`,
        1008,
        y + 19
      )
      context.fillStyle = "#71717a"
      context.font = canvasFont(400, 16)
      context.fillText(accountCost(account), 1008, y + 43)
      context.textAlign = "left"

      models.forEach((model, index) => {
        const modelY =
          y + SHARE_ACCOUNT_HEADER_HEIGHT + index * SHARE_MODEL_ROW_HEIGHT
        const barWidth = Math.max(
          12,
          Math.round((model.totalTokens / maxTokens) * 400)
        )
        context.fillStyle = "#18181b"
        context.font = canvasFont(500, 17)
        context.fillText(model.model, 106, modelY + 17)
        context.fillStyle = "#71717a"
        context.font = canvasFont(400, 16)
        context.fillText(
          `${model.requests} 次调用 · 账号内 ${Math.round((model.totalTokens / Math.max(1, account.totalTokens)) * 100)}%`,
          106,
          modelY + 42
        )
        roundedRect(context, 420, modelY + 14, 400, 10, 5)
        context.fillStyle = "#e4e4e7"
        context.fill()
        roundedRect(context, 420, modelY + 14, barWidth, 10, 5)
        context.fillStyle = "#18181b"
        context.fill()
        context.fillStyle = "#18181b"
        context.font = canvasFont(500, 17)
        context.textAlign = "right"
        context.fillText(shareTokens(model.totalTokens), 1008, modelY + 17)
        context.fillStyle = "#71717a"
        context.font = canvasFont(400, 16)
        context.fillText(modelCost(model), 1008, modelY + 42)
        context.textAlign = "left"
      })
      if (hiddenModels > 0) {
        context.fillStyle = "#71717a"
        context.font = canvasFont(400, 16)
        context.fillText(
          `另外 ${hiddenModels} 个模型`,
          106,
          y +
            SHARE_ACCOUNT_HEADER_HEIGHT +
            models.length * SHARE_MODEL_ROW_HEIGHT +
            8
        )
      }
    }
    if (layout.hiddenAccounts > 0) {
      context.fillStyle = "#71717a"
      context.font = canvasFont(400, 16)
      context.fillText(
        `另外 ${layout.hiddenAccounts} 个账号`,
        72,
        layout.height - SHARE_FOOTER_HEIGHT - 21
      )
    }
  }

  context.fillStyle = "#71717a"
  context.font = canvasFont(400, 16)
  context.fillText(
    "Codex Tools · 本机统计 · 费用为估算值，不代表官方账单",
    72,
    layout.height - SHARE_FOOTER_HEIGHT + 48
  )

  return await new Promise<Blob>((resolve, reject) => {
    canvas.toBlob((result) => {
      if (result) resolve(result)
      else reject(new Error("PNG 图片生成失败。"))
    }, "image/png")
  })
}

export async function copySharePngToClipboard(
  data: UsageShareData,
  mode: UsageShareMode,
  maskAccounts: boolean,
  showAllAccounts: boolean,
  showAllModels: boolean
) {
  if (!navigator.clipboard || typeof ClipboardItem === "undefined") {
    throw new Error("当前环境不支持复制图片，请改用“保存 PNG”。")
  }
  try {
    await navigator.clipboard.write([
      new ClipboardItem({
        "image/png": renderSharePng(
          data,
          mode,
          maskAccounts,
          showAllAccounts,
          showAllModels
        ),
      }),
    ])
  } catch (reason) {
    const blob = await renderSharePng(
      data,
      mode,
      maskAccounts,
      showAllAccounts,
      showAllModels
    )
    try {
      await navigator.clipboard.write([
        new ClipboardItem({ "image/png": blob }),
      ])
    } catch {
      throw reason
    }
  }
}

export function downloadShareFile(
  filename: string,
  content: Blob | string,
  mimeType: string
) {
  const blob =
    typeof content === "string"
      ? new Blob([content], { type: mimeType })
      : content
  const url = URL.createObjectURL(blob)
  const anchor = document.createElement("a")
  anchor.href = url
  anchor.download = filename
  anchor.style.display = "none"
  document.body.appendChild(anchor)
  anchor.click()
  anchor.remove()
  window.setTimeout(() => URL.revokeObjectURL(url), 1000)
}
