import test from 'node:test';
import assert from 'node:assert/strict';
import { addPeople, attendeeLabel, createAttendeeFlow, createPeopleCatalogFlow } from '../src/people-flow.js';

const empty = () => ({ people: [], groups: [], editVersion: 'catalog:1' });
const meeting = (id = 'meeting', attendees = [], editVersion = 'meeting:1') => ({ meeting: { id, attendees }, editVersion });
const wait = async predicate => { for (let i = 0; i < 50; i++) { if (predicate()) return; await new Promise(resolve => setTimeout(resolve, 2)); } assert.fail('State did not settle'); };

test('adding a group preserves existing names, skips duplicates and never assigns a speaker', () => {
  const attendees = [{ id: 'a', personId: 'p', name: '회의 당시 이름' }]; let id = 0;
  const result = addPeople(attendees, [{ id: 'p', name: '변경된 이름' }, { id: 'q', name: '이서연' }, { id: 'q', name: '중복' }], () => String(++id));
  assert.deepEqual(result, [...attendees, { id: '1', personId: 'q', name: '이서연' }]);
  assert.equal(attendeeLabel({ id: 'guest' }, 1), '참석자 2');
  assert.equal(id, 1);
});

test('recording roster changes save against the read version and do not modify the catalog', async () => {
  const writes = []; let changed = 0;
  const flow = createAttendeeFlow({ bridge: { readMeetingAttendees: async () => meeting(), readPeopleCatalog: async () => empty(), replaceMeetingAttendees: async (...args) => { writes.push(args); return meeting('meeting', args[2], 'meeting:2'); } }, uuid: () => 'guest', onChanged: () => { changed++; } });
  await flow.open('meeting'); flow.addTemporary(); flow.rename('guest', '방문자');
  assert.equal(flow.isDirty(), true);
  assert.equal(await flow.save(), true);
  assert.deepEqual(writes, [['meeting', 'meeting:1', [{ id: 'guest', name: '방문자' }]]]);
  assert.equal(changed, 1); assert.equal(flow.isDirty(), false); assert.deepEqual(flow.getSnapshot().catalog, empty());
});

test('a conflicting roster save preserves the local draft and does not pretend it was applied', async () => {
  const flow = createAttendeeFlow({ bridge: { readMeetingAttendees: async () => meeting(), readPeopleCatalog: async () => empty(), replaceMeetingAttendees: async () => { throw Error('회의가 변경됐습니다.'); } }, uuid: () => 'guest' });
  await flow.open('meeting'); flow.addTemporary();
  assert.equal(await flow.save(), false); assert.equal(flow.isDirty(), true);
  assert.equal(flow.getSnapshot().attendees.length, 1); assert.match(flow.getSnapshot().error, /변경/);
});

test('promotion saves pending meeting changes then uses both latest revisions; failure retains the saved roster', async () => {
  const calls = [];
  const flow = createAttendeeFlow({ bridge: { readMeetingAttendees: async () => meeting(), readPeopleCatalog: async () => empty(), replaceMeetingAttendees: async (id, version, attendees) => meeting(id, attendees, 'meeting:2'), promoteMeetingAttendee: async (...args) => { calls.push(args); throw Error('인원 목록이 변경됐습니다.'); } }, uuid: () => 'guest' });
  await flow.open('meeting'); flow.addTemporary(); flow.rename('guest', '김하늘');
  assert.equal(await flow.promote('guest', ' 김하늘 '), false);
  assert.deepEqual(calls, [['meeting', 'meeting:2', 'catalog:1', 'guest', '김하늘']]);
  assert.equal(flow.isDirty(), false); assert.equal(flow.getSnapshot().attendees[0].name, '김하늘');
});

test('a slow meeting request cannot replace a different meeting opened later', async () => {
  let complete;
  const flow = createAttendeeFlow({ bridge: { readMeetingAttendees: id => id === 'old' ? new Promise(resolve => { complete = resolve; }) : Promise.resolve(meeting('new')), readPeopleCatalog: async () => empty() } });
  const pending = flow.open('old'); await flow.open('new'); complete(meeting('old')); await pending;
  assert.equal(flow.getSnapshot().meeting.id, 'new'); assert.equal(flow.getSnapshot().busy, false);
});

test('deleting a person also removes group membership but not unrelated people or groups', async () => {
  let write;
  const catalog = { ...empty(), people: [{ id: 'p', name: '가' }, { id: 'q', name: '나' }], groups: [{ id: 'g', name: '팀', personIds: ['p', 'q'] }] };
  const flow = createPeopleCatalogFlow({ bridge: { readPeopleCatalog: async () => catalog, replacePeopleCatalog: async (...args) => { write = args; return { editVersion: 'catalog:2', people: args[1], groups: args[2] }; } } });
  flow.setActive(true); await wait(() => !flow.getSnapshot().busy);
  assert.equal(await flow.remove('person', 'p'), true);
  assert.deepEqual(write, ['catalog:1', [{ id: 'q', name: '나' }], [{ id: 'g', name: '팀', personIds: ['q'] }]]);
});

test('saving a renamed person keeps the editor open on a catalog conflict', async () => {
  const flow = createPeopleCatalogFlow({ bridge: { readPeopleCatalog: async () => ({ ...empty(), people: [{ id: 'p', name: '기존' }] }), replacePeopleCatalog: async () => { throw Error('다른 곳에서 수정했습니다.'); } } });
  flow.setActive(true); await wait(() => !flow.getSnapshot().busy); flow.edit('person', { id: 'p', name: '기존' }); flow.change({ name: '수정' });
  assert.equal(await flow.save(), false); assert.equal(flow.getSnapshot().draft.name, '수정'); assert.equal(flow.getSnapshot().catalog.people[0].name, '기존');
});
