import assert from 'node:assert/strict';
import test from 'node:test';
import { createCalendarMeetingFlow, calendarJoinUrls, calendarOccurrenceTime, calendarDay, parseCalendarDay, moveCalendarDay } from '../src/calendar-meeting-flow.js';

const flush = () => new Promise(resolve => setImmediate(resolve));
const event = { key: 'one', title: 'Synthetic schedule', startAt: 1_000_000, endAt: 2_000_000, calendarName: 'Synthetic', joinUrls: [] };
const result = query => ({ calendar: { connected: true, selectedCalendars: ['one'] }, occurrences: [event], ...query, scopeRevision: 7 });
const deferred = () => { let resolve, reject; const promise = new Promise((done, fail) => { resolve = done; reject = fail; }); return { promise, resolve, reject }; };
const create = options => createCalendarMeetingFlow({ now: () => new Date(2026, 9, 29, 12), timeZone: () => 'Asia/Seoul', ...options });

test('today includes ended events, selection is read only, and explicit preparation binds the selected date and scope', async () => {
  const queries = [], calls = [], opened = [];
  const flow = create({ bridge: { readCalendarMeetingOccurrences: async query => { queries.push(query); return result(query); }, createMeetingFromCalendar: async (key, scope) => { calls.push({ key, scope }); return { id: 'existing', title: 'Manual title', notes: 'Manual notes' }; } }, onPrepared: meeting => opened.push(meeting) });
  flow.open(); await flush();
  assert.deepEqual(queries, [{ day: '2026-10-29', timeZone: 'Asia/Seoul' }]);
  assert.equal(flow.getSnapshot().occurrences[0].endAt < Date.now(), true);
  assert.deepEqual(calls, []);
  assert.equal(await flow.prepare(), false); assert.equal(flow.select('unknown'), false);
  flow.select('one'); assert.deepEqual(calls, []);
  assert.equal(await flow.prepare(), true);
  assert.deepEqual(calls, [{ key: 'one', scope: { day: '2026-10-29', timeZone: 'Asia/Seoul', scopeRevision: 7 } }]);
  assert.equal(opened[0].notes, 'Manual notes'); assert.equal(flow.getSnapshot().open, false);
});

test('a closed picker ignores late results and reopens at today without its previous selection', async () => {
  const pending = deferred(); let reads = 0; let first;
  const flow = create({ bridge: { readCalendarMeetingOccurrences: query => { if (++reads === 1) { first = query; return pending.promise; } return Promise.resolve(result(query)); } }, onPrepared() {} });
  flow.open(); flow.close(); flow.open(); await flush(); flow.select('one');
  pending.resolve({ ...result(first), occurrences: [] }); await flush();
  assert.equal(flow.getSnapshot().selectedKey, 'one');
  await flow.moveDay(-1); flow.close(); flow.open(); await flush();
  assert.equal(flow.getSnapshot().day, '2026-10-29'); assert.equal(flow.getSnapshot().selectedKey, null);
});

test('date changes supersede pending requests, discard old selection, and ignore late errors', async () => {
  const first = deferred(), second = deferred(), queries = [];
  const flow = create({ bridge: { readCalendarMeetingOccurrences: query => { queries.push(query); return queries.length === 1 ? first.promise : second.promise; } }, onPrepared() {} });
  flow.open(); const move = flow.moveDay(-1);
  assert.equal(flow.getSnapshot().day, '2026-10-28'); assert.equal(flow.getSnapshot().selectedKey, null);
  second.resolve(result(queries[1])); await move; flow.select('one');
  first.reject(Error('older request failed')); await flush();
  assert.equal(flow.getSnapshot().selectedKey, 'one'); assert.equal(flow.getSnapshot().error, '');
  assert.equal(flow.getSnapshot().day, '2026-10-28');
});

test('preparation blocks duplicate clicks, closing, refresh, and day changes until its result is handled', async () => {
  const pending = deferred(); let calls = 0;
  const flow = create({ bridge: { readCalendarMeetingOccurrences: async query => result(query), createMeetingFromCalendar: () => { calls++; return pending.promise; } }, onPrepared() {} });
  flow.open(); await flush(); flow.select('one'); const preparing = flow.prepare();
  assert.equal(await flow.prepare(), false); assert.equal(flow.close(), false);
  assert.equal(await flow.refresh(), false); assert.equal(await flow.moveDay(1), false); assert.equal(flow.select('one'), false);
  pending.resolve({ id: 'one' }); await preparing; assert.equal(calls, 1);
});

test('remote deletion or changed calendar scope leaves the picker open without replacing existing meeting fields', async () => {
  let calls = 0;
  const flow = create({ bridge: { readCalendarMeetingOccurrences: async query => result(query), createMeetingFromCalendar: async () => { if (++calls === 1) throw Error('캘린더 선택이 변경되었습니다.'); return { id: 'one', deletedAt: 1 }; } }, onPrepared: meeting => assert.equal(meeting.deletedAt, 1) });
  flow.open(); await flush(); flow.select('one');
  assert.equal(await flow.prepare(), false); assert.equal(flow.getSnapshot().error, '캘린더 선택이 변경되었습니다.');
  assert.equal(flow.getSnapshot().open, true); assert.equal(await flow.prepare(), true);
});

test('a mismatched native response is not presented as the selected day', async () => {
  const flow = create({ bridge: { readCalendarMeetingOccurrences: async query => ({ ...result(query), day: '2026-10-30' }) }, onPrepared() {} });
  flow.open(); await flush();
  assert.deepEqual(flow.getSnapshot().occurrences, []); assert.match(flow.getSnapshot().error, /조회한 날짜/);
});

test('specific past and future dates remain bounded to one requested day and today restores the local date', async () => {
  const queries = [];
  const flow = create({ bridge: { readCalendarMeetingOccurrences: async query => { queries.push(query); return result(query); } }, onPrepared() {} });
  flow.open(); await flush(); await flow.setDay('2024-02-29'); await flow.setDay('2030-01-01'); await flow.today();
  assert.deepEqual(queries.map(query => query.day), ['2026-10-29', '2024-02-29', '2030-01-01', '2026-10-29']);
  assert.equal(await flow.setDay('2024-02-30'), false); assert.equal(queries.length, 4);
});

test('calendar day arithmetic follows civil dates across month, leap-year, and DST boundaries', () => {
  assert.equal(moveCalendarDay('2026-03-08', 1), '2026-03-09');
  assert.equal(moveCalendarDay('2026-11-01', -1), '2026-10-31');
  assert.equal(moveCalendarDay('2024-03-01', -1), '2024-02-29');
  assert.equal(moveCalendarDay('2026-12-31', 1), '2027-01-01');
  assert.equal(parseCalendarDay('2026-02-29'), null); assert.equal(parseCalendarDay('invalid'), null);
  assert.equal(calendarDay(new Date(2026, 0, 2, 0, 15)), '2026-01-02');
});

test('configuration is explicit and link references only expose credential-free https URLs', async () => {
  let configured = 0;
  const flow = create({ bridge: { readCalendarMeetingOccurrences: async query => ({ ...result(query), calendar: { connected: false }, occurrences: [] }) }, onPrepared() {}, onConfigure() { configured++; } });
  flow.open(); await flush(); flow.configure();
  assert.equal(configured, 1); assert.equal(flow.getSnapshot().open, false);
  assert.deepEqual(calendarJoinUrls({ joinUrls: ['javascript:alert(1)', 'https://user:password@example.test', 'https://example.test/join'] }), ['https://example.test/join']);
  assert.match(calendarOccurrenceTime(event), / - /);
});
