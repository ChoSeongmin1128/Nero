import assert from 'node:assert/strict';
import test from 'node:test';
import { createAiChatFlow } from '../src/ai-chat-flow.js';
const tick = () => new Promise(resolve => setImmediate(resolve));
const deferred = () => { let resolve, reject; const promise = new Promise((a, b) => { resolve = a; reject = b; }); return { promise, resolve, reject }; };
const readyAccount = { id: 'account', provider: 'chatgpt', state: 'ready', configured: true, label: 'Example' };
const exampleMeetings = [{ id: 'meeting-a', title: 'Example meeting' }, { id: 'meeting-b', title: 'Another meeting' }];
async function harness(overrides = {}, options = {}) {
  const calls = [], events = [], store = new Map([['conversation', { id: 'conversation', title: 'Existing conversation', messages: [], activeRequestId: null }]]);
  let count = 0, conversationCount = 1;
  const summary = value => ({ id: value.id, title: value.title, messageCount: value.messages.length, activeRequestId: value.activeRequestId });
  const bridge = {
    accounts: async () => ({ accounts: [readyAccount, { id: 'claude', provider: 'claude', state: 'unavailable', configured: false }] }),
    models: async () => ({ models: [{ id: 'model', label: 'Model' }], defaultModelId: 'model' }),
    list: async () => [...store.values()].map(summary),
    create: async () => { const value = { id: `conversation-${++conversationCount}`, title: '새 대화', messages: [], activeRequestId: null }; store.set(value.id, value); return structuredClone(value); },
    read: async id => { if (!store.has(id)) throw Error('Conversation missing'); return structuredClone(store.get(id)); },
    remove: async id => { store.delete(id); calls.push(['remove', id]); },
    subscribe: async callback => { events.push(callback); return () => {}; },
    send: async request => {
      calls.push(['send', request]);
      const conversation = store.get(request.conversationId);
      const attachments = request.meetingIds.map(meetingId => ({ kind: 'meeting', meetingId, title: exampleMeetings.find(item => item.id === meetingId)?.title }));
      const saveTargets = attachments.map(item => ({ meetingId: item.meetingId, title: item.title, canSaveSummary: true }));
      conversation.messages.push({ id: `user:${request.requestId}`, role: 'user', status: 'completed', text: request.message, attachments }, { id: `assistant:${request.requestId}`, requestId: request.requestId, role: 'assistant', text: '', status: 'streaming', attachments, saveTargets, sequence: 0 });
      conversation.activeRequestId = request.requestId;
      return { conversationId: request.conversationId, requestId: request.requestId };
    },
    cancel: async requestId => { calls.push(['cancel', requestId]); },
    saveSummary: async request => { calls.push(['summary', request]); return { summaryId: 'summary' }; },
    appendNote: async request => { calls.push(['note', request]); },
    notionSnapshot: async () => ({ connected: true }),
    fetchSources: async request => { calls.push(['source', request]); return { sources: [{ sourceId: 'notion:page:revision', pageId: 'page', title: 'Example source' }] }; },
    cancelFetch: async requestId => { calls.push(['cancelSource', requestId]); },
    ...overrides,
  };
  for (const name of ['accounts', 'finishConnect', 'cancelConnect', 'disconnect']) {
    if (!bridge[name]) continue;
    const original = bridge[name];
    bridge[name] = async (...args) => ({ checked: true, ...await original(...args) });
  }
  const flow = createAiChatFlow({ bridge, createId: () => `request-${++count}`, getMeetings: () => exampleMeetings, ...options });
  flow.setMeeting(exampleMeetings[0]); void flow.setActive(true);
  await tick(); await tick();
  if (options.initialModels !== false && flow.getSnapshot().accounts.some(account => account.id === flow.getSnapshot().accountId && account.state === 'ready')) {
    flow.selectProvider(flow.getSnapshot().provider, { explicit: true });
    await tick();
  }
  const emit = body => {
    const event = { requestId: 'request-1', conversationId: 'conversation', ...body };
    const conversation = store.get(event.conversationId);
    const message = conversation?.messages.find(item => item.requestId === event.requestId && item.role === 'assistant');
    if (message) { event.attachments ??= message.attachments; event.saveTargets ??= message.saveTargets; message.text = event.type === 'delta' ? message.text + (event.text || '') : event.text ?? message.text; message.status = event.type === 'delta' ? 'streaming' : event.type; message.sequence = event.sequence ?? message.sequence; if (event.messageId) message.id = event.messageId; if (event.citations) message.citations = event.citations; if (['completed', 'failed', 'cancelled'].includes(event.type)) conversation.activeRequestId = null; }
    events[0](event);
  };
  return { flow, calls, events, store, state: () => flow.getSnapshot(), emit };
}

test('meeting sources preserve selected revisions and track shared and direct origins', async () => {
  const pending = deferred();
  const app = await harness({ meetingSources: id => id === 'meeting-a' ? pending.promise : Promise.resolve({ sources: [
    { pageId: 'shared', sourceId: 'shared-new', title: 'New version' }, { pageId: 'other', sourceId: 'other-1', title: 'Other' },
  ] }) });
  app.flow.edit('Question'); assert.equal(app.flow.canSend(), false);
  pending.resolve({ sources: [{ pageId: 'shared', sourceId: 'shared-old', title: 'Old version' }] }); await tick();
  assert.equal(app.flow.canSend(), true);
  app.flow.attachMeeting(exampleMeetings[1]); await tick();
  assert.deepEqual(app.state().conversation.sources.map(item => item.sourceId), ['shared-old', 'other-1']);
  app.flow.removeMeeting('meeting-a');
  assert.deepEqual(app.state().conversation.sources.map(item => item.sourceId), ['shared-new', 'other-1']);
  app.flow.removeSource('shared-new'); app.flow.setMeeting(exampleMeetings[0]);
  app.flow.attachMeeting(exampleMeetings[0]); await tick();
  assert.deepEqual(app.state().conversation.sources.map(item => item.sourceId), ['other-1']);
  await app.flow.addSource('https://notion.so/example');
  app.flow.removeMeeting('meeting-b');
  assert.deepEqual(app.state().conversation.sources.map(item => item.sourceId), ['notion:page:revision']);
  await app.flow.send();
  const request = app.calls.find(call => call[0] === 'send')[1];
  assert.deepEqual(request.sourceIds, ['notion:page:revision']);
  assert.equal(JSON.stringify(app.state().conversation.messages.at(-1).attachments).includes('originMeetingIds'), false);
  app.flow.dispose();
});


test('shared page selection follows meeting order even when loads complete in reverse', async () => {
  const first = deferred(), second = deferred();
  const app = await harness({ meetingSources: id => id === 'meeting-a' ? first.promise : second.promise });
  app.flow.attachMeeting(exampleMeetings[1]);
  second.resolve({ sources: [{ pageId: 'shared', sourceId: 'version-b' }] }); await tick();
  first.resolve({ sources: [{ pageId: 'shared', sourceId: 'version-a' }] }); await tick();
  assert.equal(app.state().conversation.sources[0].sourceId, 'version-a');
  app.flow.removeMeeting('meeting-a');
  assert.equal(app.state().conversation.sources[0].sourceId, 'version-b');
  app.flow.dispose();
});

test('reopened composition restores per-meeting versions and leaves historical attachments intact', async () => {
  const attachments = [
    { kind: 'meeting', meetingId: 'meeting-a' }, { kind: 'meeting', meetingId: 'meeting-b' },
    { kind: 'notion', pageId: 'shared', sourceId: 'version-a', title: 'Shared', origin: { directSourceId: null, meetingSources: { 'meeting-a': 'version-a', 'meeting-b': 'version-b' } } },
  ];
  const app = await harness({ read: async id => ({ id, title: 'History', messages: [{ id: 'old', role: 'user', attachments }], activeRequestId: null }) });
  assert.equal(app.state().conversation.sources[0].sourceId, 'version-a');
  app.flow.removeMeeting('meeting-a');
  assert.equal(app.state().conversation.sources[0].sourceId, 'version-b');
  assert.equal(app.state().conversation.messages[0].attachments[2].sourceId, 'version-a');
  app.flow.edit('Next'); await app.flow.send();
  const request = app.calls.find(call => call[0] === 'send')[1];
  assert.deepEqual(request.sourceOrigins['version-b'], { directSourceId: null, meetingSources: { 'meeting-b': 'version-b' } });
  app.flow.dispose();
});

test('legacy attachment keeps its frozen source when a meeting is removed', async () => {
  const attachments = [{ kind: 'meeting', meetingId: 'meeting-a' }, { kind: 'notion', pageId: 'p', sourceId: 'old', title: 'Page' }];
  const app = await harness({ read: async id => ({ id, title: 'History', messages: [{ id: 'old', role: 'user', attachments }], activeRequestId: null }), meetingSources: async () => ({ sources: [{ pageId: 'p', sourceId: 'new' }] }) });
  app.flow.removeMeeting('meeting-a');
  assert.equal(app.state().conversation.sources[0].sourceId, 'old');
  app.flow.edit('Next'); await app.flow.send();
  const request = app.calls.find(call => call[0] === 'send')[1];
  assert.deepEqual(request.sourceOrigins, {});
  app.flow.dispose();
});

test('removed meetings and conversation changes isolate pending meeting sources', async () => {
  const pending = deferred();
  const app = await harness({ meetingSources: () => pending.promise });
  app.flow.removeMeeting('meeting-a');
  assert.equal(app.state().conversation.contextBusy, false);
  await app.flow.newConversation({ attachCurrent: false });
  pending.resolve({ sources: [{ pageId: 'late', sourceId: 'late-1', title: 'Late' }] }); await tick();
  assert.deepEqual(app.state().conversation.sources, []);
  await app.flow.selectConversation('conversation');
  assert.deepEqual(app.state().conversation.sources, []);
  app.flow.dispose();
});

test('opening from a meeting attaches it while the conversation has independent ownership', async () => {
  const app = await harness();
  assert.equal(app.state().conversation.id, 'conversation');
  assert.deepEqual(app.state().conversation.meetings.map(item => item.id), ['meeting-a']);
  app.flow.removeMeeting('meeting-a'); app.flow.edit('General question'); await app.flow.send();
  assert.deepEqual(app.calls[0][1].meetingIds, []);
  assert.equal(app.calls[0][1].conversationId, 'conversation');
  assert.equal('meetingId' in app.calls[0][1], false);
  app.flow.dispose();
});

test('meeting and library navigation preserve the same draft attachments and response stream', async () => {
  const app = await harness(); app.flow.edit('Question'); await app.flow.send();
  app.flow.edit('Next question'); app.flow.setMeeting(exampleMeetings[1]); app.flow.setMeeting(null);
  app.emit({ type: 'completed', messageId: 'answer', text: 'Still here' });
  assert.equal(app.state().conversation.id, 'conversation'); assert.equal(app.state().conversation.draft, 'Next question');
  assert.equal(app.state().conversation.messages.at(-1).text, 'Still here');
  assert.deepEqual(app.state().conversation.meetings.map(item => item.id), ['meeting-a']);
  app.flow.dispose();
});

test('opening AI from another meeting adds a visible attachment instead of replacing the chat', async () => {
  const app = await harness(); app.flow.edit('Draft'); app.flow.setMeeting(exampleMeetings[1]);
  await app.flow.open(exampleMeetings[1]);
  assert.equal(app.state().conversation.id, 'conversation'); assert.equal(app.state().conversation.draft, 'Draft');
  assert.deepEqual(app.state().conversation.meetings.map(item => item.id), ['meeting-a', 'meeting-b']);
  app.flow.attachMeeting(exampleMeetings[1]); assert.equal(app.state().conversation.meetings.length, 2);
  app.flow.removeMeeting('meeting-a'); await app.flow.send(); assert.deepEqual(app.calls[0][1].meetingIds, ['meeting-b']); app.flow.dispose();
});

test('new conversations and history switching keep independent streams and drafts', async () => {
  const app = await harness(); app.flow.edit('First'); await app.flow.send(); app.flow.edit('First draft');
  await app.flow.newConversation({ attachCurrent: false });
  assert.equal(app.state().conversation.id, null); assert.equal(app.store.size, 1); assert.deepEqual(app.state().conversation.meetings, []);
  app.flow.edit('Second draft');
  app.emit({ type: 'completed', text: 'First answer', messageId: 'first-answer' });
  assert.equal(app.state().conversation.messages.length, 0); assert.equal(app.state().conversation.draft, 'Second draft');
  await app.flow.selectConversation('conversation'); assert.equal(app.state().conversation.messages.at(-1).text, 'First answer'); assert.equal(app.state().conversation.draft, 'First draft');
  await app.flow.newConversation({ attachCurrent: false }); assert.equal(app.state().conversation.draft, 'Second draft');
  await app.flow.send(); const second = app.state().conversation.id; assert.equal(second, 'conversation-2'); assert.equal(app.store.size, 2);
  app.flow.edit('Another draft'); await app.flow.selectConversation('conversation'); await app.flow.selectConversation(second); assert.equal(app.state().conversation.draft, 'Another draft'); app.flow.dispose();
});

test('Notion loading follows a conversation across meeting navigation', async () => {
  const pending = deferred(), app = await harness({ fetchSources: () => pending.promise });
  const load = app.flow.addSource('https://notion.so/example'); app.flow.setMeeting(exampleMeetings[1]); app.flow.setMeeting(null);
  pending.resolve({ sources: [{ sourceId: 'notion:snapshot', title: 'Selected page' }] }); assert.equal(await load, true);
  assert.equal(app.state().conversation.sources[0].sourceId, 'notion:snapshot'); assert.equal(app.calls.length, 0); app.flow.dispose();
});

test('switching conversations cancels source loading and discards its late result', async () => {
  const pending = deferred(), app = await harness({ fetchSources: () => pending.promise });
  const load = app.flow.addSource('https://notion.so/example'); const next = await app.flow.newConversation({ attachCurrent: false });
  pending.resolve({ sources: [{ sourceId: 'notion:late' }] }); assert.equal(await load, false);
  assert.equal(app.state().conversation.id, next); assert.deepEqual(app.state().conversation.sources, []); assert.equal(app.state().fetching, false); assert.equal(app.calls[0][0], 'cancelSource'); app.flow.dispose();
});

test('late reads cannot replace a selected independent conversation', async () => {
  const old = deferred(); const app = await harness({ read: id => id === 'conversation' ? old.promise : Promise.resolve({ id, title: 'Other', messages: [] }) });
  const selection = app.flow.selectConversation('other'); await selection;
  old.resolve({ id: 'conversation', messages: [{ id: 'old', role: 'assistant', status: 'completed', text: 'Old' }] }); await tick();
  assert.equal(app.state().conversation.id, 'other'); assert.deepEqual(app.state().conversation.messages, []); app.flow.dispose();
});

test('list failures do not create an empty replacement for unknown stored history', async () => {
  let creates = 0;
  const app = await harness({ list: async () => { throw Error('History unavailable'); }, create: async () => { creates++; return { id: 'wrong', messages: [] }; } });
  assert.equal(creates, 0); assert.equal(app.state().conversation.id, null); assert.equal(app.state().listError, 'History unavailable'); app.flow.dispose();
});

test('completed answer saving requires an explicit target when multiple meetings are attached', async () => {
  const app = await harness(); app.flow.attachMeeting(exampleMeetings[1]); app.flow.edit('Compare meetings'); await app.flow.send();
  app.emit({ type: 'completed', text: 'Answer', messageId: 'answer' });
  assert.equal(await app.flow.save('summary', 'answer'), false);
  assert.equal(await app.flow.save('summary', 'answer', 'meeting-b'), true);
  assert.equal(app.calls.find(call => call[0] === 'summary')[1].meetingId, 'meeting-b');
  assert.equal(await app.flow.save('note', 'answer', 'unattached'), false); app.flow.dispose();
});

test('copy survives meeting navigation but never copies into a newly selected conversation', async () => {
  let pending = deferred(); const copied = [];
  const app = await harness({ exportAnswer: () => pending.promise }, { writeClipboard: async text => copied.push(text) });
  app.flow.edit('Question'); await app.flow.send(); app.emit({ type: 'completed', text: 'Answer', messageId: 'answer' });
  const copy = app.flow.copy('answer', 'txt'); app.flow.setMeeting(null); pending.resolve({ text: 'Answer', format: 'txt' }); assert.equal(await copy, true);
  pending = deferred(); const laterCopy = app.flow.copy('answer', 'txt'); await app.flow.newConversation({ attachCurrent: false });
  pending.resolve({ text: 'Old', format: 'txt' }); assert.equal(await laterCopy, false); assert.deepEqual(copied, ['Answer']); app.flow.dispose();
});

test('deleting a conversation removes its history without changing the meeting or other conversations', async () => {
  const app = await harness(); await app.flow.newConversation({ attachCurrent: false }); app.flow.edit('New question'); await app.flow.send(); const next = app.state().conversation.id;
  app.emit({ conversationId: next, type: 'completed', text: 'New answer', messageId: 'answer' }); await app.flow.removeConversation(next);
  assert.equal(app.state().conversation.id, 'conversation'); assert.equal(app.state().meeting.id, 'meeting-a'); assert.equal(app.store.has(next), false);
  assert.deepEqual(app.state().conversationList.map(item => item.id), ['conversation']); app.flow.dispose();
});

test('unavailable Claude never fabricates a response or sends a request', async () => {
  const app = await harness(); app.flow.selectProvider('claude'); app.flow.edit('Question');
  assert.equal(app.flow.canSend(), false); assert.equal(await app.flow.send(), false);
  assert.equal(app.calls.length, 0); assert.equal(app.state().conversation.messages.length, 0); app.flow.dispose();
});

test('send is locked synchronously and handles events arriving before the receipt', async () => {
  const pending = deferred(); const app = await harness({ send: () => pending.promise }); app.flow.edit('Question');
  const first = app.flow.send(); assert.equal(await app.flow.send(), false); await tick();
  app.emit({ type: 'delta', text: 'From event' }); app.emit({ type: 'completed', messageId: 'answer' });
  pending.resolve({ conversationId: 'conversation', requestId: 'request-1' }); await first;
  assert.equal(app.state().conversation.messages.at(-1).text, 'From event'); assert.equal(app.state().conversation.activeRequestId, null); app.flow.dispose();
});

test('stop waits for a terminal event and failed cancellation remains retryable', async () => {
  let fail = true; const app = await harness({ cancel: async () => { if (fail) throw Error('Cancel failed'); } });
  app.flow.edit('Question'); await app.flow.send(); await app.flow.stop();
  assert.equal(app.state().conversation.activeRequestId, 'request-1'); assert.equal(app.state().conversation.stopping, false);
  fail = false; await app.flow.stop(); assert.equal(app.state().conversation.stopping, true);
  app.emit({ type: 'cancelled' }); assert.equal(app.state().conversation.messages.at(-1).status, 'cancelled');
  await app.flow.retry(app.state().conversation.messages.at(-1).id);
  assert.equal(app.calls.filter(call => call[0] === 'send').length, 2); app.flow.dispose();
});

test('only confirmed complete answers can be saved and duplicate saves are suppressed', async () => {
  const pending = deferred(); let writes = 0;
  const app = await harness({ saveSummary: async () => { writes++; await pending.promise; } }); app.flow.edit('Question'); await app.flow.send();
  assert.equal(await app.flow.save('summary', app.state().conversation.messages.at(-1).id), false);
  app.emit({ type: 'completed', text: 'Saved answer', messageId: 'answer' });
  const save = app.flow.save('summary', 'answer'); await tick(); assert.equal(await app.flow.save('summary', 'answer'), false);
  assert.equal(app.state().conversation.feedback, ''); pending.resolve(); await save;
  assert.equal(writes, 1); assert.equal(app.state().conversation.feedback, '요약을 저장했습니다.'); app.flow.dispose();
});

test('Notion sources use opaque snapshot IDs and repeated page reads replace only the chip', async () => {
  let revision = 0;
  const app = await harness({ fetchSources: async () => ({ sources: [{ sourceId: `notion:page:${++revision}`, pageId: 'page', title: 'Source' }] }) });
  await app.flow.addSource('https://notion.so/example'); await app.flow.addSource('https://notion.so/example');
  assert.equal(app.state().conversation.sources.length, 1); app.flow.edit('Question'); await app.flow.send();
  assert.deepEqual(app.calls[0][1].sourceIds, ['notion:page:2']); app.flow.dispose();
});

test('summary requests use the existing selected template and flush before sending', async () => {
  const order = []; const app = await harness({}, { beforeWrite: async () => order.push('flush'), getTemplate: async () => { order.push('template'); return 'template-id'; } });
  app.flow.summaryPrompt(); await app.flow.send(); assert.deepEqual(order, ['flush', 'template']); assert.equal(app.calls[0][1].templateId, 'template-id'); app.flow.dispose();
});

test('stale model loads cannot enable a different provider', async () => {
  const pending = deferred(); const app = await harness({ models: () => pending.promise }); app.flow.selectProvider('claude');
  pending.resolve({ models: [{ id: 'wrong', label: 'Wrong' }], defaultModelId: 'wrong' }); await tick();
  assert.equal(app.state().modelId, null); assert.deepEqual(app.state().models, []); app.flow.dispose();
});

test('unknown saved account avoids passive model access and can send after explicit selection', async () => {
  const unknown = { ...readyAccount, state: 'unknown', canInvoke: false };
  let modelCalls = 0, accountCalls = 0;
  const app = await harness({
    accounts: async () => { accountCalls++; return { checked: false, accounts: [unknown] }; },
    models: async () => { modelCalls++; return { models: [{ id: 'model', label: 'Model' }], defaultModelId: 'model' }; },
  });
  app.flow.setSettingsActive(true); await tick();
  app.flow.selectProvider('chatgpt'); await tick();
  assert.equal(modelCalls, 0);
  assert.equal(app.state().accountId, unknown.id);
  app.flow.selectProvider('chatgpt', { explicit: true }); await tick(); await tick();
  assert.equal(modelCalls, 1);
  assert.ok(accountCalls >= 2);
  app.flow.edit('Question');
  assert.equal(app.flow.canSend(), true);
  await app.flow.send();
  assert.equal(app.calls.find(call => call[0] === 'send')[1].accountId, unknown.id);
  app.flow.dispose();
});

test('settings refresh only updates account state and preserves explicitly loaded models', async () => {
  let modelCalls = 0;
  const app = await harness({
    models: async () => { modelCalls++; return { models: [{ id: 'model', label: 'Model' }], defaultModelId: 'model' }; },
  }, { initialModels: false });
  app.flow.setSettingsActive(true); await tick();
  assert.equal(modelCalls, 0);
  app.flow.selectProvider('chatgpt', { explicit: true }); await tick();
  assert.equal(modelCalls, 1);
  assert.equal(app.state().modelId, 'model');
  app.flow.setSettingsActive(false);
  app.flow.setSettingsActive(true); await tick();
  assert.equal(modelCalls, 1);
  assert.equal(app.state().modelId, 'model');
  assert.deepEqual(app.state().models.map(model => model.id), ['model']);
  app.flow.dispose();
});

test('restored account completes without a finish attempt', async () => {
  const unknown = { ...readyAccount, state: 'unknown' };
  let restored = false, finishes = 0;
  const app = await harness({
    accounts: async () => ({ checked: restored, accounts: [restored ? readyAccount : unknown] }),
    connect: async () => { restored = true; return { restored: true }; },
    finishConnect: async () => { finishes++; return { accounts: [readyAccount] }; },
  });
  await app.flow.connect('chatgpt'); await tick();
  assert.equal(finishes, 0);
  assert.equal(app.state().accounts[0].state, 'ready');
  assert.equal(app.state().connectionPhases.chatgpt, null);
  app.flow.dispose();
});

test('failed automatic account completion can be explicitly retried', async () => {
  const pendingAccount = { ...readyAccount, state: 'signed_out' };
  let attempts = 0, complete = false;
  const app = await harness({ accounts: async () => ({ accounts: [pendingAccount] }), connect: async () => { attempts++; return { attemptId: `attempt-${attempts}` }; }, cancelConnect: async () => ({ accounts: [pendingAccount] }), finishConnect: async () => { if (!complete) throw Error('Login failed'); return { accounts: [readyAccount], activeAccountId: readyAccount.id }; } });
  await app.flow.connect('chatgpt'); await app.flow.connect('chatgpt'); assert.equal(attempts, 1);
  assert.equal(app.state().accounts[0].state, 'signed_out'); assert.equal(app.state().attempts.chatgpt, 'attempt-1');
  await app.flow.retryConnect('chatgpt'); assert.equal(attempts, 2); assert.equal(app.state().attempts.chatgpt, 'attempt-2'); assert.equal(app.state().accounts[0].state, 'signed_out');
  complete = true; await app.flow.retryConnect('chatgpt'); assert.equal(app.state().accounts[0].state, 'ready'); assert.equal(app.state().attempts.chatgpt, null); app.flow.dispose();
});

test('save failures keep the answer and do not report successful persistence', async () => {
  const app = await harness({ appendNote: async () => { throw Error('Save rejected'); } }); app.flow.edit('Question'); await app.flow.send();
  app.emit({ type: 'completed', text: 'Answer', messageId: 'answer' }); assert.equal(await app.flow.save('note', 'answer'), false);
  assert.equal(app.state().conversation.feedback, ''); assert.equal(app.state().conversation.messages.at(-1).text, 'Answer'); assert.equal(app.state().conversation.error, 'Save rejected'); app.flow.dispose();
});

test('Claude OAuth automatically waits for completion without a confirm action', async () => {
  const completion = deferred(); let finishes = 0;
  const claude = { id: 'claude', provider: 'claude', state: 'signed_out', configured: true };
  const app = await harness({ accounts: async () => ({ accounts: [readyAccount, claude] }), connect: async () => ({ attemptId: 'claude-attempt' }), finishConnect: async () => { finishes++; return completion.promise; } });
  const connection = app.flow.connect('claude'); await tick();
  assert.equal(finishes, 1); assert.equal(app.state().connectionPhases.claude, 'waiting');
  assert.equal(app.state().accounts[0].state, 'ready');
  completion.resolve({ accounts: [readyAccount, { ...claude, state: 'ready' }] }); await connection;
  assert.equal(app.state().accounts[1].state, 'ready'); assert.equal(app.state().connectionPhases.claude, null); app.flow.dispose();
});

test('cancellation while awaiting OAuth completion preserves old accounts and ignores late completion', async () => {
  const completion = deferred(), cancellation = deferred(); let cancellations = 0;
  const old = { ...readyAccount, state: 'signed_out' };
  const app = await harness({ accounts: async () => ({ accounts: [old] }), connect: async () => ({ attemptId: 'attempt' }), finishConnect: () => completion.promise, cancelConnect: () => { cancellations++; return cancellation.promise; } });
  const connection = app.flow.connect('chatgpt'); await tick();
  const cancel = app.flow.cancelConnect('chatgpt'); await tick();
  assert.equal(cancellations, 1); assert.equal(app.state().connectionPhases.chatgpt, 'cancelling');
  completion.resolve({ accounts: [readyAccount] }); await connection;
  assert.equal(app.state().accounts[0].state, 'signed_out');
  cancellation.resolve({ accounts: [old] }); await cancel;
  assert.equal(app.state().accounts[0].state, 'signed_out'); assert.equal(app.state().attempts.chatgpt, null); app.flow.dispose();
});

test('cancellation during OAuth startup cancels its returned attempt without starting finish', async () => {
  const begin = deferred(); let finished = false, cancelled = null;
  const app = await harness({ accounts: async () => ({ accounts: [{ ...readyAccount, state: 'signed_out' }] }), connect: () => begin.promise, finishConnect: async () => { finished = true; }, cancelConnect: async id => { cancelled = id; return { accounts: [readyAccount] }; } });
  const connection = app.flow.connect('chatgpt'), cancel = app.flow.cancelConnect('chatgpt');
  begin.resolve({ attemptId: 'late-attempt' }); await Promise.all([connection, cancel]);
  assert.equal(cancelled, 'late-attempt'); assert.equal(finished, false); assert.equal(app.state().accounts[0].id, readyAccount.id); app.flow.dispose();
});

test('Notion automatically completes and cancellation ignores a late callback', async () => {
  let completion = deferred(), cancelled = 0;
  const previous = { connected: true, workspaceName: 'Existing workspace' };
  const app = await harness({ notionSnapshot: async () => previous, notionConnect: async () => ({ attemptId: 'notion-attempt' }), notionFinish: () => completion.promise, notionCancel: async () => { cancelled++; return previous; } });
  await app.flow.loadNotion(); const connection = app.flow.connectNotion(); await tick();
  assert.equal(app.state().connectionPhases.notion, 'waiting');
  await app.flow.cancelNotion(); completion.resolve({ connected: true, workspaceName: 'Late workspace' }); await connection;
  assert.equal(cancelled, 1); assert.equal(app.state().notion.workspaceName, 'Existing workspace');
  completion = deferred(); const next = app.flow.connectNotion(); await tick();
  completion.resolve({ connected: true, workspaceName: 'New workspace' }); await next;
  assert.equal(app.state().notion.workspaceName, 'New workspace'); assert.equal(app.state().notionAttempt, null); app.flow.dispose();
});

test('a terminal event arriving before read resolves overrides the streaming snapshot', async () => {
  const pending = deferred(); const app = await harness({ read: () => pending.promise });
  app.emit({ type: 'completed', sequence: 3, messageId: 'answer', text: 'Complete answer' });
  pending.resolve({ id: 'conversation', activeRequestId: 'request-1', messages: [{ id: 'answer', requestId: 'request-1', role: 'assistant', status: 'streaming', text: 'Complete', sequence: 1 }] });
  await tick();
  assert.equal(app.state().conversation.messages[0].text, 'Complete answer');
  assert.equal(app.state().conversation.messages[0].status, 'completed');
  assert.equal(app.state().conversation.activeRequestId, null); app.flow.dispose();
});

test('read snapshots consume matching deltas once and replay only later sequence numbers', async () => {
  const pending = deferred(); const app = await harness({ read: () => pending.promise });
  app.emit({ type: 'delta', sequence: 1, text: 'First' }); app.emit({ type: 'delta', sequence: 2, text: ' second' });
  pending.resolve({ id: 'conversation', activeRequestId: 'request-1', messages: [{ id: 'answer', requestId: 'request-1', role: 'assistant', status: 'streaming', text: 'First', sequence: 1 }] });
  await tick();
  assert.equal(app.state().conversation.messages[0].text, 'First second');
  assert.equal(app.state().conversation.messages[0].sequence, 2); app.flow.dispose();
});

test('duplicate and reordered deltas do not corrupt the answer and terminal text closes sequence gaps', async () => {
  const app = await harness(); app.flow.edit('Question'); await app.flow.send();
  app.emit({ type: 'delta', sequence: 2, text: ' second' });
  assert.equal(app.state().conversation.messages.at(-1).text, '');
  app.emit({ type: 'delta', sequence: 1, text: 'First' });
  app.emit({ type: 'delta', sequence: 1, text: 'First' }); app.emit({ type: 'delta', sequence: 2, text: ' second' });
  assert.equal(app.state().conversation.messages.at(-1).text, 'First second');
  app.emit({ type: 'delta', sequence: 4, text: ' fourth' });
  app.emit({ type: 'cancelled', sequence: 5, text: 'First second third fourth' });
  assert.equal(app.state().conversation.messages.at(-1).text, 'First second third fourth');
  assert.equal(app.state().conversation.activeRequestId, null); app.flow.dispose();
});

test('completed answers copy only native formatted text without citation options', async () => {
  const copied = [], exports = [];
  const app = await harness({ exportAnswer: async request => { exports.push(request); return { text: request.format === 'md' ? '**Native answer**' : 'Native answer', format: request.format }; } }, { writeClipboard: async text => copied.push(text) });
  app.flow.edit('Question'); await app.flow.send();
  assert.equal(await app.flow.copy(app.state().conversation.messages.at(-1).id, 'txt'), false);
  app.emit({ type: 'completed', text: 'Raw answer [1]', messageId: 'answer' });
  assert.equal(await app.flow.copy('answer', 'txt'), true); assert.equal(await app.flow.copy('answer', 'md'), true);
  assert.deepEqual(copied, ['Native answer', '**Native answer**']);
  assert.deepEqual(exports[0], { conversationId: 'conversation', messageId: 'answer', format: 'txt' });
  assert.equal(app.state().conversation.feedback, '복사했습니다.'); app.flow.dispose();
});

test('reconnect passes the old account ID and selects the newly ready active account', async () => {
  const old = { ...readyAccount, id: 'old', state: 'signed_out' }, fresh = { ...readyAccount, id: 'new' };
  const reconnects = [], modelLoads = [];
  const app = await harness({ accounts: async () => ({ accounts: [old], activeAccountId: 'old' }),
    connect: async (provider, accountId) => { reconnects.push([provider, accountId]); return { attemptId: 'reconnect' }; },
    finishConnect: async () => ({ accounts: [old, fresh], activeAccountId: 'new' }),
    models: async accountId => { modelLoads.push(accountId); return { models: [{ id: 'new-model', label: 'New model' }], defaultModelId: 'new-model' }; },
  });
  await app.flow.connect('chatgpt'); await tick(); app.flow.selectProvider('chatgpt', { explicit: true }); await tick(); app.flow.edit('Question');
  assert.deepEqual(reconnects, [['chatgpt', 'old']]); assert.equal(app.state().accountId, 'new'); assert.deepEqual(modelLoads, ['new']); assert.equal(app.flow.canSend(), true);
  await app.flow.send(); assert.equal(app.calls.find(call => call[0] === 'send')[1].accountId, 'new'); app.flow.dispose();
});

test('declined plan permission reconnects through the same account registration', async () => {
  const requested=[];
  const app=await harness({ accounts:async()=>({accounts:[{...readyAccount,state:'plan_disabled'}]}),
    connect:async(provider,accountId)=>{requested.push({provider,accountId});return {attemptId:'new-consent'};},
    finishConnect:async()=>({accounts:[readyAccount],activeAccountId:readyAccount.id}) });
  await app.flow.connect('chatgpt'); await tick();
  assert.deepEqual(requested,[{provider:'chatgpt',accountId:readyAccount.id}]);
  assert.equal(app.state().accountId,readyAccount.id);app.flow.selectProvider('chatgpt', { explicit: true });await tick();app.flow.edit('Question');assert.equal(app.flow.canSend(),true);
  app.flow.dispose();
});

test('explicit forced read can replace an active cached snapshot and merge a racing terminal event', async () => {
  const refresh = deferred(); let reads = 0;
  const app = await harness({ read: async () => { reads++; return reads === 1 ? { id: 'conversation', messages: [] } : refresh.promise; } });
  app.flow.edit('Question'); await app.flow.send(); app.emit({ type: 'delta', sequence: 1, text: 'First' });
  const loading = app.flow.loadConversation(true); assert.equal(reads, 2);
  app.emit({ type: 'completed', sequence: 3, text: 'First second', messageId: 'answer' });
  refresh.resolve({ id: 'conversation', activeRequestId: 'request-1', messages: [{ id: 'answer', requestId: 'request-1', role: 'assistant', status: 'streaming', text: 'First', sequence: 1 }] }); await loading;
  assert.equal(app.state().conversation.messages.at(-1).text, 'First second'); assert.equal(app.state().conversation.activeRequestId, null); app.flow.dispose();
});

test('a deletion cancellation event releases the hidden conversation account-disconnect guard', async () => {
  let disconnects = 0;
  const app = await harness({ disconnect: async () => { disconnects++; return { accounts: [{ ...readyAccount, state: 'signed_out' }] }; } });
  app.flow.edit('Question'); await app.flow.send(); app.flow.setMeeting(null); app.flow.setActive(false);
  app.emit({ type: 'cancelled', sequence: 1, text: '', messageId: 'answer' });
  await app.flow.disconnect(readyAccount.id);
  assert.equal(disconnects, 1); assert.equal(app.state().accountsError, ''); app.flow.dispose();
});

test('forced refresh waits for a locally starting request before reading its persisted snapshot', async () => {
  const receipt = deferred(); let reads = 0;
  const app = await harness({ send: () => receipt.promise, read: async () => {
    reads++; return reads === 1 ? { id: 'conversation', messages: [] } : { id: 'conversation', activeRequestId: 'request-1', messages: [{ id: 'answer', requestId: 'request-1', role: 'assistant', status: 'streaming', text: '', sequence: 0 }] };
  } });
  app.flow.edit('Question'); const send = app.flow.send(); await tick();
  const refresh = app.flow.loadConversation(true); await tick(); assert.equal(reads, 1);
  app.emit({ type: 'completed', sequence: 1, text: 'Actual answer', messageId: 'answer' });
  receipt.resolve({ conversationId: 'conversation', requestId: 'request-1' }); await send; await refresh;
  assert.equal(reads, 2); assert.equal(app.state().conversation.messages.at(-1).text, 'Actual answer'); assert.equal(app.state().conversation.activeRequestId, null); app.flow.dispose();
});

test('a forced re-entry supersedes an older pending read and ignores its stale streaming response', async () => {
  const old = deferred(), restored = deferred(); let reads = 0;
  const app = await harness({ read: () => ++reads === 1 ? old.promise : restored.promise });
  app.flow.setMeeting(null); app.flow.setActive(false); app.flow.setMeeting({ id: 'meeting-a', title: 'Restored' }); app.flow.setActive(true);
  assert.equal(reads, 2);
  restored.resolve({ id: 'conversation', activeRequestId: null, messages: [{ id: 'answer', requestId: 'request-1', role: 'assistant', status: 'cancelled', text: 'Preserved', sequence: 2 }] }); await tick();
  old.resolve({ id: 'conversation', activeRequestId: 'request-1', messages: [{ id: 'answer', requestId: 'request-1', role: 'assistant', status: 'streaming', text: 'Old', sequence: 1 }] }); await tick();
  assert.equal(app.state().conversation.activeRequestId, null); assert.equal(app.state().conversation.messages[0].status, 'cancelled');
  app.flow.edit('Next'); assert.equal(app.flow.canSend(), true); app.flow.dispose();
});

test('superseded read failures cannot overwrite the latest snapshot or release its event buffer', async () => {
  const old = deferred(), latest = deferred(); let reads = 0;
  const app = await harness({ read: () => ++reads === 1 ? old.promise : latest.promise });
  const load = app.flow.loadConversation(true); old.reject(Error('Stale error')); await tick();
  app.emit({ type: 'completed', sequence: 2, messageId: 'answer', text: 'Latest answer' });
  latest.resolve({ id: 'conversation', activeRequestId: 'request-1', messages: [{ id: 'answer', requestId: 'request-1', role: 'assistant', status: 'streaming', text: 'Latest', sequence: 1 }] }); await load;
  assert.equal(app.state().conversation.error, ''); assert.equal(app.state().conversation.messages[0].text, 'Latest answer'); assert.equal(app.state().conversation.activeRequestId, null); app.flow.dispose();
});

test('multiple forced reads during a pending send wait for acceptance and only the newest reads', async () => {
  const receipt = deferred(); let reads = 0;
  const app = await harness({ send: () => receipt.promise, read: async () => { reads++; return { id: 'conversation', messages: [] }; } });
  app.flow.edit('Question'); const send = app.flow.send(); await tick(); const first = app.flow.loadConversation(true), second = app.flow.loadConversation(true);
  receipt.resolve({ conversationId: 'conversation', requestId: 'request-1' }); await Promise.all([send, first, second]);
  assert.equal(reads, 2); assert.equal(app.state().conversation.loading, false); app.flow.dispose();
});

test('the active plan-disabled account is chosen over an older signed-out account for reconnection', async () => {
  const old = { ...readyAccount, id: 'old', state: 'signed_out' }, active = { ...readyAccount, id: 'active-plan', state: 'plan_disabled' };
  const reconnects = [];
  const app = await harness({ accounts: async () => ({ accounts: [old, active], activeAccountId: active.id }),
    connect: async (provider, accountId) => { reconnects.push(accountId); return { attemptId: 'consent' }; },
    finishConnect: async () => ({ accounts: [old, { ...active, state: 'ready' }], activeAccountId: active.id }),
  });
  assert.equal(app.state().accountId, active.id); await app.flow.connect('chatgpt'); await tick();
  assert.deepEqual(reconnects, [active.id]); app.flow.selectProvider('chatgpt', { explicit: true }); await tick(); app.flow.edit('Question'); assert.equal(app.flow.canSend(), true); app.flow.dispose();
});


test('a new composer keeps attachments and draft without creating any native history until send', async () => {
  let creates = 0;
  const app = await harness({ list: async () => [], create: async () => { creates++; return { id: 'first-persisted', title: 'New', messages: [], activeRequestId: null }; }, send: async request => { app.calls.push(['send', request]); return { conversationId: request.conversationId, requestId: request.requestId }; } });
  assert.equal(creates, 0); assert.equal(app.state().conversation.id, null); assert.equal(app.state().conversation.loaded, true);
  app.flow.edit('Draft'); await app.flow.addSource('https://notion.so/example');
  await app.flow.newConversation(); await app.flow.newConversation();
  assert.equal(creates, 0); assert.equal(app.state().conversation.draft, 'Draft'); assert.equal(app.state().conversation.sources.length, 1); assert.equal(app.state().conversation.meetings.length, 1);
  assert.equal(app.flow.canSend(), true); await app.flow.send(); assert.equal(creates, 1); assert.equal(app.state().conversation.id, 'first-persisted');
  assert.deepEqual(app.calls.find(call => call[0] === 'send')[1].meetingIds, ['meeting-a']); app.flow.dispose();
});

test('duplicate sends during lazy creation share one synchronous admission', async () => {
  const created = deferred(); let creates = 0, sends = 0;
  const app = await harness({ list: async () => [], create: () => { creates++; return created.promise; }, send: async request => { sends++; return { conversationId: request.conversationId, requestId: request.requestId }; } });
  app.flow.edit('Question'); const first = app.flow.send(); await tick(); assert.equal(await app.flow.send(), false);
  await app.flow.newConversation(); assert.equal(app.state().conversation.action, 'sending');
  created.resolve({ id: 'persisted', title: 'New', messages: [], activeRequestId: null }); assert.equal(await first, true);
  assert.equal(creates, 1); assert.equal(sends, 1); assert.equal(app.state().conversation.activeRequestId, 'request-1'); app.flow.dispose();
});

test('a delayed open cannot attach its old meeting to a subsequently selected conversation', async () => {
  const previous = deferred(); const app = await harness({ read: id => id === 'conversation' ? previous.promise : Promise.resolve({ id, messages: [] }) });
  await app.flow.selectConversation('other'); previous.resolve({ id: 'conversation', messages: [] }); await tick();
  assert.equal(app.state().conversation.id, 'other'); assert.deepEqual(app.state().conversation.meetings, []); app.flow.dispose();
});


test('an old opening request cannot add its meeting after an explicit fresh composer selection', async () => {
  const history = deferred(); const app = await harness({ list: () => history.promise });
  await app.flow.newConversation({ attachCurrent: false }); history.resolve([]); await tick();
  assert.equal(app.state().conversation.id, null); assert.deepEqual(app.state().conversation.meetings, []); app.flow.dispose();
});


test('summary saving uses native target capability while note appending remains available', async () => {
  const app = await harness(); app.flow.edit('Question'); await app.flow.send();
  app.emit({ type: 'completed', text: 'Answer', messageId: 'answer', saveTargets: [{ meetingId: 'meeting-a', title: 'Meeting', canSaveSummary: false }] });
  assert.equal(await app.flow.save('summary', 'answer', 'meeting-a'), false);
  assert.equal(await app.flow.save('note', 'answer', 'meeting-a'), true); app.flow.dispose();
});

test('a failed screen refresh after persistence does not report the successful write as failed', async () => {
  let writes = 0;
  const app = await harness({ saveSummary: async () => { writes++; } }, { onSaved: async () => { throw Error('Refresh failed'); } });
  app.flow.edit('Question'); await app.flow.send(); app.emit({ type: 'completed', text: 'Answer', messageId: 'answer' });
  assert.equal(await app.flow.save('summary', 'answer'), true); assert.equal(writes, 1); assert.equal(app.state().conversation.feedback, '요약을 저장했습니다.'); assert.match(app.state().conversation.error, /표시하지 못/); app.flow.dispose();
});


test('native history refresh updates the active title without replacing its draft or messages', async () => {
  let title = 'Initial';
  const app = await harness({ list: async () => [{ id: 'conversation', title }] });
  app.flow.edit('Draft'); title = 'Question title'; await app.flow.loadList(true);
  assert.equal(app.state().conversation.title, 'Question title'); assert.equal(app.state().conversation.draft, 'Draft'); app.flow.dispose();
});

test('typing into the initial composer prevents a late history list from switching away', async () => {
  const history = deferred(); const app = await harness({ list: () => history.promise });
  app.flow.edit('My new question'); history.resolve([{ id: 'conversation', title: 'Old conversation' }]); await tick();
  assert.equal(app.state().conversation.id, null); assert.equal(app.state().conversation.draft, 'My new question');
  assert.equal(app.flow.canSend(), true); app.flow.dispose();
});


test('selected template survives editing and skill saves only its explicit target', async () => {
  const app = await harness({ composerCatalog: async () => ({ skills: [{ id: 'meeting-summary', name: '회의 요약' }], templates: [{ id: 'template', name: '요약', revision: 'r1' }], defaultTemplateId: 'template' }) });
  await app.flow.loadCatalog();
  app.flow.selectTemplate('template'); app.flow.selectSkill('meeting-summary'); app.flow.edit('정리해 주세요');
  assert.equal(app.flow.getSnapshot().conversation.templateId, 'template');
  assert.equal(app.flow.canSend(), false);
  app.flow.selectSummaryTarget('meeting-a'); assert.equal(app.flow.canSend(), true);
  await app.flow.send();
  const request = app.calls.find(call => call[0] === 'send')[1];
  assert.equal(request.templateId, 'template'); assert.equal(request.skillId, 'meeting-summary'); assert.equal(request.summaryTargetId, 'meeting-a');
  app.emit({ type: 'completed', sequence: 1, messageId: 'answer', text: '요약', saveTargets: [{ meetingId: 'meeting-a', canSaveSummary: true }] });
  await tick(); await tick();
  assert.deepEqual(app.calls.find(call => call[0] === 'summary')[1], { conversationId: 'conversation', messageId: 'answer', meetingId: 'meeting-a' });
  app.flow.dispose();
});
