// Writes a modelPreparation() view into existing nodes. Values are written only when
// they change: rewriting identical text every progress tick re-lays out scroll
// containers and makes screen readers repeat the live region.
function write(node, key, value) {
  if (node && node[key] !== value) node[key] = value;
}

export function bindPreparationProgress({ region, bar, spinner, stage, size, cancel }) {
  return {
    update(view = { state: 'idle' }) {
      const busy = ['queued', 'running', 'cancelling'].includes(view.state);
      write(region, 'hidden', !busy);
      if (!busy) return;
      if (bar) {
        if (view.percent == null) {
          if (bar.hasAttribute?.('value')) bar.removeAttribute('value');
        } else {
          write(bar, 'max', view.max);
          // An indeterminate bar already reads value 0, so restore the attribute explicitly.
          if (bar.hasAttribute?.('value') === false || bar.value !== view.value) bar.value = view.value;
        }
        // Unknown totals show the spinner only, never a misleading empty bar.
        write(bar, 'hidden', view.percent == null);
      }
      write(spinner, 'hidden', view.state !== 'running');
      write(stage, 'textContent', view.stage || '');
      write(size, 'textContent', view.percent == null ? '' : `${view.percent}%${view.size ? ` · ${view.size}` : ''}`);
      if (cancel) {
        write(cancel, 'hidden', view.state === 'queued' && !view.cancellable);
        write(cancel, 'disabled', !view.cancellable);
        write(cancel, 'textContent', view.state === 'cancelling' ? '취소 중' : '취소');
        if (cancel.dataset) write(cancel.dataset, 'jobId', view.jobId || '');
      }
    },
  };
}
