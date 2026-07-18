import { AlertDialog as AlertDialogPrimitive } from "radix-ui";
import type { ComponentProps } from "react";
import { cn } from "../../lib/classNames";

export const UiAlertDialog = AlertDialogPrimitive.Root;
export const UiAlertDialogTrigger = AlertDialogPrimitive.Trigger;

export function UiAlertDialogContent({ className, ...props }: ComponentProps<typeof AlertDialogPrimitive.Content>) {
  return (
    <AlertDialogPrimitive.Portal>
      <AlertDialogPrimitive.Overlay className="ui-dialog-overlay" />
      <AlertDialogPrimitive.Content className={cn("ui-alert-dialog-content", className)} {...props} />
    </AlertDialogPrimitive.Portal>
  );
}

export function UiAlertDialogTitle({ className, ...props }: ComponentProps<typeof AlertDialogPrimitive.Title>) {
  return <AlertDialogPrimitive.Title className={cn("ui-dialog-title", className)} {...props} />;
}

export function UiAlertDialogDescription({ className, ...props }: ComponentProps<typeof AlertDialogPrimitive.Description>) {
  return <AlertDialogPrimitive.Description className={cn("ui-dialog-description", className)} {...props} />;
}

export function UiAlertDialogCancel({ className, ...props }: ComponentProps<typeof AlertDialogPrimitive.Cancel>) {
  return <AlertDialogPrimitive.Cancel className={cn("ui-dialog-button", className)} {...props} />;
}

export function UiAlertDialogAction({ className, ...props }: ComponentProps<typeof AlertDialogPrimitive.Action>) {
  return <AlertDialogPrimitive.Action className={cn("ui-dialog-button ui-dialog-button--danger", className)} {...props} />;
}
