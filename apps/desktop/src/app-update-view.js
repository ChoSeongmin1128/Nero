import { createAppUpdater, appUpdateAction } from './app-updater.js';

export function appUpdateNotices(status) {
  if (!status) return [];
  const ready = ['ready', 'waiting'].includes(status.phase) || status.phase === 'error' && status.downloadReady;
  if (!ready && !status.recoveryMessage) return [];
  const action = appUpdateAction(status);
  return [{ id: `app-update:${ready ? status.availableBuild || status.availableVersion || 'ready' : 'recovery'}`, service: 'app-update',
    title: 'Nero 업데이트', message: ready ? status.message : status.recoveryMessage,
    actionLabel: action.label, actionDisabled: action.disabled }];
}

export function mountAppUpdateView({ container, bridge, saveEdits, onChange = () => {}, onInstalling = () => {}, schedule, unschedule }) {
  const document = container.ownerDocument;
  const get = id => container.querySelector(`#app-update-${id}`);
  const methods = { app_update_status: bridge.appUpdateStatus, app_update_check: bridge.checkAppUpdate, app_update_download: bridge.downloadAppUpdate, app_update_install: bridge.installAppUpdate };
  let busy = false, installedLock = false;
  const inertState = new Map();
  function setInstalling(value) {
    if (installedLock === value) return;
    installedLock = value;
    if (value) {
      for (const node of document.querySelectorAll('.workspace, dialog, [role="dialog"], [role="alertdialog"]')) { inertState.set(node, node.inert); node.inert = true; }
    } else {
      for (const [node, inert] of inertState) if (node.isConnected) node.inert = inert;
      inertState.clear();
    }
    onInstalling(value);
  }
  function render(status) {
    const action = appUpdateAction(status);
    get('version').textContent = status.currentVersion ? `${status.currentVersion}${status.currentBuild ? ` (${status.currentBuild})` : ''}` : '';
    get('message').textContent = status.message || '업데이트를 확인할 수 있습니다.';
    get('action').textContent = action.label;
    get('action').disabled = busy || action.disabled;
    get('recovery').hidden = !status.recoveryMessage;
    get('recovery').textContent = status.recoveryMessage || '';
    const progress = get('progress');
    progress.hidden = status.phase !== 'downloading';
    if (status.totalBytes > 0) { progress.max = status.totalBytes; progress.value = Math.min(status.totalBytes, status.downloadedBytes || 0); }
    else progress.removeAttribute('value');
  }
  const updater = createAppUpdater({
    invoke: command => methods[command](), listen: (_, callback) => bridge.subscribeAppUpdate(payload => callback({ payload })),
    saveEdits, setInstalling, schedule, unschedule,
    onChange(status) { render(status); onChange(appUpdateNotices(status)); },
  });
  async function perform() {
    const action = appUpdateAction(updater.getSnapshot());
    if (busy || action.disabled || !action.action) return false;
    busy = true; render(updater.getSnapshot());
    try { return await updater[action.action](); }
    finally { busy = false; render(updater.getSnapshot()); }
  }
  const click = () => { void perform(); };
  get('action').addEventListener('click', click);
  render(updater.getSnapshot());
  return { start: () => updater.start(), perform, getSnapshot: updater.getSnapshot,
    destroy() { updater.dispose(); get('action').removeEventListener('click', click); } };
}
