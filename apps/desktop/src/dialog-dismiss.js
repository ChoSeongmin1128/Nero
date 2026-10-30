export function bindDialogDismiss(dialog, { dismiss = () => dialog.close(), blocked = () => false } = {}) {
  let composing = false;
  let pointer = null;
  let pending = false;
  const nested = () => Boolean(dialog.querySelector('[role="dialog"][aria-modal="true"], [role="alertdialog"][aria-modal="true"]'));
  const request = async () => {
    if (!dialog.open || pending || composing || blocked() || nested()) return;
    pending = true;
    try { await dismiss(); } finally { pending = false; }
  };
  const outside = event => {
    if (event.target !== dialog) return false;
    const box = dialog.getBoundingClientRect();
    return event.clientX < box.left || event.clientX > box.right || event.clientY < box.top || event.clientY > box.bottom;
  };
  dialog.addEventListener('compositionstart', () => { composing = true; });
  dialog.addEventListener('compositionend', () => { composing = false; });
  dialog.addEventListener('keydown', event => {
    if (event.key !== 'Escape' || event.defaultPrevented || blocked() || nested()) return;
    event.preventDefault();
    event.stopPropagation();
    if (event.isComposing || event.keyCode === 229 || composing) return;
    if (globalThis.NeroControls?.isOpen?.()) { globalThis.NeroControls.close(); return; }
    void request();
  });
  dialog.addEventListener('cancel', event => { event.preventDefault(); void request(); });
  dialog.addEventListener('pointerdown', event => {
    pointer = event.button === 0 && event.isPrimary !== false && !blocked() && !nested() && !globalThis.NeroControls?.isOpen?.() && outside(event) ? event.pointerId : null;
  });
  dialog.addEventListener('pointerup', event => {
    const eligible = pointer !== null && pointer === event.pointerId && event.button === 0 && event.isPrimary !== false && outside(event);
    pointer = null;
    if (eligible) void request();
  });
  dialog.addEventListener('pointercancel', () => { pointer = null; });
  dialog.addEventListener('close', () => { pointer = null; composing = false; });
}
