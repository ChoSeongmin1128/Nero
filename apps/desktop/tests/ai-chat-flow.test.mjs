import assert from 'node:assert/strict';
import test from 'node:test';
import { createAiChatFlow } from '../src/ai-chat-flow.js';
const tick = () => new Promise(resolve => setImmediate(resolve));
const deferred = () => { let resolve, reject; const promise = new Promise((a, b) => { resolve = a; reject = b; }); return { promise, resolve, reject }; };
const readyAccount = { id: 'account', provider: 'chatgpt', state: 'ready', configured: true, label: 'Example' };
async function harness(overrides = {}, options = {}) {
  const calls = [], events = [];
  let count = 0;
  const bridge = {
    accounts: async () => ({ accounts: [readyAccount, { id: 'claude', provider: 'claude', state: 'unavailable', configured: false }] }),
    models: async () => ({ models: [{ id: 'model', label: 'Model' }], defaultModelId: 'model' }),
    read: async meetingId => ({ id: null, meetingId, messages: [] }),
    subscribe: async callback => { events.push(callback); return () => {}; },
    send: async request => { calls.push(['send', request]); return { conversationId: 'conversation', requestId: request.requestId }; },
    cancel: async requestId => { calls.push(['cancel', requestId]); },
    saveSummary: async request => { calls.push(['summary', request]); },
    appendNote: async request => { calls.push(['note', request]); },
    notionSnapshot: async () => ({ connected: true }),
    fetchSources: async request => { calls.push(['source', request]); return { sources: [{ sourceId: 'notion:page:revision', pageId: 'page', title: 'Example source' }] }; },
    cancelFetch: async requestId => { calls.push(['cancelSource', requestId]); },
    ...overrides,
  };
  const flow = createAiChatFlow({ bridge, createId: () => `request-${++count}`, ...options });
  flow.setMeeting({ id: 'meeting-a', title: 'Example meeting' }); flow.setActive(true);
  await tick(); await tick();
  return { flow, calls, events, state: () => flow.getSnapshot(), emit: event => events[0]({ meetingId: 'meeting-a', requestId: 'request-1', conversationId: 'conversation', ...event }) };
}

test('meeting context is included by default and explicit removal keeps conversation ownership', async () => {
  const app = await harness(); app.flow.edit('Question'); assert.equal(app.flow.canSend(), true);
  app.flow.includeMeeting(false); await app.flow.send();
  assert.equal(app.calls[0][1].meetingId, 'meeting-a'); assert.equal(app.calls[0][1].includeMeeting, false);
  assert.equal(app.state().conversation.activeRequestId, 'request-1');
  app.emit({ type: 'delta', text: 'Actual ' }); app.emit({ type: 'delta', text: 'answer' });
  assert.equal(app.state().conversation.messages.at(-1).text, 'Actual answer');
  app.emit({ type: 'completed', messageId: 'message-1', citations: [{ reference: 1, kind: 'meeting' }] });
  assert.equal(app.state().conversation.activeRequestId, null);
  assert.equal(app.state().conversation.messages.at(-1).id, 'message-1');
  app.flow.dispose();
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

test('streaming another meeting cannot populate the currently selected meeting', async () => {
  const app = await harness(); app.flow.edit('A question'); await app.flow.send();
  app.flow.setMeeting({ id: 'meeting-b', title: 'B' }); await tick();
  app.emit({ type: 'completed', messageId: 'answer', text: 'A answer' });
  assert.equal(app.state().conversation.messages.length, 0);
  app.flow.setMeeting({ id: 'meeting-a', title: 'A' });
  assert.equal(app.state().conversation.messages.at(-1).text, 'A answer'); app.flow.dispose();
});

test('late read completion cannot replace another meeting or create messages on failure', async () => {
  const pending = deferred(); const app = await harness({ read: id => id === 'meeting-a' ? pending.promise : Promise.resolve({ id: null, messages: [] }) });
  app.flow.setMeeting({ id: 'meeting-b', title: 'B' }); await tick();
  pending.resolve({ id: 'old', messages: [{ id: 'old-answer', text: 'Old', role: 'assistant', status: 'completed' }] }); await tick();
  assert.equal(app.state().conversation.id, null); assert.equal(app.state().conversation.messages.length, 0); app.flow.dispose();
  const failed = await harness({ read: async () => { throw Error('Read failed'); } }); failed.flow.edit('Question');
  assert.equal(failed.flow.canSend(), false); assert.equal(await failed.flow.send(), false); failed.flow.dispose();
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

test('switching meetings cancels source loading and discards its late result', async () => {
  const pending = deferred(); const app = await harness({ fetchSources: () => pending.promise });
  const load = app.flow.addSource('https://notion.so/example'); app.flow.setMeeting({ id: 'meeting-b', title: 'B' });
  pending.resolve({ sources: [{ sourceId: 'old' }] }); await load;
  assert.equal(app.state().conversation.sources.length, 0); assert.equal(app.state().fetching, false); assert.equal(app.calls[0][0], 'cancelSource'); app.flow.dispose();
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
  assert.deepEqual(exports[0], { conversationId: 'conversation', messageId: 'answer', meetingId: 'meeting-a', format: 'txt' });
  assert.equal(app.state().conversation.feedback, '복사했습니다.'); app.flow.dispose();
});

test('copy failures show no success feedback and meeting changes invalidate pending exports', async () => {
  const pending = deferred(), copied = [];
  const app = await harness({ exportAnswer: () => pending.promise }, { writeClipboard: async text => copied.push(text) });
  app.flow.edit('Question'); await app.flow.send(); app.emit({ type: 'completed', text: 'Answer', messageId: 'answer' });
  const copy = app.flow.copy('answer', 'txt'); app.flow.setMeeting({ id: 'meeting-b', title: 'B' }); app.flow.setMeeting({ id: 'meeting-a', title: 'A' });
  pending.resolve({ text: 'Old answer', format: 'txt' }); assert.equal(await copy, false); assert.deepEqual(copied, []); app.flow.dispose();
  const failed = await harness({ exportAnswer: async () => ({ text: 'Answer', format: 'txt' }) }, { writeClipboard: async () => { throw Error('Clipboard denied'); } });
  failed.flow.edit('Question'); await failed.flow.send(); failed.emit({ type: 'completed', text: 'Answer', messageId: 'answer' });
  assert.equal(await failed.flow.copy('answer', 'txt'), false); assert.equal(failed.state().conversation.feedback, ''); assert.equal(failed.state().conversation.error, 'Clipboard denied'); failed.flow.dispose();
});

test('reconnect passes the old account ID and selects the newly ready active account', async () => {
  const old = { ...readyAccount, id: 'old', state: 'signed_out' }, fresh = { ...readyAccount, id: 'new' };
  const reconnects = [], modelLoads = [];
  const app = await harness({ accounts: async () => ({ accounts: [old], activeAccountId: 'old' }),
    connect: async (provider, accountId) => { reconnects.push([provider, accountId]); return { attemptId: 'reconnect' }; },
    finishConnect: async () => ({ accounts: [old, fresh], activeAccountId: 'new' }),
    models: async accountId => { modelLoads.push(accountId); return { models: [{ id: 'new-model', label: 'New model' }], defaultModelId: 'new-model' }; },
  });
  await app.flow.connect('chatgpt'); await tick(); app.flow.edit('Question');
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
  assert.equal(app.state().accountId,readyAccount.id);app.flow.edit('Question');assert.equal(app.flow.canSend(),true);
  app.flow.dispose();
});

test('re-entering a restored meeting refreshes a cached stream even when its terminal event was missed', async () => {
  let reads = 0, cancelled = false;
  const persisted = () => ({ id: 'conversation', activeRequestId: cancelled ? null : 'request-1', messages: [
    { id: 'question', role: 'user', text: 'Question', status: 'completed' },
    { id: 'answer', requestId: 'request-1', role: 'assistant', text: 'Partial answer', status: cancelled ? 'cancelled' : 'streaming', sequence: cancelled ? 2 : 1 },
  ] });
  const app = await harness({ read: async () => { reads++; return persisted(); } });
  assert.equal(app.state().conversation.activeRequestId, 'request-1');
  app.flow.setMeeting(null); app.flow.setActive(false); cancelled = true;
  app.flow.setMeeting({ id: 'meeting-a', title: 'Restored meeting' }); app.flow.setActive(true); await tick();
  assert.equal(reads, 2); assert.equal(app.state().conversation.messages.at(-1).status, 'cancelled');
  assert.equal(app.state().conversation.activeRequestId, null); app.flow.edit('Next question'); assert.equal(app.flow.canSend(), true); app.flow.dispose();
});

test('explicit forced read can replace an active cached snapshot and merge a racing terminal event', async () => {
  const refresh = deferred(); let reads = 0;
  const app = await harness({ read: async () => { reads++; return reads === 1 ? { id: null, messages: [] } : refresh.promise; } });
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
    reads++; return reads === 1 ? { id: null, messages: [] } : { id: 'conversation', activeRequestId: 'request-1', messages: [{ id: 'answer', requestId: 'request-1', role: 'assistant', status: 'streaming', text: '', sequence: 0 }] };
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
  const app = await harness({ send: () => receipt.promise, read: async () => { reads++; return { id: reads === 1 ? null : 'conversation', messages: [] }; } });
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
  assert.deepEqual(reconnects, [active.id]); app.flow.edit('Question'); assert.equal(app.flow.canSend(), true); app.flow.dispose();
});
