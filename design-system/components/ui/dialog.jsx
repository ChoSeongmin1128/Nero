import * as React from 'react';
import { Dialog as Primitive } from '@base-ui/react/dialog';
import { X } from 'lucide-react';
import { cn } from './utils.js';

export const Dialog = Primitive.Root;
export const DialogTrigger = Primitive.Trigger;
export const DialogClose = Primitive.Close;
export const DialogPortal = Primitive.Portal;

export function DialogOverlay({ className, ...props }) {
  return <Primitive.Backdrop data-slot="dialog-overlay" className={cn('nero-dialog-overlay', className)} {...props} />;
}

export function DialogContent({ className, children, container, showCloseButton = true, ...props }) {
  return <DialogPortal container={container}>
    <DialogOverlay />
    <Primitive.Popup data-slot="dialog-content" className={cn('nero-dialog-content', className)} {...props}>
      {children}
      {showCloseButton && <DialogClose className="nero-dialog-close" aria-label="닫기"><X size={18} aria-hidden="true" /></DialogClose>}
    </Primitive.Popup>
  </DialogPortal>;
}

export function DialogTitle({ className, ...props }) {
  return <Primitive.Title data-slot="dialog-title" className={cn('nero-dialog-title', className)} {...props} />;
}

export function DialogDescription({ className, ...props }) {
  return <Primitive.Description data-slot="dialog-description" className={cn('nero-dialog-description', className)} {...props} />;
}

export function DialogHeader({ className, ...props }) {
  return <div data-slot="dialog-header" className={cn('nero-dialog-header', className)} {...props} />;
}

export function DialogFooter({ className, ...props }) {
  return <div data-slot="dialog-footer" className={cn('nero-dialog-footer', className)} {...props} />;
}
