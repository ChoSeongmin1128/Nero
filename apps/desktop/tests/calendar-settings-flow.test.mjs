import assert from 'node:assert/strict';
import test from 'node:test';
import { createCalendarSettingsFlow, reminderCountdown } from '../src/calendar-settings-flow.js';

const snapshot = { revision: 1, configured: true, connected: true, connecting: false, restoring: false, accountLabel: 'synthetic', calendars: [{ id: 'one', name: 'One' }, { id: 'two', name: 'Two' }], selectedCalendars: ['one'], lastSyncedAt: null, error: null };
const deferred = () => { let resolve; const promise = new Promise(done => { resolve = done; }); return { promise, resolve }; };
const flush = () => new Promise(resolve => setImmediate(resolve));
const setup = async overrides => {
  const bridge = { readCalendarSettings: async () => snapshot, cancelCalendarConnect: async () => ({ ...snapshot, connecting: false }), ...overrides };
  const flow = createCalendarSettingsFlow({ bridge }); flow.setActive(true); await flush(); return flow;
};

test('calendar choices save explicit IDs and never create meetings or recordings', async () => {
  const saves = [];
  const flow = await setup({ selectCalendars: async ids => { saves.push(ids); return { ...snapshot, revision: 2, selectedCalendars: ids }; } });
  await flow.select('two', true);
  assert.deepEqual(saves, [['one', 'two']]);
  await flow.select('one', false);
  assert.deepEqual(saves[1], ['two']);
});

test('failed selection preserves saved choices and shows a retryable error', async () => {
  const flow = await setup({ selectCalendars: async () => { throw Error('저장 실패'); } });
  assert.equal(await flow.select('two', true), false);
  assert.deepEqual(flow.getSnapshot().snapshot.selectedCalendars, ['one']);
  assert.equal(flow.getSnapshot().error, '저장 실패');
});

test('hide cancels only its connection attempt and ignores its late completion', async () => {
  const finishing = deferred(); const cancelled = [];
  const flow = await setup({ beginCalendarConnect: async () => ({ attemptId: 'first' }), finishCalendarConnect: () => finishing.promise, cancelCalendarConnect: async id => { cancelled.push(id); return { ...snapshot, revision: 2 }; } });
  const connecting = flow.connect(); await flush(); flow.setActive(false);
  finishing.resolve({ ...snapshot, revision: 3, accountLabel: 'stale' }); await connecting;
  assert.deepEqual(cancelled, ['first']);
  assert.equal(flow.getSnapshot().snapshot.accountLabel, 'synthetic');
  assert.equal(flow.getSnapshot().busy, false);
});

test('closing before browser-open completes cancels the exact returned attempt', async () => {
  const beginning = deferred(); const cancelled = [];
  const flow = await setup({ beginCalendarConnect: () => beginning.promise, cancelCalendarConnect: async id => { cancelled.push(id); return snapshot; } });
  const connecting = flow.connect(); flow.setActive(false);
  beginning.resolve({ attemptId: 'late-attempt' }); await connecting;
  assert.deepEqual(cancelled, ['late-attempt']);
});

test('stale snapshots and responses received after leaving the panel cannot replace state', async () => {
  const pending = deferred(); const flow = await setup({ syncCalendar: () => pending.promise });
  flow.accept({ ...snapshot, revision: 0, accountLabel: 'stale' });
  assert.equal(flow.getSnapshot().snapshot.accountLabel, 'synthetic');
  const refresh = flow.refresh(); flow.setActive(false);
  pending.resolve({ ...snapshot, revision: 3, accountLabel: 'late' }); await refresh;
  assert.equal(flow.getSnapshot().snapshot.accountLabel, 'synthetic');
});

test('duplicate connects and selections are blocked while a request is running', async () => {
  const pending = deferred(); let calls = 0;
  const flow = await setup({ beginCalendarConnect: () => { calls++; return pending.promise; }, finishCalendarConnect: async () => ({ ...snapshot, revision: 2 }) });
  const connection = flow.connect(); assert.equal(await flow.connect(), false);
  assert.equal(await flow.select('two', true), false);
  pending.resolve({ attemptId: 'one' }); await connection;
  assert.equal(calls, 1);
});

test('unconfigured builds do not start authorization', async () => {
  let calls = 0; const flow = await setup({ readCalendarSettings: async () => ({ ...snapshot, configured: false }), beginCalendarConnect: async () => { calls++; } });
  assert.equal(await flow.connect(), false); assert.equal(calls, 0);
});

test('countdown handles five-minute, second, expired, and invalid values', () => {
  assert.equal(reminderCountdown(300_000, 0), '5분 후');
  assert.equal(reminderCountdown(59_001, 0), '60초 후');
  assert.equal(reminderCountdown(1, 2), '시작 시간');
  assert.equal(reminderCountdown(NaN, 0), '시작 시간');
});
