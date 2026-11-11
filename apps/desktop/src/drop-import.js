// Audio files dropped on the window. Native file drop stays off because it would capture
// in-app drags (meetings into folders), so dropped files arrive here and are streamed to
// the app in bounded chunks. Without this, the webview would open the file as a page.
export const DROP_CHUNK = 8 * 1024 * 1024;

export const carriesFiles = event => Array.from(event.dataTransfer?.types || []).includes('Files');

export async function uploadDroppedFile(bridge, file, target, onProgress, { signal, onCommitting } = {}) {
  const check = () => { if (signal?.aborted) throw new DOMException('가져오기를 취소했습니다.', 'AbortError'); };
  check();
  const token = await bridge.dropImportBegin(file.name, file.size, target);
  try {
    check();
    for (let offset = 0; offset < file.size; offset += DROP_CHUNK) {
      const bytes = new Uint8Array(await file.slice(offset, offset + DROP_CHUNK).arrayBuffer());
      check();
      await bridge.dropImportChunk(token, bytes);
      check();
      onProgress?.(Math.min(file.size, offset + bytes.length), file.size);
    }
    check();
    onCommitting?.();
    return await bridge.dropImportFinish(token);
  } catch (error) {
    await bridge.dropImportCancel(token).catch(() => { throw new Error('가져오기 임시 파일을 정리하지 못했습니다. 앱을 다시 열어 주세요.'); });
    throw error;
  }
}

export function mountDropImport({ window, overlay, message, cancelButton, onDrop }) {
  let depth = 0, busy = false, controller = null, committing = false;
  const cancel = () => {
    if (!busy || !controller || committing || controller.signal.aborted) return;
    controller.abort();
    if (cancelButton) cancelButton.disabled = true;
    message.textContent = '가져오기 취소 중';
  };
  cancelButton?.addEventListener('click', cancel);
  const show = visible => { if (overlay.hidden === visible) overlay.hidden = !visible; };
  const idleText = '여기에 놓으면 음성을 가져옵니다';
  window.addEventListener('dragenter', event => {
    if (!carriesFiles(event)) return;
    event.preventDefault();
    depth += 1;
    if (!busy) message.textContent = idleText;
    show(true);
  });
  window.addEventListener('dragover', event => {
    if (!carriesFiles(event)) return;
    event.preventDefault();
    event.dataTransfer.dropEffect = busy ? 'none' : 'copy';
  });
  window.addEventListener('dragleave', event => {
    if (!carriesFiles(event)) return;
    depth = Math.max(0, depth - 1);
    if (!depth && !busy) show(false);
  });
  window.addEventListener('drop', async event => {
    if (!carriesFiles(event)) return;
    event.preventDefault();
    depth = 0;
    const files = Array.from(event.dataTransfer.files || []);
    if (busy || !files.length) { if (!busy) show(false); return; }
    busy = true;
    committing = false;
    controller = new AbortController();
    overlay.dataset.busy = 'true';
    if (cancelButton) { cancelButton.hidden = false; cancelButton.disabled = false; }
    try {
      await onDrop(files, (done, total) => {
        if (!controller.signal.aborted) message.textContent = `음성 파일을 받는 중 ${Math.floor(done / total * 100)}%`;
      }, { signal: controller.signal, onCommitting() {
        committing = true;
        if (cancelButton) cancelButton.hidden = true;
        message.textContent = '음성 가져오기 마무리 중';
      } });
    } finally {
      busy = false;
      controller = null;
      overlay.dataset.busy = 'false';
      if (cancelButton) cancelButton.hidden = true;
      show(false);
    }
  });
}
