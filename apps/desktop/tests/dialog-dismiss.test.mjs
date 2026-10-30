import assert from 'node:assert/strict';
import test from 'node:test';
import { Window } from 'happy-dom';
import { bindDialogDismiss } from '../src/dialog-dismiss.js';

test('the Escape that opens a discard confirmation cannot dismiss it again at document level', async t => {
  const window = new Window();
  t.after(() => window.happyDOM.abort());
  const dialog = window.document.createElement('dialog');
  const input = window.document.createElement('input');
  dialog.append(input);
  window.document.body.append(dialog);
  dialog.showModal();
  let confirmationOpen = false;
  bindDialogDismiss(dialog, { dismiss() {
    confirmationOpen = true;
    window.document.addEventListener('keydown', event => {
      if (event.key === 'Escape') confirmationOpen = false;
    }, { once: true });
  } });
  input.dispatchEvent(new window.KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }));
  assert.equal(confirmationOpen, true);
  assert.equal(dialog.open, true);
  // A later Escape may cancel the confirmation independently.
  window.document.dispatchEvent(new window.KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }));
  assert.equal(confirmationOpen, false);
});
