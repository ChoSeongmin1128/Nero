import { createAppUpdater, appUpdateAction } from './app-updater.js';

export function appUpdateNotices(status) {
  if (!status || status.enabled === false) return [];
  if (!['waiting', 'error', 'recoveryRequired'].includes(status.phase) && !status.recoveryMessage) return [];
  if (!status.message && !status.recoveryMessage) return [];
  const action = appUpdateAction(status);
  return [{ id: `app-update:${status.recoveryMessage || status.recoveryRequired ? 'recovery' : status.availableBuild || status.availableVersion || 'error'}`, service: 'app-update',
    title: 'Nero 업데이트', message: status.recoveryMessage || status.message,
    actionLabel: action.label, actionDisabled: action.disabled }];
}

export function mountAppUpdateView({ container, sidebarAction, bridge, saveEdits, onChange = () => {}, onInstalling = () => {}, onRecovery = () => {}, onOpen = () => {}, schedule, unschedule }) {
  const document = container.ownerDocument;
  const sidebar = sidebarAction || document.getElementById('sidebar-app-update');
  const get = id => container.querySelector(`#app-update-${id}`);
  const methods = { app_update_status: bridge.appUpdateStatus, app_update_check: bridge.checkAppUpdate, app_update_download: bridge.downloadAppUpdate, app_update_install: bridge.installAppUpdate, app_update_recovery_quit: bridge.quitAppUpdateRecovery };
  let busy = false, installedLock = false;
  const inertState = new Map();
  function setInstalling(value) {
    if (installedLock === value) return;
    installedLock = value;
    if (value) {
      for (const node of document.querySelectorAll('.workspace, dialog, [role="dialog"], [role="alertdialog"]')) { if (node.closest('[data-app-update-recovery]')) continue; inertState.set(node, node.inert); node.inert = true; }
    } else {
      for (const [node, inert] of inertState) if (node.isConnected) node.inert = inert;
      inertState.clear();
    }
    onInstalling(value);
  }
  function render(status) {
    const action = appUpdateAction(status);
    get('version').textContent = status.currentVersion ? `${status.currentVersion}${status.currentBuild ? ` (${status.currentBuild})` : ''}` : '';
    const message = ['waiting', 'error'].includes(status.phase) && status.message !== status.recoveryMessage ? status.message || '' : '';
    get('message').textContent = message;
    get('message').hidden = !message;
    get('action').hidden = status.enabled === false;
    // Settings carries the whole flow: check, download, then restart to install.
    get('action').textContent = action.label;
    get('action').disabled = busy || action.disabled;
    get('action').classList.toggle('primary', ['download', 'install'].includes(action.action));
    get('recovery').hidden = !status.recoveryMessage;
    get('recovery').textContent = status.recoveryMessage || '';
    const progress = get('progress');
    progress.hidden = status.phase !== 'downloading';
    if (status.totalBytes > 0) { progress.max = status.totalBytes; progress.value = Math.min(status.totalBytes, status.downloadedBytes || 0); }
    else progress.removeAttribute('value');
    if (sidebar) {
      const downloading = status.phase === 'downloading';
      const percent = status.totalBytes > 0 ? Math.min(100, Math.max(0, Math.floor((status.downloadedBytes || 0) / status.totalBytes * 100))) : null;
      const recovery = status.recoveryRequired || status.phase === 'recoveryRequired';
      const version = status.availableVersion ? ` ${status.availableVersion}` : '';
      const label = recovery ? '업데이트 복구 필요' : downloading ? `업데이트 다운로드 중${percent === null ? '' : ` ${percent}%`}` : ['ready', 'waiting'].includes(status.phase) || status.downloadReady ? `업데이트${version} 설치 준비됨` : `업데이트${version} 있음`;
      // The rail shows that an update exists and opens its settings row.
      const exists = recovery || status.downloadReady || ['available', 'downloading', 'ready', 'waiting', 'installing'].includes(status.phase);
      sidebar.hidden = status.enabled === false || !exists;
      sidebar.disabled = false;
      sidebar.dataset.updatePhase = status.phase;
      sidebar.setAttribute('aria-label', label);
      sidebar.title = label;
      const text = sidebar.querySelector('[data-app-update-label]');
      if (text) text.textContent = label;
      const sidebarProgress = sidebar.querySelector('[data-app-update-progress]');
      if (sidebarProgress) {
        sidebarProgress.hidden = !downloading;
        sidebarProgress.max = 100;
        if (percent === null) sidebarProgress.removeAttribute('value');
        else sidebarProgress.value = percent;
      }
    }
  }
  const updater = createAppUpdater({
    invoke: command => methods[command](), listen: (_, callback) => bridge.subscribeAppUpdate(payload => callback({ payload })),
    saveEdits, setInstalling, schedule, unschedule,
    onChange(status) { onRecovery(status.phase === 'recoveryRequired' || status.recoveryRequired ? status : null); render(status); onChange(appUpdateNotices(status)); },
  });
  async function perform() {
    const status = updater.getSnapshot();
    const action = appUpdateAction(status);
    if (busy || action.disabled || !action.action) return false;
    busy = true; render(updater.getSnapshot());
    try { return await updater[action.action](); }
    finally { busy = false; render(updater.getSnapshot()); }
  }
  const click = () => { void onOpen(); };
  const checkClick = async () => { await perform(); };
  get('action').addEventListener('click', checkClick);
  sidebar?.addEventListener('click', click);
  render(updater.getSnapshot());
  return { start: () => updater.start(), perform, getSnapshot: updater.getSnapshot, quitForRecovery: () => updater.quitForRecovery(),
    destroy() { updater.dispose(); get('action').removeEventListener('click', checkClick); sidebar?.removeEventListener('click', click); } };
}
