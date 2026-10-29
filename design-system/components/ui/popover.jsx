import * as React from 'react';
import { Popover as Primitive } from '@base-ui/react/popover';
import { cn } from './utils.js';

export const Popover = Primitive.Root;
export const PopoverTrigger = Primitive.Trigger;
export const PopoverClose = Primitive.Close;
export const PopoverTitle = Primitive.Title;
export const PopoverDescription = Primitive.Description;

export function PopoverContent({ className, container, align = 'start', side = 'bottom', sideOffset = 5, ...props }) {
  return <Primitive.Portal container={container}>
    <Primitive.Positioner align={align} side={side} sideOffset={sideOffset} collisionPadding={8} className="nero-control-positioner">
      <Primitive.Popup data-slot="popover-content" className={cn('nero-popover', className)} {...props} />
    </Primitive.Positioner>
  </Primitive.Portal>;
}
