export function createMeetingSearch({ search, apply, delay = 250, schedule = setTimeout, clear = clearTimeout }) {
  let generation = 0, timer;
  return {
    update(query, includeDeleted = false) {
      const request = ++generation;
      clear(timer);
      if (!query.trim()) { apply({ query: '', ids: null, hits: [], unavailable: 0, pending: false }); return; }
      apply({ query, ids: null, hits: [], unavailable: 0, pending: true });
      timer = schedule(async () => {
        try {
          const result = await search(query, includeDeleted);
          if (request === generation) apply({ query, ids: result.ids, hits: result.hits || [], unavailable: result.unavailable, pending: false });
        } catch {
          if (request === generation) apply({ query, ids: null, hits: [], unavailable: 1, pending: false });
        }
      }, delay);
    },
    reset() { generation++; clear(timer); },
  };
}

export function literalOffsets(text, query) {
  const needle = query.trim();
  if (!needle) return [];
  const escaped = needle.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  return [...text.matchAll(new RegExp(escaped, 'giu'))].map(match => match.index);
}

export function mountDocumentSearch({ byId, selectView, renderContent, ready = () => true, prepare = () => true, onError = () => {} }) {
  const field = byId('document-query'), container = byId('document-search');
  let opened = false, query = '', matches = [], current = -1, generation = 0;
  function refresh() {
    container.hidden = !opened;
    byId('document-search-toggle').setAttribute('aria-expanded', String(opened));
    byId('inspector-search-toggle')?.setAttribute('aria-expanded', String(opened));
    const key = matches[current]?.key;
    matches = [];
    if (opened && query) {
      for (const view of ['transcript', 'summary', 'notes']) {
        const owner = byId({ transcript: 'transcript-rows', summary: 'summary-content', notes: 'notes-search-preview' }[view]);
        const occurrences = new Map(), groups = new Map();
        const offsets = view === 'notes' ? literalOffsets(byId('meeting-notes').value || '', query) : [];
        [...owner.querySelectorAll('mark.search-match')].forEach((node, index) => {
          const row = node.closest('[data-transcript-row]')?.dataset.transcriptRow;
          const scope = `${view}:${row ?? 'document'}`;
          const occurrence = occurrences.get(scope) || 0;
          occurrences.set(scope, occurrence + 1);
          const group = view === 'summary' ? node.dataset.searchGroup : null;
          if (group && groups.has(group)) { groups.get(group).nodes.push(node); return; }
          const match = { view, node, nodes: [node], row, offset: offsets[index], key: `${scope}:${group ? 'group-' + group : occurrence}` };
          if (group) groups.set(group, match);
          matches.push(match);
        });
      }
    }
    current = key ? matches.findIndex(match => match.key === key) : -1;
    const count = byId('document-search-count');
    count.textContent = !query ? '' : !ready() ? '불러오는 중' : matches.length ? `${current + 1} / ${matches.length}` : '일치하는 내용 없음';
    count.setAttribute('aria-label', matches.length ? `${matches.length}개 중 ${current + 1}번째` : count.textContent);
    for (const id of ['document-search-prev', 'document-search-next']) byId(id).disabled = !matches.length || !ready();
    matches.forEach((match, index) => match.nodes.forEach(node => node.classList?.toggle('is-current', index === current)));
  }
  async function go(index) {
    const target = matches[index];
    if (!target || !ready()) return false;
    const intent = ++generation;
    if (!await selectView(target.view) || intent !== generation) return false;
    refresh();
    current = matches.findIndex(match => match.key === target.key);
    const selected = matches[current];
    if (!selected) return false;
    matches.forEach((match, i) => match.nodes.forEach(node => node.classList?.toggle('is-current', i === current)));
    selected.node.scrollIntoView({ block: 'center', behavior: 'auto' });
    if (selected.view === 'notes' && selected.offset != null) byId('meeting-notes').setSelectionRange?.(selected.offset, selected.offset + query.length);
    byId('document-search-count').textContent = `${current + 1} / ${matches.length}`;
    byId('document-search-count').setAttribute('aria-label', `${matches.length}개 중 ${current + 1}번째`);
    return true;
  }
  const step = direction => go((current + direction + matches.length) % matches.length);
  async function change() {
    if (field.composing) return;
    query = field.value.trim(); current = -1; matches = [];
    const request = ++generation;
    try {
      if (!await prepare() || request !== generation) return;
      renderContent(); refresh();
      await go(0);
    } catch (error) { if (request === generation) onError(error); }
  }
  function close(focus = true) {
    const notesScroll = byId('notes-search-preview').scrollTop;
    generation++; opened = false; query = ''; field.value = '';
    renderContent(); refresh();
    byId('meeting-notes').scrollTop = notesScroll;
    if (focus) byId(byId('detail-inspector')?.hidden === false ? 'inspector-search-toggle' : 'document-search-toggle').focus({ preventScroll: true });
  }
  const toggle = async () => {
    if (opened) return close();
    const request = ++generation;
    try {
      if (!await prepare() || request !== generation) return;
      opened = true; refresh(); field.focus({ preventScroll: true });
    } catch (error) { if (request === generation) onError(error); }
  };
  byId('document-search-toggle').addEventListener('click', toggle);
  byId('inspector-search-toggle')?.addEventListener('click', toggle);
  byId('document-search-close').addEventListener('click', () => close());
  byId('document-search-prev').addEventListener('click', () => { void step(-1); });
  byId('document-search-next').addEventListener('click', () => { void step(1); });
  field.addEventListener('compositionstart', () => { field.composing = true; });
  field.addEventListener('compositionend', () => { field.composing = false; change(); });
  field.addEventListener('input', change);
  field.addEventListener('keydown', event => {
    if (event.isComposing || field.composing) return;
    if (event.key === 'Escape') { event.preventDefault(); close(); }
    if (event.key === 'Enter') { event.preventDefault(); void step(event.shiftKey ? -1 : 1); }
  });
  return {
    query: () => opened ? query : '', refresh,
    reset() { generation++; opened = false; query = ''; matches = []; current = -1; field.value = ''; refresh(); },
    async find(value, { view, index } = {}) {
      opened = true; query = value.trim(); field.value = query;
      renderContent(); refresh(); field.focus({ preventScroll: true });
      const target = matches.findIndex(match => match.view === view && (index == null || match.row === String(index)));
      return go(target < 0 ? 0 : target);
    },
  };
}
