let nextEvidenceId = 0;

const identity = context => JSON.stringify([context?.meetingId, context?.summaryId, context?.currentVersion, Boolean(context?.dirty)]);
const referencesFor = anchor => {
  const values = String(anchor?.dataset.summaryCitations || '').split(',').map(Number);
  return values.length && values.length <= 32 && values.every(value => Number.isSafeInteger(value) && value > 0)
    ? [...new Set(values)] : [];
};
const clock = value => {
  if (!Number.isFinite(value) || value < 0) return '';
  const seconds = Math.floor(value), hours = Math.floor(seconds / 3600);
  return [hours || null, Math.floor(seconds / 60) % 60, seconds % 60].filter(value => value !== null).map(value => String(value).padStart(2, '0')).join(':');
};

/** Frozen evidence is loaded by summary ID. Current-row navigation is a separate caller-owned action. */
export function mountSummaryEvidence({ container, getContext, loadCitations, onNavigate, onPlay, onError = () => {} }) {
  const doc = container.ownerDocument, win = doc.defaultView;
  const panel = doc.createElement('section');
  panel.id = `summary-evidence-${++nextEvidenceId}`;
  panel.className = 'summary-evidence';
  panel.setAttribute('role', 'dialog');
  panel.setAttribute('aria-label', '요약 근거');
  panel.hidden = true;
  doc.body.append(panel);
  let anchor = null, returnAnchor = null, returnReferences = '', returnContext = null, sourceContext = null;
  let sourceKey = '', generation = 0, pinned = false, closeTimer = null, suppressedFocus = false, destroyed = false, response = null;
  const listeners = [];
  const listen = (node, name, handler, options) => { node.addEventListener(name, handler, options); listeners.push(() => node.removeEventListener(name, handler, options)); };
  const cancelClose = () => { if (closeTimer !== null) win.clearTimeout(closeTimer); closeTimer = null; };
  const contextMatches = () => sourceContext && sourceKey === identity(getContext());
  const active = ticket => !destroyed && ticket === generation && anchor?.isConnected && container.contains(anchor) && contextMatches();
  const element = (tag, className, text) => { const node = doc.createElement(tag); if (className) node.className = className; if (text != null) node.textContent = text; return node; };
  const actionButton = (label, action) => {
    const button = element('button', 'quiet small', label); button.type = 'button'; button.dataset.evidenceAction = action; return button;
  };

  function focusAnchor(target) {
    if (!target?.isConnected || !container.contains(target)) return false;
    suppressedFocus = true;
    target.focus({ preventScroll: true });
    suppressedFocus = false;
    return true;
  }
  function reset({ focus = false } = {}) {
    cancelClose(); generation += 1;
    const previous = anchor;
    if (anchor) { anchor.setAttribute('aria-expanded', 'false'); anchor.removeAttribute('aria-controls'); }
    anchor = null; sourceContext = null; sourceKey = ''; pinned = false; response = null;
    panel.hidden = true; panel.replaceChildren();
    if (focus) focusAnchor(previous);
  }
  function position() {
    if (!anchor || panel.hidden) return;
    const viewport = win.visualViewport;
    const leftEdge = (viewport?.offsetLeft || 0) + 8, topEdge = (viewport?.offsetTop || 0) + 8;
    const width = viewport?.width || win.innerWidth, height = viewport?.height || win.innerHeight;
    panel.style.maxWidth = `${Math.max(0, Math.min(420, width - 16))}px`;
    panel.style.maxHeight = `${Math.max(0, Math.min(320, height - 16))}px`;
    const target = anchor.getBoundingClientRect(), bounds = panel.getBoundingClientRect();
    const rightEdge = leftEdge + width - 16, bottomEdge = topEdge + height - 16;
    const below = target.bottom + 6;
    const top = below + bounds.height <= bottomEdge ? below : target.top - bounds.height - 6;
    panel.style.left = `${Math.max(leftEdge, Math.min(target.left, rightEdge - bounds.width))}px`;
    panel.style.top = `${Math.max(topEdge, Math.min(top, bottomEdge - bounds.height))}px`;
  }
  function header() {
    const row = element('div', 'summary-evidence-heading');
    row.append(element('strong', '', '요약에 사용된 발언'), actionButton('닫기', 'close'));
    return row;
  }
  function status(text) {
    const restorePanelFocus = panel.contains(doc.activeElement);
    panel.replaceChildren(header());
    const message = element('p', 'summary-evidence-message', text); message.setAttribute('role', 'status'); panel.append(message); position();
    if (restorePanelFocus) panel.querySelector('button')?.focus();
  }
  function render(result) {
    const restorePanelFocus = panel.contains(doc.activeElement);
    const entries = Array.isArray(result.entries) ? result.entries : [];
    if (!entries.length) { status('근거를 찾을 수 없습니다.'); return; }
    panel.replaceChildren(header());
    for (const [index, entry] of entries.entries()) {
      const item = element('article', 'summary-evidence-entry');
      const meta = element('div', 'summary-evidence-meta');
      meta.append(element('span', 'type-timer', clock(entry.segment?.start)), element('span', '', entry.speakerName || '화자 미지정'));
      item.append(meta, element('p', 'summary-evidence-text', entry.segment?.text || ''));
      const actions = element('div', 'summary-evidence-actions');
      if (!sourceContext.dirty && Array.isArray(entry.currentSegmentIds) && entry.currentSegmentIds.length && onNavigate) {
        const button = actionButton('음성 기록에서 보기', 'navigate'); button.dataset.evidenceIndex = String(index); actions.append(button);
      }
      if (result.audioAvailable && getContext()?.canPlay && Number.isFinite(entry.segment?.start) && entry.segment.start >= 0 && onPlay) {
        const button = actionButton('재생', 'play'); button.dataset.evidenceIndex = String(index); actions.append(button);
      }
      if (actions.childElementCount) item.append(actions);
      panel.append(item);
    }
    position();
    if (restorePanelFocus) panel.querySelector('button')?.focus();
  }
  async function open(target, pin = false) {
    cancelClose();
    if (pinned && !pin && anchor !== target) return;
    if (anchor === target && contextMatches()) { pinned ||= pin; return; }
    const refs = referencesFor(target), context = getContext();
    if (!refs.length || !context?.meetingId || !context.summaryId) return;
    reset();
    anchor = target; sourceContext = { ...context }; sourceKey = identity(context); pinned = pin;
    returnAnchor = target; returnReferences = target.dataset.summaryCitations; returnContext = { meetingId: context.meetingId, summaryId: context.summaryId };
    const ticket = generation;
    target.setAttribute('aria-expanded', 'true'); target.setAttribute('aria-controls', panel.id);
    panel.hidden = false; status('불러오는 중');
    try {
      const result = await loadCitations(context.meetingId, context.summaryId, refs);
      if (!active(ticket)) { if (ticket === generation) reset(); return; }
      if (result?.summaryId !== context.summaryId) { status('근거를 불러오지 못했습니다.'); return; }
      // Only render requested references, even if an adapter returns extra entries.
      response = { ...result, entries: (Array.isArray(result.entries) ? result.entries : []).filter(entry => refs.includes(entry.reference)) };
      render(response);
    } catch (error) {
      if (!active(ticket)) { if (ticket === generation) reset(); return; }
      status('근거를 불러오지 못했습니다.'); onError(error);
    }
  }
  function scheduleClose() {
    cancelClose();
    if (pinned || panel.contains(doc.activeElement) || anchor?.contains(doc.activeElement)) return;
    closeTimer = win.setTimeout(() => reset(), 160);
  }
  const targetFor = target => { const found = target?.closest?.('[data-summary-citations]'); return found && container.contains(found) ? found : null; };
  listen(container, 'pointerover', event => { const target = targetFor(event.target); if (target && !target.contains(event.relatedTarget)) void open(target); });
  listen(container, 'pointerout', event => {
    const target = targetFor(event.target);
    if (target && !target.contains(event.relatedTarget) && !panel.contains(event.relatedTarget)) scheduleClose();
  });
  listen(container, 'focusin', event => { const target = targetFor(event.target); if (target && !suppressedFocus) void open(target); });
  listen(container, 'click', event => { const target = targetFor(event.target); if (!target) return; event.preventDefault(); void open(target, true); if (event.detail === 0) panel.querySelector('button')?.focus(); });
  listen(container, 'keydown', event => {
    if (targetFor(event.target) !== anchor || panel.hidden) return;
    if (event.key === 'ArrowDown' || (event.key === 'Tab' && !event.shiftKey)) { event.preventDefault(); pinned = true; panel.querySelector('button')?.focus(); }
  });
  listen(panel, 'pointerenter', cancelClose);
  listen(panel, 'pointerleave', event => { if (!anchor?.contains(event.relatedTarget)) scheduleClose(); });
  listen(doc, 'pointerdown', event => { if (anchor && !anchor.contains(event.target) && !panel.contains(event.target)) reset(); }, true);
  listen(doc, 'focusin', event => { if (anchor && !anchor.contains(event.target) && !panel.contains(event.target)) reset(); });
  listen(doc, 'keydown', event => { if (anchor && event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); reset({ focus: true }); } });
  listen(panel, 'click', async event => {
    const button = event.target.closest('[data-evidence-action]');
    if (!button || button.disabled) return;
    if (button.dataset.evidenceAction === 'close') { reset({ focus: true }); return; }
    const ticket = generation, context = getContext(), entry = response?.entries?.[Number(button.dataset.evidenceIndex)];
    if (!active(ticket) || !entry) { reset(); return; }
    button.disabled = true;
    try {
      if (button.dataset.evidenceAction === 'navigate') {
        if (context.dirty || !entry.currentSegmentIds?.length) return;
        const moved = await onNavigate(entry, response, { ...context });
        if (active(ticket) && moved !== false) reset();
      } else if (button.dataset.evidenceAction === 'play' && response.audioAvailable && context.canPlay) await onPlay(entry, response, { ...context });
    } catch (error) { if (active(ticket)) { status('발언을 열지 못했습니다.'); onError(error); } }
    finally { if (button.isConnected) button.disabled = false; }
  });
  const update = () => { if (anchor && (!contextMatches() || !anchor.isConnected)) reset(); else position(); };
  listen(win, 'resize', update);
  listen(doc, 'scroll', update, true);
  if (win.visualViewport) { listen(win.visualViewport, 'resize', update); listen(win.visualViewport, 'scroll', update); }
  return {
    reset, update,
    restoreFocus() {
      const context = getContext();
      if (context?.meetingId !== returnContext?.meetingId || context?.summaryId !== returnContext?.summaryId) return false;
      const target = returnAnchor?.isConnected ? returnAnchor : Array.from(container.querySelectorAll('[data-summary-citations]')).find(node => node.dataset.summaryCitations === returnReferences);
      return focusAnchor(target);
    },
    destroy() { reset(); destroyed = true; for (const remove of listeners) remove(); panel.remove(); returnAnchor = null; returnContext = null; },
  };
}
