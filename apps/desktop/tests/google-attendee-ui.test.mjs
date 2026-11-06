import assert from 'node:assert/strict';
import test from 'node:test';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';
import { Window } from 'happy-dom';

const root = fileURLToPath(new URL('../', import.meta.url));
const bundle = await build({ entryPoints: [root + 'src/people-ui.jsx'], bundle: true, write: false, format: 'iife', globalName: 'PeopleUI', jsx: 'automatic', nodePaths: [root + 'node_modules'], loader: { '.css': 'empty' }, define: { 'process.env.NODE_ENV': '"production"' } });
const clone = value => JSON.parse(JSON.stringify(value));
const wait = async condition => { for (let n = 0; n < 100; n++) { await new Promise(resolve => setTimeout(resolve, 5)); if (condition()) return; } assert.fail('Attendee dialog did not settle'); };
const sources = [
  { sourceKey: 'invite', origin: 'calendarInvitation', kind: 'person', displayName: '초대 이름', email: 'synthetic@example.test', googleUserId: null, responseStatus: 'accepted' },
  { sourceKey: 'actual', origin: 'meetParticipant', kind: 'signedInPersonOrRoom', displayName: '실제 이름', email: null, googleUserId: 'users/100', responseStatus: null },
];

async function fixture(t, overrides = {}, roster = {}) {
  const window = new Window({ url: 'http://nero.test', settings: { enableJavaScriptEvaluation: true, suppressInsecureJavaScriptEnvironmentWarning: true } });
  window.document.body.innerHTML = '<button id="origin">참석자 열기</button>'; window.eval(bundle.outputFiles[0].text);
  let stored = { editVersion: 'roster:1', meeting: { id: 'meeting', calendarReference: { occurrenceKey: 'synthetic-event' }, attendees: [{ id: 'manual', name: '기존 임시' }] }, sources: [], identityLinks: [], catalog: { editVersion: 'catalog:1', people: [{ id: 'person', name: '저장된 사람' }], groups: [] }, ...roster };
  const writes = [], reads = [], cancelled = [];
  const bridge = {
    readGoogleAttendeeRoster: async () => clone(stored),
    fetchGoogleAttendees: async () => { reads.push(true); return { sources: clone(roster.googleSources || sources), calendarComplete: true, meetStatus: 'available' }; },
    applyMeetingAttendeeDraft: async (id, expectedVersion, draft) => {
      writes.push(clone({ id, expectedVersion, draft }));
      stored = { ...stored, editVersion: 'roster:2', meeting: { ...stored.meeting, attendees: clone(draft.attendees) }, sources: clone(draft.sources), catalog: { ...stored.catalog, people: [...stored.catalog.people, ...draft.promotions.map(entry => entry.person)] } };
      return clone(stored);
    },
    cancelCalendarConnect: async id => cancelled.push(id),
    readCalendarSettings: async () => ({ connected: true }),
    ...overrides,
  };
  const editor = window.PeopleUI.mountMeetingAttendees({ container: window.document.body, bridge });
  t.after(() => { editor.dispose(); window.happyDOM.abort(); });
  const open = async () => { await editor.open('meeting', window.document.querySelector('#origin')); await wait(() => window.document.querySelector('.attendee-chip')); };
  await open();
  const buttons = text => [...window.document.querySelectorAll('button')].filter(button => button.textContent.trim() === text && !button.disabled);
  const button = text => { const result = buttons(text); assert(result.length, `Missing button ${text}`); return result.at(-1); };
  const input = (element, value) => { Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value').set.call(element, value); element.dispatchEvent(new window.Event('input', { bubbles: true })); };
  const choose = async (key, label) => {
    const row = window.document.querySelector(`[data-source-key="${key}"]`); row.querySelector('[data-slot="select-trigger"]').click();
    await wait(() => window.document.querySelector('[data-slot="select-content"]:not([data-closed]) [role="option"]'));
    const option = [...window.document.querySelectorAll('[data-slot="select-content"]:not([data-closed]) [role="option"]')].find(node => node.textContent === label); assert(option, `Missing choice ${label}`); option.click();
    await wait(() => row.querySelector('[data-slot="select-trigger"]').textContent.includes(label));
  };
  return { window, editor, writes, reads, cancelled, open, button, buttons, input, choose, stored: () => stored };
}

test('Google accounts link to existing temporary or saved attendees inside one staged dialog', async t => {
  const f = await fixture(t); f.button('Google에서 추가').click();
  await wait(() => f.window.document.querySelectorAll('[data-source-key]').length === 2);
  assert(f.window.document.querySelector('[aria-label="불러온 참석자"]'));
  assert.equal(f.buttons('새로고침').length, 0);
  assert.equal(f.window.document.querySelectorAll('.attendee-chip').length, 1);
  assert.doesNotMatch(f.window.document.body.textContent, /users\/100|Google 계정 100/);
  await f.choose('invite', '기존 임시');
  await f.choose('actual', '저장된 사람');
  f.button('선택한 2명 추가').click();
  await wait(() => f.window.document.querySelectorAll('.attendee-chip').length === 2);
  await wait(() => f.window.document.querySelector('[aria-label="초대 이름 추가"]').disabled);
  assert.equal(f.writes.length, 0);
  f.window.document.querySelector('[aria-label="Google 참석자 목록 닫기"]').click();
  f.button('Google에서 추가').click(); await wait(() => f.reads.length === 2 && !f.button('Google에서 추가').disabled);
  assert.equal(f.window.document.querySelectorAll('.attendee-chip').length, 2);
  f.button('적용').click(); await wait(() => !f.window.document.querySelector('.people-dialog'));
  assert.equal(f.writes.length, 1); assert.equal(f.writes[0].expectedVersion, 'roster:1');
  assert.equal(f.writes[0].draft.sources.find(entry => entry.source.sourceKey === 'invite').attendeeId, 'manual');
  assert.equal(f.writes[0].draft.personLinks.length, 1);
  assert.equal(f.writes[0].draft.attendees[0].name, '기존 임시');
});

test('promotion cancel and parent discard preserve the stored roster and person catalog', async t => {
  const f = await fixture(t);
  f.window.document.querySelector('[aria-label="기존 임시 인원 목록에 저장"]').click();
  await wait(() => f.window.document.querySelector('.people-confirm input'));
  f.input(f.window.document.querySelector('.people-confirm input'), '취소할 이름'); f.button('취소').click();
  await wait(() => !f.window.document.querySelector('.people-confirm'));
  assert.equal(f.writes.length, 0);
  f.window.document.querySelector('[aria-label="기존 임시 인원 목록에 저장"]').click();
  await wait(() => f.window.document.querySelector('.people-confirm input'));
  f.input(f.window.document.querySelector('.people-confirm input'), '저장 초안');
  f.button('추가').click(); await wait(() => !f.window.document.querySelector('.people-confirm'));
  assert.equal(f.writes.length, 0);
  f.button('취소').click(); await wait(() => f.buttons('버리기').length); f.button('버리기').click();
  await wait(() => !f.window.document.querySelector('.people-dialog'));
  await f.open(); assert(f.window.document.querySelector('[aria-label="기존 임시 인원 목록에 저장"]'));
  assert.equal(f.stored().catalog.people.length, 1); assert.equal(f.writes.length, 0);
});

test('Google failure retains a manual rename and Escape keeps the discard choice explicit', async t => {
  const f = await fixture(t, { fetchGoogleAttendees: async () => { throw Error('Google 조회 실패'); } });
  f.window.document.querySelector('.attendee-chip-name').click(); await wait(() => f.window.document.querySelector('.attendee-chip input'));
  const name = f.window.document.querySelector('.attendee-chip input'); f.input(name, '미저장 이름'); name.blur();
  f.button('Google에서 추가').click(); await wait(() => f.window.document.body.textContent.includes('Google 조회 실패'));
  assert.equal(f.writes.length, 0); assert(f.window.document.body.textContent.includes('미저장 이름'));
  f.window.document.dispatchEvent(new f.window.KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
  await wait(() => f.buttons('버리기').length); f.button('취소').click();
  await wait(() => !f.buttons('버리기').length); assert(f.window.document.querySelector('.people-dialog'));
  assert.equal(f.writes.length, 0);
});

test('closing during explicit Meet approval cancels that attempt without saving attendees', async t => {
  let finish;
  const f = await fixture(t, { fetchGoogleAttendees: async () => ({ sources: [], calendarComplete: true, meetStatus: 'permissionRequired' }), beginMeetConnect: async () => ({ attemptId: 'synthetic-attempt' }), finishCalendarConnect: () => new Promise(resolve => { finish = resolve; }) });
  f.button('Google에서 추가').click(); await wait(() => f.buttons('Meet 연결').length); f.button('Meet 연결').click();
  await wait(() => f.buttons('Meet 연결 취소').length && finish);
  f.button('취소').click(); await wait(() => !f.window.document.querySelector('.people-dialog'));
  finish({ meetConnected: true }); await new Promise(resolve => setImmediate(resolve));
  assert.deepEqual(f.cancelled, ['synthetic-attempt']); assert.equal(f.writes.length, 0);
  assert.equal(f.stored().meeting.attendees[0].name, '기존 임시');
});

test('one row per person merges a matching Meet participant, checks attendees by default and edits new names', async t => {
  const googleSources = [
    { sourceKey: 'invite-kim', origin: 'calendarInvitation', kind: 'person', displayName: '김민수', email: 'kim@example.test', googleUserId: null, responseStatus: 'accepted' },
    { sourceKey: 'invite-lee', origin: 'calendarInvitation', kind: 'person', displayName: '이영희', email: 'lee@example.test', googleUserId: null, responseStatus: 'declined' },
    { sourceKey: 'invite-room', origin: 'calendarInvitation', kind: 'calendarResource', displayName: '회의실 A', email: 'room@example.test', googleUserId: null, responseStatus: 'accepted' },
    { sourceKey: 'meet-kim', origin: 'meetParticipant', kind: 'signedInPersonOrRoom', displayName: '김민수', email: null, googleUserId: 'users/1', responseStatus: null },
    { sourceKey: 'meet-guest', origin: 'meetParticipant', kind: 'signedInPersonOrRoom', displayName: 'Guest Park', email: null, googleUserId: 'users/2', responseStatus: null },
  ];
  const f = await fixture(t, {}, { googleSources });
  f.button('Google에서 추가').click();
  await wait(() => f.window.document.querySelectorAll('[data-source-key]').length === 4);
  const row = key => f.window.document.querySelector(`[data-source-key="${key}"]`);
  assert.match(row('invite-kim').textContent, /수락 \/ Meet 참석/);
  assert.equal(row('meet-kim'), null);
  assert.equal(row('invite-kim').querySelector('input[type="checkbox"]').checked, true);
  assert.equal(row('invite-lee').querySelector('input[type="checkbox"]').checked, false);
  assert.equal(row('invite-room').querySelector('input[type="checkbox"]').checked, false);
  assert.equal(row('meet-guest').querySelector('input[type="checkbox"]').checked, true);
  f.input(row('meet-guest').querySelector('input[aria-label="Guest Park 이름"]'), '박손님');
  await wait(() => row('meet-guest').querySelector('input[aria-label="Guest Park 이름"]').value === '박손님');
  f.button('선택한 2명 추가').click();
  await wait(() => f.window.document.querySelectorAll('.attendee-chip').length === 3);
  assert.match(f.window.document.querySelector('.attendee-selected').textContent, /김민수[\s\S]*박손님/);
  f.button('적용').click(); await wait(() => !f.window.document.querySelector('.people-dialog'));
  const draft = f.writes[0].draft;
  const kim = draft.attendees.find(attendee => attendee.name === '김민수');
  assert.deepEqual(draft.sources.filter(entry => entry.attendeeId === kim.id).map(entry => entry.source.sourceKey).sort(), ['invite-kim', 'meet-kim']);
});

test('a unique saved name is suggested, while homonyms stay distinguishable and are not chosen automatically', async t => {
  const googleSources = [
    { sourceKey: 'meet-one', origin: 'meetParticipant', kind: 'signedInPersonOrRoom', displayName: '저장된 사람', email: null, googleUserId: 'users/1', responseStatus: null },
    { sourceKey: 'meet-two', origin: 'meetParticipant', kind: 'signedInPersonOrRoom', displayName: '홍길동', email: null, googleUserId: 'users/2', responseStatus: null },
  ];
  const catalog = { editVersion: 'catalog:1', people: [{ id: 'person', name: '저장된 사람' }, { id: 'hong-a', name: '홍길동' }, { id: 'hong-b', name: '홍길동' }], groups: [{ id: 'team', name: '개발팀', personIds: ['hong-a'] }] };
  const f = await fixture(t, {}, { googleSources, catalog });
  f.button('Google에서 추가').click();
  await wait(() => f.window.document.querySelectorAll('[data-source-key]').length === 2);
  const row = key => f.window.document.querySelector(`[data-source-key="${key}"]`);
  assert.match(row('meet-one').querySelector('[data-slot="select-trigger"]').textContent, /저장된 사람/);
  assert.match(row('meet-one').textContent, /이름이 같은 사람으로 제안했습니다/);
  assert.match(row('meet-two').querySelector('[data-slot="select-trigger"]').textContent, /새 참석자/);
  assert.match(row('meet-two').textContent, /이름이 같은 저장 인원이 2명입니다/);
  row('meet-two').querySelector('[data-slot="select-trigger"]').click();
  await wait(() => f.window.document.querySelector('[data-slot="select-content"]:not([data-closed]) [role="option"]'));
  const labels = [...f.window.document.querySelectorAll('[data-slot="select-content"]:not([data-closed]) [role="option"]')].map(node => node.textContent);
  assert.ok(labels.includes('홍길동 (개발팀)'));
  assert.ok(labels.includes('홍길동 (2번째)'));
});

test('Google import appears only for a linked event on a connected account', async t => {
  const unlinked = await fixture(t, {}, { meeting: { id: 'meeting', attendees: [{ id: 'manual', name: '기존 임시' }] } });
  assert.equal(unlinked.buttons('Google에서 추가').length, 0);
  assert.doesNotMatch(unlinked.window.document.body.textContent, /Google 캘린더 연결이 끊겨/);
  const disconnected = await fixture(t, { readCalendarSettings: async () => ({ connected: false }) });
  assert.equal(disconnected.buttons('Google에서 추가').length, 0);
  assert.match(disconnected.window.document.body.textContent, /Google 캘린더 연결이 끊겨 연결한 일정의 참석자를 불러올 수 없습니다/);
});

test('a fetch that ends the Google connection offers reconnecting instead of retrying', async t => {
  let connected = true;
  const f = await fixture(t, { readCalendarSettings: async () => ({ connected }), fetchGoogleAttendees: async () => { connected = false; throw Error('Google 연결이 만료되었거나 권한이 해제되었습니다. 다시 연결해 주세요.'); } });
  f.button('Google에서 추가').click();
  await wait(() => f.window.document.body.textContent.includes('Google 캘린더 연결이 끊겨'));
  assert.equal(f.buttons('다시 시도').length, 0);
  assert.equal(f.buttons('Google에서 추가').length, 0);
  assert.equal(f.window.document.querySelector('.google-attendee-picker'), null);
});

test('a transient Google failure keeps the retry', async t => {
  const f = await fixture(t, { fetchGoogleAttendees: async () => { throw Error('일정을 새로고침하지 못했습니다. 네트워크 상태를 확인해 주세요.'); } });
  f.button('Google에서 추가').click();
  await wait(() => f.buttons('다시 시도').length === 1);
});
