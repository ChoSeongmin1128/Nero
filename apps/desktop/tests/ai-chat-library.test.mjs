import assert from 'node:assert/strict';
import test from 'node:test';
import { build } from 'esbuild';
import { Window } from 'happy-dom';
import { fileURLToPath } from 'node:url';
import { meetingLabels, olderThan } from '../src/ai-chat-library-state.js';

const app = fileURLToPath(new URL('../', import.meta.url));
const bundle = await build({ entryPoints: [app + 'src/ai-chat-library.jsx'], nodePaths: [app + 'node_modules'], bundle: true, write: false, format: 'iife', globalName: 'AiLibrary', jsx: 'automatic', loader: { '.css': 'empty' }, define: { 'process.env.NODE_ENV': '"production"' } });
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
    selectConversation: async id => { calls.push(['select', id]); },
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
  assert.match(text, /주간 회의/); assert.match(text, /회의 없이 나눈 대화/); assert.match(text, /삭제된 회의/);
  assert.equal(window.document.querySelector('[aria-current="true"] .ai-library-title').textContent, '이번 주 정리');
  window.document.querySelectorAll('.ai-library-open')[1].click();
  await wait(() => opened.length === 1);
  assert.deepEqual(flow.calls.find(call => call[0] === 'select'), ['select', 'old']);

  check(window.document.querySelector('[aria-label="석 달 전 질문 선택"]'));
  await wait(() => !button('선택 삭제').disabled);
  button('선택 삭제').click(); await confirm('휴지통으로 이동');
  await wait(() => flow.calls.some(call => call[0] === 'trash'));
  assert.deepEqual(flow.calls.filter(call => call[0] === 'trash').at(-1), ['trash', ['old']]);

  assert.equal(button('1개 정리').disabled, false);
  button('1개 정리').click(); await confirm('휴지통으로 이동');
  await wait(() => flow.calls.filter(call => call[0] === 'trash').length === 2);
  assert.deepEqual(flow.calls.filter(call => call[0] === 'trash').at(-1), ['trash', ['old']]);

  button('전체 삭제').click(); await confirm('모두 휴지통으로');
  await wait(() => flow.calls.filter(call => call[0] === 'trash').length === 3);
  assert.deepEqual(flow.calls.filter(call => call[0] === 'trash').at(-1), ['trash', ['old', 'recent']]);

  button('휴지통').click();
  await wait(() => flow.calls.some(call => call[0] === 'loadTrash') && window.document.querySelectorAll('.ai-library-row').length === 1);
  assert.equal(window.document.querySelector('.ai-library-open').disabled, true);
  check(window.document.querySelector('[aria-label="버린 대화 선택"]'));
  await wait(() => !button('선택 복원').disabled);
  button('선택 복원').click(); await confirm('복원');
  await wait(() => flow.calls.some(call => call[0] === 'restore'));
  button('휴지통 비우기').click();
  await wait(() => [...window.document.querySelectorAll('button')].filter(node => node.textContent.trim() === '휴지통 비우기').length === 2);
  [...window.document.querySelectorAll('button')].filter(node => node.textContent.trim() === '휴지통 비우기').at(-1).click();
  await wait(() => flow.calls.some(call => call[0] === 'purge'));
  assert.deepEqual(flow.calls.find(call => call[0] === 'purge'), ['purge', ['trashed']]);
});
