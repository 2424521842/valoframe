import { Popover as PopoverPrimitive } from "radix-ui";
import type { ComponentProps } from "react";
import { cn } from "../../lib/classNames";

export const UiPopover = PopoverPrimitive.Root;
export const UiPopoverAnchor = PopoverPrimitive.Anchor;
export const UiPopoverTrigger = PopoverPrimitive.Trigger;

export function UiPopoverContent({ className, align = "start", sideOffset = 6, ...props }: ComponentProps<typeof PopoverPrimitive.Content>) {
  return (
    <PopoverPrimitive.Portal>
      <PopoverPrimitive.Content
        align={align}
        className={cn("ui-popover-content", className)}
        sideOffset={sideOffset}
        {...props}
      />
    </PopoverPrimitive.Portal>
  );
}
