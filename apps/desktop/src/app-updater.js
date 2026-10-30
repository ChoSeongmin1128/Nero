// Native verification and installation policy live in Rust. The consuming screen
// owns styling and must finish every editor save before calling install().
export function createAppUpdater({ invoke, listen, saveEdits, onChange = () => {}, setInstalling = () => {}, schedule = setTimeout, unschedule = clearTimeout }) {
  let snapshot = { phase: 'idle', message: '업데이트를 확인할 수 있습니다.' };
  let disposed = false;
  let busy = false;
  let removeListener;
  let timer;
  const publish = value => {
    if (disposed || !value) return;
    snapshot = { ...value };
    setInstalling(snapshot.phase === 'installing');
    onChange(snapshot);
  };
  const run = async (command, before) => {
    if (disposed || busy) return false;
    busy = true;
    try {
      if (before && await before() === false) return false;
      if (disposed) return false;
      publish(await invoke(command));
      return true;
    } catch (error) {
      publish({ ...snapshot, phase: 'error', message: typeof error === 'string' ? error : error?.message || '업데이트를 완료하지 못했습니다. 다시 시도해 주세요.' });
      return false;
    } finally { busy = false; }
  };
  const download = () => run('app_update_download');
  const check = async () => {
    const checked = await run('app_update_check');
    if (checked && snapshot.phase === 'available') return download();
    return checked;
  };
  const install = () => {
    if (disposed || busy) return Promise.resolve(false);
    return run('app_update_install', async () => {
      if (typeof saveEdits !== 'function') throw Error('열려 있는 편집 내용을 먼저 저장해 주세요.');
      const saved = await saveEdits();
      if (saved === false || disposed) return false;
      setInstalling(true);
      return true;
    }).finally(() => { if (snapshot.phase !== 'installing') setInstalling(false); });
  };
  const autoCheck = async () => {
    if (disposed) return;
    await check();
    if (!disposed) timer = schedule(autoCheck, 6 * 60 * 60 * 1000);
  };
  return {
    async start() {
      if (disposed || removeListener) return;
      const stop = await listen('nero://app-update', event => publish(event.payload));
      if (disposed) { stop(); return; }
      removeListener = stop;
      await run('app_update_status');
      if (!disposed) timer = schedule(autoCheck, 45_000);
    },
    check, download, install,
    getSnapshot: () => ({ ...snapshot }),
    dispose() { disposed = true; if (timer !== undefined) unschedule(timer); removeListener?.(); setInstalling(false); },
  };
}

export function appUpdateAction(status) {
  if (['checking', 'downloading', 'installing'].includes(status?.phase)) return { action: null, label: '진행 중…', disabled: true };
  if (['ready', 'waiting'].includes(status?.phase)) return { action: 'install', label: '다시 시작해 업데이트', disabled: false };
  if (status?.phase === 'error' && status.downloadReady) return { action: 'install', label: '업데이트 설치 재시도', disabled: false };
  if (status?.phase === 'available') return { action: 'download', label: '업데이트 다운로드', disabled: false };
  return { action: 'check', label: '업데이트 확인', disabled: false };
}
