import assert from 'node:assert/strict';
import test from 'node:test';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';
import { Window } from 'happy-dom';
const root = fileURLToPath(new URL('../', import.meta.url));
const bundle = await build({ stdin: { contents: `import React from 'react'; import {createRoot} from 'react-dom/client'; import {DomainGlossary} from './src/domain-glossary-ui.jsx'; window.mountGlossary = (node, bridge) => { const root = createRoot(node); const render = id => root.render(<DomainGlossary meetingId={id} bridge={bridge}/>); render("meeting"); window.switchGlossary = render; return () => root.unmount(); };`, resolveDir: root, loader: 'jsx' }, bundle: true, write: false, format: 'iife', nodePaths: [root + 'node_modules'], define: { 'process.env.NODE_ENV': '"production"' } });
const wait = async predicate => { for (let n = 0; n < 100; n++) { await new Promise(done => setTimeout(done, 5)); if (predicate()) return; } assert.fail('Glossary UI did not settle'); };
test('후보를 선택하고 직접 편집한 후 저장하며 출처를 유지합니다', async t => {
  const window = new Window({ settings: { enableJavaScriptEvaluation: true, suppressInsecureJavaScriptEnvironmentWarning: true } });
  window.document.body.innerHTML = '<main></main>'; window.eval(bundle.outputFiles[0].text);
  let saved;
  const dispose = window.mountGlossary(window.document.querySelector('main'), {
    domainGlossaryView: async () => ({ snapshot: { revision: 3, entries: [] }, candidates: [{ term: 'API', sourceId: 'source-1', sourceTitle: '자료' }], reusable: [{ term: '승인 용어', sourceId: 'older-source', meetingTitle: '이전 회의' }], unavailableSources: 0 }),
    domainGlossarySave: async request => { saved = request; return { revision: 4, entries: request.entries }; },
  });
  t.after(() => { dispose(); window.happyDOM.abort(); });
  await wait(() => window.document.querySelector('button')); window.document.querySelector('button').click();
  await wait(() => window.document.querySelector('select option[value="0"]'));
  const select = window.document.querySelector('select'); select.value = '0'; select.dispatchEvent(new window.Event('change', { bubbles: true }));
  await wait(() => window.document.querySelector('input'));
  const input = window.document.querySelector('input');
  const setter = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value').set; setter.call(input, '편집한 API'); input.dispatchEvent(new window.Event('input', { bubbles: true }));
  await wait(() => input.value === '편집한 API');
  [...window.document.querySelectorAll('button')].find(button => button.textContent === '용어집 저장').click();
  await wait(() => saved);
  assert.equal(saved.meetingId, 'meeting'); assert.equal(saved.expectedRevision, 3); assert.equal(saved.entries[0].term, '편집한 API'); assert.equal(saved.entries[0].sourceId, 'source-1');
});

test('다른 회의 용어는 선택하기 전 자동으로 추가되지 않습니다', async t => {
  const window = new Window({ settings: { enableJavaScriptEvaluation: true, suppressInsecureJavaScriptEnvironmentWarning: true } });
  window.document.body.innerHTML = '<main></main>'; window.eval(bundle.outputFiles[0].text);
  let saved;
  const dispose = window.mountGlossary(window.document.querySelector('main'), {
    domainGlossaryView: async () => ({ snapshot: { revision: 0, entries: [] }, candidates: [], reusable: [{ term: '승인 용어', sourceId: 'old-revision', meetingTitle: '이전 회의' }], unavailableSources: 0 }),
    domainGlossarySave: async request => { saved = request; return { revision: 1, entries: request.entries }; },
  });
  t.after(() => { dispose(); window.happyDOM.abort(); });
  await wait(() => window.document.querySelector('button')); window.document.querySelector('button').click();
  await wait(() => window.document.querySelector('#domain-glossary-reuse')); assert.equal(window.document.querySelectorAll('input').length, 0);
  const select = window.document.querySelector('#domain-glossary-reuse'); select.value = '0'; select.dispatchEvent(new window.Event('change', { bubbles: true }));
  await wait(() => window.document.querySelector('input')); [...window.document.querySelectorAll('button')].find(button => button.textContent === '용어집 저장').click();
  await wait(() => saved); assert.equal(saved.entries[0].term, '승인 용어'); assert.equal(saved.entries[0].sourceId, 'old-revision');
});

for (const pendingPhase of ['load', 'save']) test(`회의 전환 시 늦은 ${pendingPhase} 응답을 무시하고 새 회의를 사용할 수 있습니다`, async t => {
  const window = new Window({ settings: { enableJavaScriptEvaluation: true, suppressInsecureJavaScriptEnvironmentWarning: true } });
  window.document.body.innerHTML = '<main></main>'; window.eval(bundle.outputFiles[0].text);
  let resolvePending;
  const pending = new Promise(resolve => { resolvePending = resolve; });
  const snapshot = (term, revision = 0) => ({ snapshot: { revision, entries: term ? [{ term, sourceId: 'source' }] : [] }, candidates: [], reusable: [], unavailableSources: 0 });
  let saveStarted = false;
  const dispose = window.mountGlossary(window.document.querySelector('main'), {
    domainGlossaryView: id => id === 'meeting' && pendingPhase === 'load' ? pending : Promise.resolve(snapshot(id === 'meeting' ? '이전' : '새 회의')),
    domainGlossarySave: () => { saveStarted = true; return pending; },
  });
  t.after(() => { dispose(); window.happyDOM.abort(); });
  await wait(() => window.document.querySelector('button')); window.document.querySelector('button').click();
  if (pendingPhase === 'save') { await wait(() => window.document.querySelector('input')); [...window.document.querySelectorAll('button')].find(b => b.textContent === '용어집 저장').click(); await wait(() => saveStarted); }
  else await wait(() => window.document.querySelector('[role="status"]'));
  window.switchGlossary('next'); await wait(() => window.document.querySelector('button').textContent.includes('검토'));
  window.document.querySelector('button').click(); await wait(() => window.document.querySelector('input')?.value === '새 회의');
  resolvePending(pendingPhase === 'load' ? snapshot('늦은 이전') : { revision: 1, entries: [{ term: '늦은 이전', sourceId: 'source' }] });
  await new Promise(done => setTimeout(done, 20));
  assert.equal(window.document.querySelector('input').value, '새 회의');
  assert.equal([...window.document.querySelectorAll('button')].find(b => b.textContent === '용어집 저장').disabled, false);
});

test('용어집을 접었다 다시 열어도 저장 전 편집을 유지합니다', async t => {
  const window = new Window({ settings: { enableJavaScriptEvaluation: true, suppressInsecureJavaScriptEnvironmentWarning: true } });
  window.document.body.innerHTML = '<main></main>'; window.eval(bundle.outputFiles[0].text);
  let reads = 0;
  const dispose = window.mountGlossary(window.document.querySelector('main'), {
    domainGlossaryView: async () => { reads++; return { snapshot: { revision: 1, entries: [{ term: '원래 표기', sourceId: 'source' }] }, candidates: [], reusable: [] }; },
    domainGlossarySave: async () => { throw new Error('이 검증은 저장하지 않습니다.'); },
  });
  t.after(() => { dispose(); window.happyDOM.abort(); });
  await wait(() => window.document.querySelector('button')); window.document.querySelector('button').click();
  await wait(() => window.document.querySelector('input'));
  const input = window.document.querySelector('input');
  Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value').set.call(input, '수정한 표기');
  input.dispatchEvent(new window.Event('input', { bubbles: true }));
  await wait(() => input.value === '수정한 표기');
  window.document.querySelector('button').click(); await wait(() => !window.document.querySelector('input'));
  window.document.querySelector('button').click(); await wait(() => window.document.querySelector('input'));
  assert.equal(window.document.querySelector('input').value, '수정한 표기'); assert.equal(reads, 1);
});

test('저장 중 접기로 완료 응답과 새 revision을 버리지 않습니다', async t => {
  const window = new Window({ settings: { enableJavaScriptEvaluation: true, suppressInsecureJavaScriptEnvironmentWarning: true } });
  window.document.body.innerHTML = '<main></main>'; window.eval(bundle.outputFiles[0].text);
  let resolveSave, requests = [];
  const dispose = window.mountGlossary(window.document.querySelector('main'), {
    domainGlossaryView: async () => ({ snapshot: { revision: 1, entries: [] }, candidates: [], reusable: [] }),
    domainGlossarySave: async request => { requests.push(request); if (requests.length === 1) return new Promise(resolve => { resolveSave = resolve; }); return { revision: 3, entries: [] }; },
  });
  t.after(() => { dispose(); window.happyDOM.abort(); });
  await wait(() => window.document.querySelector('button')); window.document.querySelector('button').click();
  const save = () => [...window.document.querySelectorAll('button')].find(button => button.textContent === '용어집 저장');
  await wait(() => save() && !save().disabled); save().click(); await wait(() => resolveSave);
  assert.equal(window.document.querySelector('button').disabled, true);
  resolveSave({ revision: 2, entries: [] }); await wait(() => !save().disabled);
  save().click(); await wait(() => requests.length === 2); assert.equal(requests[1].expectedRevision, 2);
});
