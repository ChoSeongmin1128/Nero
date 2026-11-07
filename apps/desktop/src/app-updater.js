// Native verification and installation policy live in Rust. The consuming screen
// owns styling and must finish every editor save before calling install().
const RETRY_DELAY = 5 * 60 * 1000;
const CHECK_INTERVAL = 6 * 60 * 60 * 1000;

export function createAppUpdater({ invoke, listen, saveEdits, onChange = () => {}, setInstalling = () => {}, schedule = setTimeout, unschedule = clearTimeout }) {
  let snapshot = { phase: 'idle', message: '' };
  let disposed = false;
  let busy = false;
  let recoveryQuitting = false;
  let removeListener;
  let timer;
  // Each automatic retry may download the whole package again, so consecutive
  // failures back off until they reach the ordinary check interval.
  let failures = 0;
  const retryDelay = () => Math.min(RETRY_DELAY * 2 ** failures, CHECK_INTERVAL);
  const scheduleCheck = delay => {
    if (timer !== undefined) unschedule(timer);
    timer = undefined;
    if (!disposed && snapshot.enabled !== false && !snapshot.recoveryRequired && removeListener) timer = schedule(autoCheck, delay);
  };
  const publish = value => {
    if (disposed || !value) return;
    if (snapshot.recoveryRequired && !value.recoveryRequired) return;
    snapshot = { ...value };
    if (snapshot.phase === 'current' || snapshot.downloadReady) failures = 0;
    if (snapshot.enabled === false && timer !== undefined) { unschedule(timer); timer = undefined; }
    setInstalling(snapshot.phase === 'installing' || Boolean(snapshot.recoveryRequired));
    onChange(snapshot);
    if (snapshot.recoveryRequired || (snapshot.phase === 'error' && !snapshot.downloadReady)) scheduleCheck(retryDelay());
  };
  const run = async (command, before) => {
    if (disposed || busy || (snapshot.enabled === false && command !== 'app_update_status')) return false;
    if (snapshot.recoveryRequired) return false;
    busy = true;
    try {
      if (before && await before() === false) return false;
      if (disposed) return false;
      publish(await invoke(command));
      return true;
    } catch (error) {
      if (!snapshot.recoveryRequired) publish({ ...snapshot, phase: 'error', message: typeof error === 'string' ? error : error?.message || '업데이트를 완료하지 못했습니다. 다시 시도해 주세요.' });
      return false;
    } finally { busy = false; }
  };
  const download = () => run('app_update_download');
  const check = async () => {
    const checked = await run('app_update_check');
    if (checked && snapshot.phase === 'available') return download();
    return checked;
  };
  const quitForRecovery = async () => {
    if (disposed || recoveryQuitting || !snapshot.recoveryRequired) return false;
    recoveryQuitting = true;
    try { await invoke('app_update_recovery_quit'); return true; }
    catch (error) {
      publish({ ...snapshot, message: typeof error === 'string' ? error : error?.message || '앱을 종료하지 못했습니다. 다시 시도해 주세요.' });
      return false;
    } finally { recoveryQuitting = false; }
  };
  const install = () => {
    if (disposed || busy || snapshot.enabled === false || snapshot.recoveryRequired) return Promise.resolve(false);
    return run('app_update_install', async () => {
      if (typeof saveEdits !== 'function') throw Error('열려 있는 편집 내용을 먼저 저장해 주세요.');
      const saved = await saveEdits();
      if (saved === false || disposed || snapshot.recoveryRequired) return false;
      setInstalling(true);
      return true;
    }).finally(() => { if (snapshot.phase !== 'installing' && !snapshot.recoveryRequired) setInstalling(false); });
  };
  const autoCheck = async () => {
    timer = undefined;
    if (disposed || snapshot.enabled === false || snapshot.recoveryRequired) return;
    if (busy || ['checking', 'downloading', 'installing'].includes(snapshot.phase)) {
      scheduleCheck(RETRY_DELAY);
      return;
    }
    if (!snapshot.downloadReady) await check();
    const failed = snapshot.phase === 'error' && !snapshot.downloadReady;
    if (timer === undefined) scheduleCheck(failed ? retryDelay() : CHECK_INTERVAL);
    if (failed) failures += 1;
  };
  return {
    async start() {
      if (disposed || removeListener) return;
      const stop = await listen('nero://app-update', event => publish(event.payload));
      if (disposed) { stop(); return; }
      removeListener = stop;
      await run('app_update_status');
      scheduleCheck(45_000);
    },
    check, download, install,
    quitForRecovery,
    getSnapshot: () => ({ ...snapshot }),
    dispose() { disposed = true; if (timer !== undefined) unschedule(timer); removeListener?.(); setInstalling(false); },
  };
}

export function appUpdateAction(status) {
  if (status?.enabled === false) return { action: null, label: '', disabled: true };
  if (status?.recoveryRequired || status?.phase === 'recoveryRequired') return { action: 'quitForRecovery', label: '복구 안내 확인 후 종료', disabled: false };
  if (status?.phase === 'checking') return { action: null, label: '확인 중…', disabled: true };
  if (status?.phase === 'downloading') return { action: null, label: '다운로드 중…', disabled: true };
  if (status?.phase === 'installing') return { action: null, label: '설치 중…', disabled: true };
  if (['ready', 'waiting'].includes(status?.phase)) return { action: 'install', label: '다시 시작해 업데이트', disabled: false };
  if (status?.phase === 'error' && status.downloadReady) return { action: 'install', label: '업데이트 설치 재시도', disabled: false };
  if (status?.phase === 'available') return { action: 'download', label: '업데이트 다운로드', disabled: false };
  return { action: 'check', label: '업데이트 확인', disabled: false };
}
