// Shared visual and motion language for every portalled surface.
// Keep these classes centralized so dialogs, sheets, menus, selects, and
// tooltips cannot quietly drift apart again.
export const overlayBackdropStyles =
  "fixed inset-0 isolate z-50 bg-[var(--overlay-backdrop)] supports-backdrop-filter:backdrop-blur-[var(--overlay-blur)] transition-[opacity,backdrop-filter] duration-200 ease-[var(--motion-ease-out)] motion-reduce:transition-none data-starting-style:opacity-0 data-starting-style:backdrop-blur-none data-ending-style:opacity-0 data-ending-style:backdrop-blur-none"

export const hiddenOverlayStyles =
  "pointer-events-none opacity-0 backdrop-blur-none"

export const overlaySurfaceStyles =
  "bg-popover text-popover-foreground shadow-2xl ring-1 ring-foreground/10"

export const modalMotionStyles =
  "transition-[opacity,scale] duration-200 ease-[var(--motion-ease-out)] motion-reduce:transition-none data-starting-style:scale-[0.98] data-starting-style:opacity-0 data-ending-style:scale-[0.98] data-ending-style:opacity-0"

export const sheetMotionStyles =
  "transition-[opacity,translate] duration-200 ease-[var(--motion-ease-out)] motion-reduce:transition-none data-starting-style:opacity-0 data-ending-style:opacity-0 data-[side=bottom]:data-starting-style:translate-y-[calc(100%+0.5rem)] data-[side=bottom]:data-ending-style:translate-y-[calc(100%+0.5rem)] data-[side=left]:data-starting-style:-translate-x-[calc(100%+0.5rem)] data-[side=left]:data-ending-style:-translate-x-[calc(100%+0.5rem)] data-[side=right]:data-starting-style:translate-x-[calc(100%+0.5rem)] data-[side=right]:data-ending-style:translate-x-[calc(100%+0.5rem)] data-[side=top]:data-starting-style:-translate-y-[calc(100%+0.5rem)] data-[side=top]:data-ending-style:-translate-y-[calc(100%+0.5rem)]"

export const floatingSurfaceStyles =
  "bg-popover text-popover-foreground shadow-xl ring-1 ring-foreground/10"

export const floatingMotionStyles =
  "transition-[opacity,scale,translate] duration-150 ease-[var(--motion-ease-out)] motion-reduce:transition-none data-starting-style:scale-[0.97] data-starting-style:opacity-0 data-ending-style:scale-[0.97] data-ending-style:opacity-0 data-[side=bottom]:data-starting-style:-translate-y-1 data-[side=bottom]:data-ending-style:-translate-y-1 data-[side=inline-end]:data-starting-style:-translate-x-1 data-[side=inline-end]:data-ending-style:-translate-x-1 data-[side=inline-start]:data-starting-style:translate-x-1 data-[side=inline-start]:data-ending-style:translate-x-1 data-[side=left]:data-starting-style:translate-x-1 data-[side=left]:data-ending-style:translate-x-1 data-[side=right]:data-starting-style:-translate-x-1 data-[side=right]:data-ending-style:-translate-x-1 data-[side=top]:data-starting-style:translate-y-1 data-[side=top]:data-ending-style:translate-y-1"
