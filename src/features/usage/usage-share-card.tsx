type UsageShareCardProps = {
  svg: string
}

export function UsageShareCard({ svg }: UsageShareCardProps) {
  return (
    <div
      className="max-h-[min(62dvh,760px)] overflow-auto rounded-xl border bg-muted/30 p-3 shadow-inner"
      aria-label="分享卡片预览"
    >
      <div
        className="mx-auto w-full max-w-[540px] overflow-hidden rounded-lg shadow-lg [&>svg]:block [&>svg]:h-auto [&>svg]:w-full"
        // The SVG renderer escapes all user-provided labels before creating markup.
        dangerouslySetInnerHTML={{ __html: svg }}
      />
    </div>
  )
}
