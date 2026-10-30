import * as React from 'react';
import { createRoot } from 'react-dom/client';
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

export function mountConfirmDialog({ container }) {
  const host = container.ownerDocument.createElement('div');
  host.dataset.neroUiOwned = '';
  container.append(host);
  const root = createRoot(host);
  let request = null, settle = null, origin = null;
  const render = () => root.render(<ConfirmDialog request={request} container={container} resolve={resolve} finalFocus={() => origin?.isConnected ? origin : false} />);
  function resolve(value) { const finish = settle; settle = null; request = null; render(); finish?.(value); }
  const cancel = () => resolve(false);
  container.addEventListener('close', cancel);
  return {
    confirm(value) {
      if (settle) return Promise.resolve(false);
      origin = container.ownerDocument.activeElement;
      request = value;
      const result = new Promise(done => { settle = done; });
      render(); return result;
    },
    dispose() { container.removeEventListener('close', cancel); const finish = settle; settle = null; finish?.(false); root.unmount(); host.remove(); },
  };
}
