const contentOf = value => ({ name: value.name, description: value.description, prompt: value.prompt });
const emptyContent = () => ({ name: '', description: '', prompt: '' });

export function createTemplateSettingsFlow({ bridge }) {
  let epoch = 0;
  let state = { active: false, items: [], document: null, draft: null, busy: false, error: '', message: '' };
  const listeners = new Set();
  const publish = changes => { state = { ...state, ...changes }; listeners.forEach(listener => listener()); };
  const dirty = () => state.draft !== null && JSON.stringify(state.draft) !== JSON.stringify(state.document ? contentOf(state.document) : emptyContent());
  const run = async (operation, apply) => {
    if (!state.active || state.busy) return false;
    const current = ++epoch;
    publish({ busy: true, error: '', message: '' });
    try {
      const result = await operation();
      if (current !== epoch || !state.active) return false;
      publish({ ...apply(result), busy: false });
      return true;
    } catch (error) {
      if (current === epoch && state.active) publish({ busy: false, error: error?.message || '템플릿을 불러오지 못했습니다. 다시 시도해 주세요.' });
      return false;
    }
  };
  const flow = {
    getSnapshot: () => state,
    subscribe(listener) { listeners.add(listener); return () => listeners.delete(listener); },
    isDirty: dirty,
    setActive(active) {
      if (state.active === active) return;
      epoch += 1;
      publish({ active, busy: false });
      if (active && !dirty()) void flow.reload();
    },
    reload() {
      if (state.document) return run(() => bridge.getTemplate(state.document.id), document => ({ document, draft: contentOf(document) }));
      return run(() => bridge.listTemplates(), items => ({ items, draft: null }));
    },
    open(id) {
      if (dirty()) return;
      return run(() => bridge.getTemplate(id), document => ({ document, draft: contentOf(document) }));
    },
    create() { if (!state.busy && !dirty()) publish({ document: null, draft: emptyContent(), error: '', message: '' }); },
    edit(field, value) {
      if (state.busy || !state.draft || !['name', 'description', 'prompt'].includes(field)) return;
      publish({ draft: { ...state.draft, [field]: value }, message: '' });
    },
    discard() { publish({ draft: state.document ? contentOf(state.document) : null, error: '' }); },
    back() {
      if (state.busy || dirty()) return;
      publish({ document: null, draft: null });
      return flow.reload();
    },
    save() {
      if (!state.draft || !state.draft.name.trim() || !state.draft.prompt.trim()) return Promise.resolve(false);
      const content = { ...state.draft };
      const document = state.document;
      return run(() => document ? bridge.updateTemplate(document.id, document.revision, content) : bridge.createTemplate(content), saved => ({ document: saved, draft: contentOf(saved), message: '저장했습니다.' }));
    },
    reset() {
      const document = state.document;
      if (!document?.builtIn) return Promise.resolve(false);
      return run(() => bridge.resetTemplate(document.id, document.revision), saved => ({ document: saved, draft: contentOf(saved), message: '기본값으로 복원했습니다.' }));
    },
    async remove() {
      const document = state.document;
      if (!document || document.builtIn) return false;
      const removed = await run(() => bridge.deleteTemplate(document.id, document.revision), () => ({ document: null, draft: null }));
      if (removed) await flow.reload();
      return removed;
    },
    dispose() { epoch += 1; listeners.clear(); state = { ...state, active: false }; },
  };
  return flow;
}

export function bindTemplateDialogKeys(container, { isOpen, cancel }) {
  const keydown = event => {
    if (event.key !== 'Escape' || !isOpen()) return;
    event.stopImmediatePropagation();
    if (event.isComposing || event.keyCode === 229) return;
    event.preventDefault();
    cancel();
  };
  const onCancel = event => { if (isOpen()) { event.preventDefault(); event.stopImmediatePropagation(); } };
  container.addEventListener('keydown', keydown, true);
  container.addEventListener('cancel', onCancel, true);
  return () => { container.removeEventListener('keydown', keydown, true); container.removeEventListener('cancel', onCancel, true); };
}
