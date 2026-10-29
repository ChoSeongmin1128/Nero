import assert from 'node:assert/strict';
import test from 'node:test';
import { createCalendarMeetingFlow, calendarJoinUrls, calendarOccurrenceTime } from '../src/calendar-meeting-flow.js';

const flush = () => new Promise(resolve => setImmediate(resolve));
const event = { key: 'one', title: 'Synthetic schedule', startAt: 1_000_000, endAt: 2_000_000, calendarName: 'Synthetic', joinUrls: [] };
const result = { calendar: { connected: true, selectedCalendars: ['one'] }, occurrences: [event] };
const deferred = () => { let resolve; const promise = new Promise(done => { resolve = done; }); return { promise, resolve }; };

test('loading and selection never create a meeting; explicit preparation sends only the occurrence key', async () => {
  const calls = [], opened = [];
  const flow = createCalendarMeetingFlow({ bridge: { readCalendarMeetingOccurrences: async () => result, createMeetingFromCalendar: async key => { calls.push(key); return { id: 'existing', title: 'Manual title', notes: 'Manual notes' }; } }, onPrepared: meeting => opened.push(meeting) });
  flow.open(); await flush();
  assert.deepEqual(calls, []);
  assert.equal(await flow.prepare(), false);
  assert.equal(flow.select('unknown'), false);
  flow.select('one'); assert.deepEqual(calls, []);
  assert.equal(await flow.prepare(), true);
  assert.deepEqual(calls, ['one']);
  assert.equal(opened[0].notes, 'Manual notes');
  assert.equal(flow.getSnapshot().open, false);
});

test('a closed picker ignores late event responses and never retains the previous selection', async () => {
  const pending = deferred(); let reads = 0;
  const flow = createCalendarMeetingFlow({ bridge: { readCalendarMeetingOccurrences: () => ++reads === 1 ? pending.promise : Promise.resolve(result) }, onPrepared() {} });
  flow.open(); flow.close(); flow.open(); await flush();
  flow.select('one'); pending.resolve({ ...result, occurrences: [] }); await flush();
  assert.equal(flow.getSnapshot().selectedKey, 'one');
  flow.close(); flow.open(); await flush();
  assert.equal(flow.getSnapshot().selectedKey, null);
});

test('preparation prevents duplicate clicks and closing until its result is handled', async () => {
  const pending = deferred(); let calls = 0;
  const flow = createCalendarMeetingFlow({ bridge: { readCalendarMeetingOccurrences: async () => result, createMeetingFromCalendar: () => { calls++; return pending.promise; } }, onPrepared() {} });
  flow.open(); await flush(); flow.select('one');
  const preparing = flow.prepare();
  assert.equal(await flow.prepare(), false); assert.equal(flow.close(), false);
  assert.equal(await flow.refresh(), false); assert.equal(flow.select('one'), false);
  pending.resolve({ id: 'one' }); await preparing; assert.equal(calls, 1);
});

test('provider validation errors retain the picker and retry safely by key', async () => {
  let calls = 0;
  const flow = createCalendarMeetingFlow({ bridge: { readCalendarMeetingOccurrences: async () => result, createMeetingFromCalendar: async () => { if (++calls === 1) throw Error('일정이 취소되었습니다.'); return { id: 'one', deletedAt: 1 }; } }, onPrepared: meeting => assert.equal(meeting.deletedAt, 1) });
  flow.open(); await flush(); flow.select('one');
  assert.equal(await flow.prepare(), false); assert.equal(flow.getSnapshot().error, '일정이 취소되었습니다.');
  assert.equal(flow.getSnapshot().open, true); assert.equal(await flow.prepare(), true);
});

test('configuration is explicit and link references only expose credential-free https URLs', async () => {
  let configured = 0;
  const flow = createCalendarMeetingFlow({ bridge: { readCalendarMeetingOccurrences: async () => ({ calendar: { connected: false }, occurrences: [] }) }, onPrepared() {}, onConfigure() { configured++; } });
  flow.open(); await flush(); flow.configure();
  assert.equal(configured, 1); assert.equal(flow.getSnapshot().open, false);
  assert.deepEqual(calendarJoinUrls({ joinUrls: ['javascript:alert(1)', 'https://user:password@example.test', 'https://example.test/join'] }), ['https://example.test/join']);
  assert.match(calendarOccurrenceTime(event), / - /);
});
