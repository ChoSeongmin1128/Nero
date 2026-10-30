import assert from 'node:assert/strict';
import test from 'node:test';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';
import { Window } from 'happy-dom';

const root = fileURLToPath(new URL('../', import.meta.url));
const bundle = await build({ entryPoints: [root + '../../design-system/components/ui/confirm-dialog.jsx'], bundle: true, write: false, format: 'iife', globalName: 'Confirmation', jsx: 'automatic', nodePaths: [root + 'node_modules'], define: { 'process.env.NODE_ENV': '"production"' } });
const wait = async condition => { for (let n = 0; n < 100; n++) { if (condition()) return; await new Promise(resolve => setTimeout(resolve, 5)); } assert.fail('Confirmation did not settle'); };

test('imperative confirmation retains the shared cancel and discard behavior inside a native dialog', async t => {
  const window = new Window({ settings: { enableJavaScriptEvaluation: true, suppressInsecureJavaScriptEnvironmentWarning: true } });
  t.after(() => window.happyDOM.abort());
  window.document.body.innerHTML = '<dialog><input aria-label="이름"></dialog>';
  window.eval(bundle.outputFiles[0].text);
  const dialog = window.document.querySelector('dialog'); dialog.showModal();
  const input = dialog.querySelector('input'); input.focus();
  const confirmation = window.Confirmation.mountConfirmDialog({ container: dialog });
  t.after(() => confirmation.dispose());
  const request = { title: '변경을 버릴까요?', description: '저장하지 않은 수정입니다.', action: '버리기', destructive: true };
  const cancelled = confirmation.confirm(request);
  await wait(() => dialog.querySelector('[role="alertdialog"] button'));
  dialog.querySelector('[role="alertdialog"] button').click();
  assert.equal(await cancelled, false);
  await wait(() => !dialog.querySelector('[role="alertdialog"]'));
  const accepted = confirmation.confirm(request);
  await wait(() => dialog.querySelector('[role="alertdialog"] button.danger'));
  dialog.querySelector('[role="alertdialog"] button.danger').click();
  assert.equal(await accepted, true);
  await wait(() => !dialog.querySelector('[role="alertdialog"]'));
  const closing = confirmation.confirm(request);
  await wait(() => dialog.querySelector('[role="alertdialog"]'));
  dialog.close();
  assert.equal(await closing, false);
});
