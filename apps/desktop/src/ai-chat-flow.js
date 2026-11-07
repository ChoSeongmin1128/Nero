import { conversationPath, parentOf, questionVersions, ROOT } from './ai-chat-versions.js';
const DRAFT = 'draft';
export const temporaryMessageId = id => /^(?:user|assistant):/.test(String(id || ''));
const emptyConversation = id => ({ id: id || null, key: id || DRAFT, title: '새 대화', messages: [], activeRequestId: null, draft: '', templateId: null, skillId: null, summaryTargetId: null, meetings: [], sources: [], excludedSourceKeys: [], contextBusy: false, compositionInitialized: false, error: '', feedback: '', action: null, loading: false, loaded: !id });
const errorMessage = (error, fallback) => error?.message || fallback;
const terminal = new Set(['completed', 'failed', 'cancelled']);
const reconnectable = new Set(['signed_out', 'plan_disabled', 'unknown']);
export const usableAccount = account => Boolean(account?.id) && ['ready', 'unknown'].includes(account.state);
const cleanModel = item => ({ id: item.id, label: item.label, ...(item.previous === true ? { previous: true } : {}), ...(item.effort && typeof item.effort === 'object' ? { effort: { supported: typeof item.effort.supported === 'boolean' ? item.effort.supported : null, levels: Array.isArray(item.effort.levels) ? item.effort.levels.filter(level => typeof level === 'string') : [], default: typeof item.effort.default === 'string' ? item.effort.default : null } } : {}) });
const allowedEffort = (models, modelId, effort) => effort && models.find(item => item.id === modelId)?.effort?.supported === true && models.find(item => item.id === modelId).effort.levels.includes(effort) ? effort : null;
export const effectiveEffort = (models, modelId, effort) => allowedEffort(models, modelId, effort) || allowedEffort(models, modelId, models.find(item => item.id === modelId)?.effort?.default);

export function selectAiAccount(accounts, provider, { selectedId = null, activeAccountId = null } = {}) {
  const candidates = accounts.filter(account => account.provider === provider);
  return candidates.find(account => account.id === selectedId && usableAccount(account))
    || candidates.find(account => account.id === activeAccountId && usableAccount(account))
    || candidates.find(account => account.state === 'ready')
    || candidates.find(account => account.state === 'unknown')
    || candidates.find(account => account.id === activeAccountId)
    || candidates.find(account => account.id === selectedId)
    || candidates[0];
}

export function createAiChatFlow({ bridge, beforeWrite = async () => {}, onSaved = async () => {}, getTemplate = async () => null, getMeetings = () => [], writeClipboard = text => navigator.clipboard.writeText(text), createId = () => crypto.randomUUID() }) {
  let state = { meeting: null, conversation: emptyConversation(), conversationList: [], trashList: [], trashLoaded: false, trashError: '', listBusy: false, listLoaded: false, listError: '', provider: 'chatgpt', accountId: null, activeAccountId: null, modelId: null, effort: null, models: [], accounts: [], accountsLoaded: false, accountsBusy: false, accountsError: '', catalog: { skills: [], templates: [] }, catalogError: '', modelGroups: {}, modelsBusy: false, modelError: '', attempts: {}, connectionPhases: {}, notion: null, notionBusy: false, notionAttempt: null, notionError: '', fetching: false, sourceError: '', active: false, settingsActive: false };
  let subscriptionReady = false, selectionInitialized = false, contextEpoch = 0, openEpoch = 0;
  let disposed = false, accountsPromise = null, listPromise = null, modelEpoch = 0, listEpoch = 0, sourceRequest = null, unsubscribe = null;
  const conversations = new Map([[DRAFT, state.conversation]]), loads = new Map(), listeners = new Set(), connections = new Map(), eventBuffers = new Map(), pendingStarts = new Map(), contextLoads = new Map(), modelCache = new Map(), modelGenerations = new Map(), refreshedModels = new Set(), passiveModels = new Set();
  try {
    const saved = JSON.parse(globalThis.localStorage?.getItem('nero.ai.models.v1') || '{}');
    for (const [id, value] of Object.entries(saved.accounts || {})) if (Array.isArray(value.models)) modelCache.set(id, { models: value.models.filter(item => typeof item.id === 'string' && typeof item.label === 'string').map(cleanModel), defaultModelId: value.defaultModelId });
    if (saved.selection && typeof saved.selection.modelId === 'string') state = { ...state, ...saved.selection };
  } catch {}
  const persistModels = () => { try { globalThis.localStorage?.setItem('nero.ai.models.v1', JSON.stringify({ accounts: Object.fromEntries(modelCache), selection: { provider: state.provider, accountId: state.accountId, modelId: state.modelId, effort: state.effort } })); } catch {} };
  const publish = patch => { if (disposed) return; state = { ...state, ...patch }; listeners.forEach(listener => listener()); };
  const pickModel = (models, defaultModelId, current = state.modelId) => [current, defaultModelId].find(id => id && models.some(item => item.id === id)) || models[0]?.id || null;
  const storeModels = (accountId, result, patch = {}) => {
    const models = (result.models || []).map(cleanModel);
    modelCache.set(accountId, { models, defaultModelId: result.defaultModelId });
    const modelId = pickModel(models, result.defaultModelId);
    publish({ ...patch, ...(state.accountId === accountId ? { models, modelId, effort: allowedEffort(models, modelId, state.effort) } : {}) });
    persistModels();
  };
  const invalidateModels = accountId => {
    if (!accountId) return;
    modelGenerations.set(accountId, (modelGenerations.get(accountId) || 0) + 1);
    modelCache.delete(accountId);
    refreshedModels.delete(accountId);
    passiveModels.delete(accountId);
    const modelGroups = { ...state.modelGroups };
    delete modelGroups[accountId];
    const patch = { modelGroups, ...(state.provider === 'claude' && state.accountId === accountId ? { modelId: null, models: [], modelsBusy: false, modelError: '' } : {}) };
    modelEpoch += 1;
    if (disposed) state = { ...state, ...patch }; else publish(patch);
    persistModels();
  };
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
  const sourceKey = source => source.pageId || source.sourceId;
  const chooseSource = (source, meetings) => {
    const selected = source.directSource || meetings.map(meeting => source.meetingSources?.[meeting.id]).find(Boolean);
    return selected ? { ...selected, directSource: source.directSource, meetingSources: source.meetingSources || {} } : null;
  };
  function loadMeetingSources(id, meetingId) {
    if (typeof bridge.meetingSources !== 'function') return;
    const pending = contextLoads.get(id) || new Map(), token = {};
    pending.set(meetingId, token); contextLoads.set(id, pending);
    updateConversation(id, { contextBusy: true });
    Promise.resolve().then(() => bridge.meetingSources(meetingId)).then(result => {
      const current = conversations.get(id);
      if (disposed || !current || contextLoads.get(id) !== pending || pending.get(meetingId) !== token || current.action === 'deleting' || !current.meetings.some(item => item.id === meetingId)) return;
      const sources = new Map(current.sources.map(source => [sourceKey(source), source]));
      for (const source of result.sources || []) {
        const key = sourceKey(source);
        if (!source.sourceId || !key || current.excludedSourceKeys?.includes(key)) continue;
        const existing = sources.get(key);
        if (existing?.unknownOrigin) continue;
        const meetingSources = { ...(existing?.meetingSources || {}), [meetingId]: source };
        sources.set(key, chooseSource({ ...existing, meetingSources }, current.meetings));
      }
      updateConversation(id, { sources: [...sources.values()] });
    }).catch(error => {
      if (!disposed && conversations.has(id) && contextLoads.get(id) === pending && pending.get(meetingId) === token) updateConversation(id, { error: errorMessage(error, '회의 자료를 불러오지 못했습니다.') });
    }).finally(() => {
      if (contextLoads.get(id) !== pending || pending.get(meetingId) !== token) return;
      pending.delete(meetingId);
      if (!pending.size) contextLoads.delete(id);
      if (!disposed && conversations.has(id)) updateConversation(id, { contextBusy: pending.size > 0 });
    });
  }
  const availableMeetings = () => (getMeetings() || []).map(meetingChoice).filter(Boolean);
  const attachmentSelection = attachments => {
    const allowed = new Map(availableMeetings().map(meeting => [meeting.id, meeting]));
    const meetings = (attachments || []).filter(item => item.kind === 'meeting' && allowed.has(item.meetingId)).map(item => allowed.get(item.meetingId)), sources = [];
    for (const item of attachments || []) {
      if (item.kind === 'notion' && item.sourceId) {
        const snapshot = { sourceId: item.sourceId, pageId: item.pageId, title: item.title, url: item.url };
        const origin = item.origin;
        const meetingSources = Object.fromEntries(Object.entries(origin?.meetingSources || {}).filter(([id]) => allowed.has(id)).map(([id, sourceId]) => [id, { ...snapshot, sourceId }]));
        const directSource = origin?.directSourceId ? { ...snapshot, sourceId: origin.directSourceId } : null;
        const restored = origin && chooseSource({ directSource, meetingSources }, meetings);
        sources.push(restored?.sourceId === snapshot.sourceId ? restored : { ...snapshot, unknownOrigin: true });
      }
    }
    return { meetings, sources, compositionInitialized: true };
  };
  const attachmentsFor = context => [
    ...context.meetings.map(meeting => ({ kind: 'meeting', meetingId: meeting.id, title: meeting.title })),
    ...context.sources.map(source => ({ kind: 'notion', sourceId: source.sourceId, pageId: source.pageId, title: source.title, url: source.url })),
  ];
  const account = () => state.accounts.find(item => item.id === state.accountId && item.provider === state.provider);
  const ready = () => usableAccount(account());
  const acceptAccounts = result => {
    const accounts = result.accounts || [];
    const activeAccountId = result.activeAccountId || null;
    const usableProvider = provider => usableAccount(selectAiAccount(accounts, provider, { selectedId: provider === state.provider ? state.accountId : null, activeAccountId }));
    const provider = usableProvider(state.provider) ? state.provider : ['chatgpt', 'claude'].find(usableProvider) || state.provider;
    const selected = selectAiAccount(accounts, provider, { selectedId: provider === state.provider ? state.accountId : null, activeAccountId });
    const accountId = selected?.id || null;
    const changed = accountId !== state.accountId || !usableAccount(selected);
    if (changed) modelEpoch += 1;
    const cached = usableAccount(selected) && modelCache.get(accountId);
    const models = cached?.models || [];
    const current = accountId === state.accountId ? state.modelId : null;
    // The same account keeps its choice while its models are not loaded yet.
    const modelId = current && !models.length ? current : pickModel(models, cached?.defaultModelId, current);
    publish({ accounts, accountsLoaded: true, provider, accountId, activeAccountId, models, modelId, effort: changed ? null : allowedEffort(models, modelId, state.effort), ...(changed ? { modelsBusy: false, modelError: '' } : {}) });
    // Claude rows come from the app's own cache, so a passive read also replaces labels saved by an earlier version.
    for (const item of accounts.filter(item => item.provider === 'claude' && usableAccount(item) && !refreshedModels.has(item.id) && !passiveModels.has(item.id))) {
      const generation = modelGenerations.get(item.id) || 0;
      passiveModels.add(item.id);
      void bridge.models(item.id, false).then(result => {
        if (disposed || generation !== (modelGenerations.get(item.id) || 0) || refreshedModels.has(item.id) || !state.accounts.some(account => account.id === item.id && usableAccount(account))) return;
        if (result.models?.length) storeModels(item.id, result);
      }).catch(() => {});
    }
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
      if (key !== 'notion') void flow.refreshModelGroup(key, result.activeAccountId);
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
    run.begin = Promise.resolve().then(async () => {
      if (key === 'notion') return bridge.notionConnect();
      const accountId = selectAiAccount(state.accounts, key, { selectedId: state.accountId, activeAccountId: state.activeAccountId })?.id || null;
      const result = await bridge.connect(key, accountId);
      if (key === 'claude' && result?.attemptId) invalidateModels(accountId);
      return result;
    });
    try {
      const result = await run.begin;
      if (result.restored) {
        if (disposed || connections.get(key) !== run || state.connectionPhases[key] === 'cancelling') return;
        connections.delete(key);
        connectionPhase(key, null);
        await flow.loadAccounts(true);
        if (key !== 'notion') void flow.refreshModelGroup(key);
        return;
      }
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
    messages[index] = { ...previous, id: event.messageId || previous.id, requestId: event.requestId, sequence: event.sequence ?? previous.sequence ?? 0, text: event.type === 'delta' ? previous.text + (event.text || '') : event.text ?? previous.text, status, citations: event.citations || previous.citations || [], attachments: event.attachments || previous.attachments || [], saveTargets: event.saveTargets || previous.saveTargets || [], template: event.template || previous.template, actualModelId: event.actualModelId || previous.actualModelId, skillId: event.skillId || previous.skillId, summaryTargetId: event.summaryTargetId || previous.summaryTargetId, error: event.error || '' };
    updateConversation(id, { messages, activeRequestId: terminal.has(status) ? null : event.requestId, stopping: false, error: event.type === 'failed' ? event.error || '답변을 받지 못했습니다. 다시 시도해 주세요.' : '' });
    replayEvents(id);
    if (terminal.has(status)) void flow.loadList(true);
    if (status === 'completed' && messages[index].skillId === 'meeting-summary' && messages[index].summaryTargetId && messages[index].id === event.messageId) {
      const message = messages[index], meetingId = message.summaryTargetId;
      if (message.saveTargets.some(target => target.meetingId === meetingId && target.canSaveSummary === true)) void saveAnswer(id, 'summary', message.id, meetingId);
      else updateConversation(id, { error: '선택한 회의에 요약을 저장할 수 없습니다.' });
    }
  };
  async function saveAnswer(id, kind, messageId, targetMeetingId) {
    const conversation = conversations.get(id);
    const message = conversation?.messages.find(item => item.id === messageId);
    const targets = message?.saveTargets || [];
    const eligibleTargets = kind === 'summary' ? targets.filter(item => item.canSaveSummary === true) : targets;
    const meetingId = targetMeetingId || (eligibleTargets.length === 1 ? eligibleTargets[0].meetingId : null);
    if (!['summary', 'note'].includes(kind) || !id || !conversation || !meetingId || !eligibleTargets.some(item => item.meetingId === meetingId) || conversation.activeRequestId || conversation.action || message?.role !== 'assistant' || message.status !== 'completed' || !message.text.trim()) return false;
    const action = `${kind}:${messageId}`;
    updateConversation(id, { action, error: '', feedback: '' });
    try {
      await beforeWrite(meetingId);
      if (disposed || conversations.get(id)?.action === 'deleting') return false;
      const payload = { conversationId: id, messageId, meetingId };
      const result = await (kind === 'summary' ? bridge.saveSummary(payload) : bridge.appendNote(payload));
      if (kind === 'summary' && (!result?.summaryId || result.applied === false)) throw new Error('요약 저장 결과를 확인하지 못했습니다.');
      updateConversation(id, { feedback: kind === 'summary' ? '요약을 저장했습니다.' : '메모에 추가했습니다.' });
      try { await onSaved(kind, meetingId); } catch { updateConversation(id, { error: '저장한 결과를 표시하지 못했습니다. 회의를 다시 열어 주세요.' }); }
      return true;
    } catch (error) { updateConversation(id, { error: errorMessage(error, '저장하지 못했습니다. 다시 시도해 주세요.') }); return false; }
    finally { if (conversations.get(id)?.action === action) updateConversation(id, { action: null }); }
  }
  // The panel follows the meeting on screen: its latest conversation, or a new one with the
  // meeting attached. An unsent draft waits for its meeting to come back.
  const meetingDrafts = new Map();
  let followEpoch = 0;
  async function followMeeting(meeting) {
    const epoch = ++followEpoch;
    const about = id => state.conversationList.find(item => item.id === id)?.meetingIds?.includes(meeting.id);
    if (state.conversation.meetings.some(item => item.id === meeting.id) || about(state.conversation.id)) return;
    // A draft being sent becomes its own conversation first; following waits for that.
    await pendingStarts.get(DRAFT)?.done;
    if (!state.listLoaded) await flow.loadList();
    if (disposed || epoch !== followEpoch || state.meeting?.id !== meeting.id) return;
    const match = state.conversationList.find(item => item.meetingIds?.includes(meeting.id));
    if (match) { await flow.selectConversation(match.id); return; }
    const waiting = conversations.get(DRAFT);
    const restored = waiting && !waiting.action && (waiting.homeMeetingId ?? waiting.meetings?.[0]?.id) === meeting.id ? waiting : meetingDrafts.get(meeting.id);
    meetingDrafts.delete(meeting.id);
    openEpoch += 1; contextEpoch += 1;
    if (sourceRequest) void flow.cancelSource();
    const conversation = restored || { ...emptyConversation(), compositionInitialized: true, homeMeetingId: meeting.id };
    replaceDraft(conversation);
    publish({ conversation, sourceError: '', listError: '' });
    if (!restored) flow.attachMeeting(meeting);
  }
  // An unsent draft is kept for the meeting it was started from before another takes its place.
  const unsent = conversation => Boolean(conversation?.draft?.trim() || conversation?.sources?.length || conversation?.meetings?.length > 1);
  function replaceDraft(next) {
    const current = conversations.get(DRAFT);
    const home = current?.homeMeetingId ?? current?.meetings?.[0]?.id;
    if (current && current !== next && !current.action && home && unsent(current)) meetingDrafts.set(home, current);
    conversations.set(DRAFT, next);
  }
  async function ensureConversation(force = false, epoch = openEpoch) {
    if (selectionInitialized) { const id = state.conversation.id; if (id) await flow.loadConversation(force); return id; }
    await flow.loadList();
    if (listPromise) await listPromise;
    if (disposed || epoch !== openEpoch || !state.active) return null;
    if (!state.listLoaded || state.listError) return null;
    if (selectionInitialized) return state.conversation.id;
    const id = state.meeting ? state.conversationList.find(item => item.meetingIds?.includes(state.meeting.id))?.id : state.conversationList[0]?.id;
    if (id) { await flow.selectConversation(id, { fromOpen: true }); return id; }
    await flow.newConversation({ attachCurrent: true, fromOpen: true });
    return null;
  }
  const canStart = (text, context = state.conversation) => Boolean(subscriptionReady && state.conversation.key && state.conversation.loaded && ready() && state.modelId && !state.modelsBusy && !state.modelError && !state.conversation.loading && !state.conversation.contextBusy && !state.conversation.activeRequestId && !state.conversation.action && !state.fetching && (text.trim() || context.skillId) && (!context.skillId || (context.templateId && context.meetings.length === 1 && context.meetings[0].id === context.summaryTargetId)));
  const flow = {
    getSnapshot: () => state,
    subscribe(listener) { listeners.add(listener); return () => listeners.delete(listener); },
    receive,
    availableMeetings,
    canSend: () => canStart(state.conversation.draft),
    setMeeting(meeting) {
      const next = meetingChoice(meeting), previous = state.meeting;
      if (next?.id === previous?.id && next?.title === previous?.title) return;
      publish({ meeting: next });
      if (next && next.id !== previous?.id && selectionInitialized) void followMeeting(next);
    },
    async open(meeting = state.meeting) {
      const epoch = ++openEpoch;
      flow.setMeeting(meeting);
      publish({ active: true });
      void flow.loadAccounts();
      void flow.loadCatalog();
      await ensureConversation(true, epoch);
    },
    setActive(active) {
      if (active) return state.active ? undefined : flow.open();
      if (!state.active) return;
      contextEpoch += 1; openEpoch += 1;
      publish({ active: false });
    },
    // A conversation an agent changed: the list refreshes, and the shown conversation is
    // read again unless this window is streaming its own answer into it.
    externalChange(conversationIds) {
      void flow.loadList(true);
      const id = state.conversation.id;
      if (id && conversationIds.includes(id) && !state.conversation.activeRequestId && !state.conversation.action) void flow.loadConversation(true);
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
        if (fromOpen && attachCurrent && state.meeting) flow.attachMeeting(state.meeting);
        return null;
      }
      contextEpoch += 1;
      if (sourceRequest) void flow.cancelSource();
      const conversation = { ...(conversations.get(DRAFT) || emptyConversation()), compositionInitialized: true };
      conversations.set(DRAFT, conversation);
      publish({ conversation, sourceError: '', listError: '' });
      if (attachCurrent && state.meeting) flow.attachMeeting(state.meeting);
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
    async loadTrash() {
      try { const result = await bridge.list(true); publish({ trashList: Array.isArray(result) ? result : [], trashLoaded: true, trashError: '' }); }
      catch (error) { publish({ trashError: errorMessage(error, '휴지통을 불러오지 못했습니다.') }); }
    },
    async trashConversations(ids) {
      const targets = [...new Set(ids)].filter(id => id && id !== DRAFT);
      if (!targets.length || targets.some(id => conversations.get(id)?.action)) return false;
      if (targets.includes(sourceRequest?.conversationId)) void flow.cancelSource();
      let trashed;
      try { await Promise.all(targets.map(id => pendingStarts.get(id)?.done)); trashed = await bridge.trash(targets); }
      catch (error) { publish({ listError: errorMessage(error, '대화를 정리하지 못했습니다.') }); void flow.loadList(true); void flow.loadTrash(); return false; }
      const removed = Array.isArray(trashed) ? trashed : targets;
      for (const id of removed) { loads.delete(id); eventBuffers.delete(id); contextLoads.delete(id); conversations.delete(id); }
      listEpoch += 1; listPromise = null;
      const conversationList = state.conversationList.filter(item => !removed.includes(item.id));
      if (removed.includes(state.conversation.id)) {
        contextEpoch += 1;
        const conversation = conversations.get(DRAFT) || { ...emptyConversation(), compositionInitialized: true };
        conversations.set(DRAFT, conversation);
        publish({ conversation, conversationList });
        if (state.meeting) flow.attachMeeting(state.meeting);
      } else publish({ conversationList });
      void flow.loadTrash();
      return true;
    },
    async restoreConversations(ids) {
      try { await bridge.restore(ids); } catch (error) { publish({ trashError: errorMessage(error, '대화를 복원하지 못했습니다.') }); void flow.loadList(true); void flow.loadTrash(); return false; }
      await Promise.all([flow.loadList(true), flow.loadTrash()]);
      return true;
    },
    async purgeConversations(ids) {
      try { await bridge.purge(ids); } catch (error) { publish({ trashError: errorMessage(error, '대화를 완전히 삭제하지 못했습니다.') }); void flow.loadTrash(); return false; }
      await flow.loadTrash();
      return true;
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
        loads.delete(id); eventBuffers.delete(id); contextLoads.delete(id); conversations.delete(id);
        const conversationList = state.conversationList.filter(item => item.id !== id);
        if (state.conversation.id === id) { contextEpoch += 1; const draft = conversations.get(DRAFT) || emptyConversation(); if (!conversations.has(DRAFT)) contextLoads.delete(DRAFT); publish({ conversation: draft, conversationList }); conversations.set(DRAFT, state.conversation); if (conversationList[0]) await flow.selectConversation(conversationList[0].id); }
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
        const latestPrompt = [...conversationPath(result)].reverse().find(message => message.role === 'user');
        const composition = current.compositionInitialized ? {} : attachmentSelection(latestPrompt?.attachments);
        updateConversation(id, { ...result, ...composition, messages: result.messages || [], activeRequestId: result.activeRequestId || null, loaded: true, loading: false, error: '', draft: current.draft });
      } catch (error) { if (!disposed && loads.get(id) === epoch) updateConversation(id, { loading: false, error: errorMessage(error, '대화를 불러오지 못했습니다.') }); }
      finally { if (loads.get(id) === epoch) { loads.delete(id); replayEvents(id); } }
    },
    edit(draft) { if (draft.trim()) selectionInitialized = true; const id = state.conversation.key; updateConversation(id, { draft }); },
    async loadCatalog() {
      if (!bridge.composerCatalog) return;
      try { publish({ catalog: await bridge.composerCatalog(), catalogError: '' }); }
      catch (error) { publish({ catalogError: errorMessage(error, '목록을 불러오지 못했습니다.') }); }
    },
    selectTemplate(templateId) { const meetings = state.conversation.meetings; updateConversation(state.conversation.key, { templateId, useTemplate: false, ...(templateId ? { skillId: 'meeting-summary', summaryTargetId: meetings.length === 1 ? meetings[0].id : null } : {}) }); },
    selectSkill(skillId) { const meetings = state.conversation.meetings; updateConversation(state.conversation.key, { skillId, summaryTargetId: skillId && meetings.length === 1 ? meetings[0].id : null, ...(!skillId ? { templateId: null } : {}) }); if (skillId) void flow.ensureSkillTemplate(); },
    // A summary skill starts with the default template; the chip changes it.
    async ensureSkillTemplate() {
      const key = state.conversation.key;
      if (!state.conversation.skillId || state.conversation.templateId) return;
      // The template chosen for meeting summaries comes first, then the catalog default.
      let preferred = null;
      try { preferred = await getTemplate(); } catch {}
      if (!state.catalog.templates?.length) await flow.loadCatalog();
      const templates = state.catalog.templates || [];
      const templateId = templates.some(item => item.id === preferred) ? preferred : state.catalog.defaultTemplateId || templates.find(item => item.isDefault)?.id || null;
      const current = conversations.get(key);
      if (current?.skillId && !current.templateId && templateId) updateConversation(key, { templateId });
    },
    selectSummaryTarget(summaryTargetId) { if (state.conversation.meetings.length === 1 && state.conversation.meetings[0].id === summaryTargetId) updateConversation(state.conversation.key, { summaryTargetId }); },
    summaryPrompt() { selectionInitialized = true; const id = state.conversation.key; if (id) updateConversation(id, { draft: '첨부한 회의의 결정 사항과 할 일을 정리해 주세요.', useTemplate: true }); },
    attachMeeting(meeting) {
      const next = meetingChoice(meeting), id = state.conversation.key;
      if (!id || !next || state.conversation.action === 'deleting' || state.conversation.meetings.some(item => item.id === next.id)) return false;
      const meetings = [...state.conversation.meetings, next];
      updateConversation(id, { meetings, ...(state.conversation.skillId ? { summaryTargetId: meetings.length === 1 ? meetings[0].id : null } : {}), compositionInitialized: true });
      loadMeetingSources(id, next.id); return true;
    },
    removeMeeting(meetingId) {
      const id = state.conversation.key, current = state.conversation;
      if (!id || current.activeRequestId || current.action) return;
      const pending = contextLoads.get(id); pending?.delete(meetingId); if (pending && !pending.size) contextLoads.delete(id);
      const meetings = current.meetings.filter(item => item.id !== meetingId);
      const sources = current.sources.flatMap(source => {
        if (!source.meetingSources?.[meetingId]) return [source];
        const meetingSources = { ...source.meetingSources };
        delete meetingSources[meetingId];
        const selected = chooseSource({ ...source, meetingSources }, meetings);
        return selected ? [selected] : [];
      });
      updateConversation(id, { meetings, sources, summaryTargetId: current.skillId && meetings.length === 1 ? meetings[0].id : null, contextBusy: Boolean(pending?.size), compositionInitialized: true });
    },
    removeSource(sourceId) {
      const id = state.conversation.key, current = state.conversation;
      if (!id || current.activeRequestId || current.action) return;
      const removed = current.sources.find(item => item.sourceId === sourceId);
      updateConversation(id, { sources: current.sources.filter(item => item.sourceId !== sourceId), excludedSourceKeys: removed ? [...new Set([...(current.excludedSourceKeys || []), sourceKey(removed)])] : current.excludedSourceKeys, compositionInitialized: true });
    },
    async send(text = state.conversation.draft, options = {}) {
      const context = { ...state.conversation, ...(options.editOf ? { useTemplate: false } : {}), meetings: options.meetings || state.conversation.meetings, sources: options.sources || state.conversation.sources, skillId: options.skillId ?? state.conversation.skillId, templateId: options.templateId ?? state.conversation.templateId, summaryTargetId: options.summaryTargetId ?? state.conversation.summaryTargetId };
      if (!canStart(text, context)) return false;
      selectionInitialized = true;
      let id = state.conversation.key;
      const requestId = createId(), accountId = state.accountId, modelId = state.modelId;
      const editOf = options.editOf || null, questionKey = `user:${requestId}`, previousVersions = context.versions || {};
      const requestedEffort = Object.hasOwn(options, 'effort') ? options.effort : state.effort;
      const effort = requestedEffort ? allowedEffort(state.models, modelId, requestedEffort) : effectiveEffort(state.models, modelId, null);
      if (requestedEffort && !effort) { updateConversation(id, { error: '이 모델에서 이전 추론 수준을 사용할 수 없습니다. 모델을 확인해 주세요.' }); return false; }
      let finishStart;
      const starting = { requestId, done: new Promise(resolve => { finishStart = resolve; }) };
      pendingStarts.set(id, starting);
      updateConversation(id, { action: 'sending', error: '', feedback: '' });
      try {
        await beforeWrite();
        const templateId = context.templateId || (context.useTemplate ? await getTemplate() : null);
        const skillId = options.skillId || context.skillId;
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
        // Messages only this window shows (a failed start) never reach the store, so the
        // store's parent is the last stored message on the shown path.
        const shownStored = conversationPath(context).filter(item => !temporaryMessageId(item.id));
        const parentId = editOf ? parentOf(context, editOf) : shownStored.at(-1)?.id || ROOT;
        const versions = editOf ? { ...previousVersions, [parentId]: questionKey } : context.versions;
        const messages = [...context.messages, { id: questionKey, parentId, role: 'user', text: text.trim(), status: 'completed', effort, templateId, skillId, summaryTargetId: context.summaryTargetId, attachments }, { id: `assistant:${requestId}`, parentId: questionKey, requestId, role: 'assistant', text: '', status: 'streaming', modelId, provider: state.provider, effort, skillId, summaryTargetId: context.summaryTargetId, citations: [], attachments, saveTargets }];
        updateConversation(id, { messages, versions, ...(editOf ? {} : { draft: '', useTemplate: false, templateId: null, skillId: null, summaryTargetId: null }), action: null, activeRequestId: requestId });
        const receipt = await bridge.send({ requestId, conversationId: id, meetingIds: context.meetings.map(meeting => meeting.id), accountId, modelId, ...(effort ? { effort } : {}), message: text.trim(), sourceIds: context.sources.map(item => item.sourceId), sourceOrigins: Object.fromEntries(context.sources.filter(item => !item.unknownOrigin).map(item => [item.sourceId, { directSourceId: item.directSource?.sourceId || null, meetingSources: Object.fromEntries(Object.entries(item.meetingSources || {}).map(([meetingId, source]) => [meetingId, source.sourceId])) }])), ...(templateId ? { templateId } : {}), ...(skillId ? { skillId, summaryTargetId: context.summaryTargetId } : {}), ...(editOf ? { editOf } : {}), parentId });
        if (receipt.conversationId !== id) throw new Error('대화 응답을 확인하지 못했습니다.');
        // The shown question keeps its temporary id until the store's id arrives, so it can
        // be edited right after the answer without reloading the conversation.
        if (receipt.questionId) {
          const current = conversations.get(id), swap = value => value === questionKey ? receipt.questionId : value;
          if (current) updateConversation(id, { messages: current.messages.map(item => ({ ...item, id: swap(item.id), ...(item.parentId ? { parentId: swap(item.parentId) } : {}) })), versions: Object.fromEntries(Object.entries(current.versions || {}).map(([key, value]) => [key, swap(value)])) });
        }
        void flow.loadList(true);
        return true;
      } catch (error) {
        const current = conversations.get(id);
        if (!current || current.messages.some(item => item.requestId === requestId && terminal.has(item.status))) return false;
        const message = errorMessage(error, '전송하지 못했습니다. 다시 시도해 주세요.');
        // A refused edit leaves the store on the earlier version, so the window returns to it.
        if (editOf) { updateConversation(id, { activeRequestId: null, action: null, error: message, versions: previousVersions, messages: current.messages.filter(item => item.requestId !== requestId && item.id !== questionKey) }); return false; }
        updateConversation(id, { activeRequestId: null, action: null, error: message, messages: current.messages.map(item => item.requestId === requestId && item.role === 'assistant' ? { ...item, status: 'failed', error: message } : item) });
        return false;
      } finally {
        if (pendingStarts.get(id) === starting) pendingStarts.delete(id);
        finishStart();
      }
    },
    editQuestion(messageId, text) {
      const conversation = state.conversation;
      if (conversation.activeRequestId || conversation.action || !String(text || '').trim()) return false;
      const prompt = conversationPath(conversation).find(item => item.id === messageId && item.role === 'user');
      if (!prompt || temporaryMessageId(prompt.id)) return false;
      const selection = attachmentSelection(prompt.attachments);
      if ((prompt.attachments || []).filter(item => item.kind === 'meeting').length !== selection.meetings.length) { updateConversation(conversation.key, { error: '삭제된 회의를 제외하고 다시 질문해 주세요.' }); return false; }
      return flow.send(text, { editOf: messageId, templateId: prompt.templateId || prompt.template?.id || '', skillId: prompt.skillId || '', summaryTargetId: prompt.summaryTargetId || '', ...selection });
    },
    async selectVersion(messageId) {
      const conversation = state.conversation, id = conversation.id;
      if (!id || conversation.activeRequestId || conversation.action || questionVersions(conversation, messageId).siblings.length < 2) return false;
      const parent = parentOf(conversation, messageId), before = (conversation.versions || {})[parent];
      updateConversation(id, { versions: { ...(conversation.versions || {}), [parent]: messageId }, action: 'version', error: '' });
      try {
        const result = await bridge.selectVersion(id, messageId);
        if (conversations.has(id)) updateConversation(id, { versions: result?.versions || {}, action: null });
        return true;
      } catch (error) {
        if (!conversations.has(id)) return false;
        const versions = { ...(conversations.get(id).versions || {}) };
        if (before === undefined) delete versions[parent]; else versions[parent] = before;
        updateConversation(id, { versions, action: null, error: errorMessage(error, '질문 버전을 바꾸지 못했습니다.') });
        return false;
      }
    },
    retry(messageId) {
      const messages = conversationPath(state.conversation);
      const index = messages.findIndex(item => item.id === messageId && item.role === 'assistant' && ['failed', 'cancelled', 'interrupted'].includes(item.status));
      if (index < 1) return false;
      const prompt = messages.slice(0, index).reverse().find(item => item.role === 'user');
      if (!prompt) return false;
      const selection = attachmentSelection(prompt.attachments);
      if ((prompt.attachments || []).filter(item => item.kind === 'meeting').length !== selection.meetings.length) { updateConversation(state.conversation.key, { error: '삭제된 회의를 제외하고 다시 질문해 주세요.' }); return false; }
      const answer = messages[index];
      if (prompt.effort && (answer.modelId && answer.modelId !== state.modelId || answer.provider && answer.provider !== state.provider)) { updateConversation(state.conversation.key, { error: '이전 추론 수준으로 다시 시도하려면 원래 모델을 선택해 주세요.' }); return false; }
      return flow.send(prompt.text, { effort: prompt.effort || null, templateId: prompt.templateId || prompt.template?.id || answer.template?.id, skillId: prompt.skillId || answer.skillId, summaryTargetId: prompt.summaryTargetId || answer.summaryTargetId, ...selection });
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
    save(kind, messageId, targetMeetingId) { return saveAnswer(state.conversation.id, kind, messageId, targetMeetingId); },
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
    async refreshModelGroup(provider, preferredAccountId = null) {
      const target = selectAiAccount(state.accounts, provider, { selectedId: preferredAccountId || (state.provider === provider ? state.accountId : null), activeAccountId: state.activeAccountId });
      if (!usableAccount(target) || state.modelGroups[target.id]?.busy) return;
      const accountId = target.id, generation = modelGenerations.get(target.id) || 0;
      publish({ modelGroups: { ...state.modelGroups, [accountId]: { busy: true, error: '' } } });
      try {
        const result = await bridge.models(accountId);
        if (disposed || generation !== (modelGenerations.get(accountId) || 0)) return;
        refreshedModels.add(accountId);
        storeModels(accountId, result, { modelGroups: { ...state.modelGroups, [accountId]: { busy: false, error: '' } } });
      } catch (error) { if (!disposed && generation === (modelGenerations.get(accountId) || 0)) publish({ modelGroups: { ...state.modelGroups, [accountId]: { busy: false, error: errorMessage(error, '모델을 불러오지 못했습니다.') } } }); }
    },
    // Opening the model menu is the request: the cached list shows at once and each
    // account is checked once per session. Nothing else touches credentials passively.
    ensureModels() {
      for (const provider of ['chatgpt', 'claude']) {
        if (connections.has(provider)) continue;
        const target = selectAiAccount(state.accounts, provider, { selectedId: state.provider === provider ? state.accountId : null, activeAccountId: state.activeAccountId });
        if (usableAccount(target) && !refreshedModels.has(target.id)) void flow.refreshModelGroup(provider, target.id);
      }
    },
    modelsFor(provider) {
      const target = selectAiAccount(state.accounts, provider, { selectedId: state.provider === provider ? state.accountId : null, activeAccountId: state.activeAccountId });
      return { account: target, available: usableAccount(target) || Boolean(state.connectionPhases[provider]), models: modelCache.get(target?.id)?.models || [], ...(state.modelGroups[target?.id] || {}) };
    },
    chooseModel(provider, accountId, modelId) {
      if (state.conversation.activeRequestId || !modelCache.get(accountId)?.models.some(item => item.id === modelId)) return;
      publish({ provider, accountId, modelId, effort: state.accountId === accountId && state.modelId === modelId ? allowedEffort(modelCache.get(accountId).models, modelId, state.effort) : null, models: modelCache.get(accountId).models, modelsBusy: false, modelError: '' }); persistModels();
    },
    selectEffort(effort) {
      if (state.conversation.activeRequestId) return;
      const value = effort ? allowedEffort(state.models, state.modelId, effort) : null;
      if (effort && !value) return;
      publish({ effort: value }); persistModels();
    },
    selectProvider(provider, { explicit = false } = {}) {
      if (!['chatgpt', 'claude'].includes(provider) || state.conversation.activeRequestId) return;
      const selected = selectAiAccount(state.accounts, provider, { selectedId: state.accountId, activeAccountId: state.activeAccountId });
      publish({ provider, accountId: selected?.id || null, models: [], modelId: null, effort: null, modelError: '' });
      void flow.loadModels({ explicit });
    },
    async loadModels({ explicit = false } = {}) {
      if (disposed) return;
      const epoch = ++modelEpoch;
      if (!ready()) { publish({ models: [], modelId: null, modelsBusy: false, modelError: '' }); return; }
      const accountId = state.accountId;
      const cached = modelCache.get(accountId);
      if (!explicit || (account()?.state === 'unknown' && cached?.models?.length)) {
        const models = cached?.models || [];
        const modelId = pickModel(models, cached?.defaultModelId);
        publish({ models, modelId, effort: allowedEffort(models, modelId, state.effort), modelsBusy: false, modelError: '' });
        return;
      }
      publish({ modelsBusy: true, modelError: '' });
      try {
        const result = await bridge.models(accountId);
        if (epoch !== modelEpoch || disposed) return;
        refreshedModels.add(accountId);
        storeModels(accountId, result, { modelError: result.models?.length ? '' : '사용할 수 있는 모델이 없습니다.' });
        if (explicit && account()?.state === 'unknown') {
          try {
            const snapshot = await bridge.accounts();
            if (epoch === modelEpoch && !disposed) {
              acceptAccounts(snapshot);
              if (state.accountId !== accountId) publish({ models: [], modelId: null });
            }
          } catch (error) {
            if (epoch === modelEpoch && !disposed) publish({ accountsError: errorMessage(error, 'AI 계정을 확인하지 못했습니다.') });
          }
        }
      } catch (error) { if (epoch === modelEpoch) { const models = cached?.models || []; const modelId = pickModel(models, cached?.defaultModelId); publish({ models, modelId, effort: allowedEffort(models, modelId, state.effort), modelError: models.length ? '' : errorMessage(error, '모델을 불러오지 못했습니다.'), modelGroups: { ...state.modelGroups, [accountId]: { busy: false, error: errorMessage(error, '모델을 불러오지 못했습니다.') } } }); } }
      finally { if (epoch === modelEpoch) publish({ modelsBusy: false }); }
    },
    selectModel(modelId) { if (!state.conversation.activeRequestId && state.models.some(item => item.id === modelId)) { publish({ modelId, effort: modelId === state.modelId ? state.effort : null }); persistModels(); } },
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
      try { const result = await bridge.disconnect(accountId); modelCache.delete(accountId); acceptAccounts(result); }
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
        for (const source of result.sources || []) sources.set(sourceKey(source), chooseSource({ directSource: source, meetingSources: sources.get(sourceKey(source))?.meetingSources || {} }, conversations.get(id).meetings));
        const restored = new Set(result.sources.map(sourceKey));
        updateConversation(id, { sources: [...sources.values()], excludedSourceKeys: (conversations.get(id).excludedSourceKeys || []).filter(key => !restored.has(key)), compositionInitialized: true }); return true;
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
