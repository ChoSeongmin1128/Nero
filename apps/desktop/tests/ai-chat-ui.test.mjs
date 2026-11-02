import assert from 'node:assert/strict';
import test from 'node:test';
import { readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';
import { Window } from 'happy-dom';
const root = fileURLToPath(new URL('../', import.meta.url));
const html = await readFile(new URL('../src/index.html', import.meta.url), 'utf8');
const shared = await readFile(new URL('../../../design-system/components.js', import.meta.url), 'utf8');
const bundle = await build({ entryPoints: [root + 'tests/fixtures/ai-chat-fixture.jsx'], bundle: true, write: false, format: 'iife', globalName: 'AiFixture', jsx: 'automatic', nodePaths: [root + 'node_modules'], define: { 'process.env.NODE_ENV': '"production"' } });
const wait = async predicate => { for (let i = 0; i < 120; i++) { if (predicate()) return; await new Promise(resolve => setTimeout(resolve, 5)); } assert.fail('AI UI did not settle'); };
async function fixture(t, options = {}) {
  const window = new Window({ settings: { enableJavaScriptEvaluation: true, suppressInsecureJavaScriptEnvironmentWarning: true, disableJavaScriptFileLoading: true, disableCSSFileLoading: true } });
  window.document.write(html); window.eval(shared); window.eval(bundle.outputFiles[0].text);
  const app = window.AiFixture.mountAiChatFixture({ document: window.document, ...options });
  t.after(() => { app.dispose(); window.happyDOM.abort(); });
  const node = id => window.document.getElementById(id);
  await wait(() => app.ui.flow.getSnapshot().accountsLoaded && app.ui.flow.getSnapshot().conversation.loaded);
  await wait(() => node('ai-chat-inspector').querySelector('textarea') && node('inspector-notes-tab'));
  return { ...app, window, node, button: text => [...node('ai-chat-inspector').querySelectorAll('button')].find(button => button.textContent.trim() === text) };
}

test('real chat component streams, renders safe Markdown, navigates citations and saves only completed answers', async t => {
  const app = await fixture(t);
  app.button('결정 사항과 할 일 정리').click();
  await wait(() => !app.node('ai-chat-inspector').querySelector('[aria-label="질문 보내기"]').disabled);
  app.node('ai-chat-inspector').querySelector('[aria-label="질문 보내기"]').click();
  await wait(() => app.node('ai-chat-inspector').querySelector('[aria-label="답변 중지"]'));
  assert.equal(app.button('요약 저장'), undefined);
  await wait(() => app.button('요약 저장'));
  assert.ok(app.node('ai-chat-inspector').querySelector('.summary-document h3'));
  app.node('ai-chat-inspector').querySelector('[data-summary-citations]').click();
  await wait(() => app.calls.some(call => call[0] === 'citation'));
  app.button('요약 저장').click(); await wait(() => app.calls.some(call => call[0] === 'summary'));
  assert.equal(app.calls.find(call => call[0] === 'send')[1].templateId, 'example-template');
});

test('Base UI inspector tabs preserve the notes draft and shared main panels', async t => {
  const app = await fixture(t);
  app.node('inspector-notes-tab').click(); await wait(() => !app.node('notes-section').hidden);
  assert.equal(app.node('meeting-notes').value, '보존할 메모 초안');
  app.node('inspector-ai-tab').click(); await wait(() => !app.node('ai-chat-inspector').hidden);
  assert.equal(app.node('notes-section').hidden, true);
  assert.equal(app.node('summary-section').hidden, false); assert.equal(app.node('transcript-section').hidden, false);
  assert.equal(app.node('inspector-ai-tab').getAttribute('aria-selected'), 'true');
  app.node('detail-inspector-close').click(); await wait(() => app.node('detail-inspector').hidden);
  assert.equal(app.window.document.activeElement, app.node('detail-ai-open'));
});

test('unavailable accounts cannot send or misleadingly start account connection', async t => {
  const app = await fixture(t, { scenario: 'unavailable' });
  app.ui.flow.edit('A question'); await wait(() => app.node('ai-chat-inspector').textContent.includes('ChatGPT를 사용할 수 없습니다.'));
  assert.equal(app.node('ai-chat-inspector').querySelector('[aria-label="질문 보내기"]').disabled, true);
  app.ui.setSettingsActive(true); await wait(() => app.node('ai-account-settings').textContent.includes('사용할 수 없음'));
  const connections = [...app.node('ai-account-settings').querySelectorAll('button')].filter(node => node.textContent === '연결');
  assert.equal(connections[0].disabled, true); assert.equal(connections[1].disabled, true);
});

test('explicit source addition uses the product popover and appears as a removable source chip', async t => {
  const app = await fixture(t); app.button('자료 추가').click();
  await wait(() => app.window.document.getElementById('ai-source-link'));
  const input = app.window.document.getElementById('ai-source-link');
  Object.getOwnPropertyDescriptor(app.window.HTMLInputElement.prototype, 'value').set.call(input, 'https://www.notion.so/example'); input.dispatchEvent(new app.window.Event('input', { bubbles: true }));
  await wait(() => ![...app.window.document.querySelectorAll('.ai-source-popover button')].find(node => node.textContent === '추가').disabled);
  [...app.window.document.querySelectorAll('.ai-source-popover button')].find(node => node.textContent === '추가').click();
  await wait(() => app.node('ai-chat-inspector').querySelector('[aria-label="예시 자료 제외"]'));
  app.node('ai-chat-inspector').querySelector('[aria-label="예시 자료 제외"]').click();
  await wait(() => !app.node('ai-chat-inspector').querySelector('[aria-label="예시 자료 제외"]'));
});

test('failed answers offer retry and never show summary save', async t => {
  const app = await fixture(t, { scenario: 'failed' }); app.ui.flow.edit('Question');
  await app.ui.flow.send(); await wait(() => app.button('다시 시도'));
  assert.equal(app.button('요약 저장'), undefined); assert.match(app.node('ai-chat-inspector').textContent, /예시 연결 오류/);
});

test('closing the inspector closes the source popup and preserves the question draft', async t => {
  const app = await fixture(t); app.ui.flow.edit('Draft remains'); app.button('자료 추가').click();
  await wait(() => app.window.document.getElementById('ai-source-link'));
  app.node('detail-inspector-close').click(); await wait(() => app.node('detail-inspector').hidden);
  await wait(() => !app.window.document.getElementById('ai-source-link'));
  assert.equal(app.ui.flow.getSnapshot().conversation.draft, 'Draft remains');
});

test('assistant markup is escaped by the shared renderer', async t => {
  const app = await fixture(t, { delay: 10000 }); app.ui.flow.edit('Question'); await app.ui.flow.send();
  const sent = app.calls.find(call => call[0] === 'send')[1];
  app.ui.flow.receive({ requestId: sent.requestId, meetingId: sent.meetingId, conversationId: 'example-conversation', type: 'completed', messageId: 'unsafe-text', text: '<img src=x onerror=alert(1)> **Text**' });
  await wait(() => app.node('ai-chat-inspector').querySelector('.summary-document strong'));
  assert.equal(app.node('ai-chat-inspector').querySelector('.summary-document img'), null);
  assert.match(app.node('ai-chat-inspector').querySelector('.summary-document').textContent, /<img/);
});

test('IME Enter does not submit a question', async t => {
  const app = await fixture(t); app.ui.flow.edit('입력 중');
  await wait(() => !app.node('ai-chat-inspector').querySelector('[aria-label="질문 보내기"]').disabled);
  const input = app.node('ai-chat-inspector').querySelector('textarea');
  input.dispatchEvent(new app.window.KeyboardEvent('keydown', { key: 'Enter', isComposing: true, bubbles: true }));
  await new Promise(resolve => setTimeout(resolve, 20)); assert.equal(app.calls.some(call => call[0] === 'send'), false);
});
