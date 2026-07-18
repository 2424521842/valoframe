import { Tooltip as TooltipPrimitive } from "radix-ui";
import type { ComponentProps } from "react";
import { cn } from "../../lib/classNames";

export const UiTooltipProvider = TooltipPrimitive.Provider;
export const UiTooltip = TooltipPrimitive.Root;
export const UiTooltipTrigger = TooltipPrimitive.Trigger;

export function UiTooltipContent({ className, sideOffset = 6, ...props }: ComponentProps<typeof TooltipPrimitive.Content>) {
  return (
    <TooltipPrimitive.Portal>
      <TooltipPrimitive.Content
        className={cn("ui-tooltip-content", className)}
        sideOffset={sideOffset}
        {...props}
      />
    </TooltipPrimitive.Portal>
  );
}
