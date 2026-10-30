import test from 'node:test';
import assert from 'node:assert/strict';
import { nameUnnamedAttendees, attendeeInputAction, addPeople, attendeeLabel, createAttendeeFlow, createPeopleCatalogFlow } from '../src/people-flow.js';

const empty = () => ({ people: [], groups: [], editVersion: 'catalog:1' });
const meeting = (id = 'meeting', attendees = [], editVersion = 'meeting:1') => ({ meeting: { id, attendees }, editVersion });
const wait = async predicate => { for (let i = 0; i < 50; i++) { if (predicate()) return; await new Promise(resolve => setTimeout(resolve, 2)); } assert.fail('State did not settle'); };

test('adding a group preserves existing names, skips duplicates and never assigns a speaker', () => {
  const attendees = [{ id: 'a', personId: 'p', name: '회의 당시 이름' }]; let id = 0;
  const result = addPeople(attendees, [{ id: 'p', name: '변경된 이름' }, { id: 'q', name: '이서연' }, { id: 'q', name: '중복' }], () => String(++id));
  assert.deepEqual(result, [...attendees, { id: '1', personId: 'q', name: '이서연' }]);
  assert.equal(attendeeLabel({ id: 'guest' }, [{id:'first'}, {id:'guest'}]), '참석자 2');
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

test('a completed external write refreshes the catalog after an older in-flight read', async () => {
  let finishFirst, reads = 0;
  const updated = { ...empty(), editVersion: 'catalog:2', people: [{ id: 'p', name: '저장된 이름' }] };
  const flow = createPeopleCatalogFlow({ bridge: { readPeopleCatalog: () => ++reads === 1 ? new Promise(resolve => { finishFirst = resolve; }) : Promise.resolve(updated) } });
  flow.setActive(true);
  flow.refreshIfClean();
  finishFirst(empty());
  await wait(() => !flow.getSnapshot().busy);
  assert.equal(reads, 2);
  assert.equal(flow.getSnapshot().catalog.editVersion, 'catalog:2');
});

test('completed-write refresh signals coalesce while busy and a failed refresh does not loop', async () => {
  let finishFirst, reads = 0;
  const flow = createPeopleCatalogFlow({ bridge: { readPeopleCatalog: () => ++reads === 1 ? new Promise(resolve => { finishFirst = resolve; }) : Promise.reject(Error('목록 조회 실패')) } });
  flow.setActive(true);
  flow.refreshIfClean(); flow.refreshIfClean(); flow.refreshIfClean();
  finishFirst(empty());
  await wait(() => !flow.getSnapshot().busy);
  assert.equal(reads, 2);
  assert.match(flow.getSnapshot().error, /조회 실패/);
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(reads, 2);
});

test('a completed external write during a successful catalog save is refreshed after saving', async () => {
  let save, reads = 0;
  const current = { ...empty(), people: [{ id: 'p', name: '기존 이름' }] };
  const newest = { ...current, editVersion: 'catalog:3', people: [{ id: 'p', name: '다른 화면에서 저장한 이름' }] };
  const flow = createPeopleCatalogFlow({ bridge: {
    readPeopleCatalog: async () => ++reads === 1 ? current : newest,
    replacePeopleCatalog: () => new Promise(resolve => { save = resolve; }),
  } });
  flow.setActive(true); await wait(() => !flow.getSnapshot().busy);
  flow.edit('person', current.people[0]); flow.change({ name: '내가 저장한 이름' });
  const saving = flow.save(); flow.refreshIfClean();
  save({ ...current, editVersion: 'catalog:2', people: [{ id: 'p', name: '내가 저장한 이름' }] });
  await saving; await wait(() => !flow.getSnapshot().busy);
  assert.equal(reads, 2);
  assert.equal(flow.getSnapshot().catalog.editVersion, 'catalog:3');
  flow.edit('person', newest.people[0]); flow.change({ name: '미저장 초안' });
  flow.refreshIfClean();
  assert.equal(reads, 2);
  assert.equal(flow.getSnapshot().draft.name, '미저장 초안');
});

test('initial roster read failure can retry the same meeting without closing the dialog', async () => {
  let fail = true;
  const reads = [];
  const flow = createAttendeeFlow({ bridge: {
    readMeetingAttendees: async id => { reads.push(id); if (fail) throw Error('읽기 실패'); return meeting(id); },
    readPeopleCatalog: async () => empty(),
  } });
  await flow.open('retry-meeting');
  assert.equal(flow.getSnapshot().meeting, null);
  assert.equal(flow.getSnapshot().open, true);
  fail = false;
  assert.equal(await flow.reload(), true);
  assert.equal(flow.getSnapshot().meeting.id, 'retry-meeting');
  assert.equal(flow.getSnapshot().error, '');
  assert.deepEqual(reads, ['retry-meeting', 'retry-meeting']);
});

test('initial reads can be cancelled and their late reply cannot populate a later meeting', async () => {
  let resolveOld;
  const flow = createAttendeeFlow({ bridge: {
    readMeetingAttendees: id => id === 'old' ? new Promise(resolve => { resolveOld = resolve; }) : Promise.resolve(meeting(id)),
    readPeopleCatalog: async () => empty(),
  } });
  const opening = flow.open('old');
  assert.equal(flow.getSnapshot().loading, true);
  assert.equal(flow.close(), true);
  assert.equal(flow.getSnapshot().open, false);
  await flow.open('new');
  resolveOld(meeting('old')); await opening;
  assert.equal(flow.getSnapshot().meeting.id, 'new');
  assert.equal(flow.getSnapshot().loading, false);
});

test('pending attendee writes remain protected from dismissal', async () => {
  let resolveSave;
  const flow = createAttendeeFlow({ bridge: {
    readMeetingAttendees: async () => meeting(), readPeopleCatalog: async () => empty(),
    replaceMeetingAttendees: () => new Promise(resolve => { resolveSave = resolve; }),
  }, uuid: () => 'new' });
  await flow.open('meeting'); flow.addTemporary(); const saving = flow.save();
  assert.equal(flow.getSnapshot().loading, false); assert.equal(flow.close(), false);
  resolveSave(meeting('meeting', [{ id: 'new' }], 'meeting:2')); await saving;
  assert.equal(flow.close(), true);
});

test('unnamed attendees count independently of saved and named participants',()=>{
 const attendees=[{id:'saved',personId:'p',name:'김민서'},{id:'named',name:'방문자'},{id:'a'},{id:'b'}];
 assert.equal(attendeeLabel(attendees[2],attendees),'참석자 1');
 assert.equal(attendeeLabel(attendees[3],attendees),'참석자 2');
});

test('typed names resolve exact groups and people consistently without ambiguous creation',()=>{
 const catalog={people:[{id:'a',name:'민서'},{id:'b',name:'도윤'}],groups:[{id:'g',name:'제품팀',personIds:['a','b']}]};
 assert.deepEqual(attendeeInputAction(catalog,[],' 제품팀 '),{kind:'people',people:catalog.people});
 assert.deepEqual(attendeeInputAction(catalog,[],'민서'),{kind:'people',people:[catalog.people[0]]});
 assert.deepEqual(attendeeInputAction(catalog,[],'새 이름'),{kind:'create',name:'새 이름'});
 assert.equal(attendeeInputAction(catalog,[{name:'새 이름'}],'새 이름'),null);
 assert.equal(attendeeInputAction({...catalog,groups:[...catalog.groups,{id:'same',name:'민서',personIds:[]}]},[],'민서'),null);
 assert.equal(attendeeInputAction(catalog,[],' '),null);
});

test('temporary names start at one and remain stable after other participants are renamed or removed',()=>{
 const first=nameUnnamedAttendees([{id:'p',personId:'person',name:'민서'},{id:'a'},{id:'b'}]);
 assert.equal(first[1].name,'참석자 1'); assert.equal(first[2].name,'참석자 2');
 const changed=nameUnnamedAttendees([{...first[1],name:'방문자'},first[2],{id:'new'}]);
 assert.equal(changed[1].name,'참석자 2');
 assert.equal(changed[2].name,'참석자 1');
 assert.equal(attendeeInputAction({people:[],groups:[]},[{name:' 방문자 '}],'방문자'),null);
});
