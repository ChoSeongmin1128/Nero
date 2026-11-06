// Audio files dropped on the window. Native file drop stays off because it would capture
// in-app drags (meetings into folders), so dropped files arrive here and are streamed to
// the app in bounded chunks. Without this, the webview would open the file as a page.
export const DROP_CHUNK = 8 * 1024 * 1024;

export const carriesFiles = event => Array.from(event.dataTransfer?.types || []).includes('Files');

export async function uploadDroppedFile(bridge, file, target, onProgress) {
  const token = await bridge.dropImportBegin(file.name, file.size, target);
  try {
    for (let offset = 0; offset < file.size; offset += DROP_CHUNK) {
      const bytes = new Uint8Array(await file.slice(offset, offset + DROP_CHUNK).arrayBuffer());
      await bridge.dropImportChunk(token, bytes);
      onProgress?.(Math.min(file.size, offset + bytes.length), file.size);
    }
    return await bridge.dropImportFinish(token);
  } catch (error) {
    void bridge.dropImportCancel(token).catch(() => {});
    throw error;
  }
}

export function mountDropImport({ window, overlay, message, onDrop }) {
  let depth = 0, busy = false;
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
    try {
      await onDrop(files, (done, total) => {
        message.textContent = `음성 파일을 받는 중 ${Math.floor(done / total * 100)}%`;
      });
    } finally {
      busy = false;
      show(false);
    }
  });
}
