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

async function fixture(t, overrides = {}) {
  const window = new Window({ url: 'http://nero.test', settings: { enableJavaScriptEvaluation: true, suppressInsecureJavaScriptEnvironmentWarning: true } });
  window.document.body.innerHTML = '<button id="origin">참석자 열기</button>'; window.eval(bundle.outputFiles[0].text);
  let stored = { editVersion: 'roster:1', meeting: { id: 'meeting', attendees: [{ id: 'manual', name: '기존 임시' }] }, sources: [], identityLinks: [], catalog: { editVersion: 'catalog:1', people: [{ id: 'person', name: '저장된 사람' }], groups: [] } };
  const writes = [], reads = [], cancelled = [];
  const bridge = {
    readGoogleAttendeeRoster: async () => clone(stored),
    fetchGoogleAttendees: async () => { reads.push(true); return { sources: clone(sources), calendarComplete: true, meetStatus: 'available' }; },
    applyMeetingAttendeeDraft: async (id, expectedVersion, draft) => {
      writes.push(clone({ id, expectedVersion, draft }));
      stored = { ...stored, editVersion: 'roster:2', meeting: { ...stored.meeting, attendees: clone(draft.attendees) }, sources: clone(draft.sources), catalog: { ...stored.catalog, people: [...stored.catalog.people, ...draft.promotions.map(entry => entry.person)] } };
      return clone(stored);
    },
    cancelCalendarConnect: async id => cancelled.push(id),
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
    row.querySelector('.google-attendee-choice > button:last-child').click();
  };
  return { window, editor, writes, reads, cancelled, open, button, buttons, input, choose, stored: () => stored };
}

test('Google accounts link to existing temporary or saved attendees inside one staged dialog', async t => {
  const f = await fixture(t); f.button('Google에서 추가').click();
  await wait(() => f.window.document.querySelectorAll('[data-source-key]').length === 2);
  assert(f.window.document.querySelector('[aria-label="일정 초대자"]'));
  assert(f.window.document.querySelector('[aria-label="Meet 참석자"]'));
  assert.equal(f.window.document.querySelectorAll('.attendee-chip').length, 1);
  await f.choose('invite', '기존 임시');
  await wait(() => f.window.document.querySelector('[aria-label="초대 이름 추가됨"]'));
  await f.choose('actual', '저장된 사람');
  await wait(() => f.window.document.querySelectorAll('.attendee-chip').length === 2);
  assert.equal(f.writes.length, 0);
  f.button('새로고침').click(); await wait(() => f.reads.length === 2 && !f.button('Google에서 추가').disabled);
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
