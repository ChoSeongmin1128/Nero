import assert from 'node:assert/strict';
import test from 'node:test';
import { snapshotSignatures, compareSnapshotSignatures } from '../src/snapshot-update.js';

const snapshot = () => ({
  meetings: [{ id: 'a', title: 'Meeting', notes: 'Notes', attendees: [{ id: 'person', name: 'Person' }], folderId: 'folder', deletedAt: null, audio: { frames: 10 }, captureQuality: { xrunCount: 0 }, calendarReference: { title: 'Event' } }],
  folders: [{ id: 'folder', name: 'Folder' }],
  session: { id: 'capture', meetingId: 'a', state: 'recording', elapsedMs: 1000, savedFrames: 16000, warning: '' },
});

test('only the recording clock and saved frame count bypass the full render', () => {
  const initial = snapshot(), signatures = snapshotSignatures(initial);
  assert.deepEqual(compareSnapshotSignatures(null, signatures), { changed: true, timerOnly: false });
  assert.deepEqual(compareSnapshotSignatures(signatures, snapshotSignatures(structuredClone(initial))), { changed: false, timerOnly: true });
  initial.session.elapsedMs += 1000;
  initial.session.savedFrames += 16000;
  assert.deepEqual(compareSnapshotSignatures(signatures, snapshotSignatures(initial)), { changed: true, timerOnly: true });
});

test('meeting, folder and non-clock session changes remain visible', () => {
  const initial = snapshot(), signatures = snapshotSignatures(initial);
  const edits = [
    value => { value.meetings[0].title = 'Updated'; },
    value => { value.meetings[0].notes += ' updated'; },
    value => { value.meetings[0].attendees[0].name = 'Updated'; },
    value => { value.meetings[0].folderId = 'another'; },
    value => { value.meetings[0].deletedAt = 1; },
    value => { value.meetings[0].audio.frames++; },
    value => { value.meetings[0].captureQuality.xrunCount++; },
    value => { value.meetings[0].calendarReference.title = 'Updated'; },
    value => { value.meetings[0].futureField = { value: 'kept' }; },
    value => { value.meetings = []; },
    value => { value.folders[0].name = 'Updated'; },
    value => { value.session.state = 'paused'; },
    value => { value.session.warning = 'Device interrupted'; },
    value => { value.session = null; },
  ];
  for (const edit of edits) {
    const next = structuredClone(initial); edit(next);
    assert.deepEqual(compareSnapshotSignatures(signatures, snapshotSignatures(next)), { changed: true, timerOnly: false });
  }
});

test('empty folders and absent sessions retain their existing normalized identity', () => {
  const initial = snapshotSignatures({ meetings: [] });
  assert.deepEqual(compareSnapshotSignatures(initial, snapshotSignatures({ revision: 9, meetings: [], folders: [], session: null })), { changed: false, timerOnly: true });
});
