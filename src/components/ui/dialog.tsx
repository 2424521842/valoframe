import { X } from "@phosphor-icons/react";
import { Dialog as DialogPrimitive } from "radix-ui";
import type { ComponentProps } from "react";
import { cn } from "../../lib/classNames";

export const UiDialog = DialogPrimitive.Root;
export const UiDialogTrigger = DialogPrimitive.Trigger;

export function UiDialogContent({ className, children, ...props }: ComponentProps<typeof DialogPrimitive.Content>) {
  return (
    <DialogPrimitive.Portal>
      <DialogPrimitive.Overlay className="ui-dialog-overlay" />
      <DialogPrimitive.Content className={cn("ui-dialog-content", className)} {...props}>
        {children}
        <DialogPrimitive.Close aria-label="关闭" className="ui-dialog-close">
          <X weight="bold" />
        </DialogPrimitive.Close>
      </DialogPrimitive.Content>
    </DialogPrimitive.Portal>
  );
}

export function UiDialogTitle({ className, ...props }: ComponentProps<typeof DialogPrimitive.Title>) {
  return <DialogPrimitive.Title className={cn("ui-dialog-title", className)} {...props} />;
}

export function UiDialogDescription({ className, ...props }: ComponentProps<typeof DialogPrimitive.Description>) {
  return <DialogPrimitive.Description className={cn("ui-dialog-description", className)} {...props} />;
}

export function UiDialogClose({ className, ...props }: ComponentProps<typeof DialogPrimitive.Close>) {
  return <DialogPrimitive.Close className={cn("ui-dialog-button", className)} {...props} />;
}
