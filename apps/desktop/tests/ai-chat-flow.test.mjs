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

test('account connection remains pending until explicit completion succeeds', async () => {
  const pendingAccount = { ...readyAccount, state: 'signed_out' };
  let attempts = 0, complete = false;
  const app = await harness({ accounts: async () => ({ accounts: [pendingAccount] }), connect: async () => { attempts++; return { attemptId: 'attempt' }; }, finishConnect: async () => { if (!complete) throw Error('Login not finished'); return { accounts: [readyAccount] }; } });
  await app.flow.connect('chatgpt'); await app.flow.connect('chatgpt'); assert.equal(attempts, 1);
  assert.equal(app.state().accounts[0].state, 'signed_out'); assert.equal(app.state().attempts.chatgpt, 'attempt');
  await app.flow.finishConnect('chatgpt'); assert.equal(app.state().attempts.chatgpt, 'attempt'); assert.equal(app.state().accounts[0].state, 'signed_out');
  complete = true; await app.flow.finishConnect('chatgpt'); assert.equal(app.state().accounts[0].state, 'ready'); assert.equal(app.state().attempts.chatgpt, null); app.flow.dispose();
});

test('save failures keep the answer and do not report successful persistence', async () => {
  const app = await harness({ appendNote: async () => { throw Error('Save rejected'); } }); app.flow.edit('Question'); await app.flow.send();
  app.emit({ type: 'completed', text: 'Answer', messageId: 'answer' }); assert.equal(await app.flow.save('note', 'answer'), false);
  assert.equal(app.state().conversation.feedback, ''); assert.equal(app.state().conversation.messages.at(-1).text, 'Answer'); assert.equal(app.state().conversation.error, 'Save rejected'); app.flow.dispose();
});
