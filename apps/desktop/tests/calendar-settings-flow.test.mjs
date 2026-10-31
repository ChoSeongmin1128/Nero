import assert from 'node:assert/strict';
import test from 'node:test';
import { createCalendarSettingsFlow, reminderCountdown, calendarConnectionView } from '../src/calendar-settings-flow.js';

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

test('retry waits for cancellation and rejects old authorization completion', async () => {
  const old=deferred(), cancelled=deferred(); let begins=0;
  const flow=await setup({beginCalendarConnect:async()=>({attemptId:String(++begins)}),finishCalendarConnect:id=>id==='1'?old.promise:Promise.resolve({...snapshot,revision:5}),cancelCalendarConnect:()=>cancelled.promise});
  const first=flow.connect(); await flush(); const retry=flow.retry(); await flush();
  assert.equal(begins,1); assert.equal(flow.getSnapshot().phase,'cancelling');
  cancelled.resolve({...snapshot,revision:3}); await retry;
  old.resolve({...snapshot,revision:4,accountLabel:'stale'}); await first;
  assert.equal(begins,2); assert.equal(flow.getSnapshot().snapshot.accountLabel,'synthetic');
});
test('browser reopen reuses only the current attempt without another begin', async () => {
  const finishing=deferred(); const reopened=[]; let begins=0;
  const flow=await setup({beginCalendarConnect:async()=>({attemptId:String(++begins)}),finishCalendarConnect:()=>finishing.promise,reopenCalendarConnect:async id=>reopened.push(id)});
  const first=flow.connect(); await flush(); await flow.reopen();
  assert.deepEqual(reopened,['1']); assert.equal(begins,1);
  finishing.resolve({...snapshot,revision:3}); await first;
  assert.equal(await flow.reopen(),false);
});
test('failed cancellation blocks a new request until cancellation succeeds', async () => {
  const finishing=deferred(); let begins=0, cancels=0;
  const flow=await setup({beginCalendarConnect:async()=>({attemptId:String(++begins)}),finishCalendarConnect:()=>finishing.promise,cancelCalendarConnect:async()=>{if(++cancels===1)throw Error('취소 실패');return {...snapshot,revision:3};}});
  const first=flow.connect(); await flush(); assert.equal(await flow.cancel(),false);
  assert.equal(await flow.connect(),false); assert.equal(begins,1);
  assert.equal(await flow.cancel(),true);
  finishing.resolve({...snapshot,revision:2}); await first;
});
test('idle cancellation releases its barrier for later connection', async () => {
  const flow=await setup({beginCalendarConnect:async()=>({attemptId:'next'}),finishCalendarConnect:async()=>({...snapshot,revision:4})});
  await flow.cancel(); assert.equal(await flow.connect(),true);
});

test('connection presentation prioritizes active authorization and hides previous failure', () => {
  const state={snapshot:{...snapshot,error:'old failure'},phase:'authorizing',connecting:true,error:''};
  assert.equal(calendarConnectionView(state).label,'브라우저에서 승인 중');
  assert.equal(calendarConnectionView(state).error,'');
  assert.equal(calendarConnectionView({...state,connecting:false,phase:'failed',error:'timed out'}).action,'다시 시도');
  assert.equal(calendarConnectionView({...state,connecting:false,phase:'cancelling'}).label,'연결 취소 중');
});

test('leaving during failed browser launch releases the request for another connection', async () => {
  let failBegin; let calls=0;
  const flow=await setup({beginCalendarConnect:()=>{calls++;return calls===1?new Promise((_,reject)=>{failBegin=reject;}):Promise.resolve({attemptId:'new'});},finishCalendarConnect:async()=>({...snapshot,revision:4})});
  const pending=flow.connect();flow.setActive(false);failBegin(Error('browser failed'));await pending;await flush();
  flow.setActive(true);await flush();assert.equal(await flow.connect(),true);
});
test('successful authorization clears a browser-reopen error', async () => {
  const finish=deferred();
  const flow=await setup({beginCalendarConnect:async()=>({attemptId:'one'}),finishCalendarConnect:()=>finish.promise,reopenCalendarConnect:async()=>{throw Error('browser failed');}});
  const pending=flow.connect();await flush();await flow.reopen();assert.equal(flow.getSnapshot().error,'browser failed');
  finish.resolve({...snapshot,revision:4});await pending;assert.equal(flow.getSnapshot().error,'');
});


test('a healthy calendar connection offers no reconnect action', () => {
  const view=calendarConnectionView({phase:'idle',connecting:false,error:'',snapshot:{connected:true,accountLabel:'Example',restoring:false}});
  assert.equal(view.label,'Example'); assert.equal(view.action,null);
  assert.equal(calendarConnectionView({phase:'failed',connecting:false,error:'Failed',snapshot:{connected:false}}).action,'다시 시도');
});
