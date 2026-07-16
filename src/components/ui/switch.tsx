import { Switch as SwitchPrimitive } from "@base-ui/react/switch"

import { cn } from "@/lib/utils"

function Switch({
  className,
  size = "default",
  ...props
}: SwitchPrimitive.Root.Props & {
  size?: "sm" | "default"
}) {
  return (
    <SwitchPrimitive.Root
      data-slot="switch"
      data-size={size}
      className={cn(
        "peer group/switch relative inline-flex shrink-0 items-center rounded-full border-2 transition-[background-color,border-color,box-shadow] outline-none after:absolute after:-inset-x-2 after:-inset-y-2 focus-visible:ring-3 focus-visible:ring-ring/35 aria-invalid:border-destructive aria-invalid:ring-3 aria-invalid:ring-destructive/25 data-[size=default]:h-8 data-[size=default]:w-[52px] data-[size=sm]:h-7 data-[size=sm]:w-11 data-checked:border-primary data-checked:bg-primary data-unchecked:border-input data-unchecked:bg-[var(--md-sys-color-surface-container-highest)] data-disabled:cursor-not-allowed data-disabled:opacity-[0.38]",
        className
      )}
      {...props}
    >
      <SwitchPrimitive.Thumb
        data-slot="switch-thumb"
        className="pointer-events-none block rounded-full ring-0 transition-[transform,width,height,background-color] duration-200 data-checked:translate-x-[22px] data-checked:bg-primary-foreground group-data-[size=default]/switch:data-checked:size-6 group-data-[size=sm]/switch:data-checked:size-5 data-unchecked:translate-x-1 data-unchecked:bg-[var(--md-sys-color-outline)] group-data-[size=default]/switch:data-unchecked:size-4 group-data-[size=sm]/switch:data-unchecked:size-3.5"
      />
    </SwitchPrimitive.Root>
  )
}

export { Switch }
