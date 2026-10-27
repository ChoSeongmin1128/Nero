const identity = context => JSON.stringify([context?.meetingId, context?.summaryId, context?.currentVersion, Boolean(context?.dirty)]);
const referencesFor = anchor => {
  const values = String(anchor?.dataset.summaryCitations || '').split(',').map(Number);
  return values.length && values.length <= 32 && values.every(value => Number.isSafeInteger(value) && value > 0) ? [...new Set(values)] : [];
};
const clock = value => {
  if (!Number.isFinite(value) || value < 0) return '';
  const seconds = Math.floor(value), hours = Math.floor(seconds / 3600);
  return [hours || null, Math.floor(seconds / 60) % 60, seconds % 60].filter(value => value !== null).map(value => String(value).padStart(2, '0')).join(':');
};

export function mountSummaryEvidence({ container, panel, getContext, loadCitations, onOpen, onNavigate, onPlay, onReset = () => {}, onError = () => {} }) {
  const doc = container.ownerDocument;
  let anchor = null, sourceKey = '', generation = 0, navigation = 0, response = null, index = 0, destroyed = false;
  const active = ticket => !destroyed && ticket === generation && anchor?.isConnected && container.contains(anchor) && sourceKey === identity(getContext());
  const element = (tag, className, text) => { const node = doc.createElement(tag); node.className = className; if (text != null) node.textContent = text; return node; };
  const action = (label, name, disabled = false) => { const button = element('button', 'quiet small', label); button.type = 'button'; button.dataset.evidenceAction = name; button.disabled = disabled; return button; };
  function reset() {
    generation++; navigation++; onReset();
    anchor?.setAttribute('aria-pressed', 'false');
    anchor = null; sourceKey = ''; response = null; panel.hidden = true; panel.replaceChildren();
  }
  function status(text) { panel.hidden = false; const node = element('p', 'summary-evidence-message', text); node.setAttribute('role', 'status'); panel.replaceChildren(node); }
  async function reveal(ticket) {
    const move = ++navigation;
    if (!active(ticket)) return;
    const entry = response?.entries[index];
    if (!entry) { status('연결된 발언을 찾을 수 없습니다.'); return; }
    panel.replaceChildren(); panel.hidden = false;
    if (response.entries.length > 1) {
      const nav = element('div', 'summary-evidence-navigation');
      nav.append(element('span', '', `연결된 발언 ${index + 1} / ${response.entries.length}`), action('이전', 'prev', index === 0), action('다음', 'next', index === response.entries.length - 1)); panel.append(nav);
    }
    const context = getContext();
    const moved = !context.dirty && entry.currentSegmentIds?.length && await onNavigate(entry, response, { ...context });
    if (!active(ticket) || move !== navigation) return;
    if (!moved) {
      const item = element('article', 'summary-evidence-entry');
      item.append(element('strong', '', '요약 당시 발언'), element('div', 'summary-evidence-meta', `${clock(entry.segment?.start)} ${entry.speakerName || '화자 미지정'}`), element('p', 'summary-evidence-text', entry.segment?.text || ''));
      if (response.audioAvailable && context.canPlay && Number.isFinite(entry.segment?.start) && entry.segment.start >= 0) item.append(action('재생', 'play'));
      panel.append(item);
    }
    panel.hidden = panel.childElementCount === 0;
  }
  async function open(target) {
    const refs = referencesFor(target), initial = getContext();
    if (!refs.length || !initial?.meetingId || !initial.summaryId) return;
    reset(); const ticket = generation;
    try {
      if (!await onOpen() || ticket !== generation || initial.meetingId !== getContext()?.meetingId || initial.summaryId !== getContext()?.summaryId) return;
      const context = getContext();
      if (!target.isConnected || !container.contains(target)) target = Array.from(container.querySelectorAll('[data-summary-citations]')).find(node => referencesFor(node).join(',') === refs.join(','));
      if (!target) return;
      anchor = target; sourceKey = identity(context); target.setAttribute('aria-pressed', 'true'); status('불러오는 중');
      const result = await loadCitations(context.meetingId, context.summaryId, refs);
      if (!active(ticket)) { if (ticket === generation) reset(); return; }
      if (result?.summaryId !== context.summaryId) { status('발언을 불러오지 못했습니다.'); return; }
      response = { ...result, entries: (Array.isArray(result.entries) ? result.entries : []).filter(entry => refs.includes(entry.reference)) }; index = 0;
      await reveal(ticket);
    } catch (error) { if (active(ticket)) { status('발언을 불러오지 못했습니다.'); onError(error); } }
  }
  const click = event => { const target = event.target.closest?.('[data-summary-citations]'); if (target && container.contains(target)) { event.preventDefault(); void open(target); } };
  const panelClick = async event => {
    const button = event.target.closest?.('[data-evidence-action]'); if (!button || button.disabled) return;
    const ticket = generation;
    if (!active(ticket)) { reset(); return; }
    try {
      if (button.dataset.evidenceAction === 'play') await onPlay(response.entries[index], response, { ...getContext() });
      else { index += button.dataset.evidenceAction === 'next' ? 1 : -1; await reveal(ticket); }
    } catch (error) { if (active(ticket)) onError(error); }
  };
  container.addEventListener('click', click); panel.addEventListener('click', panelClick);
  return { reset, update() { if (anchor && !active(generation)) reset(); }, destroy() { reset(); destroyed = true; container.removeEventListener('click', click); panel.removeEventListener('click', panelClick); } };
}
