import assert from 'node:assert/strict';
import test from 'node:test';
import { createDocumentActionsFlow } from '../src/document-actions-flow.js';

const setup = (onExport, kind = 'summary') => {
  const flow = createDocumentActionsFlow({ kind, onExport, schedule: () => 1, cancel() {} });
  flow.update({ id: 'meeting-a', available: true, busy: false });
  return flow;
};
const deferred = () => { let resolve, reject; const promise = new Promise((yes, no) => { resolve = yes; reject = no; }); return { promise, resolve, reject }; };

test('TXT and Markdown copies and file exports omit citations by default', async () => {
  const calls = [];
  const flow = setup(async (...args) => { calls.push(args); });
  for (const copy of [true, false]) for (const format of ['txt', 'md']) {
    flow.open(copy ? 'copy' : 'export');
    await flow.perform(format, copy);
  }
  assert.deepEqual(calls, [
    ['summary', 'txt', true, false, 'meeting-a'], ['summary', 'md', true, false, 'meeting-a'],
    ['summary', 'txt', false, false, 'meeting-a'], ['summary', 'md', false, false, 'meeting-a'],
  ]);
});

test('citation opt-in applies only to this meeting and never to transcript text', async () => {
  const calls = [];
  const flow = setup(async (...args) => { calls.push(args); });
  flow.include(true); await flow.perform('md', true);
  flow.update({ id: 'meeting-b', available: true, busy: false });
  await flow.perform('md', true);
  const transcript = setup(async (...args) => { calls.push(args); }, 'transcript');
  transcript.include(true); await transcript.perform('txt', true);
  assert.deepEqual(calls.map(call => [call[0], call[3], call[4]]), [['summary', true, 'meeting-a'], ['summary', false, 'meeting-b'], ['transcript', false, 'meeting-a']]);
});

test('duplicate clicks are blocked and a clipboard failure retains the menu for retry', async () => {
  const pending = deferred(), calls = [];
  const flow = setup(async (...args) => { calls.push(args); return pending.promise; });
  flow.open('copy'); const action = flow.perform('txt', true);
  await flow.perform('md', true); assert.equal(calls.length, 1);
  pending.reject(Error('복사 권한을 확인해 주세요.')); await action;
  assert.equal(flow.getSnapshot().pending, false);
  assert.equal(flow.getSnapshot().open, 'copy');
  assert.match(flow.getSnapshot().error, /권한/);
  assert.equal(flow.getSnapshot().copied, false);
});

test('changing meetings ignores a late response and resets citation choice', async () => {
  const pending = deferred(), flow = setup(() => pending.promise);
  flow.include(true); flow.open('copy'); const action = flow.perform('md', true);
  flow.update({ id: 'meeting-b', available: true, busy: false });
  pending.resolve(true); await action;
  assert.deepEqual(flow.getSnapshot(), { open: null, pending: false, copied: false, includeCitations: false, error: '' });
});

test('unavailable or processing documents cannot start copy/export', async () => {
  let calls = 0;
  const flow = setup(async () => { calls++; });
  for (const context of [{ id: 'a', available: false, busy: false }, { id: 'a', available: true, busy: true }]) {
    flow.update(context); flow.open('copy'); await flow.perform('txt', true);
    assert.equal(flow.getSnapshot().open, null);
  }
  assert.equal(calls, 0);
});


test('copy popover exposes both formats, keeps citations opt-in, and Escape restores focus', async t => {
  const { build } = await import('esbuild');
  const { Window } = await import('happy-dom');
  const { fileURLToPath } = await import('node:url');
  const app = fileURLToPath(new URL('../', import.meta.url));
  const bundled = await build({ entryPoints: [app + 'src/document-actions.jsx'], bundle: true, write: false, format: 'iife', globalName: 'NeroDocumentActions', nodePaths: [app + 'node_modules'], define: { 'process.env.NODE_ENV': '"production"' }, jsx: 'automatic' });
  const window = new Window({ url: 'http://nero.test', settings: { enableJavaScriptEvaluation: true, suppressInsecureJavaScriptEnvironmentWarning: true } });
  t.after(() => window.happyDOM.abort());
  window.document.body.innerHTML = '<div id="actions"></div>';
  window.eval(bundled.outputFiles[0].text);
  const calls = [];
  const actions = window.NeroDocumentActions.mountDocumentActions({ container: window.document.getElementById('actions'), kind: 'summary', onExport: async (...args) => { calls.push(args); return true; } });
  const wait = async predicate => { for (let attempt=0;attempt<100;attempt++) { if(predicate()) return; await new Promise(resolve=>setTimeout(resolve,5)); } assert.fail('Expected document action state was not rendered'); };
  actions.update({ id: 'meeting-a', available: true, busy: false });
  await wait(() => window.document.querySelector('[aria-label="요약 복사"]'));
  const trigger = window.document.querySelector('[aria-label="요약 복사"]');
  trigger.focus(); trigger.click();
  await wait(() => window.document.querySelector('[data-slot="popover-content"]'));
  const buttons = [...window.document.querySelectorAll('[data-slot="popover-content"] button')];
  assert.deepEqual(buttons.map(button=>button.textContent), ['텍스트로 복사', '마크다운으로 복사']);
  assert.equal(window.document.querySelector('[data-slot="popover-content"] input').checked, false);
  window.document.activeElement.dispatchEvent(new window.KeyboardEvent('keydown', { key:'Escape', code:'Escape', bubbles:true }));
  await wait(() => trigger.getAttribute('aria-expanded') === 'false');
  await wait(() => window.document.activeElement === trigger);
  trigger.click(); await wait(() => trigger.getAttribute('aria-expanded') === 'true');
  [...window.document.querySelectorAll('[data-slot="popover-content"] button')].find(button=>button.textContent === '마크다운으로 복사').click();
  await wait(() => calls.length === 1);
  assert.deepEqual(calls[0], ['summary','md',true,false,'meeting-a']);
});
