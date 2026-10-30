const DEFAULT_WIDTH = 224;
const MIN_WIDTH = 200;
const MAX_WIDTH = 400;
const CONTENT_MIN = 400;
const OVERLAY_MAX = 640;
const OVERLAY_GAP = 48;

export function navigationWidthPreference(value) {
  const number = typeof value === 'number' || typeof value === 'string' && value.trim() ? Number(value) : NaN;
  return Number.isFinite(number) && number > 0 ? Math.round(Math.max(MIN_WIDTH, Math.min(MAX_WIDTH, number))) : DEFAULT_WIDTH;
}

export function navigationWidthBounds(viewport) {
  const width = Math.max(0, viewport);
  const overlay = width <= OVERLAY_MAX;
  const max = Math.max(0, Math.min(MAX_WIDTH, width - (overlay ? OVERLAY_GAP : CONTENT_MIN)));
  return { min: Math.min(MIN_WIDTH, max), max, overlay };
}

export function mountNavigationWidth({ shell, handle, storage, storageKey, onDragging }) {
  const document = shell.ownerDocument, view = document.defaultView;
  const read = () => { try { return navigationWidthPreference(storage?.getItem(storageKey)); } catch { return DEFAULT_WIDTH; } };
  let preferred = read(), actual = preferred, enabled = false, drag = null, frame = null;
  const bounds = () => navigationWidthBounds(shell.getBoundingClientRect().width || view.innerWidth);
  const clamp = (width, limits = bounds()) => Math.round(Math.max(limits.min, Math.min(limits.max, width)));
  function paint(width = preferred) {
    const limits = bounds();
    actual = clamp(width, limits);
    shell.style.setProperty('--navigation-width', `${actual}px`);
    shell.classList.toggle('nav-overlay', limits.overlay);
    handle.setAttribute('aria-valuemin', String(limits.min));
    handle.setAttribute('aria-valuemax', String(limits.max));
    handle.setAttribute('aria-valuenow', String(actual));
    handle.setAttribute('aria-valuetext', `${actual}픽셀`);
  }
  function commit(width) {
    preferred = navigationWidthPreference(width);
    try { storage?.setItem(storageKey, String(preferred)); } catch {}
    paint();
  }
  function finish(commitDrag = false) {
    if (!drag) return;
    const previous = drag;
    drag = null;
    if (frame !== null) { view.cancelAnimationFrame(frame); frame = null; }
    if (handle.hasPointerCapture?.(previous.id)) handle.releasePointerCapture(previous.id);
    shell.classList.remove('nav-resizing');
    if (commitDrag && previous.current !== previous.width) commit(clamp(previous.current)); else paint();
    onDragging(false);
  }
  handle.addEventListener('pointerdown', event => {
    if (!enabled || drag || event.button !== 0 || event.isPrimary === false) return;
    drag = { id: event.pointerId, x: event.clientX, width: actual, current: actual };
    handle.setPointerCapture?.(event.pointerId);
    shell.classList.add('nav-resizing');
    onDragging(true);
  });
  handle.addEventListener('pointermove', event => {
    if (!drag || drag.id !== event.pointerId) return;
    drag.current = clamp(drag.width + event.clientX - drag.x);
    if (frame === null) frame = view.requestAnimationFrame(() => { frame = null; if (drag) paint(drag.current); });
  });
  handle.addEventListener('pointerup', event => {
    if (!drag || drag.id !== event.pointerId) return;
    drag.current = clamp(drag.width + event.clientX - drag.x);
    finish(true);
  });
  const cancelPointer = event => { if (drag?.id === event.pointerId) finish(); };
  handle.addEventListener('pointercancel', cancelPointer);
  handle.addEventListener('lostpointercapture', cancelPointer);
  handle.addEventListener('dblclick', () => { if (enabled) { finish(); commit(DEFAULT_WIDTH); } });
  handle.addEventListener('keydown', event => {
    if (!enabled || event.isComposing || event.keyCode === 229 || event.altKey || event.metaKey || event.ctrlKey) return;
    const limits = bounds();
    const width = { Home: limits.min, End: limits.max, ArrowLeft: actual - 8, ArrowRight: actual + 8 }[event.key];
    if (width === undefined) return;
    event.preventDefault();
    commit(clamp(width));
  });
  document.addEventListener('keydown', event => {
    if (!drag || event.key !== 'Escape' || event.isComposing || event.keyCode === 229) return;
    event.preventDefault(); event.stopPropagation(); finish();
  }, true);
  view.addEventListener('blur', () => finish());
  document.addEventListener('visibilitychange', () => { if (document.hidden) finish(); });
  const resize = () => { finish(); paint(); };
  view.addEventListener('resize', resize);
  const observer = view.ResizeObserver ? new view.ResizeObserver(resize) : null;
  observer?.observe(shell);
  view.addEventListener('storage', event => {
    if (event.key !== storageKey && event.key !== null) return;
    finish(); preferred = read(); paint();
  });
  paint();
  return {
    update(open) { enabled = open; handle.hidden = !open; if (!open) finish(); },
    isDragging: () => Boolean(drag),
  };
}
