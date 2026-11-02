const DRAFT = 'draft';
const emptyConversation = id => ({ id: id || null, key: id || DRAFT, title: '새 대화', messages: [], activeRequestId: null, draft: '', meetings: [], sources: [], compositionInitialized: false, error: '', feedback: '', action: null, loading: false, loaded: !id });
const errorMessage = (error, fallback) => error?.message || fallback;
const terminal = new Set(['completed', 'failed', 'cancelled']);
const reconnectable = new Set(['signed_out', 'plan_disabled']);

export function selectAiAccount(accounts, provider, { selectedId = null, activeAccountId = null } = {}) {
  const candidates = accounts.filter(account => account.provider === provider);
  return candidates.find(account => account.id === selectedId && account.state === 'ready')
    || candidates.find(account => account.id === activeAccountId && account.state === 'ready')
    || candidates.find(account => account.state === 'ready')
    || candidates.find(account => account.id === activeAccountId)
    || candidates.find(account => account.id === selectedId)
    || candidates[0];
}

export function createAiChatFlow({ bridge, beforeWrite = async () => {}, onSaved = async () => {}, getTemplate = async () => null, getMeetings = () => [], writeClipboard = text => navigator.clipboard.writeText(text), createId = () => crypto.randomUUID() }) {
  let state = { meeting: null, conversation: emptyConversation(), conversationList: [], listBusy: false, listLoaded: false, listError: '', provider: 'chatgpt', accountId: null, activeAccountId: null, modelId: null, models: [], accounts: [], accountsLoaded: false, accountsBusy: false, accountsError: '', modelsBusy: false, modelError: '', attempts: {}, connectionPhases: {}, notion: null, notionBusy: false, notionAttempt: null, notionError: '', fetching: false, sourceError: '', active: false, settingsActive: false };
  let subscriptionReady = false, selectionInitialized = false, contextEpoch = 0, openEpoch = 0;
  let disposed = false, accountsPromise = null, listPromise = null, modelEpoch = 0, listEpoch = 0, sourceRequest = null, unsubscribe = null;
  const conversations = new Map([[DRAFT, state.conversation]]), loads = new Map(), listeners = new Set(), connections = new Map(), eventBuffers = new Map(), pendingStarts = new Map();
  const publish = patch => { if (disposed) return; state = { ...state, ...patch }; listeners.forEach(listener => listener()); };
  const updateConversation = (id, patch) => {
    const value = { ...(conversations.get(id) || emptyConversation(id)), ...patch, id: id === DRAFT ? null : id, key: id };
    conversations.set(id, value);
    const row = state.conversationList.find(item => item.id === id);
    const conversationList = state.conversationList.map(item => item.id === id ? { ...item, title: value.title, messageCount: value.messages.length, activeRequestId: value.activeRequestId } : item);
    if (state.conversation.key === id) publish({ conversation: value, ...(row ? { conversationList } : {}) });
    else if (row) publish({ conversationList });
    return value;
  };
  const meetingChoice = meeting => meeting?.id && !meeting.deletedAt ? { id: meeting.id, title: meeting.title || '제목 없는 회의' } : null;
  const availableMeetings = () => (getMeetings() || []).map(meetingChoice).filter(Boolean);
  const attachmentSelection = attachments => {
    const allowed = new Map(availableMeetings().map(meeting => [meeting.id, meeting]));
    const meetings = [], sources = [];
    for (const item of attachments || []) {
      if (item.kind === 'meeting' && allowed.has(item.meetingId)) meetings.push(allowed.get(item.meetingId));
      if (item.kind === 'notion' && item.sourceId) sources.push({ sourceId: item.sourceId, pageId: item.pageId, title: item.title, url: item.url });
    }
    return { meetings, sources, compositionInitialized: true };
  };
  const attachmentsFor = context => [
    ...context.meetings.map(meeting => ({ kind: 'meeting', meetingId: meeting.id, title: meeting.title })),
    ...context.sources.map(source => ({ kind: 'notion', sourceId: source.sourceId, pageId: source.pageId, title: source.title, url: source.url })),
  ];
  const account = () => state.accounts.find(item => item.id === state.accountId && item.provider === state.provider);
  const ready = () => account()?.state === 'ready';
  const acceptAccounts = result => {
    const accounts = result.accounts || [];
    const activeAccountId = result.activeAccountId || null;
    const selected = selectAiAccount(accounts, state.provider, { selectedId: state.accountId, activeAccountId });
    publish({ accounts, accountsLoaded: true, accountId: selected?.id || null, activeAccountId });
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
    run.begin = Promise.resolve().then(() => key === 'notion' ? bridge.notionConnect() : bridge.connect(key, selectAiAccount(state.accounts, key, { selectedId: state.accountId, activeAccountId: state.activeAccountId })?.id || null));
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
  const bufferEvent = event => {
    let buffer = eventBuffers.get(event.conversationId);
    if (!buffer) { buffer = new Map(); eventBuffers.set(event.conversationId, buffer); }
    buffer.set(`${event.requestId}:${event.sequence ?? buffer.size}`, event);
  };
  function replayEvents(id) {
    const buffer = eventBuffers.get(id);
    if (!buffer || loads.has(id)) return;
    eventBuffers.delete(id);
    for (const event of [...buffer.values()].sort((a, b) => (a.sequence || 0) - (b.sequence || 0))) receive(event);
  }
  const receive = event => {
    const id = event?.conversationId;
    if (disposed || !id || !event.requestId) return;
    if (event.sequence != null && (!Number.isSafeInteger(event.sequence) || event.sequence < 1)) return;
    if (loads.has(id)) { bufferEvent(event); return; }
    const conversation = conversations.get(id);
    if (!conversation || conversation.activeRequestId !== event.requestId) return;
    const messages = [...conversation.messages];
    let index = messages.findIndex(item => item.requestId === event.requestId && item.role === 'assistant');
    if (index < 0) index = messages.findIndex(item => item.role === 'assistant' && item.status === 'streaming');
    if (index < 0) return;
    const previous = messages[index];
    if (terminal.has(previous.status) || (event.sequence != null && event.sequence <= (previous.sequence || 0))) return;
    const status = event.type === 'delta' ? 'streaming' : event.type;
    if (!['streaming', ...terminal].includes(status)) return;
    if (event.type === 'delta' && event.sequence != null && event.sequence > (previous.sequence || 0) + 1) { bufferEvent(event); return; }
    messages[index] = { ...previous, id: event.messageId || previous.id, requestId: event.requestId, sequence: event.sequence ?? previous.sequence ?? 0, text: event.type === 'delta' ? previous.text + (event.text || '') : event.text ?? previous.text, status, citations: event.citations || previous.citations || [], attachments: event.attachments || previous.attachments || [], saveTargets: event.saveTargets || previous.saveTargets || [], error: event.error || '' };
    updateConversation(id, { messages, activeRequestId: terminal.has(status) ? null : event.requestId, stopping: false, error: event.type === 'failed' ? event.error || '답변을 받지 못했습니다. 다시 시도해 주세요.' : '' });
    replayEvents(id);
    if (terminal.has(status)) void flow.loadList(true);
  };
  async function ensureConversation(force = false) {
    if (selectionInitialized) { const id = state.conversation.id; if (id) await flow.loadConversation(force); return id; }
    await flow.loadList();
    if (listPromise) await listPromise;
    if (!state.listLoaded || state.listError) return null;
    if (selectionInitialized) return state.conversation.id;
    const id = state.conversationList[0]?.id;
    if (id) { await flow.selectConversation(id, { fromOpen: true }); return id; }
    await flow.newConversation({ attachCurrent: false, fromOpen: true });
    return null;
  }
  const canStart = text => Boolean(subscriptionReady && state.conversation.key && state.conversation.loaded && ready() && state.modelId && !state.modelsBusy && !state.modelError && !state.conversation.loading && !state.conversation.activeRequestId && !state.conversation.action && !state.fetching && text.trim());
  const flow = {
    getSnapshot: () => state,
    subscribe(listener) { listeners.add(listener); return () => listeners.delete(listener); },
    receive,
    availableMeetings,
    canSend: () => canStart(state.conversation.draft),
    setMeeting(meeting) {
      const next = meetingChoice(meeting);
      if (next?.id === state.meeting?.id && next?.title === state.meeting?.title) return;
      publish({ meeting: next });
    },
    async open(meeting = state.meeting) {
      const epoch = ++openEpoch;
      publish({ active: true });
      void flow.loadAccounts();
      const id = await ensureConversation(true);
      if (!disposed && epoch === openEpoch && state.active && state.conversation.id === id && meeting) flow.attachMeeting(meeting);
    },
    setActive(active) {
      if (active) return state.active ? undefined : flow.open();
      if (!state.active) return;
      contextEpoch += 1; openEpoch += 1;
      publish({ active: false });
    },
    async loadList(force = false) {
      if (listPromise && !force) return listPromise;
      if (state.listLoaded && !force) return;
      const epoch = ++listEpoch;
      publish({ listBusy: true, listError: '' });
      const pending = Promise.resolve().then(async () => {
        try {
          const result = await bridge.list();
          if (!disposed && epoch === listEpoch) {
            const conversationList = Array.isArray(result) ? result : result.conversations || [];
            for (const row of conversationList) {
              const cached = conversations.get(row.id);
              if (cached) conversations.set(row.id, { ...cached, title: row.title || cached.title, createdAt: row.createdAt, updatedAt: row.updatedAt, provider: row.provider, modelId: row.modelId });
            }
            publish({ conversationList, listLoaded: true, conversation: conversations.get(state.conversation.key) || state.conversation });
          }
        } catch (error) { if (!disposed && epoch === listEpoch) publish({ listError: errorMessage(error, '대화 기록을 불러오지 못했습니다.') }); }
        finally { if (epoch === listEpoch) publish({ listBusy: false }); if (listPromise === pending) listPromise = null; }
      });
      listPromise = pending;
      return pending;
    },
    async newConversation({ attachCurrent = true, fromOpen = false } = {}) {
      if (!fromOpen) openEpoch += 1;
      selectionInitialized = true;
      if (state.conversation.key === DRAFT) {
        if (attachCurrent && state.meeting) flow.attachMeeting(state.meeting);
        return null;
      }
      contextEpoch += 1;
      if (sourceRequest) void flow.cancelSource();
      const conversation = { ...(conversations.get(DRAFT) || emptyConversation()), compositionInitialized: true };
      if (attachCurrent && state.meeting && !conversation.meetings.some(item => item.id === state.meeting.id)) conversation.meetings = [...conversation.meetings, state.meeting];
      conversations.set(DRAFT, conversation);
      publish({ conversation, sourceError: '', listError: '' });
      return null;
    },
    async selectConversation(id, { fromOpen = false } = {}) {
      if (!id || id === DRAFT) return false;
      if (!fromOpen) openEpoch += 1;
      selectionInitialized = true;
      contextEpoch += 1;
      if (sourceRequest) void flow.cancelSource();
      const conversation = conversations.get(id) || emptyConversation(id);
      conversations.set(id, conversation);
      publish({ conversation, sourceError: '' });
      await flow.loadConversation(true);
      return state.conversation.id === id;
    },
    async removeConversation(id) {
      const conversation = conversations.get(id);
      if (!id || id === DRAFT || conversation?.action) return false;
      if (sourceRequest?.conversationId === id) void flow.cancelSource();
      updateConversation(id, { action: 'deleting', error: '' });
      try {
        await pendingStarts.get(id)?.done;
        await bridge.remove(id);
        listEpoch += 1; listPromise = null;
        loads.delete(id); eventBuffers.delete(id); conversations.delete(id);
        const conversationList = state.conversationList.filter(item => item.id !== id);
        if (state.conversation.id === id) { contextEpoch += 1; publish({ conversation: emptyConversation(), conversationList }); conversations.set(DRAFT, state.conversation); if (conversationList[0]) await flow.selectConversation(conversationList[0].id); }
        else publish({ conversationList });
        return true;
      } catch (error) { updateConversation(id, { action: null, error: errorMessage(error, '대화를 삭제하지 못했습니다.') }); return false; }
    },
    async loadConversation(force = false) {
      const id = state.conversation.id;
      if (!id || (!force && (loads.has(id) || conversations.get(id)?.activeRequestId || conversations.get(id)?.loaded))) return;
      const epoch = {};
      loads.set(id, epoch);
      updateConversation(id, { loading: true, loaded: false, error: '' });
      try {
        const starting = pendingStarts.get(id);
        if (starting) await starting.done;
        if (disposed || loads.get(id) !== epoch) return;
        const result = await bridge.read(id);
        if (disposed || loads.get(id) !== epoch) return;
        if (result.id !== id) throw new Error('대화를 확인하지 못했습니다.');
        const current = conversations.get(id);
        const latestPrompt = [...(result.messages || [])].reverse().find(message => message.role === 'user');
        const composition = current.compositionInitialized ? {} : attachmentSelection(latestPrompt?.attachments);
        updateConversation(id, { ...result, ...composition, messages: result.messages || [], activeRequestId: result.activeRequestId || null, loaded: true, loading: false, error: '', draft: current.draft });
      } catch (error) { if (!disposed && loads.get(id) === epoch) updateConversation(id, { loading: false, error: errorMessage(error, '대화를 불러오지 못했습니다.') }); }
      finally { if (loads.get(id) === epoch) { loads.delete(id); replayEvents(id); } }
    },
    edit(draft) { if (draft.trim()) selectionInitialized = true; const id = state.conversation.key; updateConversation(id, { draft, useTemplate: false }); },
    summaryPrompt() { selectionInitialized = true; const id = state.conversation.key; if (id) updateConversation(id, { draft: '첨부한 회의의 결정 사항과 할 일을 정리해 주세요.', useTemplate: true }); },
    attachMeeting(meeting) {
      const next = meetingChoice(meeting), id = state.conversation.key;
      if (!id || !next || state.conversation.action === 'deleting' || state.conversation.meetings.some(item => item.id === next.id)) return false;
      updateConversation(id, { meetings: [...state.conversation.meetings, next], compositionInitialized: true }); return true;
    },
    removeMeeting(meetingId) { const id = state.conversation.key; if (id && !state.conversation.activeRequestId && !state.conversation.action) updateConversation(id, { meetings: state.conversation.meetings.filter(item => item.id !== meetingId), compositionInitialized: true }); },
    removeSource(sourceId) { const id = state.conversation.key; if (id && !state.conversation.activeRequestId && !state.conversation.action) updateConversation(id, { sources: state.conversation.sources.filter(item => item.sourceId !== sourceId), compositionInitialized: true }); },
    async send(text = state.conversation.draft, options = {}) {
      if (!canStart(text)) return false;
      selectionInitialized = true;
      let id = state.conversation.key;
      const requestId = createId(), accountId = state.accountId, modelId = state.modelId;
      const context = { ...state.conversation, meetings: options.meetings || state.conversation.meetings, sources: options.sources || state.conversation.sources };
      let finishStart;
      const starting = { requestId, done: new Promise(resolve => { finishStart = resolve; }) };
      pendingStarts.set(id, starting);
      updateConversation(id, { action: 'sending', error: '', feedback: '' });
      try {
        await beforeWrite();
        const templateId = options.templateId || (context.useTemplate ? await getTemplate() : null);
        if (disposed || conversations.get(id)?.action === 'deleting') return false;
        if (id === DRAFT) {
          const created = await bridge.create();
          if (!created?.id || created.id === DRAFT) throw new Error('새 대화를 만들지 못했습니다.');
          const draft = conversations.get(DRAFT);
          conversations.delete(DRAFT);
          pendingStarts.delete(DRAFT);
          id = created.id;
          pendingStarts.set(id, starting);
          const conversation = { ...draft, ...created, key: id, loaded: true, messages: context.messages, action: 'sending' };
          conversations.set(id, conversation);
          if (state.conversation.key === DRAFT) publish({ conversation });
          void flow.loadList(true);
        }
        const attachments = attachmentsFor(context), saveTargets = context.meetings.map(meeting => ({ meetingId: meeting.id, title: meeting.title, canSaveSummary: false }));
        const messages = [...context.messages, { id: `user:${requestId}`, role: 'user', text: text.trim(), status: 'completed', templateId, attachments }, { id: `assistant:${requestId}`, requestId, role: 'assistant', text: '', status: 'streaming', citations: [], attachments, saveTargets }];
        updateConversation(id, { messages, draft: '', useTemplate: false, action: null, activeRequestId: requestId });
        const receipt = await bridge.send({ requestId, conversationId: id, meetingIds: context.meetings.map(meeting => meeting.id), accountId, modelId, message: text.trim(), sourceIds: context.sources.map(item => item.sourceId), ...(templateId ? { templateId } : {}) });
        if (receipt.conversationId !== id) throw new Error('대화 응답을 확인하지 못했습니다.');
        return true;
      } catch (error) {
        const current = conversations.get(id);
        if (!current || current.messages.some(item => item.requestId === requestId && terminal.has(item.status))) return false;
        const message = errorMessage(error, '전송하지 못했습니다. 다시 시도해 주세요.');
        updateConversation(id, { activeRequestId: null, action: null, error: message, messages: current.messages.map(item => item.requestId === requestId && item.role === 'assistant' ? { ...item, status: 'failed', error: message } : item) });
        return false;
      } finally {
        if (pendingStarts.get(id) === starting) pendingStarts.delete(id);
        finishStart();
      }
    },
    retry(messageId) {
      const messages = state.conversation.messages;
      const index = messages.findIndex(item => item.id === messageId && item.role === 'assistant' && ['failed', 'cancelled', 'interrupted'].includes(item.status));
      if (index < 1) return false;
      const prompt = messages.slice(0, index).reverse().find(item => item.role === 'user');
      if (!prompt) return false;
      const selection = attachmentSelection(prompt.attachments);
      if ((prompt.attachments || []).filter(item => item.kind === 'meeting').length !== selection.meetings.length) { updateConversation(state.conversation.key, { error: '삭제된 회의를 제외하고 다시 질문해 주세요.' }); return false; }
      return flow.send(prompt.text, { templateId: prompt.templateId, ...selection });
    },
    async stop() {
      const id = state.conversation.id, requestId = state.conversation.activeRequestId;
      if (!requestId || state.conversation.stopping) return;
      updateConversation(id, { stopping: true, error: '' });
      try { await bridge.cancel(requestId); }
      catch (error) { updateConversation(id, { stopping: false, error: errorMessage(error, '중지를 확인하지 못했습니다. 다시 시도해 주세요.') }); }
    },
    async copy(messageId, format) {
      const id = state.conversation.id, conversation = state.conversation, epoch = contextEpoch;
      const message = conversation.messages.find(item => item.id === messageId);
      if (!state.active || !id || conversation.action || conversation.activeRequestId || !['txt', 'md'].includes(format) || message?.role !== 'assistant' || message.status !== 'completed' || !message.text.trim()) return false;
      const action = `copy:${messageId}:${format}`;
      const owns = () => !disposed && state.active && contextEpoch === epoch && state.conversation.id === id;
      updateConversation(id, { action, error: '', feedback: '', copiedMessageId: null });
      try {
        const document = await bridge.exportAnswer({ conversationId: id, messageId, format });
        if (!owns()) return false;
        if (document.format !== format || typeof document.text !== 'string') throw new Error('복사할 답변을 확인하지 못했습니다.');
        await writeClipboard(document.text);
        if (!owns()) return false;
        updateConversation(id, { feedback: '복사했습니다.', copiedMessageId: messageId }); return true;
      } catch (error) { if (owns()) updateConversation(id, { error: errorMessage(error, '복사하지 못했습니다. 다시 시도해 주세요.') }); return false; }
      finally { if (conversations.get(id)?.action === action) updateConversation(id, { action: null }); }
    },
    async save(kind, messageId, targetMeetingId) {
      const id = state.conversation.id, conversation = state.conversation;
      const message = conversation.messages.find(item => item.id === messageId);
      const targets = message?.saveTargets || [];
      const eligibleTargets = kind === 'summary' ? targets.filter(item => item.canSaveSummary === true) : targets;
      const meetingId = targetMeetingId || (eligibleTargets.length === 1 ? eligibleTargets[0].meetingId : null);
      if (!['summary', 'note'].includes(kind) || !id || !meetingId || !eligibleTargets.some(item => item.meetingId === meetingId) || conversation.activeRequestId || conversation.action || message?.role !== 'assistant' || message.status !== 'completed' || !message.text.trim()) return false;
      updateConversation(id, { action: `${kind}:${messageId}`, error: '', feedback: '' });
      try {
        await beforeWrite(meetingId);
        if (disposed || conversations.get(id)?.action === 'deleting') return false;
        const payload = { conversationId: id, messageId, meetingId };
        await (kind === 'summary' ? bridge.saveSummary(payload) : bridge.appendNote(payload));
        updateConversation(id, { feedback: kind === 'summary' ? '요약을 저장했습니다.' : '메모에 추가했습니다.' });
        try { await onSaved(kind, meetingId); } catch { updateConversation(id, { error: '저장한 결과를 표시하지 못했습니다. 회의를 다시 열어 주세요.' }); }
        return true;
      } catch (error) { updateConversation(id, { error: errorMessage(error, '저장하지 못했습니다. 다시 시도해 주세요.') }); return false; }
      finally { if (conversations.get(id)?.action !== 'deleting') updateConversation(id, { action: null }); }
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
      const selected = selectAiAccount(state.accounts, provider, { selectedId: state.accountId, activeAccountId: state.activeAccountId });
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
      const target = selectAiAccount(state.accounts, provider, { selectedId: state.accountId, activeAccountId: state.activeAccountId });
      if (state.accountsBusy || !target?.configured || !reconnectable.has(target.state)) return;
      return startConnection(provider);
    },
    retryConnect: async provider => { await cancelConnection(provider); if (!connections.has(provider)) await startConnection(provider); },
    cancelConnect: provider => cancelConnection(provider),
    async disconnect(accountId) {
      if (state.accountsBusy) return;
      if ([...conversations.values()].some(item => item.activeRequestId)) { publish({ accountsError: '답변이 끝난 뒤 연결을 해제할 수 있습니다.' }); return; }
      publish({ accountsBusy: true, accountsError: '' });
      try { acceptAccounts(await bridge.disconnect(accountId)); }
      catch (error) { publish({ accountsError: errorMessage(error, '연결을 해제하지 못했습니다.') }); }
      finally { publish({ accountsBusy: false }); }
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
    retryNotion: async () => { await cancelConnection('notion'); if (!connections.has('notion')) await startConnection('notion'); },
    cancelNotion: () => cancelConnection('notion'),
    async disconnectNotion() {
      if (state.notionBusy || state.fetching) return;
      publish({ notionBusy: true, notionError: '' });
      try { publish({ notion: await bridge.notionDisconnect() }); }
      catch (error) { publish({ notionError: errorMessage(error, 'Notion 연결을 해제하지 못했습니다.') }); }
      finally { publish({ notionBusy: false }); }
    },
    async addSource(link) {
      const id = state.conversation.key;
      if (!id || state.fetching || state.conversation.activeRequestId || state.conversation.action || !link.trim()) return false;
      selectionInitialized = true;
      const requestId = createId(), current = { id: requestId, conversationId: id };
      sourceRequest = current;
      publish({ fetching: true, sourceError: '' });
      try {
        const result = await bridge.fetchSources({ requestId, links: [link.trim()] });
        if (disposed || sourceRequest !== current) return false;
        if (!result.sources?.length) throw new Error('자료를 찾지 못했습니다. 링크를 확인해 주세요.');
        const sources = new Map(conversations.get(id).sources.map(item => [item.pageId || item.sourceId, item]));
        for (const source of result.sources || []) sources.set(source.pageId || source.sourceId, source);
        updateConversation(id, { sources: [...sources.values()], compositionInitialized: true }); return true;
      } catch (error) { if (sourceRequest === current) publish({ sourceError: errorMessage(error, '자료를 불러오지 못했습니다. 링크와 연결을 확인해 주세요.') }); return false; }
      finally { if (sourceRequest === current) { sourceRequest = null; publish({ fetching: false }); } }
    },
    async cancelSource() {
      const current = sourceRequest;
      if (!current) return;
      sourceRequest = null;
      publish({ fetching: false });
      try { await bridge.cancelFetch(current.id); }
      catch (error) { if (state.conversation.key === current.conversationId) publish({ sourceError: errorMessage(error, '자료 불러오기 취소를 확인하지 못했습니다.') }); }
    },
    dispose() { disposed = true; modelEpoch++; listEpoch++; listeners.clear(); eventBuffers.clear(); unsubscribe?.(); if (sourceRequest) void bridge.cancelFetch(sourceRequest.id).catch(() => {}); },
  };
  Promise.resolve().then(() => bridge.subscribe(receive)).then(stop => { if (disposed) stop?.(); else { unsubscribe = stop; subscriptionReady = true; publish({}); } }).catch(error => publish({ accountsError: errorMessage(error, '답변 수신을 준비하지 못했습니다.') }));
  return flow;
}
