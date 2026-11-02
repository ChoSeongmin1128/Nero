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
  window.structuredClone = structuredClone;
  window.document.write(html); window.eval(shared); window.eval(bundle.outputFiles[0].text);
  const app = window.AiFixture.mountAiChatFixture({ document: window.document, ...options });
  t.after(() => { app.dispose(); window.happyDOM.abort(); });
  const node = id => window.document.getElementById(id);
  await wait(() => app.ui.flow.getSnapshot().accountsLoaded && app.ui.flow.getSnapshot().conversation.loaded);
  await wait(() => !app.ui.flow.getSnapshot().accountsBusy);
  await wait(() => node('ai-chat-inspector').querySelector('textarea'));
  const state = app.ui.flow.getSnapshot();
  if (options.prepareModels !== false && state.accounts.some(account => account.id === state.accountId && account.state === 'ready')) {
    node('ai-chat-inspector').querySelector('[aria-label="AI 모델 선택"]').click();
    await wait(() => window.document.querySelector('.ai-model-popover'));
    [...window.document.querySelectorAll('.ai-model-popover button')].find(button => button.textContent.trim() === 'ChatGPT').click();
    await wait(() => [...window.document.querySelectorAll('.ai-model-popover button')].some(button => button.textContent.trim() === '예시 모델'));
    [...window.document.querySelectorAll('.ai-model-popover button')].find(button => button.textContent.trim() === '예시 모델').click();
    await wait(() => !window.document.querySelector('.ai-model-popover'));
  }
  if (app.ui.flow.getSnapshot().modelId) await wait(() => node('ai-chat-inspector').querySelector('[aria-label="AI 모델 선택"]')?.textContent.includes('예시 모델'));
  await window.happyDOM.whenAsyncComplete();
  return { ...app, window, node, button: text => { const buttons = [...node('ai-chat-inspector').querySelectorAll('button')]; return buttons.find(button => button.getAttribute('aria-label') === text) || buttons.find(button => button.textContent.trim() === text); } };
}

test('model picker navigates providers and models, handles unavailable accounts and closes on Escape', async t => {
  const app = await fixture(t);
  const trigger = app.button('AI 모델 선택');
  trigger.click();
  await wait(() => app.window.document.querySelector('.ai-model-popover'));
  const menu = () => app.window.document.querySelector('.ai-model-popover');
  const option = text => [...menu().querySelectorAll('button')].find(node => node.textContent.trim() === text);
  option('Claude').click();
  await wait(() => menu()?.textContent.includes('사용할 수 없는 AI입니다.'));
  assert.equal(app.ui.flow.getSnapshot().provider, 'claude');
  assert.ok(option('계정 설정'));
  menu().querySelector('[aria-label="AI 목록으로 돌아가기"]').click();
  await wait(() => option('ChatGPT'));
  option('ChatGPT').click();
  await wait(() => option('예시 모델'));
  option('예시 모델').click();
  await wait(() => !menu());
  await wait(() => app.window.document.activeElement === trigger);
  assert.equal(app.ui.flow.getSnapshot().modelId, 'example-model');
  assert.match(trigger.textContent, /예시 모델/);
  trigger.click(); await wait(() => menu());
  await wait(() => app.window.document.activeElement.closest('.ai-model-popover'));
  app.window.document.activeElement.dispatchEvent(new app.window.KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
  await wait(() => !menu());
  await wait(() => app.window.document.activeElement === trigger);
});

test('Escape closes the focused AI popover without restoring focus to an unrelated action menu', async t => {
  const app = await fixture(t);
  const details = app.node('meeting-detail').querySelector('.transcript-maintenance');
  const summary = details.querySelector('summary');
  summary.dispatchEvent(new app.window.KeyboardEvent('keydown', { key: 'ArrowDown', bubbles: true, cancelable: true }));
  assert.equal(details.open, true);
  const trigger = app.button('AI 모델 선택'); trigger.click();
  await wait(() => app.window.document.querySelector('.ai-model-popover'));
  await wait(() => app.window.document.activeElement.closest('.ai-model-popover'));
  app.window.document.activeElement.dispatchEvent(new app.window.KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }));
  await wait(() => !app.window.document.querySelector('.ai-model-popover'));
  await wait(() => app.window.document.activeElement === trigger);
  assert.equal(details.open, false);
});

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

test('the global AI sidebar and meeting notes stay independently open and preserve the notes draft', async t => {
  const app = await fixture(t);
  await wait(() => !app.node('notes-section').hidden && app.node('app-ai-sidebar').getAttribute('aria-hidden') === 'false');
  assert.equal(app.node('meeting-notes').value, '보존할 메모 초안');
  assert.equal(app.node('summary-section').hidden, false); assert.equal(app.node('transcript-section').hidden, false);
  app.button('AI 닫기').click(); await wait(() => app.node('app-ai-sidebar').getAttribute('aria-hidden') === 'true');
  assert.equal(app.node('notes-section').hidden, false);
  assert.equal(app.node('meeting-notes').value, '보존할 메모 초안');
  assert.equal(app.window.document.activeElement, app.node('ai-chat-open'));
  app.node('ai-chat-open').click(); await wait(() => app.node('app-ai-sidebar').getAttribute('aria-hidden') === 'false');
  app.node('detail-inspector-close').click(); await wait(() => app.node('detail-inspector').hidden);
  assert.equal(app.node('app-ai-sidebar').getAttribute('aria-hidden'), 'false');
  assert.equal(app.node('meeting-notes').value, '보존할 메모 초안');
});

test('navigating meetings preserves the chat draft and opening AI explicitly attaches the new meeting', async t => {
  const app = await fixture(t);
  app.ui.flow.edit('두 회의를 비교할 질문');
  const next = { id: 'example-meeting-2', title: '다음 회의' };
  app.ui.setMeeting(next);
  app.detail.update({ id: next.id, showSummary: true, hasTranscript: true, summaryLoaded: true, transcriptLoaded: true });
  await wait(() => app.node('ai-chat-inspector').querySelector('textarea').value === '두 회의를 비교할 질문');
  assert.equal(app.ui.flow.getSnapshot().conversation.meetings.length, 1);
  assert.ok(app.button('예시 회의 제외'));
  assert.equal(app.button('다음 회의 제외'), undefined);
  app.ui.open(next);
  await wait(() => app.button('다음 회의 제외'));
  assert.equal(app.node('ai-chat-inspector').querySelector('textarea').value, '두 회의를 비교할 질문');
  app.node('ai-chat-inspector').querySelector('[aria-label="질문 보내기"]').click();
  await wait(() => app.calls.some(call => call[0] === 'send'));
  assert.deepEqual(Array.from(app.calls.find(call => call[0] === 'send')[1].meetingIds), ['example-meeting', 'example-meeting-2']);
});

test('new chat and history navigate independent conversations without changing meeting notes', async t => {
  const app = await fixture(t);
  app.ui.flow.edit('첫 번째 질문'); await app.ui.flow.send();
  await wait(() => app.button('AI 답변 복사'));
  await wait(() => app.button('대화 기록').textContent.includes('첫 번째 질문'));
  const original = app.ui.flow.getSnapshot().conversation.id;
  app.button('새 대화').click();
  await wait(() => app.ui.flow.getSnapshot().conversation.id === null && !app.node('ai-chat-inspector').querySelector('.ai-chat-message'));
  assert.equal(app.node('ai-chat-inspector').querySelector('textarea').value, '');
  app.ui.flow.edit('다른 대화의 질문'); await app.ui.flow.send();
  await wait(() => app.button('AI 답변 복사'));
  assert.notEqual(app.ui.flow.getSnapshot().conversation.id, original);
  app.button('대화 기록').click();
  await wait(() => [...app.window.document.querySelectorAll('.ai-history-row > button:first-child')].some(node => node.textContent.includes('첫 번째 질문')));
  [...app.window.document.querySelectorAll('.ai-history-row > button:first-child')].find(node => node.textContent.includes('첫 번째 질문')).click();
  await wait(() => app.ui.flow.getSnapshot().conversation.id === original && !app.ui.flow.getSnapshot().conversation.loading);
  await wait(() => app.node('ai-chat-inspector').querySelector('.ai-chat-message--user')?.textContent.includes('첫 번째 질문'));
  assert.equal(app.node('ai-chat-inspector').querySelector('.ai-chat-message--user').textContent.includes('다른 대화의 질문'), false);
  assert.equal(app.node('meeting-notes').value, '보존할 메모 초안');
  assert.equal(app.node('notes-section').hidden, false);
});

test('multiple meeting attachments require an explicit summary save target', async t => {
  const app = await fixture(t);
  app.button('자료 추가').click();
  await wait(() => [...app.window.document.querySelectorAll('.ai-attachment-option')].some(node => node.textContent.includes('다음 회의')));
  [...app.window.document.querySelectorAll('.ai-attachment-option')].find(node => node.textContent.includes('다음 회의')).click();
  await wait(() => app.button('다음 회의 제외'));
  app.ui.flow.edit('두 회의의 결정 사항을 비교해 주세요.'); await app.ui.flow.send();
  await wait(() => app.button('요약 저장'));
  assert.equal(app.calls.some(call => call[0] === 'summary'), false);
  app.button('요약 저장').click();
  await wait(() => [...app.window.document.querySelectorAll('.nero-action-popover button')].some(node => node.textContent === '다음 회의'));
  assert.equal(app.calls.some(call => call[0] === 'summary'), false);
  [...app.window.document.querySelectorAll('.nero-action-popover button')].find(node => node.textContent === '다음 회의').click();
  await wait(() => app.calls.some(call => call[0] === 'summary'));
  const saved = app.calls.find(call => call[0] === 'summary')[1];
  assert.equal(saved.meetingId, 'example-meeting-2');
  assert.equal(saved.conversationId, app.ui.flow.getSnapshot().conversation.id);
});

test('chat remains usable on the meeting list without an attached meeting', async t => {
  const app = await fixture(t);
  app.button('예시 회의 제외').click();
  app.ui.setMeeting(null);
  app.detail.update({ id: null });
  app.node('meeting-detail').hidden = true;
  app.node('library-screen').hidden = false;
  await wait(() => !app.node('ai-chat-inspector').querySelector('.ai-source-chip'));
  const input = app.node('ai-chat-inspector').querySelector('textarea');
  assert.equal(input.disabled, false);
  app.ui.flow.edit('간단한 질문입니다.');
  await wait(() => !app.node('ai-chat-inspector').querySelector('[aria-label="질문 보내기"]').disabled);
  app.node('ai-chat-inspector').querySelector('[aria-label="질문 보내기"]').click();
  await wait(() => app.calls.some(call => call[0] === 'send'));
  assert.deepEqual(Array.from(app.calls.find(call => call[0] === 'send')[1].meetingIds), []);
  await wait(() => app.button('AI 답변 복사'));
  assert.equal(app.button('요약 저장'), undefined);
  assert.equal(app.button('메모 추가'), undefined);
  assert.equal(app.node('app-ai-sidebar').getAttribute('aria-hidden'), 'false');
});

test('unavailable accounts cannot send or misleadingly start account connection', async t => {
  const app = await fixture(t, { scenario: 'unavailable' });
  app.ui.flow.edit('A question'); await wait(() => app.node('ai-chat-inspector').textContent.includes('ChatGPT를 사용할 수 없습니다.'));
  assert.equal(app.node('ai-chat-inspector').querySelector('[aria-label="질문 보내기"]').disabled, true);
  app.ui.setSettingsActive(true); await wait(() => app.node('ai-account-settings').textContent.includes('사용할 수 없음'));
  const connections = [...app.node('ai-account-settings').querySelectorAll('button')].filter(node => node.textContent === '로그인');
  assert.equal(connections[0].disabled, true); assert.equal(connections[1].disabled, true);
});

test('settings distinguish subscription sign-in from external app and reference connections', async t => {
  const app = await fixture(t); app.ui.setSettingsActive(true);
  const panel = app.node('ai-account-settings');
  await wait(() => app.ui.flow.getSnapshot().notion?.connected && !app.ui.flow.getSnapshot().accountsBusy);
  await wait(() => panel.textContent.includes('로그인됨'));
  assert.deepEqual([...panel.querySelectorAll('h4')].map(node => node.textContent), ['AI 계정', 'Notion 연결']);
  assert.equal([...app.node('settings-agent-panel').querySelectorAll('h4')].some(node => node.textContent === 'MCP 연결'), true);
  const rows = [...panel.querySelectorAll('.settings-client-row')];
  const chatgpt = rows.find(node => node.querySelector('.settings-client-name')?.textContent === 'ChatGPT');
  const notion = rows.find(node => node.querySelector('.settings-client-name')?.textContent === 'Notion');
  assert.match(chatgpt.textContent, /로그인됨/);
  assert.equal([...chatgpt.querySelectorAll('button')].some(node => node.textContent === '로그아웃'), true);
  assert.equal([...chatgpt.querySelectorAll('button')].some(node => node.textContent === '연결' || node.textContent === '해제'), false);
  assert.match(notion.textContent, /연결됨/);
  assert.equal([...notion.querySelectorAll('button')].some(node => node.textContent === '해제'), true);
  assert.equal([...notion.querySelectorAll('button')].some(node => node.textContent === '로그아웃'), false);
});

test('saved account loads models only after choosing its provider', async t => {
  const app = await fixture(t, { scenario: 'unknown' });
  app.ui.setSettingsActive(true); await wait(() => !app.ui.flow.getSnapshot().accountsBusy);
  assert.equal(app.calls.filter(call => call[0] === 'models').length, 0);
  const trigger = app.button('AI 모델 선택');
  assert.match(trigger.textContent, /모델 선택/);
  trigger.click();
  await wait(() => app.window.document.querySelector('.ai-model-popover'));
  assert.equal(app.calls.filter(call => call[0] === 'models').length, 0);
  [...app.window.document.querySelectorAll('.ai-model-popover button')].find(node => node.textContent.trim() === 'ChatGPT').click();
  await wait(() => app.window.document.querySelector('.ai-model-popover')?.textContent.includes('예시 모델'));
  assert.equal(app.calls.filter(call => call[0] === 'models').length, 1);
  assert.equal(app.node('ai-chat-inspector').textContent.includes('AI 계정에 로그인해 주세요.'), false);
  app.ui.flow.edit('Question');
  await wait(() => !app.node('ai-chat-inspector').querySelector('[aria-label="질문 보내기"]').disabled);
});

test('opening settings for a ready account does not load models', async t => {
  const app = await fixture(t, { prepareModels: false });
  app.ui.setSettingsActive(true);
  await wait(() => !app.ui.flow.getSnapshot().accountsBusy);
  assert.equal(app.calls.filter(call => call[0] === 'models').length, 0);
  assert.equal(app.ui.flow.getSnapshot().modelId, null);
});

test('explicit source addition selects a Notion link and creates a removable attachment', async t => {
  const app = await fixture(t); app.button('자료 추가').click();
  await wait(() => [...app.window.document.querySelectorAll('.ai-source-popover button')].some(node => node.textContent === 'Notion 링크'));
  [...app.window.document.querySelectorAll('.ai-source-popover button')].find(node => node.textContent === 'Notion 링크').click();
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

test('closing AI closes its attachment popup while preserving both drafts', async t => {
  const app = await fixture(t); app.ui.flow.edit('Draft remains'); app.button('자료 추가').click();
  await wait(() => app.window.document.querySelector('.ai-source-popover'));
  app.ui.close(); await wait(() => app.node('app-ai-sidebar').getAttribute('aria-hidden') === 'true');
  await wait(() => !app.window.document.querySelector('.ai-source-popover'));
  assert.equal(app.ui.flow.getSnapshot().conversation.draft, 'Draft remains');
  assert.equal(app.node('meeting-notes').value, '보존할 메모 초안');
  assert.equal(app.node('notes-section').hidden, false);
});

test('assistant markup is escaped by the shared renderer', async t => {
  const app = await fixture(t, { delay: 10000 }); app.ui.flow.edit('Question'); await app.ui.flow.send();
  const sent = app.calls.find(call => call[0] === 'send')[1];
  app.ui.flow.receive({ requestId: sent.requestId, conversationId: sent.conversationId, type: 'completed', messageId: 'unsafe-text', text: '<img src=x onerror=alert(1)> **Text**' });
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

test('account login completes automatically and cancellation stays enabled while waiting', async t => {
  const app = await fixture(t, { scenario: 'oauth' }); app.ui.setSettingsActive(true);
  const panel = app.node('ai-account-settings');
  await wait(() => panel.querySelector('.settings-client-row'));
  const chatgpt = panel.querySelector('.settings-client-row');
  await wait(() => ![...chatgpt.querySelectorAll('button')].find(node => node.textContent === '로그인').disabled);
  [...chatgpt.querySelectorAll('button')].find(node => node.textContent === '로그인').click();
  await wait(() => app.calls.some(call => call[0] === 'finishConnect'));
  assert.match(chatgpt.textContent, /로그인 대기/); assert.doesNotMatch(chatgpt.textContent, /연결 확인/);
  assert.equal([...chatgpt.querySelectorAll('button')].find(node => node.textContent === '취소').disabled, false);
  app.completeLogin(); await wait(() => chatgpt.textContent.includes('로그인됨'));
  assert.equal([...chatgpt.querySelectorAll('button')].some(node => node.textContent === '취소'), false);
});

test('Notion login completes automatically without a confirmation button', async t => {
  const app = await fixture(t, { scenario: 'oauth' }); app.ui.setSettingsActive(true);
  const panel = app.node('ai-account-settings');
  await wait(() => app.ui.flow.getSnapshot().notion !== null);
  const notion = [...panel.querySelectorAll('.settings-client-row')].find(node => node.textContent.includes('Notion'));
  [...notion.querySelectorAll('button')].find(node => node.textContent === '연결').click();
  await wait(() => app.ui.flow.getSnapshot().connectionPhases.notion === 'waiting');
  await wait(() => notion.textContent.includes('로그인 대기'));
  assert.doesNotMatch(notion.textContent, /연결 확인/);
  assert.equal([...notion.querySelectorAll('button')].find(node => node.textContent === '취소').disabled, false);
  app.completeNotion(); await wait(() => notion.textContent.includes('연결됨'));
});

test('account cancel remains usable during automatic finish and ignores its late callback', async t => {
  const app = await fixture(t, { scenario: 'oauth' }); app.ui.setSettingsActive(true);
  const panel = app.node('ai-account-settings'); await wait(() => panel.querySelector('.settings-client-row'));
  const chatgpt = panel.querySelector('.settings-client-row');
  await wait(() => ![...chatgpt.querySelectorAll('button')].find(node => node.textContent === '로그인').disabled);
  [...chatgpt.querySelectorAll('button')].find(node => node.textContent === '로그인').click();
  await wait(() => app.calls.some(call => call[0] === 'finishConnect'));
  await wait(() => [...chatgpt.querySelectorAll('button')].some(node => node.textContent === '취소'));
  [...chatgpt.querySelectorAll('button')].find(node => node.textContent === '취소').click();
  await wait(() => app.calls.some(call => call[0] === 'cancelConnect'));
  app.completeLogin(); await wait(() => chatgpt.textContent.includes('로그인 필요'));
  assert.equal(app.ui.flow.getSnapshot().accounts[0].state, 'signed_out');
});

test('answer copy menu offers native text and Markdown formats without citation settings', async t => {
  const app = await fixture(t); app.ui.flow.edit('Question'); await app.ui.flow.send();
  await wait(() => app.node('ai-chat-inspector').querySelector('[aria-label="AI 답변 복사"]'));
  app.node('ai-chat-inspector').querySelector('[aria-label="AI 답변 복사"]').click();
  await wait(() => app.window.document.querySelector('.nero-action-popover'));
  const menu = app.window.document.querySelector('.nero-action-popover');
  assert.match(menu.textContent, /텍스트로 복사/); assert.match(menu.textContent, /마크다운으로 복사/); assert.equal(menu.querySelector('input'), null);
  [...menu.querySelectorAll('button')].find(node => node.textContent === '마크다운으로 복사').click();
  await wait(() => app.calls.some(call => call[0] === 'clipboard'));
  assert.equal(app.calls.find(call => call[0] === 'exportAnswer')[1].format, 'md'); assert.match(app.calls.find(call => call[0] === 'clipboard')[1], /^##/);
  await wait(() => !app.window.document.querySelector('.nero-action-popover'));
});

test('copy menu closes with AI and clipboard errors remain visible in the chat', async t => {
  const app = await fixture(t, { scenario: 'clipboard-failed' }); app.ui.flow.edit('Question'); await app.ui.flow.send();
  await wait(() => app.node('ai-chat-inspector').querySelector('[aria-label="AI 답변 복사"]'));
  app.node('ai-chat-inspector').querySelector('[aria-label="AI 답변 복사"]').click();
  await wait(() => app.window.document.querySelector('.nero-action-popover'));
  [...app.window.document.querySelectorAll('.nero-action-popover button')].find(node => node.textContent === '텍스트로 복사').click();
  await wait(() => app.node('ai-chat-inspector').querySelector('[role="alert"]')?.textContent.includes('클립보드'));
  assert.equal(app.ui.flow.getSnapshot().conversation.feedback, '');
  app.ui.close(); await wait(() => !app.window.document.querySelector('.nero-action-popover'));
});

test('the account settings display and reconnect the active plan-disabled account consistently', async t => {
  const app = await fixture(t, { scenario: 'plan-disabled-multi' }); app.ui.setSettingsActive(true);
  const panel = app.node('ai-account-settings');
  await wait(() => panel.textContent.includes('활성 계정의 구독 권한을 확인해 주세요.'));
  const row = [...panel.querySelectorAll('.settings-client-row')].find(node => node.textContent.includes('ChatGPT'));
  await wait(() => ![...row.querySelectorAll('button')].find(node => node.textContent === '로그인').disabled);
  assert.match(row.textContent, /구독 확인 필요/);
  [...row.querySelectorAll('button')].find(node => node.textContent === '로그인').click();
  await wait(() => app.calls.some(call => call[0] === 'connect'));
  assert.equal(app.calls.find(call => call[0] === 'connect')[2], 'active-plan-account');
});
