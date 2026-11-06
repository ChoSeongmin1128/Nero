// Model downloads report their progress in Settings. This sidebar entry keeps a running
// download visible elsewhere and opens the Settings panel that holds its progress.
const busy = view => ['queued', 'running', 'cancelling'].includes(view?.state);

function write(node, key, value) {
  if (node && node[key] !== value) node[key] = value;
}

export function modelDownloadLabel(view) {
  if (view.state === 'queued') return '모델 다운로드 대기';
  if (view.state === 'cancelling') return '모델 다운로드 취소 중';
  return view.percent == null ? '모델 확인 중' : `모델 다운로드 ${view.percent}%`;
}

export function mountModelDownloadStatus({ button, onOpen }) {
  const label = button.querySelector('[data-model-download-label]');
  const bar = button.querySelector('[data-model-download-progress]');
  let panel = null;
  button.addEventListener('click', () => { if (panel && !button.disabled) onOpen(panel); });
  return {
    update({ speech, summary } = {}) {
      const [view, target] = busy(speech) ? [speech, 'transcription'] : busy(summary) ? [summary, 'appearance'] : [null, null];
      panel = target;
      write(button, 'hidden', !view);
      if (!view) return;
      const text = modelDownloadLabel(view);
      write(label, 'textContent', text);
      const description = `${text}, 설정에서 보기`;
      if (button.getAttribute('aria-label') !== description) button.setAttribute('aria-label', description);
      write(button, 'title', description);
      button.dataset.state = view.state;
      if (bar) {
        write(bar, 'hidden', view.state !== 'running');
        write(bar, 'max', 100);
        if (view.percent == null) { if (bar.hasAttribute('value')) bar.removeAttribute('value'); }
        else if (!bar.hasAttribute('value') || bar.value !== view.percent) bar.value = view.percent;
      }
    },
  };
}
