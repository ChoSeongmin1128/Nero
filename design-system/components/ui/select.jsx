import * as React from 'react';
import { Select as Primitive } from '@base-ui/react/select';
import { Check, ChevronDown, ChevronUp } from 'lucide-react';
import { cn } from './utils.js';

export const Select = Primitive.Root;
export const SelectGroup = Primitive.Group;
export const SelectLabel = Primitive.GroupLabel;
export const SelectSeparator = Primitive.Separator;

export function SelectValue({ className, ...props }) {
  return <Primitive.Value data-slot="select-value" className={cn('nero-select-value', className)} {...props} />;
}

export function SelectTrigger({ className, children, ...props }) {
  return <Primitive.Trigger data-slot="select-trigger" className={cn('nero-select-trigger', className)} {...props}>
    {children}<Primitive.Icon><ChevronDown size={14} aria-hidden="true" /></Primitive.Icon>
  </Primitive.Trigger>;
}

export function SelectContent({ className, children, container, side = 'bottom', sideOffset = 5, align = 'start', alignItemWithTrigger = false, ...props }) {
  return <Primitive.Portal container={container}>
    <Primitive.Positioner side={side} sideOffset={sideOffset} align={align} alignItemWithTrigger={alignItemWithTrigger} collisionPadding={8} className="nero-control-positioner">
      <Primitive.Popup data-slot="select-content" className={cn('nero-select-popover', className)} {...props}>
        <Primitive.ScrollUpArrow className="nero-select-scroll"><ChevronUp size={14} /></Primitive.ScrollUpArrow>
        <Primitive.List>{children}</Primitive.List>
        <Primitive.ScrollDownArrow className="nero-select-scroll"><ChevronDown size={14} /></Primitive.ScrollDownArrow>
      </Primitive.Popup>
    </Primitive.Positioner>
  </Primitive.Portal>;
}

export function SelectItem({ className, children, ...props }) {
  return <Primitive.Item data-slot="select-item" className={cn('nero-select-option', className)} {...props}>
    <Primitive.ItemText>{children}</Primitive.ItemText>
    <Primitive.ItemIndicator className="nero-select-indicator"><Check size={14} aria-hidden="true" /></Primitive.ItemIndicator>
  </Primitive.Item>;
}
