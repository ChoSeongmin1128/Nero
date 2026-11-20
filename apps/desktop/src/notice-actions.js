// Carries out what a notice's button asks and keeps how far it got, for every screen that
// shows the notice: the notification list and home run the same action and show the same state.
// A failure is shown beside the notice's button, not again as an app-wide error.
export function createNoticeActions({ handlers }) {
  const pending = new Map(), errors = new Map(), notes = new Map(), runs = new Map();
  const listeners = new Set();
  // A screen that fails to redraw must not leave the action marked as running or stop the
  // other screens from redrawing.
  const changed = () => { for (const listener of listeners) try { listener(); } catch (error) { console.error(error); } };
  const message = error => (error instanceof Error ? error.message : String(error ?? '')) || '작업을 끝내지 못했습니다.';
  return {
    state: id => ({ pending: pending.get(id) || null, error: errors.get(id) || '', note: notes.get(id) || '' }),
    busy: id => pending.has(id),
    // `context` tells the handler where the action was asked, such as where Settings returns focus.
    async run(notice, action, context = {}) {
      const handler = Object.hasOwn(handlers, action) ? handlers[action] : null;
      if (!notice || typeof handler !== 'function' || pending.has(notice.id)) return;
      const run = {};
      runs.set(notice.id, run);
      pending.set(notice.id, action); errors.delete(notice.id); notes.delete(notice.id); changed();
      let note = '', error = '';
      try {
        const result = await handler(notice, context);
        if (typeof result === 'string') note = result;
      } catch (failure) { error = message(failure); }
      finally {
        // A notice that left while its action ran may be back as a new one; the old result is not its.
        if (runs.get(notice.id) === run) {
          runs.delete(notice.id);
          if (note) notes.set(notice.id, note);
          if (error) errors.set(notice.id, error);
        }
        pending.delete(notice.id); changed();
      }
    },
    // A notice that went away takes its note, its error and the result still on its way with it.
    keep(ids) {
      const live = new Set(ids);
      let removed = false;
      for (const id of [...runs.keys()]) if (!live.has(id)) runs.delete(id);
      for (const map of [errors, notes]) for (const id of [...map.keys()]) if (!live.has(id)) { map.delete(id); removed = true; }
      if (removed) changed();
    },
    subscribe(listener) { listeners.add(listener); return () => listeners.delete(listener); },
  };
}
