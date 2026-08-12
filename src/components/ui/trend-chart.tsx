import {
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type MouseEvent,
} from "react"

import { usageChartConfig, type TrendSeriesPoint } from "@/lib/chart"
import { formatInteger, formatTokens } from "@/lib/format"
import { cn } from "@/lib/utils"

const SERIES_KEYS = ["input", "output", "cache"] as const

// 图表内边距：左侧留给 Y 轴紧凑 Token 标签，底部留给日期标签。
const MARGIN = { top: 6, right: 10, bottom: 22, left: 52 } as const
const MAX_X_LABELS = 7
const Y_TICK_COUNT = 4

/** 把最大值向上取整为「漂亮数」（1/2/2.5/5/10 × 10^n），让 Y 轴刻度整齐。 */
function niceCeil(value: number) {
  if (value <= 0) return 1
  const exponent = Math.floor(Math.log10(value))
  const fraction = value / 10 ** exponent
  const niceFraction =
    fraction <= 1
      ? 1
      : fraction <= 2
        ? 2
        : fraction <= 2.5
          ? 2.5
          : fraction <= 5
            ? 5
            : 10
  return niceFraction * 10 ** exponent
}

function useElementSize() {
  const ref = useRef<HTMLDivElement>(null)
  const [size, setSize] = useState({ width: 0, height: 0 })

  useLayoutEffect(() => {
    const element = ref.current
    if (!element) return
    const observer = new ResizeObserver((entries) => {
      const entry = entries[0]
      if (!entry) return
      const { width, height } = entry.contentRect
      setSize({ width, height })
    })
    observer.observe(element)
    const rect = element.getBoundingClientRect()
    setSize({ width: rect.width, height: rect.height })
    return () => observer.disconnect()
  }, [])

  return [ref, size] as const
}

/**
 * 轻量的用量趋势折线图（输入/输出/缓存三条线）。
 * 替代 recharts：仅覆盖本应用用到的响应式折线图、网格、坐标轴、悬浮提示与图例，
 * 体积从约 350KB 降到几 KB，且仪表盘首屏不再加载重型图表依赖。
 */
export function TrendChart({
  points,
  showDots = false,
  className,
}: {
  points: TrendSeriesPoint[]
  showDots?: boolean
  className?: string
}) {
  const [plotRef, { width, height }] = useElementSize()
  const [hover, setHover] = useState<number>()

  const plotWidth = Math.max(0, width - MARGIN.left - MARGIN.right)
  const plotHeight = Math.max(0, height - MARGIN.top - MARGIN.bottom)

  const maxValue = useMemo(
    () =>
      points.reduce(
        (max, point) => Math.max(max, point.input, point.output, point.cache),
        0
      ),
    [points]
  )
  const yMax = niceCeil(maxValue)
  const yTicks = useMemo(
    () =>
      Array.from(
        { length: Y_TICK_COUNT + 1 },
        (_, index) => (yMax / Y_TICK_COUNT) * index
      ),
    [yMax]
  )

  const count = points.length
  const xFor = (index: number) =>
    count <= 1 ? plotWidth / 2 : (index / (count - 1)) * plotWidth
  const yFor = (value: number) => plotHeight * (1 - value / yMax)

  const labelStep = Math.max(1, Math.ceil(count / MAX_X_LABELS))
  const showLabel = (index: number) =>
    index % labelStep === 0 || index === count - 1

  const handleMove = (event: MouseEvent<SVGRectElement>) => {
    if (count === 0) return
    // 覆盖层 rect 的左边缘就是绘图区 x=0（其 bounding rect 已包含外层 <g> 的平移）。
    const rect = event.currentTarget.getBoundingClientRect()
    const x = event.clientX - rect.left
    const index = count <= 1 ? 0 : Math.round((x / plotWidth) * (count - 1))
    setHover(Math.min(count - 1, Math.max(0, index)))
  }

  const hoverPoint = hover === undefined ? undefined : points[hover]

  return (
    <div className={cn("flex min-h-0 flex-col", className)}>
      <div ref={plotRef} className="relative min-h-0 flex-1">
        {width > 0 && height > 0 && count > 0 && (
          <svg
            width={width}
            height={height}
            role="img"
            aria-label="Token 用量趋势"
            className="block select-none"
          >
            <g transform={`translate(${MARGIN.left},${MARGIN.top})`}>
              {yTicks.map((tick) => (
                <line
                  key={tick}
                  x1={0}
                  x2={plotWidth}
                  y1={yFor(tick)}
                  y2={yFor(tick)}
                  stroke="var(--border)"
                  strokeOpacity={0.5}
                  strokeDasharray="4 4"
                />
              ))}
              {yTicks.map((tick) => (
                <text
                  key={`label-${tick}`}
                  x={-8}
                  y={yFor(tick)}
                  textAnchor="end"
                  dominantBaseline="central"
                  fontSize={11}
                  fill="var(--muted-foreground)"
                >
                  {formatTokens(tick)}
                </text>
              ))}
              {points.map((point, index) =>
                showLabel(index) ? (
                  <text
                    key={`x-${index}`}
                    x={xFor(index)}
                    y={plotHeight + 16}
                    textAnchor="middle"
                    fontSize={11}
                    fill="var(--muted-foreground)"
                  >
                    {point.date}
                  </text>
                ) : null
              )}
              {hover !== undefined && (
                <line
                  x1={xFor(hover)}
                  x2={xFor(hover)}
                  y1={0}
                  y2={plotHeight}
                  stroke="var(--border)"
                />
              )}
              {SERIES_KEYS.map((key) => {
                const config = usageChartConfig[key]
                const path = points
                  .map(
                    (point, index) =>
                      `${index === 0 ? "M" : "L"}${xFor(index)},${yFor(point[key])}`
                  )
                  .join(" ")
                return (
                  <path
                    key={key}
                    d={path}
                    fill="none"
                    stroke={config.color}
                    strokeWidth={key === "cache" ? 1.75 : 2.5}
                    strokeDasharray={config.dashed ? "5 4" : undefined}
                    strokeLinejoin="round"
                    strokeLinecap="round"
                  />
                )
              })}
              {SERIES_KEYS.map((key) => {
                const config = usageChartConfig[key]
                return points.map((point, index) => {
                  const active = index === hover
                  if (!showDots && !active) return null
                  return (
                    <circle
                      key={`${key}-${index}`}
                      cx={xFor(index)}
                      cy={yFor(point[key])}
                      r={active ? 4 : key === "cache" ? 2.5 : 3}
                      fill={config.color}
                    />
                  )
                })
              })}
              <rect
                x={0}
                y={0}
                width={plotWidth}
                height={plotHeight}
                fill="transparent"
                onMouseMove={handleMove}
                onMouseLeave={() => setHover(undefined)}
              />
            </g>
          </svg>
        )}
        {hoverPoint && hover !== undefined && (
          <div
            className="pointer-events-none absolute z-10 min-w-32 rounded-2xl border border-border/50 bg-popover px-2.5 py-1.5 text-xs shadow-md"
            style={{
              left: Math.min(
                Math.max(MARGIN.left + xFor(hover), 72),
                Math.max(72, width - 72)
              ),
              top: 0,
              transform: "translate(-50%, 0)",
            }}
          >
            <div className="font-medium">{hoverPoint.date}</div>
            <div className="mt-1.5 grid gap-1.5">
              {SERIES_KEYS.map((key) => {
                const config = usageChartConfig[key]
                return (
                  <div key={key} className="flex items-center gap-2">
                    <span
                      className={cn(
                        "shrink-0 rounded-[2px] border",
                        config.dashed
                          ? "h-0 w-2.5 border-t-[1.5px] border-dashed"
                          : "h-2.5 w-2.5"
                      )}
                      style={{
                        backgroundColor: config.dashed
                          ? "transparent"
                          : config.color,
                        borderColor: config.color,
                      }}
                    />
                    <span className="text-muted-foreground">
                      {config.label}
                    </span>
                    <span className="ml-auto font-mono font-medium tabular-nums">
                      {formatInteger(hoverPoint[key])}
                    </span>
                  </div>
                )
              })}
            </div>
          </div>
        )}
      </div>
      <div className="flex items-center justify-center gap-3 pt-3 text-xs">
        {SERIES_KEYS.map((key) => {
          const config = usageChartConfig[key]
          return (
            <div key={key} className="flex items-center gap-1.5">
              <span
                className={cn(
                  "shrink-0 rounded-[2px]",
                  config.dashed
                    ? "h-0 w-2 border-t-[1.5px] border-dashed"
                    : "h-2 w-2"
                )}
                style={{
                  backgroundColor: config.dashed ? "transparent" : config.color,
                  borderColor: config.color,
                }}
              />
              <span className="text-muted-foreground">{config.label}</span>
            </div>
          )
        })}
      </div>
    </div>
  )
}
