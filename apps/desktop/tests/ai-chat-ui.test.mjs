import assert from 'node:assert/strict';
import { markdownSource as buildMarkdownSource } from '../scripts/build-markdown.mjs';
import test from 'node:test';
import { readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';
import { Window } from 'happy-dom';
const root = fileURLToPath(new URL('../', import.meta.url));
const html = await readFile(new URL('../src/index.html', import.meta.url), 'utf8');
const shared = await buildMarkdownSource() + '\n' + await readFile(new URL('../../../design-system/components.js', import.meta.url), 'utf8');
const bundle = await build({ entryPoints: [root + 'tests/fixtures/ai-chat-fixture.jsx'], bundle: true, write: false, format: 'iife', globalName: 'AiFixture', jsx: 'automatic', nodePaths: [root + 'node_modules'], define: { 'process.env.NODE_ENV': '"production"' } });
const wait = async predicate => { for (let i = 0; i < 120; i++) { if (predicate()) return; await new Promise(resolve => setTimeout(resolve, 5)); } assert.fail('AI UI did not settle'); };
const menuButtons = window => [...window.document.querySelectorAll('.ai-model-popover button')];
const menuButton = (window, text) => menuButtons(window).find(button => button.textContent.trim() === text);
const openProvider = async (window, provider) => {
  await wait(() => window.document.querySelector(`.ai-model-popover [data-provider="${provider}"]`));
  window.document.querySelector(`.ai-model-popover [data-provider="${provider}"]`).click();
  await wait(() => window.document.querySelector('.ai-model-back'));
};
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
    await openProvider(window, state.provider);
    await wait(() => menuButton(window, '예시 모델'));
    menuButton(window, '예시 모델').click();
    await wait(() => !window.document.querySelector('.ai-model-popover'));
  }
  const chosen = app.ui.flow.getSnapshot(), chosenLabel = chosen.models.find(model => model.id === chosen.modelId)?.label;
  if (chosenLabel) await wait(() => node('ai-chat-inspector').querySelector('[aria-label="AI 모델 선택"]')?.textContent.includes(chosenLabel));
  await window.happyDOM.whenAsyncComplete();
  return { ...app, window, node, button: text => { const buttons = [...node('ai-chat-inspector').querySelectorAll('button')]; return buttons.find(button => button.getAttribute('aria-label') === text) || buttons.find(button => button.textContent.trim() === text); } };
}

test('model picker lists connected providers and browses them without changing the selection', async t => {
  const app = await fixture(t);
  const trigger = app.button('AI 모델 선택');
  assert.ok(trigger.querySelector('img[src*="chatgpt"]')); assert.equal(trigger.querySelector('svg.lucide-bot'), null);
  trigger.click();
  await wait(() => app.window.document.querySelector('.ai-model-popover'));
  const menu = app.window.document.querySelector('.ai-model-popover');
  assert.ok(menu.querySelector('[data-provider="chatgpt"] img[src*="chatgpt"]'));
  assert.equal(menu.querySelector('[data-provider="claude"]'), null);
  assert.ok(menuButton(app.window, '계정 설정'));
  await openProvider(app.window, 'chatgpt');
  assert.equal(menuButton(app.window, '예시 모델').getAttribute('aria-pressed'), 'true');
  menuButton(app.window, 'ChatGPT').click();
  await wait(() => menu.querySelector('[data-provider="chatgpt"]'));
  assert.equal(app.ui.flow.getSnapshot().provider, 'chatgpt');
  assert.equal(app.ui.flow.getSnapshot().modelId, 'example-model');
  app.window.document.activeElement.dispatchEvent(new app.window.KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
  await wait(() => !app.window.document.querySelector('.ai-model-popover'));
});

test('a provider placeholder without an account is neither listed nor probed', async t => {
  const app = await fixture(t, { scenario: 'placeholder', prepareModels: false });
  await wait(() => app.ui.flow.getSnapshot().provider === 'claude');
  app.button('AI 모델 선택').click();
  await wait(() => app.window.document.querySelector('.ai-model-popover [data-provider="claude"]'));
  assert.equal(app.window.document.querySelector('.ai-model-popover [data-provider="chatgpt"]'), null);
  await app.window.happyDOM.whenAsyncComplete();
  assert.equal(app.calls.some(call => call[0] === 'models' && call[1] === ''), false);
  assert.equal(app.node('ai-chat-inspector').textContent.includes('AI 계정에 로그인해 주세요.'), false);
});

test('Claude models keep earlier releases folded, search across providers and show the documented effort', async t => {
  const app = await fixture(t, { scenario: 'claude-catalog', prepareModels: false });
  await wait(() => app.ui.flow.getSnapshot().provider === 'claude');
  const trigger = app.button('AI 모델 선택'); trigger.click();
  await openProvider(app.window, 'claude');
  await wait(() => menuButton(app.window, 'Opus 5.5'));
  assert.equal(menuButton(app.window, 'Opus 4.7'), undefined);
  menuButton(app.window, '이전 모델').click();
  await wait(() => menuButton(app.window, 'Opus 4.7'));
  const search = app.window.document.querySelector('.ai-model-search input');
  Object.getOwnPropertyDescriptor(app.window.HTMLInputElement.prototype, 'value').set.call(search, '4.7');
  search.dispatchEvent(new app.window.Event('input', { bubbles: true }));
  await wait(() => app.window.document.querySelector('.ai-model-popover .ai-model-back') === null && menuButton(app.window, 'Opus 4.7'));
  assert.ok(menuButton(app.window, 'Opus 4.7').querySelector('img[src*="claude"]'));
  assert.equal(menuButton(app.window, 'Sonnet 5.5'), undefined);
  menuButton(app.window, 'Opus 4.7').click();
  await wait(() => !app.window.document.querySelector('.ai-model-popover'));
  assert.match(trigger.textContent, /Opus 4\.7/); assert.ok(trigger.querySelector('img[src*="claude"]'));
  const effort = app.node('ai-chat-inspector').querySelector('.ai-effort-trigger');
  assert.match(effort.textContent, /Extra High/); assert.equal(effort.textContent.includes('기본값'), false);
  effort.click();
  await wait(() => app.window.document.querySelector('.ai-effort-popover'));
  assert.deepEqual([...app.window.document.querySelectorAll('.ai-effort-popover button')].map(button => button.textContent.trim()), ['Low', 'Medium', 'High', 'Extra High', 'Max']);
  [...app.window.document.querySelectorAll('.ai-effort-popover button')].find(button => button.textContent.trim() === 'Max').click();
  await wait(() => app.ui.flow.getSnapshot().effort === 'max');
  assert.match(effort.textContent, /Max/);
});

for (const [runtime, label] of [['missing', '설치하고 로그인'], ['present', '로그인 확인']]) {
  test(`Claude ${runtime} installation exposes the appropriate explicit action`, async t => {
    const app = await fixture(t, { scenario: `claude-runtime-${runtime}`, prepareModels: false });
    app.ui.setSettingsActive(true);
    const row = () => [...app.node('ai-account-settings').querySelectorAll('.settings-client-row')].find(node => node.querySelector('.settings-client-name')?.textContent === 'Claude');
    await wait(() => row()?.textContent.includes(label));
    const button = [...row().querySelectorAll('button')].find(node => node.textContent === label);
    await wait(() => !button.disabled);
    assert.equal(app.calls.some(call => call[0] === 'connect'), false);
    button.click();
    await wait(() => app.calls.some(call => call[0] === 'connect' && call[1] === 'claude'));
  });
}

test('Escape closes the focused AI popover without restoring focus to an unrelated action menu', async t => {
  const app = await fixture(t);
  const details = app.node('meeting-detail').querySelector('[data-meeting-menu]');
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

test('navigating to another meeting shows its own new chat and coming back restores the draft', async t => {
  const app = await fixture(t);
  const textarea = () => app.node('ai-chat-inspector').querySelector('textarea');
  app.ui.flow.edit('첫 회의 질문');
  const next = { id: 'example-meeting-2', title: '다음 회의' };
  app.ui.setMeeting(next);
  app.detail.update({ id: next.id, showSummary: true, hasTranscript: true, summaryLoaded: true, transcriptLoaded: true });
  await wait(() => app.button('다음 회의 제외') && textarea().value === '');
  assert.equal(app.button('예시 회의 제외'), undefined);
  app.ui.flow.edit('다음 회의 질문');
  app.ui.setMeeting({ id: 'example-meeting', title: '예시 회의' });
  await wait(() => textarea().value === '첫 회의 질문' && app.button('예시 회의 제외'));
  app.ui.setMeeting(next);
  await wait(() => textarea().value === '다음 회의 질문' && app.button('다음 회의 제외'));
  app.node('ai-chat-inspector').querySelector('[aria-label="질문 보내기"]').click();
  await wait(() => app.calls.some(call => call[0] === 'send'));
  assert.deepEqual(Array.from(app.calls.find(call => call[0] === 'send')[1].meetingIds), ['example-meeting-2']);
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

test('saved account loads models when the model menu opens, without a refresh button', async t => {
  const app = await fixture(t, { scenario: 'unknown' });
  app.ui.setSettingsActive(true); await wait(() => !app.ui.flow.getSnapshot().accountsBusy);
  assert.equal(app.calls.filter(call => call[0] === 'models').length, 0);
  const trigger = app.button('AI 모델 선택');
  assert.match(trigger.textContent, /모델 선택/);
  trigger.click();
  await openProvider(app.window, 'chatgpt');
  await wait(() => app.window.document.querySelector('.ai-model-popover')?.textContent.includes('예시 모델'));
  assert.equal(app.window.document.querySelector('.ai-model-popover').textContent.includes('새로고침'), false);
  const loaded = app.calls.filter(call => call[0] === 'models').length;
  assert.ok(loaded >= 1);
  menuButton(app.window, '예시 모델').click();
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


test('slash chooser selects a skill without sending and requires a meeting target', async t => {
  const app = await fixture(t);
  const input = app.node('ai-chat-inspector').querySelector('textarea');
  Object.getOwnPropertyDescriptor(app.window.HTMLTextAreaElement.prototype, 'value').set.call(input, '/회의'); input.dispatchEvent(new app.window.Event('input', { bubbles: true }));
  await wait(() => app.node('ai-chat-inspector').querySelector('[role="listbox"]'));
  input.dispatchEvent(new app.window.KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true }));
  await wait(() => app.ui.flow.getSnapshot().conversation.skillId === 'meeting-summary');
  assert.equal(app.calls.some(call => call[0] === 'send'), false);
  assert.equal(app.ui.flow.getSnapshot().conversation.summaryTargetId, 'example-meeting');
  assert.equal(app.node('ai-chat-inspector').querySelector('[aria-label="요약할 회의"]'), null);
  await wait(() => app.ui.flow.getSnapshot().conversation.templateId === 'example-template');
  const chip = app.node('ai-chat-inspector').querySelector('.ai-skill-chip');
  assert.match(chip.textContent, /회의 요약/); assert.match(chip.textContent, /기본 요약/);
  assert.equal(app.node('ai-chat-inspector').querySelector('[role="listbox"]'), null);
  chip.querySelector('.ai-skill-template').click();
  await wait(() => [...app.node('ai-chat-inspector').querySelectorAll('[role=option]')].some(item => item.textContent.includes('기본 요약')));
  assert.equal([...app.node('ai-chat-inspector').querySelectorAll('[role=option]')].some(item => item.textContent.includes('회의 요약')), false);
  [...app.node('ai-chat-inspector').querySelectorAll('[role=option]')].find(item => item.textContent.includes('기본 요약')).click();
  await wait(() => !app.node('ai-chat-inspector').querySelector('[aria-label="질문 보내기"]').disabled);
  app.node('ai-chat-inspector').querySelector('[aria-label="질문 보내기"]').click();
  await wait(() => app.calls.some(call => call[0] === 'send'));
  assert.equal(app.calls.find(call => call[0] === 'send')[1].message, '');
  const bubble = () => app.node('ai-chat-inspector').querySelector('.ai-message-skill');
  await wait(() => bubble());
  assert.match(bubble().textContent, /회의 요약/); assert.match(bubble().textContent, /기본 요약/);
  assert.equal(app.node('ai-chat-inspector').textContent.includes('첨부한 회의를 요약해 주세요'), false);
  assert.equal(app.node('ai-chat-inspector').textContent.includes('템플릿:'), false);
  app.node('ai-chat-inspector').querySelector('.ai-message-meeting').click();
  await wait(() => app.calls.some(call => call[0] === 'openMeeting' && call[1] === 'example-meeting'));
});

test('templates are reached only through the summary skill', async t => {
  const app = await fixture(t);
  app.button('자료 추가').click();
  await wait(() => app.window.document.querySelector('.ai-source-popover'));
  const kinds = [...app.window.document.querySelectorAll('.ai-source-popover .ai-source-kinds button')].map(button => button.textContent.trim());
  assert.deepEqual(kinds, ['회의', '스킬', 'Notion 링크']);
  [...app.window.document.querySelectorAll('.ai-source-popover .ai-source-kinds button')].find(button => button.textContent.trim() === '스킬').click();
  await wait(() => [...app.window.document.querySelectorAll('.ai-source-popover .ai-attachment-option')].length);
  assert.deepEqual([...app.window.document.querySelectorAll('.ai-source-popover .ai-attachment-option')].map(button => button.textContent.trim()), ['회의 요약']);
  const input = app.node('ai-chat-inspector').querySelector('textarea');
  Object.getOwnPropertyDescriptor(app.window.HTMLTextAreaElement.prototype, 'value').set.call(input, '/기본'); input.dispatchEvent(new app.window.Event('input', { bubbles: true }));
  await app.window.happyDOM.whenAsyncComplete();
  assert.equal(app.node('ai-chat-inspector').querySelector('[role="listbox"]'), null);
});


test('IME Enter does not select a slash option or send', async t => {
  const app = await fixture(t);
  const input = app.node('ai-chat-inspector').querySelector('textarea');
  Object.getOwnPropertyDescriptor(app.window.HTMLTextAreaElement.prototype, 'value').set.call(input, '/회의');
  input.dispatchEvent(new app.window.Event('input', { bubbles: true }));
  await wait(() => app.node('ai-chat-inspector').querySelector('[role="listbox"]'));
  input.dispatchEvent(new app.window.KeyboardEvent('keydown', { key: 'Enter', isComposing: true, bubbles: true, cancelable: true }));
  assert.equal(app.ui.flow.getSnapshot().conversation.skillId, null);
  assert.equal(app.calls.some(call => call[0] === 'send'), false);
});

test('reopening model picker never probes models', async t => {
  const app = await fixture(t); const trigger = app.button('AI 모델 선택');
  const before = app.calls.filter(call => call[0] === 'models').length;
  trigger.click(); await openProvider(app.window, 'chatgpt');
  menuButton(app.window, '예시 모델').click();
  await wait(() => !app.window.document.querySelector('.ai-model-popover'));
  await app.window.happyDOM.whenAsyncComplete();
  trigger.click(); await wait(() => app.window.document.querySelector('.ai-model-popover'));
  assert.equal(app.calls.filter(call => call[0] === 'models').length, before);
});


test('Claude reconnect clears cached model without probing when the picker opens', async t => {
  const app = await fixture(t, { scenario: 'claude-runtime-present', prepareModels: false });
  await app.ui.flow.refreshModelGroup('claude'); app.ui.flow.chooseModel('claude', 'claude-code', 'example-model');
  const claudeModels = () => app.calls.filter(call => call[0] === 'models' && call[1] === 'claude-code').length;
  const before = claudeModels();
  void app.ui.flow.connect('claude');
  await wait(() => app.ui.flow.getSnapshot().connectionPhases.claude === 'waiting');
  assert.equal(app.ui.flow.getSnapshot().modelId, null);
  app.button('AI 모델 선택').click(); await openProvider(app.window, 'claude');
  const group = app.window.document.querySelector('.ai-model-popover');
  assert.match(group.textContent, /연결을 마치면 모델 목록을 불러옵니다/);
  assert.equal(group.textContent.includes('새로고침'), false);
  assert.equal(claudeModels(), before);
  app.completeLogin();
  await wait(() => group.textContent.includes('예시 모델'));
  [...group.querySelectorAll('button')].find(item => item.textContent.trim() === '예시 모델').click();
  await wait(() => app.ui.flow.getSnapshot().modelId === 'example-model');
});


test('send button selects the highlighted slash command instead of sending raw text', async t => {
  const app = await fixture(t); const input = app.node('ai-chat-inspector').querySelector('textarea');
  Object.getOwnPropertyDescriptor(app.window.HTMLTextAreaElement.prototype, 'value').set.call(input, '/회의');
  input.dispatchEvent(new app.window.Event('input', { bubbles: true }));
  await wait(() => app.node('ai-chat-inspector').querySelector('[role="option"]'));
  app.node('ai-chat-inspector').querySelector('[aria-label="질문 보내기"]').click();
  await wait(() => app.ui.flow.getSnapshot().conversation.skillId === 'meeting-summary');
  assert.equal(app.calls.some(call => call[0] === 'send'), false);
  assert.equal(app.ui.flow.getSnapshot().conversation.summaryTargetId, 'example-meeting');
});

test('unmatched slash text remains an ordinary question', async t => {
  const app = await fixture(t); const input = app.node('ai-chat-inspector').querySelector('textarea');
  Object.getOwnPropertyDescriptor(app.window.HTMLTextAreaElement.prototype, 'value').set.call(input, '/unknown-path');
  input.dispatchEvent(new app.window.Event('input', { bubbles: true }));
  await wait(() => app.ui.flow.getSnapshot().conversation.draft === '/unknown-path');
  app.node('ai-chat-inspector').querySelector('[aria-label="질문 보내기"]').click();
  await wait(() => app.calls.some(call => call[0] === 'send'));
  assert.equal(app.calls.find(call => call[0] === 'send')[1].message, '/unknown-path');
});

test('meeting history filters saved question attachments without switching the active chat', async t => {
  const app = await fixture(t);
  app.ui.flow.edit('첫 회의 질문'); await app.ui.flow.send();
  await wait(() => app.button('AI 답변 복사'));
  const first = app.ui.flow.getSnapshot().conversation.id;
  app.ui.setMeeting({ id: 'example-meeting-2', title: '다음 회의' });
  app.button('새 대화').click();
  await wait(() => app.ui.flow.getSnapshot().conversation.id === null);
  app.ui.flow.edit('두 번째 회의 질문');
  await wait(() => app.ui.flow.canSend());
  assert.equal(await app.ui.flow.send(), true);
  await wait(() => app.button('AI 답변 복사'));
  const second = app.ui.flow.getSnapshot().conversation.id;
  app.ui.flow.edit('유지할 초안');
  app.button('대화 기록').click();
  await wait(() => app.window.document.querySelector('.ai-history-popover'));
  const menu = () => app.window.document.querySelector('.ai-history-popover');
  await wait(() => menu().textContent.includes('두 번째 회의 질문'));
  assert.equal(menu().textContent.includes('첫 회의 질문'), false);
  [...menu().querySelectorAll('button')].find(button => button.textContent === '전체 대화').click();
  await wait(() => menu().textContent.includes('첫 회의 질문'));
  assert.equal(app.ui.flow.getSnapshot().conversation.id, second);
  assert.equal(app.ui.flow.getSnapshot().conversation.draft, '유지할 초안');
  [...menu().querySelectorAll('.ai-history-row > button:first-child')].find(button => button.textContent.includes('첫 회의 질문')).click();
  await wait(() => app.ui.flow.getSnapshot().conversation.id === first);
  assert.deepEqual(Array.from(app.ui.flow.getSnapshot().conversation.meetings, meeting => meeting.id), ['example-meeting']);
});

test('a sent question is recalled with the arrow keys and edited into a new version', async t => {
  const app = await fixture(t);
  const inspector = () => app.node('ai-chat-inspector');
  const composer = () => inspector().querySelector('textarea[aria-label="AI에게 질문"]');
  const press = (node, key) => node.dispatchEvent(new app.window.KeyboardEvent('keydown', { key, bubbles: true, cancelable: true }));
  const type = (node, value) => { Object.getOwnPropertyDescriptor(app.window.HTMLTextAreaElement.prototype, 'value').set.call(node, value); node.dispatchEvent(new app.window.Event('input', { bubbles: true })); };
  app.ui.flow.edit('첫 질문'); await app.ui.flow.send();
  await wait(() => app.button('AI 답변 복사'));
  press(composer(), 'ArrowUp'); await wait(() => composer().value === '첫 질문');
  press(composer(), 'ArrowDown'); await wait(() => composer().value === '');
  app.button('질문 수정').click();
  await wait(() => inspector().querySelector('textarea[aria-label="질문 수정"]'));
  type(inspector().querySelector('textarea[aria-label="질문 수정"]'), '고친 질문');
  await wait(() => app.button('다시 보내기') && !app.button('다시 보내기').disabled);
  app.button('다시 보내기').click();
  await wait(() => app.calls.filter(call => call[0] === 'send').length === 2);
  assert.match(app.calls.filter(call => call[0] === 'send')[1][1].editOf, /^question-/);
  await wait(() => inspector().querySelector('[aria-label="질문 버전"]')?.textContent.includes('2/2'));
  await wait(() => !app.ui.flow.getSnapshot().conversation.activeRequestId);
  assert.match(inspector().querySelector('[aria-label="AI 대화"]').textContent, /고친 질문/);
  assert.doesNotMatch(inspector().querySelector('[aria-label="AI 대화"]').textContent, /첫 질문/);
  app.button('이전 버전').click();
  await wait(() => inspector().querySelector('[aria-label="질문 버전"]')?.textContent.includes('1/2'));
  assert.match(inspector().querySelector('[aria-label="AI 대화"]').textContent, /첫 질문/);
  assert.ok(app.calls.some(call => call[0] === 'selectVersion'));
});
