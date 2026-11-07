import test from 'node:test';
import assert from 'node:assert/strict';
import { createMeetingContextFlow } from '../src/meeting-context-flow.js';

const tick = () => new Promise(resolve => setImmediate(resolve));
const deferred = () => { let resolve; const promise = new Promise(done => { resolve = done; }); return { promise, resolve }; };
function setup(overrides = {}) {
  let meeting = { id: 'one' };
  const calls = [];
  const source = { pageId: 'page', sourceId: 'snapshot', title: '자료', url: 'https://notion.so/page' };
  const bridge = {
    meetingSources: async () => ({ revision: 1, sources: [source] }),
    notionSnapshot: async () => ({ connected: true }),
    fetchSources: async () => ({ sources: [source] }),
    meetingAddSource: async request => { calls.push(request); return { revision: 2, sources: [source] }; },
    meetingRemoveSource: async request => { calls.push(request); return { revision: 2, sources: [] }; },
    cancelFetch: async id => { calls.push({ cancelled: id }); },
    openSource: async () => {}, ...overrides,
  };
  const flow = createMeetingContextFlow({ bridge, getMeeting: () => meeting });
  return { flow, calls, source, select: value => { meeting = value; flow.setMeeting(value); } };
}

test('회의 전환 후 늦은 fetch 결과를 연결하지 않습니다', async () => {
  const pending = deferred();
  const { flow, calls, source, select } = setup({ fetchSources: () => pending.promise });
  select({ id: 'one' }); flow.open(); await tick();
  flow.setLink('https://notion.so/page'); const add = flow.add();
  select({ id: 'two' }); pending.resolve({ sources: [source] }); await add; await tick();
  assert.equal(calls.filter(call => call.sourceId).length, 0);
  assert.equal(calls.filter(call => call.cancelled).length, 1);
  assert.equal(flow.getSnapshot().meetingId, 'two');
});

test('연결이 끊겨도 기존 자료를 조회하고 제거합니다', async () => {
  const { flow, calls, select } = setup({ notionSnapshot: async () => ({ connected: false }) });
  select({ id: 'one' }); flow.open(); await tick();
  assert.equal(flow.getSnapshot().sources.length, 1);
  await flow.remove('page');
  assert.deepEqual(calls[0], { meetingId: 'one', expectedRevision: 1, pageId: 'page' });
  assert.deepEqual(flow.getSnapshot().sources, []);
});

test('가져오기 실패 시 입력을 보존하고 재시도합니다', async () => {
  let count = 0;
  const { flow, calls, source, select } = setup({ fetchSources: async () => { if (++count === 1) throw new Error('오류'); return { sources: [source] }; } });
  select({ id: 'one' }); flow.open(); await tick();
  flow.setLink('https://notion.so/page'); await flow.add();
  assert.equal(flow.getSnapshot().link, 'https://notion.so/page');
  assert.equal(flow.getSnapshot().error, '오류');
  await flow.add();
  assert.equal(calls[0].sourceId, 'snapshot');
  assert.equal(flow.getSnapshot().link, '');
});

test('명시적 갱신은 같은 페이지의 새 스냅샷만 교체합니다', async () => {
  const fresh = { pageId: 'page', sourceId: 'new-snapshot', url: 'https://notion.so/page' };
  const calls = [];
  const { flow, source, select } = setup({
    fetchSources: async request => { assert.deepEqual(request.links, [fresh.url]); return { sources: [fresh] }; },
    meetingRefreshSource: async request => { calls.push(request); return { revision: 2, sources: [fresh] }; },
  });
  select({ id: 'one' }); flow.open(); await tick();
  await flow.refresh(source);
  assert.deepEqual(calls, [{ meetingId: 'one', expectedRevision: 1, sourceId: 'new-snapshot' }]);
  assert.equal(flow.getSnapshot().sources[0].sourceId, 'new-snapshot');
});

test('취소한 요청이 끝날 때까지 바쁨 상태를 유지해 바로 다시 추가해도 거절되지 않습니다', async () => {
  const pending = deferred();
  const { flow, select } = setup({ fetchSources: () => pending.promise });
  select({ id: 'one' }); flow.open(); await tick();
  flow.setLink('https://notion.so/page');
  const adding = flow.add(); await tick();
  const cancelling = flow.cancel(); await tick();
  assert.equal(flow.getSnapshot().busy, true);
  pending.resolve({ sources: [] }); await cancelling; await adding;
  assert.equal(flow.getSnapshot().busy, false);
});

test('갱신 충돌이나 취소 시 기존 참조를 유지합니다', async () => {
  const pending = deferred(); let calls = 0;
  const { flow, source, select } = setup({
    fetchSources: () => pending.promise,
    meetingRefreshSource: async () => { calls++; throw new Error('변경 충돌'); },
  });
  select({ id: 'one' }); flow.open(); await tick();
  const cancelled = flow.refresh(source); flow.cancel();
  pending.resolve({ sources: [{ ...source, sourceId: 'new' }] }); await cancelled;
  assert.equal(calls, 0);
  assert.equal(flow.getSnapshot().sources[0].sourceId, source.sourceId);
  await flow.refresh(source);
  assert.equal(calls, 1);
  assert.equal(flow.getSnapshot().error, '변경 충돌');
  assert.equal(flow.getSnapshot().sources[0].sourceId, source.sourceId);
});
