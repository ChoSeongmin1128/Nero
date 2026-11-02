const emptyConversation = () => ({ id: null, messages: [], activeRequestId: null, draft: '', includeMeeting: true, sources: [], error: '', feedback: '', action: null, loading: false, loaded: false });
const errorMessage = (error, fallback) => error?.message || fallback;
const terminal = new Set(['completed', 'failed', 'cancelled']);

export function createAiChatFlow({ bridge, beforeWrite = async () => {}, onSaved = async () => {}, getTemplate = async () => null, createId = () => crypto.randomUUID() }) {
  let state = { meeting: null, conversation: emptyConversation(), provider: 'chatgpt', accountId: null, modelId: null, models: [], accounts: [], accountsLoaded: false, accountsBusy: false, accountsError: '', modelsBusy: false, modelError: '', attempts: {}, connectionPhases: {}, notion: null, notionBusy: false, notionAttempt: null, notionError: '', fetching: false, sourceError: '', active: false, settingsActive: false };
  let subscriptionReady = false;
  let disposed = false, accountsPromise = null, modelEpoch = 0, sourceRequest = null, unsubscribe = null;
  const conversations = new Map(), loads = new Map(), listeners = new Set(), connections = new Map();
  const publish = patch => { if (disposed) return; state = { ...state, ...patch }; listeners.forEach(listener => listener()); };
  const updateConversation = (id, patch) => {
    const value = { ...(conversations.get(id) || emptyConversation()), ...patch };
    conversations.set(id, value);
    if (state.meeting?.id === id) publish({ conversation: value });
    return value;
  };
  const account = () => state.accounts.find(item => item.id === state.accountId && item.provider === state.provider);
  const ready = () => account()?.state === 'ready';
  const acceptAccounts = result => {
    const accounts = result.accounts || [];
    const selected = accounts.find(item => item.id === state.accountId && item.provider === state.provider)
      || accounts.find(item => item.provider === state.provider && item.state === 'ready')
      || accounts.find(item => item.provider === state.provider);
    publish({ accounts, accountsLoaded: true, accountId: selected?.id || null });
    void flow.loadModels();
  };
  const connectionError = (key, error, fallback) => publish({ [key === 'notion' ? 'notionError' : 'accountsError']: errorMessage(error, fallback) });
  const connectionPhase = (key, phase) => publish({ connectionPhases: { ...state.connectionPhases, [key]: phase } });
  const connectionAttempt = (key, attemptId) => publish(key === 'notion' ? { notionAttempt: attemptId } : { attempts: { ...state.attempts, [key]: attemptId } });
  const acceptConnection = (key, result) => { if (key === 'notion') { if (result) publish({ notion: result }); } else if (result) acceptAccounts(result); };
  async function finishConnection(key) {
    const run = connections.get(key);
    if (!run?.attemptId || ['waiting', 'cancelling'].includes(state.connectionPhases[key])) return;
    const generation = ++run.generation;
    connectionPhase(key, 'waiting');
    connectionError(key, null, '');
    try {
      const result = await (key === 'notion' ? bridge.notionFinish(run.attemptId) : bridge.finishConnect(run.attemptId));
      if (disposed || connections.get(key) !== run || run.generation !== generation) return;
      acceptConnection(key, result);
      connections.delete(key);
      connectionAttempt(key, null);
      connectionPhase(key, null);
    } catch (error) {
      if (disposed || connections.get(key) !== run || run.generation !== generation) return;
      connectionPhase(key, 'failed');
      connectionError(key, error, '연결을 완료하지 못했습니다. 다시 시도해 주세요.');
    }
  }
  async function startConnection(key) {
    if (connections.has(key)) return;
    const run = { generation: 0, attemptId: null, begin: null };
    connections.set(key, run);
    connectionPhase(key, 'starting');
    connectionError(key, null, '');
    run.begin = Promise.resolve().then(() => key === 'notion' ? bridge.notionConnect() : bridge.connect(key));
    try {
      const result = await run.begin;
      run.attemptId = result.attemptId;
      if (disposed || connections.get(key) !== run || state.connectionPhases[key] === 'cancelling') return;
      connectionAttempt(key, result.attemptId);
      await finishConnection(key);
    } catch (error) {
      if (disposed || connections.get(key) !== run || state.connectionPhases[key] === 'cancelling') return;
      connections.delete(key);
      connectionPhase(key, null);
      connectionError(key, error, '연결을 시작하지 못했습니다. 다시 시도해 주세요.');
    }
  }
  async function cancelConnection(key) {
    const run = connections.get(key);
    if (!run || state.connectionPhases[key] === 'cancelling') return;
    run.generation += 1;
    connectionPhase(key, 'cancelling');
    connectionError(key, null, '');
    try {
      const started = await run.begin.catch(() => null);
      const attemptId = run.attemptId || started?.attemptId;
      const result = attemptId ? await (key === 'notion' ? bridge.notionCancel(attemptId) : bridge.cancelConnect(attemptId)) : null;
      if (disposed || connections.get(key) !== run) return;
      acceptConnection(key, result);
      connections.delete(key);
      connectionAttempt(key, null);
      connectionPhase(key, null);
    } catch (error) {
      if (disposed || connections.get(key) !== run) return;
      connectionAttempt(key, run.attemptId);
      connectionPhase(key, 'failed');
      connectionError(key, error, '연결 취소를 확인하지 못했습니다. 다시 시도해 주세요.');
    }
  }
  const receive = event => {
    if (!event?.meetingId || !event.requestId) return;
    const conversation = conversations.get(event.meetingId);
    if (!conversation || conversation.activeRequestId !== event.requestId || (conversation.id && event.conversationId !== conversation.id)) return;
    const messages = [...conversation.messages];
    let index = messages.findIndex(item => item.requestId === event.requestId && item.role === 'assistant');
    if (index < 0) index = messages.findIndex(item => item.role === 'assistant' && item.status === 'streaming');
    if (index < 0) return;
    const previous = messages[index];
    if (terminal.has(previous.status)) return;
    const status = event.type === 'delta' ? 'streaming' : event.type;
    if (!['streaming', ...terminal].includes(status)) return;
    messages[index] = { ...previous, id: event.messageId || previous.id, text: event.type === 'delta' ? previous.text + (event.text || '') : event.text ?? previous.text, status, citations: event.citations || previous.citations || [], error: event.error || '' };
    updateConversation(event.meetingId, { id: event.conversationId || conversation.id, messages, activeRequestId: terminal.has(status) ? null : event.requestId, stopping: false, error: event.type === 'failed' ? event.error || '답변을 받지 못했습니다. 다시 시도해 주세요.' : '' });
  };
  const flow = {
    getSnapshot: () => state,
    subscribe(listener) { listeners.add(listener); return () => listeners.delete(listener); },
    receive,
    canSend: () => Boolean(subscriptionReady && state.meeting && state.conversation.loaded && ready() && state.modelId && !state.modelsBusy && !state.modelError && !state.conversation.loading && !state.conversation.activeRequestId && !state.conversation.action && !state.fetching && state.conversation.draft.trim()),
    setMeeting(meeting) {
      const next = meeting ? { id: meeting.id, title: meeting.title } : null;
      if (next?.id === state.meeting?.id) { if (next?.title !== state.meeting?.title) publish({ meeting: next }); return; }
      if (sourceRequest) void flow.cancelSource();
      publish({ meeting: next, conversation: next ? conversations.get(next.id) || emptyConversation() : emptyConversation(), sourceError: '' });
      if (next && !conversations.has(next.id)) conversations.set(next.id, state.conversation);
      if (state.active && next) void flow.loadConversation();
    },
    setActive(active) {
      if (active === state.active) return;
      publish({ active });
      if (active) { void flow.loadAccounts(); void flow.loadConversation(); }
    },
    setSettingsActive(settingsActive) {
      if (settingsActive === state.settingsActive) return;
      publish({ settingsActive });
      if (settingsActive) { void flow.loadAccounts(true); void flow.loadNotion(); }
    },
    async loadAccounts(force = false) {
      if ([...connections.keys()].some(key => key !== 'notion')) return;
      if (accountsPromise) return accountsPromise;
      if (state.accountsLoaded && !force) return;
      publish({ accountsBusy: true, accountsError: '' });
      accountsPromise = (async () => {
        try { acceptAccounts(await bridge.accounts()); }
        catch (error) { publish({ accountsError: errorMessage(error, 'AI 계정을 확인하지 못했습니다.') }); }
        finally { accountsPromise = null; publish({ accountsBusy: false }); }
      })();
      return accountsPromise;
    },
    selectProvider(provider) {
      if (!['chatgpt', 'claude'].includes(provider) || state.conversation.activeRequestId) return;
      const selected = state.accounts.find(item => item.provider === provider && item.state === 'ready') || state.accounts.find(item => item.provider === provider);
      publish({ provider, accountId: selected?.id || null, models: [], modelId: null, modelError: '' });
      void flow.loadModels();
    },
    async loadModels() {
      if (disposed) return;
      const epoch = ++modelEpoch;
      if (!ready()) { publish({ models: [], modelId: null, modelsBusy: false, modelError: '' }); return; }
      const accountId = state.accountId;
      publish({ modelsBusy: true, modelError: '' });
      try {
        const result = await bridge.models(accountId);
        if (epoch !== modelEpoch || disposed) return;
        const models = result.models || [];
        const modelId = models.some(item => item.id === state.modelId) ? state.modelId : models.some(item => item.id === result.defaultModelId) ? result.defaultModelId : models[0]?.id || null;
        publish({ models, modelId, modelError: models.length ? '' : '사용할 수 있는 모델이 없습니다.' });
      } catch (error) { if (epoch === modelEpoch) publish({ models: [], modelId: null, modelError: errorMessage(error, '모델을 불러오지 못했습니다.') }); }
      finally { if (epoch === modelEpoch) publish({ modelsBusy: false }); }
    },
    selectModel(modelId) { if (!state.conversation.activeRequestId && state.models.some(item => item.id === modelId)) publish({ modelId }); },
    connect(provider) {
      const target = state.accounts.find(item => item.provider === provider);
      if (state.accountsBusy || !target?.configured || target.state !== 'signed_out') return;
      return startConnection(provider);
    },
    finishConnect: provider => finishConnection(provider),
    cancelConnect: provider => cancelConnection(provider),
    async disconnect(accountId) {
      if (state.accountsBusy) return;
      if ([...conversations.values()].some(item => item.activeRequestId)) { publish({ accountsError: '답변이 끝난 뒤 연결을 해제할 수 있습니다.' }); return; }
      publish({ accountsBusy: true, accountsError: '' });
      try { acceptAccounts(await bridge.disconnect(accountId)); }
      catch (error) { publish({ accountsError: errorMessage(error, '연결을 해제하지 못했습니다.') }); }
      finally { publish({ accountsBusy: false }); }
    },
    async loadConversation(force = false) {
      const id = state.meeting?.id;
      if (!id || loads.has(id) || conversations.get(id)?.activeRequestId || (conversations.get(id)?.loaded && !force)) return;
      const epoch = {};
      loads.set(id, epoch);
      updateConversation(id, { loading: true, error: '' });
      try {
        const result = await bridge.read(id);
        if (disposed || loads.get(id) !== epoch) return;
        const current = conversations.get(id);
        updateConversation(id, { id: result.id, messages: result.messages || [], activeRequestId: result.activeRequestId || null, loaded: true, loading: false, error: '', draft: current.draft });
      } catch (error) { if (!disposed) updateConversation(id, { loading: false, error: errorMessage(error, '대화를 불러오지 못했습니다.') }); }
      finally { loads.delete(id); }
    },
    edit(draft) { if (state.meeting) updateConversation(state.meeting.id, { draft, useTemplate: false }); },
    summaryPrompt() { if (state.meeting) updateConversation(state.meeting.id, { draft: '이 회의의 결정 사항과 할 일을 정리해 주세요.', useTemplate: true }); },
    includeMeeting(includeMeeting) { if (state.meeting && !state.conversation.activeRequestId) updateConversation(state.meeting.id, { includeMeeting }); },
    removeSource(sourceId) { if (state.meeting && !state.conversation.activeRequestId) updateConversation(state.meeting.id, { sources: state.conversation.sources.filter(item => item.sourceId !== sourceId) }); },
    async send(text = state.conversation.draft, options = {}) {
      const id = state.meeting?.id;
      if (!subscriptionReady || !id || !state.conversation.loaded || !ready() || !state.modelId || state.modelsBusy || state.modelError || state.fetching || state.conversation.loading || state.conversation.activeRequestId || state.conversation.action || !text.trim()) return false;
      const requestId = createId(), accountId = state.accountId, modelId = state.modelId;
      const context = state.conversation;
      updateConversation(id, { action: 'sending', error: '', feedback: '' });
      try {
        await beforeWrite();
        const templateId = options.templateId || (context.useTemplate ? await getTemplate() : null);
        if (state.meeting?.id !== id || disposed) { updateConversation(id, { action: null }); return false; }
        const messages = [...context.messages, { id: `user:${requestId}`, role: 'user', text: text.trim(), status: 'completed', templateId }, { id: `assistant:${requestId}`, requestId, role: 'assistant', text: '', status: 'streaming', citations: [] }];
        updateConversation(id, { messages, draft: '', useTemplate: false, action: null, activeRequestId: requestId });
        const receipt = await bridge.send({ requestId, conversationId: context.id, meetingId: id, includeMeeting: context.includeMeeting, accountId, modelId, message: text.trim(), sourceIds: context.sources.map(item => item.sourceId), ...(templateId ? { templateId } : {}) });
        if (!disposed) updateConversation(id, { id: receipt.conversationId });
        return true;
      } catch (error) {
        const current = conversations.get(id);
        if (current.messages.some(item => item.requestId === requestId && terminal.has(item.status))) return false;
        const message = errorMessage(error, '전송하지 못했습니다. 다시 시도해 주세요.');
        updateConversation(id, { activeRequestId: null, action: null, error: message, messages: current.messages.map(item => item.requestId === requestId && item.role === 'assistant' ? { ...item, status: 'failed', error: message } : item) });
        return false;
      }
    },
    retry(messageId) {
      const index = state.conversation.messages.findIndex(item => item.id === messageId && item.role === 'assistant' && ['failed', 'cancelled', 'interrupted'].includes(item.status));
      if (index < 1) return false;
      const prompt = state.conversation.messages.slice(0, index).reverse().find(item => item.role === 'user');
      return prompt ? flow.send(prompt.text, { templateId: prompt.templateId }) : false;
    },
    async stop() {
      const id = state.meeting?.id, requestId = state.conversation.activeRequestId;
      if (!requestId || state.conversation.stopping) return;
      updateConversation(id, { stopping: true, error: '' });
      try { await bridge.cancel(requestId); }
      catch (error) { updateConversation(id, { stopping: false, error: errorMessage(error, '중지를 확인하지 못했습니다. 다시 시도해 주세요.') }); }
    },
    async save(kind, messageId) {
      const id = state.meeting?.id, conversation = state.conversation;
      const message = conversation.messages.find(item => item.id === messageId);
      if (!id || !conversation.id || conversation.activeRequestId || conversation.action || message?.role !== 'assistant' || message.status !== 'completed' || !message.text.trim()) return false;
      updateConversation(id, { action: `${kind}:${messageId}`, error: '', feedback: '' });
      try {
        await beforeWrite();
        if (state.meeting?.id !== id || disposed) return false;
        const payload = { conversationId: conversation.id, messageId, meetingId: id };
        await (kind === 'summary' ? bridge.saveSummary(payload) : bridge.appendNote(payload));
        await onSaved(kind, id);
        updateConversation(id, { feedback: kind === 'summary' ? '요약을 저장했습니다.' : '메모에 추가했습니다.' });
        return true;
      } catch (error) { updateConversation(id, { error: errorMessage(error, '저장하지 못했습니다. 다시 시도해 주세요.') }); return false; }
      finally { updateConversation(id, { action: null }); }
    },
    async loadNotion() {
      if (connections.has('notion')) return;
      if (state.notionBusy) return;
      publish({ notionBusy: true, notionError: '' });
      try { publish({ notion: await bridge.notionSnapshot() }); }
      catch (error) { publish({ notionError: errorMessage(error, 'Notion 연결을 확인하지 못했습니다.') }); }
      finally { publish({ notionBusy: false }); }
    },
    connectNotion() {
      if (state.notionBusy) return;
      return startConnection('notion');
    },
    finishNotion: () => finishConnection('notion'),
    cancelNotion: () => cancelConnection('notion'),
    async disconnectNotion() {
      if (state.notionBusy || state.fetching) return;
      publish({ notionBusy: true, notionError: '' });
      try { publish({ notion: await bridge.notionDisconnect() }); }
      catch (error) { publish({ notionError: errorMessage(error, 'Notion 연결을 해제하지 못했습니다.') }); }
      finally { publish({ notionBusy: false }); }
    },
    async addSource(link) {
      const id = state.meeting?.id;
      if (!id || state.fetching || state.conversation.activeRequestId || !link.trim()) return false;
      const requestId = createId();
      sourceRequest = requestId;
      publish({ fetching: true, sourceError: '' });
      try {
        const result = await bridge.fetchSources({ requestId, links: [link.trim()] });
        if (disposed || sourceRequest !== requestId || state.meeting?.id !== id) return false;
        if (!result.sources?.length) throw new Error('자료를 찾지 못했습니다. 링크를 확인해 주세요.');
        const sources = new Map(conversations.get(id).sources.map(item => [item.pageId || item.sourceId, item]));
        for (const source of result.sources || []) sources.set(source.pageId || source.sourceId, source);
        updateConversation(id, { sources: [...sources.values()] });
        return true;
      } catch (error) { if (sourceRequest === requestId) publish({ sourceError: errorMessage(error, '자료를 불러오지 못했습니다. 링크와 연결을 확인해 주세요.') }); return false; }
      finally { if (sourceRequest === requestId) { sourceRequest = null; publish({ fetching: false }); } }
    },
    async cancelSource() {
      const requestId = sourceRequest;
      if (!requestId) return;
      sourceRequest = null;
      publish({ fetching: false });
      try { await bridge.cancelFetch(requestId); }
      catch (error) { publish({ sourceError: errorMessage(error, '자료 불러오기 취소를 확인하지 못했습니다.') }); }
    },
    dispose() { disposed = true; modelEpoch++; listeners.clear(); unsubscribe?.(); if (sourceRequest) void bridge.cancelFetch(sourceRequest).catch(() => {}); },
  };
  Promise.resolve().then(() => bridge.subscribe(receive)).then(stop => { if (disposed) stop?.(); else { unsubscribe = stop; subscriptionReady = true; publish({}); } }).catch(error => publish({ accountsError: errorMessage(error, '답변 수신을 준비하지 못했습니다.') }));
  return flow;
}
