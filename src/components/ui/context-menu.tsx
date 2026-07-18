import { ContextMenu as ContextMenuPrimitive } from "radix-ui";
import type { ComponentProps } from "react";
import { cn } from "../../lib/classNames";

export const UiContextMenu = ContextMenuPrimitive.Root;
export const UiContextMenuTrigger = ContextMenuPrimitive.Trigger;

export function UiContextMenuContent({ className, ...props }: ComponentProps<typeof ContextMenuPrimitive.Content>) {
  return (
    <ContextMenuPrimitive.Portal>
      <ContextMenuPrimitive.Content
        className={cn("ui-context-menu-content", className)}
        collisionPadding={10}
        {...props}
      />
    </ContextMenuPrimitive.Portal>
  );
}

export function UiContextMenuLabel({ className, ...props }: ComponentProps<typeof ContextMenuPrimitive.Label>) {
  return <ContextMenuPrimitive.Label className={cn("ui-context-menu-label", className)} {...props} />;
}

export function UiContextMenuItem({ className, ...props }: ComponentProps<typeof ContextMenuPrimitive.Item>) {
  return <ContextMenuPrimitive.Item className={cn("ui-context-menu-item", className)} {...props} />;
}

export function UiContextMenuSeparator({ className, ...props }: ComponentProps<typeof ContextMenuPrimitive.Separator>) {
  return <ContextMenuPrimitive.Separator className={cn("ui-context-menu-separator", className)} {...props} />;
}

export function UiContextMenuShortcut({ className, ...props }: ComponentProps<"span">) {
  return <span className={cn("ui-context-menu-shortcut", className)} {...props} />;
}
