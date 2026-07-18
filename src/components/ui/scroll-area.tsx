import { ScrollArea as ScrollAreaPrimitive } from "radix-ui";
import type { ComponentProps } from "react";
import { cn } from "../../lib/classNames";

export function UiScrollArea({ className, children, ...props }: ComponentProps<typeof ScrollAreaPrimitive.Root>) {
  return (
    <ScrollAreaPrimitive.Root className={cn("ui-scroll-area", className)} {...props}>
      <ScrollAreaPrimitive.Viewport className="ui-scroll-area-viewport">
        {children}
      </ScrollAreaPrimitive.Viewport>
      <ScrollAreaPrimitive.Scrollbar className="ui-scroll-area-scrollbar" orientation="vertical">
        <ScrollAreaPrimitive.Thumb className="ui-scroll-area-thumb" />
      </ScrollAreaPrimitive.Scrollbar>
      <ScrollAreaPrimitive.Corner />
    </ScrollAreaPrimitive.Root>
  );
}
