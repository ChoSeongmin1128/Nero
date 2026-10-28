export function createDocumentActionsFlow({ kind, onExport, schedule = setTimeout, cancel = clearTimeout }) {
  let context = { id: null, available: false, busy: false };
  let state = { open: null, pending: false, copied: false, includeCitations: false, error: '' };
  let generation = 0, feedback;
  const listeners = new Set();
  const publish = values => { state = { ...state, ...values }; for (const listener of listeners) listener(); };
  return {
    subscribe(listener) { listeners.add(listener); return () => listeners.delete(listener); },
    getSnapshot: () => state,
    update(next) {
      const changed = next.id !== context.id;
      context = next;
      if (changed) { generation++; cancel(feedback); publish({ open: null, pending: false, copied: false, includeCitations: false, error: '' }); }
      else if ((!next.available || next.busy) && state.open && !state.pending) publish({ open: null });
    },
    open(value) { if (context.available && !context.busy && !state.pending) publish({ open: value, error: '' }); },
    close() { if (!state.pending) publish({ open: null }); },
    include(value) { publish({ includeCitations: kind === 'summary' && Boolean(value) }); },
    async perform(format, copy) {
      if (!context.available || context.busy || state.pending || !['txt', 'md'].includes(format)) return;
      const request = generation, id = context.id;
      publish({ pending: true, error: '', copied: false });
      try {
        const result = await onExport(kind, format, copy, kind === 'summary' && state.includeCitations, id);
        if (generation !== request) return;
        if (result === false) { publish({ pending: false }); return; }
        publish({ open: null, pending: false, copied: copy });
        cancel(feedback);
        if (copy) feedback = schedule(() => { if (request === generation) publish({ copied: false }); }, 1500);
      } catch (error) {
        if (generation === request) publish({ pending: false, error: error.message || '내용을 내보내지 못했습니다.' });
      }
    },
  };
}
