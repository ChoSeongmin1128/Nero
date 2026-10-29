import assert from 'node:assert/strict';
import test from 'node:test';
import { readFile } from 'node:fs/promises';
import { Window } from 'happy-dom';
import { bindConnectionDialogKeys, createAgentConnectionFlow } from '../src/agent-connection-flow.js';
import { agentConnectionPrompt } from '../src/agent-connection-prompt.js';

const info = { available: true, executablePath: '/Example App/nero', dataDir: '/Example Library' };
const deferred = () => { let resolve, reject; const promise = new Promise((yes, no) => { resolve = yes; reject = no; }); return { promise, resolve, reject }; };

test('connection instructions precede copying and carry the displayed prompt exactly', async () => {
  const copied = [];
  const flow = createAgentConnectionFlow({ writeText: async text => copied.push(text) });
  await flow.copy();
  assert.equal(copied.length, 0);
  flow.open(info);
  const visible = flow.getSnapshot().prompt;
  await flow.copy();
  assert.equal(copied[0], visible);
  assert.equal(copied[0], agentConnectionPrompt(info));
  assert.equal(flow.getSnapshot().copied, true);
});

test('missing connection information still opens usable discovery instructions', async () => {
  const copied = [];
  const flow = createAgentConnectionFlow({ writeText: async text => copied.push(text) });
  flow.open(null);
  assert.match(flow.getSnapshot().prompt, /설치된 Nero 앱/);
  await flow.copy();
  assert.equal(copied.length, 1);
  assert.doesNotMatch(copied[0], /Example App|mcpServers/);
});

test('duplicate copy actions do not issue a second clipboard request', async () => {
  const pending = deferred();
  let writes = 0;
  const flow = createAgentConnectionFlow({ writeText: () => { writes += 1; return pending.promise; } });
  flow.open(info);
  const first = flow.copy();
  await flow.copy();
  assert.equal(writes, 1);
  pending.resolve();
  await first;
  assert.equal(flow.getSnapshot().copying, false);
});

test('closing and reopening cannot receive a previous copy success', async () => {
  const pending = deferred();
  const flow = createAgentConnectionFlow({ writeText: () => pending.promise });
  flow.open(info);
  const first = flow.copy();
  flow.close();
  flow.open({ ...info, dataDir: '/Another Library' });
  pending.resolve();
  await first;
  assert.equal(flow.getSnapshot().copied, false);
  assert.match(flow.getSnapshot().prompt, /Another Library/);
});

test('new native information replaces a fallback without stale copy feedback', async () => {
  const pending = deferred();
  const flow = createAgentConnectionFlow({ writeText: () => pending.promise });
  flow.open(null);
  const copy = flow.copy();
  flow.update(info);
  pending.reject(Error('private clipboard failure'));
  await copy;
  assert.equal(flow.getSnapshot().error, '');
  assert.equal(flow.getSnapshot().copying, false);
  assert.equal(flow.getSnapshot().prompt, agentConnectionPrompt(info));
});

test('polling unchanged paths does not clear copy confirmation', async () => {
  const flow = createAgentConnectionFlow({ writeText: async () => {} });
  flow.open(info);
  await flow.copy();
  flow.update({ ...info, clients: [{ id: 'claude', state: 'installation_unknown' }] });
  assert.equal(flow.getSnapshot().copied, true);
});

test('clipboard failures are bounded and can be retried', async () => {
  let rejected = true;
  const flow = createAgentConnectionFlow({ writeText: async () => { if (rejected) throw Error('/private/native/clipboard'); } });
  flow.open(info);
  await flow.copy();
  assert.equal(flow.getSnapshot().error, '복사하지 못했습니다. 다시 시도해 주세요.');
  assert.equal(flow.getSnapshot().copied, false);
  rejected = false;
  await flow.copy();
  assert.equal(flow.getSnapshot().copied, true);
  assert.equal(flow.getSnapshot().error, '');
});

test('IME Escape stays in the prompt while ordinary Escape closes only the child dialog', () => {
  const window = new Window();
  const container = window.document.createElement('dialog');
  const input = window.document.createElement('input');
  container.append(input);
  window.document.body.append(container);
  let opened = true, closes = 0, parentKeys = 0;
  window.document.body.addEventListener('keydown', () => { parentKeys += 1; });
  const unbind = bindConnectionDialogKeys(container, { isOpen: () => opened, close: () => { opened = false; closes += 1; } });
  const composing = new window.KeyboardEvent('keydown', { key: 'Escape', isComposing: true, bubbles: true, cancelable: true });
  input.dispatchEvent(composing);
  assert.equal(closes, 0);
  assert.equal(composing.defaultPrevented, false);
  const legacyIme = new window.KeyboardEvent('keydown', { key: 'Escape', keyCode: 229, bubbles: true, cancelable: true });
  input.dispatchEvent(legacyIme);
  assert.equal(closes, 0);
  assert.equal(legacyIme.defaultPrevented, false);
  const ordinary = new window.KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true });
  input.dispatchEvent(ordinary);
  assert.equal(closes, 1);
  assert.equal(ordinary.defaultPrevented, true);
  assert.equal(parentKeys, 2);
  unbind();
  window.happyDOM.abort();
});

test('manual connection owns a contextual dialog rather than a detached copy button', async () => {
  const html = await readFile(new URL('../src/index.html', import.meta.url), 'utf8');
  const component = await readFile(new URL('../src/agent-connection-dialog.jsx', import.meta.url), 'utf8');
  assert.doesNotMatch(html, /id="settings-agent-copy-prompt"/);
  assert.match(html, /다른 에이전트/);
  assert.match(html, /id="settings-agent-manual"[^>]+aria-label="다른 에이전트 연결"/);
  assert.match(component, /DialogContent container=\{container\}/);
  assert.match(component, /<DialogTitle>에이전트로 연결<\/DialogTitle>/);
  assert.match(component, /<pre aria-labelledby="agent-prompt-label"/);
  assert.match(component, /Claude Code, ChatGPT, Cursor 등의 대화에 붙여넣으세요/);
});
