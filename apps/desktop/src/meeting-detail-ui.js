const views = ['summary', 'transcript', 'notes'];

export function mountMeetingDetail({ byId, finishEditing, onReveal = () => {}, onError = () => {} }) {
  const panels = new Map(views.map(view => [view, byId(`${view}-section`)]));
  const scrollers = new Map(views.map(view => [view, byId(view === 'transcript' ? 'transcript-scroll' : `${view}-section`)]));
  const controls = new Map(['transcript', 'notes'].map(view => [view, byId(`detail-${view}-open`)]));
  const inspector = byId('detail-inspector'), close = byId('detail-inspector-close');
  const notesEditor = byId('meeting-notes'), memories = new Map();
  let meetingId = null, request = 0, renderedId = null, renderedView = null;
  let summaryVisible = true, notesInline = false, requestedInline = false;
  let ready = {}, pendingRestore = new Set(views);
  const memory = () => memories.get(meetingId);
  function rememberScroll() {
    if (renderedId !== meetingId || !meetingId) return;
    for (const view of views) if (ready[view] && !pendingRestore.has(view) && !panels.get(view).hidden) memory().scroll[view] = scrollers.get(view).scrollTop;
    if (!panels.get('notes').hidden) memory().scroll.notesEditor = notesEditor.scrollTop;
  }
  function display({ reveal = false, focus = false } = {}) {
    const view = memory()?.view || 'summary', changed = renderedId !== meetingId || renderedView !== view;
    const inline = Boolean(meetingId && notesInline && view !== 'transcript');
    const home = byId('draft-notes-home');
    home.hidden = !inline;
    const notes = panels.get('notes'), notesDestination = inline ? home : inspector;
    if (notes.parentNode !== notesDestination) notesDestination.append(notes);
    notes.querySelector('h2')?.classList.toggle('sr-only', !inline);
    byId('meeting-detail').dataset.inlineNotes = String(inline);
    controls.get('notes').hidden = notesInline;
    panels.get('summary').hidden = !meetingId || !summaryVisible;
    inspector.hidden = !meetingId || inline || view === 'summary';
    byId('meeting-detail').dataset.inspector = inspector.hidden ? '' : view;
    for (const name of ['transcript', 'notes']) {
      panels.get(name).hidden = !meetingId || (name === 'notes' && inline ? false : name !== view);
      controls.get(name).setAttribute('aria-expanded', String(!inspector.hidden && name === view));
    }
    const search = byId('document-search'), destination = inspector.hidden ? byId('detail-search-home') : inspector;
    if (search.parentNode !== destination) {
      const focused = search.ownerDocument?.activeElement;
      if (inspector.hidden) destination.append(search); else inspector.insertBefore(search, byId('summary-evidence'));
      if (search.contains(focused)) focused.focus({ preventScroll: true });
    }
    byId('detail-inspector-title').textContent = view === 'notes' ? '메모' : '음성 기록';
    renderedId = meetingId; renderedView = view;
    if (!meetingId) return;
    if (changed) pendingRestore.add(view);
    if (changed || reveal) onReveal(view);
    for (const name of views) if (ready[name] && pendingRestore.has(name) && !panels.get(name).hidden) {
      scrollers.get(name).scrollTop = memory().scroll[name];
      if (name === 'notes') notesEditor.scrollTop = memory().scroll.notesEditor;
      pendingRestore.delete(name);
    }
    if (focus) (view === 'summary' ? controls.get(memory().lastPanel || 'transcript') : close).focus({ preventScroll: true });
  }
  function update({ id, recording = false, summaryLoaded = false, transcriptLoaded = false, hasSummary = false, hasTranscript = false, showSummary = true, inlineNotes = false }) {
    close.setAttribute('aria-label', hasSummary ? '요약으로 돌아가기' : '본문으로 돌아가기');
    const back = close.querySelector('.inspector-back');
    if (back) back.textContent = hasSummary ? '요약으로' : '본문으로';
    const sameMeeting = id === meetingId;
    const editingInline = sameMeeting && notesInline && notesEditor.ownerDocument?.activeElement === notesEditor;
    if (!sameMeeting) { meetingId = id; request++; pendingRestore = new Set(views); }
    requestedInline = inlineNotes;
    summaryVisible = showSummary; notesInline = inlineNotes || editingInline;
    ready = { summary: summaryLoaded, transcript: transcriptLoaded, notes: true };
    for (const view of views) if (!ready[view]) pendingRestore.add(view);
    if (id && !memories.has(id)) memories.set(id, { view: recording && !hasSummary && !hasTranscript ? 'notes' : 'summary', explicit: false, recordingConsidered: false, lastPanel: 'transcript', scroll: { summary: 0, transcript: 0, notes: 0, notesEditor: 0 } });
    const current = memory();
    if (current && !recording && current.recordingConsidered && !current.explicit && current.view === 'notes') {
      rememberScroll(); current.view = 'summary';
    }
    if (current && recording && !current.recordingConsidered && summaryLoaded && transcriptLoaded) {
      current.recordingConsidered = true;
      if (!current.explicit && !hasSummary && !hasTranscript) { rememberScroll(); current.view = 'notes'; current.lastPanel = 'notes'; }
    }
    display();
  }
  async function select(view, { focus = false } = {}) {
    if (!views.includes(view) || !meetingId) return false;
    const id = meetingId, intent = ++request;
    memory().explicit = true;
    try {
      return Boolean(await finishEditing(() => {
        if (id !== meetingId || intent !== request) return false;
        rememberScroll(); memory().view = view;
        if (view !== 'summary') memory().lastPanel = view;
        display({ reveal: true, focus }); return true;
      }));
    } catch (error) { onError(error); return false; }
  }
  for (const view of views) scrollers.get(view).addEventListener('scroll', rememberScroll);
  notesEditor.addEventListener('scroll', rememberScroll);
  notesEditor.addEventListener('blur', () => { if (notesInline && !requestedInline) { rememberScroll(); notesInline = false; display(); } });
  for (const [view, control] of controls) control.addEventListener('click', () => { void select(view, { focus: true }); });
  close.addEventListener('click', () => { void select('summary', { focus: true }); });
  inspector.addEventListener('keydown', event => {
    if (event.key === 'Escape' && !event.defaultPrevented && !event.target.closest?.('textarea, input, [open]')) { event.preventDefault(); void select('summary', { focus: true }); }
  });
  return { update, select, rememberScroll, active: () => memory()?.view || 'summary' };
}
