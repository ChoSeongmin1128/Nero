// Carries out what a notice's button asks and keeps how far it got, for every screen that
// shows the notice: the notification list and home run the same action and show the same state.
// A failure is shown beside the notice's button, not again as an app-wide error.
export function createNoticeActions({ handlers }) {
  const pending = new Map(), errors = new Map(), notes = new Map();
  const listeners = new Set();
  const changed = () => { for (const listener of listeners) listener(); };
  const message = error => error instanceof Error ? error.message : String(error);
  return {
    state: id => ({ pending: pending.get(id) || null, error: errors.get(id) || '', note: notes.get(id) || '' }),
    busy: id => pending.has(id),
    async run(notice, action) {
      const handler = handlers[action];
      if (!notice || !handler || pending.has(notice.id)) return;
      pending.set(notice.id, action); errors.delete(notice.id); notes.delete(notice.id); changed();
      try {
        const note = await handler(notice);
        if (typeof note === 'string' && note) notes.set(notice.id, note);
      } catch (error) { errors.set(notice.id, message(error)); }
      finally { pending.delete(notice.id); changed(); }
    },
    // A notice that went away takes its note and error with it.
    keep(ids) {
      const live = new Set(ids);
      let removed = false;
      for (const map of [errors, notes]) for (const id of [...map.keys()]) if (!live.has(id)) { map.delete(id); removed = true; }
      if (removed) changed();
    },
    subscribe(listener) { listeners.add(listener); return () => listeners.delete(listener); },
  };
}
