import * as React from 'react';
import { AlertDialog } from '@base-ui/react/alert-dialog';
import { cn } from './utils.js';

export function ConfirmDialog({ request, container, resolve, finalFocus, className }) {
  const cancel = React.useRef(null);
  return <AlertDialog.Root open={Boolean(request)} onOpenChange={open => { if (!open) resolve(false); }}>
    <AlertDialog.Portal container={container}>
      <AlertDialog.Backdrop className="nero-dialog-overlay" />
      <AlertDialog.Popup className={cn('nero-dialog-content nero-confirm-dialog', className)} initialFocus={cancel} finalFocus={finalFocus} data-nero-ui-owned="">
        <div className="nero-dialog-header"><AlertDialog.Title className="nero-dialog-title">{request?.title}</AlertDialog.Title><AlertDialog.Description className="nero-dialog-description">{request?.description}</AlertDialog.Description></div>
        <div className="nero-dialog-footer"><button ref={cancel} type="button" onClick={() => resolve(false)}>취소</button><button type="button" className={request?.destructive ? 'danger' : 'primary'} onClick={() => resolve(true)}>{request?.action}</button></div>
      </AlertDialog.Popup>
    </AlertDialog.Portal>
  </AlertDialog.Root>;
}
