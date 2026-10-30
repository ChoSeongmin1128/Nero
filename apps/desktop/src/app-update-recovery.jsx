import * as React from 'react';
import { createRoot } from 'react-dom/client';
import { ConfirmDialog } from '../../../design-system/components/ui/confirm-dialog.jsx';
import './app-update-recovery.css';

function AppUpdateRecovery({ status, container, quit, copy }) {
  const [busy, setBusy] = React.useState(false);
  const [feedback, setFeedback] = React.useState('');
  const [error, setError] = React.useState('');
  const finish = async () => {
    if (busy) return;
    setBusy(true); setError('');
    try { if (await quit() === false) { setError('앱을 종료하지 못했습니다. 다시 시도해 주세요.'); setBusy(false); } }
    catch (failure) { setError(failure?.message || String(failure)); setBusy(false); }
  };
  return <ConfirmDialog request={status ? { title: '앱 업데이트 복구가 필요합니다', description: status.recoveryMessage || status.message, action: busy ? '종료 중' : '앱 종료' } : null}
    container={container} resolve={accepted => { if (accepted) void finish(); }} cancelLabel={null} disabled={busy} finalFocus={false} className="app-update-recovery">
    <div className="app-update-recovery-body">
      <p>앱을 종료한 뒤 보관된 이전 앱의 위치를 확인해 주세요.</p>
      {status?.previousAppPath ? <><code>{status.previousAppPath}</code><button type="button" className="small" disabled={busy} onClick={async () => {
        try { await copy(status.previousAppPath); setFeedback('이전 앱 위치를 복사했습니다.'); setError(''); }
        catch (failure) { setError(failure?.message || '위치를 복사하지 못했습니다.'); }
      }}>이전 앱 위치 복사</button></> : <p>보관된 이전 앱 위치를 확인하지 못했습니다.</p>}
      {status?.recoveryReceiptSaved === false && <p className="notice">복구 정보를 저장하지 못했습니다. 종료 전에 이전 앱 위치를 별도로 보관해 주세요.</p>}
      {feedback && <p className="inline-feedback" role="status">{feedback}</p>}
      {error && <p className="notice" role="alert">{error}</p>}
    </div>
  </ConfirmDialog>;
}

export function mountAppUpdateRecovery({ document, quit, copy = text => document.defaultView.navigator.clipboard.writeText(text) }) {
  const host = document.createElement('dialog');
  host.dataset.appUpdateRecovery = '';
  host.className = 'app-update-recovery-host';
  host.setAttribute('aria-label', '앱 업데이트 복구');
  host.addEventListener('cancel', event => event.preventDefault());
  document.body.append(host);
  const root = createRoot(host);
  return {
    update(status) {
      if (status && !host.open) host.showModal();
      root.render(<AppUpdateRecovery status={status} container={host} quit={quit} copy={copy} />);
      if (!status && host.open) host.close();
    },
    destroy() { root.unmount(); if (host.open) host.close(); host.remove(); },
  };
}
