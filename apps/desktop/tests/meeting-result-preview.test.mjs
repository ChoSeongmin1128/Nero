import assert from 'node:assert/strict';
import test from 'node:test';
import { meetingResultPreview } from '../src/meeting-result-preview.js';

const meeting = { id: 'meeting', audio: { sessionId: 'capture' } };
const preview = { meetingId: 'meeting', sessionId: 'capture', text: '보존된 초안', words: [] };
const result = extra => meetingResultPreview({ meeting, preview, ...extra });

test('a saved capture keeps its bound draft after the active session disappears', () => {
  const view = result({ processing: { id: 'job', kind: 'transcribe', status: 'running', meetingId: meeting.id } });
  assert.equal(view.visible, true);
  assert.equal(view.busy, true);
  assert.equal(view.draft, true);
  assert.deepEqual(view.rows, [{ start: null, end: null, text: preview.text, speaker: null }]);
});

test('a different capture or meeting never supplies a preview', () => {
  assert.deepEqual(result({ preview: { ...preview, sessionId: 'other' } }).rows, []);
  assert.deepEqual(result({ preview: { ...preview, meetingId: 'other' } }).rows, []);
  assert.deepEqual(result({ meeting: { id: meeting.id } }).rows, []);
});

test('canonical rows, including an empty final result, replace the draft', () => {
  const segments = [{ id: 'segment', text: '확정 기록', start: 1, end: 2 }];
  assert.equal(result({ transcript: { segments } }).rows, segments);
  assert.equal(result({ transcript: { segments } }).draft, false);
  assert.deepEqual(result({ transcript: { segments: [] } }).rows, []);
  assert.equal(result({ transcript: { segments: [] } }).draft, false);
});

test('only the current capture finalization or text jobs keep the preview busy', () => {
  assert.equal(result({ processing: { id: 'other', meetingId: 'other', kind: 'transcribe', status: 'running' } }).busy, false);
  assert.equal(result({ processing: { id: 'model', kind: 'summary-model', status: 'running' } }).busy, false);
  assert.equal(result({ session: { id: 'capture', meetingId: meeting.id, state: 'finalizing' } }).busy, true);
  for (const status of ['failed', 'cancelled', 'interrupted', 'completed']) {
    const view = result({ processing: { id: 'job', meetingId: meeting.id, kind: 'transcribe', status } });
    assert.equal(view.busy, false);
    assert.equal(view.rows[0].text, preview.text);
  }
});

test('recording keeps inline notes and does not replace the body with a preview', () => {
  assert.equal(result({ session: { id: 'capture', meetingId: meeting.id, state: 'recording' } }).visible, false);
  assert.equal(result({ session: { id: 'capture', meetingId: meeting.id, state: 'paused' } }).visible, false);
  assert.equal(result({ meeting: { id: meeting.id }, preview: null }).visible, false);
});
