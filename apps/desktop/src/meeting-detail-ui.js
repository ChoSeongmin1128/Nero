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
    const current = memory();
    current.scroll[current.section] = body.scrollTop;
    current.notesScroll = editor.scrollTop;
  }
  function displayDocumentActions(section){
    for(const kind of ['summary','transcript']){const node=byId(`${kind}-document-actions`);if(node)node.hidden=section!==kind;}
  }
  function display() {
    const current = memory(), notesOpen = Boolean(current?.notesOpen);
    if (current) {
      const previous = current.section;
      if (current.section === 'summary' && !summaryVisible && transcriptVisible) current.section = 'transcript';
      else if (current.section === 'transcript' && !transcriptVisible && summaryVisible) current.section = 'summary';
      if (current.section !== previous) pendingRestore = true;
    }
    summary.hidden = !meetingId || !summaryVisible || current?.section !== 'summary';
    transcript.hidden = !meetingId || !transcriptVisible || current?.section !== 'transcript';
    displayDocumentActions(!meetingId?null:current?.section==='transcript'&&!transcript.hidden?'transcript':!summary.hidden?'summary':null);
    byId('result-transcript-home').hidden = transcript.hidden;
    inspector.hidden = !meetingId || !notesOpen;
    width.update(!inspector.hidden);
    notes.hidden = inspector.hidden;
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
      body.scrollTop = current.scroll[current.section];
      editor.scrollTop = current.notesScroll;
      pendingRestore = false;
    }
  }
  function update({ id, recording = false, summaryLoaded = false, transcriptLoaded = false, hasTranscript = false, hasPreview = false, hasAudio = false, showSummary = true }) {
    if (id !== meetingId) { meetingId = id; request++; pendingRestore = true; }
    if (id && !memories.has(id)) memories.set(id, { section: 'summary', notesOpen: false, explicitNotes: false, recordingSeen: false, scroll: { summary: 0, transcript: 0 }, notesScroll: 0 });
    summaryVisible = showSummary;
    transcriptVisible = hasAudio || hasTranscript || hasPreview || recording;
    ready = summaryLoaded && transcriptLoaded;
    const current = memory();
    if (current && recording && !current.recordingSeen) {
      rememberScroll();
      current.recordingSeen = true;
      current.section = 'transcript';
      pendingRestore = true;
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
        if (view === 'summary' && !summaryVisible || view === 'transcript' && !transcriptVisible) return false;
        rememberScroll();
        const current = memory();
        if (view === 'notes') { current.notesOpen = toggle ? !current.notesOpen : true; current.explicitNotes = true; }
        else if (current.section !== view) { current.section = view; pendingRestore = true; }
        display();
        onReveal(view);
        if (view !== 'notes') {
          const section = view === 'summary' ? summary : transcript;
          if (focus) section.focus({ preventScroll: true });
        } else if (focus) {
          const preview = byId('notes-search-preview');
          const target = preview && !preview.hidden ? preview : editor;
          (current.notesOpen ? target : controls.get(view))?.focus({ preventScroll: true });
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
  return { update, select, rememberScroll, active: () => memory()?.section || 'summary', notesOpen: () => Boolean(memory()?.notesOpen), inspectorOpen: () => Boolean(memory()?.notesOpen) };
}
