import assert from 'node:assert/strict';
import test from 'node:test';
import { processingJobs, pendingJob, processingResults } from '../src/processing-state.js';

test('pending ownership follows the meeting and model kind rather than the foreground download', () => {
  const progress = { id: 'download', kind: 'summary-model', status: 'running', jobs: [
    { id: 'download', kind: 'summary-model', status: 'running' },
    { id: 'speech', kind: 'transcribe', meetingId: 'meeting-a', status: 'queued' },
    { id: 'cancelled', kind: 'summary', meetingId: 'meeting-b', status: 'cancelled' },
  ] };
  assert.equal(pendingJob(progress, { meetingId: 'meeting-a' }).id, 'speech');
  assert.equal(pendingJob(progress, { meetingId: 'meeting-b' }), undefined);
  assert.equal(pendingJob(progress, { models: true }).id, 'download');
  assert.equal(processingJobs(progress).length, 3);
});

test('a background result transition remains visible when the foreground job is unchanged', () => {
  const background = { id: 'speech', kind: 'transcribe', meetingId: 'meeting-a', status: 'running' };
  const previous = { id: 'download', kind: 'model', status: 'running', jobs: [background] };
  const current = { ...previous, jobs: [{ ...background, status: 'interrupted' }] };
  assert.deepEqual(processingResults(previous, current).map(job => job.id), ['speech']);
  assert.deepEqual(processingResults(current, current), []);
});

test('initial recovery does not replay every old result and a user cancellation is never pending', () => {
  const progress = { status: 'idle', jobs: [
    { id: 'old', kind: 'summary', meetingId: 'meeting-a', status: 'completed' },
    { id: 'cancelled', kind: 'transcribe', meetingId: 'meeting-b', status: 'cancelled' },
  ] };
  assert.deepEqual(processingResults({}, progress), []);
  assert.equal(pendingJob(progress), undefined);
});

test('current metadata supplies retry eligibility without replacing running transfer fields', () => {
  const progress = { id: 'current', kind: 'model', status: 'running', bytes: 20, jobs: [
    { id: 'current', kind: 'model', status: 'running', retryable: false },
    { id: 'retry', kind: 'summary', status: 'interrupted', retryable: true },
  ] };
  assert.equal(processingJobs(progress).find(job => job.id === 'current').bytes, 20);
  assert.equal(processingJobs(progress).find(job => job.id === 'retry').retryable, true);
});

test('cancellation keeps the meeting owned until worker termination is confirmed', () => {
  const progress = { status: 'idle', jobs: [{ id: 'stopping', kind: 'summary', meetingId: 'meeting-a', status: 'cancelling' }] };
  assert.equal(pendingJob(progress, { meetingId: 'meeting-a' }).id, 'stopping');
  assert.equal(pendingJob({ ...progress, jobs: [{ ...progress.jobs[0], status: 'cancelled' }] }), undefined);
});
