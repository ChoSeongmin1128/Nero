import assert from 'node:assert/strict';
import test from 'node:test';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';
import { Window } from 'happy-dom';

const root = fileURLToPath(new URL('../', import.meta.url));
const bundle = await build({ entryPoints: [root + 'src/people-ui.jsx'], bundle: true, write: false, format: 'iife', globalName: 'PeopleUI', jsx: 'automatic', nodePaths: [root + 'node_modules'], loader: { '.css': 'empty' }, define: { 'process.env.NODE_ENV': '"production"' } });
// A bridge call can be counted before its promise/React update has settled.
// Move to the next task before accepting controls left over from the prior event.
const wait = async condition => { for (let n = 0; n < 100; n++) { await new Promise(resolve => setTimeout(resolve, 5)); if (condition()) return; } assert.fail('People settings did not settle'); };
const clone = value => JSON.parse(JSON.stringify(value));
async function fixture(t) {
  const window = new Window({ url: 'http://nero.test', settings: { enableJavaScriptEvaluation: true, suppressInsecureJavaScriptEnvironmentWarning: true } });
  window.document.body.innerHTML = '<dialog><section></section></dialog>'; window.eval(bundle.outputFiles[0].text);
  const dialog = window.document.querySelector('dialog'), panel = dialog.querySelector('section'); dialog.showModal();
  let catalog = { editVersion: 'catalog:1', people: Array.from({ length: 50 }, (_, index) => ({ id: `p${index}`, name: `사람 ${index + 1}` })), groups: [{ id: 'g1', name: '제품팀', personIds: ['p0', 'p1'] }, { id: 'g2', name: '운영팀', personIds: ['p2'] }] };
  let conflict = false, reads = 0, writeGate = null; const writes = [];
  const editor = window.PeopleUI.mountPeopleSettings({ panel, container: dialog, bridge: {
    readPeopleCatalog: async () => { reads++; return clone(catalog); },
    replacePeopleCatalog: async (version, people, groups) => {
      assert.equal(version, catalog.editVersion); writes.push({ version, people: clone(people), groups: clone(groups) });
      if (writeGate) { const pending = writeGate; writeGate = null; await pending; }
      if (conflict) throw Error('다른 곳에서 목록을 수정했습니다.');
      catalog = { editVersion: `catalog:${writes.length + 1}`, people: clone(people), groups: clone(groups) }; return clone(catalog);
    },
  } });
  t.after(() => { editor.dispose(); window.happyDOM.abort(); }); editor.show();
  await wait(() => panel.querySelectorAll('.people-row').length === 50);
  const input = (element, value) => {
    Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value').set.call(element, value);
    element.dispatchEvent(new window.Event('input', { bubbles: true }));
  };
  const tab = kind => [...panel.querySelectorAll('[role="tab"]')].find(node => node.textContent.startsWith(kind));
  return { window, panel, editor, input, tab, writes, catalog: () => catalog, conflict: () => { conflict = true; }, reads: () => reads,
    pauseNextWrite() { let release; writeGate = new Promise(resolve => { release = resolve; }); return release; } };
}

test('groups have their own tab regardless of a fifty-person list', async t => {
  const f = await fixture(t);
  assert(f.tab('그룹').closest('[role="tablist"]'));
  f.tab('그룹').click();
  await wait(() => f.panel.querySelectorAll('.people-row').length === 2);
  assert(f.panel.querySelector('[aria-label="제품팀 편집"]'));
  assert(f.panel.querySelector('[aria-label="그룹 이름 연속 추가"]'));
  f.tab('사람').click();
  await wait(() => f.panel.querySelectorAll('.people-row').length === 50);
});

test('membership filter and continuous addition save atomically and keep input focus', async t => {
  const f = await fixture(t);
  f.panel.querySelector('[aria-label="소속 그룹 필터"]').click();
  await wait(() => f.window.document.querySelector('[role="option"]'));
  [...f.window.document.querySelectorAll('[role="option"]')].find(node => node.textContent === '제품팀').click();
  await wait(() => f.panel.querySelectorAll('.people-row').length === 2);
  const input = f.panel.querySelector('[aria-label="사람 이름 연속 추가"]'); input.focus();
  f.input(input, '새 사람,');
  await wait(() => f.writes.length === 1 && input.value === '' && !input.readOnly);
  assert.equal(f.window.document.activeElement, input);
  assert.equal(f.catalog().groups[0].personIds.length, 3);
  const paste = new f.window.Event('paste', { bubbles: true, cancelable: true });
  Object.defineProperty(paste, 'clipboardData', { value: { getData: () => '다음 사람\n또 다른 사람' } });
  input.dispatchEvent(paste);
  await wait(() => f.writes.length === 2 && input.value === '' && !input.readOnly);
  assert.equal(f.window.document.activeElement, input);
  assert.equal(f.catalog().groups[0].personIds.length, 5);
  f.input(input, 'Enter 이름');
  await wait(() => input.value === 'Enter 이름'); input.closest('form').requestSubmit();
  await wait(() => f.writes.length === 3 && input.value === '' && !input.readOnly);
  assert.equal(f.window.document.activeElement, input);
  assert.deepEqual(f.writes.map(write => write.version), ['catalog:1', 'catalog:2', 'catalog:3']);
});

test('continuous input stays focused and read-only until a pending save completes', async t => {
  const f = await fixture(t), release = f.pauseNextWrite();
  const input = f.panel.querySelector('[aria-label="사람 이름 연속 추가"]'); input.focus();
  f.input(input, '첫 사람,');
  await wait(() => input.readOnly);
  assert.equal(f.window.document.activeElement, input);
  assert.equal(await f.editor.beforeLeave(), false);
  assert.equal(f.catalog().people.length, 50);
  release(); await wait(() => !input.readOnly && input.value === '');
  assert.equal(f.window.document.activeElement, input);
  f.input(input, '다음 사람'); await wait(() => input.value === '다음 사람');
  input.closest('form').requestSubmit(); await wait(() => f.writes.length === 2 && input.value === '' && !input.readOnly);
  assert.equal(f.catalog().people.length, 52);
  assert.equal(f.window.document.activeElement, input);
});

test('group membership search, save and delete use the actual catalog write flow', async t => {
  const f = await fixture(t); f.tab('그룹').click();
  await wait(() => f.panel.querySelector('[aria-label="제품팀 편집"]'));
  f.panel.querySelector('[aria-label="제품팀 편집"]').click();
  await wait(() => f.panel.querySelector('[aria-label="그룹에 포함할 사람 검색"]'));
  f.input(f.panel.querySelector('[aria-label="그룹에 포함할 사람 검색"]'), '사람 3');
  await wait(() => f.panel.querySelectorAll('.people-member-list .people-check').length < 50);
  const member = [...f.panel.querySelectorAll('.people-check')].find(node => node.textContent === '사람 3');
  member.querySelector('input').click();
  f.panel.querySelector('.people-editor').requestSubmit();
  await wait(() => !f.panel.querySelector('.people-editor') && f.catalog().groups[0].personIds.includes('p2'));
  f.panel.querySelector('[aria-label="제품팀 삭제"]').click();
  await wait(() => f.window.document.querySelector('[role="alertdialog"] button.danger'));
  f.window.document.querySelector('[role="alertdialog"] button.danger').click();
  await wait(() => f.catalog().groups.length === 1);
  assert.equal(f.catalog().people.length, 50);
});

test('pending continuous input is guarded on dismissal and retained after a write conflict', async t => {
  const f = await fixture(t); const input = f.panel.querySelector('[aria-label="사람 이름 연속 추가"]');
  f.input(input, '보존할 이름'); await wait(() => input.value === '보존할 이름');
  const leave = f.editor.beforeLeave();
  await wait(() => f.window.document.querySelector('[role="alertdialog"] button'));
  f.window.document.querySelector('[role="alertdialog"] button').click();
  assert.equal(await leave, false); assert.equal(input.value, '보존할 이름');
  f.conflict(); input.closest('form').requestSubmit();
  await wait(() => f.panel.querySelector('[role="alert"]'));
  assert.equal(input.value, '보존할 이름');
  await f.editor.refreshIfClean(); assert.equal(f.reads(), 1);
});
