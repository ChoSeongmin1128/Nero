export function mountWindowChrome({ root, nativeWindow }) {
  if (root?.dataset.windowChrome !== 'macos-overlay') return { dispose() {} };

  const subscriptions = new Set();
  let disposed = false, epoch = 0;
  const report = error => { if (!disposed) console.warn('Nero 창 상태를 확인하지 못했습니다.', error); };

  async function refresh() {
    if (disposed) return;
    const current = ++epoch;
    try {
      const fullscreen = await nativeWindow.isFullscreen();
      if (typeof fullscreen !== 'boolean') throw new TypeError('Invalid fullscreen state');
      if (!disposed && current === epoch) root.dataset.windowFullscreen = String(fullscreen);
    } catch (error) {
      if (current === epoch) report(error);
    }
  }

  function unsubscribe(remove) {
    try { Promise.resolve(remove()).catch(report); } catch (error) { report(error); }
  }

  async function subscribe(register) {
    try {
      const remove = await register(refresh);
      if (typeof remove !== 'function') throw new TypeError('Invalid window subscription');
      if (disposed) unsubscribe(remove);
      else subscriptions.add(remove);
    } catch (error) {
      report(error);
    }
  }

  const pending = [subscribe(callback => nativeWindow.onResized(callback))];
  if (typeof nativeWindow.onFocusChanged === 'function') pending.push(subscribe(callback => nativeWindow.onFocusChanged(callback)));
  refresh();
  Promise.all(pending).then(refresh);

  return {
    dispose() {
      if (disposed) return;
      disposed = true;
      epoch++;
      for (const remove of subscriptions) unsubscribe(remove);
      subscriptions.clear();
    },
  };
}
