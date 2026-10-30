import * as React from 'react';
import { AlertDialog } from '@base-ui/react/alert-dialog';
import { cn } from './utils.js';

export function ConfirmDialog({ request, container, resolve, finalFocus, className, children, cancelLabel = '취소', disabled = false }) {
  const cancel = React.useRef(null);
  const action = React.useRef(null);
  return <AlertDialog.Root open={Boolean(request)} onOpenChange={open => { if (!open && cancelLabel !== null && !disabled) resolve(false); }}>
    <AlertDialog.Portal container={container}>
      <AlertDialog.Backdrop className="nero-dialog-overlay" />
      <AlertDialog.Popup className={cn('nero-dialog-content nero-confirm-dialog', className)} initialFocus={cancelLabel === null ? action : cancel} finalFocus={finalFocus} data-nero-ui-owned="">
        <div className="nero-dialog-header"><AlertDialog.Title className="nero-dialog-title">{request?.title}</AlertDialog.Title><AlertDialog.Description className="nero-dialog-description">{request?.description}</AlertDialog.Description></div>
        {children}
        <div className="nero-dialog-footer">{cancelLabel !== null && <button ref={cancel} type="button" disabled={disabled} onClick={() => resolve(false)}>{cancelLabel}</button>}<button ref={action} type="button" disabled={disabled} className={request?.destructive ? 'danger' : 'primary'} onClick={() => resolve(true)}>{request?.action}</button></div>
      </AlertDialog.Popup>
    </AlertDialog.Portal>
  </AlertDialog.Root>;
}
