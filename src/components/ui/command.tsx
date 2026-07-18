import { Command as CommandPrimitive } from "cmdk";
import type { ComponentProps } from "react";
import { cn } from "../../lib/classNames";

export function UiCommand({ className, ...props }: ComponentProps<typeof CommandPrimitive>) {
  return <CommandPrimitive className={cn("ui-command", className)} {...props} />;
}

export function UiCommandInput({ className, ...props }: ComponentProps<typeof CommandPrimitive.Input>) {
  return <CommandPrimitive.Input className={cn("ui-command-input", className)} {...props} />;
}

export function UiCommandList({ className, ...props }: ComponentProps<typeof CommandPrimitive.List>) {
  return <CommandPrimitive.List className={cn("ui-command-list", className)} {...props} />;
}

export function UiCommandEmpty({ className, ...props }: ComponentProps<typeof CommandPrimitive.Empty>) {
  return <CommandPrimitive.Empty className={cn("ui-command-empty", className)} {...props} />;
}

export function UiCommandGroup({ className, ...props }: ComponentProps<typeof CommandPrimitive.Group>) {
  return <CommandPrimitive.Group className={cn("ui-command-group", className)} {...props} />;
}

export function UiCommandItem({ className, ...props }: ComponentProps<typeof CommandPrimitive.Item>) {
  return <CommandPrimitive.Item className={cn("ui-command-item", className)} {...props} />;
}

export function UiCommandSeparator({ className, ...props }: ComponentProps<typeof CommandPrimitive.Separator>) {
  return <CommandPrimitive.Separator className={cn("ui-command-separator", className)} {...props} />;
}
