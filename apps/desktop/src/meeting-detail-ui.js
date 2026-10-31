import { mountNotesWidth } from './notes-width.js';

const views = ['summary', 'transcript', 'notes'];

export function mountMeetingDetail({ byId, finishEditing, onReveal = () => {}, onError = () => {} }) {
  const body = byId('detail-body-scroll');
  const summary = byId('summary-section'), transcript = byId('transcript-section');
  const inspector = byId('detail-inspector'), notes = byId('notes-section');
  const editor = byId('meeting-notes'), close = byId('detail-inspector-close');
  const width = mountNotesWidth({ root: byId('meeting-detail'), handle: byId('notes-resize') });
  const controls = new Map(views.map(view => [view, byId(`detail-${view}-open`)]));
  const memories = new Map();
  let meetingId = null, renderedId = null, request = 0, ready = false, pendingRestore = true;
  let summaryVisible = true, transcriptVisible = false;
  const memory = () => memories.get(meetingId);
  function rememberScroll() {
    if (!meetingId || renderedId !== meetingId || pendingRestore || !ready) return;
    Object.assign(memory(), { scroll: body.scrollTop, notesScroll: editor.scrollTop });
  }
  function display() {
    const current = memory(), notesOpen = Boolean(current?.notesOpen);
    summary.hidden = !meetingId || !summaryVisible;
    transcript.hidden = !meetingId || !transcriptVisible;
    byId('result-transcript-home').hidden = transcript.hidden;
    inspector.hidden = !meetingId || !notesOpen;
    width.update(!inspector.hidden);
    notes.hidden = inspector.hidden;
    byId('meeting-detail').dataset.inspector = inspector.hidden ? '' : 'notes';
    for (const [view, control] of controls) {
      if (!control) continue;
      control.hidden = !meetingId;
      if (view === 'notes') control.setAttribute('aria-expanded', String(notesOpen));
      else {
        control.disabled = view === 'transcript' && !transcriptVisible || view === 'summary' && !summaryVisible;
        control.setAttribute('aria-pressed', String(current?.section === view));
      }
    }
    renderedId = meetingId;
    if (current && ready && pendingRestore) {
      body.scrollTop = current.scroll;
      editor.scrollTop = current.notesScroll;
      pendingRestore = false;
    }
  }
  function update({ id, recording = false, summaryLoaded = false, transcriptLoaded = false, hasTranscript = false, hasPreview = false, hasAudio = false, showSummary = true }) {
    if (id !== meetingId) { meetingId = id; request++; pendingRestore = true; }
    if (id && !memories.has(id)) memories.set(id, { section: 'summary', notesOpen: false, explicitNotes: false, recordingSeen: false, scroll: 0, notesScroll: 0 });
    summaryVisible = showSummary;
    transcriptVisible = hasAudio || hasTranscript || hasPreview || recording;
    ready = summaryLoaded && transcriptLoaded;
    const current = memory();
    if (current && recording && !current.recordingSeen) {
      current.recordingSeen = true;
      if (!current.explicitNotes) current.notesOpen = true;
    }
    display();
  }
  async function select(view, { focus = false, toggle = false } = {}) {
    if (!views.includes(view) || !meetingId) return false;
    const id = meetingId, intent = ++request;
    try {
      return Boolean(await finishEditing(() => {
        if (id !== meetingId || intent !== request) return false;
        rememberScroll();
        const current = memory();
        if (view === 'notes') { current.notesOpen = toggle ? !current.notesOpen : true; current.explicitNotes = true; }
        else current.section = view;
        display();
        onReveal(view);
        if (view !== 'notes') {
          const section = view === 'summary' ? summary : transcript;
          const reduced = body.ownerDocument?.defaultView?.matchMedia?.('(prefers-reduced-motion: reduce)')?.matches;
          section.scrollIntoView?.({ block: 'start', behavior: reduced ? 'instant' : 'smooth' });
          if (focus) section.focus({ preventScroll: true });
        } else if (focus) {
          const preview = byId('notes-search-preview');
          (current.notesOpen ? preview && !preview.hidden ? preview : editor : controls.get('notes')).focus({ preventScroll: true });
        }
        return true;
      }));
    } catch (error) { onError(error); return false; }
  }
  body.addEventListener('scroll', rememberScroll);
  editor.addEventListener('scroll', rememberScroll);
  for (const [view, control] of controls) control?.addEventListener('click', () => { void select(view, { focus: true, toggle: view === 'notes' }); });
  close.addEventListener('click', () => { if (memory()?.notesOpen) void select('notes', { toggle: true, focus: true }); });
  inspector.addEventListener('keydown', event => {
    if (event.key === 'Escape' && !event.defaultPrevented && !event.isComposing && !event.target.closest?.('[role="listbox"],[role="menu"],[open]')) {
      event.preventDefault(); if (memory()?.notesOpen) void select('notes', { toggle: true, focus: true });
    }
  });
  return { update, select, rememberScroll, active: () => memory()?.section || 'summary', notesOpen: () => Boolean(memory()?.notesOpen) };
}
