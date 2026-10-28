import { agentConnectionPrompt } from './agent-connection-prompt.js';

export function bindConnectionDialogKeys(container, { isOpen, close }) {
  const onKeyDown = event => {
    if (event.key !== 'Escape' || event.isComposing || event.keyCode === 229 || !isOpen()) return;
    event.preventDefault();
    event.stopPropagation();
    close();
  };
  container.addEventListener('keydown', onKeyDown, true);
  return () => container.removeEventListener('keydown', onKeyDown, true);
}

export function createAgentConnectionFlow({ writeText }) {
  let request = 0;
  let state = { open: false, prompt: '', copying: false, copied: false, error: '' };
  const listeners = new Set();
  const publish = changes => {
    state = { ...state, ...changes };
    listeners.forEach(listener => listener());
  };
  return {
    getSnapshot: () => state,
    subscribe(listener) { listeners.add(listener); return () => listeners.delete(listener); },
    open(info) {
      request += 1;
      publish({ open: true, prompt: agentConnectionPrompt(info), copying: false, copied: false, error: '' });
    },
    update(info) {
      if (!state.open) return;
      const prompt = agentConnectionPrompt(info);
      if (prompt === state.prompt) return;
      request += 1;
      publish({ prompt, copying: false, copied: false, error: '' });
    },
    close() {
      request += 1;
      if (state.open) publish({ open: false, copying: false, copied: false, error: '' });
    },
    async copy() {
      if (!state.open || state.copying) return;
      const current = ++request;
      const prompt = state.prompt;
      publish({ copying: true, copied: false, error: '' });
      try {
        await writeText(prompt);
        if (current === request && state.open) publish({ copying: false, copied: true });
      } catch {
        if (current === request && state.open) publish({ copying: false, error: '복사하지 못했습니다. 다시 시도해 주세요.' });
      }
    },
  };
}
