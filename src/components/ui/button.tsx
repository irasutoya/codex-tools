import { Button as ButtonPrimitive } from "@base-ui/react/button"
import { cva, type VariantProps } from "class-variance-authority"

import { cn } from "@/lib/utils"

const buttonVariants = cva(
  "group/button relative inline-flex shrink-0 items-center justify-center rounded-full border border-transparent bg-clip-padding text-sm font-medium tracking-[0.00625em] whitespace-nowrap transition-[background-color,color,border-color,box-shadow,transform] duration-100 ease-out outline-none select-none focus-visible:ring-3 focus-visible:ring-ring/35 active:not-aria-[haspopup]:scale-[0.98] disabled:pointer-events-none disabled:opacity-[0.38] aria-invalid:border-destructive aria-invalid:ring-3 aria-invalid:ring-destructive/25 [&_svg]:pointer-events-none [&_svg]:shrink-0 [&_svg:not([class*='size-'])]:size-[18px]",
  {
    variants: {
      variant: {
        default:
          "bg-primary text-primary-foreground shadow-sm hover:bg-[color-mix(in_srgb,var(--md-sys-color-primary),var(--md-sys-color-on-primary)_8%)] hover:shadow-md",
        outline:
          "border-input bg-transparent text-primary hover:bg-[color-mix(in_srgb,var(--md-sys-color-primary)_8%,transparent)] aria-expanded:bg-secondary aria-expanded:text-secondary-foreground",
        secondary:
          "bg-secondary text-secondary-foreground hover:bg-[color-mix(in_srgb,var(--md-sys-color-secondary-container),var(--md-sys-color-on-secondary-container)_8%)] aria-expanded:bg-secondary aria-expanded:text-secondary-foreground",
        ghost:
          "text-primary hover:bg-[color-mix(in_srgb,var(--md-sys-color-primary)_8%,transparent)] aria-expanded:bg-secondary aria-expanded:text-secondary-foreground",
        destructive:
          "bg-destructive text-[var(--md-sys-color-on-error)] shadow-sm hover:bg-[color-mix(in_srgb,var(--md-sys-color-error),var(--md-sys-color-on-error)_8%)] focus-visible:ring-destructive/30",
        link: "text-primary underline-offset-4 hover:underline",
      },
      size: {
        default:
          "h-10 gap-2 px-6 has-data-[icon=inline-end]:pr-5 has-data-[icon=inline-start]:pl-5",
        xs: "h-8 gap-1.5 px-3 text-xs [&_svg:not([class*='size-'])]:size-4",
        sm: "h-9 gap-1.5 px-4 text-[0.8125rem] [&_svg:not([class*='size-'])]:size-4",
        lg: "h-12 gap-2 px-7",
        icon: "size-10 p-0",
        "icon-xs": "size-8 p-0 [&_svg:not([class*='size-'])]:size-4",
        "icon-sm": "size-10 p-0 [&_svg:not([class*='size-'])]:size-[18px]",
        "icon-lg": "size-12 p-0 [&_svg:not([class*='size-'])]:size-6",
      },
    },
    defaultVariants: {
      variant: "default",
      size: "default",
    },
  }
)

function Button({
  className,
  variant = "default",
  size = "default",
  ...props
}: ButtonPrimitive.Props & VariantProps<typeof buttonVariants>) {
  return (
    <ButtonPrimitive
      data-slot="button"
      data-variant={variant}
      data-size={size}
      className={cn(buttonVariants({ variant, size, className }))}
      {...props}
    />
  )
}

export { Button, buttonVariants }
