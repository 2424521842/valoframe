import { Checkbox as CheckboxPrimitive } from "radix-ui";
import type { ComponentProps } from "react";
import { cn } from "../../lib/classNames";

export function UiCheckbox({ className, ...props }: ComponentProps<typeof CheckboxPrimitive.Root>) {
  return (
    <CheckboxPrimitive.Root className={cn("ui-checkbox", className)} {...props}>
      <CheckboxPrimitive.Indicator className="ui-checkbox-indicator" />
    </CheckboxPrimitive.Root>
  );
}
