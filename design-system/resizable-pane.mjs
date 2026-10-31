export function mountResizablePane({ root, handle, bounds, preference, defaultValue, render,
  storage, storageKey, direction = 1, resizingClass = 'pane-resizing', onDragging = () => {} }) {
  const document = root.ownerDocument, view = document.defaultView;
  const cleanups = [];
  const on = (node, name, listener, options) => {
    node.addEventListener(name, listener, options);
    cleanups.push(() => node.removeEventListener(name, listener, options));
  };
  const read = () => { try { return preference(storage?.getItem(storageKey)); } catch { return defaultValue; } };
  let preferred = read(), actual = preferred, enabled = false, drag = null, frame = null;
  const clamp = (value, limits = bounds()) => Math.round(Math.max(limits.min, Math.min(limits.max, value)));
  function paint(value = preferred) {
    const limits = bounds();
    actual = clamp(value, limits);
    render(actual, limits);
    handle.setAttribute('aria-valuemin', String(limits.min));
    handle.setAttribute('aria-valuemax', String(limits.max));
    handle.setAttribute('aria-valuenow', String(actual));
    handle.setAttribute('aria-valuetext', `${actual}픽셀`);
  }
  function commit(value) {
    preferred = preference(value);
    try { storage?.setItem(storageKey, String(preferred)); } catch {}
    paint();
  }
  function finish(save = false) {
    if (!drag) return;
    const previous = drag;
    drag = null;
    if (frame !== null) { view.cancelAnimationFrame(frame); frame = null; }
    if (handle.hasPointerCapture?.(previous.id)) handle.releasePointerCapture(previous.id);
    root.classList.remove(resizingClass);
    if (save && previous.current !== previous.width) commit(clamp(previous.current)); else paint();
    onDragging(false);
  }
  on(handle, 'pointerdown', event => {
    if (!enabled || drag || event.button !== 0 || event.isPrimary === false) return;
    event.preventDefault();
    drag = { id: event.pointerId, x: event.clientX, width: actual, current: actual };
    handle.setPointerCapture?.(event.pointerId);
    handle.focus({ preventScroll: true });
    root.classList.add(resizingClass);
    onDragging(true);
  });
  on(handle, 'pointermove', event => {
    if (!drag || drag.id !== event.pointerId) return;
    drag.current = clamp(drag.width + direction * (event.clientX - drag.x));
    if (frame === null) frame = view.requestAnimationFrame(() => { frame = null; if (drag) paint(drag.current); });
  });
  on(handle, 'pointerup', event => {
    if (!drag || drag.id !== event.pointerId) return;
    drag.current = clamp(drag.width + direction * (event.clientX - drag.x));
    finish(true);
  });
  const cancel = event => { if (drag?.id === event.pointerId) finish(); };
  on(handle, 'pointercancel', cancel);
  on(handle, 'lostpointercapture', cancel);
  on(handle, 'dblclick', () => { if (enabled) { finish(); commit(defaultValue); } });
  on(handle, 'keydown', event => {
    if (!enabled || event.isComposing || event.keyCode === 229 || event.altKey || event.metaKey || event.ctrlKey) return;
    const limits = bounds();
    const value = { Home: limits.min, End: limits.max, ArrowLeft: actual - direction * 8, ArrowRight: actual + direction * 8 }[event.key];
    if (value === undefined) return;
    event.preventDefault(); commit(clamp(value));
  });
  on(document, 'keydown', event => {
    if (!drag || event.key !== 'Escape' || event.isComposing || event.keyCode === 229) return;
    event.preventDefault(); event.stopPropagation(); finish();
  }, true);
  on(view, 'blur', () => finish());
  on(document, 'visibilitychange', () => { if (document.hidden) finish(); });
  const resize = () => { finish(); paint(); };
  on(view, 'resize', resize);
  const observer = view.ResizeObserver ? new view.ResizeObserver(resize) : null;
  observer?.observe(root);
  on(view, 'storage', event => {
    if (event.key !== storageKey && event.key !== null) return;
    finish(); preferred = read(); paint();
  });
  paint();
  return {
    update(open) { const changed = enabled !== open; enabled = open; handle.hidden = !open; if (!open) finish(); else if (changed) paint(); },
    isDragging: () => Boolean(drag),
    refresh: resize,
    destroy() { finish(); observer?.disconnect(); cleanups.forEach(cleanup => cleanup()); },
  };
}
