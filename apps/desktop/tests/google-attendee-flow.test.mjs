import test from 'node:test';
import assert from 'node:assert/strict';
import { createAttendeeFlow } from '../src/people-flow.js';
import { googleTargetFor } from '../src/google-attendee-draft.js';

const clone = value => JSON.parse(JSON.stringify(value));
const source = (key, fields = {}) => ({ sourceKey: key, origin: 'meetParticipant', kind: 'signedInPersonOrRoom', displayName: 'Same name', email: null, googleUserId: `users/${key}`, responseStatus: null, ...fields });
const initial = () => ({ editVersion: 'roster:1', meeting: { id: 'meeting', notes: 'Saved notes', attendees: [{ id: 'manual', name: 'Manual name' }] }, sources: [], identityLinks: [], catalog: { editVersion: 'catalog:1', people: [{ id: 'person', name: 'Saved person' }], groups: [] } });

function fixture(overrides = {}, original = initial()) {
  let stored = clone(original), sequence = 0, changed = 0;
  const writes = [];
  const bridge = {
    readGoogleAttendeeRoster: async () => clone(stored),
    fetchGoogleAttendees: async () => ({ sources: [source('100'), source('200')], meetStatus: 'available', calendarComplete: true }),
    applyMeetingAttendeeDraft: async (id, version, draft) => {
      writes.push(clone({ id, version, draft }));
      assert.equal(version, stored.editVersion);
      stored = { ...stored, editVersion: 'roster:2', meeting: { ...stored.meeting, attendees: clone(draft.attendees) }, sources: clone(draft.sources), catalog: { ...stored.catalog, people: [...stored.catalog.people, ...draft.promotions.map(item => item.person)] } };
      return clone(stored);
    },
    ...overrides,
  };
  const flow = createAttendeeFlow({ bridge, uuid: () => `new-${++sequence}`, onChanged: () => { changed++; } });
  return { flow, bridge, writes, stored: () => stored, changed: () => changed };
}

test('failed Google reads and dialog cancellation never save manual edits or staged promotions', async () => {
  const f = fixture({ fetchGoogleAttendees: async () => { throw Error('Google unavailable'); } });
  await f.flow.open('meeting'); f.flow.rename('manual', 'Draft name');
  assert.equal(await f.flow.promote('manual', 'New saved person'), true);
  assert.equal(await f.flow.loadGoogle(), false);
  assert.equal(f.flow.getSnapshot().attendees[0].name, 'New saved person');
  assert.equal(f.writes.length, 0);
  assert.equal(f.flow.isDirty(), true);
  f.flow.close(); await f.flow.open('meeting');
  assert.deepEqual(f.flow.getSnapshot().attendees, initial().meeting.attendees);
  assert.equal(f.flow.getSnapshot().catalog.people.length, 1);
  assert.equal(f.flow.getSnapshot().promotions.length, 0);
});

test('same display names never preselect a person, and consecutive Google additions stay in one draft', async () => {
  const f = fixture(); await f.flow.open('meeting'); await f.flow.loadGoogle();
  assert.equal(f.flow.getSnapshot().attendees.length, 1);
  for (const row of f.flow.getSnapshot().googleResult.sources) {
    const state = f.flow.getSnapshot();
    assert.equal(googleTargetFor(state, state.catalog, state.identityLinks, row), 'new');
    f.flow.chooseGoogle(row, 'new');
  }
  assert.equal(f.flow.getSnapshot().attendees.length, 3);
  assert.equal(f.writes.length, 0);
  const current = f.flow.getSnapshot(), row = current.googleResult.sources[0];
  assert.equal(googleTargetFor(current, current.catalog, current.identityLinks, row), 'attendee:new-1');
  f.flow.chooseGoogle(row, 'attendee:new-1');
  assert.equal(f.flow.getSnapshot().attendees.length, 3);
  assert.equal(await f.flow.save(), true);
  assert.equal(f.writes.length, 1); assert.equal(f.changed(), 1);
  assert.equal(f.writes[0].draft.personLinks.length, 0);
  assert.equal(f.stored().meeting.notes, 'Saved notes');
});

test('existing temporary and saved attendees retain IDs and names when sources are linked', async () => {
  const f = fixture(); await f.flow.open('meeting');
  f.flow.add([{ id: 'person', name: 'Saved person' }]);
  const savedId = f.flow.getSnapshot().attendees.find(item => item.personId).id;
  const first = source('100'), second = source('200');
  f.flow.chooseGoogle(first, 'attendee:manual');
  f.flow.chooseGoogle(second, `attendee:${savedId}`);
  assert.deepEqual(f.flow.getSnapshot().attendees, [{ id: 'manual', name: 'Manual name' }, { id: savedId, personId: 'person', name: 'Saved person' }]);
  assert.deepEqual(f.flow.getSnapshot().personLinks, [{ attendeeId: savedId, personId: 'person' }]);
  f.flow.chooseGoogle(source('rejoined', { googleUserId: first.googleUserId }), 'attendee:manual');
  assert.equal(f.flow.getSnapshot().attendees.length, 2);
  assert.equal(f.flow.getSnapshot().sources.filter(entry => entry.attendeeId === 'manual').length, 2);
});

test('one failed CAS preserves manual, Google and promotion changes without another write path', async () => {
  let calls = 0;
  const f = fixture({ applyMeetingAttendeeDraft: async () => { calls++; throw Error('Version conflict'); }, replaceMeetingAttendees: async () => assert.fail('Must not save a subset'), promoteMeetingAttendee: async () => assert.fail('Must not save a subset') });
  await f.flow.open('meeting'); f.flow.rename('manual', 'Changed');
  f.flow.chooseGoogle(source('100'), 'new'); await f.flow.promote('new-1', 'Promoted person');
  const draft = clone(f.flow.getSnapshot());
  assert.equal(await f.flow.save(), false); assert.equal(calls, 1);
  const result = f.flow.getSnapshot();
  for (const key of ['attendees', 'sources', 'promotions', 'personLinks']) assert.deepEqual(result[key], draft[key]);
  assert.equal(f.flow.isDirty(), true); assert.deepEqual(f.stored(), initial());
});

test('a repeated fetch updates source metadata while retaining manual names and removed attendees', async () => {
  const original = initial(); original.sources = [{ attendeeId: 'manual', source: source('100') }];
  const f = fixture({ fetchGoogleAttendees: async () => ({ sources: [source('100', { displayName: 'Provider rename' })], meetStatus: 'available' }) }, original);
  await f.flow.open('meeting'); f.flow.rename('manual', 'Manual rename'); await f.flow.loadGoogle();
  assert.equal(f.flow.getSnapshot().sources[0].source.displayName, 'Provider rename');
  assert.equal(f.flow.getSnapshot().attendees[0].name, 'Manual rename');
  f.flow.remove('manual'); await f.flow.loadGoogle();
  assert.equal(f.flow.getSnapshot().attendees.length, 0); assert.equal(f.flow.getSnapshot().sources.length, 1);
});

test('late Google replies cannot replace the draft after closing and opening another meeting', async () => {
  let finish;
  const f = fixture({ fetchGoogleAttendees: () => new Promise(resolve => { finish = resolve; }) });
  await f.flow.open('meeting'); const pending = f.flow.loadGoogle();
  assert.equal(f.flow.close(), true); await f.flow.open('another');
  finish({ sources: [source('100')], meetStatus: 'available' }); await pending;
  assert.equal(f.flow.getSnapshot().googleResult, null); assert.equal(f.writes.length, 0);
});

test('cancelling Meet while the browser is opening cancels the exact attempt and preserves the draft', async () => {
  let begin, finish; const cancelled = [];
  const f = fixture({ beginMeetConnect: () => new Promise(resolve => { begin = resolve; }), finishCalendarConnect: () => new Promise(resolve => { finish = resolve; }), cancelCalendarConnect: async id => cancelled.push(id) });
  await f.flow.open('meeting'); f.flow.rename('manual', 'Draft name');
  const connecting = f.flow.connectMeet(), cancelling = f.flow.cancelMeetConnect();
  assert.equal(f.flow.close(), false);
  begin({ attemptId: 'attempt-1' }); await connecting; await cancelling;
  assert.deepEqual(cancelled, ['attempt-1']); assert.equal(finish, undefined);
  assert.equal(f.flow.getSnapshot().attendees[0].name, 'Draft name');
  assert.equal(f.flow.getSnapshot().googleConnecting, false); assert.equal(f.writes.length, 0);
});

test('a failed Meet cancellation retries the same attempt rather than losing it', async () => {
  let finish; const cancelled = [];
  const f = fixture({ beginMeetConnect: async () => ({ attemptId: 'attempt-1' }), finishCalendarConnect: () => new Promise(resolve => { finish = resolve; }), cancelCalendarConnect: async id => { cancelled.push(id); if (cancelled.length === 1) throw Error('Retry cancellation'); } });
  await f.flow.open('meeting'); const connecting = f.flow.connectMeet();
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(await f.flow.cancelMeetConnect(), false); assert.equal(f.flow.close(), false);
  assert.equal(await f.flow.cancelMeetConnect(), true);
  assert.deepEqual(cancelled, ['attempt-1', 'attempt-1']);
  finish({ meetConnected: true }); await connecting;
  assert.equal(f.flow.getSnapshot().googleResult, null); assert.equal(f.writes.length, 0);
});

test('a staged saved-person choice can change before Apply without leaving the first person selected', async () => {
  const original = initial(); original.catalog.people.push({ id: 'other', name: 'Other person' });
  const f = fixture({}, original); await f.flow.open('meeting');
  const row = source('100');
  assert.equal(f.flow.chooseGoogle(row, 'person:person'), true);
  assert.equal(f.flow.chooseGoogle(row, 'person:other'), true);
  assert.equal(f.writes.length, 0);
  assert.deepEqual(f.flow.getSnapshot().attendees.map(attendee => attendee.personId).filter(Boolean), ['other']);
  assert.deepEqual(f.flow.getSnapshot().personLinks, [{ attendeeId: 'new-1', personId: 'other' }]);
});

test('linking a Google temporary attendee to an unselected saved person preserves its ID and manual name', async () => {
  const f = fixture(); await f.flow.open('meeting');
  const row = source('100'); f.flow.chooseGoogle(row, 'new');
  f.flow.rename('new-1', 'Manual Google attendee name');
  assert.equal(f.flow.chooseGoogle(row, 'person:person'), true);
  assert.deepEqual(f.flow.getSnapshot().attendees.find(attendee => attendee.personId), { id: 'new-1', personId: 'person', name: 'Manual Google attendee name' });
  assert.equal(f.flow.getSnapshot().sources[0].attendeeId, 'new-1');
});

test('switching a staged promotion to an existing person removes the uncommitted promoted person', async () => {
  const f = fixture(); await f.flow.open('meeting'); const row = source('100');
  f.flow.chooseGoogle(row, 'new'); await f.flow.promote('new-1', 'Uncommitted person');
  assert.equal(f.flow.getSnapshot().promotions.length, 1);
  assert.equal(f.flow.chooseGoogle(row, 'person:person'), true);
  assert.equal(f.flow.getSnapshot().promotions.length, 0);
  assert.deepEqual(f.flow.getSnapshot().catalog.people, initial().catalog.people);
  assert.equal(await f.flow.save(), true);
  assert.equal(f.writes[0].draft.promotions.length, 0);
  assert.equal(f.writes[0].draft.attendees.find(attendee => attendee.id === 'new-1').personId, 'person');
});
