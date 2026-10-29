import assert from 'node:assert/strict';
import test from 'node:test';
import { readStoredSnapshot } from '../../../design-system/prototype/notification-snapshot.mjs';
import { createNotificationFlow, notificationTransition } from '../../../design-system/notification-flow.mjs';

const initialFolders = ['제품팀', '인터뷰'];
const meeting = { id: 5, title: '새 회의', folder: '추가 폴더', date: '10. 30. 10:30', length: '', draft: true, notes: '남길 메모', audioName: '음성.wav', calendar: { title: '일정', date: '2026-10-30T10:00' } };

test('snapshot reload preserves custom and empty folders, meeting metadata and interrupted jobs', () => {
  const flow = notificationTransition(createNotificationFlow(), { type: 'start', meetingId: 5 });
  const payload = { version: 1, meetings: [{ ...meeting, recording: true }], userFolders: [...initialFolders, '추가 폴더', '빈 폴더'], flow };
  const restored = readStoredSnapshot(JSON.parse(JSON.stringify(payload)), initialFolders);
  assert.deepEqual(restored.userFolders, payload.userFolders);
  assert.deepEqual(restored.meetings, [{ ...meeting, recording: false }]);
  assert.equal(restored.flow.jobs[0].status, 'interrupted');
  assert.equal(payload.meetings[0].recording, true);
});

test('historical snapshot recovers referenced folders without replacing meeting metadata', () => {
  const restored = readStoredSnapshot({ meetings: [meeting], flow: createNotificationFlow() }, initialFolders);
  assert.deepEqual(restored.userFolders, [...initialFolders, '추가 폴더']);
  assert.deepEqual(restored.meetings, [{ ...meeting, recording: false }]);
  for (const userFolders of [null, 'invalid', [12], ['']]) {
    assert.deepEqual(readStoredSnapshot({ meetings: [meeting], userFolders }, initialFolders).userFolders, restored.userFolders);
  }
});

test('unsupported snapshots and malformed meeting collections are rejected', () => {
  for (const value of [null, {}, { version: 2, meetings: [meeting] }, { meetings: {} }, { meetings: [] }, { meetings: [null] }, { meetings: [meeting, meeting] }, { meetings: [{ ...meeting, folder: {} }] }]) {
    assert.equal(readStoredSnapshot(value, initialFolders), null);
  }
  assert.deepEqual(initialFolders, ['제품팀', '인터뷰']);
});
