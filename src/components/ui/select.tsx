import { CaretDown, CaretUp, Check } from "@phosphor-icons/react";
import { Select as SelectPrimitive } from "radix-ui";
import type { ComponentProps } from "react";
import { cn } from "../../lib/classNames";

export const UiSelect = SelectPrimitive.Root;
export const UiSelectGroup = SelectPrimitive.Group;
export const UiSelectValue = SelectPrimitive.Value;

export function UiSelectTrigger({ className, children, ...props }: ComponentProps<typeof SelectPrimitive.Trigger>) {
  return (
    <SelectPrimitive.Trigger className={cn("ui-select-trigger", className)} {...props}>
      {children}
      <SelectPrimitive.Icon className="ui-select-icon">
        <CaretDown weight="bold" />
      </SelectPrimitive.Icon>
    </SelectPrimitive.Trigger>
  );
}

export function UiSelectContent({
  className,
  children,
  position = "popper",
  ...props
}: ComponentProps<typeof SelectPrimitive.Content>) {
  return (
    <SelectPrimitive.Portal>
      <SelectPrimitive.Content
        className={cn("ui-select-content", className)}
        position={position}
        sideOffset={6}
        {...props}
      >
        <SelectPrimitive.ScrollUpButton className="ui-select-scroll-button">
          <CaretUp weight="bold" />
        </SelectPrimitive.ScrollUpButton>
        <SelectPrimitive.Viewport className="ui-select-viewport">
          {children}
        </SelectPrimitive.Viewport>
        <SelectPrimitive.ScrollDownButton className="ui-select-scroll-button">
          <CaretDown weight="bold" />
        </SelectPrimitive.ScrollDownButton>
      </SelectPrimitive.Content>
    </SelectPrimitive.Portal>
  );
}

export function UiSelectLabel({ className, ...props }: ComponentProps<typeof SelectPrimitive.Label>) {
  return <SelectPrimitive.Label className={cn("ui-select-label", className)} {...props} />;
}

export function UiSelectItem({ className, children, ...props }: ComponentProps<typeof SelectPrimitive.Item>) {
  return (
    <SelectPrimitive.Item className={cn("ui-select-item", className)} {...props}>
      <SelectPrimitive.ItemText>{children}</SelectPrimitive.ItemText>
      <SelectPrimitive.ItemIndicator className="ui-select-item-indicator">
        <Check weight="bold" />
      </SelectPrimitive.ItemIndicator>
    </SelectPrimitive.Item>
  );
}

export function UiSelectSeparator({ className, ...props }: ComponentProps<typeof SelectPrimitive.Separator>) {
  return <SelectPrimitive.Separator className={cn("ui-select-separator", className)} {...props} />;
}
