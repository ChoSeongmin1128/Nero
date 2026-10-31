import assert from 'node:assert/strict';
import test from 'node:test';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';
import { Window } from 'happy-dom';
const root = fileURLToPath(new URL('../', import.meta.url));
const bundle = await build({ entryPoints: [root + 'src/template-settings.jsx'], bundle: true, write: false, format: 'iife', globalName: 'TemplateUI', jsx: 'automatic', nodePaths: [root + 'node_modules'], define: { 'process.env.NODE_ENV': '"production"' } });
const wait = async predicate => { for (let n = 0; n < 100; n++) { await new Promise(done => setTimeout(done, 5)); if (predicate()) return; } assert.fail('Template settings did not settle'); };
const builtin = { id: 'meeting', name: '회의', description: '합성 용도', prompt: '합성 기본 지침', revision: 'r1', builtIn: true, customized: false };
const custom = { ...builtin, id: 'custom', name: '직접 추가', builtIn: false };
async function fixture(t) {
  const window = new Window({ settings: { enableJavaScriptEvaluation: true, suppressInsecureJavaScriptEnvironmentWarning: true } });
  window.document.body.innerHTML = '<dialog><section></section></dialog>'; window.eval(bundle.outputFiles[0].text);
  const dialog = window.document.querySelector('dialog'), panel = dialog.querySelector('section'); dialog.showModal();
  let items = [builtin], revision = 'catalog:1', resets = 0;
  const editor = window.TemplateUI.mountTemplateSettings({ container: dialog, panel, bridge: {
    readTemplateCatalog: async () => ({ revision, templates: items }), getTemplate: async id => items.find(item => item.id === id),
    deleteTemplate: async (id, expected) => { assert.equal(expected, builtin.revision); items = items.filter(item => item.id !== id); revision = 'catalog:2'; },
    resetTemplateCatalog: async expected => { resets++; if (expected !== revision) throw Error('템플릿 목록이 변경되었습니다.'); items = [builtin]; revision = 'catalog:3'; return { revision, templates: items }; },
  } });
  t.after(() => { editor.dispose(); window.happyDOM.abort(); }); editor.setActive(true);
  await wait(() => panel.querySelector('.template-settings-row'));
  const button = text => [...panel.querySelectorAll('button')].find(node => node.textContent === text);
  return { window, panel, button, items: () => items, resets: () => resets, addExternally() { items = [builtin, custom]; revision = 'catalog:2'; } };
}

test('built-in templates can be deleted and an empty catalog offers reset without a routine refresh', async t => {
  const f = await fixture(t); assert.equal(f.button('새로고침'), undefined);
  f.panel.querySelector('[aria-label="회의 편집"]').click();
  await wait(() => f.panel.querySelector('[aria-label="템플릿 삭제"]'));
  f.panel.querySelector('[aria-label="템플릿 삭제"]').click();
  await wait(() => f.window.document.querySelector('[role="alertdialog"]'));
  assert.match(f.window.document.querySelector('[role="alertdialog"]').textContent, /기존 회의와 저장된 요약은 유지/);
  f.window.document.querySelector('[role="alertdialog"] button.danger').click();
  await wait(() => f.panel.textContent.includes('템플릿이 없습니다.'));
  assert.equal(f.items().length, 0); assert.equal(f.button('전체 초기화').disabled, false);
  assert.equal(f.button('새로고침'), undefined);
});

test('reset-all explains restoration and removal, and cancellation sends no write', async t => {
  const f = await fixture(t); f.addExternally();
  f.button('전체 초기화').click(); await wait(() => f.window.document.querySelector('[role="alertdialog"]'));
  const dialog = f.window.document.querySelector('[role="alertdialog"]');
  assert.match(dialog.textContent, /삭제한 기본 템플릿이 다시 생깁니다/);
  assert.match(dialog.textContent, /직접 추가한 템플릿은 모두 지워집니다/);
  assert.match(dialog.textContent, /저장된 요약은 바뀌지 않습니다/);
  dialog.querySelector('button').click();
  await wait(() => !f.window.document.querySelector('[role="alertdialog"]'));
  assert.equal(f.resets(), 0); assert.equal(f.items().length, 2);
});

test('stale reset cannot erase a concurrent template and retry requires an explicit fresh read', async t => {
  const f = await fixture(t); f.addExternally();
  f.button('전체 초기화').click(); await wait(() => f.window.document.querySelector('[role="alertdialog"] button.danger'));
  f.window.document.querySelector('[role="alertdialog"] button.danger').click();
  await wait(() => f.panel.querySelector('[role="alert"]'));
  assert.equal(f.items().length, 2); assert.equal(f.resets(), 1);
  f.button('다시 불러오기').click(); await wait(() => f.panel.querySelectorAll('.template-settings-row').length === 2);
  f.button('전체 초기화').click(); await wait(() => f.window.document.querySelector('[role="alertdialog"] button.danger'));
  f.window.document.querySelector('[role="alertdialog"] button.danger').click();
  await wait(() => f.items().length === 1 && !f.panel.querySelector('[role="alert"]'));
  assert.equal(f.resets(), 2); assert.equal(f.items()[0].builtIn, true);
});
