const views = ['summary', 'transcript', 'notes'];

export function mountMeetingDetail({ byId, finishEditing, onReveal = () => {}, onError = () => {} }) {
  const tabs = new Map(views.map(view => [view, byId(`detail-${view}-tab`)]));
  const panels = new Map(views.map(view => [view, byId(`${view}-section`)]));
  const notesEditor = byId('meeting-notes');
  const memories = new Map();
  let meetingId = null, request = 0, renderedId = null, renderedView = null;
  let ready = { summary: false, transcript: false, notes: true }, pendingRestore = new Set(views);
  const memory = () => memories.get(meetingId);

  function rememberScroll() {
    if (!meetingId || renderedId !== meetingId || renderedView !== memory()?.view || !ready[renderedView] || pendingRestore.has(renderedView)) return;
    const panel = panels.get(renderedView);
    if (panel && !panel.hidden) {
      memory().scroll[renderedView] = panel.scrollTop;
      if (renderedView === 'notes' && notesEditor) memory().scroll.notesEditor = notesEditor.scrollTop;
    }
  }

  function display({ reveal = false, focus = false } = {}) {
    const view = memory()?.view;
    const changed = renderedId !== meetingId || renderedView !== view;
    for (const name of views) {
      const selected = Boolean(meetingId && view === name);
      const tab = tabs.get(name), panel = panels.get(name);
      panel.hidden = !selected;
      tab.setAttribute('aria-selected', String(selected));
      tab.tabIndex = selected ? 0 : -1;
    }
    renderedId = meetingId;
    renderedView = view;
    if (!meetingId) return;
    if (changed) pendingRestore.add(view);
    const restore = ready[view] && pendingRestore.has(view);
    if (changed || reveal || restore) onReveal(view);
    if (restore) {
      panels.get(view).scrollTop = memory().scroll[view];
      if (view === 'notes' && notesEditor) notesEditor.scrollTop = memory().scroll.notesEditor;
      pendingRestore.delete(view);
    }
    if (focus) tabs.get(view).focus({ preventScroll: true });
  }

  function update({ id, recording = false, summaryLoaded = false, transcriptLoaded = false, hasSummary = false, hasTranscript = false }) {
    if (id !== meetingId) {
      meetingId = id; request += 1;
      pendingRestore = new Set(views);
    }
    ready = { summary: Boolean(summaryLoaded), transcript: Boolean(transcriptLoaded), notes: true };
    for (const view of views) if (!ready[view]) pendingRestore.add(view);
    if (!id) { display(); return; }
    if (!memories.has(id)) memories.set(id, {
      view: recording ? 'notes' : 'summary', explicit: false, resolved: false,
      scroll: { summary: 0, transcript: 0, notes: 0, notesEditor: 0 },
    });
    const current = memory();
    if (!current.resolved && summaryLoaded && transcriptLoaded) {
      current.resolved = true;
      if (!current.explicit) {
        const next = recording ? 'notes' : hasSummary || hasTranscript ? 'summary' : 'notes';
        if (next !== current.view) { rememberScroll(); current.view = next; }
      }
    }
    display();
  }

  async function select(view, { focus = false } = {}) {
    if (!views.includes(view) || !meetingId || tabs.get(view)?.disabled) return false;
    const id = meetingId, intent = ++request;
    memory().explicit = true;
    try {
      return Boolean(await finishEditing(() => {
        if (id !== meetingId || intent !== request) return false;
        rememberScroll();
        memory().view = view;
        display({ reveal: true, focus });
        return true;
      }));
    } catch (error) {
      if (id === meetingId && intent === request && focus) tabs.get(memory().view).focus({ preventScroll: true });
      onError(error);
      return false;
    }
  }

  for (const view of views) {
    const tab = tabs.get(view), panel = panels.get(view);
    tab.setAttribute('aria-controls', panel.id);
    if (!tab.id) tab.id = `detail-tab-${view}`;
    panel.setAttribute('aria-labelledby', tab.id);
    panel.addEventListener('scroll', () => {
      if (meetingId && renderedId === meetingId && memory()?.view === view && !panel.hidden && ready[view] && !pendingRestore.has(view)) memory().scroll[view] = panel.scrollTop;
    });
    tab.addEventListener('click', () => { void select(view, { focus: true }); });
    tab.addEventListener('keydown', event => {
      if (event.isComposing || event.keyCode === 229 || event.altKey || event.ctrlKey || event.metaKey) return;
      const index = views.indexOf(view);
      const next = event.key === 'ArrowRight' ? views[(index + 1) % views.length]
        : event.key === 'ArrowLeft' ? views[(index + views.length - 1) % views.length]
        : event.key === 'Home' ? views[0] : event.key === 'End' ? views.at(-1) : null;
      if (!next) return;
      event.preventDefault();
      void select(next, { focus: true });
    });
  }
  notesEditor?.addEventListener('scroll', () => {
    if (meetingId && renderedId === meetingId && memory()?.view === 'notes' && !panels.get('notes').hidden && !pendingRestore.has('notes')) memory().scroll.notesEditor = notesEditor.scrollTop;
  });

  return { update, select, active: () => memory()?.view || 'summary' };
}
