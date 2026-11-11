import assert from 'node:assert/strict';
import test from 'node:test';
import { build } from 'esbuild';
import { Window } from 'happy-dom';
import { fileURLToPath } from 'node:url';
import { meetingLabels, olderThan } from '../src/ai-chat-library-state.js';

const app = fileURLToPath(new URL('../', import.meta.url));
const bundle = await build({ entryPoints: [app + 'src/ai-chat-library.jsx'], nodePaths: [app + 'node_modules'], bundle: true, write: false, format: 'iife', globalName: 'AiLibrary', jsx: 'automatic', loader: { '.css': 'empty' }, define: { 'process.env.NODE_ENV': '"production"' } });
const sidebarBundle = await build({ entryPoints: [app + 'src/ai-chat-sidebar.jsx'], nodePaths: [app + 'node_modules'], bundle: true, write: false, format: 'iife', globalName: 'AiSidebar', jsx: 'automatic', loader: { '.css': 'empty' }, define: { 'process.env.NODE_ENV': '"production"' } });
const DAY = 24 * 60 * 60 * 1000, now = Date.now();
const meetings = [{ id: 'm1', title: '주간 회의' }, { id: 'm2', title: '지난 회의', deletedAt: 1 }];

test('rows name their meetings and the period cleanup skips answers in progress', () => {
  assert.deepEqual(meetingLabels({ meetingIds: ['m1', 'm2', 'gone'] }, meetings), ['주간 회의', '지난 회의 (휴지통)', '삭제된 회의']);
  const items = [{ id: 'old', updatedAt: now - 12 * DAY }, { id: 'busy', updatedAt: now - 40 * DAY, activeRequestId: 'r' }, { id: 'new', updatedAt: now - DAY }];
  assert.deepEqual(olderThan(items, 10, now).map(item => item.id), ['old']);
  assert.deepEqual(olderThan(items, 30, now), []);
});

function fakeFlow() {
  const listeners = new Set(), calls = [];
  let state = {
    conversation: { id: 'recent' },
    conversationList: [
      { id: 'recent', title: '이번 주 정리', updatedAt: now - DAY, meetingIds: ['m1'] },
      { id: 'old', title: '석 달 전 질문', updatedAt: now - 100 * DAY, meetingIds: [] },
      { id: 'running', title: '답변 중', updatedAt: now - 200 * DAY, meetingIds: ['gone'], activeRequestId: 'r' },
    ],
    trashList: [{ id: 'trashed', title: '버린 대화', updatedAt: now - 5 * DAY, meetingIds: [] }],
    listError: '', trashError: '',
  };
  const publish = patch => { state = { ...state, ...patch }; listeners.forEach(listener => listener()); };
  const record = name => async ids => { calls.push([name, [...ids].sort()]); return true; };
  return {
    calls, publish,
    subscribe: listener => { listeners.add(listener); return () => listeners.delete(listener); },
    getSnapshot: () => state,
    loadList: async () => { calls.push(['loadList']); },
    loadTrash: async () => { calls.push(['loadTrash']); },
    selectConversation: async id => { calls.push(['select', id]); return true; },
    trashConversations: record('trash'), restoreConversations: record('restore'), purgeConversations: record('purge'),
  };
}

test('the AI list opens a chat, trashes selections, old and all chats, and manages its trash', async t => {
  const window = new Window({ url: 'http://nero.test', settings: { enableJavaScriptEvaluation: true, suppressInsecureJavaScriptEnvironmentWarning: true } });
  t.after(() => window.happyDOM.abort());
  window.document.body.innerHTML = '<section id="host"></section>';
  window.eval(bundle.outputFiles[0].text);
  const flow = fakeFlow(), opened = [];
  const view = window.AiLibrary.mountAiChatLibrary({ node: window.document.getElementById('host'), flow, getMeetings: () => meetings, onOpen: () => opened.push(true), container: window.document.body });
  t.after(() => view.dispose());
  view.show();
  const wait = async (predicate, step = '') => { for (let i = 0; i < 100; i++) { if (predicate()) return; await new Promise(resolve => setTimeout(resolve, 5)); } assert.fail(`Expected AI list state ${step}: ${window.document.body.textContent.slice(-300)}`); };
  const button = text => [...window.document.querySelectorAll('button')].find(node => node.textContent.trim() === text);
  const check = node => node.click();
  const last = text => [...window.document.querySelectorAll('[data-slot="dialog-content"]:not([data-closed]) button, .ai-conversation-confirm:not([data-closed]) button')].filter(node => node.textContent.trim() === text).at(-1);
  const settled = () => wait(() => !window.document.querySelector('[data-slot="dialog-content"]'), 'dialog closed');
  const confirm = async label => { await wait(() => last(label), 'confirm'); last(label).click(); await settled(); };
  await wait(() => window.document.querySelectorAll('.ai-library-row').length === 3);
  const text = window.document.querySelector('.ai-library-list').textContent;
  assert.match(text, /주간 회의/); assert.doesNotMatch(text, /회의 없이 나눈 대화/); assert.match(text, /삭제된 회의/);
  assert.equal(window.document.querySelector('[aria-current="true"] .ai-library-title').textContent, '이번 주 정리');
  window.document.querySelectorAll('.ai-library-open')[1].click();
  await wait(() => opened.length === 1);
  assert.deepEqual(flow.calls.find(call => call[0] === 'select'), ['select', 'old']);

  check(window.document.querySelector('[aria-label="석 달 전 질문 선택"]'));
  await wait(() => !button('선택 삭제').disabled);
  button('선택 삭제').click(); await confirm('휴지통으로 이동');
  await wait(() => flow.calls.some(call => call[0] === 'trash'));
  assert.deepEqual(flow.calls.filter(call => call[0] === 'trash').at(-1), ['trash', ['old']]);

  const openCleanup = async () => { window.document.querySelector('[aria-label="대화 정리"]').click(); await wait(() => button('휴지통'), 'cleanup menu'); };
  await openCleanup();
  assert.equal(button('1개 정리').disabled, false);
  button('1개 정리').click(); await confirm('휴지통으로 이동');
  await wait(() => flow.calls.filter(call => call[0] === 'trash').length === 2);
  assert.deepEqual(flow.calls.filter(call => call[0] === 'trash').at(-1), ['trash', ['old']]);

  await openCleanup();
  button('전체 삭제').click(); await confirm('모두 휴지통으로');
  await wait(() => flow.calls.filter(call => call[0] === 'trash').length === 3);
  assert.deepEqual(flow.calls.filter(call => call[0] === 'trash').at(-1), ['trash', ['old', 'recent']]);

  await openCleanup();
  button('휴지통').click();
  await wait(() => flow.calls.some(call => call[0] === 'loadTrash') && window.document.querySelectorAll('.ai-library-row').length === 1);
  assert.equal(window.document.querySelector('.ai-library-open').disabled, true);
  check(window.document.querySelector('[aria-label="버린 대화 선택"]'));
  await wait(() => !button('선택 복원').disabled);
  button('선택 복원').click(); await confirm('복원');
  await wait(() => flow.calls.some(call => call[0] === 'restore'));
  await openCleanup();
  button('휴지통 비우기').click();
  await confirm('휴지통 비우기');
  await wait(() => flow.calls.some(call => call[0] === 'purge'));
  assert.deepEqual(flow.calls.find(call => call[0] === 'purge'), ['purge', ['trashed']]);
});

test('the sidebar lists chats, opens one in the AI panel and links to its meeting', async t => {
  const window = new Window({ url: 'http://nero.test', settings: { enableJavaScriptEvaluation: true, suppressInsecureJavaScriptEnvironmentWarning: true } });
  t.after(() => window.happyDOM.abort());
  window.document.body.innerHTML = '<div id="host"></div>';
  window.eval(sidebarBundle.outputFiles[0].text);
  const flow = fakeFlow(), opened = [];
  flow.publish({ active: true, listLoaded: true });
  const view = window.AiSidebar.mountAiChatSidebar({ node: window.document.getElementById('host'), flow, getMeetings: () => meetings, onOpenConversation: async id => opened.push(['chat', id]), onOpenMeeting: async (meetingId, id) => opened.push(['meeting', meetingId, id]) });
  t.after(() => view.dispose());
  view.show();
  const wait = async predicate => { for (let i = 0; i < 100; i++) { if (predicate()) return; await new Promise(resolve => setTimeout(resolve, 5)); } assert.fail(window.document.body.textContent); };
  await wait(() => window.document.querySelectorAll('.sidebar-ai-row').length === 3);
  assert.ok(flow.calls.some(call => call[0] === 'loadList'));
  const rows = [...window.document.querySelectorAll('.sidebar-ai-row')];
  assert.equal(rows[0].querySelector('[data-navigation-item]').getAttribute('aria-current'), 'page');
  assert.match(rows[0].textContent, /이번 주 정리/); assert.match(rows[0].textContent, /주간 회의/);
  const timestamp = rows[0].querySelector('time');
  assert.ok(timestamp.textContent.length <= 9);
  assert.doesNotMatch(timestamp.textContent, /:/);
  assert.equal(timestamp.getAttribute('datetime'), new Date(now - DAY).toISOString());
  assert.ok(timestamp.title.length > timestamp.textContent.length);
  assert.equal(rows[1].querySelector('.sidebar-ai-note'),null);
  assert.equal(rows[1].querySelector('.sidebar-ai-meeting'),null);
  assert.match(rows[2].textContent, /답변 중/); assert.match(rows[2].textContent, /삭제된 회의/);
  assert.equal(rows[2].querySelector('.sidebar-ai-meeting'), null);
  rows[1].querySelector('[data-navigation-item]').click();
  rows[0].querySelector('.sidebar-ai-meeting').click();
  await wait(() => opened.length === 2);
  assert.deepEqual(opened, [['chat', 'old'], ['meeting', 'm1', 'recent']]);
});

for (const scenario of [
  { trash: false, action: 'trash', label: '선택 삭제', confirm: '휴지통으로 이동' },
  { trash: true, action: 'restore', label: '선택 복원', confirm: '복원' },
  { trash: true, action: 'purge', label: '선택 완전히 삭제', confirm: '완전히 삭제' },
]) {
  test(`search changes clear hidden selections before ${scenario.action}`, async t => {
    const window = new Window({ url: 'http://nero.test', settings: { enableJavaScriptEvaluation: true, suppressInsecureJavaScriptEnvironmentWarning: true } });
    window.document.body.innerHTML = '<section id="host"></section>';
    window.eval(bundle.outputFiles[0].text);
    const flow = fakeFlow();
    flow.publish({ trashList: [{ id: 'trashed', title: '버린 대화', updatedAt: now }, { id: 'other-trash', title: '다른 기록', updatedAt: now }] });
    const view = window.AiLibrary.mountAiChatLibrary({ node: window.document.getElementById('host'), flow, getMeetings: () => meetings, onOpen: () => {}, container: window.document.body });
    t.after(() => { view.dispose(); window.happyDOM.abort(); });
    view.show();
    const wait = async predicate => { for (let index = 0; index < 100; index++) { if (predicate()) return; await new Promise(resolve => setTimeout(resolve, 5)); } assert.fail('Expected filtered selection state'); };
    const button = label => [...window.document.querySelectorAll('button')].find(node => node.textContent.trim() === label);
    await wait(() => window.document.querySelectorAll('.ai-library-row').length === 3);
    if (scenario.trash) {
      window.document.querySelector('[aria-label="대화 정리"]').click();
      await wait(() => button('휴지통'));
      button('휴지통').click();
      await wait(() => window.document.querySelectorAll('.ai-library-row').length === 2);
    }
    const oldLabel = scenario.trash ? '버린 대화' : '석 달 전 질문';
    const currentLabel = scenario.trash ? '다른 기록' : '이번 주 정리';
    const term = scenario.trash ? '다른' : '이번';
    window.document.querySelector(`[aria-label="${oldLabel} 선택"]`).click();
    await wait(() => button(scenario.label));
    const search = window.document.querySelector('input[type="search"]');
    const type = value => { Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value').set.call(search, value); search.dispatchEvent(new window.Event('input', { bubbles: true })); };
    type(term);
    await wait(() => window.document.querySelectorAll('.ai-library-row').length === 1 && !button(scenario.label));
    type('');
    await wait(() => window.document.querySelector(`[aria-label="${oldLabel} 선택"]`));
    assert.equal(window.document.querySelector(`[aria-label="${oldLabel} 선택"]`).checked, false);
    type(term);
    await wait(() => window.document.querySelectorAll('.ai-library-row').length === 1);
    window.document.querySelector(`[aria-label="${currentLabel} 선택"]`).click();
    await wait(() => button(scenario.label));
    assert.match(window.document.querySelector('.ai-library-actions').textContent, /^1개 선택/);
    button(scenario.label).click();
    await wait(() => window.document.querySelector('[data-slot="dialog-content"]'));
    [...window.document.querySelectorAll('[data-slot="dialog-content"] button')].find(node => node.textContent.trim() === scenario.confirm).click();
    await wait(() => flow.calls.some(call => call[0] === scenario.action));
    assert.deepEqual(flow.calls.find(call => call[0] === scenario.action), [scenario.action, [scenario.trash ? 'other-trash' : 'recent']]);
  });
}
