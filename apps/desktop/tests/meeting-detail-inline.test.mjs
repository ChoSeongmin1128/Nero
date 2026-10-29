import assert from 'node:assert/strict';
import test from 'node:test';
import { readFile } from 'node:fs/promises';
import { Window } from 'happy-dom';
import { mountMeetingDetail } from '../src/meeting-detail-ui.js';

const html = await readFile(new URL('../src/index.html', import.meta.url), 'utf8');
function page(t) {
  const window = new Window({ settings: { disableCSSFileLoading: true, disableJavaScriptFileLoading: true } });
  window.document.write(html);
  t.after(() => window.happyDOM.abort());
  const byId = id => window.document.getElementById(id);
  byId('meeting-detail').hidden = false;
  const controller = mountMeetingDetail({ byId, finishEditing: async action => action() });
  const update = values => controller.update({ id: 'draft-a', inlineNotes: true, showSummary: false, summaryLoaded: true, transcriptLoaded: true, ...values });
  return { window, byId, controller, update, editor: byId('meeting-notes') };
}

test('inline draft notes preserve editor identity, focus and composition across processing updates', t => {
  const { window, byId, update, editor } = page(t);
  update();
  editor.value = '회의 메모';
  editor.focus();
  editor.setSelectionRange(2, 4);
  editor.scrollTop = 240;
  editor.dispatchEvent(new window.Event('scroll'));
  editor.dispatchEvent(new window.CompositionEvent('compositionstart', { data: '메' }));
  let blurCount = 0;
  editor.addEventListener('blur', () => blurCount++);
  for (let index = 0; index < 3; index++) update({ inlineNotes: false, showSummary: true, hasTranscript: true });
  assert.equal(byId('notes-section').parentNode, byId('draft-notes-home'));
  assert.equal(window.document.activeElement, editor);
  assert.equal(editor.selectionStart, 2);
  assert.equal(editor.selectionEnd, 4);
  assert.equal(editor.scrollTop, 240);
  assert.equal(blurCount, 0);
  editor.dispatchEvent(new window.CompositionEvent('compositionend', { data: '메모' }));
  editor.blur();
  assert.equal(byId('notes-section').parentNode, byId('detail-inspector'));
  assert.equal(byId('meeting-notes'), editor);
  assert.equal(editor.value, '회의 메모');
  assert.equal(byId('detail-notes-open').hidden, false);
  assert.equal(byId('meeting-detail').dataset.inlineNotes, 'false');
});

test('inline draft notes restore their own scroll after visiting another meeting', t => {
  const { window, update, controller, editor } = page(t);
  update();
  assert.equal(controller.active(), 'summary');
  editor.scrollTop = 360;
  editor.dispatchEvent(new window.Event('scroll'));
  update({ id: 'draft-b' });
  assert.equal(editor.scrollTop, 0);
  editor.scrollTop = 90;
  editor.dispatchEvent(new window.Event('scroll'));
  update();
  assert.equal(editor.scrollTop, 360);
  update({ id: 'draft-b' });
  assert.equal(editor.scrollTop, 90);
});

test('opening and closing transcript returns the single notes editor to the draft body', async t => {
  const { byId, controller, update, editor } = page(t);
  update();
  editor.value = '보존할 메모';
  await controller.select('transcript', { focus: true });
  assert.equal(byId('detail-inspector').hidden, false);
  assert.equal(byId('notes-section').parentNode, byId('detail-inspector'));
  assert.equal(byId('notes-section').hidden, true);
  assert.equal(byId('meeting-detail').dataset.inspector, 'transcript');
  await controller.select('summary', { focus: true });
  assert.equal(byId('detail-inspector').hidden, true);
  assert.equal(byId('notes-section').parentNode, byId('draft-notes-home'));
  assert.equal(byId('notes-section').hidden, false);
  assert.equal(byId('meeting-notes'), editor);
  assert.equal(editor.value, '보존할 메모');
  assert.equal(editor.ownerDocument.querySelectorAll('#meeting-notes').length, 1);
});
