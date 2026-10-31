import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import { Window } from 'happy-dom';
import { mountMeetingDetail } from '../src/meeting-detail-ui.js';
const html = await readFile(new URL('../src/index.html', import.meta.url), 'utf8');
test('notes editor identity, IME selection and focus survive result/recording updates', async t => {
  const window = new Window({ settings: { disableCSSFileLoading: true, disableJavaScriptFileLoading: true } });
  t.after(() => window.happyDOM.abort()); window.document.write(html);
  const byId = id => window.document.getElementById(id); byId('meeting-detail').hidden = false;
  const detail = mountMeetingDetail({ byId, finishEditing: async action => action() });
  const update = state => detail.update({ id: 'a', showSummary: true, summaryLoaded: true, transcriptLoaded: true, ...state });
  update({ recording: true }); const editor = byId('meeting-notes'); editor.value = '보존할 회의 메모'; editor.focus(); editor.setSelectionRange(2, 4);
  let blur = 0; editor.addEventListener('blur', () => blur++);
  for (const state of [{ hasPreview: true }, { hasTranscript: true }, { hasSummary: true }, { recording: false }]) update(state);
  assert.equal(byId('meeting-notes'), editor); assert.equal(editor.parentElement, byId('notes-section'));
  assert.equal(window.document.activeElement, editor); assert.equal(editor.selectionStart, 2); assert.equal(editor.selectionEnd, 4); assert.equal(blur, 0);
  update({ hasTranscript: true }); await detail.select('transcript'); assert.equal(editor.value, '보존할 회의 메모'); assert.equal(byId('notes-section').hidden, false); assert.equal(byId('transcript-section').hidden, false);
});
